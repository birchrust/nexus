//! HDR histogram benchmark for all critical heap + list operations.
//!
//! Measures each operation in isolation with rdtscp per-op timing.
//!
//! Run with:
//!   cargo build --release --example perf_push_hist
//!   taskset -c 0 ./target/release/examples/perf_push_hist

use hdrhistogram::Histogram;
use std::hint::black_box;

mod hpq {
    nexus_collections::heap_allocator!(u64, bounded);
}

mod lq {
    nexus_collections::list_allocator!(u64, bounded);
}

const CAPACITY: usize = 100_000;
const N: usize = 50_000;

#[inline(always)]
fn rdtscp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut aux: u32 = 0;
        std::arch::x86_64::__rdtscp(&raw mut aux)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        panic!("rdtscp only supported on x86_64");
    }
}

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

fn print_hist(label: &str, hist: &Histogram<u64>) {
    println!(
        "  {:<24} p50={:>5}  p90={:>5}  p99={:>5}  p999={:>6}  max={:>8}  (n={})",
        label,
        hist.value_at_quantile(0.50),
        hist.value_at_quantile(0.90),
        hist.value_at_quantile(0.99),
        hist.value_at_quantile(0.999),
        hist.max(),
        hist.len()
    );
}

fn new_hist() -> Histogram<u64> {
    Histogram::new(3).unwrap()
}

