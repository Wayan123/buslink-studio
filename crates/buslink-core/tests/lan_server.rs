//! LAN dashboard (lan-mqtt Req 1 and 2): properties L1, L3, L5, L6 and the limits.
//! The server runs for real on loopback with TLS; the client pins its certificate.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

#[path = "common/lan_client.rs"]
mod lan_client;

use buslink_core::alarm::AlarmSnapshot;
use buslink_core::bus::{BusSubscriber, SampleBus};
use buslink_core::codec::RawValue;
use buslink_core::lan::limits::LimitConfig;
use buslink_core::lan::{Asset, AssetSource, LanConfig, LanServer, LanSource, MapAssets, ROUTES, TrendFail};
use buslink_core::poll::scheduler::DeviceCounters;
use buslink_core::profile::Profile;
use buslink_core::quality::{Quality, Sample, Value};
use buslink_core::session::{DeviceView, LinkState, LinkView, LiveSnapshot};
use buslink_core::trend::TrendSeries;
use futures_util::StreamExt;
use lan_client::{request, tls_connect, ws_connect};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

struct TestSource {
    bus: SampleBus,
}

impl LanSource for TestSource {
    fn subscribe(&self) -> BusSubscriber {
        self.bus.subscribe()
    }

    fn snapshot(&self) -> LiveSnapshot {
        let device = DeviceView {
            id: "dev1".into(),
            name: "Soil sensor".into(),
            link_id: "bus1".into(),
            unit_id: 1,
            profile_id: "p".into(),
            profile_version: "1.0.0".into(),
            simulated_only: false,
            poll_interval_ms: 1000,
            stale_after_ms: 3000,
            counters: DeviceCounters::default(),
        };
        LiveSnapshot {
            ts_ms: 1,
            session_id: "s-test".into(),
            tags: vec![],
            links: vec![LinkView {
                id: "bus1".into(),
                kind: "simulator",
                label: "test".into(),
                simulated: true,
                state: LinkState::Polling,
                error: None,
                devices: vec![device],
            }],
            alarms: AlarmSnapshot::default(),
            historian_error: None,
        }
    }

    fn profiles(&self) -> Vec<Arc<Profile>> {
        vec![]
    }

    fn trend(
        &self,
        device: &str,
        tag: &str,
        from_ms: i64,
        to_ms: i64,
        _buckets: usize,
    ) -> Result<TrendSeries, TrendFail> {
        if device == "dev1" && tag == "t1" {
            Ok(TrendSeries {
                t: vec![from_ms, to_ms],
                v: vec![Some(1.0), None],
                ..TrendSeries::default()
            })
        } else {
            Err(TrendFail::NotFound)
        }
    }
}

fn assets() -> MapAssets {
    let mut m = HashMap::new();
    m.insert(
        "index.html".to_string(),
        (
            "text/html".to_string(),
            b"<!doctype html><html><head><title>x</title></head><body></body></html>".to_vec(),
        ),
    );
    m.insert(
        "assets/app.js".to_string(),
        ("text/javascript".to_string(), b"console.log(1)".to_vec()),
    );
    MapAssets(m)
}

fn cfg() -> LanConfig {
    let mut c = LanConfig::new(IpAddr::from([127, 0, 0, 1]), 0);
    c.allow_loopback = true;
    c.limits = LimitConfig {
        capacity: 10_000.0,
        refill_per_s: 10_000.0,
        fail_limit: 10_000,
        ..LimitConfig::default()
    };
    c
}

async fn start(cfg: LanConfig) -> (LanServer, SampleBus) {
    let bus = SampleBus::default();
    let src = Arc::new(TestSource { bus: bus.clone() });
    let s = LanServer::start(cfg, src, Arc::new(assets())).await.unwrap();
    (s, bus)
}

