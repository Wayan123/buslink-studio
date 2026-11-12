//! USB-serial bridge for platforms where the port lives outside Rust
//! (Android: Kotlin plugin over usb-serial-for-android, architecture D2).
//!
//! USB delivers bytes in packets, not at character timing: one RTU reply can
//! arrive as several packets, and two replies (or a reply and noise) can
//! arrive as one. [`BufferedUsb`] turns a packet source ([`UsbPort`]) into the
//! byte-stream [`Transport`] the length-based RTU framer expects. It never
//! frames anything itself and never builds a request: `write_all` carries
//! bytes the poller or the command service already encoded.

use super::{Transport, TransportError};
use crate::clock::{Clock, ManualClock};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Bytes kept between `read` calls. Two maximum RTU frames fit several times;
/// more means nobody is reading (stale input), see [`BufferedUsb::overflowed`].
pub const DEFAULT_CAPACITY: usize = 4096;

/// A USB serial port seen as a source of packets.
pub trait UsbPort: Send {
    /// Send all bytes or fail.
    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Wait at most `timeout_ms` for the next packet. An empty vector means
    /// nothing arrived (a timeout, not end of stream). A detached device is
    /// `Err(Disconnected)`.
    fn read_packet(&mut self, timeout_ms: u64) -> Result<Vec<u8>, TransportError>;

    fn close(&mut self) {}
}

/// Byte-stream view of a [`UsbPort`].
pub struct BufferedUsb<P: UsbPort> {
    port: P,
    pending: VecDeque<u8>,
    capacity: usize,
    overflowed: u64,
    dead: Option<TransportError>,
}

impl<P: UsbPort> BufferedUsb<P> {
    pub fn new(port: P) -> Self {
        Self::with_capacity(port, DEFAULT_CAPACITY)
    }

    pub fn with_capacity(port: P, capacity: usize) -> Self {
        Self {
            port,
            pending: VecDeque::new(),
            capacity: capacity.max(1),
            overflowed: 0,
            dead: None,
        }
    }

    /// Bytes waiting for the next `read`.
    pub fn buffered(&self) -> usize {
        self.pending.len()
    }

    /// Total bytes dropped because the buffer was full (oldest first).
    pub fn overflowed(&self) -> u64 {
        self.overflowed
    }

    fn push(&mut self, packet: &[u8]) {
        // A packet larger than the whole buffer keeps only its newest bytes.
        let packet = if packet.len() > self.capacity {
            self.overflowed += (packet.len() - self.capacity) as u64;
            &packet[packet.len() - self.capacity..]
        } else {
            packet
        };
        let free = self.capacity - self.pending.len();
        if packet.len() > free {
            let drop = packet.len() - free;
            self.pending.drain(..drop);
            self.overflowed += drop as u64;
        }
        self.pending.extend(packet.iter().copied());
    }

    fn fail(&mut self, e: TransportError) -> TransportError {
        self.pending.clear();
        if matches!(e, TransportError::Disconnected(_)) {
            self.dead = Some(e.clone());
        }
        e
    }
}

impl<P: UsbPort> Transport for BufferedUsb<P> {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if let Some(e) = &self.dead {
            return Err(e.clone());
        }
        match self.port.write(bytes) {
            Ok(()) => Ok(()),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        if let Some(e) = &self.dead {
            return Err(e.clone());
        }
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pending.is_empty() {
            match self.port.read_packet(timeout_ms) {
                Ok(packet) if packet.is_empty() => return Ok(0),
                Ok(packet) => self.push(&packet),
                Err(e) => return Err(self.fail(e)),
            }
        }
        let n = buf.len().min(self.pending.len());
        for (slot, byte) in buf.iter_mut().zip(self.pending.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }

    fn close(&mut self) {
        self.pending.clear();
        if self.dead.is_none() {
            self.dead = Some(TransportError::Disconnected("port closed".into()));
        }
        self.port.close();
    }
}

/// Lowercase hex without separators, the wire format between Rust and the
/// Kotlin plugin (JSON carries no raw bytes).
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0F)]));
    }
    out
}

