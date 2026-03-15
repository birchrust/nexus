//! Cross-thread throughput benchmark for SPSC and MPSC.
//!
//! Measures bytes/second with random-sized messages up to 1KB.
//!
//! Run with:
//!   cargo run --release --example throughput_bench

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_utils::Backoff;
use nexus_logbuf::queue::{mpsc, spsc};
use rand::Rng;

const BUFFER_SIZE: usize = 2 * 1024 * 1024; // 2MB
const MAX_MESSAGE_SIZE: usize = 1024; // 1KB
const MIN_MESSAGE_SIZE: usize = 8;
const DURATION_SECS: u64 = 5;

#[derive(Clone, Copy)]
enum ContentionMode {
    /// Tight spin on contention (maximum throughput attempt)
    TightSpin,
    /// Backoff on contention (reduces CPU waste, may improve throughput)
    Backoff,
}

fn bench_spsc() -> (u64, u64) {
    let (mut prod, mut cons) = spsc::new(BUFFER_SIZE);

    let running = Arc::new(AtomicBool::new(true));
    let bytes_sent = Arc::new(AtomicU64::new(0));
    let messages_sent = Arc::new(AtomicU64::new(0));

    let prod_running = Arc::clone(&running);
    let prod_bytes = Arc::clone(&bytes_sent);
    let prod_msgs = Arc::clone(&messages_sent);

    // Producer thread
    let producer = thread::spawn(move || {
        let mut rng = rand::thread_rng();
        let payload = vec![0xABu8; MAX_MESSAGE_SIZE];
        let mut local_bytes = 0u64;
        let mut local_msgs = 0u64;

        while prod_running.load(Ordering::Relaxed) {
            let len = rng.gen_range(MIN_MESSAGE_SIZE..=MAX_MESSAGE_SIZE);

            loop {
                match prod.try_claim(len) {
                    Ok(mut claim) => {
                        claim.copy_from_slice(&payload[..len]);
                        claim.commit();
                        local_bytes += len as u64;
                        local_msgs += 1;
                        break;
                    }
                    Err(_) => std::hint::spin_loop(),
                }
            }
        }

        prod_bytes.fetch_add(local_bytes, Ordering::Relaxed);
        prod_msgs.fetch_add(local_msgs, Ordering::Relaxed);
    });

    // Consumer thread
    let cons_running = Arc::clone(&running);
    let consumer = thread::spawn(move || {
        while cons_running.load(Ordering::Relaxed) {
            while cons.try_claim().is_some() {}
            std::hint::spin_loop();
        }
        // Drain remaining
        while cons.try_claim().is_some() {}
    });

    // Let it run
    thread::sleep(Duration::from_secs(DURATION_SECS));
    running.store(false, Ordering::Relaxed);

    producer.join().unwrap();
    consumer.join().unwrap();

    (
        bytes_sent.load(Ordering::Relaxed),
        messages_sent.load(Ordering::Relaxed),
    )
}

