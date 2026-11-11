//! Command service (Req 16) and property P10: no send without the current
//! confirmation, at most one send per confirmation, never unit 0 or an
//! out-of-bounds value, and every send preceded by a `sent` audit record.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

mod common;

use buslink_core::clock::{Clock, ManualClock};
use buslink_core::command::audit::{AuditEvent, MemoryAudit};
use buslink_core::command::wire::WriteRequest;
use buslink_core::command::{
    Ack, CommandConfig, CommandError, CommandRequest, CommandService, CommandState, CommandTarget, DirectLink,
    DisarmReason, Operator, Readback, Role,
};
use buslink_core::device::{DeviceInstance, LinkConfig};
use buslink_core::protocol::rtu;
use buslink_core::testkit::check;
use buslink_core::transport::exchange::{Exchanger, Framing};
use buslink_core::transport::fake::{FakeHandle, FakeTransport};
use common::slave::Bus;
use common::*;
use std::sync::Arc;

struct World {
    clock: Arc<ManualClock>,
    bus: Bus,
    t: FakeTransport,
    h: FakeHandle,
    ex: Exchanger,
}

impl World {
    fn new() -> Self {
        let clock = ManualClock::shared(1_800_000_000_000);
        let bus = Bus::new();
        // VFD at unit 2: control 0, setpoint 0, ramp 5.0 s, rest 0. Temp controller at unit 3.
        bus.add(2, &[(0, 0), (1, 0), (2, 50), (3, 0), (4, 0), (5, 0), (6, 0)]);
        bus.add(3, &[(0, 250), (1, 400), (2, 0), (3, 0), (4, 0), (5, 0)]);
        let (t, h) = FakeTransport::new(clock.clone(), bus.responder());
        let ex = Exchanger::new(Framing::Rtu, clock.clone() as Arc<dyn Clock>, 5);
        Self { clock, bus, t, h, ex }
    }

    fn link(&mut self) -> DirectLink<'_> {
        DirectLink {
            transport: &mut self.t,
            exchanger: &mut self.ex,
            timeout_ms: 500,
        }
    }

    fn write_frames(&self) -> Vec<Vec<u8>> {
        self.h
            .writes()
            .into_iter()
            .map(|(_, w)| w)
            .filter(|w| matches!(w[1], 0x05 | 0x06 | 0x0F | 0x10))
            .collect()
    }
}

fn target(file: &str, link: &LinkConfig, unit: u8) -> CommandTarget {
    let p = Arc::new(bundled(file));
    let mut d = DeviceInstance::from_profile("dev", "Device", link, &p);
    d.unit_id = unit;
    CommandTarget {
        device: d,
        link: link.clone(),
        profile: p,
    }
}

fn op(role: Role) -> Operator {
    Operator {
        user: "tech1".into(),
        role,
    }
}

fn service(w: &World, audit: &MemoryAudit) -> CommandService {
    CommandService::new(
        w.clock.clone() as Arc<dyn Clock>,
        Box::new(audit.clone()),
        "session-1",
        CommandConfig::default(),
    )
}

fn set(reg: &str, v: f64) -> CommandRequest {
    CommandRequest::SetRegister {
        register: reg.into(),
        value: v,
    }
}

#[test]
fn happy_path_preview_confirm_execute_readback_audit() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    assert_eq!(svc.state(), CommandState::Armed);
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 30.0)).unwrap();
    assert_eq!(
        (p.unit_id, p.address, p.function, p.old_value, p.new_value),
        (2, 1, 0x06, Some(0.0), 30.0)
    );
    assert_eq!(p.raw_registers, vec![3000]);
    assert_eq!(p.pdu_hex, "06 00 01 0B B8");
    let expected_frame = rtu::encode_adu(2, &[0x06, 0x00, 0x01, 0x0B, 0xB8]).unwrap();
    assert_eq!(p.frame_hex, buslink_core::testkit::hex(&expected_frame));
    assert!(p.simulated);
    assert!(w.write_frames().is_empty(), "preview never writes");
    svc.confirm(p.id).unwrap();
    assert!(w.write_frames().is_empty(), "confirm never writes");
    let o = svc.execute(&mut w.link()).unwrap();
    assert_eq!(
        (o.ack.clone(), o.readback, o.readback_value),
        (Ack::Acknowledged, Readback::Verified, Some(30.0))
    );
    assert_eq!(svc.state(), CommandState::Done);
    assert_eq!(w.write_frames(), vec![expected_frame]);
    assert_eq!(w.bus.with(2, |u| u.holding[&1]), 3000);
    let events: Vec<AuditEvent> = audit.records().iter().map(|r| r.event.unwrap()).collect();
    assert_eq!(
        events,
        vec![
            AuditEvent::Arm,
            AuditEvent::Preview,
            AuditEvent::Confirm,
            AuditEvent::Sent,
            AuditEvent::Result
        ]
    );
    let last = audit.records().last().unwrap().clone();
    assert_eq!(last.session_id, "session-1");
    assert_eq!((last.user.as_str(), last.role.as_str()), ("tech1", "operator"));
    assert_eq!(
        (last.profile_id.as_str(), last.profile_version.as_str()),
        ("sim.vfd-basic", "1.0.0")
    );
    assert_eq!(
        (last.function, last.address, last.old_value, last.requested_value),
        (Some(6), Some(1), Some(0.0), Some(30.0))
    );
    assert!(last.request_hex.is_some() && last.response_hex.as_deref().is_some_and(|s| !s.is_empty()));
    assert_eq!(last.readback.as_deref(), Some("verified"));
}

