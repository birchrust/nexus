//! UUID v4 generator (random).
//!
//! UUID v4 consists of 122 random bits plus 6 fixed bits for version and variant.
//! This implementation uses a fast PRNG seeded once at construction, avoiding
//! syscalls on the hot path.

use crate::prng::WyRand;
use crate::types::{Uuid, UuidCompact};

/// UUID v4 generator.
///
/// Generates RFC 9562 compliant UUID v4 values using a fast PRNG.
/// The PRNG is seeded once at construction; subsequent calls to `next()`
/// require no syscalls.
///
/// # Performance
///
/// - Construction with `from_entropy()`: one `getrandom` syscall
/// - `next()` / `next_compact()`: ~35-40 cycles (PRNG + formatting)
/// - Zero allocation
///
/// # Example
///
/// ```rust
/// use nexus_id::uuid::UuidV4;
///
/// let mut generator = UuidV4::new(12345); // deterministic seed
/// let id = generator.next();
/// assert_eq!(id.len(), 36);
///
/// // Or seed from system entropy
/// let mut generator = UuidV4::from_entropy();
/// let id = generator.next();
/// ```
#[derive(Debug, Clone)]
pub struct UuidV4 {
    rng: WyRand,
}

impl UuidV4 {
    /// Create a new generator with an explicit seed.
    ///
    /// Use this for deterministic/reproducible UUID generation in tests.
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self {
            rng: WyRand::new(seed),
        }
    }

    /// Create a new generator seeded from system entropy.
    ///
    /// Makes one `getrandom` syscall. Use this for production.
    pub fn from_entropy() -> Self {
        Self {
            rng: WyRand::from_entropy(),
        }
    }

    /// Generate raw UUID bytes as (hi, lo) pair.
    ///
    /// Returns the 128-bit UUID as two 64-bit values in big-endian byte order.
    /// Version (4) and variant (RFC) bits are already set.
    #[inline]
    pub fn next_raw(&mut self) -> (u64, u64) {
        let rand_hi = self.rng.next_u64();
        let rand_lo = self.rng.next_u64();

        // Set version = 4 (bits 12-15 of hi)
        let hi = (rand_hi & 0xFFFF_FFFF_FFFF_0FFF) | (0x4 << 12);

        // Set variant = 0b10 (bits 62-63 of lo)
        let lo = (rand_lo & 0x3FFF_FFFF_FFFF_FFFF) | (0b10 << 62);

        (hi, lo)
    }

    /// Generate a UUID in standard dashed format.
    ///
    /// Returns a 36-character string: `xxxxxxxx-xxxx-4xxx-Nxxx-xxxxxxxxxxxx`
    /// where N is 8, 9, a, or b (RFC variant).
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Uuid {
        let (hi, lo) = self.next_raw();
        Uuid::from_raw(hi, lo)
    }

    /// Generate a UUID in compact format (no dashes).
    ///
    /// Returns a 32-character hex string.
    #[inline]
    pub fn next_compact(&mut self) -> UuidCompact {
        let (hi, lo) = self.next_raw();
        UuidCompact::from_raw(hi, lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_with_seed() {
        let mut gen1 = UuidV4::new(42);
        let mut gen2 = UuidV4::new(42);

        for _ in 0..10 {
            assert_eq!(gen1.next().as_str(), gen2.next().as_str());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut gen1 = UuidV4::new(42);
        let mut gen2 = UuidV4::new(43);

        assert_ne!(gen1.next().as_str(), gen2.next().as_str());
    }

    #[test]
    fn version_is_4() {
        let mut generator = UuidV4::new(12345);

        for _ in 0..100 {
            let (hi, _lo) = generator.next_raw();
            // Version is in bits 12-15
            let version = (hi >> 12) & 0xF;
            assert_eq!(version, 4);
        }
    }

    #[test]
    fn variant_is_rfc() {
        let mut generator = UuidV4::new(12345);

        for _ in 0..100 {
            let (_hi, lo) = generator.next_raw();
            // Variant is in bits 62-63, should be 0b10
            let variant = (lo >> 62) & 0b11;
            assert_eq!(variant, 0b10);
        }
    }

    #[test]
    fn format_is_correct() {
        let mut generator = UuidV4::new(12345);
        let uuid = generator.next();

        assert_eq!(uuid.len(), 36);

        let s = uuid.as_str();
        // Check dashes at correct positions
        assert_eq!(s.as_bytes()[8], b'-');
        assert_eq!(s.as_bytes()[13], b'-');
        assert_eq!(s.as_bytes()[18], b'-');
        assert_eq!(s.as_bytes()[23], b'-');

        // Check version char is '4'
        assert_eq!(s.as_bytes()[14], b'4');

        // Check variant char is 8, 9, a, or b
        let variant_char = s.as_bytes()[19];
        assert!(
            variant_char == b'8'
                || variant_char == b'9'
                || variant_char == b'a'
                || variant_char == b'b'
        );
    }

    #[test]
    fn compact_format_is_correct() {
        let mut generator = UuidV4::new(12345);
        let uuid = generator.next_compact();

        assert_eq!(uuid.len(), 32);

        // All chars should be hex
        for c in uuid.as_str().chars() {
            assert!(c.is_ascii_hexdigit());
        }
    }

    #[test]
    fn uniqueness() {
        let mut generator = UuidV4::new(12345);
        let mut seen = std::collections::HashSet::new();

        for _ in 0..10000 {
            let uuid = generator.next();
            assert!(seen.insert(uuid.as_str().to_string()));
        }
    }

    #[test]
    fn from_entropy_works() {
        let mut generator = UuidV4::from_entropy();
        let uuid = generator.next();
        assert_eq!(uuid.len(), 36);
    }
}
