//! Historian (Req 17): WAL, NULL values for non-GOOD, append-only audit,
//! lossless raw, decimation, minute rollups, retention.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::codec::RawValue;
use buslink_core::command::audit::{AuditEvent, AuditRecord, AuditSink};
use buslink_core::historian::batch::{LogDecimator, SampleBatcher};
use buslink_core::historian::{DeviceRow, EventRow, Historian};
use buslink_core::quality::{Quality, Sample, Value};
use buslink_core::testkit::check;

const T0: i64 = 1_800_000_000_000; // a minute boundary (divisible by 60 000)

fn sample(tag: &str, seq: u64, ts: i64, q: Quality, v: Option<f64>) -> Sample {
    Sample {
        seq,
        link_id: "bus1".into(),
        device_id: "soil".into(),
        tag_id: tag.into(),
        profile_id: "bgt.smps-8in1".into(),
        profile_version: "1.0.0".into(),
        ts_ms: ts,
        quality: q,
        value: v.map(|x| Value {
            raw: RawValue::I16((x * 10.0) as i16),
            engineering: Some(x),
            text: None,
        }),
        value_ts_ms: v.map(|_| ts),
        age_ms: Some(0),
        ts_source_ms: None,
        raw: Some(vec![(v.unwrap_or(0.0) * 10.0) as u16]),
        unit: Some("°C".into()),
        error: None,
    }
}

fn hist() -> Historian {
    let mut h = Historian::open_in_memory().unwrap();
    h.begin_session("s1", T0, "0.1.0", "test", 1000).unwrap();
    h.register_device(&DeviceRow {
        session: "s1".into(),
        device_id: "soil".into(),
        name: "Soil sensor".into(),
        profile_id: "bgt.smps-8in1".into(),
        profile_version: "1.0.0".into(),
        unit_id: 1,
        link: "bus1".into(),
    })
    .unwrap();
    h
}

