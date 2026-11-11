//! JSON export. 64-bit integers (seq, raw values) are strings; no NaN.

use super::{ExportDoc, raw_hex};
use crate::time::iso8601_utc_ms;
use serde_json::{Value, json};

fn finite(v: Option<f64>) -> Value {
    v.filter(|x| x.is_finite()).map(Value::from).unwrap_or(Value::Null)
}

pub fn document(doc: &ExportDoc) -> Value {
    let measurements: Vec<Value> = doc
        .measurements
        .iter()
        .map(|s| {
            json!({
                "ts_utc": iso8601_utc_ms(s.ts_ms),
                "ts_ms": s.ts_ms,
                "device_id": s.device_id,
                "tag_id": s.tag_id,
                "value": finite(s.value),
                "raw_value": s.raw_value,
                "text": s.text,
                "unit": s.unit,
                "quality": s.quality.as_str(),
                "exception_code": s.quality.exception_code(),
                "age_ms": s.age_ms.map(|a| a.to_string()),
                "value_ts_utc": s.value_ts_ms.map(iso8601_utc_ms),
                "raw_hex": raw_hex(&s.raw),
                "seq": s.seq.to_string(),
                "profile_id": s.profile_id,
                "profile_version": s.profile_version,
            })
        })
        .collect();
    let commands: Vec<Value> = doc
        .commands
        .iter()
        .map(|c| {
            json!({
                "ts_utc": iso8601_utc_ms(c.ts_ms),
                "session_id": c.session_id,
                "user": c.user,
                "role": c.role,
                "event": c.event,
                "device_id": c.device_id,
                "profile_id": c.profile_id,
                "profile_version": c.profile_version,
                "unit_id": c.unit_id,
                "function": c.function,
                "address": c.address,
                "register_id": c.register_id,
                "command_id": c.command_id,
                "old_value": finite(c.old_value),
                "requested_value": finite(c.requested_value),
                "raw_value_hex": c.raw_value_hex,
                "request_hex": c.request_hex,
                "response_hex": c.response_hex,
                "exception": c.exception,
                "ack": c.ack,
                "readback": c.readback,
                "readback_value": finite(c.readback_value),
                "note": c.note,
            })
        })
        .collect();
    json!({
        "format": "buslink-export",
        "format_version": 1,
        "generated_utc": iso8601_utc_ms(doc.generated_ms),
        "app_version": doc.app_version,
        "session": {
            "id": doc.session.id,
            "started_utc": iso8601_utc_ms(doc.session.started_ms),
            "platform": doc.session.platform,
            "app_version": doc.session.app_version,
            "logging_interval_ms": doc.session.logging_interval_ms,
        },
        "devices": doc.devices,
        "measurements": measurements,
        "events": doc.events.iter().map(|e| json!({
            "ts_utc": iso8601_utc_ms(e.ts_ms), "kind": e.kind, "device": e.device, "tag": e.tag,
            "severity": e.severity, "text": e.text,
        })).collect::<Vec<_>>(),
        "commands": commands,
    })
}

pub fn to_string(doc: &ExportDoc) -> String {
    serde_json::to_string_pretty(&document(doc)).unwrap_or_else(|_| "{}".into())
}
