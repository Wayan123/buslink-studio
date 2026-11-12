//! Block planning (P7), scheduler fairness (P8), command priority and single
//! transmission (P9), backoff, retries, STALE, link loss, worker thread, bus.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

mod common;

use buslink_core::bus::{BusRecv, SampleBus};
use buslink_core::clock::{Clock, ManualClock};
use buslink_core::codec::TypeName;
use buslink_core::command::audit::MemoryAudit;
use buslink_core::command::{
    Ack, CommandConfig, CommandLink, CommandRequest, CommandService, CommandTarget, Operator, PreparedFrame, Role,
};
use buslink_core::device::DeviceInstance;
use buslink_core::poll::plan::{self, PlanLimits};
use buslink_core::poll::scheduler::{LinkScheduler, Priority, Step, UrgentJob};
use buslink_core::poll::worker::LinkWorker;
use buslink_core::profile::{Access, Profile, Register, Text};
use buslink_core::protocol::pdu::{ReadData, ReadFunction, ReadRequest, Table};
use buslink_core::protocol::rtu;
use buslink_core::quality::{Quality, Sample};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;
use buslink_core::transport::exchange::{ExchangeError, ExchangeRecord};
use buslink_core::transport::fake::{FakeHandle, FakeTransport};
use buslink_core::transport::{LinkRegistry, Transport};
use common::slave::Bus;
use common::*;
use std::sync::{Arc, Mutex};

const SOIL: [(u16, u16); 8] = [
    (0, 253),
    (1, 312),
    (2, 480),
    (3, 260),
    (4, 35),
    (5, 12),
    (6, 80),
    (7, 680),
];

fn soil_device(
    id: &str,
    unit: u8,
    interval: u64,
    link: &buslink_core::device::LinkConfig,
) -> (DeviceInstance, Arc<Profile>) {
    let p = Arc::new(bundled("bgt-smps-8in1.json"));
    let mut d = DeviceInstance::from_profile(id, id, link, &p);
    d.unit_id = unit;
    d.poll_interval_ms = interval;
    (d, p)
}

fn register(id: &str, table: Table, address: u16, ty: TypeName, length: Option<u16>) -> Register {
    Register {
        id: id.into(),
        name: Text::Plain(id.into()),
        description: None,
        table,
        address,
        doc_address: None,
        type_name: ty,
        length,
        word_order: if matches!(ty, TypeName::U32 | TypeName::F32 | TypeName::U64 | TypeName::F64) {
            Some(buslink_core::codec::WordOrder::ABCD)
        } else {
            None
        },
        scale: None,
        offset: None,
        unit: None,
        decimals: None,
        valid_range: None,
        invalid_sentinels: None,
        bits: None,
        access: Access::Read,
        write: None,
        poll: None,
        source_page: None,
        display_range: None,
    }
}

// ---------------------------------------------------------------- planning

#[test]
fn soil_sensor_is_one_fc03_block() {
    let p = bundled("bgt-smps-8in1.json");
    let plan = plan::plan(&p, PlanLimits::default());
    assert_eq!(plan.blocks.len(), 1);
    let r = plan.blocks[0].request;
    assert_eq!(
        (r.function(), r.address(), r.quantity()),
        (ReadFunction::HoldingRegisters, 0, 8)
    );
    assert_eq!(plan.blocks[0].tags.len(), 8);
    assert!(plan.unreadable.is_empty());
}

#[test]
fn gaps_limits_and_unreadable() {
    let mut p = bundled("generic-holding.json");
    p.registers = vec![
        register("a", Table::HoldingRegister, 0, TypeName::U16, None),
        register("b", Table::HoldingRegister, 2, TypeName::U32, None),
        register("c", Table::HoldingRegister, 10, TypeName::U16, None),
        register("s", Table::HoldingRegister, 200, TypeName::String, Some(20)),
        register("x", Table::Coil, 5, TypeName::Bool, None),
        register("y", Table::Coil, 6, TypeName::Bool, None),
    ];
    let contiguous = plan::plan(&p, PlanLimits::default());
    let qty: Vec<(u16, u16)> = contiguous
        .blocks
        .iter()
        .map(|b| (b.request.address(), b.request.quantity()))
        .collect();
    assert_eq!(qty, vec![(5, 2), (0, 1), (2, 2), (10, 1), (200, 20)]);
    let gap1 = plan::plan(
        &p,
        PlanLimits {
            max_gap: 1,
            ..PlanLimits::default()
        },
    );
    assert_eq!(
        gap1.blocks
            .iter()
            .filter(|b| b.request.table() == Table::HoldingRegister)
            .count(),
        3
    );
    let small = plan::plan(
        &p,
        PlanLimits {
            max_registers: 10,
            ..PlanLimits::default()
        },
    );
    assert_eq!(
        small.unreadable,
        vec![3],
        "a 20-register string cannot be read with a 10-register limit"
    );
}

