//! BGT-SMPS 8-in-1 soil sensor (simulated). Register map as documented in the
//! manual (pages 3-4): holding registers 0-7, FC03 only, read-only.
//! The drift model and its mean values are simulator choices, not field data.

use super::SimModel;
use crate::image::{ImageBuilder, RegisterImage, quantise};
use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::{ExceptionCode, Table};
use buslink_core::rng::SplitMix64;
use std::any::Any;

/// One mean-reverting channel (Ornstein–Uhlenbeck, bounded).
#[derive(Clone, Debug)]
pub struct Channel {
    pub value: f64,
    mean: f64,
    theta_per_s: f64,
    sigma: f64,
    lo: f64,
    hi: f64,
}

impl Channel {
    const fn new(mean: f64, theta_per_s: f64, sigma: f64, lo: f64, hi: f64) -> Self {
        Self {
            value: mean,
            mean,
            theta_per_s,
            sigma,
            lo,
            hi,
        }
    }

    fn step(&mut self, dt_s: f64, rng: &mut SplitMix64) {
        let dx = self.theta_per_s * (self.mean - self.value) * dt_s + self.sigma * dt_s.sqrt() * rng.normal();
        self.value = (self.value + dx).clamp(self.lo, self.hi);
    }
}

pub struct SoilSensor {
    /// temperature °C, moisture %, EC µS/cm, salinity, N, P, K mg/kg, pH.
    pub channels: [Channel; 8],
}

impl SoilSensor {
    pub fn new() -> Self {
        Self {
            channels: [
                Channel::new(24.0, 0.002, 0.02, -30.0, 70.0),
                Channel::new(32.0, 0.002, 0.03, 0.0, 100.0),
                Channel::new(450.0, 0.002, 1.0, 0.0, 20_000.0),
                Channel::new(240.0, 0.002, 0.6, 0.0, 20_000.0),
                Channel::new(40.0, 0.001, 0.08, 0.0, 1999.0),
                Channel::new(15.0, 0.001, 0.04, 0.0, 1999.0),
                Channel::new(90.0, 0.001, 0.15, 0.0, 1999.0),
                Channel::new(6.8, 0.002, 0.002, 3.0, 9.0),
            ],
        }
    }

    /// Raw register values as the sensor would send them.
    pub fn registers(&self) -> [u16; 8] {
        let c = &self.channels;
        [
            quantise(c[0].value, 0.1, -32768.0, 32767.0) as i16 as u16,
            quantise(c[1].value, 0.1, 0.0, 65535.0) as u16,
            quantise(c[2].value, 1.0, 0.0, 65535.0) as u16,
            quantise(c[3].value, 1.0, 0.0, 65535.0) as u16,
            quantise(c[4].value, 1.0, 0.0, 65535.0) as u16,
            quantise(c[5].value, 1.0, 0.0, 65535.0) as u16,
            quantise(c[6].value, 1.0, 0.0, 65535.0) as u16,
            quantise(c[7].value, 0.01, 0.0, 65535.0) as u16,
        ]
    }
}

impl Default for SoilSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl SimModel for SoilSensor {
    fn model_id(&self) -> &'static str {
        "bgt_smps_8in1"
    }

    fn step(&mut self, dt_ms: u64, rng: &mut SplitMix64) {
        let dt = dt_ms as f64 / 1000.0;
        for c in &mut self.channels {
            c.step(dt, rng);
        }
    }

    fn image(&self, wrong_order: bool) -> RegisterImage {
        let mut b = ImageBuilder::new(wrong_order);
        for (i, r) in self.registers().iter().enumerate() {
            b.u16(Table::HoldingRegister, i as u16, *r, false);
        }
        b.build()
    }

    fn supports(&self, function: u8) -> bool {
        function == 0x03
    }

    fn apply(&mut self, _write: &WriteRequest) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::ILLEGAL_FUNCTION)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
