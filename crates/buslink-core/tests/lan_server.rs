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
        vec![Arc::new(sensitive_profile())]
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

/// The session cookie a browser holds after the entry redirect.
fn cookie(t: &str) -> [(&'static str, String); 1] {
    [("Cookie", format!("bl_t={t}"))]
}

async fn get_with_cookie(srv: &LanServer, target: &str) -> lan_client::Resp {
    let ck = cookie(srv.token());
    request(
        srv.addr(),
        srv.fingerprint(),
        "GET",
        target,
        &[(ck[0].0, ck[0].1.as_str())],
        None,
    )
    .await
}

const SECRET_URL: &str = "https://intranet.example/manuals/secret-manual.pdf";
const SECRET_RECORD: &str = "D:\\records\\secret-test-log.txt";

/// A real profile with the fields that must not reach a browser filled in.
fn sensitive_profile() -> Profile {
    use buslink_core::profile::{Evidence, HardwareRecord, TestResult, Text};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/bgt-smps-8in1.json");
    let mut p = buslink_core::profile::load(&std::fs::read(path).unwrap())
        .unwrap()
        .profile;
    if let Some(s) = p.source.as_mut() {
        s.url = Some(SECRET_URL.into());
    }
    p.evidence = Some(Evidence {
        hardware: vec![HardwareRecord {
            date: "2026-11-01".into(),
            adapter: Text::Plain("adapter".into()),
            platform: Text::Plain("platform".into()),
            record: Text::Plain(SECRET_RECORD.into()),
            result: TestResult::Pass,
            firmware: None,
            unit_id: None,
            notes: None,
        }],
    });
    p
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
        let r = request(addr, &fp, "GET", p, &[("Cookie", &format!("bl_t={t}"))], None).await;
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
        let url = target.clone();
        let ck = format!("bl_t={t}");
        let ck = [("Cookie", ck.as_str())];
        for m in METHODS.iter().filter(|m| **m != "GET") {
            let r = request(addr, &fp, m, &url, &ck, None).await;
            assert!(!(200..300).contains(&r.status), "{m} {target} returned {}", r.status);
            assert_eq!(r.status, 405, "{m} {target}");
            assert_eq!(r.header("allow"), Some("GET"));
            // A body does not help either.
            let r = request(addr, &fp, m, &url, &ck, Some(b"{\"write\":1}")).await;
            assert!(!(200..300).contains(&r.status), "{m} {target} with body");
        }
        let r = request(addr, &fp, "GET", &url, &ck, Some(b"x")).await;
        assert_eq!(r.status, 400, "GET {target} with a body");
    }
    srv.stop().await;
}

