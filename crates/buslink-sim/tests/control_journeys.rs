//! J1/J2 (commissioning-control design §9): the command service over the
//! simulated RS485 bus with a manual clock. Simulated devices only; this is
//! not hardware evidence.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use buslink_core::clock::{Clock, ManualClock};
use buslink_core::command::audit::{AuditEvent, MemoryAudit};
use buslink_core::command::feedback::{FeedbackState, FeedbackWatch, Observation};
use buslink_core::command::{
    Ack, CommSettingKind, CommandConfig, CommandLink, CommandRequest, CommandService, CommandTarget, DirectLink,
    Operator, Readback, Role,
};
use buslink_core::device::DeviceInstance;
use buslink_core::protocol::pdu::ReadRequest;
use buslink_core::transport::exchange::Exchanger;
use buslink_sim::{Chunking, SharedWorld, SimTransport, SimWorld};
use common::{bundled, rtu_exchanger, sim_link};
use std::sync::Arc;

const STOP_EN: &str = "Software stop. Not an emergency stop.";
const STOP_ID: &str = "Stop perangkat lunak. Bukan tombol darurat.";

struct Rig {
    clock: Arc<ManualClock>,
    world: SharedWorld,
    t: SimTransport,
    ex: Exchanger,
    audit: MemoryAudit,
    svc: CommandService,
}

impl Rig {
    fn new(seed: u64) -> Self {
        let clock = ManualClock::shared(1_800_000_000_000);
        let world = SimWorld::training(seed).shared();
        let t = SimTransport::rtu_at(world.clone(), clock.clone() as Arc<dyn Clock>, Chunking::Whole, 9600);
        let ex = rtu_exchanger(&clock);
        let audit = MemoryAudit::new();
        let svc = CommandService::new(
            clock.clone() as Arc<dyn Clock>,
            Box::new(audit.clone()),
            "journey",
            CommandConfig::default(),
        );
        Self {
            clock,
            world,
            t,
            ex,
            audit,
            svc,
        }
    }

    fn link(&mut self) -> DirectLink<'_> {
        DirectLink {
            transport: &mut self.t,
            exchanger: &mut self.ex,
            timeout_ms: 300,
        }
    }

    fn target(&self, file: &str, unit: u8) -> CommandTarget {
        let link = sim_link("training");
        let p = Arc::new(bundled(file));
        let mut d = DeviceInstance::from_profile("dev", "Simulated device", &link, &p);
        d.unit_id = unit;
        CommandTarget {
            device: d,
            link,
            profile: p,
        }
    }

    fn run(&mut self, request: CommandRequest) -> buslink_core::command::CommandOutcome {
        let p = self
            .svc
            .preview(
                &mut DirectLink {
                    transport: &mut self.t,
                    exchanger: &mut self.ex,
                    timeout_ms: 300,
                },
                request,
            )
            .unwrap();
        self.svc.confirm(p.id).unwrap();
        self.svc
            .execute(&mut DirectLink {
                transport: &mut self.t,
                exchanger: &mut self.ex,
                timeout_ms: 300,
            })
            .unwrap()
    }

    /// Read one holding register as an engineering value (scale applied).
    fn read(&mut self, unit: u8, address: u16, scale: f64) -> Option<f64> {
        let req = ReadRequest::for_table(buslink_core::protocol::pdu::Table::HoldingRegister, address, 1).unwrap();
        CommandLink::read(&mut self.link(), unit, req)
            .ok()
            .and_then(|d| d.as_words().first().copied())
            .map(|v| f64::from(v as i16) * scale)
    }

    /// Advance in `step_ms` steps, feeding a fresh read to the watch, until it resolves.
    fn watch(&mut self, w: &mut FeedbackWatch, unit: u8, address: u16, scale: f64, step_ms: u64) -> FeedbackState {
        for _ in 0..100_000 {
            self.clock.advance(step_ms);
            let v = self.read(unit, address, scale);
            let now = self.clock.utc_ms();
            let st = w.observe(
                Some(Observation {
                    ts_ms: now,
                    good: v.is_some(),
                    value: v,
                }),
                now,
            );
            if st.is_final() {
                return st;
            }
        }
        panic!("watch never resolved");
    }
}

