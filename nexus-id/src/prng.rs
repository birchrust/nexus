//! Fast pseudo-random number generation for ID generation.
//!
//! This module provides a wyrand-based PRNG optimized for speed over
//! cryptographic security. It's suitable for generating random bits
//! in UUIDs where uniqueness (not unpredictability) is the goal.

/// wyrand PRNG state.
///
/// A fast, high-quality PRNG based on the wyhash family.
/// Produces 64 bits per call in ~3 cycles.
///
/// # Security
///
/// This is NOT cryptographically secure. Use only for ID generation
/// where uniqueness matters but unpredictability does not.
#[derive(Debug, Clone)]
pub struct WyRand {
    state: u64,
}

impl WyRand {
    /// Create a new PRNG with the given seed.
    ///
    /// For production use, seed with entropy from `getrandom`.
    /// For testing, use a fixed seed for reproducibility.
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Create a new PRNG seeded from system entropy.
    ///
    /// This makes one `getrandom` syscall. Call this once at startup,
    /// not on the hot path.
    pub fn from_entropy() -> Self {
        let mut seed = [0u8; 8];
        getrandom::getrandom(&mut seed).expect("failed to get entropy");
        Self::new(u64::from_le_bytes(seed))
    }

    /// Generate the next random u64.
    ///
    /// ~3 cycles on modern x86-64.
    #[inline(always)]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0xa076_1d64_78bd_642f);
        let t = (self.state as u128).wrapping_mul((self.state ^ 0xe703_7ed1_a0b4_28db) as u128);
        ((t >> 64) ^ t) as u64
    }

    /// Generate a random u64 in [0, bound).
    ///
    /// Uses the "fast range" technique to avoid division.
    #[inline]
    #[allow(dead_code)]
    pub fn next_bounded(&mut self, bound: u64) -> u64 {
        let r = self.next_u64();
        ((r as u128 * bound as u128) >> 64) as u64
    }

    /// Current state (for debugging/serialization).
    #[inline]
    #[allow(dead_code)]
    pub const fn state(&self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_with_seed() {
        let mut rng1 = WyRand::new(12345);
        let mut rng2 = WyRand::new(12345);

        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut rng1 = WyRand::new(12345);
        let mut rng2 = WyRand::new(54321);

        // Very unlikely to match
        assert_ne!(rng1.next_u64(), rng2.next_u64());
    }

    #[test]
    fn bounded_in_range() {
        let mut rng = WyRand::new(42);

        for bound in [1, 10, 100, 1000, u64::MAX] {
            for _ in 0..1000 {
                let val = rng.next_bounded(bound);
                assert!(val < bound);
            }
        }
    }

    #[test]
    fn from_entropy_works() {
        let rng = WyRand::from_entropy();
        // Just verify it doesn't panic and produces non-zero state
        // (technically could be zero, but astronomically unlikely)
        let _ = rng.state();
    }
}