#[tokio::test]
async fn get_routes_serve_data_with_security_headers_and_cookie() {
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());

    // The entry link sets the cookie and redirects to `/` without the token (L1).
    let r = request(addr, &fp, "GET", &format!("/?t={t}"), &[], None).await;
    assert_eq!(r.status, 303);
    assert_eq!(r.header("location"), Some("/"));
    assert!(r.body.is_empty() && !r.text().contains(&t));
    let cookie = r.header("set-cookie").unwrap().to_string();
    assert!(
        cookie.contains(&format!("bl_t={t}"))
            && cookie.contains("Secure")
            && cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict")
    );
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let r = get_with_cookie(&srv, "/").await;
    assert_eq!(r.status, 200);
    assert!(
        r.text().contains("<head><meta name=\"buslink-mode\" content=\"lan\">"),
        "{}",
        r.text()
    );
    assert!(r.header("set-cookie").is_none());
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
        let r = get_with_cookie(&srv, p).await;
        assert_eq!(r.status, 404, "{p}");
    }

    let r = get_with_cookie(&srv, "/api/snapshot").await;
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["session_id"], "s-test");
    assert!(v["profiles"].is_array() && v["links"][0]["devices"][0]["id"] == "dev1");

    let r = get_with_cookie(&srv, "/api/trend?tag=dev1/t1&from=1000&to=5000&buckets=50").await;
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
        let r = get_with_cookie(&srv, &format!("/api/trend?{q}")).await;
        assert_eq!(r.status, want, "{q}");
    }
    // A plain GET of the stream route is not a WebSocket upgrade.
    let r = get_with_cookie(&srv, "/api/stream").await;
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
    let mut ws = ws_connect(addr, &fp, "", Some(&t), None).await.unwrap();
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
    let r = get_with_cookie(&srv, "/api/snapshot").await;
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
    let mut ws = ws_connect(addr, &fp, "", Some(&t), None).await.unwrap();
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
    let old = request(
        addr,
        &fp2,
        "GET",
        "/api/snapshot",
        &[("Cookie", &format!("bl_t={t}"))],
        None,
    )
    .await;
    assert_eq!(old.status, 401);
    let old_entry = request(addr, &fp2, "GET", &format!("/?t={t}"), &[], None).await;
    assert_eq!(old_entry.status, 401);
    assert_eq!(get_with_cookie(&srv2, "/api/snapshot").await.status, 200);
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
    let tk = Some(t.as_str());

    // Cross-origin upgrade is refused.
    assert_eq!(
        ws_connect(addr, &fp, "", tk, Some("https://evil.example")).await.err(),
        Some(403)
    );
    let host = addr.to_string();
    let same = format!("https://{host}");
    let a = ws_connect(addr, &fp, "", tk, Some(&same)).await.unwrap();
    let _b = ws_connect(addr, &fp, "", tk, None).await.unwrap();
    assert_eq!(
        ws_connect(addr, &fp, "", tk, None).await.err(),
        Some(503),
        "third viewer is refused"
    );
    assert_eq!(srv.status().details.unwrap().refused_clients, 1);
    drop(a);
    let mut ok = false;
    for _ in 0..40 {
        if ws_connect(addr, &fp, "", tk, None).await.is_ok() {
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
    let r = request(
        addr,
        &fp,
        "GET",
        "/api/snapshot",
        &[("Cookie", &format!("bl_t={t}"))],
        None,
    )
    .await;
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
            request(
                addr,
                &fp,
                "GET",
                "/api/snapshot",
                &[("Cookie", &format!("bl_t={t}"))],
                None,
            )
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

/// L2 (security review 07): viewers get a display subset of each profile, not the profile.
#[tokio::test]
async fn l2_snapshot_profiles_are_a_display_subset() {
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    let full = serde_json::to_string(&sensitive_profile()).unwrap();
    assert!(
        full.contains(SECRET_URL) && full.contains("secret-test-log"),
        "fixture sanity"
    );

    let r = get_with_cookie(&srv, "/api/snapshot").await;
    assert_eq!(r.status, 200);
    let mut bodies = vec![r.text()];
    let mut ws = ws_connect(addr, &fp, "", Some(&t), None).await.unwrap();
    let Message::Text(first) = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    else {
        panic!("first message must be text")
    };
    bodies.push(first.as_str().to_string());
    let allowed_profile = [
        "profile_id",
        "profile_version",
        "vendor",
        "model",
        "support_level",
        "simulated_only",
        "registers",
    ];
    let allowed_register = [
        "id",
        "name",
        "type",
        "unit",
        "scale",
        "decimals",
        "valid_range",
        "display_range",
    ];
    for body in &bodies {
        for banned in [
            SECRET_URL,
            "secret-test-log",
            "secret-manual",
            "\"evidence\"",
            "\"source\"",
            "\"url\"",
            "\"record\"",
            "\"write\"",
            "\"commands\"",
            "\"comm_config\"",
            "\"word_order\"",
            "\"address\"",
            "\"doc_address\"",
        ] {
            assert!(!body.contains(banned), "{banned} reached the browser");
        }
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let v = if v["type"] == "snapshot" { v["data"].clone() } else { v };
        let entry = &v["profiles"][0];
        let want_key = format!(
            "{}@{}",
            entry["profile"]["profile_id"].as_str().unwrap(),
            entry["profile"]["profile_version"].as_str().unwrap()
        );
        assert_eq!(entry["key"], want_key.as_str());
        let prof = entry["profile"].as_object().unwrap();
        assert!(
            prof.keys().all(|k| allowed_profile.contains(&k.as_str())),
            "{:?}",
            prof.keys()
        );
        let regs = prof["registers"].as_array().unwrap();
        assert!(!regs.is_empty());
        for r in regs {
            let keys: Vec<_> = r.as_object().unwrap().keys().collect();
            assert!(keys.iter().all(|k| allowed_register.contains(&k.as_str())), "{keys:?}");
            assert!(r["id"].is_string() && (r["name"].is_object() || r["name"].is_string()));
        }
        // What the tiles and bars need is still there.
        assert!(regs.iter().any(|r| r["unit"].is_string()));
        assert!(
            regs.iter()
                .any(|r| r.get("display_range").is_some() || r.get("valid_range").is_some())
        );
    }
    srv.stop().await;
}

/// L1 (security review 07): the token in a URL works only on `GET /`, which hands it to a cookie.
#[tokio::test]
async fn l1_query_token_is_accepted_only_on_the_entry_page() {
    let (srv, _bus) = start(cfg()).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    for target in [
        "/api/snapshot",
        "/api/trend?tag=dev1/t1&from=1&to=2",
        "/api/stream",
        "/assets/app.js",
        "/index.html",
        "/nope",
    ] {
        let sep = if target.contains('?') { '&' } else { '?' };
        let r = request(addr, &fp, "GET", &format!("{target}{sep}t={t}"), &[], None).await;
        assert_eq!(r.status, 401, "{target} must not accept ?t=");
        assert!(r.header("set-cookie").is_none());
    }
    // The WebSocket upgrade does not take it either.
    assert_eq!(
        ws_connect(addr, &fp, &format!("?t={t}"), None, None).await.err(),
        Some(401)
    );
    // Entry page: redirect to `/` with no token or other query in the Location.
    for target in [format!("/?t={t}"), format!("/?x=1&t={t}&y=2")] {
        let r = request(addr, &fp, "GET", &target, &[], None).await;
        assert_eq!(r.status, 303, "{target}");
        assert_eq!(r.header("location"), Some("/"));
        assert!(r.header("set-cookie").unwrap().starts_with(&format!("bl_t={t};")));
        assert!(
            r.headers
                .iter()
                .all(|(_, v)| !v.contains("t=") || v.starts_with("bl_t="))
        );
    }
    // A wrong token on the entry page is a failure like any other.
    assert_eq!(request(addr, &fp, "GET", "/?t=nope", &[], None).await.status, 401);
    // With the cookie the page and the API work, and a stray ?t= is ignored there.
    assert_eq!(get_with_cookie(&srv, "/").await.status, 200);
    assert_eq!(get_with_cookie(&srv, "/api/snapshot?t=whatever").await.status, 200);
    assert!(ws_connect(addr, &fp, "", Some(&t), None).await.is_ok());
    srv.stop().await;
}

/// L3 (security review 07): one address cannot hold more than its share of connections.
#[tokio::test]
async fn l3_per_address_connection_cap() {
    let mut c = cfg();
    c.max_connections_per_ip = 3;
    c.max_unauthenticated = 16;
    let (srv, _bus) = start(c).await;
    let (addr, fp) = (srv.addr(), srv.fingerprint().to_string());
    let mut idle = Vec::new();
    for _ in 0..3 {
        idle.push(tls_connect(addr, &fp).await.expect("within the cap"));
    }
    assert!(
        tls_connect(addr, &fp).await.is_err(),
        "a fourth connection from this address is refused"
    );
    assert!(srv.status().details.unwrap().refused_connections >= 1);
    drop(idle.pop());
    let mut again = None;
    for _ in 0..80 {
        if let Ok(s) = tls_connect(addr, &fp).await {
            again = Some(s);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(again.is_some(), "a slot frees when a connection closes");
    srv.stop().await;
}

/// L3: idle connections without a token cannot fill the server; a viewer already connected is unaffected.
#[tokio::test]
async fn l3_unauthenticated_connections_have_their_own_cap() {
    let mut c = cfg();
    c.max_connections = 32;
    c.max_connections_per_ip = 8;
    c.max_unauthenticated = 2;
    let (srv, _bus) = start(c).await;
    let (addr, fp, t) = (srv.addr(), srv.fingerprint().to_string(), srv.token().to_string());
    let mut ws = ws_connect(addr, &fp, "", Some(&t), None).await.unwrap();
    let _ = ws.next().await; // snapshot
    let mut idle = vec![
        tls_connect(addr, &fp).await.unwrap(),
        tls_connect(addr, &fp).await.unwrap(),
    ];
    assert!(
        tls_connect(addr, &fp).await.is_err(),
        "a third connection without a token is refused although only a few of the 32 slots are used"
    );
    // The connected viewer keeps its stream.
    let msg = tokio::time::timeout(Duration::from_secs(3), ws.next()).await;
    assert!(matches!(msg, Ok(Some(Ok(_)))), "the viewer still receives data");
    assert_eq!(srv.status().details.unwrap().clients.len(), 1);
    idle.clear();
    let mut ok = false;
    for _ in 0..80 {
        if request(
            addr,
            &fp,
            "GET",
            "/api/snapshot",
            &[("Cookie", &format!("bl_t={t}"))],
            None,
        )
        .await
        .status
            == 200
        {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "new viewers can connect again once the idle connections are gone");
    srv.stop().await;
}