#[test]
fn plan_properties() {
    let mut base = bundled("generic-holding.json");
    check("plan_props", 1000, |r| {
        let n = r.range_usize(1, 30);
        let mut regs = Vec::new();
        for i in 0..n {
            let table = *r.pick(&Table::ALL).unwrap();
            let ty = if table.is_bit() {
                TypeName::Bool
            } else {
                *r.pick(&[TypeName::U16, TypeName::U32, TypeName::U64, TypeName::String])
                    .unwrap()
            };
            let len = (ty == TypeName::String).then(|| r.range_u64(1, 40) as u16);
            let addr = r.range_u64(0, 300) as u16;
            regs.push(register(&format!("t{i}"), table, addr, ty, len));
        }
        base.registers = regs;
        let limits = PlanLimits {
            max_registers: r.range_u64(4, 125) as u16,
            max_bits: r.range_u64(1, 2000) as u16,
            max_gap: if r.chance(300) { r.range_u64(1, 5) as u16 } else { 0 },
        };
        let plan = plan::plan(&base, limits);
        let mut seen = vec![0u32; base.registers.len()];
        for b in &plan.blocks {
            let q = b.request.quantity();
            let lim = if b.request.table().is_bit() {
                limits.max_bits
            } else {
                limits.max_registers
            };
            assert!(q >= 1 && q <= lim, "block within limits");
            for t in &b.tags {
                seen[t.register] += 1;
                let reg = &base.registers[t.register];
                assert_eq!(reg.table, b.request.table());
                assert!(reg.address >= b.request.address());
                assert!(
                    u32::from(reg.address) + u32::from(reg.width()) <= u32::from(b.request.address()) + u32::from(q),
                    "tag never split"
                );
                assert_eq!(t.offset, reg.address - b.request.address());
            }
            if limits.max_gap == 0 {
                for a in b.request.address()..=b.request.end_address() {
                    assert!(
                        b.tags.iter().any(|t| {
                            let reg = &base.registers[t.register];
                            u32::from(a) >= u32::from(reg.address) && u32::from(a) <= reg.end_address()
                        }),
                        "address {a} read but covered by no tag"
                    );
                }
            }
        }
        for u in &plan.unreadable {
            seen[*u] += 1;
        }
        assert!(seen.iter().all(|&c| c == 1), "every tag planned exactly once: {seen:?}");
    });
}

// ---------------------------------------------------------------- scheduling

fn sched_with(
    bus: &Bus,
    devices: Vec<(DeviceInstance, Arc<Profile>)>,
) -> (LinkScheduler, FakeTransport, FakeHandle, Arc<ManualClock>) {
    let clock = ManualClock::shared(1_800_000_000_000);
    let (t, h) = FakeTransport::new(clock.clone(), bus.responder());
    let s = LinkScheduler::new(rtu_link("bus1"), devices, clock.clone() as Arc<dyn Clock>).unwrap();
    (s, t, h, clock)
}

fn good_times(samples: &[Sample], device: &str) -> Vec<i64> {
    samples
        .iter()
        .filter(|s| s.device_id == device && s.tag_id == "temperature" && s.quality == Quality::Good)
        .map(|s| s.ts_ms)
        .collect()
}

fn writes_to(h: &FakeHandle, unit: u8) -> Vec<u64> {
    h.writes()
        .into_iter()
        .filter(|(_, w)| w[0] == unit)
        .map(|(t, _)| t)
        .collect()
}

