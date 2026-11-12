//! The read-only HTTP surface (safety rule 15). Every route is GET, every request
//! passes the same guard (rate limit, token, no body, GET only), and nothing here
//! can reach the command service, the poll workers or a transport.

use super::hub::Hub;
use super::limits::{Limiter, Verdict};
use super::token::ct_eq;
use super::{AssetSource, LanSource, TrendFail};
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};

/// What a route does. The router is built from [`ROUTES`], so the table is the whole surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Index,
    Snapshot,
    Trend,
    Stream,
}

/// Every route of the server. All GET. Other paths fall through to the static
/// files of the frontend build (also GET only, also behind the token).
pub const ROUTES: &[(&str, &str, Kind)] = &[
    ("/", "GET", Kind::Index),
    ("/api/snapshot", "GET", Kind::Snapshot),
    ("/api/trend", "GET", Kind::Trend),
    ("/api/stream", "GET", Kind::Stream),
];

const COOKIE: &str = "bl_t";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_TREND_SPAN_MS: i64 = 366 * 86_400_000;

#[derive(Clone, Copy)]
pub(crate) struct ClientAddr(pub SocketAddr);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientInfo {
    pub id: u64,
    pub addr: String,
    pub since_ms: i64,
}

#[derive(Default)]
pub(crate) struct Stats {
    pub rejected: AtomicU64,
    pub limited: AtomicU64,
    pub refused_connections: AtomicU64,
    pub refused_clients: AtomicU64,
}

pub(crate) struct Ctx {
    pub token: String,
    pub host_port: String,
    pub source: Arc<dyn LanSource>,
    pub assets: Arc<dyn AssetSource>,
    pub hub: Arc<Hub>,
    pub limiter: Mutex<Limiter>,
    pub clients: Mutex<(u64, BTreeMap<u64, ClientInfo>)>,
    pub max_clients: usize,
    pub stats: Stats,
    pub shutdown: watch::Receiver<bool>,
    pub epoch: Instant,
    pub epoch_ms: i64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Ctx {
    fn now_ms(&self) -> i64 {
        self.epoch_ms + i64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(i64::MAX)
    }

    pub fn client_list(&self) -> Vec<ClientInfo> {
        lock(&self.clients).1.values().cloned().collect()
    }
}

struct ClientGuard {
    ctx: Arc<Ctx>,
    id: u64,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        lock(&self.ctx.clients).1.remove(&self.id);
    }
}

pub(crate) fn router(ctx: Arc<Ctx>) -> Router {
    let mut r = Router::new();
    for (path, _method, kind) in ROUTES {
        r = match kind {
            Kind::Index => r.route(path, get(static_file)),
            Kind::Snapshot => r.route(path, get(snapshot)),
            Kind::Trend => r.route(path, get(trend)),
            Kind::Stream => r.route(path, get(stream)),
        };
    }
    r.fallback(get(static_file))
        .layer(middleware::from_fn_with_state(Arc::clone(&ctx), guard))
        .with_state(ctx)
}

// ---- guard ----

fn plain(status: StatusCode, body: &'static str) -> Response {
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r
}

fn harden(mut r: Response, csp: &str) -> Response {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if let Ok(v) = HeaderValue::from_str(csp) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    r
}

pub(crate) fn csp(host_port: &str) -> String {
    format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
         font-src 'self'; connect-src 'self' wss://{host_port}; object-src 'none'; base-uri 'self'; \
         frame-ancestors 'none'; form-action 'none'"
    )
}

/// `(token valid, it came from the query string)`
fn authenticate(req: &Request, token: &str) -> (bool, bool) {
    let mut via_query = false;
    if let Some(q) = req.uri().query() {
        for pair in q.split('&') {
            if let Some(("t", v)) = pair.split_once('=')
                && ct_eq(v.as_bytes(), token.as_bytes())
            {
                via_query = true;
            }
        }
    }
    if via_query {
        return (true, true);
    }
    let mut cookie_ok = false;
    for v in req.headers().get_all(header::COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        for part in s.split(';') {
            if let Some((k, v)) = part.trim().split_once('=')
                && k == COOKIE
                && ct_eq(v.as_bytes(), token.as_bytes())
            {
                cookie_ok = true;
            }
        }
    }
    (cookie_ok, false)
}

