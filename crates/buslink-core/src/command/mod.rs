//! The command service: the only write path (safety rules 2–8, architecture D8).
//!
//! `Idle → Armed → Previewed → Confirmed → Sent → ReadBack → Done | Failed`.
//! Every transition is audited. A confirmation produces exactly one
//! [`PreparedFrame`]; the frame has no `Clone` and is consumed by
//! [`CommandLink::send`], so the same confirmation cannot be sent twice, and
//! there is no retry path anywhere in this module.

pub mod audit;
pub mod commission;
pub mod control;
pub mod feedback;
pub mod wire;

use crate::clock::Clock;
use crate::codec::{self, DataType};
use crate::device::{DeviceInstance, LinkConfig};
use crate::profile::{
    Apply, CapabilityLevel, CommConfig, CommSetting, CommandKind, Profile, Register, Text, WriteRole,
};
use crate::protocol::pdu::{ExpectedReply, ReadData, ReadRequest, Table, decode_read_response};
use crate::protocol::{rtu, tcp};
use crate::time::hex_spaced;
use crate::transport::Transport;
use crate::transport::exchange::{ExchangeError, ExchangeRecord, Exchanger, Outgoing};
use audit::{AuditError, AuditEvent, AuditRecord, AuditSink};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use wire::WriteRequest;

pub const DEFAULT_ARM_TIMEOUT_MS: u64 = 60_000;

/// A confirmed write, ready for exactly one transmission. Only this module can build one.
#[derive(Debug, PartialEq, Eq)]
pub struct PreparedFrame {
    preview_id: u64,
    unit: u8,
    request: WriteRequest,
    pdu: Vec<u8>,
}

impl PreparedFrame {
    pub fn unit(&self) -> u8 {
        self.unit
    }
    pub fn pdu(&self) -> &[u8] {
        &self.pdu
    }
    pub fn expected_reply(&self) -> ExpectedReply {
        self.request.expected_reply()
    }
    pub fn preview_id(&self) -> u64 {
        self.preview_id
    }
    pub fn function_code(&self) -> u8 {
        self.request.function_code()
    }
}

/// How the command service reaches the device: through the link worker's
/// queue in the app, or directly over a transport in tests.
pub trait CommandLink {
    /// A read at command priority (old value, read-back).
    fn read(&mut self, unit: u8, request: ReadRequest) -> Result<ReadData, ExchangeError>;
    /// Send a confirmed frame once. Implementations must never retry.
    fn send(&mut self, frame: PreparedFrame) -> ExchangeRecord;
}

/// `CommandLink` straight over a transport (tests, simulator tools).
pub struct DirectLink<'a> {
    pub transport: &'a mut dyn Transport,
    pub exchanger: &'a mut Exchanger,
    pub timeout_ms: u64,
}

