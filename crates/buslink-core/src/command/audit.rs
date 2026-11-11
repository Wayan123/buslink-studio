//! Command audit records (safety rule 3). One record per state transition.

use serde::Serialize;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    Arm,
    Disarm,
    ArmExpired,
    Preview,
    Confirm,
    Cancel,
    Sent,
    Result,
    Rejected,
    /// Commissioning journal: old settings, recorded before arming.
    Journal,
    /// Commissioning verification at the new settings.
    Verify,
    /// Process feedback resolved (reached / not reached / unavailable).
    Feedback,
}

impl AuditEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditEvent::Arm => "arm",
            AuditEvent::Disarm => "disarm",
            AuditEvent::ArmExpired => "arm_expired",
            AuditEvent::Preview => "preview",
            AuditEvent::Confirm => "confirm",
            AuditEvent::Cancel => "cancel",
            AuditEvent::Sent => "sent",
            AuditEvent::Result => "result",
            AuditEvent::Rejected => "rejected",
            AuditEvent::Journal => "journal",
            AuditEvent::Verify => "verify",
            AuditEvent::Feedback => "feedback",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AuditRecord {
    pub ts_ms: i64,
    pub session_id: String,
    pub user: String,
    pub role: String,
    pub event: Option<AuditEvent>,
    pub device_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub unit_id: Option<u8>,
    pub function: Option<u8>,
    pub address: Option<u16>,
    pub register_id: Option<String>,
    pub command_id: Option<String>,
    pub preview_id: Option<u64>,
    pub old_value: Option<f64>,
    pub requested_value: Option<f64>,
    pub raw_value_hex: Option<String>,
    pub request_hex: Option<String>,
    pub response_hex: Option<String>,
    pub exception: Option<u8>,
    pub ack: Option<String>,
    pub readback: Option<String>,
    pub readback_value: Option<f64>,
    pub note: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("audit log write failed: {0}")]
pub struct AuditError(pub String);

/// Append-only destination for audit records.
pub trait AuditSink: Send {
    fn append(&mut self, record: &AuditRecord) -> Result<(), AuditError>;
}

impl<T: AuditSink> AuditSink for Arc<Mutex<T>> {
    fn append(&mut self, record: &AuditRecord) -> Result<(), AuditError> {
        self.lock().unwrap_or_else(|e| e.into_inner()).append(record)
    }
}

/// In-memory audit log (tests, and a fallback view). Cloning shares the log.
#[derive(Clone, Debug, Default)]
pub struct MemoryAudit {
    records: Arc<Mutex<Vec<AuditRecord>>>,
    fail: Arc<Mutex<bool>>,
}

impl MemoryAudit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<AuditRecord> {
        self.records.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Make the next appends fail (tests the fail-closed rule).
    pub fn set_failing(&self, fail: bool) {
        *self.fail.lock().unwrap_or_else(|e| e.into_inner()) = fail;
    }
}

impl AuditSink for MemoryAudit {
    fn append(&mut self, record: &AuditRecord) -> Result<(), AuditError> {
        if *self.fail.lock().unwrap_or_else(|e| e.into_inner()) {
            return Err(AuditError("simulated storage failure".into()));
        }
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(record.clone());
        Ok(())
    }
}