#[test]
fn healthy_device_good_samples_and_counters() {
    let bus = Bus::new();
    bus.add(1, &SOIL);
    let link = rtu_link("bus1");
    let (mut s, mut t, _h, clock) = sched_with(&bus, vec![soil_device("soil", 1, 1000, &link)]);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 10_500, &mut out);
    let temps: Vec<&Sample> = out.iter().filter(|x| x.tag_id == "temperature").collect();
    assert_eq!(temps.len(), 11);
    assert!(temps.iter().all(|x| x.quality == Quality::Good));
    assert_eq!(temps[0].value.as_ref().unwrap().engineering, Some(25.3));
    let ph = out.iter().find(|x| x.tag_id == "ph").unwrap();
    assert_eq!(ph.value.as_ref().unwrap().engineering, Some(6.8));
    assert_eq!(ph.profile_version, "1.0.0");
    let (_, c) = &s.counters()[0];
    assert_eq!((c.requests, c.good, c.timeouts), (11, 11, 0));
    assert_eq!(c.cycle_ms, Some(1000));
}

#[test]
fn timeout_keeps_last_good_then_backs_off_and_recovers() {
    let bus = Bus::new();
    bus.add(1, &SOIL);
    let link = rtu_link("bus1");
    let (mut d, p) = soil_device("soil", 1, 1000, &link);
    d.read_retries = 1;
    let (mut s, mut t, h, clock) = sched_with(&bus, vec![(d, p)]);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 500, &mut out);
    let first = out.iter().find(|x| x.tag_id == "temperature").unwrap().clone();
    bus.set_alive(1, false);
    run_until(&mut s, &mut t, &clock, 70_000, &mut out);
    let fails: Vec<&Sample> = out
        .iter()
        .filter(|x| x.tag_id == "temperature" && x.quality == Quality::Timeout)
        .collect();
    assert!(!fails.is_empty());
    for f in &fails {
        assert_eq!(f.value_ts_ms, Some(first.ts_ms), "last good value keeps its timestamp");
        assert_eq!(f.value.as_ref().unwrap().engineering, Some(25.3));
        assert!(f.age_ms.unwrap() > 0);
    }
    // Requests: first failure retried once, then one attempt per backoff period.
    let w = writes_to(&h, 1);
    let gaps: Vec<u64> = w.windows(2).map(|p| p[1] - p[0]).collect();
    assert!(gaps.len() >= 6, "{gaps:?}");
    // gaps[0]: good→first try; gaps[1]: retry (immediate); then growing backoff.
    let backoffs: Vec<u64> = gaps[2..].to_vec();
    for pair in backoffs.windows(2) {
        assert!(
            pair[1] >= pair[0] || pair[1] >= 30_000,
            "backoff never shrinks while dead: {backoffs:?}"
        );
    }
    assert!(backoffs.iter().all(|g| *g <= 30_000 + 2_000), "cap 30 s: {backoffs:?}");
    let c = &s.counters()[0].1;
    assert!(c.consecutive_failures >= 5 && c.backoff_until_mono_ms.is_some());

    bus.set_alive(1, true);
    run_until(&mut s, &mut t, &clock, 110_000, &mut out);
    let c = &s.counters()[0].1;
    assert_eq!(c.consecutive_failures, 0, "a GOOD cycle resets the backoff");
    let recovered = good_times(&out, "soil");
    let tail: Vec<i64> = recovered.windows(2).map(|p| p[1] - p[0]).rev().take(3).collect();
    assert!(tail.iter().all(|g| *g == 1000), "back to the poll interval: {tail:?}");
}

#[test]
fn exception_on_one_block_does_not_stop_the_cycle() {
    let bus = Bus::new();
    bus.add(1, &SOIL);
    let link = rtu_link("bus1");
    let (d, p) = soil_device("soil", 1, 1000, &link);
    let mut p2 = (*p).clone();
    p2.registers
        .push(register("missing", Table::HoldingRegister, 100, TypeName::U16, None));
    let (mut s, mut t, _h, clock) = sched_with(&bus, vec![(d, Arc::new(p2))]);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 2_500, &mut out);
    assert!(
        out.iter()
            .any(|x| x.tag_id == "missing" && x.quality == Quality::Exception(2))
    );
    assert_eq!(good_times(&out, "soil").len(), 3);
    assert_eq!(
        s.counters()[0].1.consecutive_failures,
        0,
        "exceptions do not back off a live device"
    );
}

