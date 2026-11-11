//! Session report data (PRD R19). The UI renders it as HTML (text nodes only)
//! and prints it to PDF; this module only gathers facts. Nothing here claims
//! hardware behavior: simulated devices are flagged as such.

use crate::alarm::AlarmSummary;
use crate::historian::{AuditRow, EventRow, SessionInfo};
use crate::poll::scheduler::DeviceCounters;
use crate::profile::SupportLevel;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportDevice {
    pub device_id: String,
    pub name: String,
    pub link: String,
    /// Unit ID now (after any commissioning change).
    pub unit_id: u8,
    pub profile_id: String,
    pub profile_version: String,
    pub support_level: Option<SupportLevel>,
    pub simulated_only: bool,
    /// Link counters while the device is still in the session.
    pub counters: Option<DeviceCounters>,
    /// Stored samples per quality label.
    pub quality: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportAlarms {
    pub summary: AlarmSummary,
    /// Alarm events of the session (activations, acknowledgements, shelving).
    pub events: Vec<EventRow>,
    /// Activations per priority 1..3 counted from the events.
    pub activations: [u64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportData {
    pub generated_ms: i64,
    pub app_version: String,
    pub session: SessionInfo,
    pub devices: Vec<ReportDevice>,
    pub alarms: ReportAlarms,
    /// Every audit row of the session in order (commands, wizard journal/verify, feedback).
    pub audit: Vec<AuditRow>,
    /// Link and commissioning events.
    pub events: Vec<EventRow>,
}

/// Count alarm activations per priority from event rows (`severity` = `P1`..`P3`, text `activate: ...`).
pub fn activations(events: &[EventRow]) -> [u64; 3] {
    let mut out = [0u64; 3];
    for e in events.iter().filter(|e| e.kind == "alarm") {
        let p = match e.severity.as_str() {
            "P1" => 0,
            "P2" => 1,
            "P3" => 2,
            _ => continue,
        };
        if e.text.starts_with("activate:") {
            out[p] += 1;
        }
    }
    out
}
