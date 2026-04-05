//! WebSocket latency comparison: nexus-async-rt vs tokio-tungstenite.
//!
//! Ping-pong echo over TCP loopback. HDR histogram for tail latency.
//!
//! Usage:
//!   cargo run --release -p nexus-net --features nexus-rt --example perf_ws_compare

#![cfg(unix)]

use std::hint::black_box;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;

const WARMUP: usize = 1_000;
const ITERATIONS: usize = 50_000;
const PAYLOAD: &str = r#"{"bid":1.2345,"ask":1.2346,"ts":1234567890}"#;

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
    // 1 ns to 10 seconds, 3 significant digits
    Histogram::new_with_bounds(1, 10_000_000_000, 3).expect("histogram bounds valid")
}

// =============================================================================
// 1. nexus-async-rt + ws::Client
// =============================================================================

fn bench_nexus_async_rt() -> Histogram<u64> {
    use nexus_async_rt::{DefaultRuntime, TcpListener, TcpStream, spawn};
    use nexus_net::ws;
    use nexus_rt::WorldBuilder;

    let addr: SocketAddr = "127.0.0.1:19100".parse().expect("valid addr");
    let url = "ws://127.0.0.1:19100/";

    let mut world = WorldBuilder::new().build();
    let mut rt = DefaultRuntime::new(&mut world, 64);

    rt.block_on(async move {
        let io = nexus_async_rt::io();
        let mut listener = TcpListener::bind(addr, io).expect("bind failed");

        spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept failed");
            tcp.set_nodelay(true).expect("set_nodelay failed");
            let mut server = ws::Client::accept(tcp).await.expect("ws accept failed");
            // Echo loop: copy payload to break the borrow from recv().
            loop {
                let reply = match server.recv().await {
                    Ok(Some(ws::Message::Text(t))) => Some(t.to_owned()),
                    Ok(Some(ws::Message::Binary(_))) => None, // unused in this bench
                    _ => break,
                };
                if let Some(text) = reply {
                    server.send_text(&text).await.expect("send failed");
                }
            }
        });

        nexus_async_rt::sleep(Duration::from_millis(50)).await;

        let tcp = TcpStream::connect(addr, io).expect("connect failed");
        tcp.set_nodelay(true).expect("set_nodelay failed");
        let mut client = ws::Client::connect_with(tcp, url)
            .await
            .expect("ws connect failed");

        // Warmup
        for _ in 0..WARMUP {
            client.send_text(PAYLOAD).await.expect("send failed");
            let msg = client.recv().await.expect("recv failed").expect("no msg");
            black_box(&msg);
        }

        // Measure
        let mut hist = new_histogram();
        for _ in 0..ITERATIONS {
            let start = Instant::now();
            client.send_text(PAYLOAD).await.expect("send failed");
            let msg = client.recv().await.expect("recv failed").expect("no msg");
            let elapsed = start.elapsed().as_nanos() as u64;
            black_box(&msg);
            hist.record(elapsed).expect("record failed");
        }

        client
            .close(ws::CloseCode::Normal, "done")
            .await
            .expect("close failed");

        hist
    })
}

// =============================================================================
// 2. tokio + nexus-async-net
// =============================================================================

fn bench_tokio_nexus_async_net() -> Histogram<u64> {
    use nexus_async_net::ws::WsStream;
    use nexus_net::ws::Message;

    let addr: SocketAddr = "127.0.0.1:19101".parse().expect("valid addr");
    let url = "ws://127.0.0.1:19101/";

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind failed");

        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept failed");
            tcp.set_nodelay(true).expect("set_nodelay failed");
            let mut server = WsStream::accept(tcp).await.expect("ws accept failed");
            loop {
                let reply = match server.recv().await {
                    Ok(Some(Message::Text(t))) => Some(t.to_owned()),
                    Ok(Some(Message::Binary(_))) => None,
                    _ => break,
                };
                if let Some(text) = reply {
                    server.send_text(&text).await.expect("send failed");
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tcp = tokio::net::TcpStream::connect(addr)
            .await
            .expect("connect failed");
        tcp.set_nodelay(true).expect("set_nodelay failed");
        let mut client = WsStream::connect_with(tcp, url)
            .await
            .expect("ws connect failed");

        // Warmup
        for _ in 0..WARMUP {
            client.send_text(PAYLOAD).await.expect("send failed");
            let msg = client.recv().await.expect("recv failed").expect("no msg");
            black_box(&msg);
        }

        // Measure
        let mut hist = new_histogram();
        for _ in 0..ITERATIONS {
            let start = Instant::now();
            client.send_text(PAYLOAD).await.expect("send failed");
            let msg = client.recv().await.expect("recv failed").expect("no msg");
            let elapsed = start.elapsed().as_nanos() as u64;
            black_box(&msg);
            hist.record(elapsed).expect("record failed");
        }

        client
            .close(nexus_net::ws::CloseCode::Normal, "done")
            .await
            .expect("close failed");

        hist
    })
}

// =============================================================================
// 3. tokio + tokio-tungstenite
// =============================================================================

fn bench_tokio_tungstenite() -> Histogram<u64> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TungMsg;

    let addr: SocketAddr = "127.0.0.1:19102".parse().expect("valid addr");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind failed");

        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept failed");
            tcp.set_nodelay(true).expect("set_nodelay failed");
            let mut server = tokio_tungstenite::accept_async(tcp)
                .await
                .expect("ws accept failed");
            while let Some(msg) = server.next().await {
                match msg {
                    Ok(TungMsg::Text(t)) => {
                        server.send(TungMsg::Text(t)).await.expect("send failed");
                    }
                    Ok(TungMsg::Binary(b)) => {
                        server
                            .send(TungMsg::Binary(b))
                            .await
                            .expect("send failed");
                    }
                    _ => break,
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tcp = tokio::net::TcpStream::connect(addr)
            .await
            .expect("connect failed");
        tcp.set_nodelay(true).expect("set_nodelay failed");
        let (mut client, _) = tokio_tungstenite::client_async("ws://127.0.0.1:19102/", tcp)
            .await
            .expect("ws connect failed");

        // Warmup
        for _ in 0..WARMUP {
            client
                .send(TungMsg::Text(PAYLOAD.into()))
                .await
                .expect("send failed");
            let msg = client.next().await.expect("stream ended").expect("recv failed");
            black_box(&msg);
        }

        // Measure
        let mut hist = new_histogram();
        for _ in 0..ITERATIONS {
            let start = Instant::now();
            client
                .send(TungMsg::Text(PAYLOAD.into()))
                .await
                .expect("send failed");
            let msg = client.next().await.expect("stream ended").expect("recv failed");
            let elapsed = start.elapsed().as_nanos() as u64;
            black_box(&msg);
            hist.record(elapsed).expect("record failed");
        }

        client
            .close(None)
            .await
            .expect("close failed");

        hist
    })
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!();
    println!("=== WebSocket Ping-Pong Latency ({ITERATIONS} iterations) ===");
    println!("Payload: {} bytes text", PAYLOAD.len());
    println!();

    let h1 = bench_nexus_async_rt();
    print_histogram("nexus-async-rt + ws::Client", &h1);

    let h2 = bench_tokio_nexus_async_net();
    print_histogram("tokio + nexus-async-net", &h2);

    let h3 = bench_tokio_tungstenite();
    print_histogram("tokio + tokio-tungstenite", &h3);
}
