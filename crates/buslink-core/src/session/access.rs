//! Session services for the command layer (commissioning-control design §2):
//! the worker handle for a device, link generations, settings changes after a
//! commissioning write, the historian audit sink, audit rows and report data.
//!
//! Nothing here builds a write. `command_access` hands out the link worker's
//! queue handle; only `crate::command` can create a frame it will send.

use super::{LinkState, Session, SessionError, Shared, lock};
use crate::command::CommandTarget;
use crate::command::audit::{AuditError, AuditRecord, AuditSink};
use crate::export::report::{ReportAlarms, ReportData, ReportDevice, activations};
use crate::export::{self, ExportDoc};
use crate::historian::AuditRow;
use crate::poll::worker::LinkHandle;
use crate::protocol::rtu::SerialSettings;
use crate::session::live::{TagView, tag_key};
use crate::session::{ExportFormat, link_label};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What the command layer needs to talk to one device.
pub struct CommandAccess {
    pub target: CommandTarget,
    /// Queue of the link worker that also polls this bus (one master per bus).
    pub handle: LinkHandle,
    pub generation: u64,
}

/// Audit rows go to the historian's append-only table. No historian, no write (fail closed).
struct HistorianAudit(Arc<Shared>);

impl AuditSink for HistorianAudit {
    fn append(&mut self, record: &AuditRecord) -> Result<(), AuditError> {
        let Some(h) = &self.0.historian else {
            return Err(AuditError("historian unavailable; commands are disabled".into()));
        };
        lock(h).append_audit(record).map_err(|e| AuditError(e.to_string()))
    }
}

impl Session {
    /// The session clock (UTC for records, monotonic for deadlines).
    pub fn clock(&self) -> Arc<dyn crate::clock::Clock> {
        Arc::clone(&self.shared.clock)
    }

    /// Current generation and state of a link (`None` once removed).
    pub fn link_generation(&self, link_id: &str) -> Option<(u64, LinkState)> {
        lock(&self.shared.links).get(link_id).map(|e| (e.generation, e.state))
    }

    /// Target and worker handle for a device. The link must be polling.
    pub fn command_access(&self, device_id: &str) -> Result<CommandAccess, SessionError> {
        let links = lock(&self.shared.links);
        for e in links.values() {
            let Some((d, p)) = e.devices.iter().find(|(d, _)| d.id == device_id) else {
                continue;
            };
            let Some(w) = e.worker.as_ref().filter(|_| e.state == LinkState::Polling) else {
                return Err(SessionError::NotConnected(e.config.id.clone()));
            };
            let timeout = d.timeout_ms(p, &e.config);
            return Ok(CommandAccess {
                target: CommandTarget {
                    device: d.clone(),
                    link: e.config.clone(),
                    profile: Arc::clone(p),
                    session_simulator: e.simulated && e.config.is_simulator(),
                },
                handle: w.handle(timeout),
                generation: e.generation,
            });
        }
        Err(SessionError::UnknownDevice(device_id.into()))
    }

    /// Point a device at a new unit ID (commissioning, after the device accepted it).
    /// Restarts polling if it was running; drops any arm by changing the link generation.
    pub fn set_device_unit(&self, device_id: &str, unit: u8) -> Result<(), SessionError> {
        let link_id = {
            let mut links = lock(&self.shared.links);
            let e = links
                .values_mut()
                .find(|e| e.devices.iter().any(|(d, _)| d.id == device_id))
                .ok_or_else(|| SessionError::UnknownDevice(device_id.into()))?;
            if unit == 0 || unit > e.config.max_unit_id() {
                return Err(SessionError::Settings(format!(
                    "unit ID {unit} is outside 1..{}",
                    e.config.max_unit_id()
                )));
            }
            if e.devices.iter().any(|(d, _)| d.id != device_id && d.unit_id == unit) {
                return Err(SessionError::UnitTaken(unit));
            }
            let old = e
                .devices
                .iter()
                .find(|(d, _)| d.id == device_id)
                .map(|(d, _)| d.unit_id)
                .unwrap_or(unit);
            let polling = e.state == LinkState::Polling;
            if polling {
                self.shared.stop_worker(e);
            }
            for (d, _) in e.devices.iter_mut().filter(|(d, _)| d.id == device_id) {
                d.unit_id = unit;
            }
            e.generation = self.shared.next_generation();
            if polling {
                e.state = super::LinkState::Connected;
                self.shared.spawn_worker(e)?;
            }
            (e.config.id.clone(), old)
        };
        self.shared.bump_links();
        self.shared.event(
            "commissioning",
            Some(device_id),
            None,
            "info",
            format!(
                "device {device_id} on link {}: unit ID {} -> {unit}",
                link_id.0, link_id.1
            ),
        );
        Ok(())
    }

