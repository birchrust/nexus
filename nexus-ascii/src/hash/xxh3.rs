//! XXH3 - Fast hash optimized for both small and large inputs.
//!
//! This is the scalar implementation of XXH3-64. XXH3 is designed to be
//! fast across all input sizes, with special optimizations for small inputs.
//!
//! Reference: <https://github.com/Cyan4973/xxHash>

/// XXH3 secret (first 192 bytes of the default secret).
#[rustfmt::skip]
pub(super) const SECRET: [u8; 192] = [
    0xb8, 0xfe, 0x6c, 0x39, 0x23, 0xa4, 0x4b, 0xbe, 0x7c, 0x01, 0x81, 0x2c, 0xf7, 0x21, 0xad, 0x1c,
    0xde, 0xd4, 0x6d, 0xe9, 0x83, 0x90, 0x97, 0xdb, 0x72, 0x40, 0xa4, 0xa4, 0xb7, 0xb3, 0x67, 0x1f,
    0xcb, 0x79, 0xe6, 0x4e, 0xcc, 0xc0, 0xe5, 0x78, 0x82, 0x5a, 0xd0, 0x7d, 0xcc, 0xff, 0x72, 0x21,
    0xb8, 0x08, 0x46, 0x74, 0xf7, 0x43, 0x24, 0x8e, 0xe0, 0x35, 0x90, 0xe6, 0x81, 0x3a, 0x26, 0x4c,
    0x3c, 0x28, 0x52, 0xbb, 0x91, 0xc3, 0x00, 0xcb, 0x88, 0xd0, 0x65, 0x8b, 0x1b, 0x53, 0x2e, 0xa3,
    0x71, 0x64, 0x48, 0x97, 0xa2, 0x0d, 0xf9, 0x4e, 0x38, 0x19, 0xef, 0x46, 0xa9, 0xde, 0xac, 0xd8,
    0xa8, 0xfa, 0x76, 0x3f, 0xe3, 0x9c, 0x34, 0x3f, 0xf9, 0xdc, 0xbb, 0xc7, 0xc7, 0x0b, 0x4f, 0x1d,
    0x8a, 0x51, 0xe0, 0x4b, 0xcd, 0xb4, 0x59, 0x31, 0xc8, 0x9f, 0x7e, 0xc9, 0xd9, 0x78, 0x73, 0x64,
    0xea, 0xc5, 0xac, 0x83, 0x34, 0xd3, 0xeb, 0xc3, 0xc5, 0x81, 0xa0, 0xff, 0xfa, 0x13, 0x63, 0xeb,
    0x17, 0x0d, 0xdd, 0x51, 0xb7, 0xf0, 0xda, 0x49, 0xd3, 0x16, 0xca, 0xca, 0x89, 0x46, 0x5c, 0xd7,
    0x9c, 0x44, 0x8b, 0xed, 0x3f, 0x41, 0x66, 0x25, 0x87, 0x5f, 0x1f, 0x0b, 0x4e, 0x2a, 0xbc, 0x3f,
    0xab, 0x82, 0xb5, 0xdf, 0x4c, 0x54, 0xc0, 0x24, 0x56, 0xe5, 0x4e, 0xea, 0xea, 0xb3, 0xd1, 0xf6,
];

pub(super) const PRIME32_1: u64 = 0x9E37_79B1;
pub(super) const PRIME32_2: u64 = 0x85EB_CA77;
pub(super) const PRIME64_1: u64 = 0x9E37_79B1_85EB_CA87;
pub(super) const PRIME64_2: u64 = 0xC2B2_AE3D_27D4_EB4F;

/// Read 4 bytes as little-endian u32.
#[inline(always)]
fn read_u32(data: &[u8]) -> u32 {
    u32::from_le_bytes(data[..4].try_into().unwrap())
}

/// Read 8 bytes as little-endian u64.
#[inline(always)]
pub(super) fn read_u64(data: &[u8]) -> u64 {
    u64::from_le_bytes(data[..8].try_into().unwrap())
}

/// 64-bit multiply, return high and low parts XORed.
#[inline(always)]
pub(super) const fn mul128_fold64(lhs: u64, rhs: u64) -> u64 {
    let product = (lhs as u128).wrapping_mul(rhs as u128);
    (product as u64) ^ ((product >> 64) as u64)
}

/// XOR and fold to 64 bits.
#[inline(always)]
const fn xorshift64(v: u64, shift: u32) -> u64 {
    v ^ (v >> shift)
}

/// Avalanche finalizer.
#[inline(always)]
pub(super) const fn avalanche(mut h: u64) -> u64 {
    h = xorshift64(h, 37);
    h = h.wrapping_mul(PRIME64_2);
    h = xorshift64(h, 32);
    h
}

/// Read secret as u64.
#[inline(always)]
pub(super) fn secret_u64(offset: usize) -> u64 {
    read_u64(&SECRET[offset..])
}

/// Read secret as u32.
#[inline(always)]
fn secret_u32(offset: usize) -> u32 {
    read_u32(&SECRET[offset..])
}

