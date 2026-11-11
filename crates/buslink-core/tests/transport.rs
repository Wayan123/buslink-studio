//! Exchanger over FakeTransport and TcpTransport (Req 2, 4, 11).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::clock::{Clock, ManualClock, SystemClock};
use buslink_core::protocol::pdu::{ExceptionCode, ReadData, ReadFunction, ReadRequest, decode_read_response};
use buslink_core::protocol::{FrameError, rtu, tcp};
use buslink_core::quality::Quality;
use buslink_core::testkit::{hex, unhex};
use buslink_core::transport::exchange::{ExchangeError, Exchanger, Framing, Outgoing};
use buslink_core::transport::fake::{FakeReply, FakeTransport};
use buslink_core::transport::tcp::TcpTransport;
use buslink_core::transport::{Transport, TransportError};
use std::io::{Read, Write};
use std::sync::Arc;

fn soil_request() -> ReadRequest {
    ReadRequest::new(ReadFunction::HoldingRegisters, 0, 8).unwrap()
}

/// RTU response for unit/FC03 with these registers.
fn regs_reply(unit: u8, regs: &[u16]) -> Vec<u8> {
    let mut pdu = vec![0x03, (regs.len() * 2) as u8];
    for r in regs {
        pdu.extend_from_slice(&r.to_be_bytes());
    }
    rtu::encode_adu(unit, &pdu).unwrap()
}

const SOIL: [u16; 8] = [253, 312, 480, 260, 35, 12, 80, 680];

fn rtu_exchanger(clock: &Arc<ManualClock>) -> Exchanger {
    Exchanger::new(Framing::Rtu, clock.clone() as Arc<dyn Clock>, 5)
}

#[test]
fn soil_read_with_usb_fragmentation() {
    let clock = ManualClock::shared(1_000_000);
    let (mut t, h) = FakeTransport::scripted(
        clock.clone(),
        vec![FakeReply::Bytes {
            data: regs_reply(1, &SOIL),
            delay_ms: 30,
            chunk: 3,
        }],
    );
    let mut ex = rtu_exchanger(&clock);
    let req = soil_request();
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&req), 1000);
    assert_eq!(hex(&rec.request), "01 03 00 00 00 08 44 0C");
    assert_eq!(h.writes()[0].1, unhex("01 03 00 00 00 08 44 0C"));
    let pdu = rec.outcome.unwrap();
    assert_eq!(
        decode_read_response(&req, &pdu).unwrap(),
        ReadData::Registers(SOIL.to_vec())
    );
    assert_eq!(rec.response.len(), 21);
}

#[test]
fn stale_input_is_drained_before_the_request() {
    let clock = ManualClock::shared(0);
    let (mut t, h) = FakeTransport::scripted(clock.clone(), vec![FakeReply::now(regs_reply(1, &SOIL))]);
    h.inject(vec![0xAA, 0x55, 0x01], 0);
    let mut ex = rtu_exchanger(&clock);
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&soil_request()), 1000);
    assert!(rec.outcome.is_ok(), "{:?}", rec.outcome);
    assert_eq!(rec.discarded, 3);
    assert_eq!(h.drained(), 3);
}

#[test]
fn inter_frame_gap_is_kept() {
    let clock = ManualClock::shared(0);
    let (mut t, h) = FakeTransport::new(clock.clone(), Box::new(|_| FakeReply::now(regs_reply(1, &SOIL))));
    let mut ex = rtu_exchanger(&clock);
    let r1 = ex.transact(&mut t, 1, Outgoing::Read(&soil_request()), 1000);
    let end1 = r1.started_mono_ms + r1.elapsed_ms;
    ex.transact(&mut t, 1, Outgoing::Read(&soil_request()), 1000);
    let w = h.writes();
    assert!(
        w[1].0 >= end1 + 5,
        "second request at {} must wait ≥ 5 ms after {end1}",
        w[1].0
    );
}

#[test]
fn silence_is_timeout() {
    let clock = ManualClock::shared(0);
    let (mut t, _) = FakeTransport::scripted(clock.clone(), vec![FakeReply::Silence]);
    let mut ex = rtu_exchanger(&clock);
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&soil_request()), 800);
    assert_eq!(rec.outcome, Err(ExchangeError::Timeout));
    assert_eq!(rec.outcome.unwrap_err().quality(), Quality::Timeout);
    assert!((800..=900).contains(&rec.elapsed_ms), "elapsed {}", rec.elapsed_ms);
}

#[test]
fn late_reply_is_never_matched_to_the_next_request() {
    let clock = ManualClock::shared(0);
    let late = regs_reply(1, &[9999; 8]);
    let fresh = regs_reply(1, &SOIL);
    let (mut t, h) = FakeTransport::scripted(clock.clone(), vec![FakeReply::Silence, FakeReply::now(fresh)]);
    let mut ex = rtu_exchanger(&clock);
    let req = soil_request();
    let r1 = ex.transact(&mut t, 1, Outgoing::Read(&req), 500);
    assert_eq!(r1.outcome, Err(ExchangeError::Timeout));
    // The device answers the first request 60 ms after the timeout.
    h.inject(late, 60);
    let r2 = ex.transact(&mut t, 1, Outgoing::Read(&req), 500);
    let data = decode_read_response(&req, &r2.outcome.unwrap()).unwrap();
    assert_eq!(
        data,
        ReadData::Registers(SOIL.to_vec()),
        "the late reply must be discarded"
    );
    assert_eq!(r2.discarded, 21);
}

