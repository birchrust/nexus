//! Simple multiply-mix hash - baseline for comparison.
//!
//! This is a minimal hash using multiply-xor-shift operations.
//! Not expected to be optimal, but provides a calibration baseline.

/// Multiply-mix constant (golden ratio derived).
const GOLDEN: u64 = 0x9e3779b97f4a7c15;

/// Secondary mixing constant.
const MIX: u64 = 0xbf58476d1ce4e5b9;

/// Read up to 8 bytes as little-endian u64, zero-padded.
#[inline(always)]
fn read_small(data: &[u8]) -> u64 {
    debug_assert!(data.len() <= 8);
    let mut buf = [0u8; 8];
    buf[..data.len()].copy_from_slice(data);
    u64::from_le_bytes(buf)
}

/// Read exactly 8 bytes as little-endian u64.
#[inline(always)]
fn read_u64(data: &[u8]) -> u64 {
    debug_assert!(data.len() >= 8);
    u64::from_le_bytes(data[..8].try_into().unwrap())
}

/// Simple multiply-mix finalizer.
#[inline(always)]
const fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(MIX);
    x ^= x >> 33;
    x
}

/// Hash bytes using simple multiply-mix.
///
/// Returns 64-bit hash. Caller truncates to 48 bits as needed.
#[inline]
pub fn hash(data: &[u8]) -> u64 {
    let len = data.len();
    let mut h = GOLDEN ^ (len as u64);

    // Process 8-byte chunks
    let mut offset = 0;
    while offset + 8 <= len {
        let word = read_u64(&data[offset..]);
        h ^= word;
        h = h.wrapping_mul(GOLDEN);
        h ^= h >> 31;
        offset += 8;
    }

    // Process remaining bytes
    if offset < len {
        let word = read_small(&data[offset..]);
        h ^= word;
        h = h.wrapping_mul(GOLDEN);
    }

    mix(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let h = hash(b"");
        assert_ne!(h, 0);
    }

    #[test]
    fn deterministic() {
        let h1 = hash(b"hello");
        let h2 = hash(b"hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_inputs_different_hashes() {
        let h1 = hash(b"hello");
        let h2 = hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn length_affects_hash() {
        let h1 = hash(b"a");
        let h2 = hash(b"aa");
        assert_ne!(h1, h2);
    }
}
