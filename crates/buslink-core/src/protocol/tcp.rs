//! Modbus TCP: MBAP header and stream framing by the length field.

use super::FrameError;
use super::pdu::MAX_PDU_LEN;

pub const MBAP_HEADER_LEN: usize = 7;
pub const MAX_TCP_ADU_LEN: usize = 260;
pub const DEFAULT_PORT: u16 = 502;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MbapHeader {
    pub transaction_id: u16,
    pub protocol_id: u16,
    /// Unit ID + PDU bytes.
    pub length: u16,
    pub unit_id: u8,
}

impl MbapHeader {
    pub fn parse(b: &[u8; MBAP_HEADER_LEN]) -> Result<Self, FrameError> {
        let h = Self {
            transaction_id: u16::from_be_bytes([b[0], b[1]]),
            protocol_id: u16::from_be_bytes([b[2], b[3]]),
            length: u16::from_be_bytes([b[4], b[5]]),
            unit_id: b[6],
        };
        if h.protocol_id != 0 {
            return Err(FrameError::ProtocolId(h.protocol_id));
        }
        if !(2..=254).contains(&h.length) {
            return Err(FrameError::MbapLength(h.length));
        }
        Ok(h)
    }

    pub fn pdu_len(&self) -> usize {
        usize::from(self.length) - 1
    }
}

pub fn encode_adu(transaction_id: u16, unit: u8, pdu: &[u8]) -> Result<Vec<u8>, FrameError> {
    if pdu.is_empty() {
        return Err(FrameError::EmptyPdu);
    }
    if pdu.len() > MAX_PDU_LEN {
        return Err(FrameError::TooLong(pdu.len() + MBAP_HEADER_LEN));
    }
    let len = (pdu.len() + 1) as u16;
    let mut adu = Vec::with_capacity(MBAP_HEADER_LEN + pdu.len());
    adu.extend_from_slice(&transaction_id.to_be_bytes());
    adu.extend_from_slice(&[0, 0]);
    adu.extend_from_slice(&len.to_be_bytes());
    adu.push(unit);
    adu.extend_from_slice(pdu);
    Ok(adu)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TcpFrame {
    pub transaction_id: u16,
    pub unit_id: u8,
    pub pdu: Vec<u8>,
}

/// Stream reassembly. After a header error the stream cannot be framed any
/// more; the error is sticky and the connection must be dropped.
#[derive(Debug, Default, Clone)]
pub struct MbapReceiver {
    buf: Vec<u8>,
    broken: Option<FrameError>,
}

impl MbapReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.broken.is_none() {
            self.buf.extend_from_slice(bytes);
        }
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    pub fn is_broken(&self) -> bool {
        self.broken.is_some()
    }

    /// Next complete frame, `None` if more bytes are needed.
    pub fn next_frame(&mut self) -> Option<Result<TcpFrame, FrameError>> {
        if let Some(e) = &self.broken {
            return Some(Err(e.clone()));
        }
        if self.buf.len() < MBAP_HEADER_LEN {
            return None;
        }
        let mut hb = [0u8; MBAP_HEADER_LEN];
        hb.copy_from_slice(&self.buf[..MBAP_HEADER_LEN]);
        let header = match MbapHeader::parse(&hb) {
            Ok(h) => h,
            Err(e) => {
                self.broken = Some(e.clone());
                self.buf.clear();
                return Some(Err(e));
            }
        };
        let total = MBAP_HEADER_LEN + header.pdu_len();
        if self.buf.len() < total {
            return None;
        }
        let pdu = self.buf[MBAP_HEADER_LEN..total].to_vec();
        self.buf.drain(..total);
        Some(Ok(TcpFrame {
            transaction_id: header.transaction_id,
            unit_id: header.unit_id,
            pdu,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_header() {
        let adu = encode_adu(0x1234, 1, &[0x03, 0, 0, 0, 8]).unwrap();
        assert_eq!(adu, vec![0x12, 0x34, 0, 0, 0, 6, 1, 0x03, 0, 0, 0, 8]);
        assert!(encode_adu(1, 1, &[]).is_err());
        assert!(encode_adu(1, 1, &[1; 254]).is_err());
    }

    #[test]
    fn rejects_bad_headers() {
        for (len, ok) in [(0u16, false), (1, false), (2, true), (254, true), (255, false)] {
            let b = [0, 1, 0, 0, (len >> 8) as u8, len as u8, 1];
            assert_eq!(MbapHeader::parse(&b).is_ok(), ok, "length {len}");
        }
        assert_eq!(
            MbapHeader::parse(&[0, 1, 0, 1, 0, 6, 1]),
            Err(FrameError::ProtocolId(1))
        );
    }

    #[test]
    fn partial_and_coalesced() {
        let a = encode_adu(1, 1, &[0x03, 2, 0, 5]).unwrap();
        let b = encode_adu(2, 1, &[0x83, 2]).unwrap();
        let mut all = a.clone();
        all.extend_from_slice(&b);
        let mut r = MbapReceiver::new();
        r.push(&all[..3]);
        assert_eq!(r.next_frame(), None);
        r.push(&all[3..]);
        assert_eq!(
            r.next_frame(),
            Some(Ok(TcpFrame {
                transaction_id: 1,
                unit_id: 1,
                pdu: vec![0x03, 2, 0, 5]
            }))
        );
        assert_eq!(
            r.next_frame(),
            Some(Ok(TcpFrame {
                transaction_id: 2,
                unit_id: 1,
                pdu: vec![0x83, 2]
            }))
        );
        assert_eq!(r.next_frame(), None);
    }

    #[test]
    fn header_error_is_sticky() {
        let mut r = MbapReceiver::new();
        r.push(&[0, 1, 0, 9, 0, 6, 1, 3]);
        assert_eq!(r.next_frame(), Some(Err(FrameError::ProtocolId(9))));
        r.push(&encode_adu(1, 1, &[3, 0]).unwrap());
        assert!(r.is_broken());
        assert!(matches!(r.next_frame(), Some(Err(_))));
    }
}
