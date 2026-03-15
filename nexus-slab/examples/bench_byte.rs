//! Byte slab benchmark: BoxSlot (TLS) and SlotBox (raw slab) for Sized and dyn.
//!
//! Measures cycle-level latency for byte slab operations across four handle types:
//! - BoxSlot<T, A> — TLS byte allocator, Sized
//! - BoxSlot<dyn Trait, A> — TLS byte allocator, dyn
//! - SlotBox<T> — raw slab, Sized
//! - SlotBox<dyn Trait> — raw slab, dyn
//!
//! Usage:
//!   cargo build --release --example bench_byte
//!   taskset -c 0 ./target/release/examples/bench_byte <tls|raw>

use std::hint::black_box;

// ============================================================================
// Pod types
// ============================================================================

#[derive(Clone, Debug, Default)]
#[repr(C)]
pub struct Pod32 {
    pub data: [u8; 32],
}

trait PodTrait {
    fn first(&self) -> u8;
}

impl PodTrait for Pod32 {
    #[inline]
    fn first(&self) -> u8 {
        self.data[0]
    }
}

// ============================================================================
// TLS byte allocators (64B slots)
// ============================================================================

mod bounded_byte {
    nexus_slab::bounded_byte_allocator!(64);
}

mod unbounded_byte {
    nexus_slab::unbounded_byte_allocator!(64);
}

// ============================================================================
// Timing
// ============================================================================

#[inline(always)]
fn rdtsc_start() -> u64 {
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    unsafe {
        let tsc = std::arch::x86_64::__rdtscp(&mut 0u32 as *mut _);
        std::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_row(label: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<22} {:>5} {:>5} {:>5} {:>6} {:>7} {:>7}",
        label,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        percentile(samples, 99.99),
        samples[samples.len() - 1],
    );
}

fn print_header() {
    println!(
        "  {:<22} {:>5} {:>5} {:>5} {:>6} {:>7} {:>7}",
        "", "p50", "p90", "p99", "p99.9", "p99.99", "max"
    );
}

const SAMPLES: usize = 50_000;
const WARMUP: usize = 5_000;

// ============================================================================
// Unroll
// ============================================================================

macro_rules! unroll_10 {
    ($op:expr) => {
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
    };
}

macro_rules! unroll_100 {
    ($op:expr) => {
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
    };
}

// ============================================================================
// Cold cache support
// ============================================================================

const COLD_SAMPLES: usize = 10_000;
const POLLUTER_SIZE: usize = 8 * 1024 * 1024; // 8MB

#[inline(never)]
fn evict_cache(polluter: &[u8]) {
    let ptr = polluter.as_ptr();
    let len = polluter.len();
    for i in (0..len).step_by(64) {
        unsafe {
            std::ptr::read_volatile(ptr.add(i));
        }
    }
    unsafe {
        std::arch::x86_64::_mm_lfence();
    }
}

// ============================================================================
// TLS benchmarks — BoxSlot (Sized)
// ============================================================================

