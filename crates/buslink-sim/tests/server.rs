//! Server rules (sim Req 1): exceptions 01/02/03, atomic multi-writes (S4),
//! silence for bad CRC / unknown unit, broadcast handling.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::ExceptionCode;
use buslink_core::protocol::rtu;
use buslink_sim::SimWorld;

fn world() -> SimWorld {
    let mut w = SimWorld::training(7);
    w.advance_to(0);
    w
}

fn ask(w: &mut SimWorld, unit: u8, pdu: &[u8]) -> Vec<u8> {
    w.handle_pdu(unit, pdu, 1000).pdu.unwrap()
}

#[test]
fn soil_sensor_reads_and_exceptions() {
    let mut w = world();
    let r = ask(&mut w, 1, &[0x03, 0, 0, 0, 8]);
    assert_eq!((r[0], r[1], r.len()), (0x03, 16, 18));
    assert_eq!(
        ask(&mut w, 1, &[0x03, 0, 8, 0, 2]),
        vec![0x83, 0x02],
        "registers 8-9 are not served"
    );
    assert_eq!(
        ask(&mut w, 1, &[0x03, 0, 7, 0, 2]),
        vec![0x83, 0x02],
        "a block reaching past 7 is refused"
    );
    assert_eq!(
        ask(&mut w, 1, &[0x04, 0, 0, 0, 1]),
        vec![0x84, 0x01],
        "FC04 not implemented"
    );
    assert_eq!(
        ask(
            &mut w,
            1,
            &WriteRequest::SingleRegister { address: 0, value: 1 }.encode().unwrap()
        ),
        vec![0x86, 0x01]
    );
    assert_eq!(
        ask(&mut w, 1, &[0x03, 0, 0, 0, 126]),
        vec![0x83, 0x03],
        "quantity above 125"
    );
    assert_eq!(ask(&mut w, 1, &[0x03, 0, 0, 0, 0]), vec![0x83, 0x03]);
    assert_eq!(ask(&mut w, 1, &[0x2B, 0x0E, 1, 0]), vec![0xAB, 0x01]);
}

#[test]
fn vfd_writes_validate_values() {
    let mut w = world();
    let ok = WriteRequest::SingleRegister {
        address: 1,
        value: 3000,
    };
    assert_eq!(ask(&mut w, 2, &ok.encode().unwrap()), ok.response_pdu());
    let bad = WriteRequest::SingleRegister {
        address: 1,
        value: 5001,
    };
    assert_eq!(ask(&mut w, 2, &bad.encode().unwrap()), vec![0x86, 0x03]);
    let bad_cw = WriteRequest::SingleRegister { address: 0, value: 2 };
    assert_eq!(ask(&mut w, 2, &bad_cw.encode().unwrap()), vec![0x86, 0x03]);
    let ro = WriteRequest::SingleRegister { address: 3, value: 1 };
    assert_eq!(
        ask(&mut w, 2, &ro.encode().unwrap()),
        vec![0x86, 0x02],
        "output frequency is read-only"
    );
    let coil = WriteRequest::SingleCoil { address: 0, on: true };
    assert_eq!(ask(&mut w, 2, &coil.encode().unwrap()), vec![0x85, 0x01]);
    assert_eq!(ask(&mut w, 2, &[0x06, 0, 1]), vec![0x86, 0x03], "malformed write");
}

/// S4: a multi-write touching any non-writable address or invalid value changes nothing.
#[test]
fn rejected_multi_write_changes_nothing() {
    let mut w = world();
    let before = w.image(2).unwrap();
    // Addresses 1..=3: 3 is read-only.
    let req = WriteRequest::MultipleRegisters {
        address: 1,
        values: vec![1000, 100, 7],
    };
    assert_eq!(ask(&mut w, 2, &req.encode().unwrap()), vec![0x90, 0x02]);
    // Addresses 1..=2 writable but the ramp value 0 is invalid.
    let req = WriteRequest::MultipleRegisters {
        address: 1,
        values: vec![1000, 0],
    };
    assert_eq!(ask(&mut w, 2, &req.encode().unwrap()), vec![0x90, 0x03]);
    assert_eq!(w.image(2).unwrap(), before);
    let good = WriteRequest::MultipleRegisters {
        address: 1,
        values: vec![1000, 100],
    };
    assert_eq!(ask(&mut w, 2, &good.encode().unwrap()), vec![0x10, 0, 1, 0, 2]);
    let img = w.image(2).unwrap();
    assert_eq!((img.holding[&1], img.holding[&2]), (1000, 100));
}

#[test]
fn silence_for_unknown_unit_and_bad_crc() {
    let mut w = world();
    assert!(w.handle_pdu(99, &[0x03, 0, 0, 0, 1], 0).pdu.is_none());
    let mut adu = rtu::encode_adu(1, &[0x03, 0, 0, 0, 8]).unwrap();
    assert!(w.handle_rtu(&adu, 0).is_some());
    adu[3] ^= 1;
    assert!(w.handle_rtu(&adu, 0).is_none(), "a slave ignores frames with a bad CRC");
}

#[test]
fn broadcast_applies_writes_and_never_replies() {
    let mut w = world();
    let req = WriteRequest::SingleRegister {
        address: 1,
        value: 2500,
    };
    assert!(w.handle_pdu(0, &req.encode().unwrap(), 0).pdu.is_none());
    assert_eq!(w.image(2).unwrap().holding[&1], 2500, "the VFD accepted it");
    assert!(
        w.handle_pdu(0, &[0x03, 0, 0, 0, 1], 0).pdu.is_none(),
        "broadcast reads are ignored"
    );
    let _ = ExceptionCode::ILLEGAL_FUNCTION;
}

#[test]
fn units_and_models() {
    let mut w = SimWorld::new(1);
    assert!(w.add_model(0, "vfd_basic").is_err());
    assert!(w.add_model(248, "vfd_basic").is_err());
    assert!(w.add_model(5, "warp_drive").is_err());
    w.add_model(5, "vfd_basic").unwrap();
    assert!(w.add_model(5, "vfd_basic").is_err());
    assert_eq!(SimWorld::training(1).units(), vec![1, 2, 3, 4]);
}
