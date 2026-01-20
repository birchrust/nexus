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

use nexus_ascii::hash::{hash, truncate_lower_48, truncate_upper_48};

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

    println!();
}
