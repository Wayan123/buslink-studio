//! SplitMix64: a tiny deterministic generator (Steele, Lea, Flood 2014).
//! Used by the property tests and by the simulator's seeded drift and faults.
//! Not for anything security related (tokens use the OS RNG elsewhere).

#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

pub const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    pub fn next_u16(&mut self) -> u16 {
        (self.next_u64() >> 48) as u16
    }

    pub fn next_u8(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }

    /// Uniform in `0..n` (`n > 0`); returns 0 for `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // Lemire's multiply-shift; the tiny bias is irrelevant for tests and simulation.
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// Uniform in `lo..=hi`.
    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        match (hi - lo).checked_add(1) {
            Some(span) => lo + self.below(span),
            None => self.next_u64(),
        }
    }

    pub fn range_usize(&mut self, lo: usize, hi: usize) -> usize {
        self.range_u64(lo as u64, hi as u64) as usize
    }

    /// Uniform in `[0, 1)`.
    pub fn unit_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Approximately standard normal (sum of 4 uniforms, scaled); cheap and bounded.
    pub fn normal(&mut self) -> f64 {
        let s: f64 = (0..4).map(|_| self.unit_f64()).sum();
        (s - 2.0) * (3.0f64).sqrt()
    }

    pub fn chance(&mut self, per_mille: u16) -> bool {
        self.below(1000) < u64::from(per_mille)
    }

    pub fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u8()).collect()
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(self.below(items.len() as u64) as usize)
        }
    }

    /// Derive an independent generator (for per-device streams).
    pub fn fork(&mut self) -> SplitMix64 {
        SplitMix64::new(self.next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_values() {
        // Reference outputs of SplitMix64 for seed 0 (as published with the algorithm).
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(r.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn ranges_are_inclusive_and_bounded() {
        let mut r = SplitMix64::new(42);
        for _ in 0..10_000 {
            let v = r.range_u64(3, 7);
            assert!((3..=7).contains(&v));
            let f = r.unit_f64();
            assert!((0.0..1.0).contains(&f));
        }
        assert_eq!(r.range_u64(5, 5), 5);
        assert_eq!(r.below(0), 0);
        let _ = r.range_u64(0, u64::MAX);
    }
}
