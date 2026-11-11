//! CSV export of measurements (UTF-8, RFC 4180 quoting, `.` decimals).

use super::{AUDIT_COLUMNS, ExportDoc, MEASUREMENT_COLUMNS, guard_text, number, raw_hex};
use crate::historian::AuditRow;
use crate::time::iso8601_utc_ms;

fn field(out: &mut String, s: &str) {
    if s.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn row(out: &mut String, cells: &[String]) {
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        field(out, c);
    }
    out.push_str("\r\n");
}

/// Measurements as CSV. Text columns pass through the formula guard; numeric columns are plain numbers.
pub fn measurements(doc: &ExportDoc) -> String {
    let mut out = String::new();
    row(&mut out, &MEASUREMENT_COLUMNS.map(String::from));
    for s in &doc.measurements {
        row(
            &mut out,
            &[
                iso8601_utc_ms(s.ts_ms),
                s.ts_ms.to_string(),
                guard_text(&s.device_id),
                guard_text(&s.tag_id),
                s.value.map(number).unwrap_or_default(),
                s.text.as_deref().map(guard_text).unwrap_or_default(),
                s.unit.as_deref().map(guard_text).unwrap_or_default(),
                s.quality.as_str().to_owned(),
                s.quality
                    .exception_code()
                    .map(|c| format!("{c:02X}"))
                    .unwrap_or_default(),
                s.age_ms.map(|a| a.to_string()).unwrap_or_default(),
                s.value_ts_ms.map(iso8601_utc_ms).unwrap_or_default(),
                raw_hex(&s.raw),
                s.seq.to_string(),
                guard_text(&s.profile_id),
                guard_text(&s.profile_version),
            ],
        );
    }
    out
}

/// Audit rows as CSV (audit view export). Same columns as the XLSX "Commands" sheet.
pub fn audit(rows: &[AuditRow]) -> String {
    let mut out = String::new();
    row(&mut out, &AUDIT_COLUMNS.map(String::from));
    let txt = |s: &str| guard_text(s);
    let otxt = |s: &Option<String>| s.as_deref().map(guard_text).unwrap_or_default();
    let num = |v: Option<f64>| v.map(number).unwrap_or_default();
    let int = |v: Option<u64>| v.map(|x| x.to_string()).unwrap_or_default();
    for a in rows {
        row(
            &mut out,
            &[
                iso8601_utc_ms(a.ts_ms),
                txt(&a.session_id),
                txt(&a.user),
                txt(&a.role),
                otxt(&a.event),
                txt(&a.device_id),
                txt(&a.profile_id),
                txt(&a.profile_version),
                int(a.unit_id.map(u64::from)),
                int(a.function.map(u64::from)),
                int(a.address.map(u64::from)),
                otxt(&a.register_id),
                otxt(&a.command_id),
                num(a.old_value),
                num(a.requested_value),
                otxt(&a.raw_value_hex),
                otxt(&a.request_hex),
                otxt(&a.response_hex),
                int(a.exception.map(u64::from)),
                otxt(&a.ack),
                otxt(&a.readback),
                num(a.readback_value),
                otxt(&a.note),
            ],
        );
    }
    out
}
