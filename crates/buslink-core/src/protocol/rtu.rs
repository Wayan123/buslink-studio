//! Modbus RTU: ADU encoding, length-based response framing, serial timing.
//!
//! Over USB-serial, bytes arrive in USB packets rather than at character
//! timing, so responses are framed by the length implied by the function
//! code and byte count, never by silence alone.

use super::pdu::{EXCEPTION_FLAG, ExpectedReply, MAX_PDU_LEN};
use super::{FrameError, crc};
use serde::{Deserialize, Serialize};

pub const MAX_ADU_LEN: usize = 256;

/// `unit | pdu | crc_lo crc_hi`.
pub fn encode_adu(unit: u8, pdu: &[u8]) -> Result<Vec<u8>, FrameError> {
    if pdu.is_empty() {
        return Err(FrameError::EmptyPdu);
    }
    if pdu.len() > MAX_PDU_LEN {
        return Err(FrameError::TooLong(pdu.len() + 3));
    }
    let mut adu = Vec::with_capacity(pdu.len() + 3);
    adu.push(unit);
    adu.extend_from_slice(pdu);
    crc::append(&mut adu);
    Ok(adu)
}

/// Split a complete ADU into unit and PDU after checking length and CRC.
pub fn decode_adu(adu: &[u8]) -> Result<(u8, &[u8]), FrameError> {
    if adu.len() < 4 {
        return Err(FrameError::TooShort(adu.len()));
    }
    if adu.len() > MAX_ADU_LEN {
        return Err(FrameError::TooLong(adu.len()));
    }
    let Some((body, received)) = crc::split(adu) else {
        return Err(FrameError::TooShort(adu.len()));
    };
    let expected = crc::crc16(body);
    if expected != received {
        return Err(FrameError::Crc { expected, received });
    }
    Ok((body[0], &body[1..]))
}

/// Progress of a [`ResponseFramer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramerStatus {
    /// At least this many more bytes are needed.
    NeedMore(usize),
    /// The frame is complete. `Ok` holds the PDU (function code first,
    /// possibly an exception PDU); `Err` says why it is not acceptable.
    Complete(Result<Vec<u8>, FrameError>),
}

/// Accumulates response bytes for one outstanding request.
#[derive(Debug, Clone)]
pub struct ResponseFramer {
    unit: u8,
    function: u8,
    expect: ExpectedReply,
    buf: Vec<u8>,
    needed: Option<usize>,
    trailing: usize,
    done: Option<Result<Vec<u8>, FrameError>>,
}

impl ResponseFramer {
    pub fn new(unit: u8, function: u8, expect: ExpectedReply) -> Self {
        Self { unit, function, expect, buf: Vec::with_capacity(MAX_ADU_LEN), needed: None, trailing: 0, done: None }
    }

    /// Bytes received so far for this frame (including any trailing bytes).
    pub fn received(&self) -> &[u8] {
        &self.buf
    }

    /// Bytes that arrived after the frame was complete.
    pub fn trailing(&self) -> usize {
        self.trailing
    }

    fn total_len(&self) -> Result<Option<usize>, FrameError> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        let fc = self.buf[1];
        if fc == self.function | EXCEPTION_FLAG {
            return Ok(Some(5));
        }
        if fc != self.function {
            return Err(FrameError::UnexpectedFunction { expected: self.function, received: fc });
        }
        match self.expect {
            ExpectedReply::Fixed(n) => Ok(Some(4 + usize::from(n))),
            ExpectedReply::ByteCount => Ok(self.buf.get(2).map(|&count| 5 + usize::from(count))),
        }
    }

    /// Feed received bytes. Bytes after a complete frame are counted as trailing and ignored.
    pub fn push(&mut self, bytes: &[u8]) -> FramerStatus {
        if let Some(done) = &self.done {
            self.trailing += bytes.len();
            self.buf.extend_from_slice(bytes);
            return FramerStatus::Complete(done.clone());
        }
        for (i, &b) in bytes.iter().enumerate() {
            self.buf.push(b);
            if self.needed.is_none() {
                match self.total_len() {
                    Ok(n) => self.needed = n,
                    Err(e) => return self.finish(Err(e), &bytes[i + 1..]),
                }
            }
            if let Some(n) = self.needed
                && self.buf.len() >= n
            {
                let result = self.validate(n);
                return self.finish(result, &bytes[i + 1..]);
            }
        }
        FramerStatus::NeedMore(match self.needed {
            Some(n) => n - self.buf.len(),
            None => if self.buf.len() < 2 { 2 - self.buf.len() } else { 1 },
        })
    }

    fn finish(&mut self, result: Result<Vec<u8>, FrameError>, rest: &[u8]) -> FramerStatus {
        self.trailing += rest.len();
        self.buf.extend_from_slice(rest);
        self.done = Some(result.clone());
        FramerStatus::Complete(result)
    }

    fn validate(&self, n: usize) -> Result<Vec<u8>, FrameError> {
        let (unit, pdu) = decode_adu(&self.buf[..n])?;
        if unit != self.unit {
            return Err(FrameError::WrongUnit { expected: self.unit, received: unit });
        }
        Ok(pdu.to_vec())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// Serial line settings for an RTU link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SerialSettings {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
}

impl SerialSettings {
    pub const fn new(baud: u32, data_bits: u8, parity: Parity, stop_bits: u8) -> Self {
        Self { baud, data_bits, parity, stop_bits }
    }

    /// 9600 8N1, the BGT-SMPS default.
    pub const fn default_9600_8n1() -> Self {
        Self::new(9600, 8, Parity::None, 1)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !(300..=3_000_000).contains(&self.baud) {
            return Err("baud must be 300..3000000");
        }
        if !matches!(self.data_bits, 7 | 8) {
            return Err("data bits must be 7 or 8");
        }
        if !matches!(self.stop_bits, 1 | 2) {
            return Err("stop bits must be 1 or 2");
        }
        Ok(())
    }

