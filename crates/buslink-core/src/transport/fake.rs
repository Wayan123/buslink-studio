//! Scripted transport for deterministic tests. Time comes from a
//! [`ManualClock`]: a read with nothing ready advances the clock by the
//! timeout instead of sleeping.

use super::{Transport, TransportError};
use crate::clock::{Clock, ManualClock};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// What the fake device does in response to one written frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeReply {
    /// Deliver `data` after `delay_ms`, split into pieces of `chunk` bytes (0 = whole).
    Bytes { data: Vec<u8>, delay_ms: u64, chunk: usize },
    /// No answer.
    Silence,
    /// The link goes away (USB detach).
    Disconnect,
}

impl FakeReply {
    pub fn now(data: Vec<u8>) -> Self {
        FakeReply::Bytes { data, delay_ms: 5, chunk: 0 }
    }
}

pub type Responder = Box<dyn FnMut(&[u8]) -> FakeReply + Send>;

impl State {
    /// Bytes are delivered in time order, as on a real line.
    fn queue(&mut self, at: u64, data: Vec<u8>) {
        let pos = self.pending.iter().position(|(t, _)| *t > at).unwrap_or(self.pending.len());
        self.pending.insert(pos, (at, data));
    }
}

struct State {
    responder: Responder,
    pending: VecDeque<(u64, Vec<u8>)>,
    writes: Vec<(u64, Vec<u8>)>,
    disconnected: bool,
    drained: usize,
}

pub struct FakeTransport {
    clock: Arc<ManualClock>,
    state: Arc<Mutex<State>>,
}

/// Test-side view of a [`FakeTransport`].
#[derive(Clone)]
pub struct FakeHandle {
    clock: Arc<ManualClock>,
    state: Arc<Mutex<State>>,
}

impl FakeTransport {
    pub fn new(clock: Arc<ManualClock>, responder: Responder) -> (FakeTransport, FakeHandle) {
        let state = Arc::new(Mutex::new(State {
            responder,
            pending: VecDeque::new(),
            writes: Vec::new(),
            disconnected: false,
            drained: 0,
        }));
        (FakeTransport { clock: Arc::clone(&clock), state: Arc::clone(&state) }, FakeHandle { clock, state })
    }

    /// Replies in order; silence once the script is exhausted.
    pub fn scripted(clock: Arc<ManualClock>, replies: Vec<FakeReply>) -> (FakeTransport, FakeHandle) {
        let mut queue: VecDeque<FakeReply> = replies.into();
        Self::new(clock, Box::new(move |_| queue.pop_front().unwrap_or(FakeReply::Silence)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl FakeHandle {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every frame written so far, with the monotonic time it was written.
    pub fn writes(&self) -> Vec<(u64, Vec<u8>)> {
        self.lock().writes.clone()
    }

    pub fn write_count(&self) -> usize {
        self.lock().writes.len()
    }

    /// Queue unsolicited bytes (noise, a late reply) that become readable after `delay_ms`.
    pub fn inject(&self, data: Vec<u8>, delay_ms: u64) {
        let at = self.clock.mono_ms() + delay_ms;
        self.lock().queue(at, data);
    }

    pub fn disconnect(&self) {
        self.lock().disconnected = true;
    }

    /// Bytes thrown away by `drain`.
    pub fn drained(&self) -> usize {
        self.lock().drained
    }

    pub fn set_responder(&self, responder: Responder) {
        self.lock().responder = responder;
    }
}

impl Transport for FakeTransport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let now = self.clock.mono_ms();
        let mut st = self.lock();
        if st.disconnected {
            return Err(TransportError::Disconnected("fake".into()));
        }
        st.writes.push((now, bytes.to_vec()));
        match (st.responder)(bytes) {
            FakeReply::Bytes { data, delay_ms, chunk } => {
                let at = now + delay_ms;
                if chunk == 0 {
                    st.queue(at, data);
                } else {
                    for piece in data.chunks(chunk) {
                        st.queue(at, piece.to_vec());
                    }
                }
            }
            FakeReply::Silence => {}
            FakeReply::Disconnect => st.disconnected = true,
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        let now = self.clock.mono_ms();
        let mut st = self.lock();
        if st.disconnected {
            return Err(TransportError::Disconnected("fake".into()));
        }
        let ready_at = st.pending.front().map(|(at, _)| *at);
        match ready_at {
            Some(at) if at <= now.saturating_add(timeout_ms) => {
                self.clock.advance_to(at);
                let Some((at, mut data)) = st.pending.pop_front() else { return Ok(0) };
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    st.pending.push_front((at, data.split_off(n)));
                }
                Ok(n)
            }
            _ => {
                self.clock.advance(timeout_ms.max(1));
                Ok(0)
            }
        }
    }

    fn drain(&mut self, quiet_ms: u64) -> Result<usize, TransportError> {
        let mut buf = [0u8; 256];
        let mut total = 0;
        for _ in 0..16 {
            let n = self.read(&mut buf, quiet_ms)?;
            if n == 0 {
                break;
            }
            total += n;
        }
        self.lock().drained += total;
        Ok(total)
    }
}
