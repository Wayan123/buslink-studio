//! Exports (PRD R21): CSV, XLSX, JSON. Every measurement row carries
//! quality, exception code, age and the profile ID + version. Times are
//! ISO-8601 UTC; decimals always use `.`. Text cells that start with
//! `= + - @`, tab or CR get a leading `'` (formula-injection guard).
//! Export types contain no credential fields at all.

pub mod csv;
pub mod json;
pub mod report;
pub mod xlsx;

use crate::historian::{AuditRow, DeviceRow, EventRow, Historian, HistorianError, SessionInfo, StoredSample};
use serde::Serialize;

/// Escape a text cell for spreadsheets.
pub fn guard_text(s: &str) -> String {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExportDoc {
    pub generated_ms: i64,
    pub app_version: String,
    pub session: SessionInfo,
    pub devices: Vec<DeviceRow>,
    pub measurements: Vec<StoredSample>,
    pub events: Vec<EventRow>,
    pub commands: Vec<AuditRow>,
}

impl ExportDoc {
    pub fn from_historian(
        h: &Historian,
        session: &str,
        generated_ms: i64,
        app_version: &str,
    ) -> Result<Self, HistorianError> {
        let info = h
            .sessions()?
            .into_iter()
            .find(|s| s.id == session)
            .ok_or_else(|| HistorianError::Db(format!("unknown session '{session}'")))?;
        Ok(Self {
            generated_ms,
            app_version: app_version.to_owned(),
            devices: h.devices(session)?,
            measurements: h.samples(session, None, None, i64::MIN, i64::MAX)?,
            events: h.events(session)?,
            commands: h.audit(session)?,
            session: info,
        })
    }
}

/// Raw registers as compact hex words: `00FD 0138`.
pub fn raw_hex(regs: &[u16]) -> String {
    regs.iter().map(|r| format!("{r:04X}")).collect::<Vec<_>>().join(" ")
}

/// Shortest round-trip decimal with `.` (never locale-formatted).
pub fn number(v: f64) -> String {
    if v.is_finite() { format!("{v}") } else { String::new() }
}

pub const MEASUREMENT_COLUMNS: [&str; 15] = [
    "ts_utc",
    "ts_ms",
    "device_id",
    "tag_id",
    "value",
    "text",
    "unit",
    "quality",
    "exception_code",
    "age_ms",
    "value_ts_utc",
    "raw_hex",
    "seq",
    "profile_id",
    "profile_version",
];

pub const AUDIT_COLUMNS: [&str; 23] = [
    "ts_utc",
    "session_id",
    "user",
    "role",
    "event",
    "device_id",
    "profile_id",
    "profile_version",
    "unit_id",
    "function",
    "address",
    "register_id",
    "command_id",
    "old_value",
    "requested_value",
    "raw_value_hex",
    "request_hex",
    "response_hex",
    "exception",
    "ack",
    "readback",
    "readback_value",
    "note",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_guard() {
        for s in ["=1+1", "+cmd", "-2", "@SUM(A1)", "\tx", "\rx"] {
            assert_eq!(guard_text(s), format!("'{s}"));
        }
        for s in ["soil", "", "25.3", " =x", "a=b"] {
            assert_eq!(guard_text(s), s);
        }
    }
}
