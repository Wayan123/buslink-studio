//! Per-session token (128 bits from the OS random source) and a constant-time compare.

use ring::rand::{SecureRandom, SystemRandom};

/// 16 random bytes as 32 lowercase hex characters, or `None` if the OS source fails.
pub fn generate() -> Option<String> {
    let mut b = [0u8; 16];
    SystemRandom::new().fill(&mut b).ok()?;
    Some(hex(&b))
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[usize::from(b >> 4)] as char);
        s.push(DIGITS[usize::from(b & 15)] as char);
    }
    s
}

/// Equality without an early exit: the loop always covers the longer input,
/// and a length difference is folded into the result, not branched on.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SplitMix64;

    #[test]
    fn token_is_32_hex_and_differs() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        assert_ne!(a, b);
    }

    /// L2: `ct_eq` agrees with `==` for random inputs, including prefixes and different lengths.
    #[test]
    fn l2_ct_eq_matches_equality() {
        let mut rng = SplitMix64::new(0x1A2B);
        for _ in 0..500 {
            let la = rng.below(40) as usize;
            let a: Vec<u8> = (0..la).map(|_| rng.below(4) as u8).collect();
            let b: Vec<u8> = match rng.below(4) {
                0 => a.clone(),
                1 => a.iter().copied().chain([0u8]).collect(),
                2 => a.iter().copied().take(la / 2).collect(),
                _ => (0..rng.below(40) as usize).map(|_| rng.below(4) as u8).collect(),
            };
            assert_eq!(ct_eq(&a, &b), a == b, "{a:?} vs {b:?}");
        }
        assert!(ct_eq(b"", b""));
        assert!(!ct_eq(b"", b"\0"));
    }
}
