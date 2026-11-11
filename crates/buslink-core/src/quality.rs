//! Sample quality and age (PRD R7, safety rule 11).
//!
//! A failed read keeps the last good value with its original timestamp; its
//! age keeps growing. Nothing here ever invents a value or a fresh timestamp.

use crate::codec::RawValue;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "code", content = "exception", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Quality {
    Good,
    Timeout,
    CrcError,
    Exception(u8),
    Stale,
    DecodeError,
    Disconnected,
}

/// OPC UA status class (research/02 mapping): STALE is Uncertain (last usable value), the rest is Bad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum OpcUaClass {
    Good,
    Uncertain,
    Bad,
}

impl Quality {
    pub fn is_good(self) -> bool {
        self == Quality::Good
    }

    /// Integer code stored in the historian.
    pub fn code(self) -> u8 {
        match self {
            Quality::Good => 0,
            Quality::Timeout => 1,
            Quality::CrcError => 2,
            Quality::Exception(_) => 3,
            Quality::Stale => 4,
            Quality::DecodeError => 5,
            Quality::Disconnected => 6,
        }
    }

    pub fn exception_code(self) -> Option<u8> {
        match self {
            Quality::Exception(c) => Some(c),
            _ => None,
        }
    }

    pub fn from_code(code: u8, exception: Option<u8>) -> Option<Quality> {
        Some(match code {
            0 => Quality::Good,
            1 => Quality::Timeout,
            2 => Quality::CrcError,
            3 => Quality::Exception(exception?),
            4 => Quality::Stale,
            5 => Quality::DecodeError,
            6 => Quality::Disconnected,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Good => "GOOD",
            Quality::Timeout => "TIMEOUT",
            Quality::CrcError => "CRC_ERROR",
            Quality::Exception(_) => "EXCEPTION",
            Quality::Stale => "STALE",
            Quality::DecodeError => "DECODE_ERROR",
            Quality::Disconnected => "DISCONNECTED",
        }
    }

    pub fn opc_ua(self) -> OpcUaClass {
        match self {
            Quality::Good => OpcUaClass::Good,
            Quality::Stale => OpcUaClass::Uncertain,
            _ => OpcUaClass::Bad,
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quality::Exception(c) => write!(f, "EXCEPTION {c:02X}"),
            q => f.write_str(q.as_str()),
        }
    }
}

/// A decoded value: raw (exact), engineering (if numeric), text (strings).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Value {
    pub raw: RawValue,
    pub engineering: Option<f64>,
    pub text: Option<String>,
}

/// One published result for one tag (architecture D4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sample {
    pub seq: u64,
    pub link_id: String,
    pub device_id: String,
    pub tag_id: String,
    pub profile_id: String,
    pub profile_version: String,
    /// UTC time this result was produced.
    pub ts_ms: i64,
    pub quality: Quality,
    /// GOOD: the fresh value. Otherwise: the last good value (shown dimmed), or none.
    pub value: Option<Value>,
    /// UTC time `value` was received; never refreshed by a failed read.
    pub value_ts_ms: Option<i64>,
    /// Age of `value` at `ts_ms`.
    pub age_ms: Option<u64>,
    /// Device-provided timestamp (Modbus devices normally have none).
    pub ts_source_ms: Option<i64>,
    /// Registers (or 0/1 bits) read in this attempt, kept for DECODE_ERROR and the inspector.
    pub raw: Option<Vec<u16>>,
    pub unit: Option<String>,
    pub error: Option<String>,
}

impl Sample {
    /// The engineering value only when it is a current GOOD reading.
    pub fn fresh_value(&self) -> Option<f64> {
        if self.quality.is_good() {
            self.value.as_ref().and_then(|v| v.engineering)
        } else {
            None
        }
    }
}

/// Identity fields shared by all samples of one tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagIdentity {
    pub link_id: String,
    pub device_id: String,
    pub tag_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct LastGood {
    value: Value,
    utc_ms: i64,
    mono_ms: u64,
}

/// Per-tag memory: last good value with its receive time, and the last quality published.
#[derive(Clone, Debug, PartialEq)]
pub struct TagState {
    pub identity: TagIdentity,
    last_good: Option<LastGood>,
    last_quality: Option<Quality>,
}

pub struct Observation {
    pub quality: Quality,
    /// Fresh value for GOOD; ignored otherwise.
    pub value: Option<Value>,
    pub raw: Option<Vec<u16>>,
    pub error: Option<String>,
}

impl TagState {
    pub fn new(identity: TagIdentity) -> Self {
        Self {
            identity,
            last_good: None,
            last_quality: None,
        }
    }

    pub fn last_quality(&self) -> Option<Quality> {
        self.last_quality
    }

    pub fn age_ms(&self, now_mono: u64) -> Option<u64> {
        self.last_good.as_ref().map(|g| now_mono.saturating_sub(g.mono_ms))
    }

    /// True when the last published quality is GOOD and its value is older than `threshold_ms`.
    pub fn needs_stale(&self, now_mono: u64, threshold_ms: u64) -> bool {
        self.last_quality == Some(Quality::Good) && self.age_ms(now_mono).is_some_and(|a| a > threshold_ms)
    }

