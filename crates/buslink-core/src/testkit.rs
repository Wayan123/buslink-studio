//! Seeded property-test runner without external dependencies (design §12).
//!
//! `check("name", 1000, |rng| { ... })` runs the closure for each case with
//! its own generator. On failure the panic message names the property, the
//! case index and the seed, so the case can be replayed with
//! `BUSLINK_PBT_SEED=<seed> BUSLINK_PBT_CASES=1`.
//! `BUSLINK_PBT_CASES` multiplies coverage without code changes.

use crate::rng::{GOLDEN_GAMMA, SplitMix64};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

pub const DEFAULT_BASE_SEED: u64 = 0xB05_11AC_2026;

fn env_u64(name: &str) -> Option<u64> {
    let raw = std::env::var(name).ok()?;
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        raw.parse().ok()
    }
}

/// Number of cases to run: `BUSLINK_PBT_CASES` if set, else `default`.
pub fn cases(default: u32) -> u32 {
    env_u64("BUSLINK_PBT_CASES")
        .map(|v| v.clamp(1, 10_000_000) as u32)
        .unwrap_or(default)
}

/// Run a property `default_cases` times (or `BUSLINK_PBT_CASES`). Returns the number of cases run.
pub fn check<F>(name: &str, default_cases: u32, mut property: F) -> u32
where
    F: FnMut(&mut SplitMix64),
{
    let n = cases(default_cases);
    let fixed = env_u64("BUSLINK_PBT_SEED");
    let base = DEFAULT_BASE_SEED ^ fnv1a(name.as_bytes());
    for i in 0..n {
        let seed = fixed.unwrap_or_else(|| base ^ u64::from(i).wrapping_mul(GOLDEN_GAMMA));
        let mut rng = SplitMix64::new(seed);
        let result = catch_unwind(AssertUnwindSafe(|| property(&mut rng)));
        if let Err(payload) = result {
            eprintln!(
                "property '{name}' failed at case {i}; replay with BUSLINK_PBT_SEED={seed:#x} BUSLINK_PBT_CASES=1"
            );
            resume_unwind(payload);
        }
    }
    n
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Format bytes as spaced upper-case hex, e.g. `01 03 00 00`.
pub fn hex(bytes: &[u8]) -> String {
    crate::time::hex_spaced(bytes)
}

/// Parse spaced hex (`"01 03 00 00"`) into bytes. Test helper; panics on bad input.
#[allow(clippy::panic)]
pub fn unhex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| match u8::from_str_radix(b, 16) {
            Ok(v) => v,
            Err(_) => panic!("bad hex byte '{b}'"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_requested_cases_deterministically() {
        let mut first = Vec::new();
        let n = check("demo", 5, |r| first.push(r.next_u64()));
        let mut second = Vec::new();
        check("demo", 5, |r| second.push(r.next_u64()));
        assert_eq!(first, second);
        assert_eq!(n as usize, first.len());
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex("01 03 0a FF"), vec![1, 3, 10, 255]);
        assert_eq!(hex(&[1, 3, 10, 255]), "01 03 0A FF");
    }
}
