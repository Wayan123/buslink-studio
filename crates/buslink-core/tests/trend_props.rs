//! Trend decimation properties T1–T5 (app-shell-desktop design §4).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use buslink_core::quality::Quality;
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;
use buslink_core::trend::{TrendInput, TrendSeries, decimate};
use std::collections::HashMap;

struct Case {
    samples: Vec<TrendInput>,
    from: i64,
    to: i64,
    buckets: usize,
    max_gap: u64,
}

fn gen_case(rng: &mut SplitMix64) -> Case {
    let n = rng.range_usize(0, 3000);
    let mut ts = rng.range_usize(0, 1000) as i64;
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        ts += rng.range_usize(0, 40) as i64;
        if rng.chance(20) {
            ts += rng.range_usize(100, 2000) as i64; // occasional hole
        }
        let quality = match rng.range_usize(0, 30) {
            0 => Quality::Timeout,
            1 => Quality::CrcError,
            2 => Quality::Exception(2),
            3 => Quality::Stale,
            4 => Quality::DecodeError,
            _ => Quality::Good,
        };
        let value = if rng.chance(5) {
            None
        } else {
            Some((rng.range_usize(0, 20_000) as f64 - 10_000.0) / 10.0)
        };
        samples.push(TrendInput {
            ts_ms: ts,
            value,
            quality,
        });
    }
    let from = rng.range_usize(0, 500) as i64;
    let to = ts - rng.range_usize(0, 500) as i64 + 1;
    Case {
        samples,
        from,
        to: to.max(from + 1),
        buckets: rng.range_usize(1, 400),
        max_gap: rng.range_usize(50, 1500) as u64,
    }
}

fn run(c: &Case) -> TrendSeries {
    decimate(&c.samples, c.from, c.to, c.buckets, c.max_gap)
}

fn in_range(c: &Case) -> Vec<&TrendInput> {
    c.samples
        .iter()
        .filter(|s| s.ts_ms >= c.from && s.ts_ms < c.to)
        .collect()
}

fn is_good(s: &TrendInput) -> bool {
    s.quality.is_good() && s.value.is_some_and(f64::is_finite)
}

/// Runs of GOOD samples as the decimator must see them.
fn runs(c: &Case) -> Vec<Vec<&TrendInput>> {
    let mut out: Vec<Vec<&TrendInput>> = Vec::new();
    let mut cur: Vec<&TrendInput> = Vec::new();
    for s in in_range(c) {
        if !is_good(s) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if let Some(prev) = cur.last()
            && (s.ts_ms - prev.ts_ms) as u64 > c.max_gap
        {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(s);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[test]
fn t1_values_are_real_good_samples() {
    check("trend T1", 500, |rng| {
        let c = gen_case(rng);
        let out = run(&c);
        let good: Vec<(i64, f64)> = in_range(&c)
            .into_iter()
            .filter(|s| is_good(s))
            .map(|s| (s.ts_ms, s.value.unwrap()))
            .collect();
        for (t, v) in out.t.iter().zip(&out.v) {
            if let Some(v) = v {
                assert!(
                    good.iter().any(|(gt, gv)| gt == t && gv == v),
                    "value {v} at {t} is not a GOOD sample"
                );
            }
        }
    });
}

#[test]
fn t2_nothing_drawn_across_a_gap() {
    check("trend T2", 500, |rng| {
        let c = gen_case(rng);
        let out = run(&c);
        let samples = in_range(&c);
        let pts: Vec<(i64, Option<f64>)> = out.t.iter().copied().zip(out.v.iter().copied()).collect();
        for w in pts.windows(2) {
            let ((a, va), (b, vb)) = (w[0], w[1]);
            if va.is_none() || vb.is_none() {
                continue;
            }
            assert!(
                !samples.iter().any(|s| s.ts_ms > a && s.ts_ms < b && !is_good(s)),
                "bad sample between {a} and {b}"
            );
            // Between two drawn points every consecutive pair of input samples is within max_gap.
            let inner: Vec<i64> = samples
                .iter()
                .filter(|s| s.ts_ms >= a && s.ts_ms <= b)
                .map(|s| s.ts_ms)
                .collect();
            for p in inner.windows(2) {
                assert!((p[1] - p[0]) as u64 <= c.max_gap, "hole {}..{} drawn over", p[0], p[1]);
            }
        }
    });
}

#[test]
fn t3_extremes_of_every_bucket_kept() {
    check("trend T3", 500, |rng| {
        let c = gen_case(rng);
        let out = run(&c);
        let drawn: Vec<(i64, f64)> = out
            .t
            .iter()
            .zip(&out.v)
            .filter_map(|(t, v)| v.map(|v| (*t, v)))
            .collect();
        let span = i128::from(c.to - c.from).max(1);
        for r in runs(&c) {
            let mut cells: HashMap<i128, (f64, f64)> = HashMap::new();
            for s in &r {
                let b = (i128::from(s.ts_ms - c.from) * c.buckets as i128 / span).clamp(0, c.buckets as i128 - 1);
                let v = s.value.unwrap();
                let e = cells.entry(b).or_insert((v, v));
                e.0 = e.0.min(v);
                e.1 = e.1.max(v);
            }
            let ts: Vec<i64> = r.iter().map(|s| s.ts_ms).collect();
            let (lo, hi) = (ts[0], ts[ts.len() - 1]);
            for (min, max) in cells.values() {
                for want in [min, max] {
                    assert!(
                        drawn.iter().any(|(t, v)| v == want && *t >= lo && *t <= hi),
                        "extreme {want} of a run/bucket missing"
                    );
                }
            }
        }
    });
}

#[test]
fn t4_t5_bounded_and_sorted() {
    check("trend T4/T5", 500, |rng| {
        let c = gen_case(rng);
        let out = run(&c);
        assert_eq!(out.t.len(), out.v.len());
        assert!(out.t.windows(2).all(|w| w[0] <= w[1]), "times not sorted");
        let nulls = out.v.iter().filter(|v| v.is_none()).count();
        assert_eq!(nulls, out.gaps.len(), "one null per gap");
        for g in &out.gaps {
            assert!(g.from_ms <= g.to_ms);
        }
        if out.decimated {
            let values = out.v.len() - nulls;
            assert!(values <= 2 * (c.buckets + runs(&c).len()), "{values} values");
        }
    });
}

#[test]
fn hundred_thousand_samples_decimate_quickly() {
    let samples: Vec<TrendInput> = (0..100_000)
        .map(|i| TrendInput {
            ts_ms: i * 1000,
            value: Some((i as f64 / 50.0).sin()),
            quality: if i % 9_973 == 0 {
                Quality::Timeout
            } else {
                Quality::Good
            },
        })
        .collect();
    let started = std::time::Instant::now();
    let out = decimate(&samples, 0, 100_000_000, 1200, 3000);
    let took = started.elapsed();
    eprintln!("decimate 100k → {} points in {took:?}", out.t.len());
    assert!(out.decimated && out.t.len() <= 2 * (1200 + 12) + 12);
    assert!(took.as_millis() < 1000, "took {took:?}");
}
