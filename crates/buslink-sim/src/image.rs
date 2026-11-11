//! Register image: what a simulated device serves right now. The register map
//! of each model is defined once, by the code that builds its image.

use buslink_core::codec::{self, DataType, RawValue, WordOrder};
use buslink_core::protocol::pdu::Table;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegisterImage {
    pub holding: BTreeMap<u16, u16>,
    pub input: BTreeMap<u16, u16>,
    pub coils: BTreeMap<u16, bool>,
    pub discrete: BTreeMap<u16, bool>,
    pub writable_holding: BTreeSet<u16>,
    pub writable_coils: BTreeSet<u16>,
}

impl RegisterImage {
    pub fn word(&self, table: Table, address: u16) -> Option<u16> {
        match table {
            Table::HoldingRegister => self.holding.get(&address).copied(),
            Table::InputRegister => self.input.get(&address).copied(),
            Table::Coil => self.coils.get(&address).map(|&b| u16::from(b)),
            Table::DiscreteInput => self.discrete.get(&address).map(|&b| u16::from(b)),
        }
    }

    pub fn is_writable(&self, table: Table, address: u16) -> bool {
        match table {
            Table::HoldingRegister => self.writable_holding.contains(&address),
            Table::Coil => self.writable_coils.contains(&address),
            _ => false,
        }
    }
}

/// Builds an image with the codec. With `wrong_order`, 32/64-bit values are
/// served word-swapped and 16-bit registers byte-swapped (fault injection).
pub struct ImageBuilder {
    image: RegisterImage,
    wrong_order: bool,
}

impl ImageBuilder {
    pub fn new(wrong_order: bool) -> Self {
        Self {
            image: RegisterImage::default(),
            wrong_order,
        }
    }

    fn put_words(&mut self, table: Table, address: u16, words: &[u16], writable: bool) {
        for (i, w) in words.iter().enumerate() {
            let a = address.saturating_add(i as u16);
            match table {
                Table::HoldingRegister => {
                    self.image.holding.insert(a, *w);
                    if writable {
                        self.image.writable_holding.insert(a);
                    }
                }
                Table::InputRegister => {
                    self.image.input.insert(a, *w);
                }
                Table::Coil | Table::DiscreteInput => {}
            }
        }
    }

    /// Put a typed value. Encoding failures (a model bug) leave the address unmapped,
    /// so a client sees exception 02 instead of a made-up value.
    pub fn value(
        &mut self,
        table: Table,
        address: u16,
        ty: DataType,
        order: Option<WordOrder>,
        v: RawValue,
        writable: bool,
    ) -> &mut Self {
        let order = match (self.wrong_order, order) {
            (true, Some(o)) => Some(o.toggled_words()),
            (_, o) => o,
        };
        if let Ok(mut words) = codec::encode(ty, order, &v) {
            if self.wrong_order && words.len() == 1 && ty != DataType::Bool {
                words[0] = words[0].swap_bytes();
            }
            self.put_words(table, address, &words, writable);
        }
        self
    }

    pub fn u16(&mut self, table: Table, address: u16, v: u16, writable: bool) -> &mut Self {
        self.value(table, address, DataType::U16, None, RawValue::U16(v), writable)
    }

    pub fn i16(&mut self, table: Table, address: u16, v: i16, writable: bool) -> &mut Self {
        self.value(table, address, DataType::I16, None, RawValue::I16(v), writable)
    }

    pub fn bits(&mut self, table: Table, address: u16, v: u16) -> &mut Self {
        self.value(table, address, DataType::Bitfield, None, RawValue::Bits(v), false)
    }

    pub fn coil(&mut self, address: u16, on: bool, writable: bool) -> &mut Self {
        self.image.coils.insert(address, on);
        if writable {
            self.image.writable_coils.insert(address);
        }
        self
    }

    pub fn discrete(&mut self, address: u16, on: bool) -> &mut Self {
        self.image.discrete.insert(address, on);
        self
    }

    pub fn build(self) -> RegisterImage {
        self.image
    }
}

/// Engineering value → raw integer with rounding, clamped to the type range.
/// Simulator-side only: the simulated device quantises its own process value.
pub fn quantise(value: f64, scale: f64, lo: f64, hi: f64) -> f64 {
    if !value.is_finite() || scale == 0.0 {
        return lo.max(0.0).min(hi);
    }
    (value / scale).round().clamp(lo, hi)
}
