//! Shared helpers for integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::panic, clippy::type_complexity)]

pub mod slave;

use buslink_core::clock::{Clock, ManualClock};
use buslink_core::device::{LinkConfig, LinkKind};
use buslink_core::poll::scheduler::{LinkScheduler, Step};
use buslink_core::profile::{self, Profile};
use buslink_core::protocol::rtu::SerialSettings;
use buslink_core::quality::Sample;
use buslink_core::transport::Transport;
use std::path::Path;

pub fn bundled(name: &str) -> Profile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles").join(name);
    profile::load(&std::fs::read(path).unwrap()).unwrap().profile
}

pub fn rtu_link(id: &str) -> LinkConfig {
    LinkConfig {
        id: id.into(),
        kind: LinkKind::Rtu {
            port: "COM7".into(),
            serial: SerialSettings::default_9600_8n1(),
        },
        min_inter_frame_ms: 0,
    }
}

pub fn sim_link(id: &str) -> LinkConfig {
    LinkConfig {
        id: id.into(),
        kind: LinkKind::Simulator {
            name: id.into(),
            serial: SerialSettings::default_9600_8n1(),
        },
        min_inter_frame_ms: 0,
    }
}

pub fn tcp_link(id: &str, host: &str) -> LinkConfig {
    LinkConfig {
        id: id.into(),
        kind: LinkKind::Tcp {
            host: host.into(),
            port: 1502,
        },
        min_inter_frame_ms: 0,
    }
}

/// Run the scheduler until the manual clock reaches `until_mono`, collecting samples.
pub fn run_until(
    sched: &mut LinkScheduler,
    t: &mut dyn Transport,
    clock: &ManualClock,
    until_mono: u64,
    out: &mut Vec<Sample>,
) -> Step {
    loop {
        if clock.mono_ms() >= until_mono {
            return Step::Worked;
        }
        match sched.step(t, &mut |s| out.push(s)) {
            Step::Worked => {}
            Step::LinkDown => return Step::LinkDown,
            Step::Idle { until_mono_ms } => clock.advance_to(until_mono_ms.min(until_mono)),
        }
    }
}
