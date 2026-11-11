//! UTC formatting helpers (no chrono): ISO-8601 with milliseconds and `Z`.

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's civil-from-days.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2026-11-12T03:04:05.678Z`. Years outside 0000–9999 are clamped into the
/// representable range so the output is always a valid ISO-8601 string.
pub fn iso8601_utc_ms(epoch_ms: i64) -> String {
    const MIN: i64 = -62_167_219_200_000; // 0000-01-01T00:00:00Z
    const MAX: i64 = 253_402_300_799_999; // 9999-12-31T23:59:59.999Z
    let ms = epoch_ms.clamp(MIN, MAX);
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Upper-case hex with single spaces: `01 03 00 00`.
pub fn hex_spaced(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{b:02X}"));
    }
    s
}

/// Upper-case hex without separators: `0103`.
pub fn hex_compact(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants() {
        assert_eq!(iso8601_utc_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601_utc_ms(1_794_366_245_678), "2026-11-11T03:04:05.678Z");
        assert_eq!(iso8601_utc_ms(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn out_of_range_is_clamped_not_garbage() {
        assert_eq!(iso8601_utc_ms(i64::MAX), "9999-12-31T23:59:59.999Z");
        assert_eq!(iso8601_utc_ms(i64::MIN), "0000-01-01T00:00:00.000Z");
    }

    #[test]
    fn hex_formats() {
        assert_eq!(hex_spaced(&[0x01, 0xAB]), "01 AB");
        assert_eq!(hex_compact(&[0x01, 0xAB]), "01AB");
        assert_eq!(hex_spaced(&[]), "");
    }
}
