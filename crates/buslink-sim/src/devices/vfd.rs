//! Simulated VFD. Holding registers (simulator design, see profiles/sim-vfd.json):
//! 0 control word (rw: 0 stop, 1 run, 4 reset fault), 1 setpoint ×0.01 Hz (rw 0–5000),
//! 2 ramp time 0–50 Hz ×0.1 s (rw 1–600), 3 output frequency ×0.01 Hz,
//! 4 status (bit0 running, bit1 at speed, bit3 fault), 5 fault code, 6 motor current ×0.1 A,
//! 10 unit ID and 11 baud code (rw, see `comm.rs`).
//!
//! A run command while a fault is latched is acknowledged but the drive stays
//! stopped: "write acknowledged" is not "process reached target".

use super::comm::CommRegs;
use super::{SimModel, register_values};
use crate::image::{ImageBuilder, RegisterImage, quantise};
use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::{ExceptionCode, Table};
use buslink_core::rng::SplitMix64;
use std::any::Any;

pub const MAX_HZ: f64 = 50.0;
pub const FAULT_OVERCURRENT: u16 = 7;

pub struct Vfd {
    pub control_word: u16,
    pub setpoint_hz: f64,
    pub ramp_s: f64,
    pub output_hz: f64,
    pub running: bool,
    pub fault_code: u16,
    pub current_a: f64,
    pub comm: CommRegs,
}

impl Vfd {
    pub fn new() -> Self {
        Self {
            control_word: 0,
            setpoint_hz: 0.0,
            ramp_s: 5.0,
            output_hz: 0.0,
            running: false,
            fault_code: 0,
            current_a: 0.0,
            comm: CommRegs::default(),
        }
    }

    /// Latch a fault (training scenario). The drive coasts to a stop.
    pub fn trip(&mut self, code: u16) {
        self.fault_code = code.max(1);
        self.running = false;
        self.output_hz = 0.0;
    }

    pub fn status(&self) -> u16 {
        let mut s = 0;
        if self.running {
            s |= 1;
        }
        if self.running && (self.output_hz - self.setpoint_hz).abs() < 0.005 {
            s |= 1 << 1;
        }
        if self.fault_code != 0 {
            s |= 1 << 3;
        }
        s
    }
}

impl Default for Vfd {
    fn default() -> Self {
        Self::new()
    }
}

impl SimModel for Vfd {
    fn model_id(&self) -> &'static str {
        "vfd_basic"
    }

    fn step(&mut self, dt_ms: u64, rng: &mut SplitMix64) {
        let dt = dt_ms as f64 / 1000.0;
        let target = if self.running && self.fault_code == 0 {
            self.setpoint_hz
        } else {
            0.0
        };
        let rate = MAX_HZ / self.ramp_s.max(0.1);
        let delta = (target - self.output_hz).clamp(-rate * dt, rate * dt);
        self.output_hz = (self.output_hz + delta).clamp(0.0, MAX_HZ);
        self.current_a = if self.output_hz > 0.0 {
            0.8 + self.output_hz * 0.14 + rng.normal() * 0.02
        } else {
            0.0
        };
    }

    fn image(&self, wrong_order: bool) -> RegisterImage {
        let mut b = ImageBuilder::new(wrong_order);
        let h = Table::HoldingRegister;
        b.u16(h, 0, self.control_word, true)
            .u16(h, 1, quantise(self.setpoint_hz, 0.01, 0.0, 5000.0) as u16, true)
            .u16(h, 2, quantise(self.ramp_s, 0.1, 1.0, 600.0) as u16, true)
            .u16(h, 3, quantise(self.output_hz, 0.01, 0.0, 5000.0) as u16, false)
            .bits(h, 4, self.status())
            .u16(h, 5, self.fault_code, false)
            .u16(h, 6, quantise(self.current_a.max(0.0), 0.1, 0.0, 65535.0) as u16, false);
        self.comm.image(&mut b);
        b.build()
    }

    fn supports(&self, function: u8) -> bool {
        matches!(function, 0x03 | 0x06 | 0x10)
    }

    fn apply(&mut self, write: &WriteRequest) -> Result<(), ExceptionCode> {
        let values = register_values(write).ok_or(ExceptionCode::ILLEGAL_FUNCTION)?;
        for (a, v) in &values {
            let ok = match a {
                0 => matches!(v, 0 | 1 | 4),
                1 => *v <= 5000,
                2 => (1..=600).contains(v),
                a if CommRegs::handles(*a) => CommRegs::check(*a, *v).is_ok(),
                _ => false,
            };
            if !ok {
                return Err(ExceptionCode::ILLEGAL_DATA_VALUE);
            }
        }
        for (a, v) in values {
            match a {
                0 => match v {
                    0 => {
                        self.control_word = 0;
                        self.running = false;
                    }
                    1 => {
                        // Acknowledged; with a latched fault the drive does not start.
                        self.control_word = 1;
                        self.running = self.fault_code == 0;
                    }
                    _ => {
                        if !self.running && self.output_hz == 0.0 {
                            self.fault_code = 0;
                        }
                        self.control_word = if self.running { 1 } else { 0 };
                    }
                },
                1 => self.setpoint_hz = f64::from(v) * 0.01,
                2 => self.ramp_s = f64::from(v) * 0.1,
                a => self.comm.apply(a, v),
            }
        }
        Ok(())
    }

    fn comm(&mut self) -> Option<&mut CommRegs> {
        Some(&mut self.comm)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