impl CommandLink for DirectLink<'_> {
    fn read(&mut self, unit: u8, request: ReadRequest) -> Result<ReadData, ExchangeError> {
        let rec = self
            .exchanger
            .transact(self.transport, unit, Outgoing::Read(&request), self.timeout_ms);
        let pdu = rec.outcome?;
        decode_read_response(&request, &pdu).map_err(|e| match e {
            crate::protocol::pdu::ReplyError::Exception(c) => ExchangeError::Exception(c),
            crate::protocol::pdu::ReplyError::Frame(f) => ExchangeError::Decode(f),
        })
    }

    fn send(&mut self, frame: PreparedFrame) -> ExchangeRecord {
        let unit = frame.unit;
        self.exchanger
            .transact(self.transport, unit, Outgoing::Command(&frame), self.timeout_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Operator,
    Commissioner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Operator => "operator",
            Role::Commissioner => "commissioner",
        }
    }

    fn satisfies(self, needed: WriteRole) -> bool {
        match needed {
            WriteRole::Operator => self >= Role::Operator,
            WriteRole::Commissioner => self >= Role::Commissioner,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operator {
    pub user: String,
    pub role: Role,
}

/// The device a command is for, with its link and profile.
#[derive(Clone, Debug)]
pub struct CommandTarget {
    pub device: DeviceInstance,
    pub link: LinkConfig,
    pub profile: Arc<Profile>,
    /// Marker set only by `Session::command_access`, for a link the session itself
    /// created for the in-process training simulator (`Session::connect(.., true)`).
    /// `simulated_only` profiles are controllable only when it is set. A TCP link,
    /// even to a loopback address, never carries it (security review 07, M2).
    pub session_simulator: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandRequest {
    /// Write one register to an engineering value (commissioning, setpoints).
    SetRegister { register: String, value: f64 },
    /// A profile command (simulated devices only). Setpoint commands need `value`.
    Named { command: String, value: Option<f64> },
    /// Commissioning: new unit ID or baud through the profile's `comm_config` register.
    CommConfig { setting: CommSettingKind, value: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisarmReason {
    Operator,
    Disconnect,
    Reconnect,
    ProfileChange,
    AppBackground,
    LinkLost,
    /// Idle time ran out (recorded as `arm_expired`, shown in the status).
    Expired,
}

impl DisarmReason {
    fn as_str(self) -> &'static str {
        match self {
            DisarmReason::Operator => "operator",
            DisarmReason::Disconnect => "disconnect",
            DisarmReason::Reconnect => "reconnect",
            DisarmReason::ProfileChange => "profile_change",
            DisarmReason::AppBackground => "app_background",
            DisarmReason::LinkLost => "link_lost",
            DisarmReason::Expired => "expired",
        }
    }
}

/// Everything the confirmation dialog must show (safety rule 3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Preview {
    pub id: u64,
    pub device_id: String,
    pub device_name: String,
    pub simulated: bool,
    pub unit_id: u8,
    pub register_id: String,
    pub register_name: Text,
    pub table: Table,
    /// Manual label (or the conventional label in parentheses when the manual gives none).
    pub doc_address: String,
    /// Zero-based PDU offset.
    pub address: u16,
    pub function: u8,
    pub function_name: String,
    pub command_id: Option<String>,
    pub command_kind: Option<CommandKind>,
    pub command_label: Option<Text>,
    pub old_value: Option<f64>,
    pub old_raw: Option<Vec<u16>>,
    pub old_error: Option<String>,
    pub new_value: f64,
    pub unit: Option<String>,
    pub decimals: Option<u8>,
    pub raw_registers: Vec<u16>,
    pub pdu_hex: String,
    /// The ADU as it will be sent (MBAP previews show transaction ID 0000; the real ID is assigned at send).
    pub frame_hex: String,
    pub readback_register: String,
    pub risk: String,
    /// Set for commissioning requests.
    pub comm_change: Option<CommChangePreview>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Ack {
    Acknowledged,
    Exception(u8),
    /// No reply: the effect is unknown.
    Timeout,
    BadEcho,
    CrcError,
    Invalid(String),
    LinkLost,
    /// The link was already down; the frame never left the queue.
    NotSent,
}

impl Ack {
    fn as_str(&self) -> String {
        match self {
            Ack::Acknowledged => "acknowledged".into(),
            Ack::Exception(c) => format!("exception {c:02X}"),
            Ack::Timeout => "timeout (effect unknown)".into(),
            Ack::BadEcho => "bad echo (effect unknown)".into(),
            Ack::CrcError => "crc error (effect unknown)".into(),
            Ack::Invalid(e) => format!("invalid reply (effect unknown): {e}"),
            Ack::LinkLost => "link lost (effect unknown)".into(),
            Ack::NotSent => "not sent (link down)".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Readback {
    Verified,
    Mismatch,
    Unknown,
}

impl Readback {
    fn as_str(self) -> &'static str {
        match self {
            Readback::Verified => "verified",
            Readback::Mismatch => "mismatch",
            Readback::Unknown => "unknown",
        }
    }
}

/// Acknowledgement and read-back are separate: "write acknowledged" is not "process reached target".
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommandOutcome {
    pub preview_id: u64,
    pub ack: Ack,
    pub readback: Readback,
    pub readback_value: Option<f64>,
    pub readback_raw: Option<Vec<u16>>,
    pub readback_error: Option<String>,
    pub request_hex: String,
    pub response_hex: String,
    /// Set if the final audit record could not be stored (the write already happened).
    pub audit_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    Idle,
    Armed,
    Previewed,
    Confirmed,
    Sent,
    ReadBack,
    Done,
    Failed,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CommandError {
    #[error("control is not armed")]
    NotArmed,
    #[error("arming expired; arm again")]
    ArmExpired,
    #[error("armed for device '{armed}', not '{requested}'")]
    WrongDevice { armed: String, requested: String },
    #[error("role {have} cannot do this (needs {need})")]
    Role { have: &'static str, need: &'static str },
    #[error("register '{0}' does not exist in the profile")]
    UnknownRegister(String),
    #[error("command '{0}' does not exist in the profile")]
    UnknownCommand(String),
    #[error("register '{0}' is not writable")]
    NotWritable(String),
    #[error("value rejected: {0}")]
    OutOfBounds(String),
    #[error("value cannot be encoded: {0}")]
    NotRepresentable(String),
    #[error("type {0} cannot be written by the command service")]
    UnsupportedType(&'static str),
    #[error("unit ID 0 (broadcast) is never written")]
    BroadcastRejected,
    #[error("unit ID {0} is outside the range for this link")]
    UnitRange(u8),
    #[error("{0}")]
    ControlNotAllowed(&'static str),
    #[error("invalid device or profile: {0}")]
    Target(String),
    #[error("command service is {actual:?}, expected {expected}")]
    WrongState {
        expected: &'static str,
        actual: CommandState,
    },
    #[error("confirmation does not match the current preview")]
    PreviewMismatch,
    #[error(transparent)]
    Audit(#[from] AuditError),
    #[error("internal encoding check failed: {0}")]
    Encode(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Compare {
    SameRaw(Vec<u16>),
    Expected(f64, f64),
    Report,
    /// Comm-config change: read back after reconnecting at the new settings.
    Deferred,
}

/// Which communication setting a commissioning request changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommSettingKind {
    UnitId,
    Baud,
}

impl CommSettingKind {
    pub fn of(self, c: &CommConfig) -> Option<&CommSetting> {
        match self {
            CommSettingKind::UnitId => c.unit_id.as_ref(),
            CommSettingKind::Baud => c.baud.as_ref(),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CommSettingKind::UnitId => "unit_id",
            CommSettingKind::Baud => "baud",
        }
    }
}

/// The comm-config part of a preview: the setting as the operator thinks of it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommChangePreview {
    pub setting: CommSettingKind,
    /// New unit ID or new baud rate (not the raw code).
    pub value: u32,
    pub apply: Apply,
}

/// The comm-config setting whose register is `register`, if any.
pub fn comm_register(profile: &Profile, register: &str) -> Option<CommSettingKind> {
    let c = profile.comm_config.as_ref()?;
    [CommSettingKind::UnitId, CommSettingKind::Baud]
        .into_iter()
        .find(|k| k.of(c).is_some_and(|s| s.register == register))
}

/// Register and raw value for a comm-config request (design §3).
fn comm_plan(
    profile: &Profile,
    role: Role,
    setting: CommSettingKind,
    value: u32,
) -> Result<(String, f64), CommandError> {
    let spec = profile
        .comm_config
        .as_ref()
        .and_then(|c| setting.of(c))
        .ok_or(CommandError::ControlNotAllowed(
            "the profile does not document this communication setting",
        ))?;
    if profile.capability(|c| c.comm_config) == CapabilityLevel::Unsupported {
        return Err(CommandError::ControlNotAllowed(
            "the profile does not declare comm_config support",
        ));
    }
    if role < Role::Commissioner {
        return Err(CommandError::Role {
            have: role.as_str(),
            need: "commissioner",
        });
    }
    let raw = match setting {
        CommSettingKind::UnitId => {
            if value == 0 {
                return Err(CommandError::BroadcastRejected);
            }
            if value > 247 {
                return Err(CommandError::OutOfBounds(format!("unit ID {value} is outside 1..247")));
            }
            f64::from(value)
        }
        CommSettingKind::Baud => {
            let code = spec
                .codes
                .as_ref()
                .and_then(|codes| codes.iter().find(|(_, b)| **b == value))
                .and_then(|(code, _)| code.parse::<u32>().ok())
                .ok_or_else(|| CommandError::OutOfBounds(format!("{value} baud has no code in the profile")))?;
            f64::from(code)
        }
    };
    Ok((spec.register.clone(), raw))
}

#[derive(Debug)]
struct Plan {
    preview: Preview,
    frame: PreparedFrame,
    readback_request: ReadRequest,
    readback_type: DataType,
    readback_register: Register,
    compare: Compare,
}

#[derive(Debug)]
enum Inner {
    Idle,
    Armed,
    Previewed(Box<Plan>),
    Confirmed(Box<Plan>),
    Sent,
    ReadBack,
    Done(Box<CommandOutcome>),
    Failed(Box<CommandOutcome>),
}

struct ArmedFor {
    target: CommandTarget,
    operator: Operator,
    expires_mono: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct CommandConfig {
    pub arm_timeout_ms: u64,
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            arm_timeout_ms: DEFAULT_ARM_TIMEOUT_MS,
        }
    }
}

pub struct CommandService {
    clock: Arc<dyn Clock>,
    audit: Box<dyn AuditSink>,
    session_id: String,
    config: CommandConfig,
    armed: Option<ArmedFor>,
    inner: Inner,
    next_preview: u64,
    sends: u64,
}

fn same_value(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn check_bounds(reg: &Register, value: f64) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(CommandError::OutOfBounds("value is not a finite number".into()));
    }
    let Some(w) = &reg.write else {
        return Err(CommandError::NotWritable(reg.id.clone()));
    };
    if let Some(allowed) = &w.allowed_values {
        if !allowed.iter().any(|a| same_value(*a, value)) {
            return Err(CommandError::OutOfBounds(format!("{value} is not one of {allowed:?}")));
        }
    } else if let (Some(min), Some(max)) = (w.min, w.max) {
        if value < min || value > max {
            return Err(CommandError::OutOfBounds(format!("{value} is outside {min}..{max}")));
        }
        if let Some(step) = w.step {
            let k = (value - min) / step;
            if (k - k.round()).abs() > 1e-6 {
                return Err(CommandError::OutOfBounds(format!(
                    "{value} is not a multiple of {step} from {min}"
                )));
            }
        }
    } else if reg.table == Table::Coil {
        if value != 0.0 && value != 1.0 {
            return Err(CommandError::OutOfBounds("a coil takes 0 or 1".into()));
        }
    } else {
        return Err(CommandError::OutOfBounds("the profile gives no write bounds".into()));
    }
    if let Some([lo, hi]) = reg.valid_range
        && (value < lo || value > hi)
    {
        return Err(CommandError::OutOfBounds(format!(
            "{value} is outside the valid range {lo}..{hi}"
        )));
    }
    Ok(())
}

fn read_error_text(e: &ExchangeError) -> String {
    e.to_string()
}

impl CommandService {
    pub fn new(clock: Arc<dyn Clock>, audit: Box<dyn AuditSink>, session_id: &str, config: CommandConfig) -> Self {
        Self {
            clock,
            audit,
            session_id: session_id.to_owned(),
            config,
            armed: None,
            inner: Inner::Idle,
            next_preview: 1,
            sends: 0,
        }
    }

    pub fn state(&self) -> CommandState {
        match self.inner {
            Inner::Idle => CommandState::Idle,
            Inner::Armed => CommandState::Armed,
            Inner::Previewed(_) => CommandState::Previewed,
            Inner::Confirmed(_) => CommandState::Confirmed,
            Inner::Sent => CommandState::Sent,
            Inner::ReadBack => CommandState::ReadBack,
            Inner::Done(_) => CommandState::Done,
            Inner::Failed(_) => CommandState::Failed,
        }
    }

    /// Frames sent since creation (for tests and diagnostics).
    pub fn sends(&self) -> u64 {
        self.sends
    }

    pub fn armed_unit(&self) -> Option<u8> {
        self.armed.as_ref().map(|a| a.target.device.unit_id)
    }

    pub fn armed_device(&self) -> Option<&str> {
        self.armed.as_ref().map(|a| a.target.device.id.as_str())
    }

    pub fn arm_timeout_ms(&self) -> u64 {
        self.config.arm_timeout_ms
    }

    /// Drop an arm whose idle time ran out (records `arm_expired`). True if it expired now.
    pub fn expire_if_due(&mut self) -> bool {
        self.armed.is_some() && self.ensure_armed().is_err()
    }

    pub fn armed_operator(&self) -> Option<&Operator> {
        self.armed.as_ref().map(|a| &a.operator)
    }

    /// The current preview, if one waits for confirmation (or is confirmed and not yet sent).
    pub fn pending_preview(&self) -> Option<&Preview> {
        match &self.inner {
            Inner::Previewed(p) | Inner::Confirmed(p) => Some(&p.preview),
            _ => None,
        }
    }

    /// An audit row that is not part of a command transition (wizard journal and
    /// verification, process feedback). Works armed or not; never sends anything.
    pub fn note(
        &mut self,
        event: AuditEvent,
        target: &CommandTarget,
        operator: &Operator,
        fill: impl FnOnce(&mut AuditRecord),
    ) -> Result<(), AuditError> {
        let mut r = AuditRecord {
            ts_ms: self.clock.utc_ms(),
            session_id: self.session_id.clone(),
            event: Some(event),
            user: operator.user.clone(),
            role: operator.role.as_str().into(),
            device_id: target.device.id.clone(),
            profile_id: target.profile.profile_id.clone(),
            profile_version: target.profile.profile_version.clone(),
            unit_id: Some(target.device.unit_id),
            ..AuditRecord::default()
        };
        fill(&mut r);
        self.audit.append(&r)
    }

    /// Remaining arming time, for "Armed: control enabled for 52 s".
    pub fn armed_remaining_ms(&self) -> Option<u64> {
        let now = self.clock.mono_ms();
        self.armed.as_ref().map(|a| a.expires_mono.saturating_sub(now))
    }

    pub fn last_outcome(&self) -> Option<&CommandOutcome> {
        match &self.inner {
            Inner::Done(o) | Inner::Failed(o) => Some(o),
            _ => None,
        }
    }

    fn base_record(&self, event: AuditEvent) -> AuditRecord {
        let mut r = AuditRecord {
            ts_ms: self.clock.utc_ms(),
            session_id: self.session_id.clone(),
            event: Some(event),
            ..AuditRecord::default()
        };
        if let Some(a) = &self.armed {
            r.user = a.operator.user.clone();
            r.role = a.operator.role.as_str().into();
            r.device_id = a.target.device.id.clone();
            r.profile_id = a.target.profile.profile_id.clone();
            r.profile_version = a.target.profile.profile_version.clone();
            r.unit_id = Some(a.target.device.unit_id);
        }
        r
    }

    fn plan_record(&self, event: AuditEvent, p: &Preview) -> AuditRecord {
        let mut r = self.base_record(event);
        r.function = Some(p.function);
        r.address = Some(p.address);
        r.register_id = Some(p.register_id.clone());
        r.command_id = p.command_id.clone();
        r.preview_id = Some(p.id);
        r.old_value = p.old_value;
        r.requested_value = Some(p.new_value);
        r.raw_value_hex = Some(
            p.raw_registers
                .iter()
                .map(|v| format!("{v:04X}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        r.request_hex = Some(p.frame_hex.clone());
        r
    }

    fn refresh(&mut self) {
        let now = self.clock.mono_ms();
        if let Some(a) = &mut self.armed {
            a.expires_mono = now + self.config.arm_timeout_ms;
        }
    }

    /// Fails with `NotArmed`/`ArmExpired` and drops any pending plan when arming is gone.
    fn ensure_armed(&mut self) -> Result<(), CommandError> {
        let Some(a) = &self.armed else {
            return Err(CommandError::NotArmed);
        };
        if self.clock.mono_ms() >= a.expires_mono {
            let rec = self.base_record(AuditEvent::ArmExpired);
            let _ = self.audit.append(&rec); // best effort: nothing is sent either way
            self.armed = None;
            self.inner = Inner::Idle;
            return Err(CommandError::ArmExpired);
        }
        Ok(())
    }

    /// Arm control for one device. Re-arming another device drops the old one.
    pub fn arm(&mut self, target: CommandTarget, operator: Operator) -> Result<(), CommandError> {
        if matches!(self.inner, Inner::Sent | Inner::ReadBack) {
            return Err(CommandError::WrongState {
                expected: "not executing",
                actual: self.state(),
            });
        }
        if operator.role < Role::Operator {
            return Err(CommandError::Role {
                have: operator.role.as_str(),
                need: "operator",
            });
        }
        target
            .device
            .check(&target.profile, &target.link)
            .map_err(|e| CommandError::Target(e.to_string()))?;
        if self.armed.is_some() {
            self.disarm(DisarmReason::Operator);
        }
        let expires_mono = self.clock.mono_ms() + self.config.arm_timeout_ms;
        self.armed = Some(ArmedFor {
            target,
            operator,
            expires_mono,
        });
        let rec = self.base_record(AuditEvent::Arm);
        if let Err(e) = self.audit.append(&rec) {
            self.armed = None;
            return Err(e.into());
        }
        self.inner = Inner::Armed;
        Ok(())
    }

    /// Raise the armed operator to `Commissioner` (a separate, deliberate host action).
    /// Audited before it takes effect (fail closed). The raise lives in the arm: any
    /// disarm, expiry, reconnect or re-arm returns to `Operator`.
    pub fn raise_role(&mut self, role: Role) -> Result<(), CommandError> {
        if matches!(self.inner, Inner::Sent | Inner::ReadBack) {
            return Err(CommandError::WrongState {
                expected: "not executing",
                actual: self.state(),
            });
        }
        self.ensure_armed()?;
        let Some(a) = &self.armed else {
            return Err(CommandError::NotArmed);
        };
        let from = a.operator.role;
        if role <= from {
            return Ok(());
        }
        let mut rec = self.base_record(AuditEvent::RoleUnlock);
        rec.note = Some(format!(
            "role {} -> {} until the arm ends; single-user local app, the role is an interlock, not authentication",
            from.as_str(),
            role.as_str()
        ));
        self.audit.append(&rec)?;
        if let Some(a) = &mut self.armed {
            a.operator.role = role;
        }
        self.refresh();
        Ok(())
    }

    /// Drop arming (operator, disconnect, reconnect, profile change, app background).
    pub fn disarm(&mut self, reason: DisarmReason) {
        if self.armed.is_none() {
            self.inner = Inner::Idle;
            return;
        }
        let mut rec = self.base_record(AuditEvent::Disarm);
        rec.note = Some(reason.as_str().into());
        let _ = self.audit.append(&rec);
        self.armed = None;
        self.inner = Inner::Idle;
    }

    /// Drop a preview or confirmation without sending.
    pub fn cancel(&mut self) -> Result<(), CommandError> {
        match std::mem::replace(&mut self.inner, Inner::Armed) {
            Inner::Previewed(plan) | Inner::Confirmed(plan) => {
                let rec = self.plan_record(AuditEvent::Cancel, &plan.preview);
                let _ = self.audit.append(&rec);
                if self.armed.is_none() {
                    self.inner = Inner::Idle;
                }
                Ok(())
            }
            other => {
                let actual = matches!(other, Inner::Idle);
                self.inner = other;
                if actual { Err(CommandError::NotArmed) } else { Ok(()) }
            }
        }
    }

    fn reject(&mut self, err: CommandError, request: &CommandRequest) -> CommandError {
        let mut rec = self.base_record(AuditEvent::Rejected);
        rec.note = Some(format!("{err} ({request:?})"));
        let _ = self.audit.append(&rec);
        err
    }

    /// Check a request and build the confirmation preview. Reads the current value first.
    pub fn preview(&mut self, link: &mut dyn CommandLink, request: CommandRequest) -> Result<Preview, CommandError> {
        if matches!(self.inner, Inner::Confirmed(_) | Inner::Sent | Inner::ReadBack) {
            return Err(CommandError::WrongState {
                expected: "armed",
                actual: self.state(),
            });
        }
        self.ensure_armed()?;
        match self.build_plan(link, &request) {
            Ok(plan) => {
                let rec = self.plan_record(AuditEvent::Preview, &plan.preview);
                self.audit.append(&rec)?;
                let preview = plan.preview.clone();
                self.inner = Inner::Previewed(Box::new(plan));
                self.refresh();
                Ok(preview)
            }
            Err(e) => Err(self.reject(e, &request)),
        }
    }

    fn build_plan(&mut self, link: &mut dyn CommandLink, request: &CommandRequest) -> Result<Plan, CommandError> {
        let Some(armed) = &self.armed else {
            return Err(CommandError::NotArmed);
        };
        let target = armed.target.clone();
        let role = armed.operator.role;
        let profile = &target.profile;
        let simulator = target.link.is_simulator() && target.session_simulator;
        if profile.simulated_only && !simulator {
            return Err(CommandError::ControlNotAllowed(
                "simulated_only profiles are only written on a simulator link",
            ));
        }
        if profile.capability(|c| c.write) == CapabilityLevel::Unsupported {
            return Err(CommandError::ControlNotAllowed(
                "the profile does not declare write support",
            ));
        }
        let (reg_id, value, command) = match request {
            CommandRequest::SetRegister { register, value } => {
                if comm_register(profile, register).is_some() {
                    return Err(CommandError::ControlNotAllowed(
                        "communication settings are changed only through the commissioning wizard",
                    ));
                }
                (register.clone(), *value, None)
            }
            CommandRequest::CommConfig { setting, value } => {
                let (reg, raw) = comm_plan(profile, role, *setting, *value)?;
                (reg, raw, None)
            }
            CommandRequest::Named { command, value } => {
                let c = profile
                    .command(command)
                    .ok_or_else(|| CommandError::UnknownCommand(command.clone()))?;
                if comm_register(profile, &c.register).is_some() {
                    return Err(CommandError::ControlNotAllowed(
                        "communication settings are changed only through the commissioning wizard",
                    ));
                }
                if !profile.simulated_only || !simulator {
                    return Err(CommandError::ControlNotAllowed(
                        "control commands exist only for simulated devices in this release",
                    ));
                }
                if profile.capability(|c| c.control) == CapabilityLevel::Unsupported {
                    return Err(CommandError::ControlNotAllowed(
                        "the profile does not declare control support",
                    ));
                }
                let v = match (c.kind, c.value, value) {
                    (CommandKind::Setpoint, _, Some(v)) => *v,
                    (CommandKind::Setpoint, _, None) => {
                        return Err(CommandError::OutOfBounds("a setpoint needs a value".into()));
                    }
                    (_, Some(fixed), None) => fixed,
                    (_, Some(fixed), Some(v)) if same_value(fixed, *v) => fixed,
                    (_, _, _) => return Err(CommandError::OutOfBounds("this command has a fixed value".into())),
                };
                (c.register.clone(), v, Some(c.clone()))
            }
        };
        let reg = profile
            .register(&reg_id)
            .ok_or_else(|| CommandError::UnknownRegister(reg_id.clone()))?
            .clone();
        if !reg.is_writable() {
            return Err(CommandError::NotWritable(reg.id.clone()));
        }
        let wspec = reg
            .write
            .clone()
            .ok_or_else(|| CommandError::NotWritable(reg.id.clone()))?;
        if !role.satisfies(wspec.role) {
            let need = match wspec.role {
                WriteRole::Operator => "operator",
                WriteRole::Commissioner => "commissioner",
            };
            return Err(CommandError::Role {
                have: role.as_str(),
                need,
            });
        }
        check_bounds(&reg, value)?;
        let unit = target.device.unit_id;
        if unit == 0 {
            return Err(CommandError::BroadcastRejected);
        }
        if unit > target.link.max_unit_id() {
            return Err(CommandError::UnitRange(unit));
        }
        let dt = reg.data_type().ok_or(CommandError::UnsupportedType("unknown"))?;
        if matches!(dt, DataType::Bitfield | DataType::Str { .. }) {
            return Err(CommandError::UnsupportedType(if dt == DataType::Bitfield {
                "bitfield"
            } else {
                "string"
            }));
        }
        let scaling = reg.scaling();
        let raw = codec::to_raw(dt, value, scaling.scale, scaling.offset)
            .map_err(|e| CommandError::NotRepresentable(e.to_string()))?;
        let regs =
            codec::encode(dt, reg.word_order, &raw).map_err(|e| CommandError::NotRepresentable(e.to_string()))?;
        let write = match (reg.table, regs.as_slice()) {
            (Table::Coil, [v]) => WriteRequest::SingleCoil {
                address: reg.address,
                on: *v == 1,
            },
            (Table::HoldingRegister, [v]) => WriteRequest::SingleRegister {
                address: reg.address,
                value: *v,
            },
            (Table::HoldingRegister, _) => WriteRequest::MultipleRegisters {
                address: reg.address,
                values: regs.clone(),
            },
            _ => return Err(CommandError::NotWritable(reg.id.clone())),
        };
        let pdu = write.encode().map_err(|e| CommandError::Encode(e.to_string()))?;
        // Self-check: the bytes decode back to exactly the intended request.
        if WriteRequest::decode(&pdu).as_ref() != Ok(&write) {
            return Err(CommandError::Encode(
                "encoded frame does not decode to the request".into(),
            ));
        }
        let frame_bytes = if target.link.uses_rtu_framing() {
            rtu::encode_adu(unit, &pdu)
        } else {
            tcp::encode_adu(0, unit, &pdu)
        }
        .map_err(|e| CommandError::Encode(e.to_string()))?;

        // Current value (shown as "old"); a failed read is shown as unknown, never as 0.
        let old_req = ReadRequest::for_table(reg.table, reg.address, reg.width())
            .map_err(|e| CommandError::Encode(e.to_string()))?;
        let (old_value, old_raw, old_error) = match link.read(unit, old_req) {
            Ok(data) => {
                let words = data.as_words();
                match codec::decode(dt, reg.word_order, &words)
                    .and_then(|r| codec::to_engineering(&r, &scaling.without_range()))
                {
                    Ok(v) => (v, Some(words), None),
                    Err(e) => (None, Some(words), Some(e.to_string())),
                }
            }
            Err(e) => (None, None, Some(read_error_text(&e))),
        };

        let rb_id = command
            .as_ref()
            .map(|c| c.readback_register.clone())
            .unwrap_or_else(|| reg.id.clone());
        let rb_reg = profile
            .register(&rb_id)
            .ok_or_else(|| CommandError::UnknownRegister(rb_id.clone()))?
            .clone();
        let rb_type = rb_reg.data_type().ok_or(CommandError::UnsupportedType("unknown"))?;
        let rb_req = ReadRequest::for_table(rb_reg.table, rb_reg.address, rb_reg.width())
            .map_err(|e| CommandError::Encode(e.to_string()))?;
        let compare = if matches!(request, CommandRequest::CommConfig { .. }) {
            Compare::Deferred
        } else if rb_reg.id == reg.id {
            Compare::SameRaw(regs.clone())
        } else if let Some(expected) = command.as_ref().and_then(|c| c.expected) {
            Compare::Expected(expected, (rb_reg.scale.unwrap_or(1.0).abs() / 2.0).max(1e-9))
        } else {
            Compare::Report
        };

        let id = self.next_preview;
        self.next_preview += 1;
        let preview = Preview {
            id,
            device_id: target.device.id.clone(),
            device_name: target.device.name.clone(),
            simulated: profile.simulated_only,
            unit_id: unit,
            register_id: reg.id.clone(),
            register_name: reg.name.clone(),
            table: reg.table,
            doc_address: reg.doc_label(),
            address: reg.address,
            function: write.function_code(),
            function_name: write.name().into(),
            command_id: command.as_ref().map(|c| c.id.clone()),
            command_kind: command.as_ref().map(|c| c.kind),
            command_label: command.as_ref().map(|c| c.label.clone()),
            old_value,
            old_raw,
            old_error,
            new_value: value,
            unit: reg.unit.clone(),
            decimals: reg.decimals,
            raw_registers: regs,
            pdu_hex: hex_spaced(&pdu),
            frame_hex: hex_spaced(&frame_bytes),
            readback_register: rb_reg.id.clone(),
            risk: format!("{:?}", wspec.risk).to_lowercase(),
            comm_change: match request {
                CommandRequest::CommConfig { setting, value } => Some(CommChangePreview {
                    setting: *setting,
                    value: *value,
                    apply: profile
                        .comm_config
                        .as_ref()
                        .and_then(|c| setting.of(c))
                        .map_or(Apply::Unknown, |s| s.apply),
                }),
                _ => None,
            },
        };
        Ok(Plan {
            preview,
            frame: PreparedFrame {
                preview_id: id,
                unit,
                request: write,
                pdu,
            },
            readback_request: rb_req,
            readback_type: rb_type,
            readback_register: rb_reg,
            compare,
        })
    }

    /// The operator confirmed exactly this preview.
    pub fn confirm(&mut self, preview_id: u64) -> Result<(), CommandError> {
        if !matches!(self.inner, Inner::Previewed(_)) {
            return Err(CommandError::WrongState {
                expected: "previewed",
                actual: self.state(),
            });
        }
        self.ensure_armed()?;
        let Inner::Previewed(plan) = std::mem::replace(&mut self.inner, Inner::Armed) else {
            return Err(CommandError::WrongState {
                expected: "previewed",
                actual: self.state(),
            });
        };
        if plan.preview.id != preview_id {
            self.inner = Inner::Previewed(plan);
            return Err(CommandError::PreviewMismatch);
        }
        let rec = self.plan_record(AuditEvent::Confirm, &plan.preview);
        if let Err(e) = self.audit.append(&rec) {
            // Not confirmed; nothing will be sent. The operator must preview again.
            return Err(e.into());
        }
        self.inner = Inner::Confirmed(plan);
        self.refresh();
        Ok(())
    }

    /// Send the confirmed frame once, verify the echo, read back, audit.
    pub fn execute(&mut self, link: &mut dyn CommandLink) -> Result<CommandOutcome, CommandError> {
        if !matches!(self.inner, Inner::Confirmed(_)) {
            return Err(CommandError::WrongState {
                expected: "confirmed",
                actual: self.state(),
            });
        }
        self.ensure_armed()?;
        let Inner::Confirmed(plan) = std::mem::replace(&mut self.inner, Inner::Sent) else {
            return Err(CommandError::WrongState {
                expected: "confirmed",
                actual: self.state(),
            });
        };
        let Plan {
            preview,
            frame,
            readback_request,
            readback_type,
            readback_register,
            compare,
        } = *plan;
        let mut sent_rec = self.plan_record(AuditEvent::Sent, &preview);
        sent_rec.note = Some("about to send (single attempt)".into());
        // Fail closed: no audit record, no write.
        if let Err(e) = self.audit.append(&sent_rec) {
            let outcome = CommandOutcome {
                preview_id: preview.id,
                ack: Ack::Invalid("not sent: audit log unavailable".into()),
                readback: Readback::Unknown,
                readback_value: None,
                readback_raw: None,
                readback_error: None,
                request_hex: preview.frame_hex.clone(),
                response_hex: String::new(),
                audit_error: Some(e.to_string()),
            };
            self.inner = Inner::Failed(Box::new(outcome));
            return Err(e.into());
        }

        let request = frame.request.clone();
        let unit = frame.unit;
        let record = link.send(frame);
        if !record.request.is_empty() {
            self.sends += 1;
        }
        let ack = match &record.outcome {
            Ok(reply) => match request.verify_echo(reply) {
                Ok(()) => Ack::Acknowledged,
                Err(_) => Ack::BadEcho,
            },
            Err(ExchangeError::Exception(c)) => Ack::Exception(c.0),
            Err(ExchangeError::Timeout) => Ack::Timeout,
            Err(ExchangeError::Crc(_)) => Ack::CrcError,
            Err(ExchangeError::Link(_)) if record.request.is_empty() => Ack::NotSent,
            Err(ExchangeError::Link(_)) => Ack::LinkLost,
            Err(e @ (ExchangeError::Decode(_) | ExchangeError::Encode(_))) => Ack::Invalid(e.to_string()),
        };

        self.inner = Inner::ReadBack;
        let (readback, readback_value, readback_raw, readback_error) = if matches!(ack, Ack::LinkLost | Ack::NotSent) {
            (Readback::Unknown, None, None, Some("link lost".to_owned()))
        } else if compare == Compare::Deferred {
            // The device now answers at its new settings; the wizard verifies after reconnecting.
            (
                Readback::Unknown,
                None,
                None,
                Some("verified after reconnect at the new settings".to_owned()),
            )
        } else {
            match link.read(unit, readback_request) {
                Err(e) => (Readback::Unknown, None, None, Some(e.to_string())),
                Ok(data) => {
                    let words = data.as_words();
                    let decoded = codec::decode(readback_type, readback_register.word_order, &words)
                        .and_then(|r| codec::to_engineering(&r, &readback_register.scaling().without_range()));
                    let value = decoded.as_ref().ok().copied().flatten();
                    let verdict = match &compare {
                        Compare::SameRaw(expected) => {
                            if *expected == words {
                                Readback::Verified
                            } else {
                                Readback::Mismatch
                            }
                        }
                        Compare::Expected(e, tol) => match value {
                            Some(v) if (v - e).abs() <= *tol => Readback::Verified,
                            Some(_) => Readback::Mismatch,
                            None => Readback::Unknown,
                        },
                        Compare::Report => Readback::Unknown,
                        Compare::Deferred => Readback::Unknown,
                    };
                    (verdict, value, Some(words), decoded.err().map(|e| e.to_string()))
                }
            }
        };

        let mut outcome = CommandOutcome {
            preview_id: preview.id,
            ack: ack.clone(),
            readback,
            readback_value,
            readback_raw,
            readback_error,
            request_hex: hex_spaced(&record.request),
            response_hex: hex_spaced(&record.response),
            audit_error: None,
        };
        let mut rec = sent_rec;
        rec.event = Some(AuditEvent::Result);
        rec.ts_ms = self.clock.utc_ms();
        rec.request_hex = Some(outcome.request_hex.clone());
        rec.response_hex = Some(outcome.response_hex.clone());
        rec.exception = match ack {
            Ack::Exception(c) => Some(c),
            _ => None,
        };
        rec.ack = Some(ack.as_str());
        rec.readback = Some(readback.as_str().into());
        rec.readback_value = readback_value;
        rec.note = outcome.readback_error.clone();
        if let Err(e) = self.audit.append(&rec) {
            outcome.audit_error = Some(e.to_string());
        }
        let link_lost = matches!(ack, Ack::LinkLost | Ack::NotSent);
        self.inner = if ack == Ack::Acknowledged {
            Inner::Done(Box::new(outcome.clone()))
        } else {
            Inner::Failed(Box::new(outcome.clone()))
        };
        if link_lost {
            // Arming is dropped on link loss; the outcome stays visible.
            let mut r = self.base_record(AuditEvent::Disarm);
            r.note = Some(DisarmReason::LinkLost.as_str().into());
            let _ = self.audit.append(&r);
            self.armed = None;
        } else {
            self.refresh();
        }
        Ok(outcome)
    }
}
