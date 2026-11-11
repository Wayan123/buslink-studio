//! Alarm engine: state transitions and properties A1–A6 (dashboard-ui design §6).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use buslink_core::alarm::{
    AlarmDef, AlarmEngine, AlarmKind, AlarmState, FLOOD_THRESHOLD, default_deadband, default_delay_ms,
};
use buslink_core::codec::RawValue;
use buslink_core::quality::{Quality, Sample, Value};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;

fn sample(device: &str, tag: &str, q: Quality, v: f64, ts: i64) -> Sample {
    Sample {
        seq: 0,
        link_id: "l".into(),
        device_id: device.into(),
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
        value_ts_ms: Some(ts),
        age_ms: Some(0),
        ts_source_ms: None,
        raw: None,
        unit: Some("°C".into()),
        error: None,
    }
}

fn def(kind: AlarmKind, limit: f64, deadband: f64, on: u64, off: u64, latch: bool) -> AlarmDef {
    AlarmDef {
        id: "a1".into(),
        device_id: "d".into(),
        tag_id: "t".into(),
        kind,
        limit,
        deadband,
        on_delay_ms: on,
        off_delay_ms: off,
        priority: 2,
        latch,
        message: None,
    }
}

fn feed(e: &mut AlarmEngine, q: Quality, v: f64, ts: i64) {
    e.observe(&sample("d", "t", q, v, ts), ts);
}

#[test]
fn hi_alarm_full_cycle_with_delay_and_deadband() {
    let mut e = AlarmEngine::new();
    e.add(def(AlarmKind::Hi, 80.0, 2.0, 5_000, 1_000, false), 0).unwrap();
    feed(&mut e, Quality::Good, 81.0, 0);
    assert_eq!(e.state("a1", 0), Some(AlarmState::Normal), "on-delay pending");
    feed(&mut e, Quality::Good, 82.0, 4_999);
    assert_eq!(e.state("a1", 4_999), Some(AlarmState::Normal));
    e.tick(5_000);
    assert_eq!(e.state("a1", 5_000), Some(AlarmState::Unacked));
    e.ack("a1", 6_000).unwrap();
    assert_eq!(e.state("a1", 6_000), Some(AlarmState::Acked), "acked stays active");
    feed(&mut e, Quality::Good, 79.0, 7_000); // inside deadband: still active
    e.tick(20_000);
    assert_eq!(e.state("a1", 20_000), Some(AlarmState::Acked));
    feed(&mut e, Quality::Good, 77.0, 21_000);
    e.tick(22_000);
    assert_eq!(e.state("a1", 22_000), Some(AlarmState::Normal));
    let kinds: Vec<String> = e.take_events().into_iter().map(|ev| ev.event).collect();
    assert_eq!(kinds, vec!["activate", "ack", "clear"]);
}

#[test]
fn rtn_unacked_and_latch() {
    let mut e = AlarmEngine::new();
    e.add(def(AlarmKind::Lo, 10.0, 0.0, 0, 0, false), 0).unwrap();
    feed(&mut e, Quality::Good, 5.0, 0);
    feed(&mut e, Quality::Good, 15.0, 1);
    assert_eq!(e.state("a1", 1), Some(AlarmState::RtnUnacked));
    e.ack("a1", 2).unwrap();
    assert_eq!(e.state("a1", 2), Some(AlarmState::Normal));

    let mut l = AlarmEngine::new();
    l.add(def(AlarmKind::Lo, 10.0, 0.0, 0, 0, true), 0).unwrap();
    feed(&mut l, Quality::Good, 5.0, 0);
    feed(&mut l, Quality::Good, 15.0, 1);
    assert_eq!(l.state("a1", 1), Some(AlarmState::Unacked), "latched");
    l.ack("a1", 2).unwrap();
    assert_eq!(l.state("a1", 2), Some(AlarmState::Normal));
}

#[test]
fn shelve_expiry_and_out_of_service() {
    let mut e = AlarmEngine::new();
    e.add(def(AlarmKind::Hi, 1.0, 0.0, 0, 0, false), 0).unwrap();
    feed(&mut e, Quality::Good, 2.0, 0);
    e.shelve("a1", 10_000, 0).unwrap();
    assert_eq!(e.state("a1", 5_000), Some(AlarmState::Shelved));
    assert_eq!(e.snapshot(5_000).summary.unacked, 0);
    e.tick(10_000);
    assert_eq!(e.state("a1", 10_000), Some(AlarmState::Unacked));
    assert!(e.take_events().iter().any(|ev| ev.event == "shelve_expired"));
    assert!(e.shelve("a1", 0, 1).is_err(), "past expiry rejected");
    assert!(e.shelve("a1", 9 * 3_600_000, 0).is_err(), "longer than 8 h rejected");
    e.set_out_of_service("a1", true, 11_000).unwrap();
    assert_eq!(e.state("a1", 11_000), Some(AlarmState::OutOfService));
    assert!(e.ack("nope", 0).is_err());
}