#[test]
fn stale_is_emitted_when_no_new_data_arrives() {
    let bus = Bus::new();
    bus.add(1, &SOIL);
    let link = rtu_link("bus1");
    let (mut d, p) = soil_device("soil", 1, 20_000, &link);
    d.stale_after_ms = Some(3_000);
    let (mut s, mut t, _h, clock) = sched_with(&bus, vec![(d, p)]);
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 10_000, &mut out);
    let temps: Vec<&Sample> = out.iter().filter(|x| x.tag_id == "temperature").collect();
    assert_eq!(
        temps.iter().map(|x| x.quality).collect::<Vec<_>>(),
        vec![Quality::Good, Quality::Stale]
    );
    assert!(temps[1].age_ms.unwrap() > 3_000);
    assert_eq!(temps[1].value_ts_ms, temps[0].value_ts_ms);
}

#[test]
fn link_loss_marks_everything_disconnected_and_fails_queued_jobs() {
    let bus = Bus::new();
    bus.add(1, &SOIL);
    bus.add(2, &SOIL);
    let link = rtu_link("bus1");
    let (mut s, mut t, h, clock) = sched_with(
        &bus,
        vec![soil_device("a", 1, 1000, &link), soil_device("b", 2, 1000, &link)],
    );
    let mut out = Vec::new();
    run_until(&mut s, &mut t, &clock, 1_500, &mut out);
    h.disconnect();
    let result: Arc<Mutex<Option<Result<ReadData, ExchangeError>>>> = Arc::default();
    let r2 = result.clone();
    s.submit(UrgentJob::Read {
        unit: 1,
        request: ReadRequest::new(ReadFunction::HoldingRegisters, 0, 1).unwrap(),
        priority: Priority::Alarm,
        timeout_ms: 500,
        reply: Box::new(move |r| *r2.lock().unwrap() = Some(r)),
    });
    // The alarm read runs first and hits the dead link.
    let step = run_until(&mut s, &mut t, &clock, 5_000, &mut out);
    assert_eq!(step, Step::LinkDown);
    assert!(matches!(
        result.lock().unwrap().as_ref().unwrap(),
        Err(ExchangeError::Link(_))
    ));
    let disc = out.iter().filter(|x| x.quality == Quality::Disconnected).count();
    assert_eq!(disc, 16, "every tag of both devices");
    // A job submitted after the link went down fails immediately and is never replayed.
    let late: Arc<Mutex<Option<Result<ReadData, ExchangeError>>>> = Arc::default();
    let l2 = late.clone();
    s.submit(UrgentJob::Read {
        unit: 2,
        request: ReadRequest::new(ReadFunction::HoldingRegisters, 0, 1).unwrap(),
        priority: Priority::Command,
        timeout_ms: 500,
        reply: Box::new(move |r| *l2.lock().unwrap() = Some(r)),
    });
    assert!(late.lock().unwrap().is_some());
    assert_eq!(s.step(&mut t, &mut |_| {}), Step::LinkDown);
}

