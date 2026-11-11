//! Modbus TCP training server for the simulated devices.
//!
//! Binds loopback unless the caller passes another address explicitly, caps
//! connections (8), frames by MBAP length (≤ 260 bytes per ADU) and stops
//! cleanly. Accepted sockets are switched back to blocking mode because on
//! Windows they inherit non-blocking mode from the listener.

use crate::world::SharedWorld;
use buslink_core::clock::Clock;
use buslink_core::protocol::tcp::{self, MbapReceiver};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub const MAX_CONNECTIONS: usize = 8;

pub struct SimTcpServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    accept: Option<JoinHandle<()>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

struct ConnGuard(Arc<AtomicUsize>);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl SimTcpServer {
    /// Bind `127.0.0.1:port` (0 = any free port).
    pub fn start_loopback(world: SharedWorld, clock: Arc<dyn Clock>, port: u16) -> std::io::Result<Self> {
        Self::start(
            world,
            clock,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)),
        )
    }

    /// Bind an explicit address. Callers outside training mode must choose it deliberately.
    pub fn start(world: SharedWorld, clock: Arc<dyn Clock>, addr: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let workers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let (s, c, w) = (stop.clone(), connections.clone(), workers.clone());
        let accept = std::thread::Builder::new()
            .name("buslink-sim-tcp".into())
            .spawn(move || {
                while !s.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if c.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                                c.fetch_sub(1, Ordering::SeqCst);
                                drop(stream);
                                continue;
                            }
                            let guard = ConnGuard(c.clone());
                            let (world, clock, stop) = (world.clone(), clock.clone(), s.clone());
                            if let Ok(h) = std::thread::Builder::new()
                                .name("buslink-sim-conn".into())
                                .spawn(move || serve(stream, world, clock, stop, guard))
                            {
                                w.lock().unwrap_or_else(|e| e.into_inner()).push(h);
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
            })?;
        Ok(Self {
            addr,
            stop,
            connections,
            accept: Some(accept),
            workers,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// Close the listener and every connection, and join all threads.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *self.workers.lock().unwrap_or_else(|e| e.into_inner()));
        for h in handles {
            let _ = h.join();
        }
    }
}

impl Drop for SimTcpServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn serve(mut stream: TcpStream, world: SharedWorld, clock: Arc<dyn Clock>, stop: Arc<AtomicBool>, _guard: ConnGuard) {
    if stream.set_nonblocking(false).is_err() || stream.set_read_timeout(Some(Duration::from_millis(50))).is_err() {
        return;
    }
    let _ = stream.set_nodelay(true);
    let mut rx = MbapReceiver::new();
    let mut buf = [0u8; tcp::MAX_TCP_ADU_LEN];
    while !stop.load(Ordering::SeqCst) {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => rx.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(_) => return,
        }
        while let Some(frame) = rx.next_frame() {
            let Ok(f) = frame else { return }; // unframeable stream: drop the connection
            let reply = world
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .handle_pdu(f.unit_id, &f.pdu, clock.mono_ms());
            let Some(mut pdu) = reply.pdu else { continue };
            if reply.corrupt && pdu.len() > 1 {
                pdu[1] ^= 0xFF;
            }
            if reply.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(reply.delay_ms));
            }
            let Ok(adu) = tcp::encode_adu(f.transaction_id, f.unit_id, &pdu) else {
                continue;
            };
            if stream.write_all(&adu).is_err() {
                return;
            }
        }
    }
}
