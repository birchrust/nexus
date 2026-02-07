//! Bare push loop for perf stat analysis.
//!
//! Zero instrumentation — let hardware counters do the work.
//! Pre-allocates all handles upfront, then pushes in one straight run.
//! No pop/clear interleaved — pure push measurement.
//!
//! Run with:
//!   cargo build --release --example perf_push_stat
//!   perf stat -r 25 -e cycles,instructions,... taskset -c 0 ./target/release/examples/perf_push_stat

use std::hint::black_box;

mod pq {
    nexus_collections::heap_allocator!(u64, unbounded);
}

const COUNT: usize = 500_000;

struct Xorshift {
    state: u64,
}

impl Xorshift {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }
}

fn main() {
    pq::Allocator::builder().chunk_size(8192).build().unwrap();

    let mut rng = Xorshift::new(0xDEAD_BEEF_CAFE_BABEu64);

    // Pre-allocate all handles
    let handles: Vec<pq::Handle> = (0..COUNT).map(|_| pq::create_node(rng.next())).collect();

    let mut heap = pq::Heap::new();

    // Warmup: push/clear to fault pages and warm TLB
    for handle in &handles {
        heap.push(handle);
    }
    heap.clear();

    // ---- Measured section: pure push, no cleanup ----
    for handle in &handles {
        black_box(heap.push(handle));
    }

    black_box(&heap);
    heap.clear();
}