#[test]
fn bounds_are_rejected_never_clamped() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    for (reg, v) in [
        ("freq_setpoint", 50.01),
        ("freq_setpoint", -0.01),
        ("freq_setpoint", 30.005),
        ("control_word", 2.0),
        ("freq_setpoint", f64::NAN),
    ] {
        let e = svc.preview(&mut w.link(), set(reg, v)).unwrap_err();
        assert!(matches!(e, CommandError::OutOfBounds(_)), "{reg}={v}: {e:?}");
    }
    assert!(matches!(
        svc.preview(&mut w.link(), set("output_freq", 1.0)),
        Err(CommandError::NotWritable(_))
    ));
    assert!(matches!(
        svc.preview(&mut w.link(), set("nope", 1.0)),
        Err(CommandError::UnknownRegister(_))
    ));
    assert!(w.write_frames().is_empty());
    assert!(audit.records().iter().any(|r| r.event == Some(AuditEvent::Rejected)));
    // A valid value still works afterwards.
    svc.preview(&mut w.link(), set("freq_setpoint", 50.0)).unwrap();
}

#[test]
fn role_and_arming_rules() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    let t = target("sim-vfd.json", &sim_link("sim"), 2);
    assert!(matches!(
        svc.arm(t.clone(), op(Role::Viewer)),
        Err(CommandError::Role { .. })
    ));
    assert!(matches!(
        svc.preview(&mut w.link(), set("freq_setpoint", 1.0)),
        Err(CommandError::NotArmed)
    ));
    svc.arm(t.clone(), op(Role::Operator)).unwrap();
    // ramp_time needs the commissioner role.
    assert!(matches!(
        svc.preview(&mut w.link(), set("ramp_time", 10.0)),
        Err(CommandError::Role {
            need: "commissioner",
            ..
        })
    ));
    svc.arm(t.clone(), op(Role::Commissioner)).unwrap();
    svc.preview(&mut w.link(), set("ramp_time", 10.0)).unwrap();
    // Expiry after 60 s idle, measured from the last action.
    w.clock.advance(59_000);
    let p = svc.preview(&mut w.link(), set("ramp_time", 12.0)).unwrap();
    w.clock.advance(59_000);
    svc.confirm(p.id).unwrap();
    w.clock.advance(60_001);
    assert_eq!(svc.execute(&mut w.link()), Err(CommandError::ArmExpired));
    assert_eq!(svc.state(), CommandState::Idle);
    assert!(w.write_frames().is_empty());
    for reason in [
        DisarmReason::Disconnect,
        DisarmReason::Reconnect,
        DisarmReason::ProfileChange,
        DisarmReason::AppBackground,
    ] {
        svc.arm(t.clone(), op(Role::Operator)).unwrap();
        let p = svc.preview(&mut w.link(), set("freq_setpoint", 5.0)).unwrap();
        svc.confirm(p.id).unwrap();
        svc.disarm(reason);
        assert_eq!(
            svc.execute(&mut w.link()).unwrap_err(),
            CommandError::WrongState {
                expected: "confirmed",
                actual: CommandState::Idle
            }
        );
    }
    assert!(w.write_frames().is_empty());
    let notes: Vec<String> = audit
        .records()
        .iter()
        .filter(|r| r.event == Some(AuditEvent::Disarm))
        .filter_map(|r| r.note.clone())
        .collect();
    for n in ["disconnect", "reconnect", "profile_change", "app_background"] {
        assert!(notes.iter().any(|x| x == n), "{n} audited");
    }
}

