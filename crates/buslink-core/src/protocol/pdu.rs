//! Read PDUs (FC01–FC04), exception PDUs and spec limits.

use super::{FrameError, PduError};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Largest PDU in an RTU ADU (256 − unit − CRC).
pub const MAX_PDU_LEN: usize = 253;
/// FC03/FC04 quantity limit.
pub const MAX_READ_REGISTERS: u16 = 125;
/// FC01/FC02 quantity limit.
pub const MAX_READ_BITS: u16 = 2000;
/// Bit set in the function code of an exception response.
pub const EXCEPTION_FLAG: u8 = 0x80;

/// One of the four Modbus data tables. The function code selects the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Table {
    Coil,
    DiscreteInput,
    InputRegister,
    HoldingRegister,
}

impl Table {
    pub const ALL: [Table; 4] = [Table::Coil, Table::DiscreteInput, Table::InputRegister, Table::HoldingRegister];

    pub fn is_bit(self) -> bool {
        matches!(self, Table::Coil | Table::DiscreteInput)
    }

    pub fn read_function(self) -> ReadFunction {
        match self {
            Table::Coil => ReadFunction::Coils,
            Table::DiscreteInput => ReadFunction::DiscreteInputs,
            Table::HoldingRegister => ReadFunction::HoldingRegisters,
            Table::InputRegister => ReadFunction::InputRegisters,
        }
    }

    /// Modbus allows writes only to coils and holding registers.
    pub fn is_writable(self) -> bool {
        matches!(self, Table::Coil | Table::HoldingRegister)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Table::Coil => "coil",
            Table::DiscreteInput => "discrete_input",
            Table::InputRegister => "input_register",
            Table::HoldingRegister => "holding_register",
        }
    }

    pub fn parse(s: &str) -> Option<Table> {
        Table::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Conventional one-based label (`40001` for holding offset 0). Shown only
    /// as a hint when the profile has no `doc_address`; it is never sent.
    pub fn conventional_label(self, offset: u16) -> String {
        let prefix = match self {
            Table::Coil => 0,
            Table::DiscreteInput => 1,
            Table::InputRegister => 3,
            Table::HoldingRegister => 4,
        };
        let n = u32::from(offset) + 1;
        if n <= 9999 { format!("{prefix}{n:04}") } else { format!("{prefix}{n:05}") }
    }
}

/// The read function codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadFunction {
    Coils,
    DiscreteInputs,
    HoldingRegisters,
    InputRegisters,
}

impl ReadFunction {
    pub const ALL: [ReadFunction; 4] =
        [ReadFunction::Coils, ReadFunction::DiscreteInputs, ReadFunction::HoldingRegisters, ReadFunction::InputRegisters];

    pub fn code(self) -> u8 {
        match self {
            ReadFunction::Coils => 0x01,
            ReadFunction::DiscreteInputs => 0x02,
            ReadFunction::HoldingRegisters => 0x03,
            ReadFunction::InputRegisters => 0x04,
        }
    }

    pub fn from_code(code: u8) -> Option<ReadFunction> {
        ReadFunction::ALL.into_iter().find(|f| f.code() == code)
    }

    pub fn table(self) -> Table {
        match self {
            ReadFunction::Coils => Table::Coil,
            ReadFunction::DiscreteInputs => Table::DiscreteInput,
            ReadFunction::HoldingRegisters => Table::HoldingRegister,
            ReadFunction::InputRegisters => Table::InputRegister,
        }
    }

    pub fn is_bit(self) -> bool {
        self.table().is_bit()
    }

    pub fn max_quantity(self) -> u16 {
        if self.is_bit() { MAX_READ_BITS } else { MAX_READ_REGISTERS }
    }

    pub fn name(self) -> &'static str {
        match self {
            ReadFunction::Coils => "Read Coils",
            ReadFunction::DiscreteInputs => "Read Discrete Inputs",
            ReadFunction::HoldingRegisters => "Read Holding Registers",
            ReadFunction::InputRegisters => "Read Input Registers",
        }
    }
}

/// How the receiver knows the length of a normal (non-exception) response PDU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedReply {
    /// Function code, one byte count `n`, then `n` data bytes (FC01–FC04).
    ByteCount,
    /// Function code followed by exactly this many data bytes (echo replies).
    Fixed(u8),
}

