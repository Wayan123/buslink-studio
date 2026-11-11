//! Simulated device models. Every register map here is a simulator design
//! choice except the BGT-SMPS soil sensor, which follows its manual.

pub mod energy;
pub mod soil;
pub mod temp;
pub mod vfd;

use crate::image::RegisterImage;
use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::ExceptionCode;
use buslink_core::rng::SplitMix64;
use std::any::Any;

/// Fixed simulation step.
pub const STEP_MS: u64 = 100;

pub trait SimModel: Send + Any {
    fn model_id(&self) -> &'static str;
    /// Advance the process by `dt_ms` (always [`STEP_MS`] from the world).
    fn step(&mut self, dt_ms: u64, rng: &mut SplitMix64);
    /// Registers as served now.
    fn image(&self, wrong_order: bool) -> RegisterImage;
    /// Function codes the device implements (others answer exception 01).
    fn supports(&self, function: u8) -> bool;
    /// Apply a write after the server checked that every address exists and is
    /// writable. Validate all values before changing anything.
    fn apply(&mut self, write: &WriteRequest) -> Result<(), ExceptionCode>;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub const MODEL_IDS: [&str; 4] = [
    "bgt_smps_8in1",
    "vfd_basic",
    "temp_controller_basic",
    "energy_meter_basic",
];

/// Create a model by the `transport.simulator.model` ID used in profiles.
pub fn create(model_id: &str) -> Option<Box<dyn SimModel>> {
    Some(match model_id {
        "bgt_smps_8in1" => Box::new(soil::SoilSensor::new()),
        "vfd_basic" => Box::new(vfd::Vfd::new()),
        "temp_controller_basic" => Box::new(temp::TempController::new()),
        "energy_meter_basic" => Box::new(energy::EnergyMeter::new()),
        _ => return None,
    })
}

/// (address, value) pairs of a register write, for models that only take holding registers.
pub fn register_values(w: &WriteRequest) -> Option<Vec<(u16, u16)>> {
    match w {
        WriteRequest::SingleRegister { address, value } => Some(vec![(*address, *value)]),
        WriteRequest::MultipleRegisters { address, values } => Some(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| (address.wrapping_add(i as u16), *v))
                .collect(),
        ),
        _ => None,
    }
}
