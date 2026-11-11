//! Simulator driving the core (sim Req 10): S5 faults → qualities, S6 fairness
//! with a dead device, S7 control with read-back, S10 write round trip,
//! historian + export of a session with a communication gap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

mod common;

use buslink_core::bus::{BusRecv, SampleBus};
use buslink_core::clock::{Clock, ManualClock, SystemClock};
use buslink_core::command::audit::MemoryAudit;
use buslink_core::command::wire::{MAX_WRITE_COILS, MAX_WRITE_REGISTERS, WriteRequest};
use buslink_core::command::{
    Ack, CommandConfig, CommandRequest, CommandService, CommandTarget, DirectLink, Operator, Readback, Role,
};
use buslink_core::device::{DeviceInstance, LinkConfig};
use buslink_core::export::{self, ExportDoc};
use buslink_core::historian::batch::{LogDecimator, SampleBatcher};
use buslink_core::historian::{DeviceRow, Historian};
use buslink_core::poll::scheduler::LinkScheduler;
use buslink_core::poll::worker::LinkWorker;
use buslink_core::profile::Profile;
use buslink_core::protocol::pdu::{ExceptionCode, ReadFunction, Table};
use buslink_core::quality::{Quality, Sample};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;
use buslink_core::transport::LinkRegistry;
use buslink_sim::devices::SimModel;
use buslink_sim::devices::vfd::{FAULT_OVERCURRENT, Vfd};
use buslink_sim::image::{ImageBuilder, RegisterImage};
use buslink_sim::{Chunking, FaultKind, FaultRule, SharedWorld, SimTransport, SimWorld, Trigger};
use common::*;
use std::any::Any;
use std::sync::{Arc, Mutex};

const UTC0: i64 = 1_800_000_000_000;

fn device(file: &str, link: &LinkConfig, interval: u64) -> (DeviceInstance, Arc<Profile>) {
    let p = Arc::new(bundled(file));
    let id = p.profile_id.replace('.', "-");
    let mut d = DeviceInstance::from_profile(&id, &id, link, &p);
    d.poll_interval_ms = interval;
    d.response_timeout_ms = Some(500);
    (d, p)
}

fn setup(world: SharedWorld, files: &[&str], interval: u64) -> (LinkScheduler, SimTransport, Arc<ManualClock>) {
    let clock = ManualClock::shared(UTC0);
    let link = sim_link("training");
    let devices = files.iter().map(|f| device(f, &link, interval)).collect();
    let s = LinkScheduler::new(link, devices, clock.clone() as Arc<dyn Clock>).unwrap();
    let t = SimTransport::rtu(
        world,
        clock.clone() as Arc<dyn Clock>,
        Chunking::Random { max: 7, seed: 3 },
    );
    (s, t, clock)
}

fn of<'a>(out: &'a [Sample], device: &str, tag: &str) -> Vec<&'a Sample> {
    out.iter()
        .filter(|s| s.device_id == device && s.tag_id == tag)
        .collect()
}

/// S5: each fault kind as the client sees it.
#[test]
fn faults_become_client_qualities() {
    let cases: Vec<(FaultKind, &str, &str)> = vec![
        (FaultKind::Timeout, "bgt-smps-8in1", "temperature"),
        (FaultKind::CrcError, "bgt-smps-8in1", "temperature"),
        (FaultKind::Exception(4), "bgt-smps-8in1", "temperature"),
        (FaultKind::WrongOrder, "bgt-smps-8in1", "temperature"),
    ];
    for (kind, dev, tag) in cases {
        let mut w = SimWorld::training(77);
        w.add_fault(FaultRule {
            unit: 1,
            kind,
            trigger: Trigger::Window {
                from_ms: 5_000,
                until_ms: 15_000,
            },
        })
        .unwrap();
        let (mut s, mut t, clock) = setup(w.shared(), &["bgt-smps-8in1.json"], 1000);
        let mut out = Vec::new();
        run_until(&mut s, &mut t, &clock, 40_000, &mut out);
        let q: Vec<Quality> = of(&out, dev, tag).iter().map(|x| x.quality).collect();
        let expected = match kind {
            FaultKind::Timeout => Quality::Timeout,
            FaultKind::CrcError => Quality::CrcError,
            FaultKind::Exception(c) => Quality::Exception(c),
            _ => Quality::DecodeError, // 24 °C byte-swapped is far outside -30..70 °C
        };
        assert_eq!(q.first(), Some(&Quality::Good), "{kind:?}");
        assert!(q.contains(&expected), "{kind:?}: {q:?}");
        assert_eq!(q.last(), Some(&Quality::Good), "{kind:?}: recovers after the window");
        let mut last_good_ts = None;
        for f in of(&out, dev, tag) {
            if f.quality.is_good() {
                last_good_ts = Some(f.ts_ms);
            } else {
                assert_eq!(
                    f.value_ts_ms, last_good_ts,
                    "{kind:?}: the last good value keeps its own timestamp"
                );
                assert!(f.fresh_value().is_none(), "never shown as a fresh reading");
            }
        }
    }
}