/// P8: with dead devices on the link, every healthy device keeps its poll interval within the bound.
#[test]
fn no_starvation_with_dead_devices() {
    check("no_starvation", 300, |r| {
        let n = r.range_usize(2, 5);
        let bus = Bus::new();
        let link = rtu_link("bus1");
        let mut devices = Vec::new();
        let mut alive = Vec::new();
        for i in 0..n {
            let unit = (i + 1) as u8;
            bus.add(unit, &SOIL);
            let interval = *r.pick(&[200u64, 500, 1000, 2000]).unwrap();
            let (mut d, p) = soil_device(&format!("d{unit}"), unit, interval, &link);
            d.response_timeout_ms = Some(r.range_u64(100, 1000));
            d.read_retries = r.range_u64(0, 2) as u8;
            alive.push(i == 0 || r.chance(500));
            devices.push((d, p));
        }
        if alive.iter().all(|a| *a) {
            alive[n - 1] = false;
        }
        for (i, a) in alive.iter().enumerate() {
            bus.set_alive((i + 1) as u8, *a);
        }
        let worst_other = |me: usize| -> u64 {
            devices
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != me)
                .map(|(_, (d, _))| (u64::from(d.read_retries) + 1) * (d.response_timeout_ms.unwrap() + 210))
                .sum()
        };
        let bounds: Vec<u64> = (0..n)
            .map(|i| devices[i].0.poll_interval_ms + worst_other(i) + 250)
            .collect();
        let ids: Vec<String> = devices.iter().map(|(d, _)| d.id.clone()).collect();
        let (mut s, mut t, _h, clock) = sched_with(&bus, devices);
        let mut out = Vec::new();
        run_until(&mut s, &mut t, &clock, 60_000, &mut out);
        for i in (0..n).filter(|i| alive[*i]) {
            let times = good_times(&out, &ids[i]);
            assert!(times.len() >= 10, "{} got {} GOOD samples", ids[i], times.len());
            for g in times.windows(2).map(|p| (p[1] - p[0]) as u64) {
                assert!(g <= bounds[i], "{}: gap {g} ms > bound {} ms", ids[i], bounds[i]);
            }
        }
        for i in (0..n).filter(|i| !alive[*i]) {
            assert!(good_times(&out, &ids[i]).is_empty());
            assert!(s.counters()[i].1.consecutive_failures >= 2, "dead devices back off");
        }
    });
}

// ---------------------------------------------------------------- commands through the scheduler

/// CommandLink that queues jobs on a scheduler and steps it, like the worker does.
struct QueueLink<'a> {
    sched: &'a mut LinkScheduler,
    t: &'a mut dyn Transport,
    out: &'a mut Vec<Sample>,
}

impl CommandLink for QueueLink<'_> {
    fn read(&mut self, unit: u8, request: ReadRequest) -> Result<ReadData, ExchangeError> {
        let slot: Arc<Mutex<Option<Result<ReadData, ExchangeError>>>> = Arc::default();
        let s2 = slot.clone();
        self.sched.submit(UrgentJob::Read {
            unit,
            request,
            priority: Priority::Command,
            timeout_ms: 500,
            reply: Box::new(move |r| *s2.lock().unwrap() = Some(r)),
        });
        let out = &mut *self.out;
        self.sched.step(self.t, &mut |s| out.push(s));
        slot.lock().unwrap().take().unwrap()
    }

    fn send(&mut self, frame: PreparedFrame) -> ExchangeRecord {
        let slot: Arc<Mutex<Option<ExchangeRecord>>> = Arc::default();
        let s2 = slot.clone();
        self.sched.submit(UrgentJob::Command {
            frame,
            timeout_ms: 500,
            reply: Box::new(move |r| *s2.lock().unwrap() = Some(r)),
        });
        let out = &mut *self.out;
        self.sched.step(self.t, &mut |s| out.push(s));
        slot.lock().unwrap().take().unwrap()
    }
}

fn vfd_regs() -> Vec<(u16, u16)> {
    (0..7)
        .map(|a| (a, 0))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(a, v)| if a == 2 { (a, 50) } else { (a, v) })
        .collect()
}

fn vfd_target(link: &buslink_core::device::LinkConfig, unit: u8) -> CommandTarget {
    let p = Arc::new(bundled("sim-vfd.json"));
    let mut d = DeviceInstance::from_profile("vfd", "VFD", link, &p);
    d.unit_id = unit;
    CommandTarget {
        device: d,
        link: link.clone(),
        profile: p,
    }
}