#[test]
fn unit_zero_is_never_written() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    let t = target("sim-vfd.json", &sim_link("sim"), 0);
    assert!(matches!(svc.arm(t, op(Role::Operator)), Err(CommandError::Target(_))));
    assert!(matches!(
        svc.preview(&mut w.link(), set("freq_setpoint", 1.0)),
        Err(CommandError::NotArmed)
    ));
    assert!(w.h.writes().is_empty());
}

#[test]
fn simulated_profiles_only_on_simulator_links() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &rtu_link("bus1"), 2), op(Role::Operator))
        .unwrap();
    assert!(matches!(
        svc.preview(&mut w.link(), set("freq_setpoint", 1.0)),
        Err(CommandError::ControlNotAllowed(_))
    ));
    let named = CommandRequest::Named {
        command: "run".into(),
        value: None,
    };
    assert!(matches!(
        svc.preview(&mut w.link(), named),
        Err(CommandError::ControlNotAllowed(_))
    ));
    // The loopback training TCP server counts as a simulator link.
    svc.arm(
        target("sim-vfd.json", &tcp_link("t", "127.0.0.1"), 2),
        op(Role::Operator),
    )
    .unwrap();
    // A read-only real profile has no write support at all.
    svc.arm(target("bgt-smps-8in1.json", &rtu_link("bus1"), 1), op(Role::Operator))
        .unwrap();
    assert!(matches!(
        svc.preview(&mut w.link(), set("ph", 7.0)),
        Err(CommandError::ControlNotAllowed(_))
    ));
    assert!(w.write_frames().is_empty());
}

#[test]
fn named_commands_and_fixed_values() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    let p = svc
        .preview(
            &mut w.link(),
            CommandRequest::Named {
                command: "run".into(),
                value: None,
            },
        )
        .unwrap();
    assert_eq!(
        (p.raw_registers.clone(), p.command_id.as_deref()),
        (vec![1], Some("run"))
    );
    assert!(matches!(
        svc.preview(
            &mut w.link(),
            CommandRequest::Named {
                command: "run".into(),
                value: Some(4.0)
            }
        ),
        Err(CommandError::OutOfBounds(_))
    ));
    assert!(matches!(
        svc.preview(
            &mut w.link(),
            CommandRequest::Named {
                command: "set_frequency".into(),
                value: None
            }
        ),
        Err(CommandError::OutOfBounds(_))
    ));
    let p = svc
        .preview(
            &mut w.link(),
            CommandRequest::Named {
                command: "set_frequency".into(),
                value: Some(40.0),
            },
        )
        .unwrap();
    svc.confirm(p.id).unwrap();
    assert_eq!(svc.execute(&mut w.link()).unwrap().readback, Readback::Verified);
    // Stop is labelled as a software stop in the profile.
    let stop = bundled("sim-vfd.json")
        .command("stop")
        .unwrap()
        .label
        .get("en")
        .to_owned();
    assert_eq!(stop, "Software stop. Not an emergency stop.");
}

#[test]
fn timeout_means_effect_unknown_read_back_and_no_retry() {
    let mut w = World::new();
    w.bus.with(2, |u| u.silent_writes = true);
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 25.0)).unwrap();
    svc.confirm(p.id).unwrap();
    let o = svc.execute(&mut w.link()).unwrap();
    assert_eq!(o.ack, Ack::Timeout);
    // The device applied it silently; the read-back shows the real state.
    assert_eq!((o.readback, o.readback_value), (Readback::Verified, Some(25.0)));
    assert_eq!(svc.state(), CommandState::Failed);
    assert_eq!(w.write_frames().len(), 1, "never retried");
    assert_eq!(w.bus.with(2, |u| u.writes_seen), 1);
    assert!(svc.execute(&mut w.link()).is_err());
    assert_eq!(w.write_frames().len(), 1);
    let rec = audit.records().last().unwrap().clone();
    assert!(rec.ack.unwrap().contains("effect unknown"));
}

