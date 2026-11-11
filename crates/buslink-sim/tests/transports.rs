//! Sim Req 2: core exchanger over the in-memory RTU bus with random chunking,
//! MBAP in memory, and the core TcpTransport against the loopback server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

mod common;

use buslink_core::clock::{Clock, ManualClock, SystemClock};
use buslink_core::protocol::pdu::ReadFunction;
use buslink_core::testkit::check;
use buslink_core::transport::exchange::{ExchangeError, Exchanger, Framing};
use buslink_core::transport::tcp::TcpTransport;
use buslink_sim::tcp::{MAX_CONNECTIONS, SimTcpServer};
use buslink_sim::{Chunking, SimTransport, SimWorld};
use common::*;
use std::sync::Arc;

#[test]
fn exchanger_over_fragmented_rtu_bus() {
    check("sim_rtu_chunking", 200, |r| {
        let clock = ManualClock::shared(0);
        let world = SimWorld::training(r.next_u64()).shared();
        let chunking = Chunking::Random {
            max: r.range_usize(1, 16),
            seed: r.next_u64(),
        };
        let mut t = SimTransport::rtu(world.clone(), clock.clone() as Arc<dyn Clock>, chunking);
        let mut ex = rtu_exchanger(&clock);
        for _ in 0..10 {
            clock.advance(r.range_u64(0, 2000));
            let soil = regs(read(&mut ex, &mut t, 1, ReadFunction::HoldingRegisters, 0, 8).unwrap());
            let direct = world.lock().unwrap().image(1).unwrap();
            assert_eq!(soil, (0..8).map(|a| direct.holding[&a]).collect::<Vec<_>>());
            assert_eq!(
                regs(read(&mut ex, &mut t, 4, ReadFunction::InputRegisters, 0, 16).unwrap()).len(),
                16
            );
            assert_eq!(
                read(&mut ex, &mut t, 1, ReadFunction::HoldingRegisters, 8, 2),
                Err(ExchangeError::Exception(
                    buslink_core::protocol::pdu::ExceptionCode::ILLEGAL_DATA_ADDRESS
                ))
            );
            assert_eq!(
                read(&mut ex, &mut t, 77, ReadFunction::HoldingRegisters, 0, 1),
                Err(ExchangeError::Timeout)
            );
        }
    });
}

#[test]
fn mbap_in_memory() {
    let clock = ManualClock::shared(0);
    let world = SimWorld::training(1).shared();
    let mut t = SimTransport::mbap(world, clock.clone() as Arc<dyn Clock>);
    let mut ex = Exchanger::new(Framing::Mbap, clock.clone() as Arc<dyn Clock>, 0);
    let v = regs(read(&mut ex, &mut t, 2, ReadFunction::HoldingRegisters, 0, 7).unwrap());
    assert_eq!(v[2], 50, "default ramp 5.0 s");
}

#[test]
fn core_tcp_client_against_loopback_server() {
    let clock = SystemClock::shared();
    let world = SimWorld::training(11).shared();
    let server = SimTcpServer::start_loopback(world, clock.clone(), 0).unwrap();
    let addr = server.local_addr();
    assert!(addr.ip().is_loopback(), "loopback by default");
    let mut t = TcpTransport::connect("127.0.0.1", addr.port(), 1000).unwrap();
    let mut ex = Exchanger::new(Framing::Mbap, clock, 0);
    for _ in 0..20 {
        assert_eq!(
            regs(read(&mut ex, &mut t, 1, ReadFunction::HoldingRegisters, 0, 8).unwrap()).len(),
            8
        );
        assert_eq!(
            regs(read(&mut ex, &mut t, 3, ReadFunction::HoldingRegisters, 0, 6).unwrap()).len(),
            6
        );
    }
    assert_eq!(
        read(&mut ex, &mut t, 1, ReadFunction::InputRegisters, 0, 1)
            .unwrap_err()
            .quality(),
        buslink_core::quality::Quality::Exception(1)
    );
    drop(t);
    server.stop();
}

#[test]
fn connection_cap_and_clean_stop() {
    let clock = SystemClock::shared();
    let server = SimTcpServer::start_loopback(SimWorld::training(1).shared(), clock.clone(), 0).unwrap();
    let port = server.local_addr().port();
    let mut conns: Vec<TcpTransport> = (0..MAX_CONNECTIONS)
        .map(|_| TcpTransport::connect("127.0.0.1", port, 1000).unwrap())
        .collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while server.connections() < MAX_CONNECTIONS && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(server.connections(), MAX_CONNECTIONS);
    // The ninth connection is accepted by the OS and closed by the server at once.
    let mut extra = TcpTransport::connect("127.0.0.1", port, 1000).unwrap();
    let mut ex = Exchanger::new(Framing::Mbap, clock.clone(), 0);
    let r = read(&mut ex, &mut extra, 1, ReadFunction::HoldingRegisters, 0, 1);
    assert!(matches!(r, Err(ExchangeError::Link(_))), "{r:?}");
    let mut ex = Exchanger::new(Framing::Mbap, clock, 0);
    assert!(read(&mut ex, &mut conns[0], 1, ReadFunction::HoldingRegisters, 0, 1).is_ok());
    let started = std::time::Instant::now();
    server.stop();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "stop joins promptly"
    );
    let mut ex = Exchanger::new(Framing::Mbap, SystemClock::shared(), 0);
    assert!(
        read(&mut ex, &mut conns[1], 1, ReadFunction::HoldingRegisters, 0, 1).is_err(),
        "connections are closed after stop"
    );
    conns.clear();
}