fn main() {
    hpq::Allocator::builder()
        .capacity(CAPACITY)
        .build()
        .unwrap();
    lq::Allocator::builder().capacity(CAPACITY).build().unwrap();

    let mut rng = Xorshift::new(0xDEAD_BEEF_CAFE_BABEu64);

    // Pre-allocate heap handles
    let heap_handles: Vec<hpq::Handle> = (0..N)
        .map(|_| hpq::create_node(rng.next()).expect("alloc"))
        .collect();

    // Pre-allocate list handles
    let list_handles: Vec<lq::Handle> = (0..N)
        .map(|_| lq::create_node(rng.next()).expect("alloc"))
        .collect();

    println!("OPERATION LATENCY (cycles) — all critical methods");
    println!("================================================================\n");

    // =================================================================
    // HEAP
    // =================================================================
    println!("HEAP");
    println!("----");

    // heap push (growing)
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        // warmup
        for h in heap_handles.iter().take(5000) {
            heap.link(h);
        }
        heap.clear();
        // measure
        for h in &heap_handles {
            let s = rdtscp();
            heap.link(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("push (growing)", &hist);
        heap.clear();
    }

    // heap push (steady-state push-pop)
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        let half = N / 2;
        for h in heap_handles.iter().take(half) {
            heap.link(h);
        }
        for h in heap_handles.iter().skip(half) {
            let s = rdtscp();
            heap.link(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
            let _ = black_box(heap.pop());
        }
        print_hist("push (steady @25k)", &hist);
        heap.clear();
    }

    // heap pop
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        for h in &heap_handles {
            heap.link(h);
        }
        while !heap.is_empty() {
            let s = rdtscp();
            let _ = black_box(heap.pop());
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("pop (drain 50k)", &hist);
    }

    // heap unlink (middle elements)
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        for h in &heap_handles {
            heap.link(h);
        }
        for h in &heap_handles {
            let s = rdtscp();
            heap.unlink(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("unlink (all, arb order)", &hist);
    }

    // heap unlink_unchecked
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        for h in &heap_handles {
            heap.link(h);
        }
        for h in &heap_handles {
            let s = rdtscp();
            unsafe { heap.unlink_unchecked(h) };
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("unlink_unchk (all, arb)", &hist);
    }

    // heap try_push (allocation + link)
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        let mut rng2 = Xorshift::new(0xCAFE_BABE_DEAD_BEEFu64);
        // warmup: fill and drain to warm slab freelist
        let warmup: Vec<hpq::Handle> = (0..5000)
            .map(|_| heap.try_push(rng2.next()).unwrap())
            .collect();
        heap.clear();
        drop(warmup);
        // measure
        for _ in 0..N {
            let val = rng2.next();
            let s = rdtscp();
            let h = heap.try_push(val).unwrap();
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
            heap.clear();
            drop(h);
        }
        print_hist("try_push (alloc+link)", &hist);
    }

    // heap peek
    {
        let mut heap = hpq::Heap::new(hpq::Allocator);
        let mut hist = new_hist();
        for h in heap_handles.iter().take(N / 2) {
            heap.link(h);
        }
        for _ in 0..N {
            let s = rdtscp();
            let _ = black_box(heap.peek());
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("peek", &hist);
        heap.clear();
    }

    println!();

    // =================================================================
    // LIST
    // =================================================================
    println!("LIST");
    println!("----");

    // list link_back (growing)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in list_handles.iter().take(5000) {
            list.link_back(h);
        }
        list.clear();
        for h in &list_handles {
            let s = rdtscp();
            list.link_back(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("link_back (growing)", &hist);
        list.clear();
    }

    // list link_front (growing)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in list_handles.iter().take(5000) {
            list.link_front(h);
        }
        list.clear();
        for h in &list_handles {
            let s = rdtscp();
            list.link_front(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("link_front (growing)", &hist);
        list.clear();
    }

    // list link_back (steady-state push-pop)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        let half = N / 2;
        for h in list_handles.iter().take(half) {
            list.link_back(h);
        }
        for h in list_handles.iter().skip(half) {
            let s = rdtscp();
            list.link_back(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
            let _ = black_box(list.pop_front());
        }
        print_hist("link_back (steady @25k)", &hist);
        list.clear();
    }

    // list pop_front
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        while !list.is_empty() {
            let s = rdtscp();
            let _ = black_box(list.pop_front());
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("pop_front (drain 50k)", &hist);
    }

    // list pop_back
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        while !list.is_empty() {
            let s = rdtscp();
            let _ = black_box(list.pop_back());
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("pop_back (drain 50k)", &hist);
    }

    // list unlink (arbitrary order)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        for h in &list_handles {
            let s = rdtscp();
            list.unlink(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("unlink (all, arb order)", &hist);
    }

    // list unlink_unchecked (arbitrary order)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        for h in &list_handles {
            let s = rdtscp();
            unsafe { list.unlink_unchecked(h) };
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("unlink_unchk (all, arb)", &hist);
    }

    // list move_to_front (from random positions)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        for h in &list_handles {
            let s = rdtscp();
            list.move_to_front(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("move_to_front (all)", &hist);
        list.clear();
    }

    // list move_to_front_unchecked
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        for h in &list_handles {
            let s = rdtscp();
            unsafe { list.move_to_front_unchecked(h) };
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("move_front_unchk (all)", &hist);
        list.clear();
    }

    // list move_to_back (from random positions)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        for h in list_handles.iter().rev() {
            let s = rdtscp();
            list.move_to_back(h);
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("move_to_back (all)", &hist);
        list.clear();
    }

    // list move_to_back_unchecked
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        for h in &list_handles {
            list.link_back(h);
        }
        for h in list_handles.iter().rev() {
            let s = rdtscp();
            unsafe { list.move_to_back_unchecked(h) };
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
        }
        print_hist("move_back_unchk (all)", &hist);
        list.clear();
    }

    // list try_push_back (allocation + link)
    {
        let mut list = lq::List::new(lq::Allocator);
        let mut hist = new_hist();
        let mut rng2 = Xorshift::new(0xCAFE_BABE_DEAD_BEEFu64);
        // warmup: fill and drain to warm slab freelist
        let warmup: Vec<lq::Handle> = (0..5000)
            .map(|_| list.try_push_back(rng2.next()).unwrap())
            .collect();
        list.clear();
        drop(warmup);
        // measure
        for _ in 0..N {
            let val = rng2.next();
            let s = rdtscp();
            let h = list.try_push_back(val).unwrap();
            let e = rdtscp();
            let _ = hist.record(e.wrapping_sub(s));
            list.clear();
            drop(h);
        }
        print_hist("try_push_back (alloc+lnk)", &hist);
    }
}
