//! Read-only LAN dashboard (PRD R22, architecture D6, safety rule 15).
//!
//! An axum server that serves the built frontend, a snapshot, a decimated trend
//! and a WebSocket stream of the sample bus to browsers on one private interface.
//! Off by default; a new self-signed certificate and token per start; GET only.
//! Nothing in this module can write to a device: it holds a [`LanSource`] that
//! can only read, and an [`AssetSource`] for static files.

pub mod hub;
pub mod limits;
pub mod net;
pub mod routes;
mod serve;
pub mod tls;
pub mod token;

use crate::bus::BusSubscriber;
use crate::profile::Profile;
use crate::session::{LiveSnapshot, Session};
use crate::trend::TrendSeries;
use limits::{LimitConfig, Limiter};
use net::{BindError, QrMatrix};
use routes::{ClientInfo, Ctx, Stats};
use serde::Serialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub use routes::ROUTES;

#[derive(Debug, Error)]
pub enum LanError {
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error("could not listen on {0}: {1}")]
    Listen(SocketAddr, String),
    #[error(transparent)]
    Tls(#[from] tls::TlsError),
    #[error("the operating system random source failed")]
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendFail {
    NotFound,
    Unavailable,
}

/// Everything the server may read. There is deliberately no method that writes.
pub trait LanSource: Send + Sync + 'static {
    fn subscribe(&self) -> BusSubscriber;
    fn snapshot(&self) -> LiveSnapshot;
    fn profiles(&self) -> Vec<Arc<Profile>>;
    fn trend(
        &self,
        device: &str,
        tag: &str,
        from_ms: i64,
        to_ms: i64,
        buckets: usize,
    ) -> Result<TrendSeries, TrendFail>;
}

impl LanSource for Session {
    fn subscribe(&self) -> BusSubscriber {
        Session::subscribe(self)
    }

    fn snapshot(&self) -> LiveSnapshot {
        Session::snapshot(self)
    }

    fn profiles(&self) -> Vec<Arc<Profile>> {
        Session::profiles(self)
    }

    fn trend(
        &self,
        device: &str,
        tag: &str,
        from_ms: i64,
        to_ms: i64,
        buckets: usize,
    ) -> Result<TrendSeries, TrendFail> {
        use crate::session::SessionError;
        Session::trend(self, device, tag, from_ms, to_ms, buckets).map_err(|e| match e {
            SessionError::UnknownDevice(_) => TrendFail::NotFound,
            _ => TrendFail::Unavailable,
        })
    }
}

pub struct Asset {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Static files of the built frontend (`index.html`, `assets/…`).
pub trait AssetSource: Send + Sync + 'static {
    fn get(&self, path: &str) -> Option<Asset>;
}

/// Fixed in-memory files; used by tests.
#[derive(Default)]
pub struct MapAssets(pub HashMap<String, (String, Vec<u8>)>);

impl AssetSource for MapAssets {
    fn get(&self, path: &str) -> Option<Asset> {
        self.0.get(path).map(|(m, b)| Asset {
            mime: m.clone(),
            bytes: b.clone(),
        })
    }
}

#[derive(Serialize)]
pub struct ProfileEntry {
    pub key: String,
    pub profile: Profile,
}

/// `GET /api/snapshot` and the first WebSocket message. Profiles ride along so the
/// browser can draw analog bars without a fifth route.
#[derive(Serialize)]
pub struct LanSnapshot {
    #[serde(flatten)]
    pub snapshot: LiveSnapshot,
    pub profiles: Vec<ProfileEntry>,
}

#[derive(Clone, Debug)]
pub struct LanConfig {
    pub ip: IpAddr,
    pub port: u16,
    /// Tests and the e2e build only; the UI never offers loopback.
    pub allow_loopback: bool,
    pub max_connections: usize,
    pub max_clients: usize,
    pub limits: LimitConfig,
}

