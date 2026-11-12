//! Device instance vs profile (Req 9) and bundled profile loading.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

mod common;

use buslink_core::device::{DeviceError, DeviceInstance};
use buslink_core::profile::SupportLevel;
use common::*;

#[test]
fn soil_sensor_defaults_from_profile() {
    let p = bundled("bgt-smps-8in1.json");
    assert_eq!(p.support_level, SupportLevel::Documented);
    assert_eq!(p.registers.len(), 8);
    let link = rtu_link("bus1");
    let d = DeviceInstance::from_profile("soil-1", "Soil sensor", &link, &p);
    assert_eq!(d.unit_id, 1);
    d.check(&p, &link).unwrap();
    assert_eq!(d.stale_threshold_ms(), 3000);
    assert_eq!(d.timeout_ms(&p, &link), 1000, "manual: response below 1 s");
}

#[test]
fn unit_id_rules() {
    let p = bundled("generic-holding.json");
    let rtu = rtu_link("bus1");
    let mut d = DeviceInstance::from_profile("dev", "Device", &rtu, &p);
    d.unit_id = 0;
    assert_eq!(d.check(&p, &rtu), Err(DeviceError::UnitZero));
    d.unit_id = 248;
    assert_eq!(d.check(&p, &rtu), Err(DeviceError::UnitRange { unit: 248, max: 247 }));
    let tcp = tcp_link("plc", "10.0.0.5");
    let mut t = DeviceInstance::from_profile("dev", "Device", &tcp, &p);
    t.unit_id = 255;
    t.check(&p, &tcp).unwrap();
    t.unit_id = 0;
    assert_eq!(t.check(&p, &tcp), Err(DeviceError::UnitZero));
}

#[test]
fn profile_and_link_must_match() {
    let p = bundled("generic-holding.json");
    let other = bundled("sim-vfd.json");
    let link = rtu_link("bus1");
    let d = DeviceInstance::from_profile("dev", "Device", &link, &p);
    assert!(matches!(
        d.check(&other, &link),
        Err(DeviceError::ProfileMismatch { .. })
    ));
    let mut v2 = p.clone();
    v2.profile_version = "1.0.1".into();
    assert!(matches!(d.check(&v2, &link), Err(DeviceError::ProfileMismatch { .. })));
    assert!(matches!(
        d.check(&p, &rtu_link("bus2")),
        Err(DeviceError::LinkMismatch { .. })
    ));
    let mut slow = d.clone();
    slow.poll_interval_ms = 10;
    assert_eq!(slow.check(&p, &link), Err(DeviceError::PollInterval(10)));
    let mut big = d.clone();
    big.max_registers_per_read = Some(126);
    assert_eq!(big.check(&p, &link), Err(DeviceError::ReadLimit(126)));
}

#[test]
fn stale_threshold_and_owner_keys() {
    let p = bundled("generic-holding.json");
    let link = rtu_link("bus1");
    let mut d = DeviceInstance::from_profile("dev", "Device", &link, &p);
    d.poll_interval_ms = 5000;
    assert_eq!(d.stale_threshold_ms(), 15_000);
    d.stale_after_ms = Some(2000);
    assert_eq!(d.stale_threshold_ms(), 2000);
    assert_eq!(
        rtu_link("a").owner_key(),
        rtu_link("b").owner_key(),
        "same COM port = same bus"
    );
}

#[test]
fn every_bundled_profile_loads() {
    for f in [
        "bgt-smps-8in1.json",
        "sim-vfd.json",
        "sim-temp-controller.json",
        "sim-energy-meter.json",
        "generic-holding.json",
    ] {
        let p = bundled(f);
        assert!(p.registers.iter().all(|r| r.data_type().is_some()), "{f}");
        if p.simulated_only {
            assert_eq!(p.support_level, SupportLevel::Simulated, "{f}");
        }
    }
}
