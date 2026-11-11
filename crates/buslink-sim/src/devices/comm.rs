//! Communication-settings registers shared by the simulated VFD and temperature
//! controller (simulator design, commissioning-control design §6):
//! holding 10 = unit ID (1–247), holding 11 = baud code (0 4800, 1 9600, 2 19200, 3 38400).
//! A write is answered at the old settings; the world applies the change after the reply.

use crate::image::ImageBuilder;
use buslink_core::protocol::pdu::{ExceptionCode, Table};

pub const UNIT_REGISTER: u16 = 10;
pub const BAUD_REGISTER: u16 = 11;
pub const BAUD_CODES: [(u16, u32); 4] = [(0, 4800), (1, 9600), (2, 19_200), (3, 38_400)];
pub const DEFAULT_BAUD: u32 = 9600;

pub fn baud_of(code: u16) -> Option<u32> {
    BAUD_CODES.iter().find(|(c, _)| *c == code).map(|(_, b)| *b)
}

pub fn code_of(baud: u32) -> Option<u16> {
    BAUD_CODES.iter().find(|(_, b)| *b == baud).map(|(c, _)| *c)
}

/// A settings change the world applies after the reply has been built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommChange {
    Unit(u8),
    Baud(u32),
}

#[derive(Clone, Debug)]
pub struct CommRegs {
    pub unit: u8,
    pub baud: u32,
    pending: Vec<CommChange>,
}

impl Default for CommRegs {
    fn default() -> Self {
        Self {
            unit: 1,
            baud: DEFAULT_BAUD,
            pending: Vec::new(),
        }
    }
}

impl CommRegs {
    pub fn handles(address: u16) -> bool {
        address == UNIT_REGISTER || address == BAUD_REGISTER
    }

    pub fn image(&self, b: &mut ImageBuilder) {
        let h = Table::HoldingRegister;
        b.u16(h, UNIT_REGISTER, u16::from(self.unit), true).u16(
            h,
            BAUD_REGISTER,
            code_of(self.baud).unwrap_or(1),
            true,
        );
    }

    pub fn check(address: u16, value: u16) -> Result<(), ExceptionCode> {
        let ok = match address {
            UNIT_REGISTER => (1..=247).contains(&value),
            BAUD_REGISTER => baud_of(value).is_some(),
            _ => false,
        };
        if ok {
            Ok(())
        } else {
            Err(ExceptionCode::ILLEGAL_DATA_VALUE)
        }
    }

    /// Queue an already checked value.
    pub fn apply(&mut self, address: u16, value: u16) {
        match address {
            UNIT_REGISTER => {
                if let Ok(u) = u8::try_from(value) {
                    self.pending.push(CommChange::Unit(u));
                }
            }
            _ => {
                if let Some(b) = baud_of(value) {
                    self.pending.push(CommChange::Baud(b));
                }
            }
        }
    }

    pub fn take_pending(&mut self) -> Vec<CommChange> {
        std::mem::take(&mut self.pending)
    }
}
