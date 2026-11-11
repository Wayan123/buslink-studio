#![allow(dead_code, clippy::unwrap_used, clippy::panic, clippy::type_complexity)]

use buslink_core::clock::{Clock, ManualClock};
use buslink_core::device::{LinkConfig, LinkKind};
use buslink_core::profile::{self, Profile};
use buslink_core::protocol::pdu::{ReadData, ReadFunction, ReadRequest, ReplyError, decode_read_response};
use buslink_core::protocol::rtu::SerialSettings;
use buslink_core::transport::Transport;
use buslink_core::transport::exchange::{ExchangeError, Exchanger, Framing, Outgoing};
use std::path::Path;
use std::sync::Arc;

pub fn bundled(name: &str) -> Profile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles").join(name);
    profile::load(&std::fs::read(path).unwrap()).unwrap().profile
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

pub fn rtu_exchanger(clock: &Arc<ManualClock>) -> Exchanger {
    Exchanger::new(Framing::Rtu, clock.clone() as Arc<dyn Clock>, 5)
}

/// One read through the real core exchanger.
pub fn read(
    ex: &mut Exchanger,
    t: &mut dyn Transport,
    unit: u8,
    f: ReadFunction,
    address: u16,
    qty: u16,
) -> Result<ReadData, ExchangeError> {
    let req = ReadRequest::new(f, address, qty).unwrap();
    let pdu = ex.transact(t, unit, Outgoing::Read(&req), 500).outcome?;
    decode_read_response(&req, &pdu).map_err(|e| match e {
        ReplyError::Exception(c) => ExchangeError::Exception(c),
        ReplyError::Frame(f) => ExchangeError::Decode(f),
    })
}

pub fn regs(d: ReadData) -> Vec<u16> {
    match d {
        ReadData::Registers(r) => r,
        ReadData::Bits(_) => panic!("expected registers"),
    }
}

use buslink_core::poll::scheduler::{LinkScheduler, Step};
use buslink_core::quality::Sample;

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
