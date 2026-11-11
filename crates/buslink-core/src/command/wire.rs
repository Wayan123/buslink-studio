//! Write PDUs: FC05, FC06, FC15, FC16. This file and the simulator are the
//! only places that know these function codes (safety rule 2).

use crate::protocol::pdu::ExpectedReply;
use crate::protocol::{FrameError, PduError};

pub const FC_WRITE_SINGLE_COIL: u8 = 0x05;
pub const FC_WRITE_SINGLE_REGISTER: u8 = 0x06;
pub const FC_WRITE_MULTIPLE_COILS: u8 = 0x0F;
pub const FC_WRITE_MULTIPLE_REGISTERS: u8 = 0x10;
pub const MAX_WRITE_COILS: u16 = 1968;
pub const MAX_WRITE_REGISTERS: u16 = 123;
const COIL_ON: u16 = 0xFF00;
const COIL_OFF: u16 = 0x0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteRequest {
    SingleCoil { address: u16, on: bool },
    SingleRegister { address: u16, value: u16 },
    MultipleCoils { address: u16, values: Vec<bool> },
    MultipleRegisters { address: u16, values: Vec<u16> },
}

fn check_span(address: u16, quantity: usize, max: u16) -> Result<(), PduError> {
    let q = u16::try_from(quantity).unwrap_or(u16::MAX);
    if quantity == 0 || quantity > usize::from(max) {
        return Err(PduError::Quantity { quantity: q, max });
    }
    if u32::from(address) + quantity as u32 > 65_536 {
        return Err(PduError::AddressRange { address, quantity: q });
    }
    Ok(())
}

impl WriteRequest {
    pub fn function_code(&self) -> u8 {
        match self {
            WriteRequest::SingleCoil { .. } => FC_WRITE_SINGLE_COIL,
            WriteRequest::SingleRegister { .. } => FC_WRITE_SINGLE_REGISTER,
            WriteRequest::MultipleCoils { .. } => FC_WRITE_MULTIPLE_COILS,
            WriteRequest::MultipleRegisters { .. } => FC_WRITE_MULTIPLE_REGISTERS,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            WriteRequest::SingleCoil { .. } => "Write Single Coil",
            WriteRequest::SingleRegister { .. } => "Write Single Register",
            WriteRequest::MultipleCoils { .. } => "Write Multiple Coils",
            WriteRequest::MultipleRegisters { .. } => "Write Multiple Registers",
        }
    }

    pub fn address(&self) -> u16 {
        match self {
            WriteRequest::SingleCoil { address, .. }
            | WriteRequest::SingleRegister { address, .. }
            | WriteRequest::MultipleCoils { address, .. }
            | WriteRequest::MultipleRegisters { address, .. } => *address,
        }
    }

    pub fn quantity(&self) -> usize {
        match self {
            WriteRequest::SingleCoil { .. } | WriteRequest::SingleRegister { .. } => 1,
            WriteRequest::MultipleCoils { values, .. } => values.len(),
            WriteRequest::MultipleRegisters { values, .. } => values.len(),
        }
    }

    pub fn validate(&self) -> Result<(), PduError> {
        match self {
            WriteRequest::SingleCoil { .. } | WriteRequest::SingleRegister { .. } => Ok(()),
            WriteRequest::MultipleCoils { address, values } => check_span(*address, values.len(), MAX_WRITE_COILS),
            WriteRequest::MultipleRegisters { address, values } => {
                check_span(*address, values.len(), MAX_WRITE_REGISTERS)
            }
        }
    }

