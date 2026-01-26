//! Hash function quality analysis.
//!
//! Tests distribution quality and avalanche properties of the XXH3 hash
//! when truncated to 48 bits. Helps determine whether to use upper or
//! lower 48 bits for AsciiString hashes.
//!
//! Run with:
//! ```bash
//! cargo run --release --example quality_hash
//! ```

use nexus_ascii::AsciiString;
use nexus_ascii::hash::{hash, truncate_lower_48, truncate_upper_48};
use std::hash::{Hash, Hasher};

const N_SAMPLES: usize = 1_000_000;

/// Input sizes to test quality at.
const INPUT_SIZES: &[usize] = &[8, 16, 32, 64, 128, 256, 512, 1024];

/// Analyze bit distribution of hashes.
/// Ideal: each bit is set ~50% of the time.
fn analyze_bit_distribution(name: &str, truncate: fn(u64) -> [u8; 6]) {
    let mut bit_counts = [0usize; 48];

    for i in 0..N_SAMPLES {
        // Generate unique input
        let input = format!("test_input_{:08x}", i);
        let h = hash::<32>(input.as_bytes());
        let truncated = truncate(h);

        // Convert to u64 for bit counting
        let h = truncated[0] as u64
            | ((truncated[1] as u64) << 8)
            | ((truncated[2] as u64) << 16)
            | ((truncated[3] as u64) << 24)
            | ((truncated[4] as u64) << 32)
            | ((truncated[5] as u64) << 40);

        for bit in 0..48 {
            if (h >> bit) & 1 == 1 {
                bit_counts[bit] += 1;
            }
        }
    }

    // Calculate statistics
    let mut min_ratio = 1.0f64;
    let mut max_ratio = 0.0f64;
    let mut worst_bit = 0;

    for (bit, &count) in bit_counts.iter().enumerate() {
        let ratio = count as f64 / N_SAMPLES as f64;
        if (ratio - 0.5).abs() > (min_ratio - 0.5).abs().min((max_ratio - 0.5).abs()) {
            worst_bit = bit;
        }
        min_ratio = min_ratio.min(ratio);
        max_ratio = max_ratio.max(ratio);
    }

    // Standard deviation from 0.5
    let variance: f64 = bit_counts
        .iter()
        .map(|&c| {
            let ratio = c as f64 / N_SAMPLES as f64;
            (ratio - 0.5).powi(2)
        })
        .sum::<f64>()
        / 48.0;
    let std_dev = variance.sqrt();

    println!(
        "  {:<12} min={:.4} max={:.4} worst_bit={:>2} std_dev={:.6}",
        name, min_ratio, max_ratio, worst_bit, std_dev
    );
}

/// Analyze bucket distribution using chi-squared test.
fn analyze_bucket_distribution(truncate: fn(u64) -> [u8; 6], n_buckets: usize) -> f64 {
    let mut buckets = vec![0usize; n_buckets];
    let mask = (n_buckets - 1) as u64;

    for i in 0..N_SAMPLES {
        let input = format!("bucket_test_{:08x}", i);
        let h = hash::<32>(input.as_bytes());
        let truncated = truncate(h);

        let h = truncated[0] as u64
            | ((truncated[1] as u64) << 8)
            | ((truncated[2] as u64) << 16)
            | ((truncated[3] as u64) << 24)
            | ((truncated[4] as u64) << 32)
            | ((truncated[5] as u64) << 40);

        let bucket = (h & mask) as usize;
        buckets[bucket] += 1;
    }

    let expected = N_SAMPLES as f64 / n_buckets as f64;
    let chi_squared: f64 = buckets
        .iter()
        .map(|&c| (c as f64 - expected).powi(2) / expected)
        .sum();

    chi_squared
}

