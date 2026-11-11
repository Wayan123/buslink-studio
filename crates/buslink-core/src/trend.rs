//! Trend series for display (PRD R14, research/02 §7).
//!
//! Gaps are gaps: every non-GOOD sample, and every interval without samples
//! longer than `max_gap_ms`, becomes one `None` point plus a [`Gap`] with its
//! reason. Long ranges are decimated min-max per bucket (pixel column) inside
//! each run of GOOD samples, so spikes survive and nothing is drawn across a gap.
//! LTTB or averaging is never used here.

use crate::quality::Quality;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrendInput {
    pub ts_ms: i64,
    /// Engineering value; only used when `quality` is GOOD.
    pub value: Option<f64>,
    pub quality: Quality,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Gap {
    pub from_ms: i64,
    pub to_ms: i64,
    /// Quality text of the first bad sample (`TIMEOUT`, `EXCEPTION 02`, ...) or `NO_DATA`.
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TrendSeries {
    pub t: Vec<i64>,
    pub v: Vec<Option<f64>>,
    pub gaps: Vec<Gap>,
    /// True when min-max decimation was applied.
    pub decimated: bool,
    /// Samples in the requested range before decimation.
    pub raw_count: usize,
}

pub const NO_DATA: &str = "NO_DATA";

fn good_value(s: &TrendInput) -> Option<f64> {
    if s.quality.is_good() {
        s.value.filter(|v| v.is_finite())
    } else {
        None
    }
}

/// (time, value) of one extreme.
type Extreme = (i64, f64);

struct Builder {
    from: i64,
    span: i128,
    buckets: i128,
    decimate: bool,
    out: TrendSeries,
    /// Current (bucket, min, max) inside the current run.
    cell: Option<(i128, Extreme, Extreme)>,
}

impl Builder {
    fn bucket(&self, ts: i64) -> i128 {
        let off = i128::from(ts) - i128::from(self.from);
        (off * self.buckets / self.span).clamp(0, self.buckets - 1)
    }

    fn push(&mut self, t: i64, v: Option<f64>) {
        self.out.t.push(t);
        self.out.v.push(v);
    }

    fn flush_cell(&mut self) {
        if let Some((_, lo, hi)) = self.cell.take() {
            let (a, b) = if lo.0 <= hi.0 { (lo, hi) } else { (hi, lo) };
            self.push(a.0, Some(a.1));
            if a != b {
                self.push(b.0, Some(b.1));
            }
        }
    }

    fn good(&mut self, ts: i64, v: f64) {
        if !self.decimate {
            self.push(ts, Some(v));
            return;
        }
        let b = self.bucket(ts);
        match &mut self.cell {
            Some((cb, lo, hi)) if *cb == b => {
                if v < lo.1 {
                    *lo = (ts, v);
                }
                if v > hi.1 {
                    *hi = (ts, v);
                }
            }
            _ => {
                self.flush_cell();
                self.cell = Some((b, (ts, v), (ts, v)));
            }
        }
    }

    fn gap(&mut self, null_at: i64, gap: Gap) {
        self.flush_cell();
        self.push(null_at, None);
        self.out.gaps.push(gap);
    }
}

/// Build the display series for samples in `[from_ms, to_ms)`.
/// `samples` must be in time order (the historian returns them so).
pub fn decimate(samples: &[TrendInput], from_ms: i64, to_ms: i64, buckets: usize, max_gap_ms: u64) -> TrendSeries {
    let in_range: Vec<&TrendInput> = samples
        .iter()
        .filter(|s| s.ts_ms >= from_ms && s.ts_ms < to_ms)
        .collect();
    let buckets = buckets.clamp(1, 100_000);
    let good_count = in_range.iter().filter(|s| good_value(s).is_some()).count();
    let mut b = Builder {
        from: from_ms,
        span: (i128::from(to_ms) - i128::from(from_ms)).max(1),
        buckets: buckets as i128,
        decimate: good_count > 2 * buckets,
        out: TrendSeries {
            raw_count: in_range.len(),
            ..TrendSeries::default()
        },
        cell: None,
    };
    b.out.decimated = b.decimate;
    let max_gap = i64::try_from(max_gap_ms).unwrap_or(i64::MAX);
    // Last GOOD sample time while in a run; open gap start + reason while in a gap.
    let mut last_good: Option<i64> = None;
    let mut open_gap: Option<(i64, String)> = None;
    let mut last_ts: Option<i64> = None;
    for s in in_range {
        last_ts = Some(s.ts_ms);
        match good_value(s) {
            Some(v) => {
                if let Some((start, reason)) = open_gap.take() {
                    // Close the gap at this first GOOD sample.
                    if let Some(g) = b.out.gaps.last_mut()
                        && g.from_ms == start
                    {
                        g.to_ms = s.ts_ms;
                        g.reason = reason;
                    }
                } else if let Some(prev) = last_good
                    && s.ts_ms.saturating_sub(prev) > max_gap
                {
                    let mid = prev + (s.ts_ms - prev) / 2;
                    b.gap(
                        mid,
                        Gap {
                            from_ms: prev,
                            to_ms: s.ts_ms,
                            reason: NO_DATA.into(),
                        },
                    );
                }
                b.good(s.ts_ms, v);
                last_good = Some(s.ts_ms);
            }
            None => {
                if open_gap.is_none() {
                    let reason = if s.quality.is_good() {
                        // GOOD without a usable value cannot be drawn either.
                        Quality::DecodeError.to_string()
                    } else {
                        s.quality.to_string()
                    };
                    let from = last_good.unwrap_or(s.ts_ms);
                    b.gap(
                        s.ts_ms,
                        Gap {
                            from_ms: from,
                            to_ms: s.ts_ms,
                            reason: reason.clone(),
                        },
                    );
                    open_gap = Some((from, reason));
                }
                last_good = None;
            }
        }
    }
    b.flush_cell();
    // A gap still open at the end lasts at least until the last bad sample.
    if let (Some((start, _)), Some(end)) = (open_gap, last_ts)
        && let Some(g) = b.out.gaps.last_mut()
        && g.from_ms == start
    {
        g.to_ms = g.to_ms.max(end);
    }
    b.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(ts: i64, v: f64) -> TrendInput {
        TrendInput {
            ts_ms: ts,
            value: Some(v),
            quality: Quality::Good,
        }
    }
    fn bad(ts: i64, q: Quality) -> TrendInput {
        TrendInput {
            ts_ms: ts,
            value: Some(99.0),
            quality: q,
        }
    }

    #[test]
    fn timeout_breaks_the_line_with_its_reason() {
        let s = [
            g(0, 1.0),
            g(1000, 2.0),
            bad(2000, Quality::Timeout),
            bad(3000, Quality::Timeout),
            g(4000, 3.0),
        ];
        let out = decimate(&s, 0, 10_000, 100, 5_000);
        assert_eq!(out.t, vec![0, 1000, 2000, 4000]);
        assert_eq!(out.v, vec![Some(1.0), Some(2.0), None, Some(3.0)]);
        assert_eq!(
            out.gaps,
            vec![Gap {
                from_ms: 1000,
                to_ms: 4000,
                reason: "TIMEOUT".into()
            }]
        );
        assert!(!out.decimated);
    }

    #[test]
    fn missing_samples_become_no_data_gaps() {
        let s = [g(0, 1.0), g(1000, 1.0), g(20_000, 2.0)];
        let out = decimate(&s, 0, 30_000, 100, 3_000);
        assert_eq!(out.v, vec![Some(1.0), Some(1.0), None, Some(2.0)]);
        assert_eq!(out.gaps[0].reason, NO_DATA);
        assert_eq!((out.gaps[0].from_ms, out.gaps[0].to_ms), (1000, 20_000));
    }

    #[test]
    fn bad_values_are_never_drawn() {
        let s = [bad(0, Quality::Stale), bad(10, Quality::Exception(2))];
        let out = decimate(&s, 0, 100, 10, 1000);
        assert_eq!(out.v, vec![None]);
        assert_eq!(out.gaps[0].reason, "STALE");
        assert_eq!(out.gaps[0].to_ms, 10);
    }

    #[test]
    fn decimation_keeps_spikes() {
        let mut s: Vec<TrendInput> = (0..10_000).map(|i| g(i, 1.0)).collect();
        s[5_123].value = Some(500.0);
        s[7_001].value = Some(-500.0);
        let out = decimate(&s, 0, 10_000, 50, 10);
        assert!(out.decimated);
        assert!(out.v.len() <= 100);
        assert!(out.v.contains(&Some(500.0)) && out.v.contains(&Some(-500.0)));
        assert!(out.t.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn out_of_range_samples_ignored() {
        let s = [g(-5, 1.0), g(5, 2.0), g(100, 3.0)];
        let out = decimate(&s, 0, 100, 10, 1000);
        assert_eq!(out.v, vec![Some(2.0)]);
        assert_eq!(out.raw_count, 1);
    }
}