    /// Change a serial link's settings: stop polling, close, reopen with the new
    /// settings, and resume polling (reads only) if it was polling. Drops any arm.
    pub fn reconfigure_serial(&self, link_id: &str, serial: SerialSettings) -> Result<(), SessionError> {
        serial.validate().map_err(|e| SessionError::Settings(e.into()))?;
        let (label, result) = {
            let mut links = lock(&self.shared.links);
            let e = links
                .get_mut(link_id)
                .ok_or_else(|| SessionError::UnknownLink(link_id.into()))?;
            let slot = match &mut e.config.kind {
                crate::device::LinkKind::Rtu { serial: s, .. }
                | crate::device::LinkKind::RtuOverTcp { serial: s, .. }
                | crate::device::LinkKind::Simulator { serial: s, .. } => s,
                crate::device::LinkKind::Tcp { .. } => {
                    return Err(SessionError::Settings(
                        "a Modbus TCP link has no serial settings".into(),
                    ));
                }
            };
            *slot = serial;
            let polling = e.state == LinkState::Polling;
            self.shared.stop_worker(e);
            if let Some(t) = e.transport.take() {
                lock(&t).close();
            }
            e.generation = self.shared.next_generation();
            let result = match (self.shared.factory)(&e.config) {
                Ok(t) => {
                    e.transport = Some(Arc::new(std::sync::Mutex::new(t)));
                    e.state = LinkState::Connected;
                    e.error = None;
                    if polling { self.shared.spawn_worker(e) } else { Ok(()) }
                }
                Err(err) => {
                    e.state = LinkState::Down;
                    e.error = Some(err.to_string());
                    Err(SessionError::Transport(err))
                }
            };
            (link_label(&e.config), result)
        };
        self.shared.bump_links();
        self.shared.event(
            "commissioning",
            None,
            None,
            "info",
            format!("link {link_id} settings changed to {label}"),
        );
        result
    }

    /// Append-only audit destination for the command service.
    pub fn audit_sink(&self) -> Box<dyn AuditSink> {
        Box::new(HistorianAudit(Arc::clone(&self.shared)))
    }

    /// Every audit row of this session, oldest first.
    pub fn audit_rows(&self) -> Result<Vec<AuditRow>, SessionError> {
        let h = self
            .shared
            .historian
            .as_ref()
            .ok_or_else(|| SessionError::Historian(self.historian_error_text()))?;
        lock(h)
            .audit(&self.shared.session_id)
            .map_err(|e| SessionError::Historian(e.to_string()))
    }

    /// Audit rows as CSV or XLSX (JSON is part of the session export).
    pub fn audit_export(&self, format: ExportFormat) -> Result<Vec<u8>, SessionError> {
        let rows = self.audit_rows()?;
        match format {
            ExportFormat::Csv => Ok(export::csv::audit(&rows).into_bytes()),
            ExportFormat::Xlsx => export::xlsx::audit_workbook(&rows).map_err(|e| SessionError::Export(e.to_string())),
            ExportFormat::Json => Err(SessionError::Export("audit export is CSV or XLSX".into())),
        }
    }

    /// Latest view of one tag.
    pub fn tag(&self, device_id: &str, tag_id: &str) -> Option<TagView> {
        lock(&self.shared.live).get(&tag_key(device_id, tag_id)).cloned()
    }

    /// An event row in the historian (commissioning steps).
    pub fn log_event(&self, kind: &str, device: Option<&str>, severity: &str, text: String) {
        self.shared.event(kind, device, None, severity, text);
    }

    /// Facts for the session report (R19).
    pub fn report(&self) -> Result<ReportData, SessionError> {
        self.shared.flush_log();
        let now = self.shared.now();
        let h = self
            .shared
            .historian
            .as_ref()
            .ok_or_else(|| SessionError::Historian(self.historian_error_text()))?;
        let (doc, counts) = {
            let h = lock(h);
            let doc = ExportDoc::from_historian(&h, &self.shared.session_id, now, &self.shared.app_version)
                .map_err(|e| SessionError::Export(e.to_string()))?;
            let counts = h
                .quality_counts(&self.shared.session_id)
                .map_err(|e| SessionError::Historian(e.to_string()))?;
            (doc, counts)
        };
        let live: BTreeMap<String, _> = {
            let links = lock(&self.shared.links);
            links
                .values()
                .flat_map(|e| {
                    e.devices
                        .iter()
                        .map(|(d, p)| (d.id.clone(), self.shared.device_view(e, d, p)))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        let profiles = self.profiles();
        let devices = doc
            .devices
            .iter()
            .map(|d| {
                let profile = profiles
                    .iter()
                    .find(|p| p.profile_id == d.profile_id && p.profile_version == d.profile_version);
                let view = live.get(&d.device_id);
                ReportDevice {
                    device_id: d.device_id.clone(),
                    name: d.name.clone(),
                    link: d.link.clone(),
                    unit_id: view.map_or(d.unit_id, |v| v.unit_id),
                    profile_id: d.profile_id.clone(),
                    profile_version: d.profile_version.clone(),
                    support_level: profile.map(|p| p.support_level),
                    simulated_only: profile.is_some_and(|p| p.simulated_only),
                    counters: view.map(|v| v.counters.clone()),
                    quality: counts
                        .iter()
                        .filter(|(dev, _, _)| *dev == d.device_id)
                        .map(|(_, q, n)| (q.clone(), *n))
                        .collect(),
                }
            })
            .collect();
        let alarm_events: Vec<_> = doc.events.iter().filter(|e| e.kind == "alarm").cloned().collect();
        let other_events: Vec<_> = doc.events.iter().filter(|e| e.kind != "alarm").cloned().collect();
        Ok(ReportData {
            generated_ms: now,
            app_version: self.shared.app_version.clone(),
            session: doc.session.clone(),
            devices,
            alarms: ReportAlarms {
                summary: lock(&self.shared.alarms).snapshot(now).summary,
                activations: activations(&alarm_events),
                events: alarm_events,
            },
            audit: doc.commands.clone(),
            events: other_events,
        })
    }
}
