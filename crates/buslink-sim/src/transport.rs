//! In-memory transport to a simulated bus, implementing the core `Transport`.
//! RTU requests are framed by function code; replies can be split into
//! seeded random chunks to mimic USB packet fragmentation.

use crate::world::SharedWorld;
use buslink_core::clock::Clock;
use buslink_core::protocol::tcp::{self, MbapReceiver};
use buslink_core::rng::SplitMix64;
use buslink_core::transport::{Transport, TransportError};
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chunking {
    Whole,
    /// Split every reply into pieces of 1..=max bytes.
    Random {
        max: usize,
        seed: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Rtu,
    Mbap,
}

pub struct SimTransport {
    world: SharedWorld,
    clock: Arc<dyn Clock>,
    mode: Mode,
    chunk_max: usize,
    rng: SplitMix64,
    rx: Vec<u8>,
    mbap: MbapReceiver,
    pending: VecDeque<(u64, Vec<u8>)>,
    closed: bool,
    /// Line speed of an RTU link; `None` = devices answer at any baud.
    baud: Option<u32>,
}

/// Length of the RTU request starting at `buf[0]`, if it can be known yet.
/// Unknown function codes take the whole buffer (the slave then ignores it on CRC).
fn rtu_request_len(buf: &[u8]) -> Option<usize> {
    let fc = *buf.get(1)?;
    match fc {
        0x01..=0x06 => Some(8),
        0x0F | 0x10 => buf.get(6).map(|&n| 9 + usize::from(n)),
        _ => Some(buf.len()),
    }
}

impl SimTransport {
    fn new(world: SharedWorld, clock: Arc<dyn Clock>, mode: Mode, chunking: Chunking) -> Self {
        let (chunk_max, seed) = match chunking {
            Chunking::Whole => (0, 0),
            Chunking::Random { max, seed } => (max.max(1), seed),
        };
        Self {
            world,
            clock,
            mode,
            chunk_max,
            rng: SplitMix64::new(seed),
            rx: Vec::new(),
            mbap: MbapReceiver::new(),
            pending: VecDeque::new(),
            closed: false,
            baud: None,
        }
    }

    /// A simulated RS485 bus with RTU framing.
    pub fn rtu(world: SharedWorld, clock: Arc<dyn Clock>, chunking: Chunking) -> Self {
        Self::new(world, clock, Mode::Rtu, chunking)
    }

    /// A simulated RS485 bus running at `baud`: devices set to another baud stay silent.
    pub fn rtu_at(world: SharedWorld, clock: Arc<dyn Clock>, chunking: Chunking, baud: u32) -> Self {
        let mut t = Self::new(world, clock, Mode::Rtu, chunking);
        t.baud = Some(baud);
        t
    }

    /// A simulated Modbus TCP connection (MBAP framing).
    pub fn mbap(world: SharedWorld, clock: Arc<dyn Clock>) -> Self {
        Self::new(world, clock, Mode::Mbap, Chunking::Whole)
    }

    fn queue(&mut self, ready_at: u64, data: Vec<u8>) {
        let pieces: Vec<Vec<u8>> = if self.chunk_max == 0 {
            vec![data]
        } else {
            let mut out = Vec::new();
            let mut rest = &data[..];
            while !rest.is_empty() {
                let n = self.rng.range_usize(1, self.chunk_max).min(rest.len());
                out.push(rest[..n].to_vec());
                rest = &rest[n..];
            }
            out
        };
        for p in pieces {
            let pos = self
                .pending
                .iter()
                .position(|(t, _)| *t > ready_at)
                .unwrap_or(self.pending.len());
            self.pending.insert(pos, (ready_at, p));
        }
    }

    fn world_handle_rtu(&mut self, frame: &[u8], now: u64) -> Option<(Vec<u8>, u64)> {
        self.world
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .handle_rtu_at(frame, now, self.baud)
    }
}

impl Transport for SimTransport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected("simulator transport closed".into()));
        }
        let now = self.clock.mono_ms();
        match self.mode {
            Mode::Rtu => {
                self.rx.extend_from_slice(bytes);
                while let Some(n) = rtu_request_len(&self.rx) {
                    if self.rx.len() < n {
                        break;
                    }
                    let frame: Vec<u8> = self.rx.drain(..n).collect();
                    if let Some((reply, delay)) = self.world_handle_rtu(&frame, now) {
                        self.queue(now + delay, reply);
                    }
                }
            }
            Mode::Mbap => {
                self.mbap.push(bytes);
                while let Some(frame) = self.mbap.next_frame() {
                    let f = frame.map_err(|e| TransportError::Disconnected(format!("MBAP stream broken: {e}")))?;
                    let reply = self
                        .world
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .handle_pdu(f.unit_id, &f.pdu, now);
                    if let Some(mut pdu) = reply.pdu {
                        if reply.corrupt && pdu.len() > 1 {
                            pdu[1] ^= 0xFF;
                        }
                        if let Ok(adu) = tcp::encode_adu(f.transaction_id, f.unit_id, &pdu) {
                            self.queue(now + reply.delay_ms, adu);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected("simulator transport closed".into()));
        }
        let now = self.clock.mono_ms();
        match self.pending.front().map(|(t, _)| *t) {
            Some(at) if at <= now.saturating_add(timeout_ms) => {
                if at > now {
                    self.clock.sleep_ms(at - now);
                }
                let Some((at, mut data)) = self.pending.pop_front() else {
                    return Ok(0);
                };
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    self.pending.push_front((at, data.split_off(n)));
                }
                Ok(n)
            }
            _ => {
                self.clock.sleep_ms(timeout_ms.max(1));
                Ok(0)
            }
        }
    }

    fn close(&mut self) {
        self.closed = true;
        self.pending.clear();
    }
}
