//! Training mode (PRD R24): a simulated RS485 bus with no hardware, plus the
//! fault buttons of the training screen. Everything here is a **simulated
//! device**; nothing changes a profile's support level (safety rule 12).

use crate::fault::{FaultId, FaultKind, FaultRule, Trigger};
use crate::transport::{Chunking, SimTransport};
use crate::world::{SharedWorld, SimWorld, WorldError};
use buslink_core::alarm::{AlarmDef, AlarmKind};
use buslink_core::clock::Clock;
use buslink_core::device::{LinkConfig, LinkKind};
use buslink_core::protocol::rtu::SerialSettings;
use buslink_core::transport::Transport;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use thiserror::Error;

pub const TRAINING_LINK_ID: &str = "training";

/// Devices added when Training mode starts: (unit ID, profile ID). The world also
/// serves the energy meter at unit 4, which the user can add from the library.
pub const TRAINING_DEVICES: [(u8, &str); 3] = [
    (1, "bgt.smps-8in1"),
    (2, "sim.vfd-basic"),
    (3, "sim.temp-controller-basic"),
];

pub const MAX_FAULT_MS: u64 = 600_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingFault {
    Timeout,
    CrcError,
    Exception02,
    Stale,
    WrongOrder,
}

impl TrainingFault {
    pub fn kind(self) -> FaultKind {
        match self {
            TrainingFault::Timeout => FaultKind::Timeout,
            TrainingFault::CrcError => FaultKind::CrcError,
            TrainingFault::Exception02 => FaultKind::Exception(0x02),
            TrainingFault::Stale => FaultKind::Stale,
            TrainingFault::WrongOrder => FaultKind::WrongOrder,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrainingError {
    #[error("fault duration must be 1 s to 10 min")]
    Duration,
    #[error("unit {0} is not a simulated device on the training bus")]
    Unit(u8),
    #[error(transparent)]
    World(#[from] WorldError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActiveFault {
    pub id: u64,
    pub unit: u8,
    pub fault: TrainingFault,
    pub until_mono_ms: u64,
    pub remaining_ms: u64,
}

pub struct Training {
    world: SharedWorld,
    clock: Arc<dyn Clock>,
    seed: u64,
    faults: Mutex<Vec<(FaultId, u8, TrainingFault, u64)>>,
}

impl Training {
    pub fn new(seed: u64, clock: Arc<dyn Clock>) -> Self {
        Self {
            world: SimWorld::training(seed).shared(),
            clock,
            seed,
            faults: Mutex::new(Vec::new()),
        }
    }

    pub fn world(&self) -> SharedWorld {
        Arc::clone(&self.world)
    }

    /// The training bus link: simulator RTU framing at 9600 8N1.
    pub fn link_config() -> LinkConfig {
        LinkConfig {
            id: TRAINING_LINK_ID.into(),
            kind: LinkKind::Simulator {
                name: "training".into(),
                serial: SerialSettings::default_9600_8n1(),
            },
            min_inter_frame_ms: 0,
        }
    }

    /// In-memory RTU transport at 9600 baud with USB-like random fragmentation.
    pub fn transport(&self) -> Box<dyn Transport> {
        self.transport_at(9600)
    }

    /// Transport for a link: a simulator link runs at its serial baud, so a
    /// device commissioned to another baud stops answering (as on a real bus).
    pub fn transport_for(&self, link: &LinkConfig) -> Box<dyn Transport> {
        self.transport_at(link.serial().map_or(9600, |s| s.baud))
    }

    fn transport_at(&self, baud: u32) -> Box<dyn Transport> {
        Box::new(SimTransport::rtu_at(
            self.world(),
            Arc::clone(&self.clock),
            Chunking::Random {
                max: 8,
                seed: self.seed ^ 0x7EA1,
            },
            baud,
        ))
    }

    /// Inject a fault on one simulated device for `duration_ms` of simulator time.
    pub fn inject(&self, unit: u8, fault: TrainingFault, duration_ms: u64) -> Result<u64, TrainingError> {
        if !(1_000..=MAX_FAULT_MS).contains(&duration_ms) {
            return Err(TrainingError::Duration);
        }
        let mut w = self.world.lock().unwrap_or_else(|e| e.into_inner());
        if w.model_id(unit).is_none() {
            return Err(TrainingError::Unit(unit));
        }
        let now = self.clock.mono_ms();
        let until = now + duration_ms;
        let id = w.add_fault(FaultRule {
            unit,
            kind: fault.kind(),
            trigger: Trigger::Window {
                from_ms: now,
                until_ms: until,
            },
        })?;
        self.faults
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, unit, fault, until));
        Ok(id.0)
    }

    /// Remove every fault of a unit.
    pub fn clear(&self, unit: u8) {
        self.world.lock().unwrap_or_else(|e| e.into_inner()).clear_faults(unit);
        self.faults
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(_, u, _, _)| *u != unit);
    }

    /// Faults whose window has not ended yet.
    pub fn active(&self) -> Vec<ActiveFault> {
        let now = self.clock.mono_ms();
        let mut f = self.faults.lock().unwrap_or_else(|e| e.into_inner());
        f.retain(|(_, _, _, until)| *until > now);
        f.iter()
            .map(|(id, unit, fault, until)| ActiveFault {
                id: id.0,
                unit: *unit,
                fault: *fault,
                until_mono_ms: *until,
                remaining_ms: until - now,
            })
            .collect()
    }
}

/// Training alarm presets. Delays are shortened for the exercise (research/02
/// suggests 60 s for temperature); the message says so.
pub fn alarm_presets(soil: &str, vfd: &str, temp: &str) -> Vec<AlarmDef> {
    vec![
        AlarmDef {
            id: format!("{temp}.pv.hi"),
            device_id: temp.into(),
            tag_id: "pv".into(),
            kind: AlarmKind::Hi,
            limit: 60.0,
            deadband: 3.5,
            on_delay_ms: 5_000,
            off_delay_ms: 5_000,
            priority: 2,
            latch: false,
            message: Some("Process temperature high (training preset, 5 s delay)".into()),
        },
        AlarmDef {
            id: format!("{vfd}.motor_current.hi"),
            device_id: vfd.into(),
            tag_id: "motor_current".into(),
            kind: AlarmKind::Hi,
            limit: 8.0,
            deadband: 0.4,
            on_delay_ms: 5_000,
            off_delay_ms: 5_000,
            priority: 3,
            latch: false,
            message: Some("Motor current high (training preset)".into()),
        },
        AlarmDef {
            id: format!("{soil}.moisture.lo"),
            device_id: soil.into(),
            tag_id: "moisture".into(),
            kind: AlarmKind::Lo,
            limit: 15.0,
            deadband: 1.0,
            on_delay_ms: 5_000,
            off_delay_ms: 5_000,
            priority: 3,
            latch: false,
            message: Some("Soil moisture low (training preset)".into()),
        },
    ]
}