#[test]
fn comm_alarm_is_separate_and_process_alarms_hold() {
    let mut e = AlarmEngine::new();
    e.add(def(AlarmKind::Hi, 50.0, 0.0, 0, 0, false), 0).unwrap();
    feed(&mut e, Quality::Good, 10.0, 0);
    feed(&mut e, Quality::Timeout, 999.0, 1_000);
    assert_eq!(
        e.state("a1", 1_000),
        Some(AlarmState::Suppressed),
        "held, not evaluated"
    );
    assert_eq!(e.state("comm:d", 1_000), Some(AlarmState::Unacked));
    let snap = e.snapshot(1_000);
    assert_eq!(snap.summary.comm_active, 1);
    let comm = snap.alarms.iter().find(|a| a.id == "comm:d").unwrap();
    assert!(comm.message.as_deref().unwrap().contains("TIMEOUT"));
    feed(&mut e, Quality::Good, 10.0, 2_000);
    assert_eq!(e.state("comm:d", 2_000), Some(AlarmState::RtnUnacked));
    assert_eq!(e.state("a1", 2_000), Some(AlarmState::Normal));
}

#[test]
fn flood_and_chatter() {
    let mut e = AlarmEngine::new();
    e.add(def(AlarmKind::Hi, 1.0, 0.0, 0, 0, false), 0).unwrap();
    for i in 0..=FLOOD_THRESHOLD as i64 {
        feed(&mut e, Quality::Good, 2.0, i * 1_000);
        feed(&mut e, Quality::Good, 0.0, i * 1_000 + 500);
    }
    let snap = e.snapshot(11_000);
    assert!(snap.summary.flood);
    assert!(snap.alarms[0].chattering);
    assert!(!e.snapshot(11_000 + 11 * 60_000).alarms[0].chattering);
}

#[test]
fn defaults_follow_research() {
    assert!((default_deadband(Some("°C"), 100.0) - 1.0).abs() < 1e-9);
    assert!((default_deadband(Some("bar"), 100.0) - 2.0).abs() < 1e-9);
    assert!((default_deadband(Some("m³/h"), 100.0) - 5.0).abs() < 1e-9);
    assert_eq!(default_delay_ms(Some("°C")), 60_000);
    assert_eq!(default_delay_ms(Some("%")), 15_000);
}

#[test]
fn invalid_definitions_rejected() {
    let mut e = AlarmEngine::new();
    let mut d = def(AlarmKind::Hi, 1.0, 0.0, 0, 0, false);
    d.priority = 0;
    assert!(e.add(d.clone(), 0).is_err());
    d.priority = 1;
    d.deadband = -1.0;
    assert!(e.add(d.clone(), 0).is_err());
    d.deadband = 0.0;
    d.limit = f64::NAN;
    assert!(e.add(d.clone(), 0).is_err());
    d.limit = 1.0;
    d.id = "comm:x".into();
    assert!(e.add(d, 0).is_err());
}

// ---- properties ----

struct Step {
    dt: i64,
    q: Quality,
    v: f64,
}

fn steps(rng: &mut SplitMix64, n: usize) -> Vec<Step> {
    (0..n)
        .map(|_| Step {
            dt: rng.range_usize(1, 3_000) as i64,
            q: if rng.chance(150) {
                Quality::Timeout
            } else {
                Quality::Good
            },
            v: rng.range_usize(0, 1000) as f64 / 10.0,
        })
        .collect()
}

#[test]
fn a1_short_conditions_never_activate() {
    check("alarm A1", 300, |rng| {
        let on = rng.range_usize(2_000, 20_000) as u64;
        let mut e = AlarmEngine::new();
        e.add(def(AlarmKind::Hi, 50.0, 1.0, on, 0, false), 0).unwrap();
        let mut t = 0i64;
        for _ in 0..rng.range_usize(1, 40) {
            // Above the limit for less than the on-delay, then clearly below.
            let above = rng.range_usize(0, on as usize - 1) as i64;
            feed(&mut e, Quality::Good, 60.0, t);
            t += above;
            feed(&mut e, Quality::Good, 60.0, t);
            t += 1;
            feed(&mut e, Quality::Good, 10.0, t);
            t += rng.range_usize(1, 5_000) as i64;
            e.tick(t);
            assert_eq!(e.state("a1", t), Some(AlarmState::Normal));
        }
    });
}