#[test]
fn exception_and_mismatch_are_reported_separately() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    w.bus.with(2, |u| u.write_exception = Some(3));
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 10.0)).unwrap();
    svc.confirm(p.id).unwrap();
    let o = svc.execute(&mut w.link()).unwrap();
    assert_eq!((o.ack, o.readback), (Ack::Exception(3), Readback::Mismatch));
    assert_eq!(audit.records().last().unwrap().exception, Some(3));
    w.bus.with(2, |u| {
        u.write_exception = None;
        u.sticky = true;
    });
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 10.0)).unwrap();
    svc.confirm(p.id).unwrap();
    let o = svc.execute(&mut w.link()).unwrap();
    assert_eq!(o.ack, Ack::Acknowledged, "acknowledged");
    assert_eq!(o.readback, Readback::Mismatch, "but the value did not change");
}

#[test]
fn audit_failure_blocks_the_send() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 10.0)).unwrap();
    svc.confirm(p.id).unwrap();
    audit.set_failing(true);
    assert!(matches!(svc.execute(&mut w.link()), Err(CommandError::Audit(_))));
    assert_eq!(svc.state(), CommandState::Failed);
    assert!(w.write_frames().is_empty(), "no audit record, no write");
    audit.set_failing(false);
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 10.0)).unwrap();
    audit.set_failing(true);
    assert!(svc.confirm(p.id).is_err());
    audit.set_failing(false);
    assert!(svc.execute(&mut w.link()).is_err(), "a failed confirm does not confirm");
    assert!(w.write_frames().is_empty());
}

#[test]
fn confirmation_must_match_the_current_preview() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(
        target("sim-temp-controller.json", &sim_link("sim"), 3),
        op(Role::Operator),
    )
    .unwrap();
    let old = svc.preview(&mut w.link(), set("sp", 40.0)).unwrap();
    let new = svc.preview(&mut w.link(), set("sp", 45.0)).unwrap();
    assert_eq!(svc.confirm(old.id), Err(CommandError::PreviewMismatch));
    assert!(svc.execute(&mut w.link()).is_err());
    svc.confirm(new.id).unwrap();
    let o = svc.execute(&mut w.link()).unwrap();
    assert_eq!((o.readback, o.readback_value), (Readback::Verified, Some(45.0)));
    assert_eq!(w.bus.with(3, |u| u.holding[&1]), 450);
    assert_eq!(w.write_frames().len(), 1);
}

#[test]
fn preview_shows_unknown_old_value_instead_of_zero() {
    let mut w = World::new();
    let audit = MemoryAudit::new();
    let mut svc = service(&w, &audit);
    svc.arm(target("sim-vfd.json", &sim_link("sim"), 2), op(Role::Operator))
        .unwrap();
    w.bus.set_alive(2, false);
    let p = svc.preview(&mut w.link(), set("freq_setpoint", 10.0)).unwrap();
    assert_eq!(p.old_value, None);
    assert!(p.old_error.is_some());
}

