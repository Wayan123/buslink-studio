//! Last request/response exchanges per device, for the register inspector (PRD R15).

use crate::time::hex_spaced;
use crate::transport::exchange::{ExchangeError, ExchangeRecord};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

pub const FRAMES_PER_DEVICE: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FrameEntry {
    pub ts_ms: i64,
    pub unit: u8,
    /// The ADU as sent, spaced hex.
    pub request_hex: String,
    /// Every byte received while waiting, spaced hex.
    pub response_hex: String,
    /// `OK` or the quality this exchange produced (`TIMEOUT`, `CRC_ERROR`, `EXCEPTION`, ...).
    pub outcome: String,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    pub discarded: usize,
}

impl FrameEntry {
    pub fn from_record(rec: &ExchangeRecord, ts_ms: i64) -> Self {
        let (outcome, error) = match &rec.outcome {
            Ok(_) => ("OK".to_owned(), None),
            Err(e) => (outcome_label(e), Some(e.to_string())),
        };
        Self {
            ts_ms,
            unit: rec.unit,
            request_hex: hex_spaced(&rec.request),
            response_hex: hex_spaced(&rec.response),
            outcome,
            error,
            elapsed_ms: rec.elapsed_ms,
            discarded: rec.discarded,
        }
    }
}

fn outcome_label(e: &ExchangeError) -> String {
    e.quality().to_string()
}

#[derive(Default)]
pub struct FrameLog {
    rings: HashMap<(String, u8), VecDeque<FrameEntry>>,
}

impl FrameLog {
    pub fn push(&mut self, link_id: &str, rec: &ExchangeRecord, ts_ms: i64) {
        let ring = self.rings.entry((link_id.to_owned(), rec.unit)).or_default();
        if ring.len() == FRAMES_PER_DEVICE {
            ring.pop_front();
        }
        ring.push_back(FrameEntry::from_record(rec, ts_ms));
    }

    /// Newest first.
    pub fn of(&self, link_id: &str, unit: u8) -> Vec<FrameEntry> {
        self.rings
            .get(&(link_id.to_owned(), unit))
            .map(|r| r.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear_link(&mut self, link_id: &str) {
        self.rings.retain(|(l, _), _| l != link_id);
    }
}