/// A validated read request. Construct with [`ReadRequest::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadRequest {
    function: ReadFunction,
    address: u16,
    quantity: u16,
}

impl ReadRequest {
    pub fn new(function: ReadFunction, address: u16, quantity: u16) -> Result<Self, PduError> {
        let max = function.max_quantity();
        if quantity == 0 || quantity > max {
            return Err(PduError::Quantity { quantity, max });
        }
        if u32::from(address) + u32::from(quantity) > 65_536 {
            return Err(PduError::AddressRange { address, quantity });
        }
        Ok(Self { function, address, quantity })
    }

    pub fn for_table(table: Table, address: u16, quantity: u16) -> Result<Self, PduError> {
        Self::new(table.read_function(), address, quantity)
    }

    pub fn function(&self) -> ReadFunction {
        self.function
    }
    pub fn address(&self) -> u16 {
        self.address
    }
    pub fn quantity(&self) -> u16 {
        self.quantity
    }
    pub fn table(&self) -> Table {
        self.function.table()
    }

    /// Last address covered (inclusive).
    pub fn end_address(&self) -> u16 {
        // new() guarantees address + quantity <= 65536, so this cannot overflow.
        self.address.saturating_add(self.quantity - 1)
    }

    pub fn encode(&self) -> [u8; 5] {
        let a = self.address.to_be_bytes();
        let q = self.quantity.to_be_bytes();
        [self.function.code(), a[0], a[1], q[0], q[1]]
    }

    /// Parse a request PDU (server side, used by the simulator).
    pub fn decode(pdu: &[u8]) -> Result<Self, PduError> {
        let Some(&code) = pdu.first() else {
            return Err(PduError::Malformed("empty PDU"));
        };
        let function = ReadFunction::from_code(code).ok_or(PduError::UnsupportedFunction(code))?;
        if pdu.len() != 5 {
            return Err(PduError::Malformed("read request must be 5 bytes"));
        }
        let address = u16::from_be_bytes([pdu[1], pdu[2]]);
        let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
        Self::new(function, address, quantity)
    }

    pub fn expected_reply(&self) -> ExpectedReply {
        ExpectedReply::ByteCount
    }

    /// Data bytes in the response.
    pub fn response_byte_count(&self) -> usize {
        let q = usize::from(self.quantity);
        if self.function.is_bit() { q.div_ceil(8) } else { 2 * q }
    }

    /// Total RTU response ADU length: unit + fc + count + data + CRC.
    pub fn rtu_response_len(&self) -> usize {
        5 + self.response_byte_count()
    }
}

/// Values carried by a read response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadData {
    Bits(Vec<bool>),
    Registers(Vec<u16>),
}

