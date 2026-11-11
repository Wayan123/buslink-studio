//! Typed profile model (schema v1). Deserialised only after `validate` passed.

use crate::codec::{DataType, Scaling, TypeName, WordOrder};
use crate::protocol::pdu::Table;
use crate::protocol::rtu::{Parity, SerialSettings};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Plain text or `{ "en", "id" }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Bilingual { en: String, id: String },
}

impl Text {
    /// Text in the requested language (`"id"` or anything else for English).
    pub fn get(&self, lang: &str) -> &str {
        match self {
            Text::Plain(s) => s,
            Text::Bilingual { en, id } => {
                if lang == "id" {
                    id
                } else {
                    en
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Notes {
    One(Text),
    Many(Vec<Text>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Generic,
    Documented,
    Simulated,
    HardwareVerified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLevel {
    Generic,
    Documented,
    Simulated,
    HardwareVerified,
    Unsupported,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<CapabilityLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<CapabilityLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comm_config: Option<CapabilityLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<CapabilityLevel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub document: Text,
    pub revision: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Text>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestResult {
    Pass,
    Fail,
    Partial,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareRecord {
    pub date: String,
    pub adapter: Text,
    pub platform: Text,
    pub record: Text,
    pub result: TestResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Text>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    #[serde(default)]
    pub hardware: Vec<HardwareRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RtuDefaults {
    pub unit_id: u8,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inter_frame_ms: Option<u32>,
}

impl RtuDefaults {
    pub fn serial(&self) -> SerialSettings {
        SerialSettings::new(self.baud, self.data_bits, self.parity, self.stop_bits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TcpDefaults {
    pub unit_id: u8,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_timeout_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulatorRef {
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtu: Option<RtuDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp: Option<TcpDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator: Option<SimulatorRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DocAddress {
    Number(i64),
    Label(String),
}

impl DocAddress {
    pub fn display(&self) -> String {
        match self {
            DocAddress::Number(n) => n.to_string(),
            DocAddress::Label(s) => s.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    #[default]
    Read,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollClass {
    Fast,
    Normal,
    Slow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRole {
    Operator,
    Commissioner,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<f64>>,
    pub risk: Risk,
    pub role: WriteRole,
    /// Always true (the validator rejects false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<bool>,
    /// Always true (the validator rejects false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readback: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Register {
    pub id: String,
    pub name: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    pub table: Table,
    /// Zero-based PDU offset. Never a 4xxxx label.
    pub address: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_address: Option<DocAddress>,
    #[serde(rename = "type")]
    pub type_name: TypeName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_order: Option<WordOrder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_range: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_sentinels: Option<Vec<i128>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bits: Option<BTreeMap<String, Text>>,
    #[serde(default)]
    pub access: Access,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<WriteSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<PollClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_page: Option<Text>,
}

impl Register {
    pub fn data_type(&self) -> Option<DataType> {
        DataType::from_name(self.type_name, self.length)
    }

    /// Registers (or bits) covered.
    pub fn width(&self) -> u16 {
        self.data_type().map(DataType::width).unwrap_or(1)
    }

    pub fn end_address(&self) -> u32 {
        u32::from(self.address) + u32::from(self.width()) - 1
    }

    pub fn is_writable(&self) -> bool {
        self.access == Access::ReadWrite && self.table.is_writable()
    }

    pub fn scaling(&self) -> Scaling {
        Scaling {
            scale: self.scale.unwrap_or(1.0),
            offset: self.offset.unwrap_or(0.0),
            valid_range: self.valid_range,
            sentinels: self.invalid_sentinels.clone().unwrap_or_default(),
        }
    }

    /// Manual label if the profile has one, else the conventional label marked as such.
    pub fn doc_label(&self) -> String {
        match &self.doc_address {
            Some(d) => d.display(),
            None => format!("({})", self.table.conventional_label(self.address)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Run,
    Stop,
    Setpoint,
    ResetFault,
    Mode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub kind: CommandKind,
    pub label: Text,
    pub register: String,
    pub readback_register: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Apply {
    Immediate,
    AfterReboot,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommSetting {
    pub register: String,
    pub apply: Apply,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codes: Option<BTreeMap<String, u32>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<CommSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<CommSetting>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(rename = "$comment", default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub schema_version: u32,
    pub profile_id: String,
    pub profile_version: String,
    pub vendor: Text,
    pub model: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Notes>,
    pub support_level: SupportLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
    #[serde(default)]
    pub simulated_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    pub transport: TransportDefaults,
    pub registers: Vec<Register>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<Command>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comm_config: Option<CommConfig>,
}

impl Profile {
    pub fn register(&self, id: &str) -> Option<&Register> {
        self.registers.iter().find(|r| r.id == id)
    }

    pub fn register_index(&self, id: &str) -> Option<usize> {
        self.registers.iter().position(|r| r.id == id)
    }

    pub fn command(&self, id: &str) -> Option<&Command> {
        self.commands.as_ref()?.iter().find(|c| c.id == id)
    }

    pub fn capability(&self, f: impl Fn(&Capabilities) -> Option<CapabilityLevel>) -> CapabilityLevel {
        self.capabilities
            .as_ref()
            .and_then(f)
            .unwrap_or(CapabilityLevel::Unsupported)
    }

    /// `profile_id@profile_version`, stored with every sample and export.
    pub fn key(&self) -> String {
        format!("{}@{}", self.profile_id, self.profile_version)
    }
}