#[test]
fn file_database_uses_wal() {
    let dir = std::env::temp_dir().join(format!("buslink-hist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.sqlite3");
    {
        let h = Historian::open(&path).unwrap();
        assert_eq!(h.journal_mode().unwrap().to_lowercase(), "wal");
    }
    // Re-opening keeps the data and schema.
    let mut h = Historian::open(&path).unwrap();
    h.begin_session("s2", T0, "0.1.0", "test", 1000).unwrap();
    assert_eq!(h.sessions().unwrap().len(), 1);
    drop(h);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn non_good_samples_are_stored_with_null_value_and_quality() {
    let mut h = hist();
    let mut stale = sample("temperature", 3, T0 + 2000, Quality::Stale, None);
    // A STALE sample carries the last good value for display; the historian must not store it as a reading.
    stale.value = Some(Value {
        raw: RawValue::I16(253),
        engineering: Some(25.3),
        text: None,
    });
    stale.value_ts_ms = Some(T0);
    let rows = vec![
        sample("temperature", 1, T0, Quality::Good, Some(25.3)),
        sample("temperature", 2, T0 + 1000, Quality::Timeout, None),
        stale,
        sample("temperature", 4, T0 + 3000, Quality::Exception(2), None),
        sample("temperature", 5, T0 + 4000, Quality::Good, Some(25.4)),
    ];
    assert_eq!(h.insert_samples(&rows).unwrap(), 5);
    let got = h
        .samples("s1", Some("soil"), Some("temperature"), T0, T0 + 10_000)
        .unwrap();
    let q: Vec<(Quality, Option<f64>)> = got.iter().map(|s| (s.quality, s.value)).collect();
    assert_eq!(
        q,
        vec![
            (Quality::Good, Some(25.3)),
            (Quality::Timeout, None),
            (Quality::Stale, None),
            (Quality::Exception(2), None),
            (Quality::Good, Some(25.4)),
        ]
    );
    assert_eq!(got[2].value_ts_ms, Some(T0), "the original value timestamp is kept");
    assert_eq!(got[0].raw_value.as_deref(), Some("253"));
    assert_eq!(
        (got[0].profile_id.as_str(), got[0].profile_version.as_str()),
        ("bgt.smps-8in1", "1.0.0")
    );
    assert!(
        h.insert_samples(&[Sample {
            device_id: "ghost".into(),
            ..rows[0].clone()
        }])
        .is_err()
    );
}

#[test]
fn raw_values_are_lossless() {
    let mut h = hist();
    let mut s = sample("energy", 1, T0, Quality::Good, None);
    s.value = Some(Value {
        raw: RawValue::U64(u64::MAX - 1),
        engineering: Some((u64::MAX - 1) as f64),
        text: None,
    });
    s.raw = Some(vec![0xFFFF, 0xFFFF, 0xFFFF, 0xFFFE]);
    h.insert_samples(&[s]).unwrap();
    let got = h.samples("s1", None, Some("energy"), T0, T0 + 1).unwrap();
    assert_eq!(got[0].raw_value.as_deref(), Some("18446744073709551614"));
    assert_eq!(got[0].raw, vec![0xFFFF, 0xFFFF, 0xFFFF, 0xFFFE]);
}

#[test]
fn command_audit_is_append_only() {
    let mut h = hist();
    let rec = AuditRecord {
        ts_ms: T0,
        session_id: "s1".into(),
        user: "tech".into(),
        role: "operator".into(),
        event: Some(AuditEvent::Result),
        device_id: "vfd".into(),
        profile_id: "sim.vfd-basic".into(),
        profile_version: "1.0.0".into(),
        unit_id: Some(2),
        function: Some(6),
        address: Some(1),
        old_value: Some(0.0),
        requested_value: Some(30.0),
        request_hex: Some("02 06 00 01 0B B8 D8 BF".into()),
        response_hex: Some("02 06 00 01 0B B8 D8 BF".into()),
        readback: Some("verified".into()),
        ..AuditRecord::default()
    };
    AuditSink::append(&mut h, &rec).unwrap();
    let rows = h.audit("s1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].event.as_deref(), rows[0].readback.as_deref()),
        (Some("result"), Some("verified"))
    );
    assert!(h.execute_raw("UPDATE command_audit SET readback = 'mismatch'").is_err());
    assert!(h.execute_raw("DELETE FROM command_audit").is_err());
    assert_eq!(h.audit("s1").unwrap()[0].readback.as_deref(), Some("verified"));
    // Retention never touches the audit.
    h.delete_before(T0 + 10_000_000).unwrap();
    assert_eq!(h.audit("s1").unwrap().len(), 1);
}

#[test]
fn decimator_keeps_quality_changes_and_logging_interval() {
    let mut d = LogDecimator::new(5000);
    let seq = [
        (0, Quality::Good, true),
        (1000, Quality::Good, false),
        (2000, Quality::Timeout, true),
        (3000, Quality::Timeout, false),
        (3500, Quality::Good, true),
        (8000, Quality::Good, false),
        (8500, Quality::Good, true),
    ];
    for (dt, q, keep) in seq {
        assert_eq!(
            d.keep(&sample("t", 0, T0 + dt, q, q.is_good().then_some(1.0))),
            keep,
            "at {dt}"
        );
    }
}

#[test]
fn batcher_groups_by_time_and_size() {
    let mut b = SampleBatcher::new(1000, 3);
    assert!(b.push(sample("t", 1, T0, Quality::Good, Some(1.0)), T0).is_none());
    assert!(b.push(sample("t", 2, T0, Quality::Good, Some(1.0)), T0 + 500).is_none());
    assert_eq!(
        b.push(sample("t", 3, T0, Quality::Good, Some(1.0)), T0 + 600)
            .unwrap()
            .len(),
        3
    );
    assert!(b.push(sample("t", 4, T0, Quality::Good, Some(1.0)), T0 + 700).is_none());
    assert_eq!(b.poll(T0 + 1700).unwrap().len(), 1);
    assert!(b.is_empty());
}

#[test]
fn minute_rollups_use_good_samples_only() {
    let mut h = hist();
    let mut rows = Vec::new();
    for i in 0..180i64 {
        let ts = T0 + i * 1000;
        let q = if (60..90).contains(&i) {
            Quality::Timeout
        } else {
            Quality::Good
        };
        rows.push(sample(
            "temperature",
            i as u64,
            ts,
            q,
            q.is_good().then_some(20.0 + (i % 60) as f64 / 10.0),
        ));
    }
    h.insert_samples(&rows).unwrap();
    // Only complete minutes before the cut-off are rolled up.
    h.rollup_until(T0 + 150_000).unwrap();
    let r = h.rollups("s1", "soil", "temperature", T0, T0 + 600_000).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].n, r[0].min, r[0].max), (60, 20.0, 25.9));
    assert_eq!(r[1].n, 30, "the TIMEOUT half-minute is excluded");
    assert_eq!(r[1].minute_start_ms, T0 + 60_000);
    assert!((r[0].mean() - 22.95).abs() < 1e-9);
    assert_eq!(h.rollup_until(T0 + 150_000).unwrap(), 0, "idempotent");
    h.rollup_until(T0 + 180_000).unwrap();
    assert_eq!(
        h.rollups("s1", "soil", "temperature", T0, T0 + 600_000).unwrap().len(),
        3
    );
}