/// P10: random operation sequences.
#[test]
fn state_machine_never_sends_without_confirmation_and_never_twice() {
    let mut total_sends = 0u64;
    check("command_state_machine", 500, |r| {
        let mut w = World::new();
        let audit = MemoryAudit::new();
        let mut svc = service(&w, &audit);
        let targets = [
            target("sim-vfd.json", &sim_link("sim"), 2),
            target("sim-temp-controller.json", &sim_link("sim"), 3),
            target("sim-vfd.json", &rtu_link("bus1"), 2),
        ];
        let roles = [Role::Viewer, Role::Operator, Role::Commissioner];
        // Model: the preview id that is currently confirmed and not yet executed.
        let mut confirmed: Option<u64> = None;
        let mut previewed: Option<u64> = None;
        let mut expected_sends = 0u64;
        for _ in 0..r.range_u64(5, 40) {
            let before = w.write_frames().len();
            // 60 %: the legal next step (so the send path is exercised); 40 %: anything.
            let op = if r.chance(600) {
                match svc.state() {
                    CommandState::Idle | CommandState::Done | CommandState::Failed if svc.armed_device().is_none() => 0,
                    CommandState::Armed | CommandState::Done | CommandState::Failed => 2,
                    CommandState::Previewed => 4,
                    _ => 5,
                }
            } else {
                r.below(9)
            };
            let good_role = op == 0 && r.chance(700);
            match op {
                0 => {
                    let t = if good_role {
                        targets[r.below(2) as usize].clone()
                    } else {
                        r.pick(&targets).unwrap().clone()
                    };
                    let role = if good_role {
                        Role::Commissioner
                    } else {
                        *r.pick(&roles).unwrap()
                    };
                    let ok = svc.arm(t, Operator { user: "u".into(), role }).is_ok();
                    if ok || svc.armed_device().is_none() {
                        confirmed = None;
                        previewed = None;
                    }
                }
                1 => {
                    svc.disarm(DisarmReason::Operator);
                    confirmed = None;
                    previewed = None;
                }
                2 | 3 => {
                    let pick = match svc.armed_unit() {
                        Some(2) if r.chance(700) => {
                            if r.chance(700) {
                                0
                            } else {
                                2
                            }
                        }
                        Some(3) if r.chance(700) => 1,
                        _ => r.below(4),
                    };
                    let (reg, v) = match pick.min(3) {
                        0 => ("freq_setpoint", r.range_u64(0, 6000) as f64 / 100.0),
                        1 => ("sp", r.range_u64(0, 1300) as f64 / 10.0),
                        2 => ("control_word", r.range_u64(0, 5) as f64),
                        _ => ("ramp_time", r.unit_f64() * 70.0),
                    };
                    match svc.preview(&mut w.link(), set(reg, v)) {
                        Ok(p) => {
                            previewed = Some(p.id);
                            assert!(confirmed.is_none());
                        }
                        Err(CommandError::WrongState { .. }) => {}
                        Err(_) => {
                            if svc.state() == CommandState::Idle {
                                previewed = None;
                                confirmed = None;
                            }
                        }
                    }
                }
                4 => {
                    let id = if r.chance(800) {
                        previewed.unwrap_or(0)
                    } else {
                        r.below(50)
                    };
                    if svc.confirm(id).is_ok() {
                        assert_eq!(Some(id), previewed, "only the current preview can be confirmed");
                        confirmed = previewed.take();
                    } else if svc.state() == CommandState::Idle {
                        previewed = None;
                    }
                }
                5 | 6 => {
                    let res = svc.execute(&mut w.link());
                    let sent = w.write_frames().len() - before;
                    match res {
                        Ok(o) => {
                            assert_eq!(Some(o.preview_id), confirmed, "execute only runs a confirmed preview");
                            assert_eq!(sent, 1);
                            expected_sends += 1;
                        }
                        Err(_) => assert_eq!(sent, 0),
                    }
                    confirmed = None;
                    // execute() in the Previewed state is refused and keeps the preview.
                    if svc.state() != CommandState::Previewed {
                        previewed = None;
                    }
                }
                7 => {
                    let _ = svc.cancel();
                    confirmed = None;
                    previewed = None;
                }
                _ => {
                    w.clock.advance(if r.chance(200) {
                        r.range_u64(50_000, 70_000)
                    } else {
                        r.range_u64(0, 5_000)
                    });
                    audit.set_failing(r.chance(150));
                }
            }
            if !matches!(op, 5 | 6) {
                assert_eq!(w.write_frames().len(), before, "only execute writes");
            }
        }
        audit.set_failing(false);
        let frames = w.write_frames();
        assert_eq!(frames.len() as u64, expected_sends);
        assert_eq!(svc.sends(), expected_sends);
        let recs = audit.records();
        let sent_records = recs.iter().filter(|x| x.event == Some(AuditEvent::Sent)).count() as u64;
        assert_eq!(sent_records, expected_sends, "every send has its 'sent' record");
        for f in &frames {
            assert_ne!(f[0], 0, "never broadcast");
            let (_, pdu) = rtu::decode_adu(f).unwrap();
            match WriteRequest::decode(pdu).unwrap() {
                WriteRequest::SingleRegister { address: 1, value } if f[0] == 2 => {
                    assert!(value <= 5000, "VFD setpoint in bounds")
                }
                WriteRequest::SingleRegister { address: 0, value } if f[0] == 2 => assert!([0, 1, 4].contains(&value)),
                WriteRequest::SingleRegister { address: 2, value } if f[0] == 2 => assert!((1..=600).contains(&value)),
                WriteRequest::SingleRegister { address: 1, value } if f[0] == 3 => assert!(value <= 1200),
                other => panic!("unexpected write {other:?} to unit {}", f[0]),
            }
        }
        total_sends += expected_sends;
    });
    eprintln!("command_state_machine: {total_sends} frames sent across all cases");
    assert!(
        total_sends > 500,
        "the generator reaches the send path ({total_sends} sends)"
    );
}
