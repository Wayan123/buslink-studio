//! RTU exchanges over the buffered USB bridge with a fake USB port (android-otg Req 4, 5).
//! Read requests only: no write frame is built here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use buslink_core::clock::{Clock, ManualClock};
use buslink_core::protocol::pdu::{ReadData, ReadFunction, ReadRequest, decode_read_response};
use buslink_core::protocol::rtu;
use buslink_core::quality::Quality;
use buslink_core::transport::exchange::{ExchangeError, Exchanger, Framing, Outgoing};
use buslink_core::transport::usb::{BufferedUsb, FakeUsbPort, UsbScript};
use buslink_core::transport::{Transport, TransportError};
use std::sync::Arc;

const SOIL: [u16; 8] = [253, 312, 480, 260, 35, 12, 80, 680];

fn reply(unit: u8) -> Vec<u8> {
    let mut pdu = vec![0x03, 16];
    for r in SOIL {
        pdu.extend_from_slice(&r.to_be_bytes());
    }
    rtu::encode_adu(unit, &pdu).unwrap()
}

fn request() -> ReadRequest {
    ReadRequest::new(ReadFunction::HoldingRegisters, 0, 8).unwrap()
}

fn exchanger(clock: &Arc<ManualClock>) -> Exchanger {
    Exchanger::new(Framing::Rtu, clock.clone() as Arc<dyn Clock>, 50)
}

/// Cut `data` into packets at the given split points.
fn cut(data: &[u8], at: &[usize]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut from = 0;
    for &i in at {
        out.push(data[from..i].to_vec());
        from = i;
    }
    out.push(data[from..].to_vec());
    out
}

fn read_once(packets: Vec<Vec<u8>>) -> buslink_core::transport::exchange::ExchangeRecord {
    let clock = ManualClock::shared(0);
    let (port, h) = FakeUsbPort::scripted(clock.clone(), vec![UsbScript::Packets { packets, delay_ms: 20 }]);
    let mut t = BufferedUsb::new(port);
    let rec = exchanger(&clock).transact(&mut t, 1, Outgoing::Read(&request()), 1000);
    assert_eq!(h.writes()[0], vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x08, 0x44, 0x0C]);
    rec
}

#[test]
fn reply_split_at_every_byte_position_decodes() {
    let full = reply(1);
    for split in 1..full.len() {
        let rec = read_once(cut(&full, &[split]));
        let pdu = rec.outcome.unwrap_or_else(|e| panic!("split {split}: {e}"));
        assert_eq!(
            decode_read_response(&request(), &pdu).unwrap(),
            ReadData::Registers(SOIL.to_vec()),
            "split {split}"
        );
    }
}

#[test]
fn reply_delivered_one_byte_per_packet_decodes() {
    let full = reply(1);
    let packets: Vec<Vec<u8>> = full.iter().map(|b| vec![*b]).collect();
    let rec = read_once(packets);
    assert!(rec.outcome.is_ok(), "{:?}", rec.outcome);
    assert_eq!(rec.response, full);
}

#[test]
fn reply_and_trailing_bytes_in_one_packet_do_not_break_the_next_exchange() {
    // First packet: the whole reply plus 3 stray bytes. The next exchange drains them.
    let clock = ManualClock::shared(0);
    let mut first = reply(1);
    first.extend_from_slice(&[0xAA, 0x55, 0x01]);
    let (port, _h) = FakeUsbPort::scripted(
        clock.clone(),
        vec![
            UsbScript::Packets {
                packets: vec![first],
                delay_ms: 10,
            },
            UsbScript::Packets {
                packets: vec![reply(1)],
                delay_ms: 10,
            },
        ],
    );
    let mut t = BufferedUsb::new(port);
    let mut ex = exchanger(&clock);
    let r1 = ex.transact(&mut t, 1, Outgoing::Read(&request()), 1000);
    assert!(r1.outcome.is_ok(), "{:?}", r1.outcome);
    let r2 = ex.transact(&mut t, 1, Outgoing::Read(&request()), 1000);
    assert!(r2.outcome.is_ok(), "{:?}", r2.outcome);
    assert_eq!(r2.response.len(), 21, "stray bytes never leak into the next reply");
}

#[test]
fn silence_is_a_timeout_quality_not_an_error_value() {
    let clock = ManualClock::shared(0);
    let (port, _h) = FakeUsbPort::scripted(clock.clone(), vec![UsbScript::Silence]);
    let mut t = BufferedUsb::new(port);
    let rec = exchanger(&clock).transact(&mut t, 1, Outgoing::Read(&request()), 300);
    let err = rec.outcome.unwrap_err();
    assert_eq!(err, ExchangeError::Timeout);
    assert_eq!(err.quality(), Quality::Timeout);
}

#[test]
fn bad_crc_in_fragments_is_crc_error() {
    let mut full = reply(1);
    let n = full.len();
    full[n - 1] ^= 0xFF;
    let rec = read_once(cut(&full, &[4, 11]));
    assert_eq!(rec.outcome.unwrap_err().quality(), Quality::CrcError);
}

#[test]
fn detach_mid_reply_is_a_link_loss() {
    let clock = ManualClock::shared(0);
    let (port, h) = FakeUsbPort::scripted(clock.clone(), vec![UsbScript::Detach]);
    let mut t = BufferedUsb::new(port);
    let rec = exchanger(&clock).transact(&mut t, 1, Outgoing::Read(&request()), 300);
    let err = rec.outcome.unwrap_err();
    assert!(err.is_link_lost(), "{err:?}");
    let mut b = [0u8; 4];
    assert!(matches!(t.read(&mut b, 1), Err(TransportError::Disconnected(_))));
    assert!(matches!(t.write_all(&[1]), Err(TransportError::Disconnected(_))));
    // Reattach is a new port object; the old one never resumes by itself.
    assert!(h.is_closed() || h.writes().len() <= 1);
}

#[test]
fn late_reply_after_timeout_is_not_matched_to_the_next_request() {
    let clock = ManualClock::shared(0);
    let (port, _h) = FakeUsbPort::scripted(
        clock.clone(),
        vec![
            // Reply 400 ms after a 200 ms timeout.
            UsbScript::Packets {
                packets: vec![reply(1)],
                delay_ms: 400,
            },
            UsbScript::Packets {
                packets: vec![reply(1)],
                delay_ms: 10,
            },
        ],
    );
    let mut t = BufferedUsb::new(port);
    let mut ex = exchanger(&clock);
    let r1 = ex.transact(&mut t, 1, Outgoing::Read(&request()), 200);
    assert_eq!(r1.outcome.unwrap_err(), ExchangeError::Timeout);
    let r2 = ex.transact(&mut t, 1, Outgoing::Read(&request()), 1000);
    assert!(r2.outcome.is_ok(), "{:?}", r2.outcome);
    assert_eq!(r2.response.len(), 21, "exactly one reply, not the late one glued on");
}
