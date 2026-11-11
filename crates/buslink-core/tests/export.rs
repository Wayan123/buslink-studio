//! Export (Req 18): CSV, JSON, XLSX, formula guard (P13).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::codec::RawValue;
use buslink_core::command::audit::{AuditEvent, AuditRecord};
use buslink_core::export::{self, ExportDoc, guard_text};
use buslink_core::historian::{DeviceRow, EventRow, Historian};
use buslink_core::quality::{Quality, Sample, Value};
use buslink_core::testkit::check;
use std::io::Read;

const T0: i64 = 1_800_000_000_000;

fn sample(tag: &str, seq: u64, ts: i64, q: Quality, v: Option<f64>) -> Sample {
    Sample {
        seq,
        link_id: "bus1".into(),
        device_id: "soil".into(),
        tag_id: tag.into(),
        profile_id: "bgt.smps-8in1".into(),
        profile_version: "1.0.0".into(),
        ts_ms: ts,
        quality: q,
        value: v.map(|x| Value {
            raw: RawValue::I16((x * 10.0).round() as i16),
            engineering: Some(x),
            text: None,
        }),
        value_ts_ms: Some(T0),
        age_ms: Some(u64::try_from(ts - T0).unwrap()),
        ts_source_ms: None,
        raw: v.map(|x| vec![(x * 10.0).round() as i16 as u16]),
        unit: Some("°C".into()),
        error: None,
    }
}

fn doc() -> ExportDoc {
    let mut h = Historian::open_in_memory().unwrap();
    h.begin_session("s1", T0, "0.1.0", "linux-test", 1000).unwrap();
    h.register_device(&DeviceRow {
        session: "s1".into(),
        device_id: "soil".into(),
        name: "=1+1 soil".into(),
        profile_id: "bgt.smps-8in1".into(),
        profile_version: "1.0.0".into(),
        unit_id: 1,
        link: "bus1".into(),
    })
    .unwrap();
    h.insert_samples(&[
        sample("temperature", 1, T0, Quality::Good, Some(-3.5)),
        sample("temperature", 2, T0 + 1000, Quality::Timeout, None),
        sample("temperature", 3, T0 + 2000, Quality::Exception(2), None),
        sample("temperature", 4, T0 + 3000, Quality::Good, Some(25.3)),
    ])
    .unwrap();
    h.add_event(&EventRow {
        session: Some("s1".into()),
        ts_ms: T0 + 1000,
        kind: "comm".into(),
        device: Some("soil".into()),
        tag: None,
        severity: "warning".into(),
        text: "@timeout".into(),
    })
    .unwrap();
    h.append_audit(&AuditRecord {
        ts_ms: T0 + 5,
        session_id: "s1".into(),
        user: "tech".into(),
        role: "operator".into(),
        event: Some(AuditEvent::Result),
        device_id: "vfd".into(),
        profile_id: "sim.vfd-basic".into(),
        profile_version: "1.0.0".into(),
        unit_id: Some(2),
        function: Some(6),
        address: Some(1),
        requested_value: Some(30.0),
        readback: Some("verified".into()),
        ..AuditRecord::default()
    })
    .unwrap();
    ExportDoc::from_historian(&h, "s1", T0 + 60_000, "0.1.0").unwrap()
}

#[test]
fn csv_has_quality_age_profile_and_utc() {
    let csv = export::csv::measurements(&doc());
    let lines: Vec<&str> = csv.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines[0],
        "ts_utc,ts_ms,device_id,tag_id,value,text,unit,quality,exception_code,age_ms,value_ts_utc,raw_hex,seq,profile_id,profile_version"
    );
    assert_eq!(lines.len(), 5);
    assert_eq!(
        lines[1],
        "2027-01-15T08:00:00.000Z,1800000000000,soil,temperature,-3.5,,°C,GOOD,,0,2027-01-15T08:00:00.000Z,FFDD,1,bgt.smps-8in1,1.0.0"
    );
    assert!(
        lines[2].contains(",,°C,TIMEOUT,,1000,"),
        "TIMEOUT row has an empty value, never 0: {}",
        lines[2]
    );
    assert!(lines[3].contains(",EXCEPTION,02,"));
    assert!(lines[4].contains(",25.3,"));
}