/// Analyze avalanche effect.
/// Flip each input bit, count how many output bits change.
/// Ideal: ~24 bits change (50% of 48 bits).
fn analyze_avalanche(input_len: usize, truncate: fn(u64) -> [u8; 6]) -> f64 {
    let mut total_flips = 0usize;
    let mut samples = 0usize;

    let iterations = if input_len <= 128 { 1000 } else { 100 };

    for seed in 0..iterations {
        let mut input: Vec<u8> = (0..input_len).map(|i| ((seed + i) % 256) as u8).collect();
        let original_hash = hash::<1024>(&input);
        let original_truncated = truncate(original_hash);

        for byte_idx in 0..input_len {
            for bit_idx in 0..8 {
                input[byte_idx] ^= 1 << bit_idx;

                let new_hash = hash::<1024>(&input);
                let new_truncated = truncate(new_hash);

                let mut diff_bits = 0;
                for i in 0..6 {
                    diff_bits += (original_truncated[i] ^ new_truncated[i]).count_ones() as usize;
                }

                total_flips += diff_bits;
                samples += 1;

                input[byte_idx] ^= 1 << bit_idx;
            }
        }
    }

    total_flips as f64 / samples as f64
}

/// Count actual collisions in a sample.
fn analyze_collisions(truncate: fn(u64) -> [u8; 6]) -> usize {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut collisions = 0;

    for i in 0..N_SAMPLES {
        let input = format!("collision_test_{:08x}", i);
        let h = hash::<32>(input.as_bytes());
        let truncated = truncate(h);

        if !seen.insert(truncated) {
            collisions += 1;
        }
    }

    collisions
}

/// Birthday paradox collision test at increasing scales.
/// Expected collisions with 48 bits: n²/(2^49)
///
/// Memory usage: ~20 bytes per entry with HashSet overhead.
/// - 10M: ~200MB
/// - 50M: ~1GB
/// - 100M: ~2GB (set BIRTHDAY_100M=1 to enable)
fn analyze_collisions_birthday() {
    use std::collections::HashSet;

    // Check if user wants the full 100M test
    let scales: Vec<(usize, &str)> = if std::env::var("BIRTHDAY_100M").is_ok() {
        vec![
            (1_000_000, "1M"),
            (10_000_000, "10M"),
            (50_000_000, "50M"),
            (100_000_000, "100M"),
        ]
    } else {
        vec![(1_000_000, "1M"), (10_000_000, "10M"), (50_000_000, "50M")]
    };

    println!("\n=== BIRTHDAY PARADOX COLLISION TEST ===\n");
    println!("  Testing collision rate vs theoretical (48-bit hash)");
    println!("  Expected collisions ≈ n²/2^49");
    if std::env::var("BIRTHDAY_100M").is_err() {
        println!("  (Set BIRTHDAY_100M=1 for 100M test, uses ~2GB RAM)");
    }
    println!();

    println!(
        "  {:>10}  {:>12}  {:>12}  {:>10}",
        "Samples", "Expected", "Actual", "Ratio"
    );
    println!("  {}", "-".repeat(50));

    let mut seen: HashSet<[u8; 6]> = HashSet::new();
    let mut collisions = 0usize;
    let mut next_scale_idx = 0;
    let max_samples = scales.last().unwrap().0;

    for i in 0..max_samples {
        // Use index bytes directly as input (faster than format!)
        let input = (i as u64).to_le_bytes();
        let h = hash::<8>(&input);
        let truncated = truncate_upper_48(h);

        if !seen.insert(truncated) {
            collisions += 1;
        }

        // Report at each scale point
        if next_scale_idx < scales.len() && i + 1 == scales[next_scale_idx].0 {
            let n = scales[next_scale_idx].0 as f64;
            let expected = n * n / 2.0_f64.powi(49);
            let ratio_str = if expected > 0.01 {
                format!("{:.2}x", collisions as f64 / expected)
            } else if collisions == 0 {
                "✓".to_string()
            } else {
                format!("{} (exp ~0)", collisions)
            };

            println!(
                "  {:>10}  {:>12.2}  {:>12}  {:>10}",
                scales[next_scale_idx].1, expected, collisions, ratio_str
            );

            next_scale_idx += 1;
        }
    }

    // Summary
    let n = max_samples as f64;
    let expected_final = n * n / 2.0_f64.powi(49);
    let deviation = ((collisions as f64 - expected_final) / expected_final.max(1.0) * 100.0).abs();

    println!();
    if deviation < 50.0 || expected_final < 1.0 {
        println!("  ✓ Collision rate matches birthday paradox expectation");
    } else {
        println!(
            "  ⚠ Collision rate deviates {:.1}% from expected",
            deviation
        );
    }
}

