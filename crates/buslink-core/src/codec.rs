//! Register codec: data types, word orders, `raw × scale + offset`, range and
//! sentinel checks (modbus.md "Data types and word order").
//!
//! Canonical form: registers in value order, most significant first, each
//! register big-endian (A = most significant byte). Word orders are applied
//! as two independent involutions: word swap (reverse the registers) and byte
//! swap (swap the two bytes inside every register).

use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WordOrder {
    ABCD,
    CDAB,
    BADC,
    DCBA,
}

impl WordOrder {
    pub const ALL: [WordOrder; 4] = [WordOrder::ABCD, WordOrder::CDAB, WordOrder::BADC, WordOrder::DCBA];

    pub fn word_swap(self) -> bool {
        matches!(self, WordOrder::CDAB | WordOrder::DCBA)
    }

    pub fn byte_swap(self) -> bool {
        matches!(self, WordOrder::BADC | WordOrder::DCBA)
    }

    /// Wire ↔ canonical. The transform is its own inverse.
    pub fn apply(self, regs: &[u16]) -> Vec<u16> {
        let mut out: Vec<u16> = regs.to_vec();
        if self.word_swap() {
            out.reverse();
        }
        if self.byte_swap() {
            for r in &mut out {
                *r = r.swap_bytes();
            }
        }
        out
    }

    /// The order with the word swap toggled (what a misconfigured device serves).
    pub fn toggled_words(self) -> WordOrder {
        match self {
            WordOrder::ABCD => WordOrder::CDAB,
            WordOrder::CDAB => WordOrder::ABCD,
            WordOrder::BADC => WordOrder::DCBA,
            WordOrder::DCBA => WordOrder::BADC,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WordOrder::ABCD => "ABCD",
            WordOrder::CDAB => "CDAB",
            WordOrder::BADC => "BADC",
            WordOrder::DCBA => "DCBA",
        }
    }
}

/// Type names as they appear in profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TypeName {
    Bool,
    U16,
    I16,
    Bitfield,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
    String,
}

impl TypeName {
    /// Register width; `None` for strings (width comes from `length`).
    pub fn fixed_width(self) -> Option<u16> {
        Some(match self {
            TypeName::Bool | TypeName::U16 | TypeName::I16 | TypeName::Bitfield => 1,
            TypeName::U32 | TypeName::I32 | TypeName::F32 => 2,
            TypeName::U64 | TypeName::I64 | TypeName::F64 => 4,
            TypeName::String => return None,
        })
    }

    pub fn needs_word_order(self) -> bool {
        matches!(self.fixed_width(), Some(2) | Some(4))
    }
}

/// A fully specified data type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataType {
    Bool,
    U16,
    I16,
    Bitfield,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
    Str { len: u16 },
}

impl DataType {
    pub fn from_name(name: TypeName, length: Option<u16>) -> Option<DataType> {
        Some(match name {
            TypeName::Bool => DataType::Bool,
            TypeName::U16 => DataType::U16,
            TypeName::I16 => DataType::I16,
            TypeName::Bitfield => DataType::Bitfield,
            TypeName::U32 => DataType::U32,
            TypeName::I32 => DataType::I32,
            TypeName::F32 => DataType::F32,
            TypeName::U64 => DataType::U64,
            TypeName::I64 => DataType::I64,
            TypeName::F64 => DataType::F64,
            TypeName::String => DataType::Str {
                len: length.filter(|l| (1..=125).contains(l))?,
            },
        })
    }

    /// Number of registers (or bits for `Bool`).
    pub fn width(self) -> u16 {
        match self {
            DataType::Bool | DataType::U16 | DataType::I16 | DataType::Bitfield => 1,
            DataType::U32 | DataType::I32 | DataType::F32 => 2,
            DataType::U64 | DataType::I64 | DataType::F64 => 4,
            DataType::Str { len } => len,
        }
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            DataType::Bool
                | DataType::U16
                | DataType::I16
                | DataType::Bitfield
                | DataType::U32
                | DataType::I32
                | DataType::U64
                | DataType::I64
        )
    }
}

