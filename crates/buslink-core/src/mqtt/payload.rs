//! Telemetry payload (research/02 §4): `{ts, seq, value, unit, quality, profile_version}`
//! plus what safety rule 11 adds: age, exception code, exact raw value and the session.

use crate::quality::Sample;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Telemetry {
    /// UTC epoch ms of this result.
    pub ts: i64,
    /// Sequence number within `session`; with the gateway it is the dedupe key.
    pub seq: u64,
    /// Engineering value; for a non-GOOD quality the last good value, or `null`. Never a stand-in zero.
    pub value: serde_json::Value,
    /// Exact decoded raw value as text (64-bit integers keep every digit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    pub unit: Option<String>,
    pub quality: &'static str,
    /// EXCEPTION code when `quality` is `EXCEPTION`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<u8>,
    /// Age of `value` at `ts`.
    pub age_ms: Option<u64>,
    pub profile_version: String,
    pub session: String,
}

pub fn from_sample(s: &Sample, session: &str) -> Telemetry {
    let value = match s.value.as_ref() {
        Some(v) => match (v.engineering.filter(|x| x.is_finite()), &v.text) {
            (Some(x), _) => serde_json::Number::from_f64(x)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            (None, Some(t)) => serde_json::Value::String(t.clone()),
            _ => serde_json::Value::Null,
        },
        None => serde_json::Value::Null,
    };
    Telemetry {
        ts: s.ts_ms,
        seq: s.seq,
        value,
        raw: s.value.as_ref().map(|v| v.raw.display()),
        unit: s.unit.clone(),
        quality: s.quality.as_str(),
        exception: s.quality.exception_code(),
        age_ms: s.age_ms,
        profile_version: s.profile_version.clone(),
        session: session.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::RawValue;
    use crate::quality::{Quality, Value};

    fn sample(q: Quality, with_value: bool) -> Sample {
        Sample {
            seq: 7,
            link_id: "l".into(),
            device_id: "d".into(),
            tag_id: "t".into(),
            profile_id: "p".into(),
            profile_version: "1.2.0".into(),
            ts_ms: 10_000,
            quality: q,
            value: with_value.then_some(Value {
                raw: RawValue::I64(i64::MAX),
                engineering: Some(21.5),
                text: None,
            }),
            value_ts_ms: with_value.then_some(4_000),
            age_ms: with_value.then_some(6_000),
            ts_source_ms: None,
            raw: None,
            unit: Some("°C".into()),
            error: None,
        }
    }

    /// M2: the required keys are there; a failed read carries the last good value and its age.
    #[test]
    fn m2_required_keys_and_honest_failures() {
        let good = serde_json::to_value(from_sample(&sample(Quality::Good, true), "s1")).unwrap();
        for k in [
            "ts",
            "seq",
            "value",
            "unit",
            "quality",
            "profile_version",
            "age_ms",
            "session",
        ] {
            assert!(good.get(k).is_some(), "{k}");
        }
        assert_eq!(good["quality"], "GOOD");
        assert_eq!(good["raw"], "9223372036854775807", "64-bit values keep every digit");
        assert!(good.get("exception").is_none());

        let bad = serde_json::to_value(from_sample(&sample(Quality::Exception(2), true), "s1")).unwrap();
        assert_eq!(bad["quality"], "EXCEPTION");
        assert_eq!(bad["exception"], 2);
        assert_eq!(bad["value"], 21.5, "last good value, not zero");
        assert_eq!(bad["age_ms"], 6000, "the age keeps growing");
        assert_eq!(bad["ts"], 10_000);

        let none = serde_json::to_value(from_sample(&sample(Quality::Timeout, false), "s1")).unwrap();
        assert!(none["value"].is_null() && none["age_ms"].is_null());
    }
}