fn has_body(h: &HeaderMap) -> bool {
    if h.contains_key(header::TRANSFER_ENCODING) {
        return true;
    }
    match h.get(header::CONTENT_LENGTH) {
        None => false,
        Some(v) => v.to_str().ok().and_then(|s| s.trim().parse::<u64>().ok()) != Some(0),
    }
}

async fn guard(
    State(ctx): State<Arc<Ctx>>,
    Extension(ClientAddr(peer)): Extension<ClientAddr>,
    req: Request,
    next: Next,
) -> Response {
    let csp = csp(&ctx.host_port);
    let now = ctx.now_ms();
    let verdict = lock(&ctx.limiter).check(peer.ip(), now);
    if let Verdict::Limited { retry_after_s } = verdict {
        ctx.stats.limited.fetch_add(1, Ordering::Relaxed);
        let mut r = plain(StatusCode::TOO_MANY_REQUESTS, "too many requests");
        if let Ok(v) = HeaderValue::from_str(&retry_after_s.to_string()) {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return harden(r, &csp);
    }
    let (ok, via_query) = authenticate(&req, &ctx.token);
    if !ok {
        lock(&ctx.limiter).record_failure(peer.ip(), now);
        ctx.stats.rejected.fetch_add(1, Ordering::Relaxed);
        return harden(plain(StatusCode::UNAUTHORIZED, "unauthorized"), &csp);
    }
    if has_body(req.headers()) {
        return harden(plain(StatusCode::BAD_REQUEST, "request bodies are not accepted"), &csp);
    }
    if req.method() != Method::GET {
        let mut r = plain(StatusCode::METHOD_NOT_ALLOWED, "read-only");
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET"));
        return harden(r, &csp);
    }
    let set_cookie = via_query && req.uri().path() == "/";
    let mut resp = next.run(req).await;
    if set_cookie
        && let Ok(v) = HeaderValue::from_str(&format!(
            "{COOKIE}={}; Path=/; Secure; HttpOnly; SameSite=Strict",
            ctx.token
        ))
    {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    harden(resp, &csp)
}

// ---- handlers ----

fn json<T: Serialize>(v: &T) -> Response {
    match serde_json::to_vec(v) {
        Ok(b) => {
            let mut r = Response::new(Body::from(b));
            r.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            r
        }
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "could not encode the response"),
    }
}

const MODE_META: &str = "<meta name=\"buslink-mode\" content=\"lan\">";

async fn static_file(State(ctx): State<Arc<Ctx>>, uri: Uri) -> Response {
    let raw = uri.path().trim_start_matches('/');
    let path = if raw.is_empty() { "index.html" } else { raw };
    // `api/…` is never a file: an asset source that falls back to index.html must not turn
    // an unknown API path into a 200.
    if path.len() > 256 || path.starts_with("api/") || path.contains("..") || path.contains('\\') || path.contains('\0')
    {
        return plain(StatusCode::NOT_FOUND, "not found");
    }
    let Some(asset) = ctx.assets.get(path) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let bytes = if path == "index.html" {
        // The server, not the URL, selects the read-only build.
        let html = String::from_utf8_lossy(&asset.bytes);
        if !html.contains("<head>") {
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "frontend cannot be served read-only");
        }
        html.replacen("<head>", &format!("<head>{MODE_META}"), 1).into_bytes()
    } else {
        asset.bytes
    };
    let mut r = Response::new(Body::from(bytes));
    if let Ok(v) = HeaderValue::from_str(&asset.mime) {
        r.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    r
}

async fn snapshot(State(ctx): State<Arc<Ctx>>) -> Response {
    json(&ctx.hub.snapshot())
}

#[derive(Deserialize)]
struct TrendQuery {
    tag: String,
    from: i64,
    to: i64,
    buckets: Option<usize>,
}

async fn trend(State(ctx): State<Arc<Ctx>>, Query(q): Query<TrendQuery>) -> Response {
    if q.from >= q.to || q.to - q.from > MAX_TREND_SPAN_MS {
        return plain(StatusCode::BAD_REQUEST, "from must be before to, within 366 days");
    }
    let Some((device, tag)) = q.tag.rsplit_once('/') else {
        return plain(StatusCode::BAD_REQUEST, "tag must be device/tag");
    };
    let (device, tag) = (device.to_string(), tag.to_string());
    let buckets = q.buckets.unwrap_or(600).clamp(10, 4000);
    let source = Arc::clone(&ctx.source);
    let res = tokio::task::spawn_blocking(move || source.trend(&device, &tag, q.from, q.to, buckets)).await;
    match res {
        Ok(Ok(series)) => json(&series),
        Ok(Err(TrendFail::NotFound)) => plain(StatusCode::NOT_FOUND, "unknown tag"),
        _ => plain(StatusCode::SERVICE_UNAVAILABLE, "history is not available"),
    }
}

fn origin_ok(h: &HeaderMap) -> bool {
    let Some(origin) = h.get(header::ORIGIN) else {
        return true;
    };
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    origin
        .to_str()
        .ok()
        .and_then(|o| o.strip_prefix("https://"))
        .is_some_and(|o| !host.is_empty() && o.eq_ignore_ascii_case(host))
}

async fn stream(
    State(ctx): State<Arc<Ctx>>,
    Extension(ClientAddr(peer)): Extension<ClientAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_ok(&headers) {
        return plain(StatusCode::FORBIDDEN, "origin not allowed");
    }
    let id = {
        let mut g = lock(&ctx.clients);
        if g.1.len() >= ctx.max_clients {
            ctx.stats.refused_clients.fetch_add(1, Ordering::Relaxed);
            let mut r = plain(StatusCode::SERVICE_UNAVAILABLE, "too many viewers");
            r.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
            return r;
        }
        g.0 += 1;
        let id = g.0;
        g.1.insert(
            id,
            ClientInfo {
                id,
                addr: peer.ip().to_string(),
                since_ms: ctx.now_ms(),
            },
        );
        id
    };
    let guard = ClientGuard {
        ctx: Arc::clone(&ctx),
        id,
    };
    ws.max_message_size(1024)
        .max_frame_size(1024)
        .on_upgrade(move |socket| client_task(socket, guard))
}

async fn send(socket: &mut WebSocket, text: &str) -> bool {
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(Utf8Bytes::from(text)))).await,
        Ok(Ok(()))
    )
}

async fn client_task(mut socket: WebSocket, guard: ClientGuard) {
    let ctx = Arc::clone(&guard.ctx);
    let mut shutdown = ctx.shutdown.clone();
    let (snap, mut rx) = ctx.hub.snapshot_and_subscribe();
    if !send(&mut socket, &snap).await {
        return;
    }
    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Ok(text) => {
                    if !send(&mut socket, &text).await { break; }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    // Never skip silently: say so, then resend the whole state.
                    let note = super::hub::message("lagged", &missed);
                    let (snap, fresh) = ctx.hub.snapshot_and_subscribe();
                    rx = fresh;
                    if !send(&mut socket, &note).await || !send(&mut socket, &snap).await { break; }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {} // pings are answered by axum; nothing a client sends is acted on
            },
            _ = shutdown.changed() => {
                let _ = socket.send(Message::Close(Some(CloseFrame { code: 1001, reason: Utf8Bytes::from_static("server stopped") }))).await;
                break;
            }
        }
    }
}