/// Hash 1-3 bytes.
#[inline(always)]
fn hash_len_1to3(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    debug_assert!((1..=3).contains(&len));

    let c1 = data[0] as u64;
    let c2 = data[len >> 1] as u64;
    let c3 = data[len - 1] as u64;

    let combined = (c1 << 16) | (c2 << 24) | c3 | ((len as u64) << 8);
    let keyed = combined ^ ((secret_u32(0) as u64).wrapping_add(seed));
    avalanche(keyed.wrapping_mul(PRIME64_1))
}

/// Hash 4-8 bytes.
#[inline(always)]
fn hash_len_4to8(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    debug_assert!((4..=8).contains(&len));

    let input1 = read_u32(data) as u64;
    let input2 = read_u32(&data[len - 4..]) as u64;
    let input64 = input1.wrapping_add(input2 << 32);
    let keyed = input64 ^ (secret_u64(8).wrapping_sub(seed));
    let mut h = (len as u64).wrapping_add(mul128_fold64(keyed, PRIME64_1));
    h = xorshift64(h, 35);
    h = h.wrapping_mul(PRIME32_2);
    h = xorshift64(h, 28);
    h
}

/// Hash 9-16 bytes.
#[inline(always)]
fn hash_len_9to16(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    debug_assert!((9..=16).contains(&len));

    let input_lo = read_u64(data) ^ (secret_u64(24).wrapping_add(seed));
    let input_hi = read_u64(&data[len - 8..]) ^ (secret_u64(32).wrapping_sub(seed));
    let acc = (len as u64)
        .wrapping_add(input_lo)
        .wrapping_add(input_hi)
        .wrapping_add(mul128_fold64(input_lo, input_hi));
    avalanche(acc)
}

/// Hash 17-128 bytes.
#[inline]
fn hash_len_17to128(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    debug_assert!((17..=128).contains(&len));

    let mut acc = (len as u64).wrapping_mul(PRIME64_1);

    // Process N chunks from start and N chunks from end, where 2*N covers the input
    let num_rounds = ((len - 1) >> 5) + 1; // ((len-1) / 32) + 1

    for i in (0..num_rounds).rev() {
        let offset_start = i * 16;
        let offset_end = len - i * 16 - 16;
        acc = acc.wrapping_add(mix_step(&data[offset_start..], i * 32, seed));
        acc = acc.wrapping_add(mix_step(&data[offset_end..], i * 32 + 16, seed));
    }

    avalanche(acc)
}

/// Mix step for medium-length inputs.
#[inline(always)]
fn mix_step(data: &[u8], secret_offset: usize, seed: u64) -> u64 {
    let input_lo = read_u64(data);
    let input_hi = read_u64(&data[8..]);
    mul128_fold64(
        input_lo ^ (secret_u64(secret_offset).wrapping_add(seed)),
        input_hi ^ (secret_u64(secret_offset + 8).wrapping_sub(seed)),
    )
}

/// Hash 129-240 bytes.
#[inline]
fn hash_len_129to240(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    debug_assert!((129..=240).contains(&len));

    let mut acc = (len as u64).wrapping_mul(PRIME64_1);

    // Process first 128 bytes in 16-byte chunks
    for i in 0..8 {
        acc = acc.wrapping_add(mix_step(&data[i * 16..], i * 16, seed));
    }
    acc = avalanche(acc);

    // Process remaining chunks
    let nb_rounds = (len - 1) / 16;
    for i in 8..nb_rounds {
        acc = acc.wrapping_add(mix_step(&data[i * 16..], (i - 8) * 16 + 3, seed));
    }

    // Final mix
    acc = acc.wrapping_add(mix_step(&data[len - 16..], 119, seed));
    avalanche(acc)
}

/// Hash long inputs (> 240 bytes).
#[inline]
fn hash_long(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    debug_assert!(len > 240);

    // Initialize accumulators
    let mut acc = [
        PRIME32_1,
        PRIME64_1,
        PRIME64_2,
        (0_u64.wrapping_sub(PRIME64_1)),
        PRIME32_2,
        PRIME64_2,
        (0_u64.wrapping_sub(PRIME64_2)),
        PRIME32_1,
    ];

    // Add seed to accumulators
    if seed != 0 {
        acc[0] = acc[0].wrapping_add(seed);
        acc[1] = acc[1].wrapping_sub(seed);
        acc[2] = acc[2].wrapping_add(seed);
        acc[3] = acc[3].wrapping_sub(seed);
        acc[4] = acc[4].wrapping_add(seed);
        acc[5] = acc[5].wrapping_sub(seed);
        acc[6] = acc[6].wrapping_add(seed);
        acc[7] = acc[7].wrapping_sub(seed);
    }

    // Process 1024-byte blocks
    let block_len = 1024;
    let stripe_len = 64;
    let nb_stripes_per_block = block_len / stripe_len;
    let nb_blocks = (len - 1) / block_len;

    for n in 0..nb_blocks {
        for s in 0..nb_stripes_per_block {
            let stripe_offset = n * block_len + s * stripe_len;
            accumulate_stripe(&mut acc, &data[stripe_offset..], s * 8);
        }
        scramble_acc(&mut acc);
    }

    // Process remaining stripes
    let last_block_offset = nb_blocks * block_len;
    let nb_stripes = ((len - 1) - last_block_offset) / stripe_len;

    for s in 0..nb_stripes {
        let stripe_offset = last_block_offset + s * stripe_len;
        accumulate_stripe(&mut acc, &data[stripe_offset..], s * 8);
    }

    // Final stripe
    accumulate_stripe(&mut acc, &data[len - 64..], 121);

    // Merge accumulators
    merge_accs(&acc, len as u64)
}