fn named(command: &str, value: Option<f64>) -> CommandRequest {
    CommandRequest::Named {
        command: command.into(),
        value,
    }
}

fn op(role: Role) -> Operator {
    Operator {
        user: "trainee".into(),
        role,
    }
}

fn feedback_of(t: &CommandTarget, command: &str) -> buslink_core::profile::Feedback {
    t.profile.command(command).unwrap().feedback.clone().unwrap()
}

/// J1: arm → 30.0 Hz → read-back verified → not reached while stopped → run → reached → software stop → 0 Hz.
#[test]
fn j1_vfd_journey() {
    let mut r = Rig::new(11);
    let t = r.target("sim-vfd.json", 2);
    let stop = t.profile.command("stop").unwrap();
    assert_eq!((stop.label.get("en"), stop.label.get("id")), (STOP_EN, STOP_ID));
    r.svc.arm(t.clone(), op(Role::Operator)).unwrap();

    // Setpoint: acknowledged and read back, but the drive is stopped.
    let o = r.run(named("set_frequency", Some(30.0)));
    assert_eq!(
        (o.ack, o.readback, o.readback_value),
        (Ack::Acknowledged, Readback::Verified, Some(30.0))
    );
    let start = r.clock.utc_ms();
    let mut w = FeedbackWatch::new("set_frequency", "dev", &feedback_of(&t, "set_frequency"), 30.0, start);
    assert_eq!(w.target, 30.0);
    // Write acknowledged is not process reached target: 65 s later the output is still 0 Hz.
    assert_eq!(r.watch(&mut w, 2, 3, 0.01, 1000), FeedbackState::NotReached);
    // Re-arm (the long wait above idled the arm out) and run.
    assert!(r.svc.expire_if_due(), "60 s idle expires the arm");
    assert!(r.svc.armed_device().is_none());
    r.svc.arm(t.clone(), op(Role::Operator)).unwrap();
    let o = r.run(named("run", None));
    assert_eq!((o.ack, o.readback), (Ack::Acknowledged, Readback::Verified));
    let start = r.clock.utc_ms();
    let mut running = FeedbackWatch::new("run", "dev", &feedback_of(&t, "run"), 1.0, start);
    assert_eq!(r.watch(&mut running, 2, 4, 1.0, 100), FeedbackState::Reached);
    let mut speed = FeedbackWatch::new("set_frequency", "dev", &feedback_of(&t, "set_frequency"), 30.0, start);
    assert_eq!(r.watch(&mut speed, 2, 3, 0.01, 200), FeedbackState::Reached);
    let took = speed.resolved_ms.unwrap() - start;
    // Ramp 5.0 s for 0-50 Hz: 30 Hz takes about 3 s.
    assert!((2_500..=4_000).contains(&took), "reached 30 Hz after {took} ms");

    let o = r.run(named("stop", None));
    assert_eq!(
        (o.ack, o.readback, o.readback_value),
        (Ack::Acknowledged, Readback::Verified, Some(0.0))
    );
    let start = r.clock.utc_ms();
    let mut stopped = FeedbackWatch::new("stop", "dev", &feedback_of(&t, "stop"), 0.0, start);
    assert_eq!(r.watch(&mut stopped, 2, 3, 0.01, 200), FeedbackState::Reached);

    let results: Vec<_> = r
        .audit
        .records()
        .into_iter()
        .filter(|x| x.event == Some(AuditEvent::Result))
        .collect();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|x| x.readback.as_deref() == Some("verified")));
    assert_eq!(
        results
            .iter()
            .map(|x| x.command_id.clone().unwrap())
            .collect::<Vec<_>>(),
        ["set_frequency", "run", "stop"]
    );
    assert!(
        results
            .iter()
            .all(|x| x.profile_version == "1.1.0" && x.unit_id == Some(2))
    );
}

