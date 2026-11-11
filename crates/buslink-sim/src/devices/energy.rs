//! Simulated single-phase energy meter. Input registers, f32/u32/u64 in ABCD
//! (simulator design, see profiles/sim-energy-meter.json): 0 voltage V,
//! 2 current A, 4 active power W, 6 power factor, 8 frequency Hz,
//! 10 energy u32 ×0.01 kWh, 12 energy u64 Wh. Read-only.

use super::SimModel;
use crate::image::{ImageBuilder, RegisterImage};
use buslink_core::codec::{DataType, RawValue, WordOrder};
use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::{ExceptionCode, Table};
use buslink_core::rng::SplitMix64;
use std::any::Any;

pub struct EnergyMeter {
    pub t_s: f64,
    pub voltage: f64,
    pub current: f64,
    pub power_w: f64,
    pub pf: f64,
    pub freq: f64,
    /// Accumulated energy in Wh (never decreases).
    pub energy_wh: f64,
}

impl EnergyMeter {
    pub fn new() -> Self {
        Self {
            t_s: 0.0,
            voltage: 230.0,
            current: 0.0,
            power_w: 0.0,
            pf: 0.92,
            freq: 50.0,
            energy_wh: 123_456.0,
        }
    }
}

impl Default for EnergyMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl SimModel for EnergyMeter {
    fn model_id(&self) -> &'static str {
        "energy_meter_basic"
    }

    fn step(&mut self, dt_ms: u64, rng: &mut SplitMix64) {
        let dt = dt_ms as f64 / 1000.0;
        self.t_s += dt;
        let base = 1500.0 + 800.0 * (std::f64::consts::TAU * self.t_s / 600.0).sin();
        self.power_w = (base + rng.normal() * 20.0).max(0.0);
        self.voltage = 230.0 + rng.normal() * 0.8;
        self.pf = (0.92 + rng.normal() * 0.005).clamp(0.5, 1.0);
        self.freq = 50.0 + rng.normal() * 0.01;
        self.current = self.power_w / (self.voltage * self.pf);
        self.energy_wh += self.power_w * dt / 3600.0;
    }

    fn image(&self, wrong_order: bool) -> RegisterImage {
        let mut b = ImageBuilder::new(wrong_order);
        let i = Table::InputRegister;
        let o = Some(WordOrder::ABCD);
        for (addr, v) in [
            (0, self.voltage),
            (2, self.current),
            (4, self.power_w),
            (6, self.pf),
            (8, self.freq),
        ] {
            b.value(i, addr, DataType::F32, o, RawValue::F32(v as f32), false);
        }
        let kwh_hundredths = (self.energy_wh / 10.0).floor().clamp(0.0, f64::from(u32::MAX)) as u32;
        b.value(i, 10, DataType::U32, o, RawValue::U32(kwh_hundredths), false);
        b.value(
            i,
            12,
            DataType::U64,
            o,
            RawValue::U64(self.energy_wh.floor().max(0.0) as u64),
            false,
        );
        b.build()
    }

    fn supports(&self, function: u8) -> bool {
        function == 0x04
    }

    fn apply(&mut self, _write: &WriteRequest) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::ILLEGAL_FUNCTION)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