/// Accumulate a 64-byte stripe.
#[inline(always)]
fn accumulate_stripe(acc: &mut [u64; 8], stripe: &[u8], secret_offset: usize) {
    for i in 0..8 {
        let data_val = read_u64(&stripe[i * 8..]);
        let secret_val = secret_u64(secret_offset + i * 8);
        acc[i] = acc[i].wrapping_add(data_val ^ secret_val);
        acc[i] = acc[i].wrapping_add((data_val & 0xFFFF_FFFF).wrapping_mul(data_val >> 32));
    }
}

/// Scramble accumulators between blocks.
#[inline(always)]
fn scramble_acc(acc: &mut [u64; 8]) {
    for i in 0..8 {
        acc[i] ^= acc[i] >> 47;
        acc[i] ^= secret_u64(128 + i * 8);
        acc[i] = acc[i].wrapping_mul(PRIME32_1);
    }
}

/// Merge accumulators into final hash.
#[inline]
pub(super) fn merge_accs(acc: &[u64; 8], len: u64) -> u64 {
    let mut result = len.wrapping_mul(PRIME64_1);

    result = result.wrapping_add(mul128_fold64(
        acc[0] ^ secret_u64(11),
        acc[1] ^ secret_u64(19),
    ));
    result = result.wrapping_add(mul128_fold64(
        acc[2] ^ secret_u64(27),
        acc[3] ^ secret_u64(35),
    ));
    result = result.wrapping_add(mul128_fold64(
        acc[4] ^ secret_u64(43),
        acc[5] ^ secret_u64(51),
    ));
    result = result.wrapping_add(mul128_fold64(
        acc[6] ^ secret_u64(59),
        acc[7] ^ secret_u64(67),
    ));

    avalanche(result)
}

/// Hash bytes using XXH3-64.
///
/// Returns 64-bit hash. Caller truncates to 48 bits as needed.
#[cfg(test)]
#[inline]
pub fn hash(data: &[u8]) -> u64 {
    hash_with_seed(data, 0)
}

/// Hash with compile-time capacity bound.
///
/// When the maximum capacity is known at compile time (e.g., `AsciiString<32>`),
/// the compiler eliminates unreachable size paths, reducing branches.
///
/// For `CAP <= 128`, no medium (129-240) or large (>240) paths are generated.
/// This is ideal for fixed-size identifiers like order IDs (32 bytes) or
/// short text fields (80-120 bytes).
#[cfg(test)]
#[inline]
pub fn hash_bounded<const CAP: usize>(data: &[u8]) -> u64 {
    hash_bounded_with_seed::<CAP>(data, 0)
}

/// Hash with compile-time capacity bound and seed.
#[inline]
pub fn hash_bounded_with_seed<const CAP: usize>(data: &[u8], seed: u64) -> u64 {
    debug_assert!(data.len() <= CAP);
    let len = data.len();

    // Empty input - always possible
    if len == 0 {
        return avalanche(seed ^ (secret_u64(56) ^ secret_u64(64)));
    }

    // CAP <= 3: only 1-3 path possible
    if CAP <= 3 {
        return hash_len_1to3(data, seed);
    }

    // CAP <= 8: 1-3 or 4-8
    if CAP <= 8 {
        if len <= 3 {
            return hash_len_1to3(data, seed);
        }
        return hash_len_4to8(data, seed);
    }

    // CAP <= 16: 1-3, 4-8, or 9-16
    if CAP <= 16 {
        if len <= 3 {
            return hash_len_1to3(data, seed);
        }
        if len <= 8 {
            return hash_len_4to8(data, seed);
        }
        return hash_len_9to16(data, seed);
    }

    // CAP <= 128: all small paths (order IDs, short text fields)
    if CAP <= 128 {
        if len <= 3 {
            return hash_len_1to3(data, seed);
        }
        if len <= 8 {
            return hash_len_4to8(data, seed);
        }
        if len <= 16 {
            return hash_len_9to16(data, seed);
        }
        return hash_len_17to128(data, seed);
    }

    // CAP <= 240: small + medium, no SIMD/long path
    if CAP <= 240 {
        if len <= 3 {
            return hash_len_1to3(data, seed);
        }
        if len <= 8 {
            return hash_len_4to8(data, seed);
        }
        if len <= 16 {
            return hash_len_9to16(data, seed);
        }
        if len <= 128 {
            return hash_len_17to128(data, seed);
        }
        return hash_len_129to240(data, seed);
    }

    // CAP > 240: full dispatch (rare for fixed-size strings)
    hash_with_seed(data, seed)
}

/// Hash bytes with a seed.
#[inline]
pub fn hash_with_seed(data: &[u8], seed: u64) -> u64 {
    let len = data.len();

    if len == 0 {
        avalanche(seed ^ (secret_u64(56) ^ secret_u64(64)))
    } else if len <= 3 {
        hash_len_1to3(data, seed)
    } else if len <= 8 {
        hash_len_4to8(data, seed)
    } else if len <= 16 {
        hash_len_9to16(data, seed)
    } else if len <= 128 {
        hash_len_17to128(data, seed)
    } else if len <= 240 {
        hash_len_129to240(data, seed)
    } else {
        hash_long(data, seed)
    }
}