/// A decoded raw value. 64-bit integers are exact.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    Bool(bool),
    U16(u16),
    I16(i16),
    Bits(u16),
    U32(u32),
    I32(i32),
    F32(f32),
    U64(u64),
    I64(i64),
    F64(f64),
    Str(String),
}

impl RawValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            RawValue::Bool(_) => "bool",
            RawValue::U16(_) => "u16",
            RawValue::I16(_) => "i16",
            RawValue::Bits(_) => "bitfield",
            RawValue::U32(_) => "u32",
            RawValue::I32(_) => "i32",
            RawValue::F32(_) => "f32",
            RawValue::U64(_) => "u64",
            RawValue::I64(_) => "i64",
            RawValue::F64(_) => "f64",
            RawValue::Str(_) => "string",
        }
    }

    /// Exact integer value for integer types.
    pub fn as_i128(&self) -> Option<i128> {
        Some(match self {
            RawValue::Bool(b) => i128::from(*b),
            RawValue::U16(v) | RawValue::Bits(v) => i128::from(*v),
            RawValue::I16(v) => i128::from(*v),
            RawValue::U32(v) => i128::from(*v),
            RawValue::I32(v) => i128::from(*v),
            RawValue::U64(v) => i128::from(*v),
            RawValue::I64(v) => i128::from(*v),
            _ => return None,
        })
    }

    /// Numeric value as f64 (may round for 64-bit integers; use `as_i128` for exactness).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            RawValue::F32(v) => Some(f64::from(*v)),
            RawValue::F64(v) => Some(*v),
            RawValue::Str(_) => None,
            other => other.as_i128().map(|v| v as f64),
        }
    }

    /// Human-readable raw value (exact for integers).
    pub fn display(&self) -> String {
        match self {
            RawValue::Str(s) => s.clone(),
            RawValue::F32(v) => v.to_string(),
            RawValue::F64(v) => v.to_string(),
            other => other.as_i128().map(|v| v.to_string()).unwrap_or_default(),
        }
    }
}

impl fmt::Display for RawValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// `{"type": "u64", "value": "18446744073709551615"}`; 64-bit integers as strings.
impl Serialize for RawValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("RawValue", 2)?;
        st.serialize_field("type", self.type_name())?;
        match self {
            RawValue::Bool(v) => st.serialize_field("value", v)?,
            RawValue::U16(v) | RawValue::Bits(v) => st.serialize_field("value", v)?,
            RawValue::I16(v) => st.serialize_field("value", v)?,
            RawValue::U32(v) => st.serialize_field("value", v)?,
            RawValue::I32(v) => st.serialize_field("value", v)?,
            RawValue::F32(v) => st.serialize_field("value", &finite_or_none(f64::from(*v)))?,
            RawValue::F64(v) => st.serialize_field("value", &finite_or_none(*v))?,
            RawValue::U64(v) => st.serialize_field("value", &v.to_string())?,
            RawValue::I64(v) => st.serialize_field("value", &v.to_string())?,
            RawValue::Str(v) => st.serialize_field("value", v)?,
        }
        st.end()
    }
}