#[test]
fn rollup_matches_naive_computation() {
    check("rollup_vs_naive", 60, |r| {
        let mut h = hist();
        let n = r.range_usize(1, 400);
        let mut rows = Vec::new();
        let mut ts = T0;
        for i in 0..n {
            ts += r.range_u64(1, 3000) as i64;
            let good = r.chance(800);
            let v = (r.range_u64(0, 1000) as f64) / 10.0;
            rows.push(sample(
                "t",
                i as u64,
                ts,
                if good { Quality::Good } else { Quality::CrcError },
                good.then_some(v),
            ));
        }
        h.insert_samples(&rows).unwrap();
        let until = ts + 60_000;
        h.rollup_until(until).unwrap();
        let got = h.rollups("s1", "soil", "t", T0, until).unwrap();
        let mut naive: std::collections::BTreeMap<i64, (u64, f64, f64)> = Default::default();
        for s in rows.iter().filter(|s| s.quality.is_good()) {
            let v = s.value.as_ref().unwrap().engineering.unwrap();
            let e = naive
                .entry(s.ts_ms.div_euclid(60_000))
                .or_insert((0, f64::MAX, f64::MIN));
            *e = (e.0 + 1, e.1.min(v), e.2.max(v));
        }
        assert_eq!(got.len(), naive.len());
        for (g, (m, (cnt, mn, mx))) in got.iter().zip(naive) {
            assert_eq!((g.minute_start_ms, g.n, g.min, g.max), (m * 60_000, cnt, mn, mx));
        }
    });
}

#[test]
fn events_and_retention() {
    let mut h = hist();
    h.insert_samples(&[
        sample("t", 1, T0, Quality::Good, Some(1.0)),
        sample("t", 2, T0 + 120_000, Quality::Good, Some(2.0)),
    ])
    .unwrap();
    h.add_event(&EventRow {
        session: Some("s1".into()),
        ts_ms: T0 + 5,
        kind: "comm".into(),
        device: Some("soil".into()),
        tag: None,
        severity: "warning".into(),
        text: "TIMEOUT".into(),
    })
    .unwrap();
    assert_eq!(h.events("s1").unwrap().len(), 1);
    h.delete_before(T0 + 60_000).unwrap();
    assert_eq!(h.samples("s1", None, None, 0, i64::MAX).unwrap().len(), 1);
    assert!(h.events("s1").unwrap().is_empty());
}
