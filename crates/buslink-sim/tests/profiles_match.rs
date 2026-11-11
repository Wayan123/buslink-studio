//! S3: every register of every simulator-backed profile reads and decodes GOOD
//! from the simulator, through the core planner and decoder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

mod common;

use buslink_core::clock::{Clock, ManualClock};
use buslink_core::poll::plan::{self, PlanLimits};
use buslink_core::poll::scheduler::decode_tag;
use buslink_core::profile::SupportLevel;
use buslink_core::protocol::pdu::decode_read_response;
use buslink_core::quality::Quality;
use buslink_core::transport::exchange::Outgoing;
use buslink_sim::{Chunking, SimTransport, SimWorld};
use common::*;
use std::sync::Arc;

#[test]
fn every_profile_register_decodes_good() {
    for file in [
        "bgt-smps-8in1.json",
        "sim-vfd.json",
        "sim-temp-controller.json",
        "sim-energy-meter.json",
    ] {
        let p = bundled(file);
        let model = p.transport.simulator.as_ref().unwrap().model.clone();
        let unit = p.transport.rtu.unwrap().unit_id;
        let clock = ManualClock::shared(0);
        let mut world = SimWorld::new(2026);
        world.add_model(unit, &model).unwrap();
        let world = world.shared();
        let mut t = SimTransport::rtu(world, clock.clone() as Arc<dyn Clock>, Chunking::Whole);
        let mut ex = rtu_exchanger(&clock);
        let plan = plan::plan(&p, PlanLimits::default());
        assert!(plan.unreadable.is_empty());
        let mut seen = 0;
        for minute in [0u64, 1, 10, 60] {
            clock.advance_to(minute * 60_000);
            for b in &plan.blocks {
                let pdu = ex
                    .transact(&mut t, unit, Outgoing::Read(&b.request), 500)
                    .outcome
                    .unwrap_or_else(|e| panic!("{file}: {e}"));
                let words = decode_read_response(&b.request, &pdu).unwrap().as_words();
                for tag in &b.tags {
                    let s = usize::from(tag.offset);
                    let obs = decode_tag(&p, tag.register, words[s..s + usize::from(tag.width)].to_vec());
                    assert_eq!(
                        obs.quality,
                        Quality::Good,
                        "{file} {}: {:?}",
                        p.registers[tag.register].id,
                        obs.error
                    );
                    seen += 1;
                }
            }
        }
        assert_eq!(seen, p.registers.len() * 4, "{file}");
        if p.simulated_only {
            assert_eq!(
                p.support_level,
                SupportLevel::Simulated,
                "simulator results never upgrade a profile"
            );
        }
    }
}

#[test]
fn soil_profile_matches_the_documented_map() {
    let p = bundled("bgt-smps-8in1.json");
    assert_eq!(p.support_level, SupportLevel::Documented);
    let ids: Vec<(&str, u16, Option<f64>)> = p
        .registers
        .iter()
        .map(|r| (r.id.as_str(), r.address, r.scale))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("temperature", 0, Some(0.1)),
            ("moisture", 1, Some(0.1)),
            ("ec", 2, None),
            ("salinity", 3, None),
            ("nitrogen", 4, None),
            ("phosphorus", 5, None),
            ("potassium", 6, None),
            ("ph", 7, Some(0.01)),
        ]
    );
    let rtu = p.transport.rtu.unwrap();
    assert_eq!((rtu.unit_id, rtu.baud, rtu.data_bits, rtu.stop_bits), (1, 9600, 8, 1));
}
