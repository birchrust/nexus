//! Cycle-level WebSocket benchmark — nexus-async-rt backend.
//!
//! Pure userspace: no runtime, no scheduler, no kernel.
//! Fenced rdtsc, noop-waker executor, memory-backed mock.
//!
//! Run with:
//!   cargo build --release -p nexus-async-net --no-default-features --features nexus \
//!     --example perf_ws_cycles_nexus
//!   taskset -c 0 ./target/release/examples/perf_ws_cycles_nexus

use std::hint::black_box;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use nexus_async_net::ws::WsStream;
use nexus_async_rt::{AsyncRead, AsyncWrite};
use nexus_net::ws::{FrameReader, FrameWriter, Role};

// =============================================================================
// Timing
// =============================================================================

#[inline(always)]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    unsafe {
        let tsc = core::arch::x86_64::__rdtscp(&mut 0u32 as *mut _);
        core::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_header() {
    println!(
        "  {:<45} {:>6} {:>6} {:>6} {:>7} {:>7}",
        "operation", "p50", "p90", "p99", "p99.9", "max"
    );
    println!("  {}", "-".repeat(83));
}

fn print_row(label: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<45} {:>6} {:>6} {:>6} {:>7} {:>7}",
        label,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        samples[samples.len() - 1],
    );
}

// =============================================================================
// Noop-waker executor
// =============================================================================

fn noop_waker() -> Waker {
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

/// Single-poll executor. Mock stream is always Ready — if this panics,
/// the future has a real await point we didn't expect.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("future returned Pending with synchronous mock"),
    }
}

// =============================================================================
// Mock stream (nexus-async-rt traits)
// =============================================================================

struct MockStream<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> MockStream<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
}

impl AsyncRead for MockStream<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let remaining = &self.data[self.pos..];
        let n = remaining.len().min(buf.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        self.pos += n;
        Poll::Ready(Ok(n))
    }
}

impl AsyncWrite for MockStream<'_> {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

// =============================================================================
// Frame construction
// =============================================================================

fn make_frame(payload: &[u8], opcode: u8) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.push(0x80 | opcode);
    if payload.len() <= 125 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    frame
}

fn build_wire(payload_size: usize, count: usize) -> Vec<u8> {
    let payload = vec![0x42u8; payload_size];
    let frame = make_frame(&payload, 0x2); // binary
    let mut wire = Vec::with_capacity(frame.len() * count);
    for _ in 0..count {
        wire.extend_from_slice(&frame);
    }
    wire
}

fn make_ws(wire: &[u8]) -> WsStream<MockStream<'_>> {
    let mock = MockStream::new(wire);
    let reader = FrameReader::builder()
        .role(Role::Client)
        .buffer_capacity(256 * 1024)
        .build();
    WsStream::from_parts(mock, reader, FrameWriter::new(Role::Client))
}

// =============================================================================
// Constants
// =============================================================================

const WARMUP: usize = 5_000;
const SAMPLES: usize = 50_000;
const BATCH: u64 = 64;
const BATCH_WARMUP: usize = 500;
const BATCH_SAMPLES: usize = 10_000;

// =============================================================================
// recv benchmarks
// =============================================================================

fn bench_recv_per_msg(label: &str, payload_size: usize) {
    let total = WARMUP + SAMPLES;
    let wire = build_wire(payload_size, total);
    let mut ws = make_ws(&wire);
    let mut samples = Vec::with_capacity(SAMPLES);

    for i in 0..total {
        let start = rdtsc_start();
        block_on(async {
            let msg = ws.recv().await.unwrap();
            black_box(&msg);
        });
        let end = rdtsc_end();
        if i >= WARMUP {
            samples.push(end - start);
        }
    }

    print_row(label, &mut samples);
}

fn bench_recv_batched(label: &str, payload_size: usize) {
    let total_batches = BATCH_WARMUP + BATCH_SAMPLES;
    let total_msgs = total_batches * BATCH as usize;
    let wire = build_wire(payload_size, total_msgs);
    let mut ws = make_ws(&wire);
    let mut samples = Vec::with_capacity(BATCH_SAMPLES);

    for i in 0..total_batches {
        let start = rdtsc_start();
        block_on(async {
            for _ in 0..BATCH {
                let msg = ws.recv().await.unwrap();
                black_box(&msg);
            }
        });
        let end = rdtsc_end();
        if i >= BATCH_WARMUP {
            samples.push((end - start) / BATCH);
        }
    }

    print_row(label, &mut samples);
}

// =============================================================================
// send benchmarks
// =============================================================================

fn bench_send_per_msg(label: &str, payload_size: usize) {
    let text = "x".repeat(payload_size);
    let wire: &[u8] = &[];
    let mut ws = make_ws(wire);
    let mut samples = Vec::with_capacity(SAMPLES);
    let total = WARMUP + SAMPLES;

    for i in 0..total {
        let start = rdtsc_start();
        block_on(ws.send_text(&text)).unwrap();
        let end = rdtsc_end();
        if i >= WARMUP {
            samples.push(end - start);
        }
    }

    print_row(label, &mut samples);
}

fn bench_send_batched(label: &str, payload_size: usize) {
    let text = "x".repeat(payload_size);
    let wire: &[u8] = &[];
    let mut ws = make_ws(wire);
    let mut samples = Vec::with_capacity(BATCH_SAMPLES);
    let total_batches = BATCH_WARMUP + BATCH_SAMPLES;

    for i in 0..total_batches {
        let start = rdtsc_start();
        block_on(async {
            for _ in 0..BATCH {
                ws.send_text(&text).await.unwrap();
            }
        });
        let end = rdtsc_end();
        if i >= BATCH_WARMUP {
            samples.push((end - start) / BATCH);
        }
    }

    print_row(label, &mut samples);
}

// =============================================================================
// main
// =============================================================================

fn main() {
    println!("\n  === WebSocket Cycle Benchmark (nexus backend) ===");
    println!("  Pure userspace, noop waker, memory-backed mock");
    println!(
        "  Per-message: {} warmup + {} samples",
        WARMUP, SAMPLES
    );
    println!(
        "  Batched: {} warmup + {} samples x {} ops\n",
        BATCH_WARMUP, BATCH_SAMPLES, BATCH
    );

    println!("  --- recv per-message (cycles) ---");
    print_header();
    bench_recv_per_msg("recv binary 40B", 40);
    bench_recv_per_msg("recv binary 128B", 128);
    bench_recv_per_msg("recv binary 1024B", 1024);

    println!("\n  --- recv batched x64 (amortized cycles/msg) ---");
    print_header();
    bench_recv_batched("recv binary 40B (batched)", 40);
    bench_recv_batched("recv binary 128B (batched)", 128);

    println!("\n  --- send_text per-message (cycles) ---");
    print_header();
    bench_send_per_msg("send_text 40B", 40);
    bench_send_per_msg("send_text 128B", 128);
    bench_send_per_msg("send_text 1024B", 1024);

    println!("\n  --- send_text batched x64 (amortized cycles/msg) ---");
    print_header();
    bench_send_batched("send_text 40B (batched)", 40);
    bench_send_batched("send_text 128B (batched)", 128);

    println!();
}
