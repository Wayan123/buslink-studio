//! The simulated bus: devices by unit ID, a fixed-step process clock, faults
//! and a seed. Same seed + same requests at the same times → identical bytes.

use crate::devices::comm::{CommChange, DEFAULT_BAUD};
use crate::devices::{self, STEP_MS, SimModel};
use crate::fault::{FaultError, FaultId, FaultRule, FaultTable};
use crate::image::RegisterImage;
use crate::server;
use buslink_core::protocol::pdu::{ExceptionCode, exception_response};
use buslink_core::protocol::rtu;
use buslink_core::rng::{GOLDEN_GAMMA, SplitMix64};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Device turnaround added to every reply.
pub const TURNAROUND_MS: u64 = 5;
/// Longest catch-up in one `advance_to` call (1 h of simulated time); beyond
/// that the process jumps instead of stepping, so a paused world never stalls.
const MAX_STEPS_PER_ADVANCE: u64 = 36_000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorldError {
    #[error("unit ID {0} is outside 1..=247")]
    UnitRange(u8),
    #[error("unit ID {0} is already used")]
    UnitTaken(u8),
    #[error("unknown simulator model '{0}'")]
    UnknownModel(String),
    #[error(transparent)]
    Fault(#[from] FaultError),
}

struct SimDevice {
    model: Box<dyn SimModel>,
    rng: SplitMix64,
    requests: u64,
    /// Serial line speed the device listens at (RTU links that carry a baud only).
    baud: u32,
}

/// Result of one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// `None`: the device stays silent.
    pub pdu: Option<Vec<u8>>,
    pub delay_ms: u64,
    /// The transport must corrupt the frame (CRC on RTU, byte count on MBAP).
    pub corrupt: bool,
}

impl Reply {
    fn silent() -> Self {
        Self {
            pdu: None,
            delay_ms: 0,
            corrupt: false,
        }
    }
}

pub struct SimWorld {
    seed: u64,
    devices: BTreeMap<u8, SimDevice>,
    faults: FaultTable,
    fault_rng: SplitMix64,
    last_step_ms: Option<u64>,
}

pub type SharedWorld = Arc<Mutex<SimWorld>>;

