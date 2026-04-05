//! REST latency comparison: nexus-async-rt vs tokio + reqwest.
//!
//! Ping-pong HTTP request/response over TCP loopback. HDR histogram
//! for tail latency. Each iteration sends a GET, reads the full
//! response body, and records the round-trip time.
//!
//! Usage:
//!   cargo run --release -p nexus-net --features nexus-rt --example perf_rest_compare

#![cfg(unix)]

use std::hint::black_box;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;

const WARMUP: usize = 1_000;
const ITERATIONS: usize = 50_000;

/// Canned JSON body (~80 bytes, realistic for a quote/status endpoint).
const RESPONSE_BODY: &str =
    r#"{"symbol":"BTC-USD","bid":68421.50,"ask":68422.00,"ts":1717459200}"#;

// =============================================================================
// Shared: blocking HTTP echo server (std threads)
// =============================================================================

/// Spawn a blocking HTTP server that responds to every request with
/// a canned 200 + JSON body. Returns the listening address.
///
/// The server runs on a background thread and handles requests on a
/// single keep-alive connection (matches the benchmark pattern).
fn spawn_http_server(addr: SocketAddr) -> SocketAddr {
    let listener = std::net::TcpListener::bind(addr).expect("bind failed");
    let local = listener.local_addr().unwrap();

    std::thread::spawn(move || {
        // Accept one connection (keep-alive).
        let (mut tcp, _) = listener.accept().expect("accept failed");
        tcp.set_nodelay(true).unwrap();
        let mut buf = [0u8; 4096];

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
            RESPONSE_BODY.len(),
            RESPONSE_BODY
        );
        let resp_bytes = response.as_bytes();

        loop {
            // Read until we see the end of HTTP headers (\r\n\r\n).
            let n = match tcp.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            // Simple: assume full request arrives in one read (loopback).
            if n > 0 {
                if tcp.write_all(resp_bytes).is_err() {
                    break;
                }
            }
        }
    });

    // Give server time to start.
    std::thread::sleep(Duration::from_millis(20));
    local
}

// =============================================================================
// Result printing
// =============================================================================

fn print_histogram(label: &str, hist: &Histogram<u64>) {
    println!("{label}");
    println!(
        "  p50:    {:.2} us",
        hist.value_at_percentile(50.0) as f64 / 1000.0
    );
    println!(
        "  p99:    {:.2} us",
        hist.value_at_percentile(99.0) as f64 / 1000.0
    );
    println!(
        "  p999:   {:.2} us",
        hist.value_at_percentile(99.9) as f64 / 1000.0
    );
    println!(
        "  p9999:  {:.2} us",
        hist.value_at_percentile(99.99) as f64 / 1000.0
    );
    println!(
        "  max:    {:.2} us",
        hist.max() as f64 / 1000.0
    );
    println!();
}

fn new_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(1, 10_000_000_000, 3).expect("histogram bounds valid")
}

// =============================================================================
// 1. nexus-async-rt + rest::Client
// =============================================================================

fn bench_nexus_async_rt(server_addr: SocketAddr) -> Histogram<u64> {
    use nexus_async_rt::{DefaultRuntime, TcpStream};
    use nexus_net::http::ResponseReader;
    use nexus_net::rest::{Client, RequestWriter};
    use nexus_rt::WorldBuilder;

    let mut world = WorldBuilder::new().build();
    let mut rt = DefaultRuntime::new(&mut world, 64);

    rt.block_on(async move {
        let io = nexus_async_rt::io();

        let tcp = TcpStream::connect(server_addr, io).expect("connect failed");
        tcp.set_nodelay(true).expect("set_nodelay");

        let mut conn = Client::new(tcp);
        let mut writer = RequestWriter::new(&server_addr.to_string()).expect("writer");
        let mut reader = ResponseReader::new(4096);

        // Warmup
        for _ in 0..WARMUP {
            let req = writer.get("/status").finish().expect("finish");
            let resp = conn.send(req, &mut reader).await.expect("send");
            black_box(resp.body());
        }

        // Measure
        let mut hist = new_histogram();
        for _ in 0..ITERATIONS {
            let start = Instant::now();
            let req = writer.get("/status").finish().expect("finish");
            let resp = conn.send(req, &mut reader).await.expect("send");
            let elapsed = start.elapsed().as_nanos() as u64;
            black_box(resp.body());
            hist.record(elapsed).expect("record");
        }

        hist
    })
}

// =============================================================================
// 2. tokio + nexus-async-net rest
// =============================================================================

fn bench_tokio_nexus_async_net(server_addr: SocketAddr) -> Histogram<u64> {
    use nexus_async_net::rest::AsyncHttpConnection;
    use nexus_net::http::ResponseReader;
    use nexus_net::rest::RequestWriter;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(async {
        let tcp = tokio::net::TcpStream::connect(server_addr)
            .await
            .expect("connect failed");
        tcp.set_nodelay(true).expect("set_nodelay");

        let mut conn = AsyncHttpConnection::new(tcp);
        let mut writer = RequestWriter::new(&server_addr.to_string()).expect("writer");
        let mut reader = ResponseReader::new(4096);

        // Warmup
        for _ in 0..WARMUP {
            let req = writer.get("/status").finish().expect("finish");
            let resp = conn.send(req, &mut reader).await.expect("send");
            black_box(resp.body());
        }

        // Measure
        let mut hist = new_histogram();
        for _ in 0..ITERATIONS {
            let start = Instant::now();
            let req = writer.get("/status").finish().expect("finish");
            let resp = conn.send(req, &mut reader).await.expect("send");
            let elapsed = start.elapsed().as_nanos() as u64;
            black_box(resp.body());
            hist.record(elapsed).expect("record");
        }

        hist
    })
}

// =============================================================================
// 3. reqwest (blocking, keep-alive)
// =============================================================================

fn bench_reqwest_blocking(server_addr: SocketAddr) -> Histogram<u64> {
    let client = reqwest::blocking::Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(1)
        .build()
        .expect("reqwest client");

    let url = format!("http://{server_addr}/status");

    // Warmup
    for _ in 0..WARMUP {
        let resp = client.get(&url).send().expect("send");
        black_box(resp.bytes().expect("bytes"));
    }

    // Measure
    let mut hist = new_histogram();
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let resp = client.get(&url).send().expect("send");
        let body = resp.bytes().expect("bytes");
        let elapsed = start.elapsed().as_nanos() as u64;
        black_box(&body);
        hist.record(elapsed).expect("record");
    }

    hist
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!();
    println!("=== REST TCP Loopback Validation ===");
    println!("Kernel-dominated — confirms end-to-end correctness, not runtime speed.");
    println!("Response: {} bytes JSON, {ITERATIONS} iterations, keep-alive", RESPONSE_BODY.len());
    println!();

    // Each variant gets its own server to avoid port reuse issues.
    let addr1 = spawn_http_server("127.0.0.1:0".parse().unwrap());
    let addr2 = spawn_http_server("127.0.0.1:0".parse().unwrap());
    let addr3 = spawn_http_server("127.0.0.1:0".parse().unwrap());

    let h1 = bench_nexus_async_rt(addr1);
    print_histogram("nexus-async-rt + rest::Client", &h1);

    let h2 = bench_tokio_nexus_async_net(addr2);
    print_histogram("tokio + nexus-async-net", &h2);

    let h3 = bench_reqwest_blocking(addr3);
    print_histogram("reqwest (blocking, keep-alive)", &h3);
}