// =============================================================================
// Const-compatible implementation (for compile-time hashing)
// =============================================================================

/// Read 4 bytes as little-endian u32 (const-compatible).
#[inline(always)]
const fn read_u32_const(data: &[u8], offset: usize) -> u32 {
    (data[offset] as u32)
        | ((data[offset + 1] as u32) << 8)
        | ((data[offset + 2] as u32) << 16)
        | ((data[offset + 3] as u32) << 24)
}

/// Read 8 bytes as little-endian u64 (const-compatible).
#[inline(always)]
const fn read_u64_const(data: &[u8], offset: usize) -> u64 {
    (data[offset] as u64)
        | ((data[offset + 1] as u64) << 8)
        | ((data[offset + 2] as u64) << 16)
        | ((data[offset + 3] as u64) << 24)
        | ((data[offset + 4] as u64) << 32)
        | ((data[offset + 5] as u64) << 40)
        | ((data[offset + 6] as u64) << 48)
        | ((data[offset + 7] as u64) << 56)
}

/// Read secret as u64 (const-compatible).
#[inline(always)]
const fn secret_u64_const(offset: usize) -> u64 {
    read_u64_const(&SECRET, offset)
}

/// Read secret as u32 (const-compatible).
#[inline(always)]
const fn secret_u32_const(offset: usize) -> u32 {
    read_u32_const(&SECRET, offset)
}

/// Hash 1-3 bytes (const).
#[inline(always)]
const fn hash_len_1to3_const(data: &[u8], len: usize, seed: u64) -> u64 {
    let c1 = data[0] as u64;
    let c2 = data[len >> 1] as u64;
    let c3 = data[len - 1] as u64;

    let combined = (c1 << 16) | (c2 << 24) | c3 | ((len as u64) << 8);
    let keyed = combined ^ ((secret_u32_const(0) as u64).wrapping_add(seed));
    avalanche(keyed.wrapping_mul(PRIME64_1))
}

/// Hash 4-8 bytes (const).
#[inline(always)]
const fn hash_len_4to8_const(data: &[u8], len: usize, seed: u64) -> u64 {
    let input1 = read_u32_const(data, 0) as u64;
    let input2 = read_u32_const(data, len - 4) as u64;
    let input64 = input1.wrapping_add(input2 << 32);
    let keyed = input64 ^ (secret_u64_const(8).wrapping_sub(seed));
    let mut h = (len as u64).wrapping_add(mul128_fold64(keyed, PRIME64_1));
    h = xorshift64(h, 35);
    h = h.wrapping_mul(PRIME32_2);
    xorshift64(h, 28)
}

/// Hash 9-16 bytes (const).
#[inline(always)]
const fn hash_len_9to16_const(data: &[u8], len: usize, seed: u64) -> u64 {
    let input_lo = read_u64_const(data, 0) ^ (secret_u64_const(24).wrapping_add(seed));
    let input_hi = read_u64_const(data, len - 8) ^ (secret_u64_const(32).wrapping_sub(seed));
    let acc = (len as u64)
        .wrapping_add(input_lo)
        .wrapping_add(input_hi)
        .wrapping_add(mul128_fold64(input_lo, input_hi));
    avalanche(acc)
}

/// Mix step for medium-length inputs (const).
#[inline(always)]
const fn mix_step_const(data: &[u8], data_offset: usize, secret_offset: usize, seed: u64) -> u64 {
    let input_lo = read_u64_const(data, data_offset);
    let input_hi = read_u64_const(data, data_offset + 8);
    mul128_fold64(
        input_lo ^ (secret_u64_const(secret_offset).wrapping_add(seed)),
        input_hi ^ (secret_u64_const(secret_offset + 8).wrapping_sub(seed)),
    )
}

/// Hash 17-128 bytes (const).
///
/// Uses unrolled if-chains instead of loops since const fn loops
/// with runtime-dependent bounds are complex.
#[inline]
const fn hash_len_17to128_const(data: &[u8], len: usize, seed: u64) -> u64 {
    let mut acc = (len as u64).wrapping_mul(PRIME64_1);

    // Unroll based on length ranges (matches runtime loop behavior)
    // len 97-128: 4 rounds (i=3,2,1,0)
    // len 65-96:  3 rounds (i=2,1,0)
    // len 33-64:  2 rounds (i=1,0)
    // len 17-32:  1 round  (i=0)

    if len > 96 {
        // i=3
        acc = acc.wrapping_add(mix_step_const(data, 48, 96, seed));
        acc = acc.wrapping_add(mix_step_const(data, len - 64, 112, seed));
    }
    if len > 64 {
        // i=2
        acc = acc.wrapping_add(mix_step_const(data, 32, 64, seed));
        acc = acc.wrapping_add(mix_step_const(data, len - 48, 80, seed));
    }
    if len > 32 {
        // i=1
        acc = acc.wrapping_add(mix_step_const(data, 16, 32, seed));
        acc = acc.wrapping_add(mix_step_const(data, len - 32, 48, seed));
    }
    // Always: i=0
    acc = acc.wrapping_add(mix_step_const(data, 0, 0, seed));
    acc = acc.wrapping_add(mix_step_const(data, len - 16, 16, seed));

    avalanche(acc)
}