impl SimWorld {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            devices: BTreeMap::new(),
            faults: FaultTable::default(),
            fault_rng: SplitMix64::new(seed ^ 0xFA17),
            last_step_ms: None,
        }
    }

    pub fn shared(self) -> SharedWorld {
        Arc::new(Mutex::new(self))
    }

    /// The four training devices at their profile unit IDs: soil 1, VFD 2, temperature 3, meter 4.
    pub fn training(seed: u64) -> Self {
        let mut w = Self::new(seed);
        for (unit, model) in [
            (1, "bgt_smps_8in1"),
            (2, "vfd_basic"),
            (3, "temp_controller_basic"),
            (4, "energy_meter_basic"),
        ] {
            // Units are distinct and in range; the error branch cannot trigger.
            let _ = w.add_model(unit, model);
        }
        w
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn add_device(&mut self, unit: u8, mut model: Box<dyn SimModel>) -> Result<(), WorldError> {
        if !(1..=247).contains(&unit) {
            return Err(WorldError::UnitRange(unit));
        }
        if self.devices.contains_key(&unit) {
            return Err(WorldError::UnitTaken(unit));
        }
        let rng = SplitMix64::new(self.seed ^ u64::from(unit).wrapping_mul(GOLDEN_GAMMA));
        if let Some(c) = model.comm() {
            c.unit = unit;
        }
        self.devices.insert(
            unit,
            SimDevice {
                model,
                rng,
                requests: 0,
                baud: DEFAULT_BAUD,
            },
        );
        Ok(())
    }

    /// Baud the device at `unit` listens at.
    pub fn baud(&self, unit: u8) -> Option<u32> {
        self.devices.get(&unit).map(|d| d.baud)
    }

    /// Apply unit-ID / baud changes a write left behind (after its reply was built).
    /// A new unit ID that is already taken is refused: the device keeps its old ID.
    fn apply_comm_changes(&mut self, unit: u8) {
        let changes = match self.devices.get_mut(&unit).and_then(|d| d.model.comm()) {
            Some(c) => c.take_pending(),
            None => return,
        };
        let mut unit = unit;
        for ch in changes {
            match ch {
                CommChange::Baud(b) => {
                    if let Some(d) = self.devices.get_mut(&unit) {
                        d.baud = b;
                        if let Some(c) = d.model.comm() {
                            c.baud = b;
                        }
                    }
                }
                CommChange::Unit(new) => {
                    if new == unit || !(1..=247).contains(&new) || self.devices.contains_key(&new) {
                        continue;
                    }
                    if let Some(mut d) = self.devices.remove(&unit) {
                        if let Some(c) = d.model.comm() {
                            c.unit = new;
                        }
                        self.devices.insert(new, d);
                        self.faults.clear_unit(unit);
                        unit = new;
                    }
                }
            }
        }
    }

    pub fn add_model(&mut self, unit: u8, model_id: &str) -> Result<(), WorldError> {
        let model = devices::create(model_id).ok_or_else(|| WorldError::UnknownModel(model_id.to_owned()))?;
        self.add_device(unit, model)
    }

    pub fn units(&self) -> Vec<u8> {
        self.devices.keys().copied().collect()
    }

    pub fn model_id(&self, unit: u8) -> Option<&'static str> {
        self.devices.get(&unit).map(|d| d.model.model_id())
    }

    pub fn add_fault(&mut self, rule: FaultRule) -> Result<FaultId, WorldError> {
        Ok(self.faults.add(rule)?)
    }

    pub fn remove_fault(&mut self, id: FaultId) -> bool {
        self.faults.remove(id)
    }

    pub fn clear_faults(&mut self, unit: u8) {
        self.faults.clear_unit(unit);
    }

    /// Typed access to a model (training scenarios, tests).
    pub fn with_model<T: 'static, R>(&mut self, unit: u8, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let d = self.devices.get_mut(&unit)?;
        d.model.as_any_mut().downcast_mut::<T>().map(f)
    }

    pub fn image(&self, unit: u8) -> Option<RegisterImage> {
        self.devices.get(&unit).map(|d| d.model.image(false))
    }

    /// Run fixed 100 ms steps up to `now_ms`. Devices under the stale fault are not stepped.
    pub fn advance_to(&mut self, now_ms: u64) {
        let Some(mut last) = self.last_step_ms else {
            self.last_step_ms = Some(now_ms);
            return;
        };
        if now_ms <= last {
            return;
        }
        let mut steps = (now_ms - last) / STEP_MS;
        if steps > MAX_STEPS_PER_ADVANCE {
            last += (steps - MAX_STEPS_PER_ADVANCE) * STEP_MS;
            steps = MAX_STEPS_PER_ADVANCE;
        }
        for _ in 0..steps {
            last += STEP_MS;
            for (unit, d) in self.devices.iter_mut() {
                if !self.faults.stale(*unit, last, d.requests) {
                    d.model.step(STEP_MS, &mut d.rng);
                }
            }
        }
        self.last_step_ms = Some(last);
    }

    /// Handle one request PDU addressed to `unit` at simulated time `now_ms`.
    pub fn handle_pdu(&mut self, unit: u8, pdu: &[u8], now_ms: u64) -> Reply {
        self.advance_to(now_ms);
        if unit == 0 {
            // Broadcast: writes apply to every device, nobody answers (serial line spec).
            if pdu.first().is_some_and(|fc| matches!(fc, 0x05 | 0x06 | 0x0F | 0x10)) {
                for d in self.devices.values_mut() {
                    let _ = server::handle(d.model.as_mut(), pdu, false);
                    // A broadcast never moves a device to another address or baud.
                    if let Some(c) = d.model.comm() {
                        c.take_pending();
                    }
                }
            }
            return Reply::silent();
        }
        let Some(d) = self.devices.get_mut(&unit) else {
            return Reply::silent();
        };
        let request = d.requests;
        d.requests += 1;
        let effect = self.faults.evaluate(unit, now_ms, request, &mut self.fault_rng);
        if effect.silent {
            return Reply::silent();
        }
        let pdu = match effect.exception {
            Some(code) => exception_response(pdu.first().copied().unwrap_or(0), ExceptionCode(code)).to_vec(),
            None => server::handle(d.model.as_mut(), pdu, effect.wrong_order),
        };
        self.apply_comm_changes(unit);
        Reply {
            pdu: Some(pdu),
            delay_ms: TURNAROUND_MS + effect.delay_ms,
            corrupt: effect.corrupt,
        }
    }

    /// Handle one RTU ADU. Bad CRC or unknown unit → silence.
    /// Returns the reply ADU and its delay.
    pub fn handle_rtu(&mut self, adu: &[u8], now_ms: u64) -> Option<(Vec<u8>, u64)> {
        self.handle_rtu_at(adu, now_ms, None)
    }

    /// Like [`handle_rtu`](Self::handle_rtu) on a line running at `baud`: a device
    /// listening at another baud does not understand the frame and stays silent.
    pub fn handle_rtu_at(&mut self, adu: &[u8], now_ms: u64, baud: Option<u32>) -> Option<(Vec<u8>, u64)> {
        let (unit, pdu) = match rtu::decode_adu(adu) {
            Ok(x) => x,
            Err(_) => {
                self.advance_to(now_ms);
                return None;
            }
        };
        if let Some(b) = baud
            && self.devices.get(&unit).is_some_and(|d| d.baud != b)
        {
            self.advance_to(now_ms);
            return None;
        }
        let reply = self.handle_pdu(unit, pdu, now_ms);
        let pdu = reply.pdu?;
        let mut out = rtu::encode_adu(unit, &pdu).ok()?;
        if reply.corrupt {
            let last = out.len() - 1;
            out[last] ^= 0x5A;
        }
        Some((out, reply.delay_ms))
    }
}
