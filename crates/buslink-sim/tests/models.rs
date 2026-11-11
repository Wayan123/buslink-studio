//! Device models (sim Req 3–6): soil drift bounds, VFD ramp/fault/reset,
//! temperature lag and PI convergence (S8), energy monotonic (S9).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::command::wire::WriteRequest;
use buslink_core::testkit::check;
use buslink_sim::SimWorld;
use buslink_sim::devices::temp::{AMBIENT_C, GAIN_C};
use buslink_sim::devices::vfd::{FAULT_OVERCURRENT, Vfd};

fn write(w: &mut SimWorld, unit: u8, address: u16, value: u16, t: u64) -> Vec<u8> {
    w.handle_pdu(
        unit,
        &WriteRequest::SingleRegister { address, value }.encode().unwrap(),
        t,
    )
    .pdu
    .unwrap()
}

fn hr(w: &SimWorld, unit: u8, a: u16) -> u16 {
    w.image(unit).unwrap().holding[&a]
}

#[test]
fn soil_values_drift_within_plausible_bounds() {
    check("soil_drift", 8, |r| {
        let mut w = SimWorld::training(r.next_u64());
        w.advance_to(0);
        let first = w.image(1).unwrap().holding.clone();
        let mut changed = false;
        for h in 1..=24u64 {
            w.advance_to(h * 3_600_000);
            let img = w.image(1).unwrap();
            let temp = f64::from(img.holding[&0] as i16) / 10.0;
            let moist = f64::from(img.holding[&1]) / 10.0;
            let ph = f64::from(img.holding[&7]) / 100.0;
            assert!(
                (-30.0..=70.0).contains(&temp) && (15.0..=35.0).contains(&temp),
                "temperature {temp}"
            );
            assert!((0.0..=100.0).contains(&moist), "moisture {moist}");
            assert!((5.5..=8.0).contains(&ph), "pH {ph}");
            changed |= img.holding != first;
        }
        assert!(changed, "values drift");
    });
}

#[test]
fn vfd_ramps_to_setpoint_and_back() {
    let mut w = SimWorld::training(1);
    w.advance_to(0);
    write(&mut w, 2, 1, 3000, 0); // 30.00 Hz
    write(&mut w, 2, 2, 100, 0); // 10.0 s for 0-50 Hz → 5 Hz/s
    write(&mut w, 2, 0, 1, 0);
    w.advance_to(3000);
    let f = hr(&w, 2, 3);
    assert!((1400..=1600).contains(&f), "3 s at 5 Hz/s ≈ 15 Hz, got {f}");
    assert_eq!(hr(&w, 2, 4) & 0b11, 0b01, "running, not at speed");
    w.advance_to(10_000);
    assert_eq!(hr(&w, 2, 3), 3000);
    assert_eq!(hr(&w, 2, 4) & 0b11, 0b11, "at speed");
    assert!(hr(&w, 2, 6) > 0, "motor current");
    write(&mut w, 2, 0, 0, 10_000);
    w.advance_to(20_000);
    assert_eq!((hr(&w, 2, 3), hr(&w, 2, 4)), (0, 0));
}

#[test]
fn vfd_fault_blocks_run_until_reset_while_stopped() {
    let mut w = SimWorld::training(1);
    w.advance_to(0);
    write(&mut w, 2, 1, 2000, 0);
    w.with_model::<Vfd, ()>(2, |v| v.trip(FAULT_OVERCURRENT)).unwrap();
    let ack = write(&mut w, 2, 0, 1, 0);
    assert_eq!(ack[0], 0x06, "run is acknowledged");
    w.advance_to(10_000);
    assert_eq!(hr(&w, 2, 0), 1, "the control word reads back 1");
    assert_eq!(hr(&w, 2, 3), 0, "but the drive did not start");
    assert_eq!(hr(&w, 2, 4), 1 << 3, "fault bit, not running");
    assert_eq!(hr(&w, 2, 5), FAULT_OVERCURRENT);
    write(&mut w, 2, 0, 4, 10_000);
    assert_eq!(
        (hr(&w, 2, 5), hr(&w, 2, 0)),
        (0, 0),
        "reset clears the fault; drive stays stopped"
    );
    write(&mut w, 2, 0, 1, 10_000);
    w.advance_to(20_000);
    assert_eq!(hr(&w, 2, 3), 2000);
    // Reset while running has no effect on a fresh fault-free drive and keeps it running.
    write(&mut w, 2, 0, 4, 20_000);
    assert_eq!(hr(&w, 2, 0), 1);
}