/// J2: temperature controller: setpoint 40.0 °C, heater on, PV reaches 40 ± 1 °C, software stop → output 0 %.
#[test]
fn j2_temperature_journey() {
    let mut r = Rig::new(12);
    let t = r.target("sim-temp-controller.json", 3);
    let off = t.profile.command("heater_off").unwrap();
    assert_eq!(off.label.get("en"), STOP_EN);
    r.svc.arm(t.clone(), op(Role::Operator)).unwrap();
    let o = r.run(named("set_temperature", Some(40.0)));
    assert_eq!((o.readback, o.readback_value), (Readback::Verified, Some(40.0)));
    let o = r.run(named("heater_on", None));
    assert_eq!(o.readback, Readback::Verified);
    let start = r.clock.utc_ms();
    let mut pv = FeedbackWatch::new(
        "set_temperature",
        "dev",
        &feedback_of(&t, "set_temperature"),
        40.0,
        start,
    );
    assert_eq!(r.watch(&mut pv, 3, 0, 0.1, 5_000), FeedbackState::Reached);
    let took = pv.resolved_ms.unwrap() - start;
    assert!(
        took > 10_000 && took < 900_000,
        "PV within 1 °C after {took} ms (τ = 60 s)"
    );

    r.svc.arm(t.clone(), op(Role::Operator)).unwrap();
    let o = r.run(named("heater_off", None));
    assert_eq!((o.readback, o.readback_value), (Readback::Verified, Some(0.0)));
    let start = r.clock.utc_ms();
    let mut out = FeedbackWatch::new("heater_off", "dev", &feedback_of(&t, "heater_off"), 0.0, start);
    assert_eq!(r.watch(&mut out, 3, 2, 0.1, 100), FeedbackState::Reached);
    // Out-of-bounds setpoint: rejected, never clamped, nothing sent.
    let mut link = DirectLink {
        transport: &mut r.t,
        exchanger: &mut r.ex,
        timeout_ms: 300,
    };
    assert!(r.svc.preview(&mut link, named("set_temperature", Some(120.1))).is_err());
    assert!(
        r.svc.preview(&mut link, named("set_temperature", Some(40.05))).is_err(),
        "step 0.1"
    );
}

/// Requirement 6: the simulated device answers a unit-ID or baud change at the
/// old settings, then only at the new ones.
#[test]
fn simulated_comm_registers_move_the_device() {
    let mut r = Rig::new(13);
    let t = r.target("sim-vfd.json", 2);
    r.svc.arm(t, op(Role::Commissioner)).unwrap();
    let o = r.run(CommandRequest::CommConfig {
        setting: CommSettingKind::UnitId,
        value: 9,
    });
    assert_eq!(o.ack, Ack::Acknowledged, "answered from the old unit ID");
    assert_eq!(r.read(2, 3, 1.0), None, "silent at the old unit ID");
    assert_eq!(r.read(9, 10, 1.0), Some(9.0), "answers at the new one");

    // Baud: the temperature controller moves to 19200; the 9600 line no longer reaches it.
    let t3 = r.target("sim-temp-controller.json", 3);
    r.svc.arm(t3, op(Role::Commissioner)).unwrap();
    let o = r.run(CommandRequest::CommConfig {
        setting: CommSettingKind::Baud,
        value: 19_200,
    });
    assert_eq!(o.ack, Ack::Acknowledged);
    assert_eq!(r.read(3, 0, 1.0), None);
    assert_eq!(r.world.lock().unwrap().baud(3), Some(19_200));
    let mut fast = SimTransport::rtu_at(
        r.world.clone(),
        r.clock.clone() as Arc<dyn Clock>,
        Chunking::Whole,
        19_200,
    );
    let mut ex = rtu_exchanger(&r.clock);
    let req = ReadRequest::for_table(buslink_core::protocol::pdu::Table::HoldingRegister, 11, 1).unwrap();
    let words = CommandLink::read(
        &mut DirectLink {
            transport: &mut fast,
            exchanger: &mut ex,
            timeout_ms: 300,
        },
        3,
        req,
    )
    .unwrap()
    .as_words();
    assert_eq!(words, vec![2], "baud code 2 = 19200");
    assert_eq!(
        r.world.lock().unwrap().baud(1),
        Some(9600),
        "other devices keep their baud"
    );
}