    /// Request PDU (function code first).
    pub fn encode(&self) -> Result<Vec<u8>, PduError> {
        self.validate()?;
        let fc = self.function_code();
        let a = self.address().to_be_bytes();
        let mut out = vec![fc, a[0], a[1]];
        match self {
            WriteRequest::SingleCoil { on, .. } => {
                out.extend_from_slice(&(if *on { COIL_ON } else { COIL_OFF }).to_be_bytes());
            }
            WriteRequest::SingleRegister { value, .. } => out.extend_from_slice(&value.to_be_bytes()),
            WriteRequest::MultipleCoils { values, .. } => {
                out.extend_from_slice(&(values.len() as u16).to_be_bytes());
                out.push(values.len().div_ceil(8) as u8);
                for chunk in values.chunks(8) {
                    out.push(chunk.iter().enumerate().fold(0u8, |b, (i, &v)| if v { b | (1 << i) } else { b }));
                }
            }
            WriteRequest::MultipleRegisters { values, .. } => {
                out.extend_from_slice(&(values.len() as u16).to_be_bytes());
                out.push((values.len() * 2) as u8);
                for v in values {
                    out.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
        Ok(out)
    }

    /// Server side: parse a write request PDU (simulator, and the self-check before sending).
    pub fn decode(pdu: &[u8]) -> Result<WriteRequest, PduError> {
        let (&fc, rest) = pdu.split_first().ok_or(PduError::Malformed("empty PDU"))?;
        if rest.len() < 4 {
            return match fc {
                FC_WRITE_SINGLE_COIL | FC_WRITE_SINGLE_REGISTER | FC_WRITE_MULTIPLE_COILS | FC_WRITE_MULTIPLE_REGISTERS => {
                    Err(PduError::Malformed("write request too short"))
                }
                other => Err(PduError::UnsupportedFunction(other)),
            };
        }
        let address = u16::from_be_bytes([rest[0], rest[1]]);
        let word = u16::from_be_bytes([rest[2], rest[3]]);
        let req = match fc {
            FC_WRITE_SINGLE_COIL | FC_WRITE_SINGLE_REGISTER if rest.len() != 4 => {
                return Err(PduError::Malformed("single write must be 5 bytes"));
            }
            FC_WRITE_SINGLE_COIL => match word {
                COIL_ON => WriteRequest::SingleCoil { address, on: true },
                COIL_OFF => WriteRequest::SingleCoil { address, on: false },
                _ => return Err(PduError::Value("coil value must be 0xFF00 or 0x0000")),
            },
            FC_WRITE_SINGLE_REGISTER => WriteRequest::SingleRegister { address, value: word },
            FC_WRITE_MULTIPLE_COILS | FC_WRITE_MULTIPLE_REGISTERS => {
                let quantity = usize::from(word);
                let count = usize::from(*rest.get(4).ok_or(PduError::Malformed("missing byte count"))?);
                let data = &rest[5..];
                let (max, expected) = if fc == FC_WRITE_MULTIPLE_COILS {
                    (MAX_WRITE_COILS, quantity.div_ceil(8))
                } else {
                    (MAX_WRITE_REGISTERS, quantity * 2)
                };
                check_span(address, quantity, max)?;
                if count != expected || data.len() != count {
                    return Err(PduError::Malformed("byte count does not match quantity"));
                }
                if fc == FC_WRITE_MULTIPLE_COILS {
                    let values = (0..quantity).map(|i| data[i / 8] & (1 << (i % 8)) != 0).collect();
                    WriteRequest::MultipleCoils { address, values }
                } else {
                    let values = data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    WriteRequest::MultipleRegisters { address, values }
                }
            }
            other => return Err(PduError::UnsupportedFunction(other)),
        };
        Ok(req)
    }

    /// Every write reply is the function code plus 4 bytes.
    pub fn expected_reply(&self) -> ExpectedReply {
        ExpectedReply::Fixed(4)
    }

    /// Server side: the normal reply (single writes echo the request; multiple
    /// writes echo address and quantity).
    pub fn response_pdu(&self) -> Vec<u8> {
        let a = self.address().to_be_bytes();
        match self {
            WriteRequest::SingleCoil { .. } | WriteRequest::SingleRegister { .. } => {
                self.encode().unwrap_or_else(|_| vec![self.function_code()])
            }
            _ => {
                let q = (self.quantity() as u16).to_be_bytes();
                vec![self.function_code(), a[0], a[1], q[0], q[1]]
            }
        }
    }

    /// Client side: does `reply` acknowledge exactly this request?
    pub fn verify_echo(&self, reply: &[u8]) -> Result<(), FrameError> {
        if reply == self.response_pdu().as_slice() { Ok(()) } else { Err(FrameError::BadEcho) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_examples() {
        // Modbus Application Protocol V1.1b3 §6.5, §6.6, §6.11, §6.12 examples.
        let c = WriteRequest::SingleCoil { address: 172, on: true };
        assert_eq!(c.encode().unwrap(), vec![0x05, 0x00, 0xAC, 0xFF, 0x00]);
        let r = WriteRequest::SingleRegister { address: 1, value: 3 };
        assert_eq!(r.encode().unwrap(), vec![0x06, 0x00, 0x01, 0x00, 0x03]);
        let mc = WriteRequest::MultipleCoils {
            address: 19,
            values: vec![true, false, true, true, false, false, true, true, true, false],
        };
        assert_eq!(mc.encode().unwrap(), vec![0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01]);
        assert_eq!(mc.response_pdu(), vec![0x0F, 0x00, 0x13, 0x00, 0x0A]);
        let mr = WriteRequest::MultipleRegisters { address: 1, values: vec![0x000A, 0x0102] };
        assert_eq!(mr.encode().unwrap(), vec![0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02]);
        assert_eq!(mr.response_pdu(), vec![0x10, 0x00, 0x01, 0x00, 0x02]);
    }

    #[test]
    fn limits() {
        let too_many = WriteRequest::MultipleRegisters { address: 0, values: vec![0; 124] };
        assert!(too_many.encode().is_err());
        let ok = WriteRequest::MultipleRegisters { address: 0, values: vec![0; 123] };
        assert!(ok.encode().is_ok());
        let past_end = WriteRequest::MultipleRegisters { address: 65_535, values: vec![0; 2] };
        assert!(matches!(past_end.encode(), Err(PduError::AddressRange { .. })));
        assert!(WriteRequest::MultipleCoils { address: 0, values: vec![] }.encode().is_err());
        assert!(WriteRequest::MultipleCoils { address: 0, values: vec![true; 1969] }.encode().is_err());
    }

    #[test]
    fn decode_rejects_bad_coil_value_and_counts() {
        assert_eq!(WriteRequest::decode(&[0x05, 0, 1, 0x12, 0x34]), Err(PduError::Value("coil value must be 0xFF00 or 0x0000")));
        assert!(WriteRequest::decode(&[0x10, 0, 1, 0, 2, 3, 0, 1, 0]).is_err());
        assert!(WriteRequest::decode(&[0x06, 0, 1]).is_err());
        assert_eq!(WriteRequest::decode(&[0x2B, 0, 1, 0, 1]), Err(PduError::UnsupportedFunction(0x2B)));
    }

    #[test]
    fn echo_verification() {
        let r = WriteRequest::SingleRegister { address: 1, value: 3000 };
        assert!(r.verify_echo(&r.encode().unwrap()).is_ok());
        assert_eq!(r.verify_echo(&[0x06, 0, 1, 0, 0]), Err(FrameError::BadEcho));
        let m = WriteRequest::MultipleRegisters { address: 4, values: vec![1, 2] };
        assert!(m.verify_echo(&[0x10, 0, 4, 0, 2]).is_ok());
        assert!(m.verify_echo(&[0x10, 0, 4, 0, 3]).is_err());
    }
}