#[test]
fn a2_deadband_prevents_clearing() {
    check("alarm A2", 300, |rng| {
        let db = rng.range_usize(1, 50) as f64 / 10.0;
        let mut e = AlarmEngine::new();
        e.add(def(AlarmKind::Hi, 50.0, db, 0, 0, false), 0).unwrap();
        feed(&mut e, Quality::Good, 55.0, 0);
        assert!(e.view("a1", 0).unwrap().active);
        let mut t = 0;
        for _ in 0..50 {
            t += rng.range_usize(1, 2_000) as i64;
            let v = 50.0 - db + rng.unit_f64() * (db + 10.0); // never below limit - deadband
            feed(&mut e, Quality::Good, v, t);
            assert!(e.view("a1", t).unwrap().active, "cleared inside deadband at {v}");
        }
    });
}

#[test]
fn a3_bad_quality_holds_process_state() {
    check("alarm A3", 300, |rng| {
        let mut e = AlarmEngine::new();
        e.add(def(AlarmKind::Hi, 50.0, 2.0, 1_000, 1_000, rng.bool()), 0)
            .unwrap();
        let mut t = 0;
        for s in steps(rng, 200) {
            t += s.dt;
            let before = e.view("a1", t).unwrap().active;
            feed(&mut e, s.q, s.v, t);
            if !s.q.is_good() {
                assert_eq!(e.view("a1", t).unwrap().active, before, "changed while quality bad");
                // Timers do not run while held either.
                t += 5_000;
                e.tick(t);
                assert_eq!(e.view("a1", t).unwrap().active, before);
            }
        }
    });
}

#[test]
fn a4_latched_never_normal_without_ack() {
    check("alarm A4", 300, |rng| {
        let mut e = AlarmEngine::new();
        e.add(def(AlarmKind::Hi, 50.0, 2.0, 0, 0, true), 0).unwrap();
        let mut t = 0;
        let mut acked_since_activation = true;
        for s in steps(rng, 200) {
            t += s.dt;
            let was_active = e.view("a1", t).unwrap().active;
            feed(&mut e, s.q, s.v, t);
            if !was_active && e.view("a1", t).unwrap().active {
                acked_since_activation = false;
            }
            if rng.chance(100) {
                e.ack("a1", t).unwrap();
                acked_since_activation = true;
            }
            if !acked_since_activation {
                assert_ne!(e.state("a1", t), Some(AlarmState::Normal));
            }
        }
    });
}

#[test]
fn a5_shelve_hides_then_matches_twin() {
    check("alarm A5", 300, |rng| {
        let mut shelved = AlarmEngine::new();
        let mut twin = AlarmEngine::new();
        let d = def(AlarmKind::Hi, 50.0, 2.0, 500, 500, rng.bool());
        shelved.add(d.clone(), 0).unwrap();
        twin.add(d, 0).unwrap();
        let until = rng.range_usize(1_000, 100_000) as i64;
        shelved.shelve("a1", until, 0).unwrap();
        let mut t = 0;
        for s in steps(rng, 150) {
            t += s.dt;
            feed(&mut shelved, s.q, s.v, t);
            feed(&mut twin, s.q, s.v, t);
            if t < until {
                assert_eq!(shelved.state("a1", t), Some(AlarmState::Shelved));
                let snap = shelved.snapshot(t);
                let comm_annunciated = snap
                    .alarms
                    .iter()
                    .filter(|a| {
                        a.id.starts_with("comm:")
                            && matches!(
                                a.state,
                                AlarmState::Unacked | AlarmState::Acked | AlarmState::RtnUnacked
                            )
                    })
                    .count();
                assert_eq!(snap.summary.counts[1], comm_annunciated, "shelved alarm counted");
                assert_ne!(snap.summary.oldest_unacked.as_deref(), Some("a1"));
            } else {
                shelved.tick(t);
                assert_eq!(shelved.state("a1", t), twin.state("a1", t));
            }
        }
    });
}

#[test]
fn a6_one_event_per_change() {
    check("alarm A6", 300, |rng| {
        let mut e = AlarmEngine::new();
        e.add(def(AlarmKind::Lo, 30.0, 1.0, 0, 0, false), 0).unwrap();
        let mut t = 0;
        let mut active = false;
        let mut expected = 0usize;
        let mut got = 0usize;
        for s in steps(rng, 200) {
            t += s.dt;
            feed(&mut e, Quality::Good, s.v, t);
            let now = e.view("a1", t).unwrap().active;
            if now != active {
                expected += 1;
                active = now;
            }
            got += e
                .take_events()
                .iter()
                .filter(|ev| ev.alarm_id == "a1" && (ev.event == "activate" || ev.event == "clear"))
                .count();
        }
        assert_eq!(got, expected);
    });
}