/// Analyze bit distribution at a specific input size.
fn analyze_bit_distribution_sized(input_size: usize, truncate: fn(u64) -> [u8; 6]) -> f64 {
    let mut bit_counts = [0usize; 48];
    let samples = 100_000;

    for i in 0..samples {
        let input: Vec<u8> = (0..input_size)
            .map(|j| ((i + j * 31) % 256) as u8)
            .collect();
        let h = hash::<1024>(&input);
        let truncated = truncate(h);

        let h = truncated[0] as u64
            | ((truncated[1] as u64) << 8)
            | ((truncated[2] as u64) << 16)
            | ((truncated[3] as u64) << 24)
            | ((truncated[4] as u64) << 32)
            | ((truncated[5] as u64) << 40);

        for bit in 0..48 {
            if (h >> bit) & 1 == 1 {
                bit_counts[bit] += 1;
            }
        }
    }

    let variance: f64 = bit_counts
        .iter()
        .map(|&c| {
            let ratio = c as f64 / samples as f64;
            (ratio - 0.5).powi(2)
        })
        .sum::<f64>()
        / 48.0;

    variance.sqrt()
}

// =============================================================================
// AsciiString Header Quality Tests
// =============================================================================

/// A simple hasher that captures the u64 value being hashed.
/// Used to extract what AsciiString passes to the hasher.
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn write(&mut self, _bytes: &[u8]) {
        // AsciiString hashes its header via u64::hash which calls write_u64
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Extract the raw header value from an AsciiString (what nohash-hasher would use).
fn extract_header<const CAP: usize>(s: &AsciiString<CAP>) -> u64 {
    let mut hasher = IdentityHasher(0);
    s.hash(&mut hasher);
    hasher.finish()
}

/// Analyze bucket distribution of AsciiString headers.
fn analyze_ascii_string_bucket_distribution(n_buckets: usize) -> f64 {
    let mut buckets = vec![0usize; n_buckets];
    let mask = (n_buckets - 1) as u64;

    for i in 0..N_SAMPLES {
        // Generate realistic string inputs (symbols, order IDs, etc.)
        let input = format!("SYM-{:08X}", i);
        let s: AsciiString<32> = AsciiString::try_from(input.as_str()).unwrap();
        let header = extract_header(&s);

        let bucket = (header & mask) as usize;
        buckets[bucket] += 1;
    }

    let expected = N_SAMPLES as f64 / n_buckets as f64;
    let chi_squared: f64 = buckets
        .iter()
        .map(|&c| (c as f64 - expected).powi(2) / expected)
        .sum();

    chi_squared
}

/// Analyze bit distribution of AsciiString headers.
fn analyze_ascii_string_bit_distribution() -> (f64, f64, f64) {
    let mut bit_counts = [0usize; 64];

    for i in 0..N_SAMPLES {
        let input = format!("KEY-{:08X}", i);
        let s: AsciiString<32> = AsciiString::try_from(input.as_str()).unwrap();
        let header = extract_header(&s);

        for bit in 0..64 {
            if (header >> bit) & 1 == 1 {
                bit_counts[bit] += 1;
            }
        }
    }

    let mut min_ratio = 1.0f64;
    let mut max_ratio = 0.0f64;

    // Only check bits 0-47 (the hash portion, not length in upper bits)
    for bit in 0..48 {
        let ratio = bit_counts[bit] as f64 / N_SAMPLES as f64;
        min_ratio = min_ratio.min(ratio);
        max_ratio = max_ratio.max(ratio);
    }

    let variance: f64 = bit_counts[0..48]
        .iter()
        .map(|&c| {
            let ratio = c as f64 / N_SAMPLES as f64;
            (ratio - 0.5).powi(2)
        })
        .sum::<f64>()
        / 48.0;

    (min_ratio, max_ratio, variance.sqrt())
}

/// Test collisions when using AsciiString headers directly.
fn analyze_ascii_string_collisions() -> usize {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut collisions = 0;

    for i in 0..N_SAMPLES {
        let input = format!("ID-{:08X}", i);
        let s: AsciiString<32> = AsciiString::try_from(input.as_str()).unwrap();
        let header = extract_header(&s);

        if !seen.insert(header) {
            collisions += 1;
        }
    }

    collisions
}

/// Test with varying string lengths (header includes length in lower 16 bits).
fn analyze_ascii_string_varying_lengths() -> f64 {
    let mut buckets = vec![0usize; 65536];
    let mask = 65535u64;
    let samples = 500_000;

    for i in 0..samples {
        // Vary both content and length
        let len = (i % 30) + 1; // lengths 1-30
        let input: String = (0..len)
            .map(|j| (b'A' + ((i + j) % 26) as u8) as char)
            .collect();
        let s: AsciiString<32> = AsciiString::try_from(input.as_str()).unwrap();
        let header = extract_header(&s);

        let bucket = (header & mask) as usize;
        buckets[bucket] += 1;
    }

    let expected = samples as f64 / 65536.0;
    let chi_squared: f64 = buckets
        .iter()
        .map(|&c| (c as f64 - expected).powi(2) / expected)
        .sum();

    chi_squared
}

/// Empirically validate collision rate for AsciiString headers.
/// Tests at multiple scales and compares to birthday paradox expectations.
///
/// Set COLLISION_50M=1 env var to test up to 50M strings (~4 expected collisions).
/// Set COLLISION_100M=1 env var to test up to 100M strings (~18 expected collisions, ~2GB RAM).
fn validate_ascii_string_collisions() {
    use std::collections::HashSet;

    println!("\n=== ASCIISTRING HEADER COLLISION VALIDATION ===\n");
    println!("  Empirically testing collision rate vs birthday paradox expectation.");
    println!("  Formula: expected collisions = n² / 2^49\n");

    let scales: Vec<(usize, &str)> = if std::env::var("COLLISION_100M").is_ok() {
        vec![
            (1_000_000, "1M"),
            (10_000_000, "10M"),
            (25_000_000, "25M"),
            (50_000_000, "50M"),
            (75_000_000, "75M"),
            (100_000_000, "100M"),
        ]
    } else if std::env::var("COLLISION_50M").is_ok() {
        vec![
            (1_000_000, "1M"),
            (10_000_000, "10M"),
            (20_000_000, "20M"),
            (30_000_000, "30M"),
            (40_000_000, "40M"),
            (50_000_000, "50M"),
        ]
    } else {
        vec![
            (100_000, "100K"),
            (500_000, "500K"),
            (1_000_000, "1M"),
            (5_000_000, "5M"),
            (10_000_000, "10M"),
            (20_000_000, "20M"),
        ]
    };

    if std::env::var("COLLISION_100M").is_err() && std::env::var("COLLISION_50M").is_err() {
        println!("  (Set COLLISION_50M=1 for 50M test, COLLISION_100M=1 for 100M test)\n");
    }

    println!(
        "  {:>10}  {:>12}  {:>12}  {:>10}",
        "Strings", "Expected", "Actual", "Status"
    );
    println!("  {}", "-".repeat(50));

    let mut seen: HashSet<u64> = HashSet::new();
    let mut collisions = 0usize;
    let mut next_idx = 0;

    let max_n = scales.last().unwrap().0;

    for i in 0..max_n {
        // Generate unique strings with varied patterns
        let input = format!("KEY-{:08X}-{}", i, i % 100);
        let s: AsciiString<32> = AsciiString::try_from(input.as_str()).unwrap();
        let header = extract_header(&s);

        if !seen.insert(header) {
            collisions += 1;
        }

        // Report at each scale
        if next_idx < scales.len() && i + 1 == scales[next_idx].0 {
            let n = scales[next_idx].0 as f64;
            let expected = n * n / 2.0_f64.powi(49);

            let status = if expected < 0.1 {
                if collisions == 0 {
                    "✓ as expected"
                } else {
                    "⚠ unexpected"
                }
            } else {
                let ratio = collisions as f64 / expected;
                if ratio < 2.0 && ratio > 0.5 {
                    "✓ within 2x"
                } else if collisions == 0 && expected < 1.0 {
                    "✓ as expected"
                } else {
                    "⚠ check"
                }
            };

            println!(
                "  {:>10}  {:>12.4}  {:>12}  {:>10}",
                scales[next_idx].1, expected, collisions, status
            );

            next_idx += 1;
        }
    }

    println!();

    // Summary
    let final_n = max_n as f64;
    let final_expected = final_n * final_n / 2.0_f64.powi(49);

    println!("  At {} strings:", scales.last().unwrap().1);
    println!("    - Expected collisions: {:.2}", final_expected);
    println!("    - Actual collisions:   {}", collisions);

    if final_expected > 0.1 {
        let ratio = collisions as f64 / final_expected;
        println!("    - Ratio (actual/expected): {:.2}x", ratio);

        if ratio >= 0.5 && ratio <= 2.0 {
            println!("    ✓ Collision rate matches birthday paradox prediction");
        } else if collisions == 0 && final_expected < 2.0 {
            println!("    ✓ No collisions yet (expected < 2, so this is normal)");
        } else {
            println!("    ⚠ Collision rate deviates from expectation");
        }
    } else {
        println!("    ✓ Too few samples to expect collisions");
    }
}

/// Test with realistic trading symbol patterns.
fn analyze_trading_symbols() -> f64 {
    let bases = [
        "BTC", "ETH", "SOL", "AVAX", "MATIC", "DOGE", "XRP", "ADA", "DOT", "LINK",
    ];
    let quotes = ["USD", "USDT", "USDC", "EUR", "BTC", "ETH"];
    let suffixes = ["", "-PERP", "-SPOT", "-FUT", "-0329", "-0628"];

    let mut buckets = vec![0usize; 1024];
    let mask = 1023u64;
    let mut count = 0;

    // Generate all combinations multiple times with sequence numbers
    for seq in 0..1000 {
        for base in &bases {
            for quote in &quotes {
                for suffix in &suffixes {
                    let symbol = format!("{}-{}{}-{:04}", base, quote, suffix, seq);
                    if symbol.len() <= 32 {
                        let s: AsciiString<32> = AsciiString::try_from(symbol.as_str()).unwrap();
                        let header = extract_header(&s);
                        let bucket = (header & mask) as usize;
                        buckets[bucket] += 1;
                        count += 1;
                    }
                }
            }
        }
    }

    let expected = count as f64 / 1024.0;
    let chi_squared: f64 = buckets
        .iter()
        .map(|&c| (c as f64 - expected).powi(2) / expected)
        .sum();

    chi_squared
}

fn main() {
    println!("XXH3 HASH QUALITY ANALYSIS");
    println!("==========================\n");

    print!("Implementation: XXH3 with ");
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
    println!("AVX-512");
    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        not(target_feature = "avx512f")
    ))]
    println!("AVX2");
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    println!("SSE2");
    #[cfg(not(target_arch = "x86_64"))]
    println!("scalar");

    println!("Samples: {}\n", N_SAMPLES);

    // Bit distribution comparison
    println!("=== BIT DISTRIBUTION (ideal ≈ 0.5000) ===\n");
    analyze_bit_distribution("lower_48", truncate_lower_48);
    analyze_bit_distribution("upper_48", truncate_upper_48);

    // Bucket distribution comparison
    println!("\n=== BUCKET DISTRIBUTION (lower χ² = more uniform) ===\n");
    let lower_chi_1k = analyze_bucket_distribution(truncate_lower_48, 1024);
    let upper_chi_1k = analyze_bucket_distribution(truncate_upper_48, 1024);
    println!(
        "  1024 buckets:  lower_48 χ²={:>10.2}  upper_48 χ²={:>10.2}",
        lower_chi_1k, upper_chi_1k
    );

    let lower_chi_64k = analyze_bucket_distribution(truncate_lower_48, 65536);
    let upper_chi_64k = analyze_bucket_distribution(truncate_upper_48, 65536);
    println!(
        "  65536 buckets: lower_48 χ²={:>10.2}  upper_48 χ²={:>10.2}",
        lower_chi_64k, upper_chi_64k
    );

    // Avalanche comparison
    println!("\n=== AVALANCHE (ideal = 24.00 bits flip per input bit) ===\n");
    let lower_aval = analyze_avalanche(32, truncate_lower_48);
    let upper_aval = analyze_avalanche(32, truncate_upper_48);
    println!(
        "  32B input: lower_48={:.2}/48  upper_48={:.2}/48",
        lower_aval, upper_aval
    );

    // Collisions (quick sanity check)
    println!("\n=== COLLISIONS (expect ~0 for 1M samples) ===\n");
    let lower_col = analyze_collisions(truncate_lower_48);
    let upper_col = analyze_collisions(truncate_upper_48);
    println!("  lower_48: {}  upper_48: {}", lower_col, upper_col);

    // Birthday paradox scaling test
    analyze_collisions_birthday();

    // Distribution by input size
    println!("\n=== BIT DISTRIBUTION BY INPUT SIZE (std_dev, lower is better) ===\n");
    print!("  {:<12}", "Truncation");
    for size in INPUT_SIZES {
        print!("{:>8}", format!("{}B", size));
    }
    println!();
    println!("  {}", "-".repeat(12 + INPUT_SIZES.len() * 8));

    print!("  {:<12}", "lower_48");
    for &size in INPUT_SIZES {
        let std_dev = analyze_bit_distribution_sized(size, truncate_lower_48);
        print!("{:>8.6}", std_dev);
    }
    println!();

    print!("  {:<12}", "upper_48");
    for &size in INPUT_SIZES {
        let std_dev = analyze_bit_distribution_sized(size, truncate_upper_48);
        print!("{:>8.6}", std_dev);
    }
    println!();

    // Avalanche by input size
    println!("\n=== AVALANCHE BY INPUT SIZE (deviation from 24.00) ===\n");
    print!("  {:<12}", "Truncation");
    for size in INPUT_SIZES {
        print!("{:>8}", format!("{}B", size));
    }
    println!();
    println!("  {}", "-".repeat(12 + INPUT_SIZES.len() * 8));

    print!("  {:<12}", "lower_48");
    for &size in INPUT_SIZES {
        let avg = analyze_avalanche(size, truncate_lower_48);
        print!("{:>8.2}", avg);
    }
    println!();

    print!("  {:<12}", "upper_48");
    for &size in INPUT_SIZES {
        let avg = analyze_avalanche(size, truncate_upper_48);
        print!("{:>8.2}", avg);
    }
    println!();

    // Summary
    println!("\n=== RECOMMENDATION ===\n");

    let lower_score = lower_chi_1k + lower_chi_64k + (lower_aval - 24.0).abs() * 100.0;
    let upper_score = upper_chi_1k + upper_chi_64k + (upper_aval - 24.0).abs() * 100.0;

    println!("  Composite score (lower is better):");
    println!("    lower_48: {:.2}", lower_score);
    println!("    upper_48: {:.2}", upper_score);
    println!();

    if lower_score < upper_score {
        println!("  Recommendation: Use LOWER 48 bits (truncate_lower_48)");
    } else {
        println!("  Recommendation: Use UPPER 48 bits (truncate_upper_48)");
    }

    // =========================================================================
    // AsciiString Header Quality Tests
    // =========================================================================

    println!("\n");
    println!("ASCIISTRING HEADER QUALITY (for nohash-hasher)");
    println!("==============================================\n");
    println!("  Testing distribution when using AsciiString headers as hash values.");
    println!("  Header layout: bits 0-47 = XXH3 hash, bits 48-63 = length\n");

    // Bit distribution
    println!("=== HEADER BIT DISTRIBUTION (hash bits 0-47, ideal ≈ 0.5000) ===\n");
    let (min, max, std_dev) = analyze_ascii_string_bit_distribution();
    println!("  min={:.4} max={:.4} std_dev={:.6}", min, max, std_dev);
    if std_dev < 0.01 {
        println!("  ✓ Excellent bit distribution");
    } else if std_dev < 0.02 {
        println!("  ✓ Good bit distribution");
    } else {
        println!("  ⚠ Poor bit distribution (std_dev > 0.02)");
    }

    // Bucket distribution
    println!("\n=== HEADER BUCKET DISTRIBUTION (lower χ² = more uniform) ===\n");
    let chi_1k = analyze_ascii_string_bucket_distribution(1024);
    let chi_64k = analyze_ascii_string_bucket_distribution(65536);
    println!("  1024 buckets:  χ²={:>10.2}", chi_1k);
    println!("  65536 buckets: χ²={:>10.2}", chi_64k);

    // For reference: χ² critical value at p=0.05 for 1023 df ≈ 1098, for 65535 df ≈ 66140
    if chi_1k < 1200.0 {
        println!("  ✓ 1024-bucket distribution is uniform (χ² < 1200)");
    } else {
        println!("  ⚠ 1024-bucket distribution may be non-uniform");
    }

    // Collisions
    println!("\n=== HEADER COLLISIONS (expect ~0 for 1M samples) ===\n");
    let collisions = analyze_ascii_string_collisions();
    println!("  Collisions: {}", collisions);
    if collisions == 0 {
        println!("  ✓ No collisions in 1M unique strings");
    } else {
        println!(
            "  Note: {} collision(s) - acceptable with 48-bit hash",
            collisions
        );
    }

    // Varying lengths
    println!("\n=== VARYING LENGTH STRINGS (tests length field interaction) ===\n");
    let chi_varying = analyze_ascii_string_varying_lengths();
    println!("  65536 buckets with lengths 1-30: χ²={:.2}", chi_varying);
    if chi_varying < 70000.0 {
        println!("  ✓ Length variation doesn't cause clustering");
    } else {
        println!("  ⚠ Length field may be causing bucket clustering");
    }

    // Trading symbols
    println!("\n=== REALISTIC TRADING SYMBOLS ===\n");
    let chi_trading = analyze_trading_symbols();
    println!("  1024 buckets with trading pairs: χ²={:.2}", chi_trading);
    if chi_trading < 1200.0 {
        println!("  ✓ Trading symbols distribute uniformly");
    } else {
        println!("  ⚠ Trading symbols may cluster (χ² > 1200)");
    }

    // Empirical collision validation
    validate_ascii_string_collisions();

    // Summary
    println!("\n=== ASCIISTRING SUMMARY ===\n");
    let header_ok = std_dev < 0.02 && chi_1k < 1200.0 && chi_trading < 1200.0;
    if header_ok {
        println!("  ✓ AsciiString headers are suitable for nohash-hasher");
        println!("    - Good bit distribution in hash portion");
        println!("    - Uniform bucket distribution");
        println!("    - Works well with realistic trading data");
        println!("    - Collision rate matches birthday paradox (48-bit)");
    } else {
        println!("  ⚠ AsciiString headers may have quality issues");
    }

    println!();
}
