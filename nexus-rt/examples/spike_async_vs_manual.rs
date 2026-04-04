//! Spike: async state machine vs hand-rolled state machine.
//!
//! Compares raw poll() cost — no executor, no waker scheduling,
//! just the state machine transition itself.
//!
//! ```bash
//! taskset -c 0 cargo run --release -p nexus-rt --example spike_async_vs_manual
//! ```

use std::future::Future;
use std::hint::black_box;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

// =============================================================================
// Bench infrastructure
// =============================================================================

const ITERATIONS: usize = 100_000;
const WARMUP: usize = 10_000;
const BATCH: u64 = 100;

#[inline(always)]
#[cfg(target_arch = "x86_64")]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
#[cfg(target_arch = "x86_64")]
fn rdtsc_end() -> u64 {
    unsafe {
        let mut aux = 0u32;
        let tsc = core::arch::x86_64::__rdtscp(&raw mut aux);
        core::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn bench_batched<F: FnMut() -> u64>(name: &str, mut f: F) {
    for _ in 0..WARMUP {
        black_box(f());
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            black_box(f());
        }
        let end = rdtsc_end();
        samples.push(end.wrapping_sub(start) / BATCH);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let p999 = percentile(&samples, 99.9);
    println!("{:<56} {:>8} {:>8} {:>8}", name, p50, p99, p999);
}

fn print_header(title: &str) {
    println!("\n=== {} ===\n", title);
    println!(
        "{:<56} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(82));
}

// =============================================================================
// Noop waker — we don't care about waking, just polling
// =============================================================================

fn noop_waker() -> Waker {
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

// =============================================================================
// Hand-rolled state machine
// =============================================================================

enum ManualSM {
    Step1(u64),
    Step2(u64),
    Step3(u64),
    Done,
}

impl ManualSM {
    fn new(input: u64) -> Self {
        Self::Step1(input)
    }

    #[inline(always)]
    fn poll(&mut self) -> Poll<u64> {
        match std::mem::replace(self, Self::Done) {
            Self::Step1(x) => {
                *self = Self::Step2(x.wrapping_mul(3));
                Poll::Pending
            }
            Self::Step2(x) => {
                *self = Self::Step3(x.wrapping_add(7));
                Poll::Pending
            }
            Self::Step3(x) => Poll::Ready(x >> 1),
            Self::Done => unreachable!(),
        }
    }
}

// =============================================================================
// Async state machine (compiler-generated)
// =============================================================================

/// A future that yields once then completes.
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

fn yield_once() -> YieldOnce {
    YieldOnce(false)
}

async fn async_sm(input: u64) -> u64 {
    let x = input.wrapping_mul(3);
    yield_once().await;
    let x = x.wrapping_add(7);
    yield_once().await;
    x >> 1
}

// =============================================================================
// Benchmarks
// =============================================================================

fn main() {
    println!("Async State Machine vs Hand-Rolled State Machine\n");
    println!("Both perform: multiply by 3, yield, add 7, yield, shift right 1\n");

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    // =========================================================================
    // 1. Poll to completion: 3 polls (2 Pending + 1 Ready)
    // =========================================================================

    print_header("Full run: 3 polls to completion");

    bench_batched("Hand-rolled (3 polls)", || {
        let mut sm = ManualSM::new(42);
        assert!(sm.poll().is_pending());
        assert!(sm.poll().is_pending());
        match sm.poll() {
            Poll::Ready(v) => v,
            Poll::Pending => unreachable!(),
        }
    });

    bench_batched("Async fn (3 polls)", || {
        let mut fut = async_sm(42);
        let mut pinned = unsafe { Pin::new_unchecked(&mut fut) };
        assert!(pinned.as_mut().poll(&mut cx).is_pending());
        assert!(pinned.as_mut().poll(&mut cx).is_pending());
        match pinned.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => unreachable!(),
        }
    });

    // =========================================================================
    // 2. Single poll cost (one state transition)
    // =========================================================================

    print_header("Single poll (one state transition)");

    bench_batched("Hand-rolled (1 poll, Pending)", || {
        let mut sm = ManualSM::new(42);
        match sm.poll() {
            Poll::Pending => 0,
            Poll::Ready(v) => v,
        }
    });

    bench_batched("Async fn (1 poll, Pending)", || {
        let mut fut = async_sm(42);
        let mut pinned = unsafe { Pin::new_unchecked(&mut fut) };
        match pinned.as_mut().poll(&mut cx) {
            Poll::Pending => 0,
            Poll::Ready(v) => v,
        }
    });

    // =========================================================================
    // 3. Larger state machine (10 yields)
    // =========================================================================

    print_header("10-step pipeline (10 yields)");

    // Hand-rolled 10-step
    struct Manual10 {
        state: u8,
        val: u64,
    }

    impl Manual10 {
        fn new(input: u64) -> Self {
            Self {
                state: 0,
                val: input,
            }
        }

        #[inline(always)]
        fn poll(&mut self) -> Poll<u64> {
            if self.state < 10 {
                self.val = self.val.wrapping_add(self.state as u64);
                self.state += 1;
                Poll::Pending
            } else {
                Poll::Ready(self.val)
            }
        }
    }

    async fn async_10(input: u64) -> u64 {
        let mut x = input;
        x = x.wrapping_add(0); yield_once().await;
        x = x.wrapping_add(1); yield_once().await;
        x = x.wrapping_add(2); yield_once().await;
        x = x.wrapping_add(3); yield_once().await;
        x = x.wrapping_add(4); yield_once().await;
        x = x.wrapping_add(5); yield_once().await;
        x = x.wrapping_add(6); yield_once().await;
        x = x.wrapping_add(7); yield_once().await;
        x = x.wrapping_add(8); yield_once().await;
        x = x.wrapping_add(9); yield_once().await;
        x
    }

    bench_batched("Hand-rolled (11 polls)", || {
        let mut sm = Manual10::new(42);
        loop {
            match sm.poll() {
                Poll::Pending => continue,
                Poll::Ready(v) => break v,
            }
        }
    });

    bench_batched("Async fn (11 polls)", || {
        let mut fut = async_10(42);
        let mut pinned = unsafe { Pin::new_unchecked(&mut fut) };
        loop {
            match pinned.as_mut().poll(&mut cx) {
                Poll::Pending => continue,
                Poll::Ready(v) => break v,
            }
        }
    });

    // =========================================================================
    // 4. Size comparison
    // =========================================================================

    println!("\n=== State Machine Sizes ===\n");
    println!(
        "ManualSM (3-step):  {} bytes",
        std::mem::size_of::<ManualSM>()
    );
    println!(
        "async_sm future:    {} bytes",
        std::mem::size_of_val(&async_sm(0))
    );
    println!(
        "Manual10 (10-step): {} bytes",
        std::mem::size_of::<Manual10>()
    );
    println!(
        "async_10 future:    {} bytes",
        std::mem::size_of_val(&async_10(0))
    );
}
