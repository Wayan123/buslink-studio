//! P3: write PDU encode → decode round trip for FC05/06/15/16, echo replies and limits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::command::wire::{MAX_WRITE_COILS, MAX_WRITE_REGISTERS, WriteRequest};
use buslink_core::protocol::rtu::{self, FramerStatus, ResponseFramer};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;

fn random_write(r: &mut SplitMix64) -> WriteRequest {
    match r.below(4) {
        0 => WriteRequest::SingleCoil {
            address: r.next_u16(),
            on: r.bool(),
        },
        1 => WriteRequest::SingleRegister {
            address: r.next_u16(),
            value: r.next_u16(),
        },
        2 => {
            let q = r.range_u64(1, u64::from(MAX_WRITE_COILS)) as usize;
            let a = r.range_u64(0, 65_536 - q as u64) as u16;
            WriteRequest::MultipleCoils {
                address: a,
                values: (0..q).map(|_| r.bool()).collect(),
            }
        }
        _ => {
            let q = r.range_u64(1, u64::from(MAX_WRITE_REGISTERS)) as usize;
            let a = r.range_u64(0, 65_536 - q as u64) as u16;
            WriteRequest::MultipleRegisters {
                address: a,
                values: (0..q).map(|_| r.next_u16()).collect(),
            }
        }
    }
}

#[test]
fn write_round_trip_every_fc() {
    let mut per_fc = [0u32; 4];
    check("write_round_trip", 1000, |r| {
        let w = random_write(r);
        per_fc[match w.function_code() {
            0x05 => 0,
            0x06 => 1,
            0x0F => 2,
            _ => 3,
        }] += 1;
        let pdu = w.encode().unwrap();
        assert!(pdu.len() <= 253);
        assert_eq!(WriteRequest::decode(&pdu).unwrap(), w);
        // Through an RTU ADU, and the echo reply through the length-based framer.
        let unit = r.range_u64(1, 247) as u8;
        let adu = rtu::encode_adu(unit, &pdu).unwrap();
        let (u, p) = rtu::decode_adu(&adu).unwrap();
        assert_eq!((u, WriteRequest::decode(p).unwrap()), (unit, w.clone()));
        let reply = rtu::encode_adu(unit, &w.response_pdu()).unwrap();
        assert_eq!(reply.len(), 8, "every write reply is 8 bytes on RTU");
        let mut f = ResponseFramer::new(unit, w.function_code(), w.expected_reply());
        match f.push(&reply) {
            FramerStatus::Complete(Ok(got)) => assert!(w.verify_echo(&got).is_ok()),
            other => panic!("{other:?}"),
        }
    });
    assert!(per_fc.iter().all(|&n| n > 150), "every FC covered: {per_fc:?}");
}

#[test]
fn decoder_never_panics_on_random_bytes() {
    check("write_decode_no_panic", 10_000, |r| {
        let len = r.range_usize(0, 260);
        let mut b = r.bytes(len);
        if let Some(first) = b.first_mut() {
            let any = r.next_u8();
            *first = *r.pick(&[0x05u8, 0x06, 0x0F, 0x10, any]).unwrap();
        }
        let _ = WriteRequest::decode(&b);
    });
}

#[test]
fn echo_must_match_exactly() {
    check("echo_mismatch", 1000, |r| {
        let w = random_write(r);
        let mut echo = w.response_pdu();
        let i = r.range_usize(0, echo.len() - 1);
        echo[i] ^= 1 << r.below(8);
        assert!(w.verify_echo(&echo).is_err());
    });
}
