//! `Control`: what the app shell calls for control, commissioning and process
//! feedback (commissioning-control design §2–§5). It owns the one
//! `CommandService` of the app and talks to devices only through the link
//! worker's queue (`Session::command_access`).
//!
//! Every call first checks the armed link's generation: a disconnect,
//! reconnect, link loss or settings change since arming drops the arm, and
//! nothing resumes on its own (safety rules 3 and 6).

use super::audit::AuditEvent;
use super::commission::{CommSettings, StepError, Wizard, WizardStep};
use super::feedback::{FeedbackWatch, Observation};
use super::{
    Ack, CommSettingKind, CommandConfig, CommandError, CommandLink, CommandOutcome, CommandRequest, CommandService,
    CommandState, CommandTarget, DisarmReason, Operator, Preview, Readback, Role,
};
use crate::profile::CapabilityLevel;
use crate::protocol::pdu::ReadRequest;
use crate::session::{LinkState, Session, SessionError};
use serde::Serialize;
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ControlError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Step(#[from] StepError),
    #[error("{0}")]
    Rejected(String),
}

/// The last command: its preview, outcome and process feedback, kept for the panel.
#[derive(Clone, Debug, Serialize)]
pub struct LastCommand {
    pub preview: Preview,
    pub sent: bool,
    pub outcome: Option<CommandOutcome>,
    pub feedback: Option<FeedbackWatch>,
    #[serde(skip)]
    target: Option<CommandTarget>,
    #[serde(skip)]
    operator: Option<Operator>,
    #[serde(skip)]
    feedback_logged: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControlStatus {
    pub state: CommandState,
    pub armed_device: Option<String>,
    pub armed_user: Option<String>,
    pub armed_role: Option<Role>,
    pub armed_remaining_ms: Option<u64>,
    pub arm_timeout_ms: u64,
    /// Why the last arm ended (shown once the panel notices).
    pub disarm_reason: Option<DisarmReason>,
    pub last: Option<LastCommand>,
    pub wizard: Option<Wizard>,
}

struct Inner {
    svc: CommandService,
    armed_link: Option<(String, u64)>,
    disarm_reason: Option<DisarmReason>,
    last: Option<LastCommand>,
    wizard: Option<Wizard>,
    wizard_target: Option<(CommandTarget, Operator)>,
}

pub struct Control {
    session: Arc<Session>,
    inner: Mutex<Inner>,
}

fn reason_for(state: Option<LinkState>, same_generation: bool) -> DisarmReason {
    match state {
        None | Some(LinkState::Disconnected) => DisarmReason::Disconnect,
        Some(LinkState::Down) => DisarmReason::LinkLost,
        Some(_) if !same_generation => DisarmReason::Reconnect,
        // Polling stopped by the user: the bus is no longer owned by a worker.
        Some(_) => DisarmReason::Disconnect,
    }
}

impl Control {
    pub fn new(session: Arc<Session>, config: CommandConfig) -> Self {
        let svc = CommandService::new(session.clock(), session.audit_sink(), session.session_id(), config);
        Self {
            session,
            inner: Mutex::new(Inner {
                svc,
                armed_link: None,
                disarm_reason: None,
                last: None,
                wizard: None,
                wizard_target: None,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drop the arm when the link changed since arming, or when its idle time ran out.
    fn check(&self, g: &mut Inner) {
        if let Some((link, gen_at_arm)) = g.armed_link.clone() {
            let now = self.session.link_generation(&link);
            let ok = now.is_some_and(|(gen_now, st)| gen_now == gen_at_arm && st == LinkState::Polling);
            if !ok {
                let reason = reason_for(now.map(|n| n.1), now.is_some_and(|n| n.0 == gen_at_arm));
                g.svc.disarm(reason);
                g.armed_link = None;
                g.disarm_reason = Some(reason);
            }
        }
        if g.armed_link.is_some() && g.svc.expire_if_due() {
            g.armed_link = None;
            g.disarm_reason = Some(DisarmReason::Expired);
        }
        if g.svc.armed_device().is_none() {
            if g.armed_link.is_some() && g.disarm_reason.is_none() {
                // Dropped inside the service: idle expiry, or link loss during a send.
                let lost = g
                    .last
                    .as_ref()
                    .and_then(|l| l.outcome.as_ref())
                    .is_some_and(|o| matches!(o.ack, Ack::LinkLost | Ack::NotSent));
                g.disarm_reason = Some(if lost {
                    DisarmReason::LinkLost
                } else {
                    DisarmReason::Expired
                });
            }
            g.armed_link = None;
        }
    }

    fn evaluate_feedback(&self, g: &mut Inner) {
        let Some(last) = g.last.as_mut() else { return };
        let Some(w) = last.feedback.as_mut() else { return };
        if last.feedback_logged {
            return;
        }
        let now = self.session.clock().utc_ms();
        let obs = self.session.tag(&w.device_id, &w.register).map(|t| Observation {
            ts_ms: t.ts_ms,
            good: t.quality == "GOOD",
            value: if w.bit.is_some() {
                t.raw.as_ref().and_then(|r| r.first()).map(|v| f64::from(*v))
            } else {
                t.value
            },
        });
        let st = w.observe(obs, now);
        if st.is_final() {
            last.feedback_logged = true;
            let w = w.clone();
            let (Some(target), Some(op)) = (last.target.clone(), last.operator.clone()) else {
                return;
            };
            let preview_id = last.preview.id;
            let command_id = last.preview.command_id.clone();
            // Best effort: the command already happened; a failed row shows in the historian error.
            let _ = g.svc.note(AuditEvent::Feedback, &target, &op, |r| {
                r.preview_id = Some(preview_id);
                r.command_id = command_id;
                r.register_id = Some(w.register.clone());
                r.requested_value = Some(w.target);
                r.readback_value = w.last_value;
                r.readback = Some(st.as_str().into());
                r.note = Some(format!(
                    "process feedback {} (target {} ± {}, {} ms window)",
                    st.as_str(),
                    w.target,
                    w.tolerance,
                    w.deadline_ms - w.started_ms
                ));
            });
        }
    }

    pub fn status(&self) -> ControlStatus {
        let mut g = self.lock();
        self.check(&mut g);
        self.evaluate_feedback(&mut g);
        ControlStatus {
            state: g.svc.state(),
            armed_device: g.svc.armed_device().map(str::to_owned),
            armed_user: g.svc.armed_operator().map(|o| o.user.clone()),
            armed_role: g.svc.armed_operator().map(|o| o.role),
            armed_remaining_ms: g.svc.armed_remaining_ms(),
            arm_timeout_ms: g.svc.arm_timeout_ms(),
            disarm_reason: g.disarm_reason,
            last: g.last.clone(),
            wizard: g.wizard.clone(),
        }
    }

    pub fn arm(&self, device_id: &str, user: &str, role: Role) -> Result<ControlStatus, ControlError> {
        let user = user.trim();
        if user.is_empty() || user.chars().count() > 64 || user.chars().any(char::is_control) {
            return Err(ControlError::Rejected("enter a user name (1-64 characters)".into()));
        }
        {
            let mut g = self.lock();
            self.check(&mut g);
            let access = self.session.command_access(device_id)?;
            let link = access.target.link.id.clone();
            g.svc.arm(
                access.target,
                Operator {
                    user: user.to_owned(),
                    role,
                },
            )?;
            g.armed_link = Some((link, access.generation));
            g.disarm_reason = None;
        }
        Ok(self.status())
    }

    pub fn disarm(&self, reason: DisarmReason) -> ControlStatus {
        {
            let mut g = self.lock();
            if g.svc.armed_device().is_some() {
                g.svc.disarm(reason);
                g.disarm_reason = Some(reason);
            }
            g.armed_link = None;
        }
        self.status()
    }

    fn armed_access(
        &self,
        g: &mut Inner,
        device_id: &str,
    ) -> Result<crate::session::access::CommandAccess, ControlError> {
        self.check(g);
        let Some(armed) = g.svc.armed_device().map(str::to_owned) else {
            return Err(CommandError::NotArmed.into());
        };
        if armed != device_id {
            return Err(CommandError::WrongDevice {
                armed,
                requested: device_id.to_owned(),
            }
            .into());
        }
        let access = self.session.command_access(device_id)?;
        if g.armed_link.as_ref().map(|(_, gen_at_arm)| *gen_at_arm) != Some(access.generation) {
            g.svc.disarm(DisarmReason::Reconnect);
            g.armed_link = None;
            g.disarm_reason = Some(DisarmReason::Reconnect);
            return Err(CommandError::NotArmed.into());
        }
        Ok(access)
    }

    /// Check and preview a panel command. Comm-config requests go through the wizard.
    pub fn preview(&self, device_id: &str, request: CommandRequest) -> Result<Preview, ControlError> {
        if matches!(request, CommandRequest::CommConfig { .. }) {
            return Err(ControlError::Rejected("use the commissioning wizard".into()));
        }
        let mut g = self.lock();
        let mut access = self.armed_access(&mut g, device_id)?;
        let p = g.svc.preview(&mut access.handle, request)?;
        g.last = Some(LastCommand {
            preview: p.clone(),
            sent: false,
            outcome: None,
            feedback: None,
            target: None,
            operator: None,
            feedback_logged: false,
        });
        Ok(p)
    }

    pub fn confirm(&self, preview_id: u64) -> Result<ControlStatus, ControlError> {
        {
            let mut g = self.lock();
            self.check(&mut g);
            g.svc.confirm(preview_id)?;
        }
        Ok(self.status())
    }

    pub fn cancel(&self) -> Result<ControlStatus, ControlError> {
        {
            let mut g = self.lock();
            self.check(&mut g);
            g.svc.cancel()?;
            if g.last.as_ref().is_some_and(|l| !l.sent) {
                g.last = None;
            }
        }
        Ok(self.status())
    }

    /// Send the confirmed command once, read back, start the feedback watch.
    pub fn execute(&self) -> Result<CommandOutcome, ControlError> {
        let mut g = self.lock();
        let Some(device) = g.svc.armed_device().map(str::to_owned) else {
            return Err(CommandError::NotArmed.into());
        };
        let mut access = self.armed_access(&mut g, &device)?;
        let preview = g.svc.pending_preview().cloned().ok_or(CommandError::WrongState {
            expected: "confirmed",
            actual: g.svc.state(),
        })?;
        if g.svc.state() != CommandState::Confirmed {
            return Err(CommandError::WrongState {
                expected: "confirmed",
                actual: g.svc.state(),
            }
            .into());
        }
        if preview.comm_change.is_some() && !g.wizard.as_ref().is_some_and(Wizard::can_send) {
            return Err(ControlError::Rejected(
                "this change belongs to a commissioning run that already sent".into(),
            ));
        }
        let operator = g.svc.armed_operator().cloned();
        let target = access.target.clone();
        let started = self.session.clock().utc_ms();
        let outcome = g.svc.execute(&mut access.handle)?;
        if preview.comm_change.is_some()
            && let Some(w) = g.wizard.as_mut()
        {
            w.sent(outcome.ack.clone())?;
        }
        let feedback = preview
            .command_id
            .as_deref()
            .and_then(|c| target.profile.command(c))
            .and_then(|c| c.feedback.as_ref())
            .filter(|_| !matches!(outcome.ack, Ack::Exception(_) | Ack::NotSent))
            .map(|fb| {
                FeedbackWatch::new(
                    preview.command_id.as_deref().unwrap_or_default(),
                    &target.device.id,
                    fb,
                    preview.new_value,
                    started,
                )
            });
        g.last = Some(LastCommand {
            preview,
            sent: true,
            outcome: Some(outcome.clone()),
            feedback,
            target: Some(target),
            operator,
            feedback_logged: false,
        });
        Ok(outcome)
    }

    // ---- commissioning wizard ----

    /// Validate the change and record the journal (old settings) before anything is armed.
    pub fn wizard_start(
        &self,
        device_id: &str,
        setting: CommSettingKind,
        value: u32,
        user: &str,
    ) -> Result<Wizard, ControlError> {
        let user = user.trim();
        if user.is_empty() || user.chars().count() > 64 || user.chars().any(char::is_control) {
            return Err(ControlError::Rejected("enter a user name (1-64 characters)".into()));
        }
        let mut g = self.lock();
        self.check(&mut g);
        if g.wizard.as_ref().is_some_and(|w| !w.is_finished() && !w.can_send()) {
            return Err(ControlError::Rejected(
                "a commissioning run is waiting for reconnect or verification; finish or revert it first".into(),
            ));
        }
        let access = self.session.command_access(device_id)?;
        let t = &access.target;
        let p = &t.profile;
        let spec = p
            .comm_config
            .as_ref()
            .and_then(|c| setting.of(c))
            .ok_or_else(|| ControlError::Rejected("the profile does not document this setting".into()))?;
        if p.capability(|c| c.comm_config) == CapabilityLevel::Unsupported {
            return Err(ControlError::Rejected(
                "the profile does not declare comm_config support".into(),
            ));
        }
        let old = CommSettings {
            unit_id: t.device.unit_id,
            serial: t.link.serial(),
        };
        let links = self.session.links();
        let others: Vec<(String, u8)> = links
            .iter()
            .filter(|l| l.id == t.link.id)
            .flat_map(|l| {
                l.devices
                    .iter()
                    .filter(|d| d.id != device_id)
                    .map(|d| (d.id.clone(), d.unit_id))
            })
            .collect();
        match setting {
            CommSettingKind::UnitId => {
                if value == 0 {
                    return Err(CommandError::BroadcastRejected.into());
                }
                if value > 247 {
                    return Err(ControlError::Rejected("unit ID must be 1-247".into()));
                }
                if value == u32::from(old.unit_id) {
                    return Err(ControlError::Rejected("that is the current unit ID".into()));
                }
                if let Some((id, _)) = others.iter().find(|(_, u)| u32::from(*u) == value) {
                    return Err(ControlError::Rejected(format!(
                        "unit ID {value} is already used by {id} on this link"
                    )));
                }
            }
            CommSettingKind::Baud => {
                let Some(serial) = old.serial else {
                    return Err(ControlError::Rejected("a Modbus TCP link has no baud rate".into()));
                };
                if !spec.codes.as_ref().is_some_and(|c| c.values().any(|b| *b == value)) {
                    return Err(ControlError::Rejected(format!(
                        "{value} baud is not in the profile's baud codes"
                    )));
                }
                if value == serial.baud {
                    return Err(ControlError::Rejected("that is the current baud rate".into()));
                }
            }
        }
        let operator = Operator {
            user: user.to_owned(),
            role: Role::Commissioner,
        };
        let wizard = Wizard::new(
            device_id,
            &t.device.name,
            &t.link.id,
            setting,
            value,
            spec.apply,
            old,
            match setting {
                CommSettingKind::Baud => others.iter().map(|(id, _)| id.clone()).collect(),
                CommSettingKind::UnitId => Vec::new(),
            },
            user,
        );
        let old_text = settings_text(&old);
        let new_text = settings_text(&wizard.new);
        let reg = spec.register.clone();
        let old_value = match setting {
            CommSettingKind::UnitId => f64::from(old.unit_id),
            CommSettingKind::Baud => old.serial.map_or(0.0, |s| f64::from(s.baud)),
        };
        g.svc
            .note(AuditEvent::Journal, t, &operator, |r| {
                r.register_id = Some(reg);
                r.old_value = Some(old_value);
                r.requested_value = Some(f64::from(value));
                r.note = Some(format!(
                    "commissioning journal: {} change; old {old_text}; new {new_text}",
                    setting.as_str()
                ));
            })
            .map_err(CommandError::from)?;
        self.session.log_event(
            "commissioning",
            Some(device_id),
            "info",
            format!(
                "journal: {} change requested; old {old_text}; new {new_text}",
                setting.as_str()
            ),
        );
        g.wizard_target = Some((access.target.clone(), operator));
        g.wizard = Some(wizard.clone());
        Ok(wizard)
    }

    pub fn wizard_preview(&self) -> Result<Preview, ControlError> {
        let mut g = self.lock();
        let w = g
            .wizard
            .clone()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        if !w.can_send() {
            return Err(ControlError::Rejected("this run already sent its change".into()));
        }
        let mut access = self.armed_access(&mut g, &w.device_id)?;
        let p = g.svc.preview(
            &mut access.handle,
            CommandRequest::CommConfig {
                setting: w.setting,
                value: w.value,
            },
        )?;
        g.last = Some(LastCommand {
            preview: p.clone(),
            sent: false,
            outcome: None,
            feedback: None,
            target: None,
            operator: None,
            feedback_logged: false,
        });
        Ok(p)
    }

    pub fn wizard_power_cycled(&self) -> Result<Wizard, ControlError> {
        let mut g = self.lock();
        let w = g
            .wizard
            .as_mut()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        w.power_cycled()?;
        Ok(w.clone())
    }

    /// Move the session to the new settings (no write). Drops the arm.
    pub fn wizard_reconnect(&self) -> Result<Wizard, ControlError> {
        let mut g = self.lock();
        let w = g
            .wizard
            .clone()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        if w.step != WizardStep::AwaitReconnect {
            return Err(StepError {
                at: w.step,
                need: "a sent change",
            }
            .into());
        }
        if g.svc.armed_device().is_some() {
            g.svc.disarm(DisarmReason::Reconnect);
            g.disarm_reason = Some(DisarmReason::Reconnect);
        }
        g.armed_link = None;
        match w.setting {
            CommSettingKind::UnitId => self.session.set_device_unit(&w.device_id, w.new.unit_id)?,
            CommSettingKind::Baud => {
                let serial = w
                    .new
                    .serial
                    .ok_or_else(|| ControlError::Rejected("no serial settings".into()))?;
                self.session.reconfigure_serial(&w.link_id, serial)?;
            }
        }
        let w = g
            .wizard
            .as_mut()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        w.reconnected()?;
        Ok(w.clone())
    }

    /// Read the setting's register at the new settings and mark pass or fail.
    pub fn wizard_verify(&self) -> Result<Wizard, ControlError> {
        let mut g = self.lock();
        let w = g
            .wizard
            .clone()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        if w.step != WizardStep::Reconnected {
            return Err(StepError {
                at: w.step,
                need: "a reconnect at the new settings",
            }
            .into());
        }
        let mut access = self.session.command_access(&w.device_id)?;
        let profile = Arc::clone(&access.target.profile);
        let spec = profile
            .comm_config
            .as_ref()
            .and_then(|c| w.setting.of(c))
            .ok_or_else(|| ControlError::Rejected("the profile does not document this setting".into()))?;
        let reg = profile
            .register(&spec.register)
            .ok_or_else(|| CommandError::UnknownRegister(spec.register.clone()))?;
        let req =
            ReadRequest::for_table(reg.table, reg.address, 1).map_err(|e| ControlError::Rejected(e.to_string()))?;
        let unit = access.target.device.unit_id;
        let read = CommandLink::read(&mut access.handle, unit, req)
            .map_err(|e| e.to_string())
            .and_then(|d| d.as_words().first().copied().ok_or_else(|| "empty reply".to_owned()))
            .and_then(|raw| match w.setting {
                CommSettingKind::UnitId => Ok(u32::from(raw)),
                CommSettingKind::Baud => spec
                    .codes
                    .as_ref()
                    .and_then(|c| c.get(&raw.to_string()).copied())
                    .ok_or_else(|| format!("baud code {raw} is not in the profile")),
            });
        let wz = g
            .wizard
            .as_mut()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        wz.verified(read.clone())?;
        let wz = wz.clone();
        let verdict = match (&read, wz.step) {
            (_, WizardStep::Passed) => Readback::Verified,
            (Ok(_), _) => Readback::Mismatch,
            (Err(_), _) => Readback::Unknown,
        };
        if let Some((_, op)) = g.wizard_target.clone() {
            let target = access.target.clone();
            let register = spec.register.clone();
            g.svc
                .note(AuditEvent::Verify, &target, &op, |r| {
                    r.register_id = Some(register);
                    r.requested_value = Some(f64::from(wz.value));
                    r.readback_value = read.as_ref().ok().map(|v| f64::from(*v));
                    r.readback = Some(
                        match verdict {
                            Readback::Verified => "verified",
                            Readback::Mismatch => "mismatch",
                            Readback::Unknown => "unknown",
                        }
                        .into(),
                    );
                    r.note = Some(match wz.step {
                        WizardStep::Passed => format!("commissioning pass at {}", settings_text(&wz.new)),
                        _ => format!(
                            "commissioning fail at {}: {}; old settings {}",
                            settings_text(&wz.new),
                            wz.message.clone().unwrap_or_default(),
                            settings_text(&wz.old)
                        ),
                    });
                })
                .map_err(CommandError::from)?;
        }
        self.session.log_event(
            "commissioning",
            Some(&wz.device_id),
            if wz.step == WizardStep::Passed {
                "info"
            } else {
                "warning"
            },
            format!("verify: {:?} at {}", wz.step, settings_text(&wz.new)),
        );
        Ok(wz)
    }

    /// Return the session to the old settings after a failure (no write is sent).
    pub fn wizard_revert(&self) -> Result<Wizard, ControlError> {
        let mut g = self.lock();
        let w = g
            .wizard
            .clone()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        if !matches!(
            w.step,
            WizardStep::Failed | WizardStep::NeedsVerification | WizardStep::Reconnected
        ) {
            return Err(StepError {
                at: w.step,
                need: "a failed or unverified run",
            }
            .into());
        }
        if g.svc.armed_device().is_some() {
            g.svc.disarm(DisarmReason::Reconnect);
        }
        g.armed_link = None;
        let current = self
            .session
            .command_access(&w.device_id)
            .ok()
            .map(|a| (a.target.device.unit_id, a.target.link.serial(), a.target.clone()));
        if let Some((unit, serial, _)) = &current {
            if *unit != w.old.unit_id {
                self.session.set_device_unit(&w.device_id, w.old.unit_id)?;
            }
            if let (Some(now), Some(old)) = (serial, w.old.serial)
                && *now != old
            {
                self.session.reconfigure_serial(&w.link_id, old)?;
            }
        } else if let Some(old) = w.old.serial {
            // Link not polling: still put the stored settings back.
            self.session.reconfigure_serial(&w.link_id, old)?;
        }
        let wz = g
            .wizard
            .as_mut()
            .ok_or_else(|| ControlError::Rejected("no commissioning run".into()))?;
        wz.reverted()?;
        let wz = wz.clone();
        if let Some((target, op)) = g.wizard_target.clone() {
            let _ = g.svc.note(AuditEvent::Journal, &target, &op, |r| {
                r.note = Some(format!(
                    "commissioning revert: link returned to the old settings {} (no write sent)",
                    settings_text(&wz.old)
                ));
            });
        }
        self.session.log_event(
            "commissioning",
            Some(&wz.device_id),
            "warning",
            format!("revert: link back to {}", settings_text(&wz.old)),
        );
        Ok(wz)
    }

    /// Close a finished run, or one that has not sent anything yet.
    pub fn wizard_close(&self) -> Result<ControlStatus, ControlError> {
        {
            let mut g = self.lock();
            if let Some(w) = &g.wizard {
                if !w.is_finished() && !w.can_send() {
                    return Err(ControlError::Rejected(
                        "the change was sent; reconnect and verify, or revert, before closing".into(),
                    ));
                }
                if w.can_send() && g.svc.pending_preview().is_some_and(|p| p.comm_change.is_some()) {
                    let _ = g.svc.cancel();
                }
            }
            g.wizard = None;
            g.wizard_target = None;
        }
        Ok(self.status())
    }

    /// Is the wizard offered for this device, and why not?
    pub fn wizard_support(&self, device_id: &str) -> Result<Vec<CommSettingKind>, String> {
        let access = self.session.command_access(device_id).map_err(|e| e.to_string())?;
        let p = &access.target.profile;
        if p.capability(|c| c.comm_config) == CapabilityLevel::Unsupported {
            return Err("the profile does not document communication settings".into());
        }
        let c = p
            .comm_config
            .as_ref()
            .ok_or_else(|| "the profile does not document communication settings".to_owned())?;
        let mut out = Vec::new();
        if c.unit_id.is_some() {
            out.push(CommSettingKind::UnitId);
        }
        if c.baud.is_some() && access.target.link.serial().is_some() {
            out.push(CommSettingKind::Baud);
        }
        Ok(out)
    }
}

fn settings_text(s: &CommSettings) -> String {
    match s.serial {
        Some(sr) => {
            let p = match sr.parity {
                crate::protocol::rtu::Parity::None => 'N',
                crate::protocol::rtu::Parity::Even => 'E',
                crate::protocol::rtu::Parity::Odd => 'O',
            };
            format!("unit {}, {} {}{}{}", s.unit_id, sr.baud, sr.data_bits, p, sr.stop_bits)
        }
        None => format!("unit {}", s.unit_id),
    }
}