/// S8: auto mode converges to SP; PV never exceeds the physical bound.
#[test]
fn temperature_converges_and_respects_physics() {
    check("temp_convergence", 30, |r| {
        let mut w = SimWorld::training(r.next_u64());
        w.advance_to(0);
        let sp = r.range_u64(300, 1200) as u16; // 30.0-120.0 °C, all reachable (max 175 °C)
        write(&mut w, 3, 1, sp, 0);
        write(&mut w, 3, 5, 1, 0);
        let bound = AMBIENT_C + GAIN_C + 0.5;
        let mut t = 0;
        while t < 1_800_000 {
            t += 10_000;
            w.advance_to(t);
            let pv = f64::from(hr(&w, 3, 0) as i16) / 10.0;
            assert!(pv <= bound && pv >= AMBIENT_C - 1.0, "PV {pv}");
        }
        let pv = f64::from(hr(&w, 3, 0) as i16) / 10.0;
        assert!(
            (pv - f64::from(sp) / 10.0).abs() < 1.0,
            "PV {pv} vs SP {}",
            f64::from(sp) / 10.0
        );
        // Heater off → output 0 and PV falls back toward ambient.
        write(&mut w, 3, 5, 0, t);
        w.advance_to(t + 600_000);
        assert_eq!(hr(&w, 3, 2), 0);
        assert!(f64::from(hr(&w, 3, 0) as i16) / 10.0 < 30.0);
    });
}

#[test]
fn temperature_manual_mode_follows_first_order_lag() {
    let mut w = SimWorld::training(9);
    w.advance_to(0);
    write(&mut w, 3, 3, 1, 0);
    write(&mut w, 3, 4, 200, 0); // 20 % → steady state 25 + 30 = 55 °C
    write(&mut w, 3, 5, 1, 0);
    w.advance_to(60_000); // one time constant: 63 %
    let pv = f64::from(hr(&w, 3, 0) as i16) / 10.0;
    assert!((43.0..=45.0).contains(&pv), "PV after τ = {pv} (expected ≈ 43.9)");
    w.advance_to(600_000);
    let pv = f64::from(hr(&w, 3, 0) as i16) / 10.0;
    assert!((pv - 55.0).abs() < 0.5, "{pv}");
    assert_eq!(hr(&w, 3, 2), 200);
}

/// S9
#[test]
fn energy_counters_never_decrease() {
    check("energy_monotonic", 20, |r| {
        let mut w = SimWorld::training(r.next_u64());
        w.advance_to(0);
        let read = |w: &SimWorld| {
            let i = &w.image(4).unwrap().input;
            let u32v = (u32::from(i[&10]) << 16) | u32::from(i[&11]);
            let u64v =
                (u64::from(i[&12]) << 48) | (u64::from(i[&13]) << 32) | (u64::from(i[&14]) << 16) | u64::from(i[&15]);
            (u32v, u64v)
        };
        let mut last = read(&w);
        for k in 1..=200u64 {
            w.advance_to(k * r.range_u64(100, 60_000));
            let now = read(&w);
            assert!(now.0 >= last.0 && now.1 >= last.1);
            last = now;
        }
        assert!(last.1 > 123_456, "energy accumulates");
    });
}