    /// Start + data + parity + stop bits.
    pub fn bits_per_char(&self) -> u32 {
        1 + u32::from(self.data_bits) + u32::from(self.parity != Parity::None) + u32::from(self.stop_bits)
    }
}

/// Timing derived from the real serial settings (Modbus over Serial Line V1.02 §2.5.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtuTiming {
    pub char_us: u64,
    pub t15_us: u64,
    pub t35_us: u64,
}

impl RtuTiming {
    pub fn new(s: &SerialSettings) -> Self {
        let baud = u64::from(s.baud.max(1));
        let char_us = (u64::from(s.bits_per_char()) * 1_000_000).div_ceil(baud);
        if s.baud > 19_200 {
            Self { char_us, t15_us: 750, t35_us: 1_750 }
        } else {
            Self { char_us, t15_us: (char_us * 3).div_ceil(2), t35_us: (char_us * 7).div_ceil(2) }
        }
    }

    /// Gap between the end of one exchange and the next request.
    pub fn inter_frame_ms(&self, floor_ms: u64) -> u64 {
        self.t35_us.div_ceil(1000).max(5).max(floor_ms)
    }

    /// Default response timeout: request + longest response on the wire plus
    /// 300 ms device turnaround, at least 500 ms.
    pub fn default_response_timeout_ms(&self) -> u64 {
        ((8 + MAX_ADU_LEN as u64) * self.char_us / 1000 + 300).max(500)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::unhex;

    #[test]
    fn known_frame() {
        assert_eq!(encode_adu(1, &[0x03, 0, 0, 0, 8]).unwrap(), unhex("01 03 00 00 00 08 44 0C"));
        assert_eq!(encode_adu(1, &[]), Err(FrameError::EmptyPdu));
        assert!(encode_adu(1, &[0u8; 254]).is_err());
        assert!(encode_adu(1, &[3u8; 253]).is_ok());
    }

    fn framer() -> ResponseFramer {
        ResponseFramer::new(1, 0x03, ExpectedReply::ByteCount)
    }

    #[test]
    fn complete_response_in_one_piece() {
        let mut f = framer();
        let frame = encode_adu(1, &[0x03, 0x02, 0x01, 0x0A]).unwrap();
        assert_eq!(f.push(&frame), FramerStatus::Complete(Ok(vec![0x03, 0x02, 0x01, 0x0A])));
    }

    #[test]
    fn exception_frame_is_five_bytes() {
        let mut f = framer();
        let frame = encode_adu(1, &[0x83, 0x02]).unwrap();
        assert_eq!(frame.len(), 5);
        assert_eq!(f.push(&frame[..3]), FramerStatus::NeedMore(2));
        assert_eq!(f.push(&frame[3..]), FramerStatus::Complete(Ok(vec![0x83, 0x02])));
    }

    #[test]
    fn bad_crc_is_crc_error() {
        let mut f = framer();
        let mut frame = encode_adu(1, &[0x03, 0x02, 0x01, 0x0A]).unwrap();
        frame[6] ^= 0xFF;
        assert!(matches!(f.push(&frame), FramerStatus::Complete(Err(FrameError::Crc { .. }))));
    }

    #[test]
    fn wrong_unit_is_decode_error_after_crc() {
        let mut f = framer();
        let frame = encode_adu(2, &[0x03, 0x02, 0x01, 0x0A]).unwrap();
        assert_eq!(f.push(&frame), FramerStatus::Complete(Err(FrameError::WrongUnit { expected: 1, received: 2 })));
    }

    #[test]
    fn unexpected_function_stops_early() {
        let mut f = framer();
        assert_eq!(
            f.push(&[0x01, 0x04]),
            FramerStatus::Complete(Err(FrameError::UnexpectedFunction { expected: 3, received: 4 }))
        );
    }

    #[test]
    fn fixed_length_reply_and_trailing_bytes() {
        let mut f = ResponseFramer::new(7, 0x2A, ExpectedReply::Fixed(4));
        let mut frame = encode_adu(7, &[0x2A, 1, 2, 3, 4]).unwrap();
        frame.extend_from_slice(&[0xEE, 0xEE]);
        assert_eq!(f.push(&frame), FramerStatus::Complete(Ok(vec![0x2A, 1, 2, 3, 4])));
        assert_eq!(f.trailing(), 2);
        assert!(matches!(f.push(&[1]), FramerStatus::Complete(Ok(_))));
        assert_eq!(f.trailing(), 3);
    }

    #[test]
    fn timing_from_real_settings() {
        let t = RtuTiming::new(&SerialSettings::default_9600_8n1());
        assert_eq!(t.char_us, 1042); // 10 bits at 9600
        assert_eq!(t.t35_us, 3647);
        assert_eq!(t.t15_us, 1563);
        assert_eq!(t.inter_frame_ms(0), 5); // 3.65 ms rounds up to 4, floor 5
        assert_eq!(t.inter_frame_ms(50), 50); // Android floor
        let e = RtuTiming::new(&SerialSettings::new(9600, 8, Parity::Even, 1));
        assert_eq!(e.char_us, 1146); // 11 bits
        let fast = RtuTiming::new(&SerialSettings::new(115_200, 8, Parity::None, 1));
        assert_eq!((fast.t15_us, fast.t35_us), (750, 1750));
        assert!(t.default_response_timeout_ms() >= 500);
        assert!(SerialSettings::new(9600, 6, Parity::None, 1).validate().is_err());
    }
}
