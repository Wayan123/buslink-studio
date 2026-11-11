//! Commissioning wizard state (PRD R17, commissioning-control design §4).
//!
//! One run changes one setting (unit ID or baud) of one device. Every step is
//! an explicit operator action; nothing advances on a timer and the write is
//! sent at most once per run (the command service enforces that). The old
//! settings stay in the run, so the recovery hint is always available.

use super::{Ack, CommSettingKind};
use crate::profile::Apply;
use crate::protocol::rtu::SerialSettings;
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CommSettings {
    pub unit_id: u8,
    /// `None` on Modbus TCP links.
    pub serial: Option<SerialSettings>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardStep {
    /// Old settings recorded; arm and preview next.
    Journaled,
    /// The frame was sent once.
    AwaitPowerCycle,
    AwaitReconnect,
    Reconnected,
    Passed,
    Failed,
    /// `apply: unknown`: the wizard stops here instead of guessing.
    NeedsVerification,
    /// The link was returned to the old settings (no write).
    Reverted,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("the wizard is at step {at:?}; this action needs {need}")]
pub struct StepError {
    pub at: WizardStep,
    pub need: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Wizard {
    pub device_id: String,
    pub device_name: String,
    pub link_id: String,
    pub setting: CommSettingKind,
    /// New unit ID or new baud rate.
    pub value: u32,
    pub apply: Apply,
    pub old: CommSettings,
    pub new: CommSettings,
    pub step: WizardStep,
    /// Acknowledgement of the single write, once sent.
    pub ack: Option<Ack>,
    /// What the verification read returned (unit ID or baud), if anything.
    pub verified_value: Option<u32>,
    pub message: Option<String>,
    /// Other devices on the link; a baud change makes them unreachable until they are changed too.
    pub other_devices: Vec<String>,
    pub user: String,
}

impl Wizard {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device_id: &str,
        device_name: &str,
        link_id: &str,
        setting: CommSettingKind,
        value: u32,
        apply: Apply,
        old: CommSettings,
        other_devices: Vec<String>,
        user: &str,
    ) -> Self {
        let mut new = old;
        match setting {
            CommSettingKind::UnitId => new.unit_id = u8::try_from(value).unwrap_or(old.unit_id),
            CommSettingKind::Baud => {
                if let Some(s) = &mut new.serial {
                    s.baud = value;
                }
            }
        }
        Self {
            device_id: device_id.to_owned(),
            device_name: device_name.to_owned(),
            link_id: link_id.to_owned(),
            setting,
            value,
            apply,
            old,
            new,
            step: WizardStep::Journaled,
            ack: None,
            verified_value: None,
            message: None,
            other_devices,
            user: user.to_owned(),
        }
    }

    fn need(&self, ok: bool, need: &'static str) -> Result<(), StepError> {
        if ok {
            Ok(())
        } else {
            Err(StepError { at: self.step, need })
        }
    }

    /// True while the single write may still be previewed and sent.
    pub fn can_send(&self) -> bool {
        self.step == WizardStep::Journaled && self.ack.is_none()
    }

    /// The write went out (or not); decide the next step from the acknowledgement and `apply`.
    pub fn sent(&mut self, ack: Ack) -> Result<(), StepError> {
        self.need(self.can_send(), "the journal step (a run sends once)")?;
        self.step = match &ack {
            Ack::Exception(c) => {
                self.message = Some(format!(
                    "the device refused the change (exception {c:02X}); settings unchanged"
                ));
                WizardStep::Failed
            }
            Ack::NotSent => {
                self.message = Some("not sent: the link was down; settings unchanged".into());
                WizardStep::Failed
            }
            _ => match self.apply {
                Apply::Immediate => WizardStep::AwaitReconnect,
                Apply::AfterReboot => WizardStep::AwaitPowerCycle,
                Apply::Unknown => {
                    self.message = Some("the profile does not say when the change applies; verify by hand".into());
                    WizardStep::NeedsVerification
                }
            },
        };
        if !matches!(ack, Ack::Acknowledged | Ack::Exception(_) | Ack::NotSent) {
            self.message = Some(format!(
                "effect unknown ({ack:?}); reconnect at the new settings to check"
            ));
        }
        self.ack = Some(ack);
        Ok(())
    }

    pub fn power_cycled(&mut self) -> Result<(), StepError> {
        self.need(self.step == WizardStep::AwaitPowerCycle, "a power cycle request")?;
        self.step = WizardStep::AwaitReconnect;
        Ok(())
    }

    pub fn reconnected(&mut self) -> Result<(), StepError> {
        self.need(self.step == WizardStep::AwaitReconnect, "a sent change")?;
        self.step = WizardStep::Reconnected;
        Ok(())
    }

    /// Result of reading the setting at the new settings.
    pub fn verified(&mut self, read: Result<u32, String>) -> Result<(), StepError> {
        self.need(self.step == WizardStep::Reconnected, "a reconnect at the new settings")?;
        match read {
            Ok(v) if v == self.value => {
                self.verified_value = Some(v);
                self.step = WizardStep::Passed;
                self.message = None;
            }
            Ok(v) => {
                self.verified_value = Some(v);
                self.step = WizardStep::Failed;
                self.message = Some(format!("the device reports {v}, expected {}", self.value));
            }
            Err(e) => {
                self.step = WizardStep::Failed;
                self.message = Some(format!("no valid answer at the new settings: {e}"));
            }
        }
        Ok(())
    }

    /// Only after a failure or an unverifiable change: the link goes back to the old settings.
    pub fn reverted(&mut self) -> Result<(), StepError> {
        self.need(
            matches!(
                self.step,
                WizardStep::Failed | WizardStep::NeedsVerification | WizardStep::Reconnected
            ),
            "a failed or unverified run",
        )?;
        self.step = WizardStep::Reverted;
        Ok(())
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.step,
            WizardStep::Passed | WizardStep::Failed | WizardStep::NeedsVerification | WizardStep::Reverted
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rtu::SerialSettings;

    fn wiz(apply: Apply) -> Wizard {
        Wizard::new(
            "vfd",
            "VFD",
            "training",
            CommSettingKind::UnitId,
            5,
            apply,
            CommSettings {
                unit_id: 2,
                serial: Some(SerialSettings::default_9600_8n1()),
            },
            vec![],
            "tech",
        )
    }

    #[test]
    fn immediate_pass() {
        let mut w = wiz(Apply::Immediate);
        assert_eq!(w.new.unit_id, 5);
        assert!(w.reconnected().is_err(), "nothing sent yet");
        w.sent(Ack::Acknowledged).unwrap();
        assert!(w.sent(Ack::Acknowledged).is_err(), "a run sends once");
        assert_eq!(w.step, WizardStep::AwaitReconnect);
        w.reconnected().unwrap();
        w.verified(Ok(5)).unwrap();
        assert_eq!(w.step, WizardStep::Passed);
        assert!(w.reverted().is_err());
    }

    #[test]
    fn exception_stops_with_settings_unchanged() {
        let mut w = wiz(Apply::Immediate);
        w.sent(Ack::Exception(3)).unwrap();
        assert_eq!(w.step, WizardStep::Failed);
        assert!(w.reconnected().is_err());
    }

    #[test]
    fn unknown_apply_and_after_reboot() {
        let mut w = wiz(Apply::Unknown);
        w.sent(Ack::Timeout).unwrap();
        assert_eq!(w.step, WizardStep::NeedsVerification);
        assert!(w.reconnected().is_err());
        w.reverted().unwrap();
        let mut r = wiz(Apply::AfterReboot);
        r.sent(Ack::Acknowledged).unwrap();
        assert!(r.reconnected().is_err(), "power cycle first");
        r.power_cycled().unwrap();
        r.reconnected().unwrap();
        r.verified(Err("timeout".into())).unwrap();
        assert_eq!(r.step, WizardStep::Failed);
        r.reverted().unwrap();
    }
}