#[test]
fn json_uses_strings_for_64_bit_and_null_for_gaps() {
    let v = export::json::document(&doc());
    let m = &v["measurements"];
    assert_eq!(m[0]["seq"], "1");
    assert_eq!(m[0]["value"], -3.5);
    assert!(m[1]["value"].is_null());
    assert_eq!(m[1]["quality"], "TIMEOUT");
    assert_eq!(m[2]["exception_code"], 2);
    assert_eq!(m[0]["profile_version"], "1.0.0");
    assert_eq!(v["commands"][0]["readback"], "verified");
    let text = export::json::to_string(&doc());
    assert!(!text.contains("NaN"));
    let lower = text.to_lowercase();
    for secret in ["password", "token", "secret"] {
        assert!(!lower.contains(secret), "no {secret} field in exports");
    }
}

fn read_zip(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    s
}

#[test]
fn xlsx_has_five_sheets_numbers_as_numbers_and_guarded_text() {
    let bytes = export::xlsx::workbook(&doc()).unwrap();
    assert_eq!(&bytes[..2], b"PK");
    let wb = read_zip(&bytes, "xl/workbook.xml");
    let names: Vec<&str> = ["Measurements", "Devices", "Configuration", "Events", "Commands"].to_vec();
    let mut pos = 0;
    for n in &names {
        let p = wb.find(&format!("name=\"{n}\"")).unwrap_or_else(|| panic!("sheet {n}"));
        assert!(p > pos, "sheet order");
        pos = p;
    }
    let sheet1 = read_zip(&bytes, "xl/worksheets/sheet1.xml");
    // E2 = value -3.5 as a number cell (no t="s"), E3 empty for TIMEOUT.
    assert!(sheet1.contains(r#"<c r="E2"><v>-3.5</v></c>"#), "{sheet1}");
    assert!(!sheet1.contains(r#"r="E3""#), "a TIMEOUT value cell stays empty");
    assert!(sheet1.contains(r#"<c r="B2"><v>1800000000000</v></c>"#));
    let shared = read_zip(&bytes, "xl/sharedStrings.xml");
    assert!(
        shared.contains("&apos;=1+1 soil") || shared.contains("'=1+1 soil"),
        "guarded device name: {shared}"
    );
    assert!(shared.contains("'@timeout") || shared.contains("&apos;@timeout"));
    assert!(!sheet1.contains("<f>"), "no formulas anywhere");
    assert!(!read_zip(&bytes, "xl/worksheets/sheet2.xml").contains("<f>"));
}

#[test]
fn formula_guard_property() {
    check("formula_guard", 5000, |r| {
        let len = r.range_usize(0, 8);
        let alphabet = ['=', '+', '-', '@', '\t', '\r', 'a', '1', ' ', '"', ',', 'é'];
        let s: String = (0..len).map(|_| *r.pick(&alphabet).unwrap()).collect();
        let g = guard_text(&s);
        let dangerous = s.starts_with(['=', '+', '-', '@', '\t', '\r']);
        if dangerous {
            assert_eq!(g, format!("'{s}"));
        } else {
            assert_eq!(g, s);
        }
        assert!(!g.starts_with(['=', '+', '-', '@', '\t', '\r']));
    });
}

#[test]
fn csv_quotes_and_guards_text() {
    let mut d = doc();
    d.measurements[0].text = Some("=HYPERLINK(\"x\",\"y\")".into());
    d.measurements[0].device_id = "a,b".into();
    let csv = export::csv::measurements(&d);
    let line = csv.split("\r\n").nth(1).unwrap();
    assert!(line.contains("\"'=HYPERLINK(\"\"x\"\",\"\"y\"\")\""), "{line}");
    assert!(line.contains(",\"a,b\","));
}