fn sample(seq: u64, tag: &str, q: Quality, v: f64, ts: i64) -> Sample {
    let good = q.is_good();
    Sample {
        seq,
        link_id: "bus1".into(),
        device_id: "dev1".into(),
        tag_id: tag.into(),
        profile_id: "p".into(),
        profile_version: "1.0.0".into(),
        ts_ms: ts,
        quality: q,
        value: Some(Value {
            raw: RawValue::F64(v),
            engineering: Some(v),
            text: None,
        }),
        value_ts_ms: Some(if good { ts } else { ts - 5000 }),
        age_ms: Some(if good { 0 } else { 5000 }),
        ts_source_ms: None,
        raw: Some(vec![1]),
        unit: Some("°C".into()),
        error: None,
    }
}

/// Like the Tauri asset resolver: any unknown path gets index.html.
struct FallbackAssets(MapAssets);

impl AssetSource for FallbackAssets {
    fn get(&self, path: &str) -> Option<Asset> {
        self.0.get(path).or_else(|| self.0.get("index.html"))
    }
}

#[tokio::test]
async fn unknown_api_paths_stay_404_when_assets_fall_back_to_index() {
    let bus = SampleBus::default();
    let src = Arc::new(TestSource { bus });
    let srv = LanServer::start(cfg(), src, Arc::new(FallbackAssets(assets())))
        .await
        .unwrap();
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    for p in ["/api/command", "/api/write", "/api/arm", "/api/"] {
        let r = request(addr, &fp, "GET", &format!("{p}?t={t}"), &[], None).await;
        assert_eq!(r.status, 404, "{p}");
    }
    srv.stop().await;
}

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"];

fn targets() -> Vec<String> {
    let mut t: Vec<String> = ROUTES.iter().map(|(p, _, _)| (*p).to_string()).collect();
    t.push("/api/trend?tag=dev1/t1&from=1&to=2".into());
    t.extend(
        [
            "/assets/app.js",
            "/nope",
            "/api/command",
            "/api/write",
            "/api/arm",
            "/%2e%2e/secret",
        ]
        .map(String::from),
    );
    t
}

/// L1: no token, wrong token, wrong cookie: 401 for every route and method, known or unknown.
#[tokio::test]
async fn l1_token_required_on_every_route_and_method() {
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp) = (srv.addr(), srv.fingerprint().to_string());
    for target in targets() {
        for m in METHODS {
            let sep = if target.contains('?') { '&' } else { '?' };
            let wrong = format!("{target}{sep}t=00000000000000000000000000000000");
            let short = format!("{target}{sep}t=");
            let r = request(addr, &fp, m, &target, &[], None).await;
            assert_eq!(r.status, 401, "{m} {target} without token");
            let r = request(addr, &fp, m, &wrong, &[], None).await;
            assert_eq!(r.status, 401, "{m} {wrong}");
            let r = request(addr, &fp, m, &short, &[], None).await;
            assert_eq!(r.status, 401, "{m} {short}");
            let r = request(addr, &fp, m, &target, &[("Cookie", "bl_t=nope")], None).await;
            assert_eq!(r.status, 401, "{m} {target} bad cookie");
        }
    }
    // The 401 gives nothing away.
    let r = request(addr, &fp, "GET", "/", &[], None).await;
    assert_eq!(r.text(), "unauthorized");
    srv.stop().await;
}

/// L3: the route table is GET only, and with a valid token only GET ever succeeds.
#[tokio::test]
async fn l3_no_write_routes_and_no_bodies() {
    assert!(ROUTES.iter().all(|(_, m, _)| *m == "GET"));
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    for target in targets() {
        let sep = if target.contains('?') { '&' } else { '?' };
        let url = format!("{target}{sep}t={t}");
        for m in METHODS.iter().filter(|m| **m != "GET") {
            let r = request(addr, &fp, m, &url, &[], None).await;
            assert!(!(200..300).contains(&r.status), "{m} {target} returned {}", r.status);
            assert_eq!(r.status, 405, "{m} {target}");
            assert_eq!(r.header("allow"), Some("GET"));
            // A body does not help either.
            let r = request(addr, &fp, m, &url, &[], Some(b"{\"write\":1}")).await;
            assert!(!(200..300).contains(&r.status), "{m} {target} with body");
        }
        let r = request(addr, &fp, "GET", &url, &[], Some(b"x")).await;
        assert_eq!(r.status, 400, "GET {target} with a body");
    }
    srv.stop().await;
}

