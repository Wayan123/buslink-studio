//! XLSX export: sheets Measurements, Devices, Configuration, Events, Commands.
//! Numbers are written as numbers; text cells pass through the formula guard
//! and are written as strings (never as formulas).

use super::{AUDIT_COLUMNS, ExportDoc, MEASUREMENT_COLUMNS, guard_text, raw_hex};
use crate::historian::AuditRow;
use crate::time::iso8601_utc_ms;
use rust_xlsxwriter::{Workbook, Worksheet, XlsxError};

enum Cell {
    Text(String),
    Num(Option<f64>),
}

fn t(s: &str) -> Cell {
    Cell::Text(s.to_owned())
}
fn ot(s: Option<&str>) -> Cell {
    Cell::Text(s.unwrap_or_default().to_owned())
}
fn n<T: Into<f64>>(v: Option<T>) -> Cell {
    Cell::Num(v.map(Into::into))
}

fn write_rows(ws: &mut Worksheet, header: &[&str], rows: Vec<Vec<Cell>>) -> Result<(), XlsxError> {
    for (c, h) in header.iter().enumerate() {
        ws.write_string(0, c as u16, *h)?;
    }
    for (r, cells) in rows.into_iter().enumerate() {
        let r = (r + 1) as u32;
        for (c, cell) in cells.into_iter().enumerate() {
            let c = c as u16;
            match cell {
                Cell::Text(s) if !s.is_empty() => {
                    ws.write_string(r, c, guard_text(&s))?;
                }
                Cell::Num(Some(v)) if v.is_finite() => {
                    ws.write_number(r, c, v)?;
                }
                _ => {}
            }
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

/// Integers above 2^53 are not exact as XLSX numbers, so they go out as text.
fn int_cell(v: u64) -> Cell {
    if v <= (1u64 << 53) {
        Cell::Num(Some(v as f64))
    } else {
        Cell::Text(v.to_string())
    }
}

pub fn workbook(doc: &ExportDoc) -> Result<Vec<u8>, XlsxError> {
    let mut wb = Workbook::new();

    let ws = wb.add_worksheet();
    ws.set_name("Measurements")?;
    let rows = doc
        .measurements
        .iter()
        .map(|s| {
            vec![
                t(&iso8601_utc_ms(s.ts_ms)),
                Cell::Num(Some(s.ts_ms as f64)),
                t(&s.device_id),
                t(&s.tag_id),
                n(s.value),
                ot(s.text.as_deref()),
                ot(s.unit.as_deref()),
                t(s.quality.as_str()),
                Cell::Text(
                    s.quality
                        .exception_code()
                        .map(|c| format!("{c:02X}"))
                        .unwrap_or_default(),
                ),
                s.age_ms.map(int_cell).unwrap_or(Cell::Num(None)),
                Cell::Text(s.value_ts_ms.map(iso8601_utc_ms).unwrap_or_default()),
                t(&raw_hex(&s.raw)),
                int_cell(s.seq),
                t(&s.profile_id),
                t(&s.profile_version),
            ]
        })
        .collect();
    write_rows(ws, &MEASUREMENT_COLUMNS, rows)?;

    let ws = wb.add_worksheet();
    ws.set_name("Devices")?;
    let rows = doc
        .devices
        .iter()
        .map(|d| {
            vec![
                t(&d.device_id),
                t(&d.name),
                n(Some(d.unit_id)),
                t(&d.link),
                t(&d.profile_id),
                t(&d.profile_version),
            ]
        })
        .collect();
    write_rows(
        ws,
        &["device_id", "name", "unit_id", "link", "profile_id", "profile_version"],
        rows,
    )?;

    let ws = wb.add_worksheet();
    ws.set_name("Configuration")?;
    let s = &doc.session;
    let rows = vec![
        vec![t("session_id"), t(&s.id)],
        vec![t("session_started_utc"), t(&iso8601_utc_ms(s.started_ms))],
        vec![t("platform"), t(&s.platform)],
        vec![t("app_version"), t(&s.app_version)],
        vec![t("logging_interval_ms"), Cell::Num(Some(s.logging_interval_ms as f64))],
        vec![t("generated_utc"), t(&iso8601_utc_ms(doc.generated_ms))],
        vec![t("export_app_version"), t(&doc.app_version)],
        vec![t("time_zone"), t("UTC")],
    ];
    write_rows(ws, &["key", "value"], rows)?;

    let ws = wb.add_worksheet();
    ws.set_name("Events")?;
    let rows = doc
        .events
        .iter()
        .map(|e| {
            vec![
                t(&iso8601_utc_ms(e.ts_ms)),
                t(&e.kind),
                ot(e.device.as_deref()),
                ot(e.tag.as_deref()),
                t(&e.severity),
                t(&e.text),
            ]
        })
        .collect();
    write_rows(ws, &["ts_utc", "kind", "device", "tag", "severity", "text"], rows)?;

    commands_sheet(&mut wb, &doc.commands)?;
    wb.save_to_buffer()
}

/// The audit rows as a "Commands" sheet (session export and the audit view's XLSX).
fn commands_sheet(wb: &mut Workbook, rows_in: &[AuditRow]) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Commands")?;
    let rows = rows_in
        .iter()
        .map(|c| {
            vec![
                t(&iso8601_utc_ms(c.ts_ms)),
                t(&c.session_id),
                t(&c.user),
                t(&c.role),
                ot(c.event.as_deref()),
                t(&c.device_id),
                t(&c.profile_id),
                t(&c.profile_version),
                n(c.unit_id),
                n(c.function),
                n(c.address),
                ot(c.register_id.as_deref()),
                ot(c.command_id.as_deref()),
                n(c.old_value),
                n(c.requested_value),
                ot(c.raw_value_hex.as_deref()),
                ot(c.request_hex.as_deref()),
                ot(c.response_hex.as_deref()),
                n(c.exception),
                ot(c.ack.as_deref()),
                ot(c.readback.as_deref()),
                n(c.readback_value),
                ot(c.note.as_deref()),
            ]
        })
        .collect();
    write_rows(ws, &AUDIT_COLUMNS, rows)?;
    Ok(())
}

/// Audit log only (audit view export).
pub fn audit_workbook(rows: &[AuditRow]) -> Result<Vec<u8>, XlsxError> {
    let mut wb = Workbook::new();
    commands_sheet(&mut wb, rows)?;
    wb.save_to_buffer()
}