#[test]
fn wrong_word_order_on_floats_is_visible() {
    let mut w = SimWorld::training(5);
    w.add_fault(FaultRule {
        unit: 4,
        kind: FaultKind::WrongOrder,
        trigger: Trigger::Always,
    })
    .unwrap();
    let world = w.shared();
    let (mut s, mut t, clock) = setup(world.clone(), &["sim-energy-meter.json"], 1000);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 5_000, &mut out);
    for v in of(&out, "sim-energy-meter-basic", "voltage") {
        // Either rejected by the range check, or clearly not ~230 V.
        match v.quality {
            Quality::DecodeError => assert!(v.raw.is_some(), "raw registers are kept"),
            Quality::Good => assert!((v.fresh_value().unwrap() - 230.0).abs() > 10.0),
            q => panic!("{q:?}"),
        }
    }
}

#[test]
fn stale_fault_keeps_good_quality_with_frozen_values() {
    let mut w = SimWorld::training(5);
    w.add_fault(FaultRule {
        unit: 4,
        kind: FaultKind::Stale,
        trigger: Trigger::Always,
    })
    .unwrap();
    let (mut s, mut t, clock) = setup(w.shared(), &["sim-energy-meter.json"], 1000);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 10_000, &mut out);
    let e: Vec<&Sample> = of(&out, "sim-energy-meter-basic", "energy_wh_total");
    assert!(e.len() >= 9 && e.iter().all(|x| x.quality == Quality::Good));
    assert!(
        e.windows(2).all(|p| p[0].value == p[1].value),
        "a frozen source looks GOOD: the trainer explains why"
    );
}

#[test]
fn late_reply_is_a_timeout_and_is_never_used_for_the_next_poll() {
    let mut w = SimWorld::training(5);
    w.add_fault(FaultRule {
        unit: 1,
        kind: FaultKind::LateReply { delay_ms: 650 },
        trigger: Trigger::Requests { skip: 3, count: 1 },
    })
    .unwrap();
    let world = w.shared();
    let clock = ManualClock::shared(UTC0);
    let link = sim_link("training");
    let (mut d, p) = device("bgt-smps-8in1.json", &link, 1000);
    d.read_retries = 0;
    let mut s = LinkScheduler::new(link, vec![(d, p)], clock.clone() as Arc<dyn Clock>).unwrap();
    let mut t = SimTransport::rtu(world.clone(), clock.clone() as Arc<dyn Clock>, Chunking::Whole);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 10_000, &mut out);
    let q: Vec<Quality> = of(&out, "bgt-smps-8in1", "ph").iter().map(|x| x.quality).collect();
    assert_eq!(
        q[..5],
        [
            Quality::Good,
            Quality::Good,
            Quality::Good,
            Quality::Timeout,
            Quality::Good
        ]
    );
    assert!(
        s.counters()[0].1.crc_errors == 0 && s.counters()[0].1.decode_errors == 0,
        "the late frame never confused the next poll"
    );
}