fn finite_or_none(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CodecError {
    #[error("expected {expected} registers, got {got}")]
    Width { expected: usize, got: usize },
    #[error("word order is required for multi-register types")]
    MissingWordOrder,
    #[error("string word order must be ABCD or BADC")]
    StringOrder,
    #[error("string is not valid UTF-8")]
    InvalidText,
    #[error("string longer than {0} bytes")]
    TextTooLong(usize),
    #[error("bool register value {0} is not 0 or 1")]
    BoolValue(u16),
    #[error("value is not finite")]
    NonFinite,
    #[error("raw value {0} is a sentinel (sensor fault / no data)")]
    Sentinel(String),
    #[error("value {value} outside the valid range {min}..{max}")]
    OutOfRange { value: f64, min: f64, max: f64 },
    #[error("value {0} is not exactly representable in the register type")]
    NotRepresentable(f64),
    #[error("value {0} is outside the register type range")]
    OutOfTypeRange(f64),
    #[error("type {0} cannot be written")]
    Unsupported(&'static str),
}

fn need_order(order: Option<WordOrder>) -> Result<WordOrder, CodecError> {
    order.ok_or(CodecError::MissingWordOrder)
}

fn be_bytes(regs: &[u16]) -> Vec<u8> {
    regs.iter().flat_map(|r| r.to_be_bytes()).collect()
}

/// Decode registers (wire order) into a raw value. For `Bool`, `regs` holds one 0/1 word.
pub fn decode(ty: DataType, order: Option<WordOrder>, regs: &[u16]) -> Result<RawValue, CodecError> {
    let width = usize::from(ty.width());
    if regs.len() != width {
        return Err(CodecError::Width {
            expected: width,
            got: regs.len(),
        });
    }
    let canon = |o: Option<WordOrder>| -> Result<Vec<u8>, CodecError> { Ok(be_bytes(&need_order(o)?.apply(regs))) };
    Ok(match ty {
        DataType::Bool => match regs[0] {
            0 => RawValue::Bool(false),
            1 => RawValue::Bool(true),
            v => return Err(CodecError::BoolValue(v)),
        },
        DataType::U16 => RawValue::U16(regs[0]),
        DataType::I16 => RawValue::I16(regs[0] as i16),
        DataType::Bitfield => RawValue::Bits(regs[0]),
        DataType::U32 => RawValue::U32(u32::from_be_bytes(arr4(&canon(order)?))),
        DataType::I32 => RawValue::I32(i32::from_be_bytes(arr4(&canon(order)?))),
        DataType::F32 => RawValue::F32(f32::from_be_bytes(arr4(&canon(order)?))),
        DataType::U64 => RawValue::U64(u64::from_be_bytes(arr8(&canon(order)?))),
        DataType::I64 => RawValue::I64(i64::from_be_bytes(arr8(&canon(order)?))),
        DataType::F64 => RawValue::F64(f64::from_be_bytes(arr8(&canon(order)?))),
        DataType::Str { .. } => {
            let o = order.unwrap_or(WordOrder::ABCD);
            if o.word_swap() {
                return Err(CodecError::StringOrder);
            }
            let mut bytes = be_bytes(&o.apply(regs));
            while bytes.last().is_some_and(|&b| b == 0 || b == b' ') {
                bytes.pop();
            }
            RawValue::Str(String::from_utf8(bytes).map_err(|_| CodecError::InvalidText)?)
        }
    })
}

fn arr4(b: &[u8]) -> [u8; 4] {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[..4]);
    a
}

fn arr8(b: &[u8]) -> [u8; 8] {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    a
}

fn words(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
        .collect()
}

/// Encode a raw value into registers in wire order.
pub fn encode(ty: DataType, order: Option<WordOrder>, value: &RawValue) -> Result<Vec<u16>, CodecError> {
    let canon: Vec<u16> = match (ty, value) {
        (DataType::Bool, RawValue::Bool(b)) => return Ok(vec![u16::from(*b)]),
        (DataType::U16, RawValue::U16(v)) | (DataType::Bitfield, RawValue::Bits(v)) => return Ok(vec![*v]),
        (DataType::I16, RawValue::I16(v)) => return Ok(vec![*v as u16]),
        (DataType::U32, RawValue::U32(v)) => words(&v.to_be_bytes()),
        (DataType::I32, RawValue::I32(v)) => words(&v.to_be_bytes()),
        (DataType::F32, RawValue::F32(v)) => words(&v.to_be_bytes()),
        (DataType::U64, RawValue::U64(v)) => words(&v.to_be_bytes()),
        (DataType::I64, RawValue::I64(v)) => words(&v.to_be_bytes()),
        (DataType::F64, RawValue::F64(v)) => words(&v.to_be_bytes()),
        (DataType::Str { len }, RawValue::Str(s)) => {
            let o = order.unwrap_or(WordOrder::ABCD);
            if o.word_swap() {
                return Err(CodecError::StringOrder);
            }
            let cap = usize::from(len) * 2;
            if s.len() > cap {
                return Err(CodecError::TextTooLong(cap));
            }
            let mut b = s.as_bytes().to_vec();
            b.resize(cap, 0);
            return Ok(o.apply(&words(&b)));
        }
        _ => {
            return Err(CodecError::Width {
                expected: usize::from(ty.width()),
                got: 0,
            });
        }
    };
    Ok(need_order(order)?.apply(&canon))
}

/// Engineering conversion settings of one tag.
#[derive(Clone, Debug, PartialEq)]
pub struct Scaling {
    pub scale: f64,
    pub offset: f64,
    pub valid_range: Option<[f64; 2]>,
    pub sentinels: Vec<i128>,
}

impl Scaling {
    /// Same conversion without the range check (old values and read-backs are
    /// reported even when outside `valid_range`).
    pub fn without_range(&self) -> Scaling {
        Scaling {
            valid_range: None,
            ..self.clone()
        }
    }
}

impl Default for Scaling {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
            valid_range: None,
            sentinels: Vec::new(),
        }
    }
}

