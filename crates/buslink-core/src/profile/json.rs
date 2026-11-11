//! Strict JSON for profiles: duplicate keys are errors, key order is kept,
//! integers and floats stay distinct (as in Python's `json` module, so the
//! Rust and Python validators see the same document).

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;

pub const MAX_PROFILE_BYTES: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    /// JSON number without fraction or exponent, within the 64-bit range.
    Int(i128),
    /// JSON number with a fraction or exponent.
    Float(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, J)]> {
        match self {
            J::Obj(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[J]> {
        match self {
            J::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            J::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Finite number (int or float), as Python's `is_num`.
    pub fn as_num(&self) -> Option<f64> {
        match self {
            J::Int(i) => Some(*i as f64),
            J::Float(f) if f.is_finite() => Some(*f),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, J::Null)
    }

    /// Convert to `serde_json::Value` (for typed deserialisation).
    pub fn to_value(&self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(*b),
            J::Int(i) => {
                if let Ok(v) = i64::try_from(*i) {
                    Value::from(v)
                } else if let Ok(v) = u64::try_from(*i) {
                    Value::from(v)
                } else {
                    Value::Null
                }
            }
            J::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null),
            J::Str(s) => Value::String(s.clone()),
            J::Arr(a) => Value::Array(a.iter().map(J::to_value).collect()),
            J::Obj(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_value())).collect()),
        }
    }
}

impl<'de> Deserialize<'de> for J {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<J, D::Error> {
        d.deserialize_any(JVisitor)
    }
}

struct JVisitor;

impl<'de> Visitor<'de> for JVisitor {
    type Value = J;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_unit<E>(self) -> Result<J, E> {
        Ok(J::Null)
    }
    fn visit_bool<E>(self, v: bool) -> Result<J, E> {
        Ok(J::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<J, E> {
        Ok(J::Int(i128::from(v)))
    }
    fn visit_u64<E>(self, v: u64) -> Result<J, E> {
        Ok(J::Int(i128::from(v)))
    }
    fn visit_f64<E>(self, v: f64) -> Result<J, E> {
        Ok(J::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<J, E> {
        Ok(J::Str(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<J, E> {
        Ok(J::Str(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<J, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element()? {
            out.push(v);
        }
        Ok(J::Arr(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<J, A::Error> {
        let mut out: Vec<(String, J)> = Vec::new();
        while let Some(k) = map.next_key::<String>()? {
            if out.iter().any(|(e, _)| *e == k) {
                return Err(de::Error::custom(format!("duplicate key '{k}'")));
            }
            let v = map.next_value()?;
            out.push((k, v));
        }
        Ok(J::Obj(out))
    }
}

/// Parse with the size limit and duplicate-key check.
pub fn parse(bytes: &[u8]) -> Result<J, String> {
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(format!("file is larger than {MAX_PROFILE_BYTES} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    serde_json::from_str::<J>(text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_int_and_float_apart() {
        let j = parse(br#"{"a": 1, "b": 1.0, "c": 1e2, "d": -5}"#).unwrap();
        assert_eq!(j.get("a"), Some(&J::Int(1)));
        assert_eq!(j.get("b"), Some(&J::Float(1.0)));
        assert_eq!(j.get("c"), Some(&J::Float(100.0)));
        assert_eq!(j.get("d"), Some(&J::Int(-5)));
    }

    #[test]
    fn rejects_duplicates_constants_and_size() {
        assert!(parse(br#"{"a": 1, "a": 2}"#).unwrap_err().contains("duplicate key"));
        assert!(parse(br#"{"a": {"b": 1, "b": 1}}"#).is_err());
        assert!(parse(br#"{"a": NaN}"#).is_err());
        assert!(parse(&vec![b' '; MAX_PROFILE_BYTES + 1]).unwrap_err().contains("larger"));
        assert!(parse(&[0xFF, 0xFE]).is_err());
    }
}
