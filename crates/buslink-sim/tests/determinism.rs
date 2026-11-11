//! S1 (same seed → same bytes), S2 (step splitting does not matter), fault triggers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::command::wire::WriteRequest;
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;
use buslink_sim::devices::temp::TempController;
use buslink_sim::fault::FaultError;
use buslink_sim::{FaultKind, FaultRule, SimWorld, Trigger};

fn requests(r: &mut SplitMix64, n: usize) -> Vec<(u64, u8, Vec<u8>)> {
    let mut t = 0;
    (0..n)
        .map(|_| {
            t += r.range_u64(0, 3000);
            let unit = r.range_u64(1, 5) as u8;
            let pdu = match r.below(4) {
                0 => vec![0x03, 0, 0, 0, r.range_u64(1, 8) as u8],
                1 => vec![0x04, 0, 0, 0, 16],
                2 => WriteRequest::SingleRegister {
                    address: 1,
                    value: r.range_u64(0, 1300) as u16,
                }
                .encode()
                .unwrap(),
                _ => WriteRequest::SingleRegister {
                    address: 5,
                    value: r.range_u64(0, 1) as u16,
                }
                .encode()
                .unwrap(),
            };
            (t, unit, pdu)
        })
        .collect()
}

fn with_faults(seed: u64) -> SimWorld {
    let mut w = SimWorld::training(seed);
    w.add_fault(FaultRule {
        unit: 1,
        kind: FaultKind::Timeout,
        trigger: Trigger::Probability { per_mille: 200 },
    })
    .unwrap();
    w.add_fault(FaultRule {
        unit: 4,
        kind: FaultKind::CrcError,
        trigger: Trigger::Probability { per_mille: 100 },
    })
    .unwrap();
    w
}

/// S1
#[test]
fn same_seed_same_bytes() {
    check("sim_determinism", 100, |r| {
        let seed = r.next_u64();
        let reqs = requests(r, 60);
        let run = |seed: u64| -> Vec<Option<Vec<u8>>> {
            let mut w = with_faults(seed);
            reqs.iter()
                .map(|(t, u, p)| {
                    let rep = w.handle_pdu(*u, p, *t);
                    rep.pdu.map(|mut x| {
                        x.push(u8::from(rep.corrupt));
                        x
                    })
                })
                .collect()
        };
        assert_eq!(run(seed), run(seed));
        let other = run(seed ^ 1);
        assert_ne!(run(seed), other, "a different seed gives different drift");
    });
}

/// S2
#[test]
fn advance_splitting_does_not_change_state() {
    check("sim_step_split", 100, |r| {
        let end = r.range_u64(1000, 600_000);
        let mut a = SimWorld::training(42);
        a.advance_to(0);
        a.handle_pdu(
            3,
            &WriteRequest::SingleRegister { address: 5, value: 1 }.encode().unwrap(),
            0,
        );
        let mut b = SimWorld::training(42);
        b.advance_to(0);
        b.handle_pdu(
            3,
            &WriteRequest::SingleRegister { address: 5, value: 1 }.encode().unwrap(),
            0,
        );
        a.advance_to(end);
        let mut t = 0;
        while t < end {
            t = (t + r.range_u64(1, 20_000)).min(end);
            b.advance_to(t);
        }
        for u in 1..=4 {
            assert_eq!(a.image(u), b.image(u), "unit {u}");
        }
    });
}

#[test]
fn trigger_windows_and_request_counts() {
    let mut w = SimWorld::training(1);
    w.add_fault(FaultRule {
        unit: 2,
        kind: FaultKind::Exception(4),
        trigger: Trigger::Window {
            from_ms: 1000,
            until_ms: 2000,
        },
    })
    .unwrap();
    w.add_fault(FaultRule {
        unit: 1,
        kind: FaultKind::Timeout,
        trigger: Trigger::Requests { skip: 2, count: 3 },
    })
    .unwrap();
    let rd = [0x03, 0, 0, 0, 1];
    assert_eq!(w.handle_pdu(2, &rd, 999).pdu.unwrap()[0], 0x03);
    assert_eq!(w.handle_pdu(2, &rd, 1000).pdu.unwrap(), vec![0x83, 0x04]);
    assert_eq!(w.handle_pdu(2, &rd, 1999).pdu.unwrap(), vec![0x83, 0x04]);
    assert_eq!(w.handle_pdu(2, &rd, 2000).pdu.unwrap()[0], 0x03);
    let answered: Vec<bool> = (0..7).map(|i| w.handle_pdu(1, &rd, 3000 + i).pdu.is_some()).collect();
    assert_eq!(answered, vec![true, true, false, false, false, true, true]);
}

