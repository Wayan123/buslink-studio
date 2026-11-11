//! Session service against the training simulator (app-shell-desktop design §10, S1–S6).
//! Uses the real clock with 100 ms poll intervals; each test runs a few seconds.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use buslink_core::bus::BusRecv;
use buslink_core::clock::{Clock, SystemClock};
use buslink_core::device::{DeviceError, DeviceInstance, LinkConfig, LinkKind};
use buslink_core::session::{ExportFormat, LinkState, Session, SessionConfig, SessionError, TransportFactory};
use buslink_core::transport::TransportError;
use buslink_sim::training::{TRAINING_DEVICES, Training, TrainingFault, alarm_presets};
use common::bundled;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FILES: [&str; 3] = ["bgt-smps-8in1.json", "sim-vfd.json", "sim-temp-controller.json"];

struct Rig {
    session: Session,
    training: Arc<Training>,
    ids: Vec<String>,
}

fn rig(seed: u64) -> Rig {
    let clock: Arc<dyn Clock> = SystemClock::shared();
    let training = Arc::new(Training::new(seed, Arc::clone(&clock)));
    let t = Arc::clone(&training);
    let factory: TransportFactory = Arc::new(move |cfg: &LinkConfig| match cfg.kind {
        LinkKind::Simulator { .. } => Ok(t.transport()),
        _ => Err(TransportError::Io("no hardware in tests".into())),
    });
    let session = Session::start(
        SessionConfig {
            db_path: None,
            app_version: "test".into(),
            platform: "test".into(),
            logging_interval_ms: 0,
        },
        clock,
        factory,
    )
    .unwrap();
    for f in FILES {
        session.add_profile(bundled(f));
    }
    let link = Training::link_config();
    session.connect(link.clone(), true).unwrap();
    let mut ids = Vec::new();
    for (unit, profile_id) in TRAINING_DEVICES {
        let p = session
            .profiles()
            .into_iter()
            .find(|p| p.profile_id == profile_id)
            .unwrap();
        let id = session.suggest_device_id(&p.profile_id, unit);
        let mut d = DeviceInstance::from_profile(&id, &id, &link, &p);
        assert_eq!(d.unit_id, unit, "profile default unit matches the training bus");
        d.poll_interval_ms = 100;
        d.response_timeout_ms = Some(150);
        session.add_device(d).unwrap();
        ids.push(id);
    }
    Rig { session, training, ids }
}

fn wait_for(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

fn all_good(s: &Session, device: &str) -> bool {
    let tags: Vec<_> = s
        .snapshot()
        .tags
        .into_iter()
        .filter(|t| t.device_id == device)
        .collect();
    !tags.is_empty() && tags.iter().all(|t| t.quality == "GOOD")
}

/// S1 + S5 + S6: the training journey with an injected timeout.
#[test]
fn s1_training_journey_with_timeout_gap() {
    let r = rig(11);
    let s = &r.session;
    let mut sub = s.subscribe();
    let (soil, vfd, temp) = (&r.ids[0], &r.ids[1], &r.ids[2]);
    for a in alarm_presets(soil, vfd, temp) {
        s.alarm_add(a).unwrap();
    }
    s.start_polling("training").unwrap();
    wait_for("three devices GOOD", 5, || r.ids.iter().all(|d| all_good(s, d)));

    let mut points = 0u64;
    let mut dropped = 0u64;
    let mut take = |s: &Session| {
        let f = s.take_frame();
        points += f.points.len() as u64;
        dropped += f.dropped;
        f
    };
    take(s);

    r.training.inject(3, TrainingFault::Timeout, 1_500).unwrap();
    assert_eq!(r.training.active().len(), 1);
    let mut saw_timeout = false;
    wait_for("temperature controller TIMEOUT", 5, || {
        let f = take(s);
        saw_timeout |= f.tags.iter().any(|t| t.device_id == *temp && t.quality == "TIMEOUT");
        saw_timeout
    });
    // The comm alarm is separate and the process alarm is held, not raised.
    let alarms = s.alarms();
    assert!(alarms.alarms.iter().any(|a| a.id == format!("comm:{temp}") && a.active));
    wait_for("temperature controller GOOD again", 8, || {
        take(s);
        all_good(s, temp)
    });
    assert!(all_good(s, soil), "other devices kept polling");

    let now = SystemClock::new().utc_ms();
    let series = s.trend(temp, "pv", now - 60_000, now + 1_000, 300).unwrap();
    assert!(series.v.iter().any(Option::is_none), "gap in the trend");
    assert!(
        series.gaps.iter().any(|g| g.reason == "TIMEOUT"),
        "gaps: {:?}",
        series.gaps
    );

    let csv = String::from_utf8(s.export(ExportFormat::Csv).unwrap()).unwrap();
    assert!(csv.lines().any(|l| l.contains(temp.as_str()) && l.contains("TIMEOUT")));
    assert!(csv.lines().any(|l| l.contains(soil.as_str()) && l.contains(",GOOD,")));
    let xlsx = s.export(ExportFormat::Xlsx).unwrap();
    assert_eq!(&xlsx[..2], b"PK");
    let json = String::from_utf8(s.export(ExportFormat::Json).unwrap()).unwrap();
    assert!(json.contains("\"TIMEOUT\""));

    let insp = s.inspect(temp).unwrap();
    assert!(insp.frames.iter().any(|f| f.outcome == "TIMEOUT"));
    assert!(
        insp.frames
            .iter()
            .any(|f| f.outcome == "OK" && f.request_hex.starts_with("03 03"))
    );
    let pv = insp.rows.iter().find(|r| r.tag_id == "pv").unwrap();
    assert_eq!((pv.table, pv.pdu_offset, pv.function), ("holding_register", 0, 3));
    assert!(pv.raw_hex.is_some() && pv.decoded.is_some());
    let meter_like = s.inspect(soil).unwrap();
    assert!(
        meter_like.rows.iter().all(|r| r.orders.is_empty()),
        "16-bit tags have no order table"
    );
    assert!(insp.device.counters.timeouts > 0);

    // S5: sequence numbers seen on the bus are strictly increasing.
    s.shutdown();
    let mut last = 0u64;
    let mut seen = 0u64;
    for m in sub.drain() {
        if let BusRecv::Sample(x) = m {
            assert!(x.seq > last, "seq {} after {last}", x.seq);
            last = x.seq;
            seen += 1;
        }
    }
    // S6: every sample reached the UI as a point or was counted as dropped.
    take(s);
    assert_eq!(points + dropped, s.applied_samples());
    assert!(seen > 0 && s.applied_samples() >= seen);
    let events = s.events(Some("alarm"), 100).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.text.starts_with("activate") && e.text.contains("TIMEOUT"))
    );
}