/// Const-compatible XXH3 hash for small inputs (≤128 bytes).
///
/// This function produces identical hashes to the runtime version but
/// can be evaluated at compile time. Only supports inputs up to 128 bytes,
/// which covers all practical `from_static` use cases.
///
/// # Panics
///
/// Panics at compile time if `CAP > 128`.
#[inline]
pub const fn hash_const<const CAP: usize>(data: &[u8], seed: u64) -> u64 {
    assert!(CAP <= 128, "hash_const only supports CAP <= 128");

    let len = data.len();

    if len == 0 {
        return avalanche(seed ^ (secret_u64_const(56) ^ secret_u64_const(64)));
    }

    if len <= 3 {
        return hash_len_1to3_const(data, len, seed);
    }

    if len <= 8 {
        return hash_len_4to8_const(data, len, seed);
    }

    if len <= 16 {
        return hash_len_9to16_const(data, len, seed);
    }

    hash_len_17to128_const(data, len, seed)
}

/// Const-compatible XXH3 hash without seed.
#[cfg(test)]
#[inline]
pub const fn hash_const_no_seed<const CAP: usize>(data: &[u8]) -> u64 {
    hash_const::<CAP>(data, 0)
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
    fn various_lengths() {
        // Test all length categories
        let _ = hash(b"");                        // 0 bytes
        let _ = hash(b"a");                       // 1 byte
        let _ = hash(b"ab");                      // 2 bytes
        let _ = hash(b"abc");                     // 3 bytes
        let _ = hash(b"abcd");                    // 4 bytes
        let _ = hash(b"abcdefgh");                // 8 bytes
        let _ = hash(b"abcdefghi");               // 9 bytes
        let _ = hash(b"abcdefghijklmnop");        // 16 bytes
        let _ = hash(b"abcdefghijklmnopq");       // 17 bytes
        let _ = hash(&[0u8; 64]);                 // 64 bytes
        let _ = hash(&[0u8; 128]);                // 128 bytes
        let _ = hash(&[0u8; 129]);                // 129 bytes
        let _ = hash(&[0u8; 240]);                // 240 bytes
        let _ = hash(&[0u8; 241]);                // 241 bytes
        let _ = hash(&[0u8; 1024]);               // 1024 bytes
        let _ = hash(&[0u8; 2048]);               // 2048 bytes
    }

    #[test]
    fn seed_affects_hash() {
        let h1 = hash_with_seed(b"hello", 0);
        let h2 = hash_with_seed(b"hello", 1);
        assert_ne!(h1, h2);
    }

    // =========================================================================
    // hash_bounded tests
    // =========================================================================

    #[test]
    fn bounded_matches_unbounded_cap_8() {
        for len in 0..=8 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_bounded = hash_bounded::<8>(&data);
            let h_unbounded = hash(&data);
            assert_eq!(h_bounded, h_unbounded, "mismatch at len={}", len);
        }
    }

    #[test]
    fn bounded_matches_unbounded_cap_16() {
        for len in 0..=16 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_bounded = hash_bounded::<16>(&data);
            let h_unbounded = hash(&data);
            assert_eq!(h_bounded, h_unbounded, "mismatch at len={}", len);
        }
    }

    #[test]
    fn bounded_matches_unbounded_cap_32() {
        // Typical order ID size
        for len in 0..=32 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_bounded = hash_bounded::<32>(&data);
            let h_unbounded = hash(&data);
            assert_eq!(h_bounded, h_unbounded, "mismatch at len={}", len);
        }
    }

    #[test]
    fn bounded_matches_unbounded_cap_128() {
        // Covers all small paths (order IDs, short text fields)
        for len in 0..=128 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_bounded = hash_bounded::<128>(&data);
            let h_unbounded = hash(&data);
            assert_eq!(h_bounded, h_unbounded, "mismatch at len={}", len);
        }
    }

    #[test]
    fn bounded_matches_unbounded_cap_240() {
        // Covers small + medium paths
        for len in 0..=240 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_bounded = hash_bounded::<240>(&data);
            let h_unbounded = hash(&data);
            assert_eq!(h_bounded, h_unbounded, "mismatch at len={}", len);
        }
    }

    #[test]
    fn bounded_matches_unbounded_cap_large() {
        // Large capacity falls back to full dispatch
        for &len in &[0, 1, 8, 16, 32, 64, 128, 200, 240, 256, 512, 1024] {
            let data: Vec<u8> = (0..len).map(|i| (i % 256) as u8).collect();
            let h_bounded = hash_bounded::<1024>(&data);
            let h_unbounded = hash(&data);
            assert_eq!(h_bounded, h_unbounded, "mismatch at len={}", len);
        }
    }

    #[test]
    fn bounded_with_seed() {
        let data = b"hello world";
        let h1 = hash_bounded_with_seed::<32>(data, 12345);
        let h2 = hash_with_seed(data, 12345);
        assert_eq!(h1, h2);
    }

    // =========================================================================
    // hash_const tests - verify const version matches runtime version
    // =========================================================================

    #[test]
    fn const_matches_runtime_empty() {
        let h_const = hash_const::<128>(b"", 0);
        let h_runtime = hash(b"");
        assert_eq!(h_const, h_runtime, "empty string mismatch");
    }

    #[test]
    fn const_matches_runtime_1to3_bytes() {
        // Test all lengths in 1-3 range
        for len in 1..=3 {
            let data: Vec<u8> = (0..len).map(|i| (i + 0x41) as u8).collect(); // A, B, C...
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }

        // Test specific patterns
        assert_eq!(hash_const::<128>(b"a", 0), hash(b"a"));
        assert_eq!(hash_const::<128>(b"ab", 0), hash(b"ab"));
        assert_eq!(hash_const::<128>(b"abc", 0), hash(b"abc"));
        assert_eq!(hash_const::<128>(b"XYZ", 0), hash(b"XYZ"));
    }

    #[test]
    fn const_matches_runtime_4to8_bytes() {
        // Test all lengths in 4-8 range
        for len in 4..=8 {
            let data: Vec<u8> = (0..len).map(|i| (i + 0x30) as u8).collect(); // 0, 1, 2...
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }

        // Test specific patterns
        assert_eq!(hash_const::<128>(b"abcd", 0), hash(b"abcd"));
        assert_eq!(hash_const::<128>(b"BTC-USD", 0), hash(b"BTC-USD"));
        assert_eq!(hash_const::<128>(b"ETH-USDT", 0), hash(b"ETH-USDT"));
    }

    #[test]
    fn const_matches_runtime_9to16_bytes() {
        // Test all lengths in 9-16 range
        for len in 9..=16 {
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }

        // Test specific patterns
        assert_eq!(hash_const::<128>(b"123456789", 0), hash(b"123456789"));
        assert_eq!(hash_const::<128>(b"order-id-1234", 0), hash(b"order-id-1234"));
        assert_eq!(hash_const::<128>(b"1234567890123456", 0), hash(b"1234567890123456"));
    }

    #[test]
    fn const_matches_runtime_17to32_bytes() {
        // Test all lengths in 17-32 range
        for len in 17..=32 {
            let data: Vec<u8> = (0..len).map(|i| (i * 11 + 5) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }

        // Test specific patterns
        assert_eq!(
            hash_const::<128>(b"12345678901234567", 0),
            hash(b"12345678901234567")
        );
        assert_eq!(
            hash_const::<128>(b"ORDER-ID-1234567890123456", 0),
            hash(b"ORDER-ID-1234567890123456")
        );
    }

    #[test]
    fn const_matches_runtime_33to64_bytes() {
        // Test all lengths in 33-64 range
        for len in 33..=64 {
            let data: Vec<u8> = (0..len).map(|i| (i * 13 + 17) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }
    }

    #[test]
    fn const_matches_runtime_65to96_bytes() {
        // Test all lengths in 65-96 range
        for len in 65..=96 {
            let data: Vec<u8> = (0..len).map(|i| (i * 17 + 23) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }
    }

    #[test]
    fn const_matches_runtime_97to128_bytes() {
        // Test all lengths in 97-128 range
        for len in 97..=128 {
            let data: Vec<u8> = (0..len).map(|i| (i * 19 + 29) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }
    }

    #[test]
    fn const_matches_runtime_all_lengths_0_to_128() {
        // Comprehensive test: every length from 0 to 128
        for len in 0..=128 {
            let data: Vec<u8> = (0..len).map(|i| (i % 256) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }
    }

    #[test]
    fn const_matches_runtime_with_various_seeds() {
        let seeds: [u64; 6] = [0, 1, 42, 12345, u64::MAX, 0xDEADBEEF];

        for seed in seeds {
            for len in [0, 1, 3, 4, 8, 9, 16, 17, 32, 64, 128] {
                let data: Vec<u8> = (0..len).map(|i| (i % 256) as u8).collect();
                let h_const = hash_const::<128>(&data, seed);
                let h_runtime = hash_with_seed(&data, seed);
                assert_eq!(h_const, h_runtime, "mismatch at len={}, seed={}", len, seed);
            }
        }
    }

    #[test]
    fn const_matches_runtime_realistic_strings() {
        // Test realistic identifier patterns
        let test_cases: &[&[u8]] = &[
            b"",
            b"X",
            b"BTC",
            b"ETH-USD",
            b"BTCUSDT",
            b"SOL-PERP",
            b"order-12345",
            b"trade_id_abc123",
            b"ORDER-2024-01-20-001",
            b"BINANCE:BTCUSDT:PERP:LINEAR",
            b"very-long-order-identifier-that-spans-many-bytes-for-testing",
            b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", // 64 bytes
            b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", // 128 bytes
        ];

        for &data in test_cases {
            if data.len() <= 128 {
                let h_const = hash_const::<128>(data, 0);
                let h_runtime = hash(data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for {:?} (len={})",
                    String::from_utf8_lossy(data),
                    data.len()
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_edge_cases() {
        // All zeros
        for len in [1, 3, 4, 8, 9, 16, 17, 32, 64, 128] {
            let data = vec![0u8; len];
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "zeros mismatch at len={}", len);
        }

        // All 0xFF
        for len in [1, 3, 4, 8, 9, 16, 17, 32, 64, 128] {
            let data = vec![0xFFu8; len];
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "0xFF mismatch at len={}", len);
        }

        // Alternating bytes
        for len in [2, 4, 8, 16, 32, 64, 128] {
            let data: Vec<u8> = (0..len).map(|i| if i % 2 == 0 { 0x55 } else { 0xAA }).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "alternating mismatch at len={}", len);
        }
    }

    #[test]
    fn const_matches_runtime_boundary_lengths() {
        // Test exact boundary lengths between hash functions
        let boundary_lengths = [
            1, 2, 3,           // 1-3 range
            4, 7, 8,           // 4-8 range
            9, 15, 16,         // 9-16 range
            17, 31, 32,        // 17-32 range (1 round)
            33, 63, 64,        // 33-64 range (2 rounds)
            65, 95, 96,        // 65-96 range (3 rounds)
            97, 127, 128,      // 97-128 range (4 rounds)
        ];

        for len in boundary_lengths {
            let data: Vec<u8> = (0..len).map(|i| ((i * 37) % 256) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "boundary mismatch at len={}", len);
        }
    }

    #[test]
    fn const_hash_no_seed_helper() {
        // Verify the no-seed helper works
        let data = b"test data";
        let h1 = hash_const_no_seed::<128>(data);
        let h2 = hash_const::<128>(data, 0);
        let h3 = hash(data);
        assert_eq!(h1, h2);
        assert_eq!(h1, h3);
    }

    #[test]
    fn const_can_be_used_in_const_context() {
        // Verify we can actually use hash_const in const contexts
        const HASH_EMPTY: u64 = hash_const::<128>(b"", 0);
        const HASH_A: u64 = hash_const::<128>(b"a", 0);
        const HASH_BTC: u64 = hash_const::<128>(b"BTC-USD", 0);

        // These should match runtime
        assert_eq!(HASH_EMPTY, hash(b""));
        assert_eq!(HASH_A, hash(b"a"));
        assert_eq!(HASH_BTC, hash(b"BTC-USD"));

        // And they should all be different
        assert_ne!(HASH_EMPTY, HASH_A);
        assert_ne!(HASH_A, HASH_BTC);
    }

    // =========================================================================
    // Exhaustive and randomized tests for const vs runtime hash
    // =========================================================================

    /// Simple LCG for reproducible pseudo-random testing
    fn lcg_next(state: &mut u64) -> u64 {
        // LCG parameters from Numerical Recipes
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *state
    }

    #[test]
    fn const_matches_runtime_exhaustive_1_byte() {
        // Test ALL possible 1-byte inputs (256 values)
        for b in 0u8..=255 {
            let data = [b];
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "mismatch for single byte 0x{:02X}", b);
        }
    }

    #[test]
    fn const_matches_runtime_exhaustive_2_bytes() {
        // Test ALL possible 2-byte inputs (65536 values)
        for b0 in 0u8..=255 {
            for b1 in 0u8..=255 {
                let data = [b0, b1];
                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for bytes [0x{:02X}, 0x{:02X}]",
                    b0, b1
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_exhaustive_3_bytes_sampled() {
        // 3 bytes = 16M combinations, so we sample strategically
        // Test all combinations where bytes are 0x00, 0x7F, 0x80, or 0xFF
        let key_values: [u8; 4] = [0x00, 0x7F, 0x80, 0xFF];

        for &b0 in &key_values {
            for &b1 in &key_values {
                for &b2 in &key_values {
                    let data = [b0, b1, b2];
                    let h_const = hash_const::<128>(&data, 0);
                    let h_runtime = hash(&data);
                    assert_eq!(
                        h_const, h_runtime,
                        "mismatch for bytes [0x{:02X}, 0x{:02X}, 0x{:02X}]",
                        b0, b1, b2
                    );
                }
            }
        }

        // Also test with LCG-generated random 3-byte sequences
        let mut rng_state = 0xDEADBEEF_u64;
        for _ in 0..10000 {
            let r = lcg_next(&mut rng_state);
            let data = [(r & 0xFF) as u8, ((r >> 8) & 0xFF) as u8, ((r >> 16) & 0xFF) as u8];
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(
                h_const, h_runtime,
                "mismatch for random bytes {:?}",
                data
            );
        }
    }

    #[test]
    fn const_matches_runtime_random_4_to_8_bytes() {
        let mut rng_state = 0x12345678_u64;

        for len in 4..=8 {
            for _ in 0..10000 {
                let mut data = vec![0u8; len];
                for byte in &mut data {
                    *byte = (lcg_next(&mut rng_state) & 0xFF) as u8;
                }

                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for len={} data={:?}",
                    len, data
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_random_9_to_16_bytes() {
        let mut rng_state = 0xCAFEBABE_u64;

        for len in 9..=16 {
            for _ in 0..5000 {
                let mut data = vec![0u8; len];
                for byte in &mut data {
                    *byte = (lcg_next(&mut rng_state) & 0xFF) as u8;
                }

                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for len={} data={:02X?}",
                    len, data
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_random_17_to_32_bytes() {
        let mut rng_state = 0xFEEDFACE_u64;

        for len in 17..=32 {
            for _ in 0..2000 {
                let mut data = vec![0u8; len];
                for byte in &mut data {
                    *byte = (lcg_next(&mut rng_state) & 0xFF) as u8;
                }

                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for len={}",
                    len
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_random_33_to_64_bytes() {
        let mut rng_state = 0xBAADF00D_u64;

        for len in 33..=64 {
            for _ in 0..1000 {
                let mut data = vec![0u8; len];
                for byte in &mut data {
                    *byte = (lcg_next(&mut rng_state) & 0xFF) as u8;
                }

                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for len={}",
                    len
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_random_65_to_128_bytes() {
        let mut rng_state = 0xC0FFEE_u64;

        for len in 65..=128 {
            for _ in 0..500 {
                let mut data = vec![0u8; len];
                for byte in &mut data {
                    *byte = (lcg_next(&mut rng_state) & 0xFF) as u8;
                }

                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "mismatch for len={}",
                    len
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_random_with_seeds() {
        let mut rng_state = 0xABCDEF01_u64;
        let seeds: [u64; 8] = [0, 1, 255, 256, 65535, 65536, u64::MAX - 1, u64::MAX];

        for seed in seeds {
            for len in [1, 3, 4, 8, 9, 16, 17, 32, 64, 128] {
                for _ in 0..100 {
                    let mut data = vec![0u8; len];
                    for byte in &mut data {
                        *byte = (lcg_next(&mut rng_state) & 0xFF) as u8;
                    }

                    let h_const = hash_const::<128>(&data, seed);
                    let h_runtime = hash_with_seed(&data, seed);
                    assert_eq!(
                        h_const, h_runtime,
                        "mismatch for len={} seed={}",
                        len, seed
                    );
                }
            }
        }
    }

    #[test]
    fn const_matches_runtime_content_diversity() {
        // All printable ASCII
        let printable: Vec<u8> = (0x20u8..=0x7E).collect();
        let h_const = hash_const::<128>(&printable, 0);
        let h_runtime = hash(&printable);
        assert_eq!(h_const, h_runtime, "printable ASCII mismatch");

        // All control characters (0x00-0x1F)
        let control: Vec<u8> = (0x00u8..=0x1F).collect();
        let h_const = hash_const::<128>(&control, 0);
        let h_runtime = hash(&control);
        assert_eq!(h_const, h_runtime, "control chars mismatch");

        // Mix of control and printable
        let mixed: Vec<u8> = (0x00u8..=0x40).collect();
        let h_const = hash_const::<128>(&mixed, 0);
        let h_runtime = hash(&mixed);
        assert_eq!(h_const, h_runtime, "mixed chars mismatch");

        // High bytes (near 127)
        let high: Vec<u8> = (0x70u8..=0x7F).collect();
        let h_const = hash_const::<128>(&high, 0);
        let h_runtime = hash(&high);
        assert_eq!(h_const, h_runtime, "high bytes mismatch");

        // Bytes just above ASCII (should still hash, even if not valid ASCII)
        let above_ascii: Vec<u8> = (0x80u8..=0xFF).collect();
        let h_const = hash_const::<128>(&above_ascii, 0);
        let h_runtime = hash(&above_ascii);
        assert_eq!(h_const, h_runtime, "above-ASCII bytes mismatch");

        // Repeated patterns
        for pattern in [0x00u8, 0x55, 0xAA, 0xFF] {
            for len in [1, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128] {
                let data = vec![pattern; len];
                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "repeated 0x{:02X} at len={} mismatch",
                    pattern, len
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_incrementing_patterns() {
        // Incrementing bytes (mod 256)
        for len in 1..=128 {
            let data: Vec<u8> = (0..len).map(|i| (i % 256) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "incrementing at len={}", len);
        }

        // Decrementing bytes
        for len in 1..=128 {
            let data: Vec<u8> = (0..len).map(|i| (255 - (i % 256)) as u8).collect();
            let h_const = hash_const::<128>(&data, 0);
            let h_runtime = hash(&data);
            assert_eq!(h_const, h_runtime, "decrementing at len={}", len);
        }
    }

    #[test]
    fn const_matches_runtime_bit_patterns() {
        // Single bit set in each position
        for bit_pos in 0..8 {
            let byte = 1u8 << bit_pos;
            for len in 1..=16 {
                let data = vec![byte; len];
                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "bit {} set at len={} mismatch",
                    bit_pos, len
                );
            }
        }

        // Single bit clear in each position
        for bit_pos in 0..8 {
            let byte = !(1u8 << bit_pos);
            for len in 1..=16 {
                let data = vec![byte; len];
                let h_const = hash_const::<128>(&data, 0);
                let h_runtime = hash(&data);
                assert_eq!(
                    h_const, h_runtime,
                    "bit {} clear at len={} mismatch",
                    bit_pos, len
                );
            }
        }
    }

    #[test]
    fn const_matches_runtime_one_byte_difference() {
        // Test that changing one byte produces different (but still matching) hashes
        let base: Vec<u8> = (0..64).collect();
        let h_base_const = hash_const::<128>(&base, 0);
        let h_base_runtime = hash(&base);
        assert_eq!(h_base_const, h_base_runtime);

        for pos in 0..64 {
            let mut modified = base.clone();
            modified[pos] ^= 0xFF; // Flip all bits at position

            let h_const = hash_const::<128>(&modified, 0);
            let h_runtime = hash(&modified);
            assert_eq!(
                h_const, h_runtime,
                "modified at pos={} mismatch",
                pos
            );
            // Also verify the hash actually changed
            assert_ne!(h_const, h_base_const, "hash didn't change at pos={}", pos);
        }
    }
}
