//! MQTT telemetry against an in-process MQTT 5 broker (lan-mqtt Req 3): M4, M5, M6,
//! plus throttle, expiry, overflow, duplicates after a lost PUBACK, and TLS.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

#[path = "common/broker.rs"]
mod broker;

use broker::{Rx, TestBroker};
use buslink_core::bus::SampleBus;
use buslink_core::codec::RawValue;
use buslink_core::mqtt::{MqttConfig, MqttInit, MqttService, MqttState, Secret, test_connection};
use buslink_core::quality::{Quality, Sample, Value};
use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::mqttbytes::v5::ConnectReturnCode;
use std::time::Duration;

const STATUS: &str = "buslink/plant/gw1/status";
const TOPIC: &str = "buslink/plant/gw1/dev1/t1";

fn cfg(b: &TestBroker) -> MqttConfig {
    MqttConfig {
        broker_url: format!("mqtt://127.0.0.1:{}", b.addr.port()),
        allow_insecure: true,
        site: "plant".into(),
        gateway: "gw1".into(),
        client_id: "test-client".into(),
        min_interval_ms: 0,
        ..MqttConfig::default()
    }
}

fn sample(seq: u64, q: Quality) -> Sample {
    let good = q.is_good();
    let ts = 1_800_000_000_000 + seq as i64 * 1000;
    Sample {
        seq,
        link_id: "bus1".into(),
        device_id: "dev1".into(),
        tag_id: "t1".into(),
        profile_id: "p".into(),
        profile_version: "1.4.0".into(),
        ts_ms: ts,
        quality: q,
        value: Some(Value {
            raw: RawValue::F64(seq as f64),
            engineering: Some(seq as f64 / 2.0),
            text: None,
        }),
        value_ts_ms: Some(if good { ts } else { ts - 4000 }),
        age_ms: Some(if good { 0 } else { 4000 }),
        ts_source_ms: None,
        raw: Some(vec![1]),
        unit: Some("°C".into()),
        error: None,
    }
}