/// P9: commands run before alarm reads and polls, and each confirmed frame is sent exactly once.
#[test]
fn command_frames_jump_the_queue_and_are_sent_once() {
    check("command_priority_once", 200, |r: &mut SplitMix64| {
        let clock = ManualClock::shared(0);
        let bus = Bus::new();
        bus.add(1, &SOIL);
        bus.add(2, &vfd_regs());
        let silent = r.bool();
        bus.with(2, |u| u.silent_writes = silent);
        let link = sim_link("simbus");
        let target = vfd_target(&link, 2);
        let (t, h) = FakeTransport::new(clock.clone(), bus.responder());
        let mut t: Box<dyn Transport> = Box::new(t);
        let mut s = LinkScheduler::new(
            link.clone(),
            vec![
                soil_device("soil", 1, 200, &link),
                (target.device.clone(), target.profile.clone()),
            ],
            clock.clone() as Arc<dyn Clock>,
        )
        .unwrap();
        let mut out = Vec::new();
        run_until(&mut s, t.as_mut(), &clock, r.range_u64(0, 3000), &mut out);
        // An alarm read is waiting when the command arrives.
        let alarm_done: Arc<Mutex<bool>> = Arc::default();
        let a2 = alarm_done.clone();
        s.submit(UrgentJob::Read {
            unit: 1,
            request: ReadRequest::new(ReadFunction::HoldingRegisters, 0, 1).unwrap(),
            priority: Priority::Alarm,
            timeout_ms: 500,
            reply: Box::new(move |_| *a2.lock().unwrap() = true),
        });
        let audit = MemoryAudit::new();
        let mut svc = CommandService::new(
            clock.clone() as Arc<dyn Clock>,
            Box::new(audit.clone()),
            "s1",
            CommandConfig::default(),
        );
        svc.arm(
            target,
            Operator {
                user: "tech".into(),
                role: Role::Operator,
            },
        )
        .unwrap();
        let hz = (r.range_u64(0, 5000) as f64) / 100.0;
        let before = h.write_count();
        let mut link_q = QueueLink {
            sched: &mut s,
            t: t.as_mut(),
            out: &mut out,
        };
        let p = svc
            .preview(
                &mut link_q,
                CommandRequest::SetRegister {
                    register: "freq_setpoint".into(),
                    value: hz,
                },
            )
            .unwrap();
        assert!(
            !*alarm_done.lock().unwrap(),
            "the command-priority read ran before the alarm read"
        );
        svc.confirm(p.id).unwrap();
        let outcome = svc.execute(&mut link_q).unwrap();
        let writes = h.writes();
        let new: Vec<&Vec<u8>> = writes[before..].iter().map(|(_, w)| w).collect();
        let fc06: Vec<&&Vec<u8>> = new.iter().filter(|w| w[1] == 0x06).collect();
        assert_eq!(fc06.len(), 1, "exactly one write frame, even when the device is silent");
        assert_eq!(new.len(), 3, "old-value read, write, read-back; no polls in between");
        assert!(new.iter().all(|w| w[0] == 2));
        if silent {
            assert_eq!(outcome.ack, Ack::Timeout);
        } else {
            assert_eq!(outcome.ack, Ack::Acknowledged);
        }
        assert!(svc.execute(&mut link_q).is_err(), "a confirmation is used once");
        assert_eq!(h.writes().iter().filter(|(_, w)| w[1] == 0x06).count(), 1);
        run_until(&mut s, t.as_mut(), &clock, clock.mono_ms() + 1000, &mut out);
        assert!(*alarm_done.lock().unwrap());
        assert_eq!(
            h.writes().iter().filter(|(_, w)| w[1] == 0x06).count(),
            1,
            "never retried later"
        );
    });
}

#[test]
fn queued_command_on_a_dead_link_is_not_sent() {
    let clock = ManualClock::shared(0);
    let bus = Bus::new();
    bus.add(2, &vfd_regs());
    let link = sim_link("simbus");
    let target = vfd_target(&link, 2);
    let (t, h) = FakeTransport::new(clock.clone(), bus.responder());
    let mut t: Box<dyn Transport> = Box::new(t);
    let mut s = LinkScheduler::new(
        link.clone(),
        vec![(target.device.clone(), target.profile.clone())],
        clock.clone() as Arc<dyn Clock>,
    )
    .unwrap();
    let mut out = Vec::new();
    let audit = MemoryAudit::new();
    let mut svc = CommandService::new(
        clock.clone() as Arc<dyn Clock>,
        Box::new(audit),
        "s1",
        CommandConfig::default(),
    );
    svc.arm(
        target,
        Operator {
            user: "tech".into(),
            role: Role::Operator,
        },
    )
    .unwrap();
    let mut q = QueueLink {
        sched: &mut s,
        t: t.as_mut(),
        out: &mut out,
    };
    let p = svc
        .preview(
            &mut q,
            CommandRequest::SetRegister {
                register: "freq_setpoint".into(),
                value: 30.0,
            },
        )
        .unwrap();
    svc.confirm(p.id).unwrap();
    h.disconnect();
    // Bring the link down with a poll first.
    run_until(&mut s, t.as_mut(), &clock, 5000, &mut out);
    assert!(s.is_down());
    let mut q = QueueLink {
        sched: &mut s,
        t: t.as_mut(),
        out: &mut out,
    };
    let o = svc.execute(&mut q).unwrap();
    assert_eq!(o.ack, Ack::NotSent);
    assert_eq!(svc.sends(), 0);
    assert!(svc.armed_device().is_none(), "link loss disarms");
    assert!(h.writes().iter().all(|(_, w)| w[1] != 0x06));
}