/// Inverse of [`to_hex`]; accepts either case, rejects odd length and non-hex.
pub fn from_hex(text: &str) -> Result<Vec<u8>, &'static str> {
    fn nibble(c: u8) -> Result<u8, &'static str> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err("not a hex digit"),
        }
    }
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(2) {
        return Err("odd number of hex digits");
    }
    raw.chunks_exact(2)
        .map(|pair| Ok((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

// ---------------------------------------------------------------------------
// Fake USB port for tests (no hardware, no sleeping).

/// What the fake device does after one written frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsbScript {
    /// Deliver these packets `delay_ms` after the write; each element is one USB packet.
    Packets {
        packets: Vec<Vec<u8>>,
        delay_ms: u64,
    },
    Silence,
    /// The adapter is unplugged.
    Detach,
}

pub type UsbResponder = Box<dyn FnMut(&[u8]) -> UsbScript + Send>;

struct FakeState {
    responder: UsbResponder,
    pending: VecDeque<(u64, Vec<u8>)>,
    writes: Vec<Vec<u8>>,
    detached: bool,
    closed: bool,
}

/// Scripted [`UsbPort`]. Time comes from a [`ManualClock`]: a read with no
/// packet ready advances the clock by the timeout.
pub struct FakeUsbPort {
    clock: Arc<ManualClock>,
    state: Arc<Mutex<FakeState>>,
}

/// Test-side view of a [`FakeUsbPort`].
#[derive(Clone)]
pub struct FakeUsbHandle {
    clock: Arc<ManualClock>,
    state: Arc<Mutex<FakeState>>,
}

impl FakeUsbPort {
    pub fn new(clock: Arc<ManualClock>, responder: UsbResponder) -> (FakeUsbPort, FakeUsbHandle) {
        let state = Arc::new(Mutex::new(FakeState {
            responder,
            pending: VecDeque::new(),
            writes: Vec::new(),
            detached: false,
            closed: false,
        }));
        (
            FakeUsbPort {
                clock: Arc::clone(&clock),
                state: Arc::clone(&state),
            },
            FakeUsbHandle { clock, state },
        )
    }

    /// One script per written frame, in order; silence afterwards.
    pub fn scripted(clock: Arc<ManualClock>, scripts: Vec<UsbScript>) -> (FakeUsbPort, FakeUsbHandle) {
        let mut queue: VecDeque<UsbScript> = scripts.into();
        Self::new(
            clock,
            Box::new(move |_| queue.pop_front().unwrap_or(UsbScript::Silence)),
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl FakeUsbHandle {
    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.lock().writes.clone()
    }

    /// Unsolicited packet (noise, a late reply) readable after `delay_ms`.
    pub fn inject(&self, packet: Vec<u8>, delay_ms: u64) {
        let at = self.clock.mono_ms() + delay_ms;
        self.lock().pending.push_back((at, packet));
    }

    pub fn detach(&self) {
        self.lock().detached = true;
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

impl UsbPort for FakeUsbPort {
    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let now = self.clock.mono_ms();
        let mut st = self.lock();
        if st.detached || st.closed {
            return Err(TransportError::Disconnected("USB adapter detached".into()));
        }
        st.writes.push(bytes.to_vec());
        match (st.responder)(bytes) {
            UsbScript::Packets { packets, delay_ms } => {
                for p in packets {
                    st.pending.push_back((now + delay_ms, p));
                }
            }
            UsbScript::Silence => {}
            UsbScript::Detach => st.detached = true,
        }
        Ok(())
    }

    fn read_packet(&mut self, timeout_ms: u64) -> Result<Vec<u8>, TransportError> {
        let now = self.clock.mono_ms();
        let mut st = self.lock();
        if st.detached || st.closed {
            return Err(TransportError::Disconnected("USB adapter detached".into()));
        }
        match st.pending.front().map(|(at, _)| *at) {
            Some(at) if at <= now.saturating_add(timeout_ms) => {
                self.clock.advance_to(at);
                Ok(st.pending.pop_front().map(|(_, p)| p).unwrap_or_default())
            }
            _ => {
                self.clock.advance(timeout_ms.max(1));
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self) {
        self.lock().closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(packets: Vec<Vec<u8>>) -> (BufferedUsb<FakeUsbPort>, FakeUsbHandle) {
        let clock = ManualClock::shared(0);
        let (port, h) = FakeUsbPort::scripted(clock, vec![]);
        for p in packets {
            h.inject(p, 0);
        }
        (BufferedUsb::new(port), h)
    }

    #[test]
    fn hex_round_trip_and_rejects_bad_input() {
        assert_eq!(to_hex(&[0x01, 0x03, 0xAB, 0x00]), "0103ab00");
        assert_eq!(from_hex("0103AB00"), Ok(vec![1, 3, 0xAB, 0]));
        assert_eq!(from_hex(""), Ok(vec![]));
        assert!(from_hex("abc").is_err());
        assert!(from_hex("zz").is_err());
        for len in 0..40u8 {
            let data: Vec<u8> = (0..len).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(from_hex(&to_hex(&data)), Ok(data));
        }
    }

    #[test]
    fn small_buffer_gets_one_packet_in_pieces_in_order() {
        let (mut u, _h) = rig(vec![vec![1, 2, 3, 4, 5]]);
        let mut b = [0u8; 2];
        assert_eq!(u.read(&mut b, 10), Ok(2));
        assert_eq!(b, [1, 2]);
        assert_eq!(u.buffered(), 3);
        assert_eq!(u.read(&mut b, 10), Ok(2));
        assert_eq!(b, [3, 4]);
        assert_eq!(u.read(&mut b, 10), Ok(1));
        assert_eq!(b[0], 5);
        assert_eq!(u.read(&mut b, 10), Ok(0));
    }

    #[test]
    fn coalesced_packets_are_served_without_loss() {
        let (mut u, _h) = rig(vec![vec![1, 2], vec![3, 4, 5]]);
        let mut out = Vec::new();
        let mut b = [0u8; 8];
        loop {
            let n = u.read(&mut b, 10).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&b[..n]);
        }
        assert_eq!(out, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn timeout_is_zero_bytes_not_an_error() {
        let (mut u, _h) = rig(vec![]);
        let mut b = [0u8; 8];
        assert_eq!(u.read(&mut b, 50), Ok(0));
        assert_eq!(u.read(&mut b, 50), Ok(0));
    }

    #[test]
    fn empty_destination_reads_nothing_and_keeps_data() {
        let (mut u, _h) = rig(vec![vec![9]]);
        assert_eq!(u.read(&mut [], 10), Ok(0));
        let mut b = [0u8; 1];
        assert_eq!(u.read(&mut b, 10), Ok(1));
        assert_eq!(b[0], 9);
    }

    #[test]
    fn detach_discards_buffer_and_stays_disconnected() {
        let (mut u, h) = rig(vec![vec![1, 2, 3]]);
        let mut b = [0u8; 1];
        assert_eq!(u.read(&mut b, 10), Ok(1));
        assert_eq!(u.buffered(), 2);
        h.detach();
        assert!(u.write_all(&[1]).is_err());
        assert!(matches!(u.read(&mut b, 10), Err(TransportError::Disconnected(_))));
        assert_eq!(u.buffered(), 0);
        assert!(matches!(u.read(&mut b, 10), Err(TransportError::Disconnected(_))));
        assert!(matches!(u.write_all(&[1]), Err(TransportError::Disconnected(_))));
    }

    #[test]
    fn overflow_drops_oldest_and_is_counted() {
        let (port, h) = FakeUsbPort::scripted(ManualClock::shared(0), vec![]);
        let mut v = BufferedUsb::with_capacity(port, 4);
        h.inject(vec![1, 2, 3, 4, 5, 6], 0);
        let mut one = [0u8; 1];
        assert_eq!(v.read(&mut one, 10), Ok(1));
        assert_eq!(one[0], 3, "a packet larger than the buffer keeps its newest bytes");
        assert_eq!(v.overflowed(), 2);
        assert_eq!(v.buffered(), 3);
    }

    #[test]
    fn close_stops_further_io() {
        let (mut u, h) = rig(vec![vec![1]]);
        u.close();
        assert!(h.is_closed());
        let mut b = [0u8; 1];
        assert!(u.read(&mut b, 1).is_err());
        assert!(u.write_all(&[0]).is_err());
    }

    #[test]
    fn drain_reads_until_quiet_through_the_buffer() {
        let (mut u, _h) = rig(vec![vec![0xAA; 300], vec![0x55; 20]]);
        assert_eq!(u.drain(5), Ok(320));
        assert_eq!(u.buffered(), 0);
    }
}