#[tokio::test]
async fn get_routes_serve_data_with_security_headers_and_cookie() {
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());

    let r = request(addr, &fp, "GET", &format!("/?t={t}"), &[], None).await;
    assert_eq!(r.status, 200);
    assert!(
        r.text().contains("<head><meta name=\"buslink-mode\" content=\"lan\">"),
        "{}",
        r.text()
    );
    let cookie = r.header("set-cookie").unwrap().to_string();
    assert!(
        cookie.contains(&format!("bl_t={t}"))
            && cookie.contains("Secure")
            && cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict")
    );
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    let csp = r.header("content-security-policy").unwrap();
    assert!(
        csp.contains("default-src 'self'") && csp.contains(&format!("wss://{addr}")) && !csp.contains("unsafe-eval")
    );

    // Assets load with the cookie alone.
    let r = request(
        addr,
        &fp,
        "GET",
        "/assets/app.js",
        &[("Cookie", &format!("bl_t={t}"))],
        None,
    )
    .await;
    assert_eq!((r.status, r.text().as_str()), (200, "console.log(1)"));
    assert_eq!(r.header("content-type"), Some("text/javascript"));
    // No directory traversal and no unknown files.
    for p in ["/../Cargo.toml", "/assets/../index.html", "/missing.js", "/assets"] {
        let r = request(addr, &fp, "GET", &format!("{p}?t={t}"), &[], None).await;
        assert_eq!(r.status, 404, "{p}");
    }

    let r = request(addr, &fp, "GET", &format!("/api/snapshot?t={t}"), &[], None).await;
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["session_id"], "s-test");
    assert!(v["profiles"].is_array() && v["links"][0]["devices"][0]["id"] == "dev1");

    let q = format!("/api/trend?tag=dev1/t1&from=1000&to=5000&buckets=50&t={t}");
    let r = request(addr, &fp, "GET", &q, &[], None).await;
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["t"], serde_json::json!([1000, 5000]));
    for (q, want) in [
        ("tag=dev1/zz&from=1&to=2", 404),
        ("tag=dev1/t1&from=5&to=5", 400),
        ("tag=dev1/t1&from=9&to=2", 400),
        ("tag=dev1/t1&from=0&to=99999999999999", 400),
        ("tag=nodevice&from=1&to=2", 400),
        ("tag=dev1/t1&from=x&to=2", 400),
        ("from=1&to=2", 400),
    ] {
        let r = request(addr, &fp, "GET", &format!("/api/trend?{q}&t={t}"), &[], None).await;
        assert_eq!(r.status, want, "{q}");
    }
    // A plain GET of the stream route is not a WebSocket upgrade.
    let r = request(addr, &fp, "GET", &format!("/api/stream?t={t}"), &[], None).await;
    assert!(!(200..300).contains(&r.status));
    srv.stop().await;
}

