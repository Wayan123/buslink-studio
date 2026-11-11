//! Logging interval and batching in front of the historian.
//!
//! `LogDecimator` stores a sample when its quality differs from the last
//! stored one for that tag (so every gap start and end is kept) or when the
//! logging interval has passed. `SampleBatcher` groups samples into one
//! transaction per second (architecture D9).

use crate::quality::{Quality, Sample};
use std::collections::HashMap;

pub struct LogDecimator {
    interval_ms: u64,
    last: HashMap<(String, String), (i64, Quality)>,
}

impl LogDecimator {
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            last: HashMap::new(),
        }
    }

    pub fn keep(&mut self, s: &Sample) -> bool {
        let key = (s.device_id.clone(), s.tag_id.clone());
        let keep = match self.last.get(&key) {
            None => true,
            Some((ts, q)) => {
                *q != s.quality || s.ts_ms.saturating_sub(*ts) >= i64::try_from(self.interval_ms).unwrap_or(i64::MAX)
            }
        };
        if keep {
            self.last.insert(key, (s.ts_ms, s.quality));
        }
        keep
    }
}

pub struct SampleBatcher {
    max_age_ms: i64,
    max_len: usize,
    pending: Vec<Sample>,
    first_ts: Option<i64>,
}

impl SampleBatcher {
    pub fn new(max_age_ms: u64, max_len: usize) -> Self {
        Self {
            max_age_ms: i64::try_from(max_age_ms).unwrap_or(i64::MAX),
            max_len: max_len.max(1),
            pending: Vec::new(),
            first_ts: None,
        }
    }

    /// Add a sample; returns a batch when it is due (by age of the oldest sample or by size).
    pub fn push(&mut self, s: Sample, now_utc: i64) -> Option<Vec<Sample>> {
        self.first_ts.get_or_insert(now_utc);
        self.pending.push(s);
        self.poll(now_utc)
    }

    pub fn poll(&mut self, now_utc: i64) -> Option<Vec<Sample>> {
        let due = self
            .first_ts
            .is_some_and(|t| now_utc.saturating_sub(t) >= self.max_age_ms)
            || self.pending.len() >= self.max_len;
        if due && !self.pending.is_empty() {
            Some(self.flush())
        } else {
            None
        }
    }

    pub fn flush(&mut self) -> Vec<Sample> {
        self.first_ts = None;
        std::mem::take(&mut self.pending)
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
