//! Property tests P1, P2, P4, P5 (design §11) for CRC, read PDUs, RTU and MBAP framing.
#![allow(clippy::unwrap_used)]

use buslink_core::protocol::crc::{self, crc16};
use buslink_core::protocol::pdu::{
    ExpectedReply, ReadData, ReadFunction, ReadRequest, decode_read_response, encode_read_response,
};
use buslink_core::protocol::rtu::{self, FramerStatus, ResponseFramer};
use buslink_core::protocol::tcp::{self, MbapReceiver};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::{check, unhex};

#[test]
fn crc_vectors() {
    assert_eq!(crc16(b"123456789"), 0x4B37);
    assert_eq!(crc16(&unhex("01 03 00 00 00 08")).to_le_bytes(), [0x44, 0x0C]);
    // Response of a BGT-SMPS-like device (unit 1, FC03, 2 registers) checked by residue.
    let mut f = unhex("01 03 04 01 0A 00 FA");
    crc::append(&mut f);
    assert!(crc::is_valid(&f));
}

#[test]
fn crc_residue() {
    // Appending the CRC low byte first makes the CRC over the whole frame zero.
    check("crc_residue", 2000, |r| {
        let len = r.range_usize(1, 256);
        let mut msg = r.bytes(len);
        crc::append(&mut msg);
        assert_eq!(crc16(&msg), 0);
        let i = r.range_usize(0, msg.len() - 1);
        msg[i] ^= 1 << r.below(8);
        assert!(!crc::is_valid(&msg), "single-bit error must be detected");
    });
}

fn random_request(r: &mut SplitMix64, function: ReadFunction) -> ReadRequest {
    let max = function.max_quantity();
    let q = r.range_u64(1, u64::from(max)) as u16;
    let a = r.range_u64(0, 65_536 - u64::from(q)) as u16;
    ReadRequest::new(function, a, q).unwrap()
}

fn random_data(r: &mut SplitMix64, req: &ReadRequest) -> ReadData {
    let q = usize::from(req.quantity());
    if req.function().is_bit() {
        ReadData::Bits((0..q).map(|_| r.bool()).collect())
    } else {
        ReadData::Registers((0..q).map(|_| r.next_u16()).collect())
    }
}

#[test]
fn read_round_trip() {
    for function in ReadFunction::ALL {
        let name = format!("read_round_trip_{:02X}", function.code());
        check(&name, 2000, |r| {
            let req = random_request(r, function);
            assert_eq!(ReadRequest::decode(&req.encode()), Ok(req));
            let data = random_data(r, &req);
            let pdu = encode_read_response(&req, &data).unwrap();
            assert_eq!(decode_read_response(&req, &pdu), Ok(data.clone()));
            let unit = r.range_u64(1, 247) as u8;

            // Through an RTU ADU.
            let req_adu = rtu::encode_adu(unit, &req.encode()).unwrap();
            let (u, p) = rtu::decode_adu(&req_adu).unwrap();
            assert_eq!((u, ReadRequest::decode(p).unwrap()), (unit, req));
            let resp_adu = rtu::encode_adu(unit, &pdu).unwrap();
            assert_eq!(resp_adu.len(), req.rtu_response_len());
            let mut f = ResponseFramer::new(unit, function.code(), ExpectedReply::ByteCount);
            match f.push(&resp_adu) {
                FramerStatus::Complete(Ok(got)) => assert_eq!(decode_read_response(&req, &got), Ok(data.clone())),
                other => panic!("unexpected {other:?}"),
            }

            // Through an MBAP ADU.
            let tid = r.next_u16();
            let mut rx = MbapReceiver::new();
            rx.push(&tcp::encode_adu(tid, unit, &pdu).unwrap());
            let frame = rx.next_frame().unwrap().unwrap();
            assert_eq!((frame.transaction_id, frame.unit_id), (tid, unit));
            assert_eq!(decode_read_response(&req, &frame.pdu), Ok(data));
        });
    }
}

#[test]
fn fragmentation() {
    check("fragmentation", 500, |r| {
        let function = *r.pick(&ReadFunction::ALL).unwrap();
        let mut req = random_request(r, function);
        if req.quantity() > 40 {
            req = ReadRequest::new(function, req.address(), 40).unwrap();
        }
        let data = random_data(r, &req);
        let unit = r.range_u64(1, 247) as u8;
        let adu = rtu::encode_adu(unit, &encode_read_response(&req, &data).unwrap()).unwrap();
        let expected = FramerStatus::Complete(Ok(encode_read_response(&req, &data).unwrap()));

        // Every single split point.
        for split in 0..=adu.len() {
            let mut f = ResponseFramer::new(unit, function.code(), ExpectedReply::ByteCount);
            let first = f.push(&adu[..split]);
            if split < adu.len() {
                assert!(matches!(first, FramerStatus::NeedMore(_)), "split {split}: {first:?}");
            }
            assert_eq!(f.push(&adu[split..]), expected);
        }
        // Random USB-like chunking.
        let mut f = ResponseFramer::new(unit, function.code(), ExpectedReply::ByteCount);
        let mut pos = 0;
        let mut last = None;
        while pos < adu.len() {
            let n = r.range_usize(1, 64).min(adu.len() - pos);
            last = Some(f.push(&adu[pos..pos + n]));
            pos += n;
        }
        assert_eq!(last.unwrap(), expected);
    });
}

#[test]
fn no_panic_on_random_bytes() {
    check("no_panic", 20_000, |r| {
        let len = r.range_usize(0, 300);
        let bytes = r.bytes(len);
        let unit = r.next_u8();
        let fc = r.range_u64(1, 4) as u8;
        let expect = if r.bool() { ExpectedReply::ByteCount } else { ExpectedReply::Fixed(r.next_u8()) };
        let mut f = ResponseFramer::new(unit, fc, expect);
        let mut pos = 0;
        while pos < bytes.len() {
            let n = r.range_usize(1, 32).min(bytes.len() - pos);
            let _ = f.push(&bytes[pos..pos + n]);
            pos += n;
        }
        let _ = rtu::decode_adu(&bytes);
        let _ = ReadRequest::decode(&bytes);
        let mut rx = MbapReceiver::new();
        rx.push(&bytes);
        for _ in 0..4 {
            let _ = rx.next_frame();
        }
        if let Some(req) = ReadFunction::from_code(fc).and_then(|f| ReadRequest::new(f, 0, r.range_u64(1, 125) as u16).ok()) {
            let _ = decode_read_response(&req, &bytes);
        }
    });
}