fn frame_tags(v: &serde_json::Value) -> Vec<&serde_json::Value> {
    v["data"]["tags"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// L5: what the WebSocket carries is what the bus carried.
#[tokio::test]
async fn l5_stream_carries_the_same_samples_as_the_bus() {
    let (srv, bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    let mut ws = ws_connect(addr, &fp, &format!("?t={t}"), None).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(first) = first else {
        panic!("first message must be text")
    };
    let first: serde_json::Value = serde_json::from_str(first.as_str()).unwrap();
    assert_eq!(first["type"], "snapshot");
    assert_eq!(first["data"]["tags"].as_array().unwrap().len(), 0);

    let published = vec![
        sample(1, "t1", Quality::Good, 21.5, 1_000),
        sample(2, "t2", Quality::Good, 40.0, 1_010),
        sample(3, "t1", Quality::Good, 21.7, 2_000),
        sample(4, "t1", Quality::Timeout, 21.7, 3_000),
        sample(5, "t2", Quality::Exception(2), 40.0, 3_010),
    ];
    for s in &published {
        bus.publish(s.clone());
    }

    let mut last_tag: HashMap<String, serde_json::Value> = HashMap::new();
    let mut points: HashMap<String, Vec<(i64, Option<f64>, String)>> = HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while points.values().map(Vec::len).sum::<usize>() < published.len() {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("frames arrive")
            .unwrap()
            .unwrap();
        let Message::Text(txt) = msg else { continue };
        let v: serde_json::Value = serde_json::from_str(txt.as_str()).unwrap();
        if v["type"] != "frame" {
            continue;
        }
        for tag in frame_tags(&v) {
            let key = tag["key"].as_str().unwrap().to_string();
            if let Some(prev) = last_tag.get(&key) {
                assert!(
                    tag["seq"].as_u64() >= prev["seq"].as_u64(),
                    "a tag never goes back in seq"
                );
            }
            last_tag.insert(key, tag.clone());
        }
        for p in v["data"]["points"].as_array().unwrap() {
            points.entry(p["k"].as_str().unwrap().into()).or_default().push((
                p["t"].as_i64().unwrap(),
                p["v"].as_f64(),
                p["q"].as_str().unwrap().into(),
            ));
        }
    }
    // Points: same time, value (GOOD only) and quality, in publish order, per tag.
    for tag in ["t1", "t2"] {
        let want: Vec<_> = published
            .iter()
            .filter(|s| s.tag_id == tag)
            .map(|s| (s.ts_ms, s.fresh_value(), s.quality.as_str().to_string()))
            .collect();
        assert_eq!(points[&format!("dev1/{tag}")], want, "{tag}");
    }
    // Final tag views: the last sample's seq, value, unit, quality, exception and age.
    for (tag, s) in [("t1", &published[3]), ("t2", &published[4])] {
        let v = &last_tag[&format!("dev1/{tag}")];
        assert_eq!(v["seq"].as_u64(), Some(s.seq));
        assert_eq!(v["quality"], s.quality.as_str());
        assert_eq!(v["exception"].as_u64(), s.quality.exception_code().map(u64::from));
        assert_eq!(
            v["value"].as_f64(),
            s.value.as_ref().and_then(|x| x.engineering),
            "last good value is kept, never 0"
        );
        assert_eq!(v["unit"], "°C");
        assert_eq!(v["age_ms"].as_u64(), s.age_ms);
        assert_eq!(
            v["value_ts_ms"].as_i64(),
            s.value_ts_ms,
            "a failed read keeps the original timestamp"
        );
    }
    // A late client sees the same state in its snapshot.
    let r = request(addr, &fp, "GET", &format!("/api/snapshot?t={t}"), &[], None).await;
    let snap: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    let tags = snap["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 2);
    assert!(
        tags.iter()
            .any(|x| x["key"] == "dev1/t1" && x["seq"] == 4 && x["quality"] == "TIMEOUT")
    );
    srv.stop().await;
}

/// L6: stop closes everything; a restart is a different server for the old URL.
#[tokio::test]
async fn l6_stop_closes_everything_and_restart_issues_new_credentials() {
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    let mut ws = ws_connect(addr, &fp, &format!("?t={t}"), None).await.unwrap();
    let _ = ws.next().await; // snapshot
    assert_eq!(srv.status().details.unwrap().clients.len(), 1);
    srv.stop().await;

    // The open socket ends and the port refuses new connections.
    let end = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "websocket was closed by stop");
    assert!(tls_connect(addr, &fp).await.is_err(), "port is closed after stop");

    // Restart on the same port: new token, new certificate, old URL is dead.
    let mut c = cfg();
    c.port = addr.port();
    let (srv2, _bus2) = start(c).await;
    assert_eq!(srv2.addr(), addr);
    assert_ne!(srv2.token(), t);
    assert_ne!(srv2.fingerprint(), fp);
    let fp2 = srv2.fingerprint().to_string();
    let old = request(addr, &fp2, "GET", &format!("/api/snapshot?t={t}"), &[], None).await;
    assert_eq!(old.status, 401);
    let new = request(
        addr,
        &fp2,
        "GET",
        &format!("/api/snapshot?t={}", srv2.token()),
        &[],
        None,
    )
    .await;
    assert_eq!(new.status, 200);
    // The old certificate is not accepted either (the client pins the fingerprint).
    assert!(tls_connect(addr, &fp).await.is_err());
    srv2.stop().await;
}

#[tokio::test]
async fn client_limit_origin_and_lockout() {
    let mut c = cfg();
    c.max_clients = 2;
    let (srv, _bus) = start(c).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    let q = format!("?t={t}");

    // Cross-origin upgrade is refused.
    assert_eq!(
        ws_connect(addr, &fp, &q, Some("https://evil.example")).await.err(),
        Some(403)
    );
    let host = addr.to_string();
    let same = format!("https://{host}");
    let a = ws_connect(addr, &fp, &q, Some(&same)).await.unwrap();
    let _b = ws_connect(addr, &fp, &q, None).await.unwrap();
    assert_eq!(
        ws_connect(addr, &fp, &q, None).await.err(),
        Some(503),
        "third viewer is refused"
    );
    assert_eq!(srv.status().details.unwrap().refused_clients, 1);
    drop(a);
    let mut ok = false;
    for _ in 0..40 {
        if ws_connect(addr, &fp, &q, None).await.is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "a slot frees up when a viewer leaves");
    srv.stop().await;
}

#[tokio::test]
async fn five_bad_tokens_lock_the_address_out_even_for_the_right_token() {
    let mut c = cfg();
    c.limits.fail_limit = 5;
    let (srv, _bus) = start(c).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    for _ in 0..5 {
        assert_eq!(
            request(addr, &fp, "GET", "/api/snapshot?t=bad", &[], None).await.status,
            401
        );
    }
    let r = request(addr, &fp, "GET", &format!("/api/snapshot?t={t}"), &[], None).await;
    assert_eq!(r.status, 429);
    assert!(r.header("retry-after").is_some());
    let d = srv.status().details.unwrap();
    assert_eq!((d.rejected, d.rate_limited), (5, 1));
    srv.stop().await;
}

#[tokio::test]
async fn request_rate_is_limited() {
    let mut c = cfg();
    c.limits.capacity = 5.0;
    c.limits.refill_per_s = 0.5;
    let (srv, _bus) = start(c).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    let mut codes = Vec::new();
    for _ in 0..8 {
        codes.push(
            request(addr, &fp, "GET", &format!("/api/snapshot?t={t}"), &[], None)
                .await
                .status,
        );
    }
    assert_eq!(&codes[..5], &[200; 5]);
    assert!(codes[5..].iter().all(|c| *c == 429), "{codes:?}");
    srv.stop().await;
}

#[tokio::test]
async fn refuses_to_bind_outside_private_ranges() {
    let bus = SampleBus::default();
    let src = Arc::new(TestSource { bus });
    for ip in [[0, 0, 0, 0], [8, 8, 8, 8], [127, 0, 0, 1]] {
        // Loopback is refused too unless the caller opts in.
        let r = LanServer::start(LanConfig::new(IpAddr::from(ip), 8443), src.clone(), Arc::new(assets())).await;
        assert!(r.is_err(), "{ip:?}");
    }
    let _ = SocketAddr::from(([127, 0, 0, 1], 1));
}