/// S6: a dead device never starves the others on the same bus.
#[test]
fn dead_soil_sensor_does_not_starve_vfd_and_meter() {
    check("sim_fairness", 20, |r: &mut SplitMix64| {
        let mut w = SimWorld::training(r.next_u64());
        w.add_fault(FaultRule {
            unit: 1,
            kind: FaultKind::Timeout,
            trigger: Trigger::Always,
        })
        .unwrap();
        let interval = *r.pick(&[500u64, 1000, 2000]).unwrap();
        let (mut s, mut t, clock) = setup(
            w.shared(),
            &["bgt-smps-8in1.json", "sim-vfd.json", "sim-energy-meter.json"],
            interval,
        );
        let mut out = Vec::new();
        run_until(&mut s, &mut t, &clock, 120_000, &mut out);
        // Bound: own interval + the dead device's worst case (2 attempts × (timeout + late guard)) + the other device's reads.
        let bound = interval + 2 * (500 + 210) + 400;
        for (dev, tag) in [("sim-vfd-basic", "output_freq"), ("sim-energy-meter-basic", "voltage")] {
            let ts: Vec<i64> = of(&out, dev, tag)
                .iter()
                .filter(|x| x.quality.is_good())
                .map(|x| x.ts_ms)
                .collect();
            assert!(ts.len() as u64 >= 120_000 / bound, "{dev}: {} GOOD samples", ts.len());
            let worst = ts.windows(2).map(|p| p[1] - p[0]).max().unwrap();
            assert!(worst as u64 <= bound, "{dev}: gap {worst} ms > {bound} ms");
        }
        let soil = &s.counters()[0].1;
        assert!(soil.consecutive_failures > 3 && soil.good == 0);
        assert!(
            soil.requests < 40,
            "backoff limits wasted bus time ({} requests)",
            soil.requests
        );
    });
}

// -------------------------------------------------------------------- control (S7)

struct Bench {
    clock: Arc<ManualClock>,
    world: SharedWorld,
    t: SimTransport,
    ex: buslink_core::transport::exchange::Exchanger,
}

impl Bench {
    fn new() -> Self {
        let clock = ManualClock::shared(UTC0);
        let world = SimWorld::training(2026).shared();
        let t = SimTransport::rtu(
            world.clone(),
            clock.clone() as Arc<dyn Clock>,
            Chunking::Random { max: 5, seed: 1 },
        );
        let ex = rtu_exchanger(&clock);
        Self { clock, world, t, ex }
    }
    fn link(&mut self) -> DirectLink<'_> {
        DirectLink {
            transport: &mut self.t,
            exchanger: &mut self.ex,
            timeout_ms: 500,
        }
    }
    fn hr(&mut self, unit: u8, a: u16) -> u16 {
        regs(read(&mut self.ex, &mut self.t, unit, ReadFunction::HoldingRegisters, a, 1).unwrap())[0]
    }
}

fn target(file: &str) -> CommandTarget {
    let link = sim_link("training");
    let (d, p) = device(file, &link, 1000);
    CommandTarget {
        device: d,
        link,
        profile: p,
    }
}

fn run_cmd(svc: &mut CommandService, b: &mut Bench, req: CommandRequest) -> buslink_core::command::CommandOutcome {
    let p = svc.preview(&mut b.link(), req).unwrap();
    svc.confirm(p.id).unwrap();
    svc.execute(&mut b.link()).unwrap()
}

#[test]
fn vfd_set_frequency_run_stop_with_readback() {
    let mut b = Bench::new();
    let audit = MemoryAudit::new();
    let mut svc = CommandService::new(
        b.clock.clone() as Arc<dyn Clock>,
        Box::new(audit.clone()),
        "s",
        CommandConfig::default(),
    );
    svc.arm(
        target("sim-vfd.json"),
        Operator {
            user: "judge".into(),
            role: Role::Operator,
        },
    )
    .unwrap();
    let p = svc
        .preview(
            &mut b.link(),
            CommandRequest::Named {
                command: "set_frequency".into(),
                value: Some(30.0),
            },
        )
        .unwrap();
    assert_eq!(
        (p.old_value, p.new_value, p.raw_registers.clone()),
        (Some(0.0), 30.0, vec![3000])
    );
    assert!(p.frame_hex.starts_with("02 06 00 01 0B B8"));
    svc.confirm(p.id).unwrap();
    let o = svc.execute(&mut b.link()).unwrap();
    assert_eq!(
        (o.ack, o.readback, o.readback_value),
        (Ack::Acknowledged, Readback::Verified, Some(30.0))
    );
    let o = run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "run".into(),
            value: None,
        },
    );
    assert_eq!((o.ack, o.readback), (Ack::Acknowledged, Readback::Verified));
    assert!(b.hr(2, 3) < 3000, "acknowledged is not 'at speed' yet");
    b.clock.advance(8_000);
    assert_eq!(b.hr(2, 3), 3000, "process reached target (measured feedback)");
    assert_eq!(b.hr(2, 4) & 0b11, 0b11);
    let o = run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "stop".into(),
            value: None,
        },
    );
    assert_eq!(o.readback, Readback::Verified);
    b.clock.advance(8_000);
    assert_eq!(b.hr(2, 3), 0);
    assert!(
        audit.records().len() >= 13,
        "arm + 3 × (preview, confirm, sent, result)"
    );
}

