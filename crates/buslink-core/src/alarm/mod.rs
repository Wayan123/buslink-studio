//! ISA-18.2-style alarm engine (PRD R16, research/02 §2).
//!
//! Two classes: **process** alarms (HI/HIHI/LO/LOLO on a tag's GOOD value,
//! with deadband and on/off delays) and **communication** alarms (one per
//! device, active while any of its tags has a bad quality). While a tag's
//! quality is not GOOD its process alarms hold their state, so a dead link
//! raises one communication alarm instead of a flood of process alarms.
//!
//! The engine is pure and clock-free: every call gets `now_ms` (UTC).

use crate::quality::{Quality, Sample};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use thiserror::Error;

pub const FLOOD_WINDOW_MS: i64 = 10 * 60_000;
pub const FLOOD_THRESHOLD: usize = 10;
pub const CHATTER_WINDOW_MS: i64 = 60_000;
pub const CHATTER_THRESHOLD: usize = 3;
pub const MAX_SHELVE_MS: i64 = 8 * 3_600_000;
pub const COMM_PRIORITY: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlarmKind {
    Hi,
    HiHi,
    Lo,
    LoLo,
}

impl AlarmKind {
    pub fn is_high(self) -> bool {
        matches!(self, AlarmKind::Hi | AlarmKind::HiHi)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmKind::Hi => "HI",
            AlarmKind::HiHi => "HIHI",
            AlarmKind::Lo => "LO",
            AlarmKind::LoLo => "LOLO",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlarmDef {
    pub id: String,
    pub device_id: String,
    pub tag_id: String,
    pub kind: AlarmKind,
    pub limit: f64,
    pub deadband: f64,
    pub on_delay_ms: u64,
    pub off_delay_ms: u64,
    pub priority: u8,
    #[serde(default)]
    pub latch: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmClass {
    Process,
    Comm,
}

/// The seven ISA-18.2 states (as displayed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlarmState {
    Normal,
    Unacked,
    Acked,
    RtnUnacked,
    Shelved,
    Suppressed,
    OutOfService,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AlarmError {
    #[error("unknown alarm '{0}'")]
    Unknown(String),
    #[error("alarm id '{0}' already exists")]
    Duplicate(String),
    #[error("invalid alarm definition: {0}")]
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AlarmEvent {
    pub ts_ms: i64,
    pub alarm_id: String,
    pub class: AlarmClass,
    pub device_id: String,
    pub tag_id: Option<String>,
    pub priority: u8,
    /// activate, clear, ack, shelve, unshelve, shelve_expired, out_of_service, in_service
    pub event: String,
    pub text: String,
}

#[derive(Clone, Debug)]
struct Alarm {
    id: String,
    class: AlarmClass,
    device_id: String,
    def: Option<AlarmDef>,
    active: bool,
    acked: bool,
    pending_on_since: Option<i64>,
    pending_off_since: Option<i64>,
    /// Latest raw condition from a GOOD value (`None` = not known yet).
    condition: Option<bool>,
    held: bool,
    shelved_until: Option<i64>,
    out_of_service: bool,
    activated_ms: Option<i64>,
    last_change_ms: i64,
    activations: VecDeque<i64>,
    value: Option<f64>,
    comm_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AlarmView {
    pub id: String,
    pub class: AlarmClass,
    pub device_id: String,
    pub tag_id: Option<String>,
    pub kind: Option<AlarmKind>,
    pub limit: Option<f64>,
    pub deadband: Option<f64>,
    pub on_delay_ms: u64,
    pub off_delay_ms: u64,
    pub priority: u8,
    pub latch: bool,
    pub message: Option<String>,
    pub state: AlarmState,
    pub active: bool,
    pub acked: bool,
    /// The tag's quality is not GOOD: the process state is held, not evaluated.
    pub held: bool,
    pub shelved_until_ms: Option<i64>,
    pub out_of_service: bool,
    pub chattering: bool,
    pub activated_ms: Option<i64>,
    pub last_change_ms: i64,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AlarmSummary {
    /// Annunciated alarms needing attention (active or unacknowledged), per priority 1..=3.
    pub counts: [usize; 3],
    pub unacked: usize,
    pub oldest_unacked: Option<String>,
    pub oldest_unacked_ms: Option<i64>,
    pub activations_10min: usize,
    pub flood: bool,
    pub comm_active: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AlarmSnapshot {
    pub alarms: Vec<AlarmView>,
    pub summary: AlarmSummary,
}

#[derive(Default)]
pub struct AlarmEngine {
    alarms: BTreeMap<String, Alarm>,
    /// (device, tag) → bad quality for the comm alarm of the device.
    tag_quality: HashMap<(String, String), Quality>,
    activations: VecDeque<i64>,
    events: Vec<AlarmEvent>,
    revision: u64,
}

/// Deadband from research/02: temperature 1 %, pressure 2 %, flow/level and others 5 % of `span`.
pub fn default_deadband(unit: Option<&str>, span: f64) -> f64 {
    let pct = match unit.map(str::trim) {
        Some("°C" | "°F" | "K" | "degC") => 0.01,
        Some("bar" | "mbar" | "Pa" | "kPa" | "MPa" | "psi") => 0.02,
        _ => 0.05,
    };
    (span.abs() * pct).max(0.0)
}

/// On/off delay from research/02: 60 s for temperature, 15 s otherwise.
pub fn default_delay_ms(unit: Option<&str>) -> u64 {
    match unit.map(str::trim) {
        Some("°C" | "°F" | "K" | "degC") => 60_000,
        _ => 15_000,
    }
}

fn comm_id(device: &str) -> String {
    format!("comm:{device}")
}

fn is_comm_bad(q: Quality) -> bool {
    !q.is_good()
}

impl Alarm {
    fn state(&self, now_ms: i64) -> AlarmState {
        if self.out_of_service {
            AlarmState::OutOfService
        } else if self.shelved_until.is_some_and(|u| u > now_ms) {
            AlarmState::Shelved
        } else if !self.acked && (self.active || self.latched()) {
            AlarmState::Unacked
        } else if self.active {
            AlarmState::Acked
        } else if !self.acked {
            AlarmState::RtnUnacked
        } else if self.held {
            AlarmState::Suppressed
        } else {
            AlarmState::Normal
        }
    }

    fn latched(&self) -> bool {
        self.def.as_ref().is_some_and(|d| d.latch)
    }

    fn priority(&self) -> u8 {
        self.def.as_ref().map(|d| d.priority).unwrap_or(COMM_PRIORITY)
    }

    fn tag_id(&self) -> Option<String> {
        self.def.as_ref().map(|d| d.tag_id.clone())
    }

    fn chattering(&self, now_ms: i64) -> bool {
        self.activations
            .iter()
            .filter(|t| now_ms - **t <= CHATTER_WINDOW_MS)
            .count()
            >= CHATTER_THRESHOLD
    }

    fn view(&self, now_ms: i64) -> AlarmView {
        let d = self.def.as_ref();
        AlarmView {
            id: self.id.clone(),
            class: self.class,
            device_id: self.device_id.clone(),
            tag_id: self.tag_id(),
            kind: d.map(|d| d.kind),
            limit: d.map(|d| d.limit),
            deadband: d.map(|d| d.deadband),
            on_delay_ms: d.map(|d| d.on_delay_ms).unwrap_or(0),
            off_delay_ms: d.map(|d| d.off_delay_ms).unwrap_or(0),
            priority: self.priority(),
            latch: self.latched(),
            message: d.and_then(|d| d.message.clone()).or_else(|| self.comm_text.clone()),
            state: self.state(now_ms),
            active: self.active,
            acked: self.acked,
            held: self.held,
            shelved_until_ms: self.shelved_until.filter(|u| *u > now_ms),
            out_of_service: self.out_of_service,
            chattering: self.chattering(now_ms),
            activated_ms: self.activated_ms,
            last_change_ms: self.last_change_ms,
            value: self.value,
        }
    }
}

impl AlarmEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increases on every change; lets the session send snapshots only when something changed.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn add(&mut self, def: AlarmDef, now_ms: i64) -> Result<(), AlarmError> {
        if def.id.is_empty() || def.id.len() > 96 || def.id.starts_with("comm:") {
            return Err(AlarmError::Invalid(
                "id must be 1-96 characters and not start with 'comm:'".into(),
            ));
        }
        if self.alarms.contains_key(&def.id) {
            return Err(AlarmError::Duplicate(def.id));
        }
        if !(1..=3).contains(&def.priority) {
            return Err(AlarmError::Invalid("priority must be 1, 2 or 3".into()));
        }
        if !def.limit.is_finite() || !def.deadband.is_finite() || def.deadband < 0.0 {
            return Err(AlarmError::Invalid("limit must be a number and deadband ≥ 0".into()));
        }
        if def.on_delay_ms > 3_600_000 || def.off_delay_ms > 3_600_000 {
            return Err(AlarmError::Invalid("delays must be ≤ 1 h".into()));
        }
        let id = def.id.clone();
        self.alarms.insert(
            id.clone(),
            Alarm {
                id,
                class: AlarmClass::Process,
                device_id: def.device_id.clone(),
                def: Some(def),
                active: false,
                acked: true,
                pending_on_since: None,
                pending_off_since: None,
                condition: None,
                held: false,
                shelved_until: None,
                out_of_service: false,
                activated_ms: None,
                last_change_ms: now_ms,
                activations: VecDeque::new(),
                value: None,
                comm_text: None,
            },
        );
        self.revision += 1;
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<(), AlarmError> {
        match self.alarms.get(id) {
            Some(a) if a.class == AlarmClass::Process => {
                self.alarms.remove(id);
                self.revision += 1;
                Ok(())
            }
            _ => Err(AlarmError::Unknown(id.to_owned())),
        }
    }

    /// Forget a device (removed from the session): its process and comm alarms go.
    pub fn remove_device(&mut self, device_id: &str) {
        self.alarms.retain(|_, a| a.device_id != device_id);
        self.tag_quality.retain(|(d, _), _| d != device_id);
        self.revision += 1;
    }

    pub fn defs(&self) -> Vec<AlarmDef> {
        self.alarms.values().filter_map(|a| a.def.clone()).collect()
    }

    fn event(&mut self, a_id: &str, now_ms: i64, event: &str, text: String) {
        let Some(a) = self.alarms.get(a_id) else { return };
        self.events.push(AlarmEvent {
            ts_ms: now_ms,
            alarm_id: a.id.clone(),
            class: a.class,
            device_id: a.device_id.clone(),
            tag_id: a.tag_id(),
            priority: a.priority(),
            event: event.to_owned(),
            text,
        });
        self.revision += 1;
    }

    fn set_active(&mut self, id: &str, active: bool, now_ms: i64) {
        let Some(a) = self.alarms.get_mut(id) else { return };
        if a.active == active {
            return;
        }
        a.active = active;
        a.last_change_ms = now_ms;
        a.pending_on_since = None;
        a.pending_off_since = None;
        let text;
        if active {
            a.acked = false;
            a.activated_ms = Some(now_ms);
            a.activations.push_back(now_ms);
            while a.activations.len() > 16 {
                a.activations.pop_front();
            }
            self.activations.push_back(now_ms);
            text = describe(a, "active");
        } else {
            text = describe(a, "cleared");
        }
        let ev = if active { "activate" } else { "clear" };
        self.event(id, now_ms, ev, text);
    }

    /// Feed one sample. Process alarms of its tag are evaluated (GOOD) or held (not GOOD);
    /// the device's communication alarm follows the worst tag quality.
    pub fn observe(&mut self, s: &Sample, now_ms: i64) {
        let key = (s.device_id.clone(), s.tag_id.clone());
        if is_comm_bad(s.quality) {
            self.tag_quality.insert(key, s.quality);
        } else {
            self.tag_quality.remove(&key);
        }
        self.update_comm(&s.device_id, now_ms);

        let ids: Vec<String> = self
            .alarms
            .values()
            .filter(|a| {
                a.class == AlarmClass::Process
                    && a.def
                        .as_ref()
                        .is_some_and(|d| d.device_id == s.device_id && d.tag_id == s.tag_id)
            })
            .map(|a| a.id.clone())
            .collect();
        let value = s.fresh_value();
        for id in ids {
            let Some(a) = self.alarms.get_mut(&id) else { continue };
            let Some(def) = a.def.clone() else { continue };
            match value {
                None => {
                    if !a.held {
                        a.held = true;
                        self.revision += 1;
                    }
                    // Hold: timers stop, the state stays as it is.
                    a.pending_on_since = None;
                    a.pending_off_since = None;
                    a.condition = None;
                }
                Some(v) => {
                    if a.held {
                        a.held = false;
                        self.revision += 1;
                    }
                    a.value = Some(v);
                    let on = if def.kind.is_high() {
                        v >= def.limit
                    } else {
                        v <= def.limit
                    };
                    let off = if def.kind.is_high() {
                        v < def.limit - def.deadband
                    } else {
                        v > def.limit + def.deadband
                    };
                    if !a.active {
                        a.condition = Some(on);
                        if on {
                            a.pending_on_since.get_or_insert(now_ms);
                        } else {
                            a.pending_on_since = None;
                        }
                    } else {
                        a.condition = Some(!off);
                        if off {
                            a.pending_off_since.get_or_insert(now_ms);
                        } else {
                            a.pending_off_since = None;
                        }
                    }
                }
            }
        }
        self.tick(now_ms);
    }

    fn update_comm(&mut self, device: &str, now_ms: i64) {
        let worst = self
            .tag_quality
            .iter()
            .filter(|((d, _), _)| d == device)
            .map(|(_, q)| *q)
            .max_by_key(|q| comm_rank(*q));
        let id = comm_id(device);
        if !self.alarms.contains_key(&id) {
            if worst.is_none() {
                return;
            }
            self.alarms.insert(
                id.clone(),
                Alarm {
                    id: id.clone(),
                    class: AlarmClass::Comm,
                    device_id: device.to_owned(),
                    def: None,
                    active: false,
                    acked: true,
                    pending_on_since: None,
                    pending_off_since: None,
                    condition: None,
                    held: false,
                    shelved_until: None,
                    out_of_service: false,
                    activated_ms: None,
                    last_change_ms: now_ms,
                    activations: VecDeque::new(),
                    value: None,
                    comm_text: None,
                },
            );
        }
        if let (Some(a), Some(q)) = (self.alarms.get_mut(&id), worst) {
            let text = format!("Communication: {q}");
            if a.comm_text.as_deref() != Some(text.as_str()) {
                a.comm_text = Some(text);
                self.revision += 1;
            }
        }
        self.set_active(&id, worst.is_some(), now_ms);
    }

    /// Complete on/off delays and expire shelves.
    pub fn tick(&mut self, now_ms: i64) {
        let ids: Vec<String> = self.alarms.keys().cloned().collect();
        for id in ids {
            let Some(a) = self.alarms.get_mut(&id) else { continue };
            if let Some(until) = a.shelved_until
                && until <= now_ms
            {
                a.shelved_until = None;
                let text = describe(a, "shelve expired");
                self.event(&id, now_ms, "shelve_expired", text);
            }
            let Some(a) = self.alarms.get(&id) else { continue };
            let Some(def) = a.def.as_ref() else { continue };
            let on_due = a
                .pending_on_since
                .is_some_and(|t| now_ms.saturating_sub(t) >= i64::try_from(def.on_delay_ms).unwrap_or(i64::MAX));
            let off_due = a
                .pending_off_since
                .is_some_and(|t| now_ms.saturating_sub(t) >= i64::try_from(def.off_delay_ms).unwrap_or(i64::MAX));
            if !a.active && on_due {
                self.set_active(&id, true, now_ms);
            } else if a.active && off_due {
                self.set_active(&id, false, now_ms);
            }
        }
        while self.activations.front().is_some_and(|t| now_ms - *t > FLOOD_WINDOW_MS) {
            self.activations.pop_front();
        }
    }

    pub fn ack(&mut self, id: &str, now_ms: i64) -> Result<(), AlarmError> {
        let a = self
            .alarms
            .get_mut(id)
            .ok_or_else(|| AlarmError::Unknown(id.to_owned()))?;
        if a.acked {
            return Ok(());
        }
        a.acked = true;
        a.last_change_ms = now_ms;
        let text = describe(a, "acknowledged");
        self.event(id, now_ms, "ack", text);
        Ok(())
    }

    /// Shelve until `until_ms` (at most 8 h ahead).
    pub fn shelve(&mut self, id: &str, until_ms: i64, now_ms: i64) -> Result<(), AlarmError> {
        if until_ms <= now_ms || until_ms - now_ms > MAX_SHELVE_MS {
            return Err(AlarmError::Invalid(
                "shelve duration must be between 1 s and 8 h".into(),
            ));
        }
        let a = self
            .alarms
            .get_mut(id)
            .ok_or_else(|| AlarmError::Unknown(id.to_owned()))?;
        a.shelved_until = Some(until_ms);
        let text = format!("{} until {until_ms}", describe(a, "shelved"));
        self.event(id, now_ms, "shelve", text);
        Ok(())
    }

    pub fn unshelve(&mut self, id: &str, now_ms: i64) -> Result<(), AlarmError> {
        let a = self
            .alarms
            .get_mut(id)
            .ok_or_else(|| AlarmError::Unknown(id.to_owned()))?;
        if a.shelved_until.take().is_some() {
            let text = describe(a, "unshelved");
            self.event(id, now_ms, "unshelve", text);
        }
        Ok(())
    }

    pub fn set_out_of_service(&mut self, id: &str, oos: bool, now_ms: i64) -> Result<(), AlarmError> {
        let a = self
            .alarms
            .get_mut(id)
            .ok_or_else(|| AlarmError::Unknown(id.to_owned()))?;
        if a.out_of_service == oos {
            return Ok(());
        }
        a.out_of_service = oos;
        let (ev, word) = if oos {
            ("out_of_service", "out of service")
        } else {
            ("in_service", "back in service")
        };
        let text = describe(a, word);
        self.event(id, now_ms, ev, text);
        Ok(())
    }

    pub fn state(&self, id: &str, now_ms: i64) -> Option<AlarmState> {
        self.alarms.get(id).map(|a| a.state(now_ms))
    }

    pub fn view(&self, id: &str, now_ms: i64) -> Option<AlarmView> {
        self.alarms.get(id).map(|a| a.view(now_ms))
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<AlarmEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn snapshot(&self, now_ms: i64) -> AlarmSnapshot {
        let alarms: Vec<AlarmView> = self.alarms.values().map(|a| a.view(now_ms)).collect();
        let mut summary = AlarmSummary {
            activations_10min: self
                .activations
                .iter()
                .filter(|t| now_ms - **t <= FLOOD_WINDOW_MS)
                .count(),
            ..AlarmSummary::default()
        };
        summary.flood = summary.activations_10min > FLOOD_THRESHOLD;
        for a in &alarms {
            if a.class == AlarmClass::Comm && a.active {
                summary.comm_active += 1;
            }
            let annunciated = matches!(
                a.state,
                AlarmState::Unacked | AlarmState::Acked | AlarmState::RtnUnacked
            );
            if annunciated {
                let p = usize::from(a.priority.clamp(1, 3)) - 1;
                summary.counts[p] += 1;
            }
            if matches!(a.state, AlarmState::Unacked | AlarmState::RtnUnacked) {
                summary.unacked += 1;
                let t = a.activated_ms.unwrap_or(a.last_change_ms);
                if summary.oldest_unacked_ms.is_none_or(|o| t < o) {
                    summary.oldest_unacked_ms = Some(t);
                    summary.oldest_unacked = Some(a.id.clone());
                }
            }
        }
        AlarmSnapshot { alarms, summary }
    }
}

fn comm_rank(q: Quality) -> u8 {
    match q {
        Quality::Disconnected => 6,
        Quality::Timeout => 5,
        Quality::CrcError => 4,
        Quality::Exception(_) => 3,
        Quality::DecodeError => 2,
        Quality::Stale => 1,
        Quality::Good => 0,
    }
}

fn describe(a: &Alarm, what: &str) -> String {
    match &a.def {
        Some(d) => format!(
            "P{} {} {}/{} limit {} {what}",
            d.priority,
            d.kind.as_str(),
            d.device_id,
            d.tag_id,
            d.limit
        ),
        None => format!(
            "P{COMM_PRIORITY} {} {what}",
            a.comm_text
                .clone()
                .unwrap_or_else(|| format!("Communication {}", a.device_id))
        ),
    }
}