fn bench_mpsc(num_producers: usize, mode: ContentionMode) -> (u64, u64) {
    let (prod, mut cons) = mpsc::new(BUFFER_SIZE);

    let running = Arc::new(AtomicBool::new(true));
    let bytes_sent = Arc::new(AtomicU64::new(0));
    let messages_sent = Arc::new(AtomicU64::new(0));

    // Producer threads
    let producers: Vec<_> = (0..num_producers)
        .map(|_| {
            let mut prod = prod.clone();
            let prod_running = Arc::clone(&running);
            let prod_bytes = Arc::clone(&bytes_sent);
            let prod_msgs = Arc::clone(&messages_sent);

            thread::spawn(move || {
                let mut rng = rand::thread_rng();
                let payload = vec![0xABu8; MAX_MESSAGE_SIZE];
                let mut local_bytes = 0u64;
                let mut local_msgs = 0u64;

                while prod_running.load(Ordering::Relaxed) {
                    let len = rng.gen_range(MIN_MESSAGE_SIZE..=MAX_MESSAGE_SIZE);

                    match mode {
                        ContentionMode::TightSpin => loop {
                            match prod.try_claim(len) {
                                Ok(mut claim) => {
                                    claim.copy_from_slice(&payload[..len]);
                                    claim.commit();
                                    local_bytes += len as u64;
                                    local_msgs += 1;
                                    break;
                                }
                                Err(_) => std::hint::spin_loop(),
                            }
                        },
                        ContentionMode::Backoff => {
                            let backoff = Backoff::new();
                            loop {
                                if let Ok(mut claim) = prod.try_claim(len) {
                                    claim.copy_from_slice(&payload[..len]);
                                    claim.commit();
                                    local_bytes += len as u64;
                                    local_msgs += 1;
                                    break;
                                }
                                backoff.snooze();
                                if backoff.is_completed() {
                                    backoff.reset();
                                }
                            }
                        }
                    }
                }

                prod_bytes.fetch_add(local_bytes, Ordering::Relaxed);
                prod_msgs.fetch_add(local_msgs, Ordering::Relaxed);
            })
        })
        .collect();

    drop(prod); // Drop original producer

    // Consumer thread
    let cons_running = Arc::clone(&running);
    let consumer = thread::spawn(move || {
        while cons_running.load(Ordering::Relaxed) {
            while cons.try_claim().is_some() {}
            std::hint::spin_loop();
        }
        // Drain remaining
        while cons.try_claim().is_some() {}
    });

    // Let it run
    thread::sleep(Duration::from_secs(DURATION_SECS));
    running.store(false, Ordering::Relaxed);

    for p in producers {
        p.join().unwrap();
    }
    consumer.join().unwrap();

    (
        bytes_sent.load(Ordering::Relaxed),
        messages_sent.load(Ordering::Relaxed),
    )
}

fn format_throughput(bytes: u64, duration_secs: u64) -> String {
    let bytes_per_sec = bytes / duration_secs;
    if bytes_per_sec >= 1_000_000_000 {
        format!("{:.2} GB/s", bytes_per_sec as f64 / 1_000_000_000.0)
    } else if bytes_per_sec >= 1_000_000 {
        format!("{:.2} MB/s", bytes_per_sec as f64 / 1_000_000.0)
    } else {
        format!("{:.2} KB/s", bytes_per_sec as f64 / 1_000.0)
    }
}

fn main() {
    println!("nexus-logbuf throughput benchmark");
    println!("==================================");
    println!("Buffer size: {} MB", BUFFER_SIZE / (1024 * 1024));
    println!(
        "Message size: {}-{} bytes (random)",
        MIN_MESSAGE_SIZE, MAX_MESSAGE_SIZE
    );
    println!("Duration: {} seconds per test", DURATION_SECS);
    println!();

    // SPSC
    println!("Running SPSC...");
    let (bytes, msgs) = bench_spsc();
    println!(
        "  Throughput: {} ({:.2}M msgs/sec)",
        format_throughput(bytes, DURATION_SECS),
        msgs as f64 / DURATION_SECS as f64 / 1_000_000.0
    );
    println!("  Total: {} bytes, {} messages", bytes, msgs);
    println!();

    // MPSC with varying producer counts and contention modes
    for num_producers in [2, 4] {
        println!("Running MPSC ({} producers, tight spin)...", num_producers);
        let (bytes, msgs) = bench_mpsc(num_producers, ContentionMode::TightSpin);
        println!(
            "  Throughput: {} ({:.2}M msgs/sec)",
            format_throughput(bytes, DURATION_SECS),
            msgs as f64 / DURATION_SECS as f64 / 1_000_000.0
        );
        println!();

        println!("Running MPSC ({} producers, backoff)...", num_producers);
        let (bytes, msgs) = bench_mpsc(num_producers, ContentionMode::Backoff);
        println!(
            "  Throughput: {} ({:.2}M msgs/sec)",
            format_throughput(bytes, DURATION_SECS),
            msgs as f64 / DURATION_SECS as f64 / 1_000_000.0
        );
        println!();
    }
}