#[test]
fn vfd_fault_run_is_acknowledged_but_not_running() {
    let mut b = Bench::new();
    b.world
        .lock()
        .unwrap()
        .with_model::<Vfd, ()>(2, |v| v.trip(FAULT_OVERCURRENT))
        .unwrap();
    let mut svc = CommandService::new(
        b.clock.clone() as Arc<dyn Clock>,
        Box::new(MemoryAudit::new()),
        "s",
        CommandConfig::default(),
    );
    svc.arm(
        target("sim-vfd.json"),
        Operator {
            user: "judge".into(),
            role: Role::Operator,
        },
    )
    .unwrap();
    run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "set_frequency".into(),
            value: Some(20.0),
        },
    );
    let o = run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "run".into(),
            value: None,
        },
    );
    assert_eq!(
        (o.ack, o.readback),
        (Ack::Acknowledged, Readback::Verified),
        "the control word reads back 1"
    );
    b.clock.advance(5_000);
    assert_eq!(b.hr(2, 4), 1 << 3, "status: fault, not running");
    assert_eq!(b.hr(2, 3), 0);
    // The drive clears a fault only while stopped.
    run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "stop".into(),
            value: None,
        },
    );
    let o = run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "reset_fault".into(),
            value: None,
        },
    );
    assert_eq!(
        (o.readback, o.readback_value),
        (Readback::Verified, Some(0.0)),
        "fault code read back as 0"
    );
}

#[test]
fn temperature_setpoint_and_heater_reach_target() {
    let mut b = Bench::new();
    let mut svc = CommandService::new(
        b.clock.clone() as Arc<dyn Clock>,
        Box::new(MemoryAudit::new()),
        "s",
        CommandConfig {
            arm_timeout_ms: 3_600_000,
        },
    );
    svc.arm(
        target("sim-temp-controller.json"),
        Operator {
            user: "judge".into(),
            role: Role::Operator,
        },
    )
    .unwrap();
    let o = run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "set_temperature".into(),
            value: Some(60.0),
        },
    );
    assert_eq!(o.readback, Readback::Verified);
    assert!(matches!(
        svc.preview(
            &mut b.link(),
            CommandRequest::Named {
                command: "set_temperature".into(),
                value: Some(130.0)
            }
        ),
        Err(buslink_core::command::CommandError::OutOfBounds(_))
    ));
    run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "heater_on".into(),
            value: None,
        },
    );
    b.clock.advance(1_800_000);
    let pv = f64::from(b.hr(3, 0) as i16) / 10.0;
    assert!((pv - 60.0).abs() < 1.0, "PV {pv}");
    let o = run_cmd(
        &mut svc,
        &mut b,
        CommandRequest::Named {
            command: "heater_off".into(),
            value: None,
        },
    );
    assert_eq!(o.readback, Readback::Verified);
}

