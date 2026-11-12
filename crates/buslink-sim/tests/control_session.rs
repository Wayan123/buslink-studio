//! J3 (commissioning-control design §9): `Control` + `Session` + the training
//! bus with the real clock. PRD journey 4, disarm on reconnect, and the
//! commissioning wizard for unit ID and baud. Simulated devices only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use buslink_core::clock::{Clock, SystemClock};
use buslink_core::command::commission::WizardStep;
use buslink_core::command::control::{Control, ControlError};
use buslink_core::command::feedback::FeedbackState;
use buslink_core::command::{
    Ack, CommSettingKind, CommandConfig, CommandError, CommandRequest, DisarmReason, Readback, Role,
};
use buslink_core::device::{DeviceInstance, LinkConfig, LinkKind};
use buslink_core::session::{ExportFormat, LinkState, Session, SessionConfig, TransportFactory};
use buslink_core::transport::TransportError;
use buslink_sim::training::{TRAINING_DEVICES, TRAINING_LINK_ID, Training, TrainingFault};
use common::bundled;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Rig {
    session: Arc<Session>,
    training: Arc<Training>,
    control: Control,
    soil: String,
    vfd: String,
    temp: String,
}

fn rig(seed: u64) -> Rig {
    let clock: Arc<dyn Clock> = SystemClock::shared();
    let training = Arc::new(Training::new(seed, Arc::clone(&clock)));
    let t = Arc::clone(&training);
    let factory: TransportFactory = Arc::new(move |cfg: &LinkConfig| match cfg.kind {
        LinkKind::Simulator { .. } => Ok(t.transport_for(cfg)),
        _ => Err(TransportError::Io("no hardware in tests".into())),
    });
    let session = Arc::new(
        Session::start(
            SessionConfig {
                db_path: None,
                app_version: "test".into(),
                platform: "test".into(),
                logging_interval_ms: 0,
            },
            clock,
            factory,
        )
        .unwrap(),
    );
    for f in ["bgt-smps-8in1.json", "sim-vfd.json", "sim-temp-controller.json"] {
        session.add_profile(bundled(f));
    }
    let link = Training::link_config();
    session.connect(link.clone(), true).unwrap();
    let mut ids = Vec::new();
    for (unit, profile_id) in TRAINING_DEVICES {
        let p = session
            .profiles()
            .into_iter()
            .find(|p| p.profile_id == profile_id)
            .unwrap();
        let id = session.suggest_device_id(&p.profile_id, unit);
        let mut d = DeviceInstance::from_profile(&id, &id, &link, &p);
        d.poll_interval_ms = 100;
        d.response_timeout_ms = Some(150);
        session.add_device(d).unwrap();
        ids.push(id);
    }
    session.start_polling(TRAINING_LINK_ID).unwrap();
    let control = Control::new(Arc::clone(&session), CommandConfig::default());
    Rig {
        session,
        training,
        control,
        soil: ids[0].clone(),
        vfd: ids[1].clone(),
        temp: ids[2].clone(),
    }
}