impl LanConfig {
    pub fn new(ip: IpAddr, port: u16) -> Self {
        Self {
            ip,
            port,
            allow_loopback: false,
            max_connections: 32,
            max_clients: 8,
            limits: LimitConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LanDetails {
    pub ip: String,
    pub port: u16,
    /// Contains the token: show it on the host device only.
    pub url: String,
    pub fingerprint: String,
    pub started_ms: i64,
    pub clients: Vec<ClientInfo>,
    pub max_clients: usize,
    pub max_connections: usize,
    pub rejected: u64,
    pub rate_limited: u64,
    pub refused_connections: u64,
    pub refused_clients: u64,
    pub qr: Option<QrMatrix>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LanStatus {
    pub running: bool,
    pub details: Option<LanDetails>,
}

impl LanStatus {
    pub fn off() -> Self {
        Self {
            running: false,
            details: None,
        }
    }
}

pub struct LanServer {
    addr: SocketAddr,
    token: String,
    fingerprint: String,
    started_ms: i64,
    cfg: LanConfig,
    ctx: Arc<Ctx>,
    hub: Arc<hub::Hub>,
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl LanServer {
    /// Bind, create a fresh certificate and token, and start serving.
    pub async fn start(
        cfg: LanConfig,
        source: Arc<dyn LanSource>,
        assets: Arc<dyn AssetSource>,
    ) -> Result<LanServer, LanError> {
        let want = net::check_bind(cfg.ip, cfg.port, cfg.allow_loopback)?;
        let token = token::generate().ok_or(LanError::Random)?;
        let started_ms = now_ms();
        let cert = tls::generate(cfg.ip, started_ms)?;
        let acceptor = TlsAcceptor::from(tls::server_config(&cert)?);
        let listener = TcpListener::bind(want)
            .await
            .map_err(|e| LanError::Listen(want, e.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|e| LanError::Listen(want, e.to_string()))?;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let hub = hub::Hub::start(Arc::clone(&source));
        let ctx = Arc::new(Ctx {
            token: token.clone(),
            host_port: addr.to_string(),
            source,
            assets,
            hub: Arc::clone(&hub),
            limiter: Mutex::new(Limiter::new(cfg.limits)),
            clients: Mutex::new((0, Default::default())),
            max_clients: cfg.max_clients,
            stats: Stats::default(),
            shutdown: shutdown_rx.clone(),
            epoch: std::time::Instant::now(),
            epoch_ms: started_ms,
        });
        let task = tokio::spawn(serve::run(
            listener,
            acceptor,
            Arc::clone(&ctx),
            cfg.max_connections,
            shutdown_rx,
        ));
        Ok(LanServer {
            addr,
            token,
            fingerprint: cert.fingerprint,
            started_ms,
            cfg,
            ctx,
            hub,
            shutdown,
            task: Some(task),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn url(&self) -> String {
        format!("https://{}:{}/?t={}", self.addr.ip(), self.addr.port(), self.token)
    }

    pub fn status(&self) -> LanStatus {
        let s = &self.ctx.stats;
        let url = self.url();
        LanStatus {
            running: true,
            details: Some(LanDetails {
                ip: self.addr.ip().to_string(),
                port: self.addr.port(),
                qr: net::qr_matrix(&url),
                url,
                fingerprint: self.fingerprint.clone(),
                started_ms: self.started_ms,
                clients: self.ctx.client_list(),
                max_clients: self.cfg.max_clients,
                max_connections: self.cfg.max_connections,
                rejected: s.rejected.load(Ordering::Relaxed),
                rate_limited: s.limited.load(Ordering::Relaxed),
                refused_connections: s.refused_connections.load(Ordering::Relaxed),
                refused_clients: s.refused_clients.load(Ordering::Relaxed),
            }),
        }
    }

    /// Close the listener and every connection, then forget the token and certificate.
    /// The port is free when this returns.
    pub async fn stop(mut self) {
        self.shutdown_now().await;
    }

    async fn shutdown_now(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        let hub = Arc::clone(&self.hub);
        // The hub thread sleeps in 20 ms steps; join it off the async worker.
        let _ = tokio::task::spawn_blocking(move || hub.stop()).await;
    }
}

impl Drop for LanServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.hub.stop();
    }
}
