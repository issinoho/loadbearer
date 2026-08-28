//! Small shared helpers: a fast seeded PRNG for workload generation and byte-size
//! constants. Kept dependency-free on purpose — benchmark workloads must stay
//! predictable and identical across platforms.

/// Binary megabyte / gigabyte as `f64`, for turning byte counts into rates.
pub const MIB: f64 = 1024.0 * 1024.0;
pub const GIB: f64 = MIB * 1024.0;

/// Geometric mean of a slice; `0.0` for an empty slice. Each value is floored
/// just above zero so a single zero cannot collapse the whole product.
pub fn geomean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let sum_ln: f64 = values.iter().map(|v| v.max(1e-9).ln()).sum();
    (sum_ln / values.len() as f64).exp()
}

/// SplitMix64 — tiny, fast, good-enough distribution for generating benchmark
/// payloads and random offsets. Not cryptographic.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish value in `[0, bound)`. Carries a negligible modulo bias that
    /// does not matter for workload placement. Panics if `bound` is zero.
    #[inline]
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "below(0)");
        self.next_u64() % bound
    }

    /// Fill `buf` with pseudo-random bytes.
    pub fn fill_bytes(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let word = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_a_seed() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = SplitMix64::new(7);
        for _ in 0..10_000 {
            assert!(r.below(4096) < 4096);
        }
    }
}