fn wait_for(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

fn good(s: &Session, device: &str, tag: &str) -> bool {
    s.tag(device, tag).is_some_and(|t| t.quality == "GOOD")
}

fn named(command: &str, value: Option<f64>) -> CommandRequest {
    CommandRequest::Named {
        command: command.into(),
        value,
    }
}

fn send(r: &Rig, device: &str, req: CommandRequest) -> buslink_core::command::CommandOutcome {
    let p = r.control.preview(device, req).unwrap();
    r.control.confirm(p.id).unwrap();
    r.control.execute().unwrap()
}

fn feedback_state(r: &Rig) -> Option<FeedbackState> {
    r.control.status().last.and_then(|l| l.feedback).map(|f| f.state)
}

/// PRD journey 4: arm → run → 30.0 Hz → read-back OK → reached target → software stop → audit.
#[test]
fn journey4_vfd_through_control() {
    let r = rig(41);
    wait_for("VFD polled", 10, || good(&r.session, &r.vfd, "output_freq"));
    // Monitoring by default: nothing is armed and a preview is refused.
    assert!(r.control.status().armed_device.is_none());
    assert!(matches!(
        r.control.preview(&r.vfd, named("run", None)),
        Err(ControlError::Command(CommandError::NotArmed))
    ));
    let st = r.control.arm(&r.vfd, "trainee").unwrap();
    assert_eq!(st.armed_device.as_deref(), Some(r.vfd.as_str()));
    assert!(st.armed_remaining_ms.unwrap() > 55_000 && st.arm_timeout_ms == 60_000);

    let o = send(&r, &r.vfd, named("run", None));
    assert_eq!((o.ack, o.readback), (Ack::Acknowledged, Readback::Verified));
    wait_for("run feedback", 5, || feedback_state(&r) == Some(FeedbackState::Reached));

    let p = r.control.preview(&r.vfd, named("set_frequency", Some(30.0))).unwrap();
    assert_eq!(
        (p.unit_id, p.address, p.function, p.unit.as_deref()),
        (2, 1, 0x06, Some("Hz"))
    );
    assert!(p.frame_hex.starts_with("02 06 00 01 0B B8"), "{}", p.frame_hex);
    r.control.confirm(p.id).unwrap();
    let o = r.control.execute().unwrap();
    assert_eq!(
        (o.ack, o.readback, o.readback_value),
        (Ack::Acknowledged, Readback::Verified, Some(30.0))
    );
    // Acknowledged first, reached later (ramp ~3 s), never the other way round.
    assert_ne!(feedback_state(&r), Some(FeedbackState::Reached));
    wait_for("30.0 Hz reached", 10, || {
        feedback_state(&r) == Some(FeedbackState::Reached)
    });

    let o = send(&r, &r.vfd, named("stop", None));
    assert_eq!(
        (o.ack, o.readback, o.readback_value),
        (Ack::Acknowledged, Readback::Verified, Some(0.0))
    );
    wait_for("0 Hz reached", 10, || {
        feedback_state(&r) == Some(FeedbackState::Reached)
    });

    let audit = r.session.audit_rows().unwrap();
    let events: Vec<_> = audit.iter().filter_map(|a| a.event.clone()).collect();
    assert_eq!(events.iter().filter(|e| *e == "result").count(), 3);
    assert_eq!(events.iter().filter(|e| *e == "feedback").count(), 3);
    assert!(audit.iter().any(|a| a.event.as_deref() == Some("result")
        && a.command_id.as_deref() == Some("set_frequency")
        && a.readback.as_deref() == Some("verified")
        && a.requested_value == Some(30.0)));
    assert!(audit.iter().any(|a| a.event.as_deref() == Some("feedback")
        && a.command_id.as_deref() == Some("stop")
        && a.readback.as_deref() == Some("reached")));
    // Audit export: CSV with a header and one line per row; XLSX opens as a zip.
    let csv = String::from_utf8(r.session.audit_export(ExportFormat::Csv).unwrap()).unwrap();
    assert!(csv.starts_with("ts_utc,session_id,user,role,event"));
    assert_eq!(csv.lines().count(), audit.len() + 1);
    assert!(r.session.audit_export(ExportFormat::Xlsx).unwrap().starts_with(b"PK"));
    // Report: devices with profile versions, commands, simulated flags.
    let rep = r.session.report().unwrap();
    assert_eq!(rep.devices.len(), 3);
    assert!(
        rep.devices
            .iter()
            .any(|d| d.device_id == r.vfd && d.simulated_only && d.profile_version == "1.1.0")
    );
    assert!(
        rep.devices
            .iter()
            .all(|d| d.quality.get("GOOD").copied().unwrap_or(0) > 0)
    );
    assert_eq!(rep.audit.len(), audit.len());
    r.session.shutdown();
}

/// Disconnect, reconnect or a link settings change drops the arm; nothing resumes.
#[test]
fn disarm_on_disconnect_and_reconnect() {
    let r = rig(42);
    wait_for("temp polled", 10, || good(&r.session, &r.temp, "pv"));
    r.control.arm(&r.temp, "trainee").unwrap();
    let p = r.control.preview(&r.temp, named("heater_on", None)).unwrap();
    r.control.confirm(p.id).unwrap();
    r.session.disconnect(TRAINING_LINK_ID).unwrap();
    let st = r.control.status();
    assert_eq!(
        (st.armed_device, st.disarm_reason),
        (None, Some(DisarmReason::Disconnect))
    );
    assert!(r.control.execute().is_err(), "the confirmed command is gone");
    r.session.reconnect(TRAINING_LINK_ID).unwrap();
    r.session.start_polling(TRAINING_LINK_ID).unwrap();
    assert!(r.control.status().armed_device.is_none(), "reconnect never re-arms");
    assert!(r.control.preview(&r.temp, named("heater_on", None)).is_err());
    // Arm again, then a reconnect in between drops it with reason "reconnect".
    r.control.arm(&r.temp, "trainee").unwrap();
    r.session.disconnect(TRAINING_LINK_ID).unwrap();
    r.session.reconnect(TRAINING_LINK_ID).unwrap();
    r.session.start_polling(TRAINING_LINK_ID).unwrap();
    let st = r.control.status();
    assert_eq!(
        (st.armed_device, st.disarm_reason),
        (None, Some(DisarmReason::Reconnect))
    );
    // The heater never received a command.
    let rows = r.session.audit_rows().unwrap();
    assert!(!rows.iter().any(|a| a.event.as_deref() == Some("sent")));
    r.session.shutdown();
}

/// Wizard: unit ID 2 → 5 on the VFD, then baud 9600 → 19200 on the whole link (temp controller).
#[test]
fn wizard_unit_id_and_baud_pass() {
    let r = rig(43);
    wait_for("VFD polled", 10, || good(&r.session, &r.vfd, "output_freq"));
    assert_eq!(
        r.control.wizard_support(&r.vfd).unwrap(),
        vec![CommSettingKind::UnitId, CommSettingKind::Baud]
    );
    assert!(
        r.control.wizard_support(&r.soil).is_err(),
        "the soil profile documents no comm registers"
    );
    // Validation before anything is armed or written.
    assert!(
        r.control
            .wizard_start(&r.vfd, CommSettingKind::UnitId, 3, "tech")
            .is_err(),
        "unit 3 is taken"
    );
    assert!(
        r.control
            .wizard_start(&r.vfd, CommSettingKind::UnitId, 0, "tech")
            .is_err()
    );
    assert!(
        r.control
            .wizard_start(&r.vfd, CommSettingKind::Baud, 57_600, "tech")
            .is_err()
    );

    let w = r
        .control
        .wizard_start(&r.vfd, CommSettingKind::UnitId, 5, "tech")
        .unwrap();
    assert_eq!((w.step, w.old.unit_id, w.new.unit_id), (WizardStep::Journaled, 2, 5));
    assert!(r.control.wizard_preview().is_err(), "arm first");
    r.control.arm(&r.vfd, "tech").unwrap();
    r.control.commissioner_unlock(&r.vfd).unwrap();
    let p = r.control.wizard_preview().unwrap();
    assert_eq!((p.address, p.raw_registers.clone()), (7, vec![5]));
    r.control.confirm(p.id).unwrap();
    let o = r.control.execute().unwrap();
    assert_eq!((o.ack, o.readback), (Ack::Acknowledged, Readback::Unknown));
    assert_eq!(r.control.status().wizard.unwrap().step, WizardStep::AwaitReconnect);
    assert!(r.control.wizard_preview().is_err(), "a run sends once");
    let w = r.control.wizard_reconnect().unwrap();
    assert_eq!(w.step, WizardStep::Reconnected);
    assert!(r.control.status().armed_device.is_none(), "reconnect drops the arm");
    let w = r.control.wizard_verify().unwrap();
    assert_eq!((w.step, w.verified_value), (WizardStep::Passed, Some(5)));
    assert_eq!(
        r.session.links()[0]
            .devices
            .iter()
            .find(|d| d.id == r.vfd)
            .unwrap()
            .unit_id,
        5
    );
    wait_for("VFD GOOD at unit 5", 10, || {
        r.session
            .tag(&r.vfd, "output_freq")
            .is_some_and(|t| t.quality == "GOOD" && t.ts_ms > 0)
    });
    r.control.wizard_close().unwrap();

    // Baud on the temperature controller: the other devices stay at 9600 and go quiet.
    let w = r
        .control
        .wizard_start(&r.temp, CommSettingKind::Baud, 19_200, "tech")
        .unwrap();
    assert_eq!(w.other_devices.len(), 2);
    r.control.arm(&r.temp, "tech").unwrap();
    r.control.commissioner_unlock(&r.temp).unwrap();
    let p = r.control.wizard_preview().unwrap();
    assert_eq!(p.raw_registers, vec![2]);
    r.control.confirm(p.id).unwrap();
    assert_eq!(r.control.execute().unwrap().ack, Ack::Acknowledged);
    r.control.wizard_reconnect().unwrap();
    assert_eq!(
        r.session.link_config(TRAINING_LINK_ID).unwrap().serial().unwrap().baud,
        19_200
    );
    let w = r.control.wizard_verify().unwrap();
    assert_eq!((w.step, w.verified_value), (WizardStep::Passed, Some(19_200)));
    wait_for("soil sensor TIMEOUT at 19200", 10, || {
        r.session
            .tag(&r.soil, "moisture")
            .is_some_and(|t| t.quality == "TIMEOUT")
    });
    let rows = r.session.audit_rows().unwrap();
    let journal = rows.iter().filter(|a| a.event.as_deref() == Some("journal")).count();
    let verify: Vec<_> = rows.iter().filter(|a| a.event.as_deref() == Some("verify")).collect();
    assert_eq!(journal, 2);
    assert_eq!(verify.len(), 2);
    assert!(verify.iter().all(|a| a.readback.as_deref() == Some("verified")));
    // The journal comes before the arm of its run.
    let first_journal = rows.iter().position(|a| a.event.as_deref() == Some("journal")).unwrap();
    let first_sent = rows.iter().position(|a| a.event.as_deref() == Some("sent")).unwrap();
    assert!(first_journal < first_sent);
    r.session.shutdown();
}

/// Wizard failure: the device moved but does not answer at the new ID (injected
/// timeout); verification fails and "revert" returns the session to the old settings.
#[test]
fn wizard_failure_and_revert() {
    let r = rig(44);
    wait_for("temp polled", 10, || good(&r.session, &r.temp, "pv"));
    r.control
        .wizard_start(&r.temp, CommSettingKind::UnitId, 7, "tech")
        .unwrap();
    r.control.arm(&r.temp, "tech").unwrap();
    r.control.commissioner_unlock(&r.temp).unwrap();
    let p = r.control.wizard_preview().unwrap();
    r.control.confirm(p.id).unwrap();
    r.control.execute().unwrap();
    assert!(r.control.wizard_close().is_err(), "cannot close a sent, unverified run");
    r.training.inject(7, TrainingFault::Timeout, 30_000).unwrap();
    r.control.wizard_reconnect().unwrap();
    let w = r.control.wizard_verify().unwrap();
    assert_eq!(w.step, WizardStep::Failed);
    assert_eq!(w.old.unit_id, 3, "recovery hint keeps the old settings");
    let w = r.control.wizard_revert().unwrap();
    assert_eq!(w.step, WizardStep::Reverted);
    assert_eq!(
        r.session.links()[0]
            .devices
            .iter()
            .find(|d| d.id == r.temp)
            .unwrap()
            .unit_id,
        3
    );
    assert_eq!(
        r.session.link_generation(TRAINING_LINK_ID).unwrap().1,
        LinkState::Polling
    );
    let rows = r.session.audit_rows().unwrap();
    assert!(
        rows.iter()
            .any(|a| a.event.as_deref() == Some("verify") && a.readback.as_deref() == Some("unknown"))
    );
    assert_eq!(
        rows.iter().filter(|a| a.event.as_deref() == Some("sent")).count(),
        1,
        "one write only"
    );
    r.control.wizard_close().unwrap();
    r.session.shutdown();
}

/// Security review 07, M1: the role is host state. Arming is always operator,
/// comm-config fails without the unlock, and the unlock ends with the arm.
#[test]
fn commissioner_unlock_is_host_state_and_ends_with_the_arm() {
    let r = rig(45);
    wait_for("VFD polled", 10, || good(&r.session, &r.vfd, "output_freq"));
    assert!(r.control.arm(&r.vfd, "  ").is_err(), "empty user");
    assert!(r.control.arm(&r.vfd, &"x".repeat(65)).is_err(), "over 64 characters");
    assert!(r.control.arm(&r.vfd, "a\nb").is_err(), "control character");
    assert!(r.control.commissioner_unlock(&r.vfd).is_err(), "unlock needs an arm");

    let st = r.control.arm(&r.vfd, "  tech  ").unwrap();
    assert_eq!(
        (st.armed_role, st.armed_user.as_deref()),
        (Some(Role::Operator), Some("tech"))
    );
    r.control
        .wizard_start(&r.vfd, CommSettingKind::UnitId, 5, "tech")
        .unwrap();
    assert!(
        matches!(
            r.control.wizard_preview(),
            Err(ControlError::Command(CommandError::Role { .. }))
        ),
        "comm-config write is refused without the unlock"
    );
    assert!(
        r.control.commissioner_unlock(&r.temp).is_err(),
        "other device is not armed"
    );
    let st = r.control.commissioner_unlock(&r.vfd).unwrap();
    assert_eq!(st.armed_role, Some(Role::Commissioner));
    r.control.wizard_preview().unwrap();
    r.control.cancel().unwrap();

    // Disconnect drops arm and unlock; a new arm is operator again.
    r.session.stop_polling(TRAINING_LINK_ID).unwrap();
    assert!(r.control.status().armed_role.is_none());
    r.session.start_polling(TRAINING_LINK_ID).unwrap();
    let st = r.control.arm(&r.vfd, "tech").unwrap();
    assert_eq!(st.armed_role, Some(Role::Operator), "unlock did not survive");
    assert!(r.control.wizard_preview().is_err());

    // Disarm (also app background) and re-arm: operator again.
    r.control.commissioner_unlock(&r.vfd).unwrap();
    r.control.disarm(DisarmReason::AppBackground);
    assert_eq!(r.control.arm(&r.vfd, "tech").unwrap().armed_role, Some(Role::Operator));
    let rows = r.session.audit_rows().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|a| a.event.as_deref() == Some("role_unlock"))
            .count(),
        2,
        "each unlock is audited"
    );
    r.session.shutdown();
}

/// The unlock expires with the idle arm timeout.
#[test]
fn commissioner_unlock_expires_with_the_arm() {
    let mut r = rig(46);
    r.control = Control::new(Arc::clone(&r.session), CommandConfig { arm_timeout_ms: 400 });
    wait_for("VFD polled", 10, || good(&r.session, &r.vfd, "output_freq"));
    r.control.arm(&r.vfd, "tech").unwrap();
    assert_eq!(
        r.control.commissioner_unlock(&r.vfd).unwrap().armed_role,
        Some(Role::Commissioner)
    );
    std::thread::sleep(Duration::from_millis(600));
    let st = r.control.status();
    assert_eq!((st.armed_role, st.disarm_reason), (None, Some(DisarmReason::Expired)));
    assert_eq!(r.control.arm(&r.vfd, "tech").unwrap().armed_role, Some(Role::Operator));
    r.session.shutdown();
}