/// Raw → engineering: sentinel check, `raw × scale + offset`, finite check, range check.
/// Strings have no engineering value (`Ok(None)`).
pub fn to_engineering(raw: &RawValue, s: &Scaling) -> Result<Option<f64>, CodecError> {
    if let Some(i) = raw.as_i128() {
        if s.sentinels.contains(&i) {
            return Err(CodecError::Sentinel(i.to_string()));
        }
    } else if let Some(f) = raw.as_f64()
        && s.sentinels.iter().any(|&v| (v as f64) == f)
    {
        return Err(CodecError::Sentinel(raw.display()));
    }
    let Some(r) = raw.as_f64() else {
        return Ok(None);
    };
    let value = r * s.scale + s.offset;
    if !value.is_finite() {
        return Err(CodecError::NonFinite);
    }
    if let Some([min, max]) = s.valid_range
        && !(min..=max).contains(&value)
    {
        return Err(CodecError::OutOfRange { value, min, max });
    }
    Ok(Some(value))
}

/// Engineering → raw for commands. Rejects (never rounds or clamps) values that
/// do not map to an exact raw integer, or are outside the type range.
pub fn to_raw(ty: DataType, engineering: f64, scale: f64, offset: f64) -> Result<RawValue, CodecError> {
    if !engineering.is_finite() || !scale.is_finite() || scale == 0.0 || !offset.is_finite() {
        return Err(CodecError::NonFinite);
    }
    let raw = (engineering - offset) / scale;
    if !raw.is_finite() {
        return Err(CodecError::NonFinite);
    }
    match ty {
        DataType::F32 => {
            let v = raw as f32;
            if !v.is_finite() {
                return Err(CodecError::OutOfTypeRange(engineering));
            }
            return Ok(RawValue::F32(v));
        }
        DataType::F64 => return Ok(RawValue::F64(raw)),
        DataType::Str { .. } => return Err(CodecError::Unsupported("string")),
        _ => {}
    }
    let rounded = raw.round();
    if (raw - rounded).abs() > 1e-9 * raw.abs().max(1.0) {
        return Err(CodecError::NotRepresentable(engineering));
    }
    // Integers above 2^53 cannot be reached exactly through f64.
    if rounded.abs() > 9_007_199_254_740_992.0 {
        return Err(CodecError::NotRepresentable(engineering));
    }
    let i = rounded as i128;
    let fits = |lo: i128, hi: i128| (lo..=hi).contains(&i);
    let out = match ty {
        DataType::Bool if fits(0, 1) => RawValue::Bool(i == 1),
        DataType::U16 if fits(0, i128::from(u16::MAX)) => RawValue::U16(i as u16),
        DataType::Bitfield if fits(0, i128::from(u16::MAX)) => RawValue::Bits(i as u16),
        DataType::I16 if fits(i128::from(i16::MIN), i128::from(i16::MAX)) => RawValue::I16(i as i16),
        DataType::U32 if fits(0, i128::from(u32::MAX)) => RawValue::U32(i as u32),
        DataType::I32 if fits(i128::from(i32::MIN), i128::from(i32::MAX)) => RawValue::I32(i as i32),
        DataType::U64 if fits(0, i128::from(u64::MAX)) => RawValue::U64(i as u64),
        DataType::I64 if fits(i128::from(i64::MIN), i128::from(i64::MAX)) => RawValue::I64(i as i64),
        _ => return Err(CodecError::OutOfTypeRange(engineering)),
    };
    Ok(out)
}