impl ReadData {
    pub fn len(&self) -> usize {
        match self {
            ReadData::Bits(b) => b.len(),
            ReadData::Registers(r) => r.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Registers, or bits as 0/1 words, for uniform raw display.
    pub fn as_words(&self) -> Vec<u16> {
        match self {
            ReadData::Bits(b) => b.iter().map(|&x| u16::from(x)).collect(),
            ReadData::Registers(r) => r.clone(),
        }
    }
}

/// Server side: build a response PDU for `req` from `data` (simulator).
pub fn encode_read_response(req: &ReadRequest, data: &ReadData) -> Result<Vec<u8>, PduError> {
    let q = usize::from(req.quantity());
    let mut out = Vec::with_capacity(2 + req.response_byte_count());
    out.push(req.function().code());
    match (req.function().is_bit(), data) {
        (true, ReadData::Bits(bits)) if bits.len() == q => {
            out.push(u8::try_from(q.div_ceil(8)).map_err(|_| PduError::Malformed("too many bits"))?);
            for chunk in bits.chunks(8) {
                let mut byte = 0u8;
                for (i, &b) in chunk.iter().enumerate() {
                    if b {
                        byte |= 1 << i;
                    }
                }
                out.push(byte);
            }
        }
        (false, ReadData::Registers(regs)) if regs.len() == q => {
            out.push(u8::try_from(2 * q).map_err(|_| PduError::Malformed("too many registers"))?);
            for r in regs {
                out.extend_from_slice(&r.to_be_bytes());
            }
        }
        _ => return Err(PduError::Malformed("data does not match the request")),
    }
    Ok(out)
}

/// Client side: decode a response PDU (function code first) for `req`.
pub fn decode_read_response(req: &ReadRequest, pdu: &[u8]) -> Result<ReadData, ReplyError> {
    let fc = req.function().code();
    if let Some(code) = exception_in(fc, pdu) {
        return Err(ReplyError::Exception(code));
    }
    let Some(&received) = pdu.first() else {
        return Err(ReplyError::Frame(FrameError::EmptyPdu));
    };
    if received != fc {
        return Err(ReplyError::Frame(FrameError::UnexpectedFunction { expected: fc, received }));
    }
    let expected = req.response_byte_count();
    let Some(&count) = pdu.get(1) else {
        return Err(ReplyError::Frame(FrameError::Length { expected: expected + 2, received: pdu.len() }));
    };
    if usize::from(count) != expected {
        return Err(ReplyError::Frame(FrameError::ByteCount { expected, received: usize::from(count) }));
    }
    if pdu.len() != 2 + expected {
        return Err(ReplyError::Frame(FrameError::Length { expected: expected + 2, received: pdu.len() }));
    }
    let data = &pdu[2..];
    let q = usize::from(req.quantity());
    if req.function().is_bit() {
        let bits = (0..q).map(|i| data[i / 8] & (1 << (i % 8)) != 0).collect();
        Ok(ReadData::Bits(bits))
    } else {
        Ok(ReadData::Registers(data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect()))
    }
}

/// Why a response PDU is not usable data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplyError {
    #[error("{0}")]
    Exception(ExceptionCode),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

/// If `pdu` is an exception response to `function`, return its code.
pub fn exception_in(function: u8, pdu: &[u8]) -> Option<ExceptionCode> {
    match pdu {
        [fc, code] if *fc == function | EXCEPTION_FLAG => Some(ExceptionCode(*code)),
        _ => None,
    }
}

/// Server side: exception PDU for `function`.
pub fn exception_response(function: u8, code: ExceptionCode) -> [u8; 2] {
    [function | EXCEPTION_FLAG, code.0]
}

/// A Modbus exception code, kept as received (unknown codes included).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExceptionCode(pub u8);

impl ExceptionCode {
    pub const ILLEGAL_FUNCTION: ExceptionCode = ExceptionCode(0x01);
    pub const ILLEGAL_DATA_ADDRESS: ExceptionCode = ExceptionCode(0x02);
    pub const ILLEGAL_DATA_VALUE: ExceptionCode = ExceptionCode(0x03);
    pub const SERVER_DEVICE_FAILURE: ExceptionCode = ExceptionCode(0x04);
    pub const ACKNOWLEDGE: ExceptionCode = ExceptionCode(0x05);
    pub const SERVER_BUSY: ExceptionCode = ExceptionCode(0x06);
    pub const MEMORY_PARITY_ERROR: ExceptionCode = ExceptionCode(0x08);
    pub const GATEWAY_PATH_UNAVAILABLE: ExceptionCode = ExceptionCode(0x0A);
    pub const GATEWAY_TARGET_FAILED: ExceptionCode = ExceptionCode(0x0B);

    pub fn name(self) -> &'static str {
        match self.0 {
            0x01 => "illegal function",
            0x02 => "illegal data address",
            0x03 => "illegal data value",
            0x04 => "server device failure",
            0x05 => "acknowledge",
            0x06 => "server device busy",
            0x08 => "memory parity error",
            0x0A => "gateway path unavailable",
            0x0B => "gateway target device failed to respond",
            _ => "unknown exception",
        }
    }
}

impl fmt::Display for ExceptionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EXCEPTION {:02X} {}", self.0, self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_at_the_edges() {
        let h = ReadFunction::HoldingRegisters;
        assert!(ReadRequest::new(h, 0, 0).is_err());
        assert!(ReadRequest::new(h, 0, 125).is_ok());
        assert_eq!(ReadRequest::new(h, 0, 126), Err(PduError::Quantity { quantity: 126, max: 125 }));
        let c = ReadFunction::Coils;
        assert!(ReadRequest::new(c, 0, 2000).is_ok());
        assert!(ReadRequest::new(c, 0, 2001).is_err());
        assert!(ReadRequest::new(h, 65_535 - 9, 10).is_ok()); // ends at 65535: sum 65536
        assert_eq!(
            ReadRequest::new(h, 65_535 - 8, 10),
            Err(PduError::AddressRange { address: 65_527, quantity: 10 })
        );
        assert_eq!(ReadRequest::new(h, 65_526, 10).map(|r| r.end_address()), Ok(65_535));
    }

    #[test]
    fn soil_sensor_request_bytes() {
        let r = ReadRequest::new(ReadFunction::HoldingRegisters, 0, 8).unwrap();
        assert_eq!(r.encode(), [0x03, 0x00, 0x00, 0x00, 0x08]);
        assert_eq!(r.rtu_response_len(), 21);
        assert_eq!(ReadRequest::decode(&r.encode()), Ok(r));
    }

    #[test]
    fn bit_response_packing_lsb_first() {
        let r = ReadRequest::new(ReadFunction::Coils, 19, 10).unwrap();
        let bits = vec![true, false, true, true, false, false, true, true, true, false];
        let pdu = encode_read_response(&r, &ReadData::Bits(bits.clone())).unwrap();
        assert_eq!(pdu, vec![0x01, 0x02, 0xCD, 0x01]);
        assert_eq!(decode_read_response(&r, &pdu), Ok(ReadData::Bits(bits)));
    }

    #[test]
    fn register_response_and_errors() {
        let r = ReadRequest::new(ReadFunction::InputRegisters, 8, 1).unwrap();
        assert_eq!(decode_read_response(&r, &[0x04, 0x02, 0x00, 0x0A]), Ok(ReadData::Registers(vec![10])));
        assert_eq!(
            decode_read_response(&r, &[0x84, 0x02]),
            Err(ReplyError::Exception(ExceptionCode::ILLEGAL_DATA_ADDRESS))
        );
        assert!(matches!(
            decode_read_response(&r, &[0x04, 0x04, 0, 1, 0, 2]),
            Err(ReplyError::Frame(FrameError::ByteCount { expected: 2, received: 4 }))
        ));
        assert!(matches!(
            decode_read_response(&r, &[0x03, 0x02, 0, 1]),
            Err(ReplyError::Frame(FrameError::UnexpectedFunction { .. }))
        ));
        assert!(matches!(decode_read_response(&r, &[0x04, 0x02, 0]), Err(ReplyError::Frame(FrameError::Length { .. }))));
        assert!(decode_read_response(&r, &[]).is_err());
    }

    #[test]
    fn exception_names() {
        assert_eq!(ExceptionCode(2).to_string(), "EXCEPTION 02 illegal data address");
        assert_eq!(ExceptionCode(0x0B).name(), "gateway target device failed to respond");
        assert_eq!(ExceptionCode(0x42).name(), "unknown exception");
        for c in [1u8, 2, 3, 4, 5, 6, 8, 0x0A, 0x0B] {
            assert_ne!(ExceptionCode(c).name(), "unknown exception");
        }
    }

    #[test]
    fn decode_request_errors_map_to_exceptions() {
        assert_eq!(ReadRequest::decode(&[0x2B]).unwrap_err().exception(), ExceptionCode::ILLEGAL_FUNCTION);
        assert_eq!(ReadRequest::decode(&[0x03, 0, 0, 0, 0]).unwrap_err().exception(), ExceptionCode::ILLEGAL_DATA_VALUE);
        assert_eq!(
            ReadRequest::decode(&[0x03, 0xFF, 0xFF, 0, 2]).unwrap_err().exception(),
            ExceptionCode::ILLEGAL_DATA_ADDRESS
        );
    }

    #[test]
    fn conventional_labels() {
        assert_eq!(Table::HoldingRegister.conventional_label(0), "40001");
        assert_eq!(Table::Coil.conventional_label(0), "00001");
        assert_eq!(Table::InputRegister.conventional_label(9999), "310000");
    }
}