/// S2: one master per bus.
#[test]
fn s2_second_master_refused() {
    let r = rig(12);
    let mut other = Training::link_config();
    other.id = "training-2".into();
    match r.session.connect(other.clone(), true) {
        Err(SessionError::BusOwned(k)) => assert_eq!(k, "sim:training"),
        x => panic!("expected BusOwned, got {x:?}"),
    }
    assert!(matches!(
        r.session.connect(Training::link_config(), true),
        Err(SessionError::LinkExists(_))
    ));
    r.session.disconnect("training").unwrap();
    r.session.connect(other, true).unwrap();
}

/// S3: unit 0 is broadcast and never a device; duplicates are refused.
#[test]
fn s3_unit_zero_and_duplicates_refused() {
    let r = rig(13);
    let link = Training::link_config();
    let p = bundled("sim-energy-meter.json");
    r.session.add_profile(p.clone());
    let mut d = DeviceInstance::from_profile("meter", "meter", &link, &p);
    d.unit_id = 0;
    assert_eq!(
        r.session.add_device(d.clone()),
        Err(SessionError::Device(DeviceError::UnitZero))
    );
    d.unit_id = 1;
    assert_eq!(r.session.add_device(d.clone()), Err(SessionError::UnitTaken(1)));
    d.unit_id = 4;
    d.id = r.ids[0].clone();
    assert!(matches!(
        r.session.add_device(d.clone()),
        Err(SessionError::DeviceExists(_))
    ));
    d.id = "meter".into();
    r.session.add_device(d).unwrap();
    assert!(matches!(
        r.session.connect(
            LinkConfig {
                id: "Bad Id".into(),
                ..Training::link_config()
            },
            true
        ),
        Err(SessionError::Settings(_))
    ));
}

/// S4: disconnect marks every tag DISCONNECTED with its old value time; nothing is polled after.
#[test]
fn s4_disconnect_keeps_last_value_and_time() {
    let r = rig(14);
    let s = &r.session;
    s.start_polling("training").unwrap();
    wait_for("GOOD", 5, || r.ids.iter().all(|d| all_good(s, d)));
    let before = s.snapshot().tags;
    s.disconnect("training").unwrap();
    wait_for("DISCONNECTED", 3, || {
        s.snapshot().tags.iter().all(|t| t.quality == "DISCONNECTED")
    });
    let after = s.snapshot().tags;
    for t in &after {
        let b = before.iter().find(|x| x.key == t.key).unwrap();
        assert!(t.value_ts_ms.is_some() && t.value_ts_ms >= b.value_ts_ms);
        assert_eq!(t.value.is_some(), b.value.is_some(), "last value kept");
    }
    let frozen: Vec<_> = after.iter().map(|t| (t.key.clone(), t.value_ts_ms)).collect();
    std::thread::sleep(Duration::from_millis(500));
    let later: Vec<_> = s.snapshot().tags.into_iter().map(|t| (t.key, t.value_ts_ms)).collect();
    assert_eq!(frozen, later, "no new samples after disconnect");
    let link = s.links().into_iter().find(|l| l.id == "training").unwrap();
    assert_eq!(link.state, LinkState::Disconnected);
    assert!(s.start_polling("training").is_err(), "disconnected link cannot poll");
    s.reconnect("training").unwrap();
    let link = s.links().into_iter().find(|l| l.id == "training").unwrap();
    assert_eq!(
        link.state,
        LinkState::Connected,
        "reconnect comes back in Monitoring, not polling"
    );
}

#[test]
fn training_faults_are_bounded_and_clearable() {
    let r = rig(15);
    assert!(r.training.inject(3, TrainingFault::Stale, 500).is_err(), "too short");
    assert!(r.training.inject(3, TrainingFault::Stale, 700_000).is_err(), "too long");
    assert!(
        r.training.inject(99, TrainingFault::Timeout, 5_000).is_err(),
        "no such unit"
    );
    r.training.inject(2, TrainingFault::WrongOrder, 5_000).unwrap();
    r.training.inject(2, TrainingFault::CrcError, 5_000).unwrap();
    assert_eq!(r.training.active().len(), 2);
    r.training.clear(2);
    assert!(r.training.active().is_empty());
    // Profiles are untouched by simulator use.
    assert!(
        r.session
            .profiles()
            .iter()
            .any(|p| p.profile_id == "bgt.smps-8in1" && format!("{:?}", p.support_level) == "Documented")
    );
}
