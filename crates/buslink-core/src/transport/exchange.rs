//! One request/response over a transport: RTU (serial, RTU-over-TCP,
//! simulator bus) or MBAP. There is no "send any PDU" entry point: the bytes
//! come from a validated [`ReadRequest`] or a [`PreparedFrame`] that only the
//! command service can create.

use super::{Transport, TransportError};
use crate::clock::Clock;
use crate::command::PreparedFrame;
use crate::protocol::pdu::{EXCEPTION_FLAG, ExceptionCode, ExpectedReply, ReadRequest, exception_in};
use crate::protocol::rtu::{self, FramerStatus, ResponseFramer};
use crate::protocol::tcp::{self, MbapReceiver};
use crate::protocol::FrameError;
use crate::quality::Quality;
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// RTU ADU with CRC (serial lines, RTU over TCP, simulator bus).
    Rtu,
    /// Modbus TCP MBAP.
    Mbap,
}

/// The only two kinds of request the link can send.
#[derive(Clone, Copy, Debug)]
pub enum Outgoing<'a> {
    Read(&'a ReadRequest),
    Command(&'a PreparedFrame),
}

impl Outgoing<'_> {
    fn pdu(&self) -> Vec<u8> {
        match self {
            Outgoing::Read(r) => r.encode().to_vec(),
            Outgoing::Command(f) => f.pdu().to_vec(),
        }
    }

    fn expected(&self) -> ExpectedReply {
        match self {
            Outgoing::Read(r) => r.expected_reply(),
            Outgoing::Command(f) => f.expected_reply(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExchangeError {
    #[error("no response before the timeout")]
    Timeout,
    #[error("CRC error: {0}")]
    Crc(FrameError),
    #[error("{0}")]
    Exception(ExceptionCode),
    #[error("invalid response: {0}")]
    Decode(FrameError),
    #[error("{0}")]
    Link(TransportError),
    #[error("request could not be encoded: {0}")]
    Encode(FrameError),
}

impl ExchangeError {
    pub fn quality(&self) -> Quality {
        match self {
            ExchangeError::Timeout => Quality::Timeout,
            ExchangeError::Crc(_) => Quality::CrcError,
            ExchangeError::Exception(c) => Quality::Exception(c.0),
            ExchangeError::Decode(_) | ExchangeError::Encode(_) => Quality::DecodeError,
            ExchangeError::Link(_) => Quality::Disconnected,
        }
    }

    pub fn is_link_lost(&self) -> bool {
        matches!(self, ExchangeError::Link(_))
    }
}

/// Everything about one exchange, for the register inspector and the audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExchangeRecord {
    pub unit: u8,
    /// The ADU as sent.
    pub request: Vec<u8>,
    /// Every byte received while waiting for this response.
    pub response: Vec<u8>,
    pub started_mono_ms: u64,
    pub elapsed_ms: u64,
    /// Stale bytes drained before sending, plus frames/bytes ignored as not ours.
    pub discarded: usize,
    /// Reply PDU (function code first) or why there is none.
    pub outcome: Result<Vec<u8>, ExchangeError>,
}

/// After an RTU timeout, the next request first drains the line for this
/// long, so a reply that arrives late is discarded instead of being taken as
/// the answer to the next request. RTU has no transaction ID: a reply later
/// than this guard is only caught if its unit, function or length differ.
pub const DEFAULT_LATE_GUARD_MS: u64 = 200;

pub struct Exchanger {
    framing: Framing,
    clock: Arc<dyn Clock>,
    inter_frame_ms: u64,
    late_guard_ms: u64,
    after_timeout: bool,
    last_end_mono: Option<u64>,
    next_tid: u16,
    mbap: MbapReceiver,
}

impl Exchanger {
    pub fn new(framing: Framing, clock: Arc<dyn Clock>, inter_frame_ms: u64) -> Self {
        Self {
            framing,
            clock,
            inter_frame_ms,
            late_guard_ms: DEFAULT_LATE_GUARD_MS,
            after_timeout: false,
            last_end_mono: None,
            next_tid: 1,
            mbap: MbapReceiver::new(),
        }
    }

    pub fn set_late_guard_ms(&mut self, ms: u64) {
        self.late_guard_ms = ms;
    }

    pub fn framing(&self) -> Framing {
        self.framing
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// Send one request and wait for its response. Never retries.
    pub fn transact(&mut self, t: &mut dyn Transport, unit: u8, out: Outgoing<'_>, timeout_ms: u64) -> ExchangeRecord {
        let pdu = out.pdu();
        let function = pdu.first().copied().unwrap_or(0);
        let expect = out.expected();
        let mut record = match self.framing {
            Framing::Rtu => self.rtu(t, unit, &pdu, function, expect, timeout_ms),
            Framing::Mbap => self.mbap(t, unit, &pdu, function, expect, timeout_ms),
        };
        let end = self.clock.mono_ms();
        record.elapsed_ms = end.saturating_sub(record.started_mono_ms);
        self.last_end_mono = Some(end);
        self.after_timeout = matches!(record.outcome, Err(ExchangeError::Timeout));
        record
    }

    fn wait_gap(&self) {
        if let Some(last) = self.last_end_mono {
            let due = last + self.inter_frame_ms;
            let now = self.clock.mono_ms();
            if due > now {
                self.clock.sleep_ms(due - now);
            }
        }
    }

    fn empty(&self, unit: u8, request: Vec<u8>, outcome: Result<Vec<u8>, ExchangeError>) -> ExchangeRecord {
        ExchangeRecord {
            unit,
            request,
            response: Vec::new(),
            started_mono_ms: self.clock.mono_ms(),
            elapsed_ms: 0,
            discarded: 0,
            outcome,
        }
    }

    /// Read once; returns bytes read. Guards against a transport that returns
    /// immediately without data (busy loop) by sleeping 1 ms.
    fn read_some(&self, t: &mut dyn Transport, buf: &mut [u8], deadline: u64) -> Result<Option<usize>, TransportError> {
        let now = self.clock.mono_ms();
        if now >= deadline {
            return Ok(None);
        }
        let n = t.read(buf, deadline - now)?;
        if n == 0 && self.clock.mono_ms() == now {
            self.clock.sleep_ms(1);
        }
        Ok(Some(n))
    }

    fn rtu(
        &mut self,
        t: &mut dyn Transport,
        unit: u8,
        pdu: &[u8],
        function: u8,
        expect: ExpectedReply,
        timeout_ms: u64,
    ) -> ExchangeRecord {
        let adu = match rtu::encode_adu(unit, pdu) {
            Ok(a) => a,
            Err(e) => return self.empty(unit, Vec::new(), Err(ExchangeError::Encode(e))),
        };
        self.wait_gap();
        let quiet = if self.after_timeout { self.inter_frame_ms.max(self.late_guard_ms) } else { self.inter_frame_ms.max(2) };
        let discarded = match t.drain(quiet) {
            Ok(n) => n,
            Err(e) => return self.empty(unit, adu, Err(ExchangeError::Link(e))),
        };
        let started = self.clock.mono_ms();
        let mut rec = ExchangeRecord {
            unit,
            request: adu,
            response: Vec::new(),
            started_mono_ms: started,
            elapsed_ms: 0,
            discarded,
            outcome: Err(ExchangeError::Timeout),
        };
        if let Err(e) = t.write_all(&rec.request) {
            rec.outcome = Err(ExchangeError::Link(e));
            return rec;
        }
        let deadline = self.clock.mono_ms() + timeout_ms;
        let mut framer = ResponseFramer::new(unit, function, expect);
        let mut buf = [0u8; rtu::MAX_ADU_LEN];
        loop {
            let n = match self.read_some(t, &mut buf, deadline) {
                Ok(Some(n)) => n,
                Ok(None) => break,
                Err(e) => {
                    rec.outcome = Err(ExchangeError::Link(e));
                    break;
                }
            };
            if n == 0 {
                continue;
            }
            rec.response.extend_from_slice(&buf[..n]);
            if let FramerStatus::Complete(result) = framer.push(&buf[..n]) {
                rec.outcome = match result {
                    Ok(reply) => classify(function, reply),
                    Err(e @ FrameError::Crc { .. }) => Err(ExchangeError::Crc(e)),
                    Err(e) => Err(ExchangeError::Decode(e)),
                };
                rec.discarded += framer.trailing();
                break;
            }
        }
        rec
    }

    fn mbap(
        &mut self,
        t: &mut dyn Transport,
        unit: u8,
        pdu: &[u8],
        function: u8,
        expect: ExpectedReply,
        timeout_ms: u64,
    ) -> ExchangeRecord {
        let tid = self.next_tid;
        self.next_tid = self.next_tid.wrapping_add(1);
        let adu = match tcp::encode_adu(tid, unit, pdu) {
            Ok(a) => a,
            Err(e) => return self.empty(unit, Vec::new(), Err(ExchangeError::Encode(e))),
        };
        self.wait_gap();
        let started = self.clock.mono_ms();
        let mut rec = ExchangeRecord {
            unit,
            request: adu,
            response: Vec::new(),
            started_mono_ms: started,
            elapsed_ms: 0,
            discarded: 0,
            outcome: Err(ExchangeError::Timeout),
        };
        if let Err(e) = t.write_all(&rec.request) {
            rec.outcome = Err(ExchangeError::Link(e));
            return rec;
        }
        let deadline = self.clock.mono_ms() + timeout_ms;
        let mut buf = [0u8; tcp::MAX_TCP_ADU_LEN];
        'wait: loop {
            // Frames already buffered (coalesced or late) are looked at first.
            while let Some(frame) = self.mbap.next_frame() {
                match frame {
                    Err(e) => {
                        rec.outcome = Err(ExchangeError::Link(TransportError::Io(format!("MBAP stream out of sync: {e}"))));
                        break 'wait;
                    }
                    Ok(f) if f.transaction_id != tid => rec.discarded += 1, // a late reply to an older request
                    Ok(f) if f.unit_id != unit => {
                        rec.outcome = Err(ExchangeError::Decode(FrameError::WrongUnit { expected: unit, received: f.unit_id }));
                        break 'wait;
                    }
                    Ok(f) => {
                        rec.outcome = check_tcp_shape(function, expect, f.pdu);
                        break 'wait;
                    }
                }
            }
            match self.read_some(t, &mut buf, deadline) {
                Ok(Some(0)) => {}
                Ok(Some(n)) => {
                    rec.response.extend_from_slice(&buf[..n]);
                    self.mbap.push(&buf[..n]);
                }
                Ok(None) => break,
                Err(e) => {
                    rec.outcome = Err(ExchangeError::Link(e));
                    break;
                }
            }
        }
        rec
    }
}

fn classify(function: u8, reply: Vec<u8>) -> Result<Vec<u8>, ExchangeError> {
    match exception_in(function, &reply) {
        Some(code) => Err(ExchangeError::Exception(code)),
        None => Ok(reply),
    }
}

fn check_tcp_shape(function: u8, expect: ExpectedReply, pdu: Vec<u8>) -> Result<Vec<u8>, ExchangeError> {
    let received = pdu.first().copied().unwrap_or(0);
    if received == function | EXCEPTION_FLAG {
        if pdu.len() != 2 {
            return Err(ExchangeError::Decode(FrameError::Length { expected: 2, received: pdu.len() }));
        }
        return classify(function, pdu);
    }
    if received != function {
        return Err(ExchangeError::Decode(FrameError::UnexpectedFunction { expected: function, received }));
    }
    if let ExpectedReply::Fixed(n) = expect
        && pdu.len() != 1 + usize::from(n)
    {
        return Err(ExchangeError::Decode(FrameError::Length { expected: 1 + usize::from(n), received: pdu.len() }));
    }
    Ok(pdu)
}