fn tls_bounded_churn_sized() {
    let val = Pod32::default();

    for _ in 0..WARMUP {
        let s = bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap();
        black_box(s.data[0]);
        drop(s);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        unroll_100!({
            let s = black_box(bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap());
            black_box(s.data[0]);
            drop(s);
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("bounded Sized", &mut samples);
}

fn tls_unbounded_churn_sized() {
    let val = Pod32::default();

    for _ in 0..WARMUP {
        let s = unbounded_byte::BoxSlot::<Pod32>::new(val.clone());
        black_box(s.data[0]);
        drop(s);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        unroll_100!({
            let s = black_box(unbounded_byte::BoxSlot::<Pod32>::new(val.clone()));
            black_box(s.data[0]);
            drop(s);
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("unbounded Sized", &mut samples);
}

// ============================================================================
// TLS benchmarks — BoxSlot (dyn)
// ============================================================================

fn tls_bounded_churn_dyn() {
    for _ in 0..WARMUP {
        let s: bounded_byte::BoxSlot<dyn PodTrait> =
            bounded_byte::BoxSlot::<Pod32>::try_new(Pod32::default())
                .unwrap()
                .unsize(|p| p as *mut dyn PodTrait);
        black_box(s.first());
        drop(s);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        unroll_100!({
            let s: bounded_byte::BoxSlot<dyn PodTrait> = black_box(
                bounded_byte::BoxSlot::<Pod32>::try_new(Pod32::default())
                    .unwrap()
                    .unsize(|p| p as *mut dyn PodTrait),
            );
            black_box(s.first());
            drop(s);
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("bounded dyn", &mut samples);
}

fn tls_unbounded_churn_dyn() {
    for _ in 0..WARMUP {
        let s: unbounded_byte::BoxSlot<dyn PodTrait> =
            unbounded_byte::BoxSlot::<Pod32>::new(Pod32::default())
                .unsize(|p| p as *mut dyn PodTrait);
        black_box(s.first());
        drop(s);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        unroll_100!({
            let s: unbounded_byte::BoxSlot<dyn PodTrait> = black_box(
                unbounded_byte::BoxSlot::<Pod32>::new(Pod32::default())
                    .unsize(|p| p as *mut dyn PodTrait),
            );
            black_box(s.first());
            drop(s);
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("unbounded dyn", &mut samples);
}

// ============================================================================
// TLS benchmarks — batch alloc / batch drop / access
// ============================================================================

fn tls_batch_alloc_sized() {
    let val = Pod32::default();

    for _ in 0..WARMUP / 10 {
        let temp: Vec<bounded_byte::BoxSlot<Pod32>> = (0..100)
            .map(|_| bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap())
            .collect();
        drop(temp);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let mut temp: Vec<bounded_byte::BoxSlot<Pod32>> = Vec::with_capacity(100);
        let start = rdtsc_start();
        unroll_100!({
            temp.push(black_box(
                bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap(),
            ));
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
        drop(temp);
    }
    print_row("bounded batch alloc", &mut samples);
}

fn tls_batch_drop_sized() {
    let val = Pod32::default();

    for _ in 0..WARMUP / 10 {
        let temp: Vec<bounded_byte::BoxSlot<Pod32>> = (0..100)
            .map(|_| bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap())
            .collect();
        drop(temp);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        #[allow(clippy::needless_collect)]
        let slots: Vec<bounded_byte::BoxSlot<Pod32>> = (0..100)
            .map(|_| bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap())
            .collect();
        let mut iter = slots.into_iter();
        let start = rdtsc_start();
        unroll_100!({
            drop(black_box(iter.next()));
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("bounded batch drop", &mut samples);
}

fn tls_access_sized() {
    let val = Pod32::default();
    let pool: Vec<bounded_byte::BoxSlot<Pod32>> = (0..1000)
        .map(|_| bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap())
        .collect();

    for p in &pool {
        black_box(p.data[0]);
    }

    let mut rng = 67890u64;
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let base = (rng as usize) % 900;
        let mut idx = base;
        let mut sum = 0u8;
        let start = rdtsc_start();
        unroll_100!({
            sum = sum.wrapping_add(pool[idx % 1000].data[0]);
            idx += 1;
        });
        let end = rdtsc_end();
        black_box(sum);
        samples.push((end - start) / 100);
    }
    print_row("bounded access", &mut samples);
    drop(pool);
}

// ============================================================================
// TLS cold churn
// ============================================================================

fn tls_cold_churn_sized() {
    let val = Pod32::default();
    let polluter = vec![0u8; POLLUTER_SIZE];

    for _ in 0..100 {
        evict_cache(&polluter);
        let s = bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap();
        black_box(s.data[0]);
        drop(s);
    }

    let mut samples = Vec::with_capacity(COLD_SAMPLES);
    for _ in 0..COLD_SAMPLES {
        evict_cache(&polluter);

        let start = rdtsc_start();
        let s = black_box(bounded_byte::BoxSlot::<Pod32>::try_new(val.clone()).unwrap());
        black_box(s.data[0]);
        drop(s);
        let end = rdtsc_end();
        samples.push(end - start);
    }
    print_row("bounded cold Sized", &mut samples);
}

fn tls_cold_churn_dyn() {
    let polluter = vec![0u8; POLLUTER_SIZE];

    for _ in 0..100 {
        evict_cache(&polluter);
        let s: bounded_byte::BoxSlot<dyn PodTrait> =
            bounded_byte::BoxSlot::<Pod32>::try_new(Pod32::default())
                .unwrap()
                .unsize(|p| p as *mut dyn PodTrait);
        black_box(s.first());
        drop(s);
    }

    let mut samples = Vec::with_capacity(COLD_SAMPLES);
    for _ in 0..COLD_SAMPLES {
        evict_cache(&polluter);

        let start = rdtsc_start();
        let s: bounded_byte::BoxSlot<dyn PodTrait> = black_box(
            bounded_byte::BoxSlot::<Pod32>::try_new(Pod32::default())
                .unwrap()
                .unsize(|p| p as *mut dyn PodTrait),
        );
        black_box(s.first());
        drop(s);
        let end = rdtsc_end();
        samples.push(end - start);
    }
    print_row("bounded cold dyn", &mut samples);
}

// ============================================================================
// Raw slab benchmarks — SlotBox (Sized + dyn)
// ============================================================================

fn raw_churn_sized() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(20_000);

    let val = Pod32::default();

    for _ in 0..WARMUP {
        let s = slab.try_insert(val.clone()).unwrap();
        black_box(s.data[0]);
        unsafe { slab.remove(s) };
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        unroll_100!({
            let s = black_box(slab.try_insert(val.clone()).unwrap());
            black_box(s.data[0]);
            unsafe { slab.remove(s) };
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("bounded Sized", &mut samples);
}

fn raw_churn_dyn() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(20_000);

    for _ in 0..WARMUP {
        let s: nexus_slab::byte::Slot<dyn PodTrait> = slab
            .try_insert(Pod32::default())
            .unwrap()
            .unsize(|p| p as *mut dyn PodTrait);
        black_box(s.first());
        unsafe { slab.remove(s) };
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        unroll_100!({
            let s: nexus_slab::byte::Slot<dyn PodTrait> = black_box(
                slab.try_insert(Pod32::default())
                    .unwrap()
                    .unsize(|p| p as *mut dyn PodTrait),
            );
            black_box(s.first());
            unsafe { slab.remove(s) };
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("bounded dyn", &mut samples);
}

fn raw_batch_alloc_sized() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(20_000);

    let val = Pod32::default();

    for _ in 0..WARMUP / 10 {
        let temp: Vec<nexus_slab::byte::Slot<Pod32>> = (0..100)
            .map(|_| slab.try_insert(val.clone()).unwrap())
            .collect();
        for s in temp {
            unsafe { slab.remove(s) };
        }
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let mut temp: Vec<nexus_slab::byte::Slot<Pod32>> = Vec::with_capacity(100);
        let start = rdtsc_start();
        unroll_100!({
            temp.push(black_box(slab.try_insert(val.clone()).unwrap()));
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
        for s in temp {
            unsafe { slab.remove(s) };
        }
    }
    print_row("bounded batch alloc", &mut samples);
}

fn raw_batch_drop_sized() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(20_000);

    let val = Pod32::default();

    for _ in 0..WARMUP / 10 {
        let temp: Vec<nexus_slab::byte::Slot<Pod32>> = (0..100)
            .map(|_| slab.try_insert(val.clone()).unwrap())
            .collect();
        for s in temp {
            unsafe { slab.remove(s) };
        }
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let slots: Vec<nexus_slab::byte::Slot<Pod32>> = (0..100)
            .map(|_| slab.try_insert(val.clone()).unwrap())
            .collect();
        let mut iter = slots.into_iter();
        let start = rdtsc_start();
        unroll_100!({
            unsafe { slab.remove(black_box(iter.next().unwrap())) };
        });
        let end = rdtsc_end();
        samples.push((end - start) / 100);
    }
    print_row("bounded batch drop", &mut samples);
}

fn raw_access_sized() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(20_000);

    let val = Pod32::default();
    let pool: Vec<nexus_slab::byte::Slot<Pod32>> = (0..1000)
        .map(|_| slab.try_insert(val.clone()).unwrap())
        .collect();

    for p in &pool {
        black_box(p.data[0]);
    }

    let mut rng = 67890u64;
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let base = (rng as usize) % 900;
        let mut idx = base;
        let mut sum = 0u8;
        let start = rdtsc_start();
        unroll_100!({
            sum = sum.wrapping_add(pool[idx % 1000].data[0]);
            idx += 1;
        });
        let end = rdtsc_end();
        black_box(sum);
        samples.push((end - start) / 100);
    }
    print_row("bounded access", &mut samples);
    for s in pool {
        unsafe { slab.remove(s) };
    }
}

fn raw_cold_churn_sized() {
    let slab = nexus_slab::bounded::Slab::<nexus_slab::byte::AlignedBytes<64>>::new();
    slab.init(20_000);

    let val = Pod32::default();
    let polluter = vec![0u8; POLLUTER_SIZE];

    for _ in 0..100 {
        evict_cache(&polluter);
        let s = slab.try_insert(val.clone()).unwrap();
        black_box(s.data[0]);
        unsafe { slab.remove(s) };
    }

    let mut samples = Vec::with_capacity(COLD_SAMPLES);
    for _ in 0..COLD_SAMPLES {
        evict_cache(&polluter);

        let start = rdtsc_start();
        let s = black_box(slab.try_insert(val.clone()).unwrap());
        black_box(s.data[0]);
        unsafe { slab.remove(s) };
        let end = rdtsc_end();
        samples.push(end - start);
    }
    print_row("bounded cold Sized", &mut samples);
}

// ============================================================================
// Runners
// ============================================================================

fn run_tls() {
    bounded_byte::Allocator::builder()
        .capacity(20_000)
        .build()
        .expect("bounded init");
    unbounded_byte::Allocator::builder()
        .chunk_size(4096)
        .build()
        .expect("unbounded init");

    println!("TARGET: BoxSlot (TLS byte allocator, 64B slots)");
    println!(
        "BoxSlot<Pod32> handle: {} bytes, BoxSlot<dyn PodTrait> handle: {} bytes",
        std::mem::size_of::<bounded_byte::BoxSlot<Pod32>>(),
        std::mem::size_of::<bounded_byte::BoxSlot<dyn PodTrait>>(),
    );
    println!("{}", "=".repeat(74));

    println!("\nCHURN (alloc + deref + drop, LIFO single-slot)");
    print_header();
    tls_bounded_churn_sized();
    tls_unbounded_churn_sized();
    tls_bounded_churn_dyn();
    tls_unbounded_churn_dyn();

    println!("\nBATCH ALLOC (100 sequential, no interleaved frees)");
    print_header();
    tls_batch_alloc_sized();

    println!("\nBATCH DROP (pre-alloc 100, then free all)");
    print_header();
    tls_batch_drop_sized();

    println!("\nACCESS (random deref from pool of 1000)");
    print_header();
    tls_access_sized();

    println!("\nCOLD CHURN (cache-evicted between each op)");
    print_header();
    tls_cold_churn_sized();
    tls_cold_churn_dyn();
}

fn run_raw() {
    println!("TARGET: SlotBox (raw bounded slab, 64B slots)");
    println!(
        "byte::Slot<Pod32> handle: {} bytes, byte::Slot<dyn PodTrait> handle: {} bytes",
        std::mem::size_of::<nexus_slab::byte::Slot<Pod32>>(),
        std::mem::size_of::<nexus_slab::byte::Slot<dyn PodTrait>>(),
    );
    println!("{}", "=".repeat(74));

    println!("\nCHURN (insert + deref + remove, LIFO single-slot)");
    print_header();
    raw_churn_sized();
    raw_churn_dyn();

    println!("\nBATCH ALLOC (100 sequential, no interleaved frees)");
    print_header();
    raw_batch_alloc_sized();

    println!("\nBATCH DROP (pre-alloc 100, then remove all)");
    print_header();
    raw_batch_drop_sized();

    println!("\nACCESS (random deref from pool of 1000)");
    print_header();
    raw_access_sized();

    println!("\nCOLD CHURN (cache-evicted between each op)");
    print_header();
    raw_cold_churn_sized();
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 2 || !matches!(args[1].as_str(), "tls" | "raw") {
        eprintln!("Usage: {} <tls|raw>", args[0]);
        eprintln!("  tls — BoxSlot benchmarks (TLS byte allocator)");
        eprintln!("  raw — byte::Slot benchmarks (struct-owned slab)");
        std::process::exit(1);
    }

    println!("BYTE SLAB BENCHMARK — nexus-slab");
    println!("Samples: {SAMPLES}, 100 unrolled ops per sample, {WARMUP} warmup iterations");
    println!("All times in CPU cycles (rdtsc)\n");

    match args[1].as_str() {
        "tls" => run_tls(),
        "raw" => run_raw(),
        _ => unreachable!(),
    }
}