// ---------------------------------------------------------------- worker thread and bus

#[test]
fn worker_thread_end_to_end_and_single_owner() {
    let clock = ManualClock::shared(0);
    let bus_dev = Bus::new();
    bus_dev.add(1, &SOIL);
    let (t, _h) = FakeTransport::new(clock.clone(), bus_dev.responder());
    let link = rtu_link("bus1");
    let registry = LinkRegistry::new();
    let claim = registry.claim(&link.owner_key()).unwrap();
    assert!(registry.claim(&link.owner_key()).is_err(), "one master per bus");
    let sched = LinkScheduler::new(
        link.clone(),
        vec![soil_device("soil", 1, 1000, &link)],
        clock.clone() as Arc<dyn Clock>,
    )
    .unwrap();
    let samples = SampleBus::new(1024);
    let mut sub = samples.subscribe();
    let worker = LinkWorker::spawn(claim, sched, Box::new(t), samples.sink()).unwrap();
    // Drive manual time from a helper thread while the worker idles.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (c2, s2) = (clock.clone(), stop.clone());
    let ticker = std::thread::spawn(move || {
        while !s2.load(std::sync::atomic::Ordering::SeqCst) {
            c2.advance(100);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let handle = worker.handle(500);
    let data = handle
        .read(
            1,
            ReadRequest::new(ReadFunction::HoldingRegisters, 7, 1).unwrap(),
            Priority::Alarm,
        )
        .unwrap();
    assert_eq!(data, ReadData::Registers(vec![680]));
    let mut good = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while good < 16 && std::time::Instant::now() < deadline {
        match sub.try_recv() {
            BusRecv::Sample(s) if s.quality == Quality::Good => good += 1,
            BusRecv::Empty => std::thread::sleep(std::time::Duration::from_millis(1)),
            _ => {}
        }
    }
    assert!(good >= 16, "samples reach the bus");
    let status = worker.stop();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    ticker.join().unwrap();
    assert!(status.counters[0].1.good >= 2);
    assert!(
        !registry.is_owned(&link.owner_key()),
        "claim released when the worker stops"
    );
    let err = handle.read(
        1,
        ReadRequest::new(ReadFunction::HoldingRegisters, 0, 1).unwrap(),
        Priority::Command,
    );
    assert!(matches!(err, Err(ExchangeError::Link(_))));
}

#[test]
fn bus_reports_lag_instead_of_dropping_silently() {
    let bus = SampleBus::new(4);
    let mut sub = bus.subscribe();
    let link = rtu_link("b");
    let (d, p) = soil_device("soil", 1, 1000, &link);
    let _ = (d, p);
    let mk = |seq: u64| Sample {
        seq,
        link_id: "b".into(),
        device_id: "soil".into(),
        tag_id: "t".into(),
        profile_id: "p".into(),
        profile_version: "1.0.0".into(),
        ts_ms: 0,
        quality: Quality::Good,
        value: None,
        value_ts_ms: None,
        age_ms: None,
        ts_source_ms: None,
        raw: None,
        unit: None,
        error: None,
    };
    for i in 0..10 {
        bus.publish(mk(i));
    }
    let got = sub.drain();
    assert_eq!(got[0], BusRecv::Lagged(6));
    assert_eq!(got.len(), 5);
    let _ = rtu::MAX_ADU_LEN;
}