#[test]
fn first_shaping_fault_wins_and_faults_can_be_removed() {
    let mut w = SimWorld::training(1);
    let a = w
        .add_fault(FaultRule {
            unit: 2,
            kind: FaultKind::Exception(6),
            trigger: Trigger::Always,
        })
        .unwrap();
    w.add_fault(FaultRule {
        unit: 2,
        kind: FaultKind::Timeout,
        trigger: Trigger::Always,
    })
    .unwrap();
    assert_eq!(w.handle_pdu(2, &[0x03, 0, 0, 0, 1], 0).pdu.unwrap(), vec![0x83, 0x06]);
    assert!(w.remove_fault(a));
    assert!(w.handle_pdu(2, &[0x03, 0, 0, 0, 1], 0).pdu.is_none());
    w.clear_faults(2);
    assert!(w.handle_pdu(2, &[0x03, 0, 0, 0, 1], 0).pdu.is_some());
    assert_eq!(
        w.add_fault(FaultRule {
            unit: 2,
            kind: FaultKind::Stale,
            trigger: Trigger::Probability { per_mille: 5 }
        }),
        Err(buslink_sim::world::WorldError::Fault(
            FaultError::StaleNeedsDeterministicTrigger
        ))
    );
}

#[test]
fn stale_fault_freezes_values_but_keeps_answering() {
    let mut w = SimWorld::training(3);
    w.advance_to(0);
    w.handle_pdu(
        3,
        &WriteRequest::SingleRegister { address: 5, value: 1 }.encode().unwrap(),
        0,
    );
    let id = w
        .add_fault(FaultRule {
            unit: 3,
            kind: FaultKind::Stale,
            trigger: Trigger::Always,
        })
        .unwrap();
    let a = w.handle_pdu(3, &[0x03, 0, 0, 0, 1], 60_000).pdu.unwrap();
    let pv_frozen = w.with_model::<TempController, f64>(3, |t| t.pv).unwrap();
    assert!(
        (pv_frozen - 25.0).abs() < 0.5,
        "PV did not move while stale: {pv_frozen}"
    );
    assert_eq!(a[0], 0x03, "still answering");
    w.remove_fault(id);
    w.advance_to(120_000);
    assert!(w.with_model::<TempController, f64>(3, |t| t.pv).unwrap() > 30.0);
}

#[test]
fn wrong_order_swaps_words_and_bytes() {
    let mut w = SimWorld::training(5);
    w.advance_to(0);
    let good = w.handle_pdu(4, &[0x04, 0, 0, 0, 2], 0).pdu.unwrap();
    let soil = w.handle_pdu(1, &[0x03, 0, 7, 0, 1], 0).pdu.unwrap();
    w.add_fault(FaultRule {
        unit: 4,
        kind: FaultKind::WrongOrder,
        trigger: Trigger::Always,
    })
    .unwrap();
    w.add_fault(FaultRule {
        unit: 1,
        kind: FaultKind::WrongOrder,
        trigger: Trigger::Always,
    })
    .unwrap();
    let bad = w.handle_pdu(4, &[0x04, 0, 0, 0, 2], 0).pdu.unwrap();
    assert_eq!(&bad[2..4], &good[4..6]);
    assert_eq!(&bad[4..6], &good[2..4]);
    let soil_bad = w.handle_pdu(1, &[0x03, 0, 7, 0, 1], 0).pdu.unwrap();
    assert_eq!((soil_bad[2], soil_bad[3]), (soil[3], soil[2]));
}