#[test]
fn exception_bad_crc_and_wrong_unit() {
    let clock = ManualClock::shared(0);
    let mut bad = regs_reply(1, &SOIL);
    let last = bad.len() - 1;
    bad[last] ^= 0x40;
    let (mut t, _) = FakeTransport::scripted(
        clock.clone(),
        vec![
            FakeReply::now(rtu::encode_adu(1, &[0x83, 0x02]).unwrap()),
            FakeReply::now(bad),
            FakeReply::now(regs_reply(7, &SOIL)),
        ],
    );
    let mut ex = rtu_exchanger(&clock);
    let req = soil_request();
    let e1 = ex.transact(&mut t, 1, Outgoing::Read(&req), 500).outcome.unwrap_err();
    assert_eq!(e1, ExchangeError::Exception(ExceptionCode::ILLEGAL_DATA_ADDRESS));
    assert_eq!(e1.quality(), Quality::Exception(2));
    let e2 = ex.transact(&mut t, 1, Outgoing::Read(&req), 500).outcome.unwrap_err();
    assert!(matches!(e2, ExchangeError::Crc(_)));
    assert_eq!(e2.quality(), Quality::CrcError);
    let e3 = ex.transact(&mut t, 1, Outgoing::Read(&req), 500).outcome.unwrap_err();
    assert_eq!(
        e3,
        ExchangeError::Decode(FrameError::WrongUnit {
            expected: 1,
            received: 7
        })
    );
    assert_eq!(e3.quality(), Quality::DecodeError);
}

#[test]
fn disconnect_is_link_error() {
    let clock = ManualClock::shared(0);
    let (mut t, h) = FakeTransport::scripted(clock.clone(), vec![FakeReply::Disconnect]);
    let mut ex = rtu_exchanger(&clock);
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&soil_request()), 500);
    assert!(matches!(
        rec.outcome,
        Err(ExchangeError::Link(TransportError::Disconnected(_)))
    ));
    assert!(rec.outcome.unwrap_err().is_link_lost());
    assert!(h.write_count() == 1);
}

fn mbap_reply(tid: u16, unit: u8, regs: &[u16]) -> Vec<u8> {
    let mut pdu = vec![0x03, (regs.len() * 2) as u8];
    for r in regs {
        pdu.extend_from_slice(&r.to_be_bytes());
    }
    tcp::encode_adu(tid, unit, &pdu).unwrap()
}

#[test]
fn mbap_matches_transaction_ids_and_discards_stale_frames() {
    let clock = ManualClock::shared(0);
    let (mut t, h) = FakeTransport::new(
        clock.clone(),
        Box::new(|req: &[u8]| {
            let tid = u16::from_be_bytes([req[0], req[1]]);
            // A stale reply for an older transaction arrives first, coalesced with ours.
            let mut data = mbap_reply(tid.wrapping_sub(1), 1, &[1, 1]);
            data.extend(mbap_reply(tid, 1, &[0x1234, 0x5678]));
            FakeReply::Bytes {
                data,
                delay_ms: 2,
                chunk: 5,
            }
        }),
    );
    let mut ex = Exchanger::new(Framing::Mbap, clock.clone() as Arc<dyn Clock>, 0);
    let req = ReadRequest::new(ReadFunction::HoldingRegisters, 10, 2).unwrap();
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&req), 500);
    assert_eq!(
        decode_read_response(&req, &rec.outcome.unwrap()).unwrap(),
        ReadData::Registers(vec![0x1234, 0x5678])
    );
    assert_eq!(rec.discarded, 1);
    let rec2 = ex.transact(&mut t, 1, Outgoing::Read(&req), 500);
    assert!(rec2.outcome.is_ok());
    let w = h.writes();
    assert_ne!(w[0].1[..2], w[1].1[..2], "transaction IDs advance");
}

#[test]
fn mbap_bad_header_is_link_error() {
    let clock = ManualClock::shared(0);
    let (mut t, _) = FakeTransport::scripted(clock.clone(), vec![FakeReply::now(vec![0, 1, 0, 7, 0, 3, 1, 3, 0])]);
    let mut ex = Exchanger::new(Framing::Mbap, clock.clone() as Arc<dyn Clock>, 0);
    let req = ReadRequest::new(ReadFunction::HoldingRegisters, 0, 1).unwrap();
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&req), 500);
    assert!(matches!(rec.outcome, Err(ExchangeError::Link(_))), "{:?}", rec.outcome);
}

#[test]
fn tcp_client_against_loopback_server() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut req = [0u8; 12];
        s.read_exact(&mut req).unwrap();
        let tid = u16::from_be_bytes([req[0], req[1]]);
        let reply = mbap_reply(tid, req[6], &[42]);
        // Split the reply to exercise partial reads.
        s.write_all(&reply[..4]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.write_all(&reply[4..]).unwrap();
        let mut rest = [0u8; 1];
        let _ = s.read(&mut rest);
    });
    let mut t = TcpTransport::connect("127.0.0.1", port, 1000).unwrap();
    let mut ex = Exchanger::new(Framing::Mbap, SystemClock::shared(), 0);
    let req = ReadRequest::new(ReadFunction::HoldingRegisters, 0, 1).unwrap();
    let rec = ex.transact(&mut t, 1, Outgoing::Read(&req), 2000);
    assert_eq!(
        decode_read_response(&req, &rec.outcome.unwrap()).unwrap(),
        ReadData::Registers(vec![42])
    );
    t.close();
    server.join().unwrap();
}

#[test]
fn tcp_connect_refused_is_an_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert!(TcpTransport::connect("127.0.0.1", port, 500).is_err());
}
