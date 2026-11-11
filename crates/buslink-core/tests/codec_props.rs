//! Property test P6: codec round trip for every type × word order, and word-order algebra.
#![allow(clippy::unwrap_used)]

use buslink_core::codec::{self, DataType, RawValue, WordOrder};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;

fn random_value(r: &mut SplitMix64, ty: DataType) -> RawValue {
    match ty {
        DataType::Bool => RawValue::Bool(r.bool()),
        DataType::U16 => RawValue::U16(r.next_u16()),
        DataType::I16 => RawValue::I16(r.next_u16() as i16),
        DataType::Bitfield => RawValue::Bits(r.next_u16()),
        DataType::U32 => RawValue::U32(r.next_u32()),
        DataType::I32 => RawValue::I32(r.next_u32() as i32),
        // Any bit pattern, NaN included: compare bit patterns below.
        DataType::F32 => RawValue::F32(f32::from_bits(r.next_u32())),
        DataType::U64 => RawValue::U64(r.next_u64()),
        DataType::I64 => RawValue::I64(r.next_u64() as i64),
        DataType::F64 => RawValue::F64(f64::from_bits(r.next_u64())),
        DataType::Str { len } => {
            let n = r.range_usize(0, usize::from(len) * 2);
            let mut s: String = (0..n).map(|_| char::from(r.range_u64(0x21, 0x7E) as u8)).collect();
            // Decoding trims trailing padding; generated text never ends with a space.
            while s.ends_with(' ') {
                s.pop();
            }
            RawValue::Str(s)
        }
    }
}

fn same(a: &RawValue, b: &RawValue) -> bool {
    match (a, b) {
        (RawValue::F32(x), RawValue::F32(y)) => x.to_bits() == y.to_bits(),
        (RawValue::F64(x), RawValue::F64(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

const TYPES: [DataType; 11] = [
    DataType::Bool,
    DataType::U16,
    DataType::I16,
    DataType::Bitfield,
    DataType::U32,
    DataType::I32,
    DataType::F32,
    DataType::U64,
    DataType::I64,
    DataType::F64,
    DataType::Str { len: 6 },
];

#[test]
fn round_trip_every_type_and_order() {
    let mut total = 0;
    for ty in TYPES {
        let orders: Vec<Option<WordOrder>> = match ty {
            DataType::Str { .. } => vec![Some(WordOrder::ABCD), Some(WordOrder::BADC)],
            t if t.width() >= 2 => WordOrder::ALL.iter().copied().map(Some).collect(),
            _ => vec![None],
        };
        for order in orders {
            total += check(&format!("codec_{ty:?}_{order:?}"), 1000, |r| {
                let v = random_value(r, ty);
                let regs = codec::encode(ty, order, &v).unwrap();
                assert_eq!(regs.len(), usize::from(ty.width()));
                let back = codec::decode(ty, order, &regs).unwrap();
                assert!(same(&v, &back), "{ty:?} {order:?}: {v:?} -> {regs:04X?} -> {back:?}");
            });
        }
    }
    assert!(total >= 30_000);
}

#[test]
fn word_order_algebra() {
    check("word_order_algebra", 1000, |r| {
        let n = if r.bool() { 2 } else { 4 };
        let regs: Vec<u16> = (0..n).map(|_| r.next_u16()).collect();
        for o in WordOrder::ALL {
            assert_eq!(o.apply(&o.apply(&regs)), regs, "{o:?} is an involution");
        }
        assert_eq!(WordOrder::DCBA.apply(&regs), WordOrder::CDAB.apply(&WordOrder::BADC.apply(&regs)));
        assert_eq!(WordOrder::DCBA.apply(&regs), WordOrder::BADC.apply(&WordOrder::CDAB.apply(&regs)));
    });
}

#[test]
fn decode_never_panics() {
    check("codec_no_panic", 5000, |r| {
        let ty = *r.pick(&TYPES).unwrap();
        let n = r.range_usize(0, 8);
        let regs: Vec<u16> = (0..n).map(|_| r.next_u16()).collect();
        let order = r.pick(&WordOrder::ALL).copied();
        let _ = codec::decode(ty, order, &regs);
        let _ = codec::decode_all_orders(ty, &regs);
        let _ = codec::to_raw(ty, f64::from_bits(r.next_u64()), f64::from_bits(r.next_u64()), 0.0);
    });
}

#[test]
fn engineering_to_raw_round_trip() {
    check("to_raw_round_trip", 2000, |r| {
        let raw = r.next_u16();
        let scale = *r.pick(&[1.0, 0.1, 0.01, 0.5, 10.0]).unwrap();
        let offset = *r.pick(&[0.0, -40.0, 273.15]).unwrap();
        let eng = f64::from(raw) * scale + offset;
        assert_eq!(codec::to_raw(DataType::U16, eng, scale, offset), Ok(RawValue::U16(raw)));
    });
}
