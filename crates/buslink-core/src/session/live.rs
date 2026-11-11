//! Latest state per tag and the trend points not yet sent to the UI
//! (app-shell-desktop design §3). Nothing here invents values: a tag view is
//! built from the last published sample only.

use crate::quality::{Quality, Sample};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Most trend points one frame carries; the rest is counted as dropped.
pub const MAX_POINTS_PER_FRAME: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TagView {
    /// `device_id/tag_id`
    pub key: String,
    pub link_id: String,
    pub device_id: String,
    pub tag_id: String,
    /// `GOOD`, `TIMEOUT`, `CRC_ERROR`, `EXCEPTION`, `STALE`, `DECODE_ERROR`, `DISCONNECTED`.
    pub quality: &'static str,
    pub exception: Option<u8>,
    /// Engineering value of `value_ts_ms` (the last good value when quality is not GOOD).
    pub value: Option<f64>,
    pub text: Option<String>,
    /// Exact decoded raw value (64-bit integers as digits).
    pub raw_value: Option<String>,
    pub value_ts_ms: Option<i64>,
    pub ts_ms: i64,
    pub age_ms: Option<u64>,
    pub raw: Option<Vec<u16>>,
    pub unit: Option<String>,
    pub error: Option<String>,
    pub seq: u64,
}

/// One sample as a trend point: `v` is set only for GOOD samples.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrendPoint {
    pub k: String,
    pub t: i64,
    pub v: Option<f64>,
    pub q: &'static str,
}

pub fn tag_key(device: &str, tag: &str) -> String {
    format!("{device}/{tag}")
}

pub fn view_of(s: &Sample) -> TagView {
    TagView {
        key: tag_key(&s.device_id, &s.tag_id),
        link_id: s.link_id.clone(),
        device_id: s.device_id.clone(),
        tag_id: s.tag_id.clone(),
        quality: s.quality.as_str(),
        exception: s.quality.exception_code(),
        value: s.value.as_ref().and_then(|v| v.engineering).filter(|x| x.is_finite()),
        text: s.value.as_ref().and_then(|v| v.text.clone()),
        raw_value: s.value.as_ref().map(|v| v.raw.display()),
        value_ts_ms: s.value_ts_ms,
        ts_ms: s.ts_ms,
        age_ms: s.age_ms,
        raw: s.raw.clone(),
        unit: s.unit.clone(),
        error: s.error.clone(),
        seq: s.seq,
    }
}

struct Entry {
    last: Arc<Sample>,
    last_raw: Option<Vec<u16>>,
    view: TagView,
}

#[derive(Default)]
pub struct LiveStore {
    tags: BTreeMap<String, Entry>,
    dirty: BTreeSet<String>,
    points: Vec<TrendPoint>,
    dropped: u64,
    applied: u64,
}

impl LiveStore {
    pub fn apply(&mut self, s: &Arc<Sample>) {
        let view = view_of(s);
        let key = view.key.clone();
        self.applied += 1;
        if self.points.len() < MAX_POINTS_PER_FRAME {
            self.points.push(TrendPoint {
                k: key.clone(),
                t: s.ts_ms,
                v: s.fresh_value().filter(|v| v.is_finite()),
                q: s.quality.as_str(),
            });
        } else {
            self.dropped += 1;
        }
        let last_raw = match (&s.raw, self.tags.get(&key)) {
            (Some(r), _) if !r.is_empty() => Some(r.clone()),
            (_, Some(e)) => e.last_raw.clone(),
            _ => None,
        };
        self.tags.insert(
            key.clone(),
            Entry {
                last: Arc::clone(s),
                last_raw,
                view,
            },
        );
        self.dirty.insert(key);
    }

    /// Bus lag: samples that never reached this store.
    pub fn add_dropped(&mut self, n: u64) {
        self.dropped = self.dropped.saturating_add(n);
    }

    /// Samples applied since the store was created.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Changed tags, pending points and the dropped count since the last call.
    pub fn take_changes(&mut self) -> (Vec<TagView>, Vec<TrendPoint>, u64) {
        let dirty = std::mem::take(&mut self.dirty);
        let tags = dirty
            .iter()
            .filter_map(|k| self.tags.get(k).map(|e| e.view.clone()))
            .collect();
        (
            tags,
            std::mem::take(&mut self.points),
            std::mem::take(&mut self.dropped),
        )
    }

    pub fn all(&self) -> Vec<TagView> {
        self.tags.values().map(|e| e.view.clone()).collect()
    }

    pub fn get(&self, key: &str) -> Option<&TagView> {
        self.tags.get(key).map(|e| &e.view)
    }

    pub fn last_raw(&self, key: &str) -> Option<&[u16]> {
        self.tags.get(key).and_then(|e| e.last_raw.as_deref())
    }

    /// Last samples of every tag on a link (for DISCONNECTED republishing).
    pub fn last_samples_of_link(&self, link_id: &str) -> Vec<Arc<Sample>> {
        self.tags
            .values()
            .filter(|e| e.last.link_id == link_id)
            .map(|e| Arc::clone(&e.last))
            .collect()
    }

    pub fn remove_device(&mut self, device_id: &str) {
        let prefix = format!("{device_id}/");
        self.tags.retain(|k, _| !k.starts_with(&prefix));
        self.dirty.retain(|k| !k.starts_with(&prefix));
    }

    pub fn worst_quality_of_device(&self, device_id: &str) -> Option<Quality> {
        self.tags
            .values()
            .filter(|e| e.last.device_id == device_id)
            .map(|e| e.last.quality)
            .find(|q| !q.is_good())
    }
}