async fn until<F: FnMut() -> bool>(what: &str, secs: u64, mut f: F) {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(tokio::time::Instant::now() < end, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn start(c: MqttConfig, bus: &SampleBus, password: Option<&str>) -> MqttService {
    MqttService::start(
        MqttInit {
            cfg: c,
            password: password.map(Secret::new),
            session_id: "sess-1".into(),
            outbox_path: None,
            tls: None,
        },
        bus.subscribe(),
    )
    .await
    .unwrap()
}

fn publish_range(bus: &SampleBus, from: u64, to: u64) {
    for s in from..=to {
        bus.publish(sample(s, Quality::Good));
    }
}

/// M4 + M5: disconnect, the outbox fills, replay in order with original ts/seq, no duplicates, no SUBSCRIBE.
#[tokio::test]
async fn m4_m5_outage_fills_outbox_and_replays_in_order() {
    let broker = TestBroker::start().await;
    let bus = SampleBus::default();
    let svc = start(cfg(&broker), &bus, None).await;

    until("online", 5, || svc.status().state == MqttState::Online).await;
    publish_range(&bus, 1, 20);
    until("first 20 acknowledged", 5, || svc.status().published == 20).await;
    assert_eq!(svc.status().outbox_depth, 0);
    assert_eq!(broker.data_seqs(), (1..=20).collect::<Vec<_>>());

    // Wi-Fi drops.
    broker.cut();
    until("link lost", 5, || svc.status().state != MqttState::Online).await;
    assert_eq!(
        broker.retained(STATUS).as_deref(),
        Some("offline"),
        "the will marks the gateway offline"
    );
    publish_range(&bus, 21, 60);
    until("outbox holds the outage", 5, || svc.status().outbox_depth == 40).await;
    assert_eq!(broker.data_seqs().len(), 20, "nothing is sent while the link is down");
    assert!(svc.status().last_error.is_some(), "the outage is visible as an error");

    // Wi-Fi returns.
    broker.restore();
    until("replay acknowledged", 15, || {
        svc.status().published == 60 && svc.status().outbox_depth == 0
    })
    .await;

    let data = broker.data();
    let seqs: Vec<u64> = data.iter().map(|(_, v, _)| v["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=60).collect::<Vec<_>>(), "every sample once, in seq order");
    for (i, (topic, v, rx)) in data.iter().enumerate() {
        let seq = i as u64 + 1;
        assert_eq!(topic, TOPIC);
        assert_eq!(
            v["ts"].as_i64(),
            Some(1_800_000_000_000 + seq as i64 * 1000),
            "original ts, not the replay time"
        );
        assert_eq!(v["value"].as_f64(), Some(seq as f64 / 2.0));
        assert_eq!(
            (v["unit"].as_str(), v["quality"].as_str(), v["profile_version"].as_str()),
            (Some("°C"), Some("GOOD"), Some("1.4.0"))
        );
        assert_eq!(v["session"], "sess-1");
        let Rx::Publish {
            qos,
            retain,
            dup,
            expiry,
            ..
        } = rx
        else {
            panic!()
        };
        assert_eq!((*qos, *retain, *dup), (QoS::AtLeastOnce, false, false));
        assert!(
            expiry.is_some_and(|e| (1..=3600).contains(&e)),
            "message expiry is set: {expiry:?}"
        );
    }

    // M5: will, retained online, never a subscription.
    let log = broker.log();
    let connects: Vec<_> = log.iter().filter(|r| matches!(r, Rx::Connect { .. })).collect();
    assert_eq!(connects.len(), 2, "one connect before and one after the outage");
    for c in &connects {
        let Rx::Connect {
            will,
            clean_start,
            client_id,
            ..
        } = c
        else {
            panic!()
        };
        let w = will.as_ref().expect("will message is registered");
        assert_eq!(
            (w.topic.as_str(), w.payload.as_slice(), w.retain, w.qos),
            (STATUS, &b"offline"[..], true, QoS::AtLeastOnce)
        );
        assert!(*clean_start);
        assert_eq!(client_id, "test-client");
    }
    assert_eq!(broker.status_publishes(), vec!["online", "online"]);
    assert!(
        log.iter()
            .any(|r| matches!(r, Rx::Publish { topic, retain: true, qos: QoS::AtLeastOnce, .. } if topic == STATUS))
    );
    assert!(
        !log.iter().any(|r| matches!(r, Rx::Subscribe | Rx::Unsubscribe)),
        "telemetry only: no subscriptions"
    );
    assert_eq!(broker.retained(STATUS).as_deref(), Some("online"));
    assert!(svc.status().reconnects >= 1);

    // A clean stop says offline itself and disconnects without firing the will.
    svc.stop().await;
    assert_eq!(broker.retained(STATUS).as_deref(), Some("offline"));
    until("broker saw DISCONNECT", 5, || {
        broker.log().iter().any(|r| matches!(r, Rx::Disconnect))
    })
    .await;
    let log = broker.log();
    let fired = log.iter().filter(|r| matches!(r, Rx::WillFired { .. })).count();
    assert_eq!(fired, 1, "only the outage fired a will, the clean stop did not");
    assert_eq!(broker.status_publishes().last().map(String::as_str), Some("offline"));
}

/// A lost PUBACK is the only way a message arrives twice; order stays, and `(session, seq)` dedupes it.
#[tokio::test]
async fn unacknowledged_messages_are_resent_and_dedupe_on_seq() {
    let broker = TestBroker::start().await;
    let bus = SampleBus::default();
    let svc = start(cfg(&broker), &bus, None).await;
    until("online", 5, || svc.status().state == MqttState::Online).await;

    broker.set_withhold_acks(true);
    publish_range(&bus, 1, 3);
    until("3 sent, none acknowledged", 5, || broker.data_seqs().len() == 3).await;
    assert_eq!(svc.status().outbox_depth, 3, "rows stay until the PUBACK");
    broker.cut();
    until("link lost", 5, || svc.status().state != MqttState::Online).await;
    publish_range(&bus, 4, 5);
    broker.set_withhold_acks(false);
    broker.restore();
    until("all acknowledged", 15, || svc.status().outbox_depth == 0).await;

    let seqs = broker.data_seqs();
    assert_eq!(
        seqs,
        vec![1, 2, 3, 1, 2, 3, 4, 5],
        "only the unacknowledged three are repeated"
    );
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<u64> = seqs.into_iter().filter(|s| seen.insert(*s)).collect();
    assert_eq!(deduped, vec![1, 2, 3, 4, 5]);
    svc.stop().await;
}

#[tokio::test]
async fn failed_reads_carry_quality_age_and_the_last_good_value() {
    let broker = TestBroker::start().await;
    let bus = SampleBus::default();
    let svc = start(cfg(&broker), &bus, None).await;
    until("online", 5, || svc.status().state == MqttState::Online).await;
    bus.publish(sample(1, Quality::Good));
    bus.publish(sample(2, Quality::Timeout));
    bus.publish(sample(3, Quality::Exception(2)));
    until("3 published", 5, || svc.status().published == 3).await;
    let d = broker.data();
    assert_eq!(d[1].1["quality"], "TIMEOUT");
    assert_eq!(d[1].1["value"].as_f64(), Some(1.0), "last good value, never 0");
    assert_eq!(d[1].1["age_ms"], 4000);
    assert_eq!(d[2].1["quality"], "EXCEPTION");
    assert_eq!(d[2].1["exception"], 2);
    svc.stop().await;
}

#[tokio::test]
async fn per_tag_interval_throttles_but_quality_changes_pass() {
    let broker = TestBroker::start().await;
    let bus = SampleBus::default();
    let mut c = cfg(&broker);
    c.min_interval_ms = 5_000;
    let svc = start(c, &bus, None).await;
    until("online", 5, || svc.status().state == MqttState::Online).await;
    // seq 1..=4 are 1 s apart: only the first passes; the TIMEOUT is a change; the next TIMEOUT is throttled.
    for s in 1..=4 {
        bus.publish(sample(s, Quality::Good));
    }
    bus.publish(sample(5, Quality::Timeout));
    bus.publish(sample(6, Quality::Timeout));
    until("2 published", 5, || svc.status().published == 2).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(broker.data_seqs(), vec![1, 5]);
    assert_eq!(svc.status().throttled, 4);
    svc.stop().await;
}

#[tokio::test]
async fn expired_messages_are_dropped_and_counted_not_sent() {
    let broker = TestBroker::start().await;
    broker.cut();
    let bus = SampleBus::default();
    let mut c = cfg(&broker);
    c.expiry_s = 1;
    let svc = start(c, &bus, None).await;
    publish_range(&bus, 1, 3);
    until("queued", 5, || svc.status().outbox_depth == 3).await;
    tokio::time::sleep(Duration::from_millis(1300)).await;
    broker.restore();
    // The three old rows are past their expiry when the link returns: dropped, counted, never sent.
    until("online", 15, || svc.status().state == MqttState::Online).await;
    until("expired", 5, || svc.status().expired == 3).await;
    publish_range(&bus, 4, 4); // fresh
    until("fresh one acknowledged", 5, || svc.status().published == 1).await;
    assert_eq!(broker.data_seqs(), vec![4]);
    assert_eq!(svc.status().outbox_depth, 0);
    svc.stop().await;
}

#[tokio::test]
async fn full_outbox_drops_the_oldest_and_counts() {
    let broker = TestBroker::start().await;
    broker.cut();
    let bus = SampleBus::default();
    let mut c = cfg(&broker);
    c.outbox_max = 100;
    let svc = start(c, &bus, None).await;
    publish_range(&bus, 1, 150);
    until("all queued", 5, || svc.status().enqueued == 150).await;
    let s = svc.status();
    assert_eq!((s.outbox_depth, s.dropped_overflow, s.outbox_max), (100, 50, 100));
    broker.restore();
    until("replayed", 15, || svc.status().outbox_depth == 0).await;
    assert_eq!(
        broker.data_seqs(),
        (51..=150).collect::<Vec<_>>(),
        "the gap is at the start, order kept"
    );
    svc.stop().await;
}

/// M6: the password reaches the broker in CONNECT and nowhere else.
#[tokio::test]
async fn m6_password_only_in_connect() {
    const PW: &str = "s3cret-pw-123";
    let broker = TestBroker::start().await;
    let bus = SampleBus::default();
    let mut c = cfg(&broker);
    c.username = "alice".into();
    let svc = start(c.clone(), &bus, Some(PW)).await;
    until("online", 5, || svc.status().state == MqttState::Online).await;
    publish_range(&bus, 1, 3);
    until("published", 5, || svc.status().published == 3).await;
    let Some(Rx::Connect { username, password, .. }) =
        broker.log().into_iter().find(|r| matches!(r, Rx::Connect { .. }))
    else {
        panic!()
    };
    assert_eq!((username.as_deref(), password.as_deref()), (Some("alice"), Some(PW)));
    for (_, _, rx) in broker.data() {
        let Rx::Publish { payload, topic, .. } = rx else {
            panic!()
        };
        assert!(!String::from_utf8_lossy(&payload).contains(PW) && !topic.contains(PW));
    }
    let status_json = serde_json::to_string(&svc.status()).unwrap();
    assert!(!status_json.contains(PW) && !format!("{:?}", svc.status()).contains(PW));
    assert!(!serde_json::to_string(&c).unwrap().contains(PW) && !format!("{c:?}").contains(PW));
    assert!(!format!("{:?}", Secret::new(PW)).contains(PW) && !Secret::new(PW).to_string().contains(PW));
    svc.stop().await;

    // A refused login produces an error text without the password, for the service and for Test connection.
    broker.refuse_with(Some(ConnectReturnCode::NotAuthorized));
    let svc = start(c.clone(), &bus, Some(PW)).await;
    until("error shown", 5, || svc.status().last_error.is_some()).await;
    let err = svc.status().last_error.unwrap();
    assert!(err.contains("NotAuthorized") && !err.contains(PW), "{err}");
    svc.stop().await;
    let report = test_connection(&c, Some(&Secret::new(PW)), None).await;
    assert!(!report.ok && !report.detail.contains(PW), "{}", report.detail);
}

#[tokio::test]
async fn test_connection_uses_its_own_client_id_and_never_publishes() {
    let broker = TestBroker::start().await;
    let c = cfg(&broker);
    let r = test_connection(&c, None, None).await;
    assert!(r.ok, "{}", r.detail);
    let log = broker.log();
    let Some(Rx::Connect { client_id, will, .. }) = log.iter().find(|r| matches!(r, Rx::Connect { .. })).cloned()
    else {
        panic!()
    };
    assert!(client_id.starts_with("test-client-test-") && client_id != "test-client");
    assert!(will.is_none());
    assert!(log.iter().all(|r| !matches!(r, Rx::Publish { .. } | Rx::Subscribe)));
    // Unreachable broker: an error, not a hang.
    broker.cut();
    let r = test_connection(&c, None, None).await;
    assert!(!r.ok && !r.detail.is_empty());
    // Invalid settings are reported in text.
    let r = test_connection(
        &MqttConfig {
            broker_url: "mqtt://x".into(),
            allow_insecure: false,
            ..c
        },
        None,
        None,
    )
    .await;
    assert!(!r.ok && r.detail.contains("unencrypted"), "{}", r.detail);
}

fn pem(der: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for chunk in der.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, b)| a | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    let lines: Vec<&str> = s
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        lines.join("\n")
    )
}

/// TLS: the broker's certificate is checked against the roots; a user CA makes a private broker work, a wrong one fails.
#[tokio::test]
async fn tls_verifies_the_broker_certificate() {
    use buslink_core::lan::tls;
    let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let cert = tls::generate(ip, now).unwrap();
    let other = tls::generate(ip, now).unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(tls::server_config(&cert).unwrap());
    let broker = TestBroker::start_tls(acceptor).await;
    let mut c = MqttConfig {
        broker_url: format!("mqtts://127.0.0.1:{}", broker.addr.port()),
        client_id: "tls-client".into(),
        ca_pem: pem(cert.cert.as_ref()),
        ..MqttConfig::default()
    };
    let ok = test_connection(&c, None, None).await;
    assert!(ok.ok, "{}", ok.detail);
    assert!(ok.detail.contains("TLS"));

    c.ca_pem = pem(other.cert.as_ref());
    let bad = test_connection(&c, None, None).await;
    assert!(!bad.ok, "a certificate from another CA must be refused");

    c.ca_pem = String::new();
    let none = test_connection(&c, None, None).await;
    assert!(
        !none.ok,
        "a self-signed broker is not trusted without its CA: {}",
        none.detail
    );
}