#[test]
fn command_through_the_link_worker_with_audit_in_the_historian() {
    let clock = SystemClock::shared();
    let world = SimWorld::training(1).shared();
    let link = sim_link("training");
    let (d, p) = device("sim-vfd.json", &link, 200);
    let sched = LinkScheduler::new(link.clone(), vec![(d.clone(), p.clone())], clock.clone()).unwrap();
    let t = SimTransport::rtu(world.clone(), clock.clone(), Chunking::Random { max: 4, seed: 9 });
    let registry = LinkRegistry::new();
    let bus = SampleBus::new(256);
    let mut sub = bus.subscribe();
    let worker = LinkWorker::spawn(
        registry.claim(&link.owner_key()).unwrap(),
        sched,
        Box::new(t),
        bus.sink(),
    )
    .unwrap();
    let mut hist = Historian::open_in_memory().unwrap();
    hist.begin_session("s-worker", UTC0, "0.1.0", "test", 1000).unwrap();
    let hist = Arc::new(Mutex::new(hist));
    let mut svc = CommandService::new(
        clock.clone(),
        Box::new(hist.clone()),
        "s-worker",
        CommandConfig::default(),
    );
    svc.arm(
        CommandTarget {
            device: d,
            link,
            profile: p,
        },
        Operator {
            user: "judge".into(),
            role: Role::Operator,
        },
    )
    .unwrap();
    let mut handle = worker.handle(500);
    let pv = svc
        .preview(
            &mut handle,
            CommandRequest::SetRegister {
                register: "freq_setpoint".into(),
                value: 12.5,
            },
        )
        .unwrap();
    svc.confirm(pv.id).unwrap();
    let o = svc.execute(&mut handle).unwrap();
    assert_eq!((o.ack, o.readback), (Ack::Acknowledged, Readback::Verified));
    assert_eq!(world.lock().unwrap().image(2).unwrap().holding[&1], 1250);
    let rows = hist.lock().unwrap().audit("s-worker").unwrap();
    let events: Vec<&str> = rows.iter().filter_map(|r| r.event.as_deref()).collect();
    assert_eq!(events, vec!["arm", "preview", "confirm", "sent", "result"]);
    let last = rows.last().unwrap();
    assert_eq!(
        (last.readback.as_deref(), last.function, last.address),
        (Some("verified"), Some(6), Some(1))
    );
    assert!(last.request_hex.as_deref().unwrap().starts_with("02 06 00 01 04 E2"));
    // Polling continued around the command.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut setpoint_seen = false;
    while !setpoint_seen && std::time::Instant::now() < deadline {
        match sub.try_recv() {
            BusRecv::Sample(s) if s.tag_id == "freq_setpoint" && s.fresh_value() == Some(12.5) => setpoint_seen = true,
            BusRecv::Empty => std::thread::sleep(std::time::Duration::from_millis(5)),
            _ => {}
        }
    }
    assert!(setpoint_seen, "the poll loop reports the new setpoint");
    worker.stop();
}

// -------------------------------------------------------------------- S10

/// Test-only model with coils and registers, to exercise every write FC.
struct Bank {
    coils: Vec<bool>,
    regs: Vec<u16>,
}

