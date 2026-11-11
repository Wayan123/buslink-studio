//! Links (one bus or connection each) and device instances. A device
//! instance holds site settings and points at a profile by ID and version;
//! one profile serves many instances.

use crate::profile::Profile;
use crate::protocol::pdu::{MAX_READ_BITS, MAX_READ_REGISTERS};
use crate::protocol::rtu::{RtuTiming, SerialSettings};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkKind {
    /// Serial RTU. `port` is the OS port (COM3, /dev/ttyUSB0) or the Android `<deviceName>#<index>`.
    Rtu { port: String, serial: SerialSettings },
    /// Modbus TCP (MBAP).
    Tcp { host: String, port: u16 },
    /// Raw RTU frames (with CRC) through a transparent serial server. Not MBAP.
    RtuOverTcp { host: String, port: u16, serial: SerialSettings },
    /// In-process simulator bus.
    Simulator { name: String, serial: SerialSettings },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkConfig {
    pub id: String,
    pub kind: LinkKind,
    /// Floor for the inter-frame gap (Android: 50 ms).
    #[serde(default)]
    pub min_inter_frame_ms: u64,
}

impl LinkConfig {
    /// Key that identifies the physical resource: two links with the same key
    /// would be two masters on one bus.
    pub fn owner_key(&self) -> String {
        match &self.kind {
            LinkKind::Rtu { port, .. } => format!("serial:{}", port.to_ascii_lowercase()),
            LinkKind::Tcp { host, port } => format!("tcp:{}:{port}", host.to_ascii_lowercase()),
            LinkKind::RtuOverTcp { host, port, .. } => format!("rtu-tcp:{}:{port}", host.to_ascii_lowercase()),
            LinkKind::Simulator { name, .. } => format!("sim:{name}"),
        }
    }

    /// True when responses are RTU frames (serial, RTU-over-TCP, simulator bus).
    pub fn uses_rtu_framing(&self) -> bool {
        !matches!(self.kind, LinkKind::Tcp { .. })
    }

    pub fn serial(&self) -> Option<SerialSettings> {
        match &self.kind {
            LinkKind::Rtu { serial, .. } | LinkKind::RtuOverTcp { serial, .. } | LinkKind::Simulator { serial, .. } => {
                Some(*serial)
            }
            LinkKind::Tcp { .. } => None,
        }
    }

    pub fn is_simulator(&self) -> bool {
        matches!(self.kind, LinkKind::Simulator { .. })
    }

    /// A TCP link whose host is a loopback IP literal (the training TCP server).
    pub fn is_loopback_tcp(&self) -> bool {
        match &self.kind {
            LinkKind::Tcp { host, .. } => host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
            _ => false,
        }
    }

    /// Maximum unit ID allowed for this link (247 on serial lines, 255 on TCP).
    pub fn max_unit_id(&self) -> u8 {
        if self.uses_rtu_framing() { 247 } else { 255 }
    }

    pub fn inter_frame_ms(&self) -> u64 {
        match self.serial() {
            Some(s) => RtuTiming::new(&s).inter_frame_ms(self.min_inter_frame_ms),
            None => self.min_inter_frame_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInstance {
    pub id: String,
    pub name: String,
    pub link_id: String,
    pub unit_id: u8,
    pub profile_id: String,
    pub profile_version: String,
    pub poll_interval_ms: u64,
    /// Default `max(3 × poll interval, 3 s)`.
    #[serde(default)]
    pub stale_after_ms: Option<u64>,
    /// Default: profile, else derived from the serial settings.
    #[serde(default)]
    pub response_timeout_ms: Option<u64>,
    /// Read retries on timeout/CRC error (writes are never retried).
    #[serde(default = "default_retries")]
    pub read_retries: u8,
    /// Unused addresses a merged read may span. 0 = contiguous only.
    #[serde(default)]
    pub max_gap: u16,
    #[serde(default)]
    pub max_registers_per_read: Option<u16>,
    #[serde(default)]
    pub max_bits_per_read: Option<u16>,
}

fn default_retries() -> u8 {
    1
}

pub const MIN_POLL_INTERVAL_MS: u64 = 100;
pub const MAX_POLL_INTERVAL_MS: u64 = 86_400_000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DeviceError {
    #[error("unit ID 0 is broadcast and cannot be a device")]
    UnitZero,
    #[error("unit ID {unit} is above {max} for this link")]
    UnitRange { unit: u8, max: u8 },
    #[error("device uses profile {expected}, got {got}")]
    ProfileMismatch { expected: String, got: String },
    #[error("device is on link '{expected}', got '{got}'")]
    LinkMismatch { expected: String, got: String },
    #[error("poll interval {0} ms outside {MIN_POLL_INTERVAL_MS}..={MAX_POLL_INTERVAL_MS}")]
    PollInterval(u64),
    #[error("read limit {0} outside the spec range")]
    ReadLimit(u16),
    #[error("read retries {0} above 3")]
    Retries(u8),
    #[error("device id must be 1-64 characters of a-z, 0-9, '-', '_' or '.'")]
    Id,
}

impl DeviceInstance {
    /// A device using the profile's defaults for the given link kind.
    pub fn from_profile(id: &str, name: &str, link: &LinkConfig, profile: &Profile) -> DeviceInstance {
        let unit_id = if link.uses_rtu_framing() {
            profile.transport.rtu.map(|r| r.unit_id).or(profile.transport.tcp.map(|t| t.unit_id))
        } else {
            profile.transport.tcp.map(|t| t.unit_id).or(profile.transport.rtu.map(|r| r.unit_id))
        };
        DeviceInstance {
            id: id.to_owned(),
            name: name.to_owned(),
            link_id: link.id.clone(),
            unit_id: unit_id.unwrap_or(1),
            profile_id: profile.profile_id.clone(),
            profile_version: profile.profile_version.clone(),
            poll_interval_ms: 1000,
            stale_after_ms: None,
            response_timeout_ms: None,
            read_retries: default_retries(),
            max_gap: 0,
            max_registers_per_read: None,
            max_bits_per_read: None,
        }
    }

    pub fn check(&self, profile: &Profile, link: &LinkConfig) -> Result<(), DeviceError> {
        let id_ok = (1..=64).contains(&self.id.len())
            && self.id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_' | b'.'));
        if !id_ok {
            return Err(DeviceError::Id);
        }
        if self.unit_id == 0 {
            return Err(DeviceError::UnitZero);
        }
        if self.unit_id > link.max_unit_id() {
            return Err(DeviceError::UnitRange { unit: self.unit_id, max: link.max_unit_id() });
        }
        if self.profile_id != profile.profile_id || self.profile_version != profile.profile_version {
            return Err(DeviceError::ProfileMismatch {
                expected: format!("{}@{}", self.profile_id, self.profile_version),
                got: profile.key(),
            });
        }
        if self.link_id != link.id {
            return Err(DeviceError::LinkMismatch { expected: self.link_id.clone(), got: link.id.clone() });
        }
        if !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&self.poll_interval_ms) {
            return Err(DeviceError::PollInterval(self.poll_interval_ms));
        }
        if let Some(n) = self.max_registers_per_read
            && !(1..=MAX_READ_REGISTERS).contains(&n)
        {
            return Err(DeviceError::ReadLimit(n));
        }
        if let Some(n) = self.max_bits_per_read
            && !(1..=MAX_READ_BITS).contains(&n)
        {
            return Err(DeviceError::ReadLimit(n));
        }
        if self.read_retries > 3 {
            return Err(DeviceError::Retries(self.read_retries));
        }
        Ok(())
    }

    pub fn stale_threshold_ms(&self) -> u64 {
        self.stale_after_ms.unwrap_or_else(|| self.poll_interval_ms.saturating_mul(3).max(3000))
    }

    /// Instance setting, else profile default for the link kind, else derived from the serial settings.
    pub fn timeout_ms(&self, profile: &Profile, link: &LinkConfig) -> u64 {
        if let Some(t) = self.response_timeout_ms {
            return t;
        }
        let from_profile = if link.uses_rtu_framing() {
            profile.transport.rtu.and_then(|r| r.response_timeout_ms)
        } else {
            profile.transport.tcp.and_then(|t| t.response_timeout_ms)
        };
        if let Some(t) = from_profile {
            return u64::from(t);
        }
        match link.serial() {
            Some(s) => RtuTiming::new(&s).default_response_timeout_ms(),
            None => 1000,
        }
    }
}