/// Decode the same registers in all four orders (register inspector).
pub fn decode_all_orders(ty: DataType, regs: &[u16]) -> Vec<(WordOrder, Result<RawValue, CodecError>)> {
    WordOrder::ALL
        .into_iter()
        .map(|o| (o, decode(ty, Some(o), regs)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modbus_md_f32_table() {
        let expect = [
            (WordOrder::ABCD, [0x42F6, 0xE979]),
            (WordOrder::CDAB, [0xE979, 0x42F6]),
            (WordOrder::BADC, [0xF642, 0x79E9]),
            (WordOrder::DCBA, [0x79E9, 0xF642]),
        ];
        for (o, wire) in expect {
            assert_eq!(
                encode(DataType::F32, Some(o), &RawValue::F32(123.456)).unwrap(),
                wire.to_vec(),
                "{o:?}"
            );
            assert_eq!(decode(DataType::F32, Some(o), &wire).unwrap(), RawValue::F32(123.456));
        }
    }

    #[test]
    fn integers_in_all_orders() {
        let v: u32 = 0x1122_3344;
        assert_eq!(
            encode(DataType::U32, Some(WordOrder::ABCD), &RawValue::U32(v)).unwrap(),
            vec![0x1122, 0x3344]
        );
        assert_eq!(
            encode(DataType::U32, Some(WordOrder::CDAB), &RawValue::U32(v)).unwrap(),
            vec![0x3344, 0x1122]
        );
        assert_eq!(
            encode(DataType::U32, Some(WordOrder::BADC), &RawValue::U32(v)).unwrap(),
            vec![0x2211, 0x4433]
        );
        assert_eq!(
            encode(DataType::U32, Some(WordOrder::DCBA), &RawValue::U32(v)).unwrap(),
            vec![0x4433, 0x2211]
        );
        let regs = [0xFFFF, 0xFFFE];
        assert_eq!(
            decode(DataType::I32, Some(WordOrder::ABCD), &regs).unwrap(),
            RawValue::I32(-2)
        );
        let w = encode(
            DataType::U64,
            Some(WordOrder::CDAB),
            &RawValue::U64(0x0102_0304_0506_0708),
        )
        .unwrap();
        assert_eq!(w, vec![0x0708, 0x0506, 0x0304, 0x0102]);
        let f = encode(DataType::F64, Some(WordOrder::DCBA), &RawValue::F64(1.0)).unwrap();
        assert_eq!(f, vec![0x0000, 0x0000, 0x0000, 0xF03F]);
    }

    #[test]
    fn u64_is_exact() {
        let v = RawValue::U64(u64::MAX - 1);
        let w = encode(DataType::U64, Some(WordOrder::ABCD), &v).unwrap();
        let back = decode(DataType::U64, Some(WordOrder::ABCD), &w).unwrap();
        assert_eq!(back.as_i128(), Some(i128::from(u64::MAX - 1)));
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            r#"{"type":"u64","value":"18446744073709551614"}"#
        );
    }

    #[test]
    fn soil_temperature_sign_and_scale() {
        // 65501 → −35 raw → −3.5 °C (BGT-SMPS temperature is i16 ÷ 10).
        let raw = decode(DataType::I16, None, &[65_501]).unwrap();
        let s = Scaling {
            scale: 0.1,
            ..Scaling::default()
        };
        let v = to_engineering(&raw, &s).unwrap().unwrap();
        assert!((v + 3.5).abs() < 1e-9);
    }

    #[test]
    fn width_and_order_errors() {
        assert_eq!(
            decode(DataType::U32, Some(WordOrder::ABCD), &[1]),
            Err(CodecError::Width { expected: 2, got: 1 })
        );
        assert_eq!(decode(DataType::U32, None, &[1, 2]), Err(CodecError::MissingWordOrder));
        assert_eq!(decode(DataType::Bool, None, &[2]), Err(CodecError::BoolValue(2)));
        assert_eq!(
            decode(DataType::Str { len: 2 }, Some(WordOrder::CDAB), &[1, 2]),
            Err(CodecError::StringOrder)
        );
        assert!(decode(DataType::Str { len: 1 }, None, &[0xFFFE]).is_err());
    }

    #[test]
    fn strings() {
        let regs = encode(DataType::Str { len: 3 }, None, &RawValue::Str("ABC".into())).unwrap();
        assert_eq!(regs, vec![0x4142, 0x4300, 0x0000]);
        let swapped = encode(
            DataType::Str { len: 3 },
            Some(WordOrder::BADC),
            &RawValue::Str("ABC".into()),
        )
        .unwrap();
        assert_eq!(swapped, vec![0x4241, 0x0043, 0x0000]);
        assert_eq!(
            decode(DataType::Str { len: 3 }, Some(WordOrder::BADC), &swapped).unwrap(),
            RawValue::Str("ABC".into())
        );
        assert!(encode(DataType::Str { len: 1 }, None, &RawValue::Str("ABC".into())).is_err());
    }

    #[test]
    fn sentinels_range_and_non_finite() {
        let s = Scaling {
            scale: 0.1,
            offset: 0.0,
            valid_range: Some([-30.0, 70.0]),
            sentinels: vec![32_767],
        };
        assert_eq!(
            to_engineering(&RawValue::I16(32_767), &s),
            Err(CodecError::Sentinel("32767".into()))
        );
        assert!(matches!(
            to_engineering(&RawValue::I16(800), &s),
            Err(CodecError::OutOfRange { .. })
        ));
        assert!(to_engineering(&RawValue::I16(700), &s).is_ok());
        let plain = Scaling::default();
        assert_eq!(
            to_engineering(&RawValue::F32(f32::NAN), &plain),
            Err(CodecError::NonFinite)
        );
        assert_eq!(
            to_engineering(&RawValue::F64(f64::INFINITY), &plain),
            Err(CodecError::NonFinite)
        );
        assert_eq!(to_engineering(&RawValue::Str("x".into()), &plain), Ok(None));
    }

    #[test]
    fn to_raw_rejects_instead_of_rounding() {
        assert_eq!(to_raw(DataType::U16, 30.0, 0.01, 0.0), Ok(RawValue::U16(3000)));
        assert_eq!(
            to_raw(DataType::U16, 30.005, 0.01, 0.0),
            Err(CodecError::NotRepresentable(30.005))
        );
        assert_eq!(
            to_raw(DataType::U16, -1.0, 1.0, 0.0),
            Err(CodecError::OutOfTypeRange(-1.0))
        );
        assert_eq!(
            to_raw(DataType::U16, 65_536.0, 1.0, 0.0),
            Err(CodecError::OutOfTypeRange(65_536.0))
        );
        assert_eq!(to_raw(DataType::I16, -3.5, 0.1, 0.0), Ok(RawValue::I16(-35)));
        assert_eq!(to_raw(DataType::Bool, 1.0, 1.0, 0.0), Ok(RawValue::Bool(true)));
        assert!(to_raw(DataType::Bool, 2.0, 1.0, 0.0).is_err());
        assert!(to_raw(DataType::U16, f64::NAN, 1.0, 0.0).is_err());
        assert!(to_raw(DataType::Str { len: 2 }, 1.0, 1.0, 0.0).is_err());
        assert_eq!(to_raw(DataType::U16, 25.0, 0.1, 20.0), Ok(RawValue::U16(50)));
    }
}
