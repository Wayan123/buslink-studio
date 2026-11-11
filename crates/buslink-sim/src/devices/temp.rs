//! Simulated temperature controller. Holding registers (simulator design, see
//! profiles/sim-temp-controller.json): 0 PV i16 ×0.1 °C, 1 SP i16 ×0.1 °C (rw 0–1200),
//! 2 output ×0.1 %, 3 mode (rw 0 auto, 1 manual), 4 manual output ×0.1 % (rw 0–1000),
//! 5 heater enable (rw 0/1).
//!
//! Plant: τ·dPV/dt = ambient + gain·out/100 − PV (τ 60 s, gain 150 °C, ambient 25 °C).
//! Auto mode: PI with anti-windup.

use super::{SimModel, register_values};
use crate::image::{ImageBuilder, RegisterImage, quantise};
use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::{ExceptionCode, Table};
use buslink_core::rng::SplitMix64;
use std::any::Any;

pub const TAU_S: f64 = 60.0;
pub const GAIN_C: f64 = 150.0;
pub const AMBIENT_C: f64 = 25.0;
const KP: f64 = 4.0;
const KI: f64 = 0.08;

pub struct TempController {
    pub pv: f64,
    pub sp: f64,
    pub output: f64,
    pub manual: bool,
    pub manual_output: f64,
    pub heater_enabled: bool,
    integral: f64,
}

impl TempController {
    pub fn new() -> Self {
        Self {
            pv: AMBIENT_C,
            sp: 40.0,
            output: 0.0,
            manual: false,
            manual_output: 0.0,
            heater_enabled: false,
            integral: 0.0,
        }
    }
}

impl Default for TempController {
    fn default() -> Self {
        Self::new()
    }
}

impl SimModel for TempController {
    fn model_id(&self) -> &'static str {
        "temp_controller_basic"
    }

    fn step(&mut self, dt_ms: u64, rng: &mut SplitMix64) {
        let dt = dt_ms as f64 / 1000.0;
        self.output = if !self.heater_enabled {
            0.0
        } else if self.manual {
            self.manual_output
        } else {
            let e = self.sp - self.pv;
            let candidate = self.integral + KI * e * dt;
            let unclamped = KP * e + candidate;
            // Anti-windup: integrate only while the output is not saturated in that direction.
            if (0.0..=100.0).contains(&unclamped) || (unclamped > 100.0 && e < 0.0) || (unclamped < 0.0 && e > 0.0) {
                self.integral = candidate.clamp(-100.0, 100.0);
            }
            (KP * e + self.integral).clamp(0.0, 100.0)
        };
        let target = AMBIENT_C + GAIN_C * self.output / 100.0;
        self.pv += (target - self.pv) * (dt / TAU_S) + rng.normal() * 0.005;
    }

    fn image(&self, wrong_order: bool) -> RegisterImage {
        let mut b = ImageBuilder::new(wrong_order);
        let h = Table::HoldingRegister;
        b.i16(h, 0, quantise(self.pv, 0.1, -32768.0, 32767.0) as i16, false)
            .i16(h, 1, quantise(self.sp, 0.1, 0.0, 1200.0) as i16, true)
            .u16(h, 2, quantise(self.output, 0.1, 0.0, 1000.0) as u16, false)
            .u16(h, 3, u16::from(self.manual), true)
            .u16(h, 4, quantise(self.manual_output, 0.1, 0.0, 1000.0) as u16, true)
            .u16(h, 5, u16::from(self.heater_enabled), true);
        b.build()
    }

    fn supports(&self, function: u8) -> bool {
        matches!(function, 0x03 | 0x06 | 0x10)
    }

    fn apply(&mut self, write: &WriteRequest) -> Result<(), ExceptionCode> {
        let values = register_values(write).ok_or(ExceptionCode::ILLEGAL_FUNCTION)?;
        for (a, v) in &values {
            let ok = match a {
                1 => (0..=1200).contains(&(*v as i16)),
                3 | 5 => *v <= 1,
                4 => *v <= 1000,
                _ => false,
            };
            if !ok {
                return Err(ExceptionCode::ILLEGAL_DATA_VALUE);
            }
        }
        for (a, v) in values {
            match a {
                1 => self.sp = f64::from(v as i16) * 0.1,
                3 => self.manual = v == 1,
                4 => self.manual_output = f64::from(v) * 0.1,
                _ => self.heater_enabled = v == 1,
            }
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
