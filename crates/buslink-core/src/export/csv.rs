//! CSV export of measurements (UTF-8, RFC 4180 quoting, `.` decimals).

use super::{ExportDoc, MEASUREMENT_COLUMNS, guard_text, number, raw_hex};
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