impl SimModel for Bank {
    fn model_id(&self) -> &'static str {
        "test_bank"
    }
    fn step(&mut self, _: u64, _: &mut SplitMix64) {}
    fn image(&self, wrong_order: bool) -> RegisterImage {
        let mut b = ImageBuilder::new(wrong_order);
        for (i, c) in self.coils.iter().enumerate() {
            b.coil(i as u16, *c, true);
        }
        for (i, r) in self.regs.iter().enumerate() {
            b.u16(Table::HoldingRegister, i as u16, *r, true);
        }
        b.build()
    }
    fn supports(&self, _: u8) -> bool {
        true
    }
    fn apply(&mut self, w: &WriteRequest) -> Result<(), ExceptionCode> {
        match w {
            WriteRequest::SingleCoil { address, on } => self.coils[usize::from(*address)] = *on,
            WriteRequest::MultipleCoils { address, values } => {
                self.coils[usize::from(*address)..usize::from(*address) + values.len()].copy_from_slice(values)
            }
            WriteRequest::SingleRegister { address, value } => self.regs[usize::from(*address)] = *value,
            WriteRequest::MultipleRegisters { address, values } => {
                self.regs[usize::from(*address)..usize::from(*address) + values.len()].copy_from_slice(values)
            }
        }
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn every_write_fc_round_trips_through_the_simulator() {
    check("sim_write_round_trip", 500, |r| {
        let mut w = SimWorld::new(1);
        w.add_device(
            9,
            Box::new(Bank {
                coils: vec![false; 2048],
                regs: vec![0; 256],
            }),
        )
        .unwrap();
        let req = match r.below(4) {
            0 => WriteRequest::SingleCoil {
                address: r.range_u64(0, 2047) as u16,
                on: r.bool(),
            },
            1 => WriteRequest::SingleRegister {
                address: r.range_u64(0, 255) as u16,
                value: r.next_u16(),
            },
            2 => {
                let q = r.range_usize(1, usize::from(MAX_WRITE_COILS));
                WriteRequest::MultipleCoils {
                    address: r.range_usize(0, 2048 - q) as u16,
                    values: (0..q).map(|_| r.bool()).collect(),
                }
            }
            _ => {
                let q = r.range_usize(1, usize::from(MAX_WRITE_REGISTERS));
                WriteRequest::MultipleRegisters {
                    address: r.range_usize(0, 256 - q) as u16,
                    values: (0..q).map(|_| r.next_u16()).collect(),
                }
            }
        };
        let reply = w.handle_pdu(9, &req.encode().unwrap(), 0).pdu.unwrap();
        req.verify_echo(&reply).unwrap();
        let img = w.image(9).unwrap();
        match &req {
            WriteRequest::SingleCoil { address, on } => assert_eq!(img.coils[address], *on),
            WriteRequest::SingleRegister { address, value } => assert_eq!(img.holding[address], *value),
            WriteRequest::MultipleCoils { address, values } => {
                for (i, v) in values.iter().enumerate() {
                    assert_eq!(img.coils[&(address + i as u16)], *v);
                }
            }
            WriteRequest::MultipleRegisters { address, values } => {
                for (i, v) in values.iter().enumerate() {
                    assert_eq!(img.holding[&(address + i as u16)], *v);
                }
            }
        }
    });
}

// -------------------------------------------------------------------- historian + export with a gap

#[test]
fn simulated_session_with_a_gap_is_logged_and_exported_honestly() {
    let mut w = SimWorld::training(4);
    w.add_fault(FaultRule {
        unit: 1,
        kind: FaultKind::Timeout,
        trigger: Trigger::Window {
            from_ms: 120_000,
            until_ms: 180_000,
        },
    })
    .unwrap();
    let (mut s, mut t, clock) = setup(w.shared(), &["bgt-smps-8in1.json", "sim-energy-meter.json"], 1000);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 300_000, &mut out);

    let mut h = Historian::open_in_memory().unwrap();
    h.begin_session("demo", UTC0, "0.1.0", "simulator", 1000).unwrap();
    for (id, pid, unit) in [
        ("bgt-smps-8in1", "bgt.smps-8in1", 1),
        ("sim-energy-meter-basic", "sim.energy-meter-basic", 4),
    ] {
        h.register_device(&DeviceRow {
            session: "demo".into(),
            device_id: id.into(),
            name: id.into(),
            profile_id: pid.into(),
            profile_version: "1.0.0".into(),
            unit_id: unit,
            link: "training".into(),
        })
        .unwrap();
    }
    let mut dec = LogDecimator::new(1000);
    let mut batch = SampleBatcher::new(1000, 500);
    for smp in out.into_iter().filter(|x| dec.keep(x)) {
        let now = smp.ts_ms;
        if let Some(b) = batch.push(smp, now) {
            h.insert_samples(&b).unwrap();
        }
    }
    h.insert_samples(&batch.flush()).unwrap();
    h.rollup_until(UTC0 + 300_000).unwrap();

    let temp = h
        .samples("demo", Some("bgt-smps-8in1"), Some("temperature"), 0, i64::MAX)
        .unwrap();
    let gap: Vec<_> = temp
        .iter()
        .filter(|x| (UTC0 + 120_000..UTC0 + 180_000).contains(&x.ts_ms))
        .collect();
    assert!(
        !gap.is_empty() && gap.iter().all(|x| x.value.is_none() && !x.quality.is_good()),
        "the gap is stored as a gap"
    );
    assert!(gap.iter().any(|x| x.quality == Quality::Timeout));
    assert!(
        temp.iter()
            .filter(|x| x.ts_ms > UTC0 + 200_000)
            .all(|x| x.quality == Quality::Good)
    );
    let minute2 = h
        .rollups("demo", "bgt-smps-8in1", "temperature", UTC0 + 120_000, UTC0 + 180_000)
        .unwrap();
    assert!(minute2.is_empty(), "no rollup is made from TIMEOUT samples");
    assert_eq!(
        h.rollups("demo", "sim-energy-meter-basic", "voltage", UTC0, UTC0 + 300_000)
            .unwrap()
            .len(),
        5
    );

    let doc = ExportDoc::from_historian(&h, "demo", UTC0 + 300_000, "0.1.0").unwrap();
    let csv = export::csv::measurements(&doc);
    assert!(
        csv.lines()
            .any(|l| l.contains(",temperature,,") && l.contains(",TIMEOUT,")),
        "TIMEOUT rows with empty values"
    );
    let xlsx = export::xlsx::workbook(&doc).unwrap();
    assert!(xlsx.len() > 1000);
    let json = export::json::document(&doc);
    assert!(
        json["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["quality"] == "TIMEOUT" && m["value"].is_null())
    );
}