    /// Record a result and build the sample to publish.
    pub fn observe(&mut self, obs: Observation, seq: u64, now_utc: i64, now_mono: u64) -> Sample {
        let mut quality = obs.quality;
        if quality.is_good() {
            match obs.value {
                Some(value) => {
                    self.last_good = Some(LastGood {
                        value,
                        utc_ms: now_utc,
                        mono_ms: now_mono,
                    })
                }
                // GOOD without a value would be a bug; never publish it as GOOD.
                None => quality = Quality::DecodeError,
            }
        }
        self.last_quality = Some(quality);
        let id = &self.identity;
        Sample {
            seq,
            link_id: id.link_id.clone(),
            device_id: id.device_id.clone(),
            tag_id: id.tag_id.clone(),
            profile_id: id.profile_id.clone(),
            profile_version: id.profile_version.clone(),
            ts_ms: now_utc,
            quality,
            value: self.last_good.as_ref().map(|g| g.value.clone()),
            value_ts_ms: self.last_good.as_ref().map(|g| g.utc_ms),
            age_ms: self.age_ms(now_mono),
            ts_source_ms: None,
            raw: obs.raw,
            unit: id.unit.clone(),
            error: obs.error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag() -> TagState {
        TagState::new(TagIdentity {
            link_id: "bus1".into(),
            device_id: "soil".into(),
            tag_id: "temperature".into(),
            profile_id: "bgt.smps-8in1".into(),
            profile_version: "1.0.0".into(),
            unit: Some("°C".into()),
        })
    }

    fn good(v: f64) -> Observation {
        Observation {
            quality: Quality::Good,
            value: Some(Value {
                raw: RawValue::I16((v * 10.0) as i16),
                engineering: Some(v),
                text: None,
            }),
            raw: Some(vec![(v * 10.0) as u16]),
            error: None,
        }
    }

    fn fail(q: Quality) -> Observation {
        Observation {
            quality: q,
            value: None,
            raw: None,
            error: Some("x".into()),
        }
    }

    #[test]
    fn failure_keeps_last_good_with_original_timestamp() {
        let mut t = tag();
        let s1 = t.observe(good(25.3), 1, 10_000, 0);
        assert_eq!(
            (s1.quality, s1.value_ts_ms, s1.age_ms),
            (Quality::Good, Some(10_000), Some(0))
        );
        let s2 = t.observe(fail(Quality::Timeout), 2, 11_000, 1_000);
        assert_eq!(s2.quality, Quality::Timeout);
        assert_eq!(s2.value.as_ref().unwrap().engineering, Some(25.3));
        assert_eq!(
            s2.value_ts_ms,
            Some(10_000),
            "timestamp of the value is never refreshed"
        );
        assert_eq!(s2.age_ms, Some(1_000));
        assert_eq!(s2.fresh_value(), None);
        let s3 = t.observe(fail(Quality::Exception(2)), 3, 15_000, 5_000);
        assert_eq!(s3.age_ms, Some(5_000), "age keeps growing");
        assert_eq!(s3.ts_ms, 15_000);
    }

    #[test]
    fn no_value_before_first_good() {
        let mut t = tag();
        let s = t.observe(fail(Quality::CrcError), 1, 0, 0);
        assert!(s.value.is_none() && s.age_ms.is_none() && s.value_ts_ms.is_none());
    }

    #[test]
    fn good_without_value_is_never_published_as_good() {
        let mut t = tag();
        let s = t.observe(
            Observation {
                quality: Quality::Good,
                value: None,
                raw: None,
                error: None,
            },
            1,
            0,
            0,
        );
        assert_eq!(s.quality, Quality::DecodeError);
    }

    #[test]
    fn stale_detection() {
        let mut t = tag();
        t.observe(good(1.0), 1, 0, 0);
        assert!(!t.needs_stale(3_000, 3_000));
        assert!(t.needs_stale(3_001, 3_000));
        t.observe(fail(Quality::Stale), 2, 3_001, 3_001);
        assert!(!t.needs_stale(10_000, 3_000), "STALE is emitted once");
    }

    #[test]
    fn codes_labels_and_opc_ua() {
        for q in [
            Quality::Good,
            Quality::Timeout,
            Quality::CrcError,
            Quality::Exception(2),
            Quality::Stale,
            Quality::DecodeError,
            Quality::Disconnected,
        ] {
            assert_eq!(Quality::from_code(q.code(), q.exception_code()), Some(q));
        }
        assert_eq!(Quality::Exception(2).to_string(), "EXCEPTION 02");
        assert_eq!(Quality::Stale.opc_ua(), OpcUaClass::Uncertain);
        assert_eq!(Quality::Timeout.opc_ua(), OpcUaClass::Bad);
        assert_eq!(
            serde_json::to_string(&Quality::Exception(2)).unwrap(),
            r#"{"code":"EXCEPTION","exception":2}"#
        );
        assert_eq!(serde_json::to_string(&Quality::Good).unwrap(), r#"{"code":"GOOD"}"#);
    }
}
