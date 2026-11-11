//! Modbus framing and read PDUs. Write PDUs are encoded only in `crate::command`.

pub mod crc;
pub mod pdu;
pub mod rtu;
pub mod tcp;

use thiserror::Error;

/// A frame that cannot be accepted as the answer to the outstanding request.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame too short ({0} bytes)")]
    TooShort(usize),
    #[error("frame too long ({0} bytes)")]
    TooLong(usize),
    #[error("empty PDU")]
    EmptyPdu,
    #[error("CRC mismatch (expected {expected:04X}, received {received:04X})")]
    Crc { expected: u16, received: u16 },
    #[error("response from unit {received}, expected unit {expected}")]
    WrongUnit { expected: u8, received: u8 },
    #[error("unexpected function code 0x{received:02X} (expected 0x{expected:02X})")]
    UnexpectedFunction { expected: u8, received: u8 },
    #[error("byte count {received} does not match the request (expected {expected})")]
    ByteCount { expected: usize, received: usize },
    #[error("PDU length {received} does not match (expected {expected})")]
    Length { expected: usize, received: usize },
    #[error("MBAP protocol ID {0} is not 0")]
    ProtocolId(u16),
    #[error("MBAP length {0} is outside 2..=254")]
    MbapLength(u16),
    #[error("response does not echo the request")]
    BadEcho,
}

/// A request PDU that the spec forbids or that a server must refuse.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PduError {
    #[error("function code 0x{0:02X} is not supported here")]
    UnsupportedFunction(u8),
    #[error("quantity {quantity} outside 1..={max}")]
    Quantity { quantity: u16, max: u16 },
    #[error("address {address} + quantity {quantity} exceeds 65536")]
    AddressRange { address: u16, quantity: u16 },
    #[error("malformed PDU: {0}")]
    Malformed(&'static str),
    #[error("value not allowed: {0}")]
    Value(&'static str),
}

impl PduError {
    /// The exception a server answers with for this request error.
    pub fn exception(&self) -> pdu::ExceptionCode {
        match self {
            PduError::UnsupportedFunction(_) => pdu::ExceptionCode::ILLEGAL_FUNCTION,
            PduError::AddressRange { .. } => pdu::ExceptionCode::ILLEGAL_DATA_ADDRESS,
            PduError::Quantity { .. } | PduError::Malformed(_) | PduError::Value(_) => {
                pdu::ExceptionCode::ILLEGAL_DATA_VALUE
            }
        }
    }
}
