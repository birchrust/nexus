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
const THROUGHPUT_MSGS: u64 = 500_000;
const INMEMORY_MSGS: u64 = 1_000_000;
const PAYLOAD: &str = r#"{"bid":1.2345,"ask":1.2346,"ts":1234567890}"#;

/// Binary payloads for in-memory parse benchmark.
const BINARY_SIZES: &[usize] = &[40, 128, 512];

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
// Throughput: server blasts pre-built frames, client receives
// =============================================================================

/// Build a wire buffer containing `count` pre-encoded WS text frames.
fn build_wire_frames(payload: &str, count: u64) -> Vec<u8> {
    let data = payload.as_bytes();
    let frame_len = 2 + data.len(); // 1 byte header + 1 byte len + payload (≤125)
    let mut wire = Vec::with_capacity(frame_len * count as usize);
    for _ in 0..count {
        wire.push(0x81); // FIN + Text
        wire.push(data.len() as u8);
        wire.extend_from_slice(data);
    }
    wire
}

fn throughput_nexus_async_rt() -> (Duration, u64) {
    use nexus_async_rt::{DefaultRuntime, TcpListener, TcpStream, spawn};
    use nexus_net::ws;
    use nexus_rt::WorldBuilder;

    let wire = build_wire_frames(PAYLOAD, THROUGHPUT_MSGS);
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let mut world = WorldBuilder::new().build();
    let mut rt = DefaultRuntime::new(&mut world, 64);

    rt.block_on(async move {
        let io = nexus_async_rt::io();
        let mut listener = TcpListener::bind(addr, io).unwrap();
        let local = listener.local_addr().unwrap();

        // Server: accept WS, blast raw frames on the TCP stream.
        spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            tcp.set_nodelay(true).unwrap();
            let mut server = ws::Client::accept(tcp).await.unwrap();
            // Write raw frames directly — skip per-message encode.
            server.stream_mut().write_all(&wire).await.unwrap();
            // Keep alive until client is done.
            nexus_async_rt::sleep(Duration::from_secs(10)).await;
        });

        nexus_async_rt::sleep(Duration::from_millis(50)).await;

        let tcp = TcpStream::connect(local, io).unwrap();
        tcp.set_nodelay(true).unwrap();
        let url = format!("ws://127.0.0.1:{}/", local.port());
        let mut client = ws::Client::connect_with(tcp, &url).await.unwrap();

        let start = Instant::now();
        let mut count = 0u64;
        while count < THROUGHPUT_MSGS {
            match client.recv().await {
                Ok(Some(_)) => {
                    count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        let elapsed = start.elapsed();
        (elapsed, count)
    })
}

fn throughput_tokio_nexus_async_net() -> (Duration, u64) {
    use nexus_async_net::ws::WsStream;

    let wire = build_wire_frames(PAYLOAD, THROUGHPUT_MSGS);
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let local = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            tcp.set_nodelay(true).unwrap();
            let mut server = WsStream::accept(tcp).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(server.stream_mut(), &wire)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tcp = tokio::net::TcpStream::connect(local).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let url = format!("ws://127.0.0.1:{}/", local.port());
        let mut client = WsStream::connect_with(tcp, &url).await.unwrap();

        let start = Instant::now();
        let mut count = 0u64;
        while count < THROUGHPUT_MSGS {
            match client.recv().await {
                Ok(Some(_)) => {
                    count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        let elapsed = start.elapsed();
        (elapsed, count)
    })
}

fn throughput_tokio_tungstenite() -> (Duration, u64) {
    use futures_util::StreamExt;

    let wire = build_wire_frames(PAYLOAD, THROUGHPUT_MSGS);
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let local = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            tcp.set_nodelay(true).unwrap();
            let mut server = tokio_tungstenite::accept_async(tcp).await.unwrap();
            // Write raw frames on the underlying stream.
            let raw = server.get_mut();
            tokio::io::AsyncWriteExt::write_all(raw, &wire)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tcp = tokio::net::TcpStream::connect(local).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let (mut client, _) =
            tokio_tungstenite::client_async(format!("ws://127.0.0.1:{}/", local.port()), tcp)
                .await
                .unwrap();

        let start = Instant::now();
        let mut count = 0u64;
        while count < THROUGHPUT_MSGS {
            match client.next().await {
                Some(Ok(_)) => {
                    count += 1;
                }
                _ => break,
            }
        }
        let elapsed = start.elapsed();
        (elapsed, count)
    })
}

fn print_throughput(label: &str, elapsed: Duration, count: u64) {
    let secs = elapsed.as_secs_f64();
    let msg_per_sec = count as f64 / secs;
    println!(
        "{label}\n  {count} msgs in {:.3}s = {:.0} msg/sec\n",
        secs, msg_per_sec
    );
}

// =============================================================================
// In-memory parse: no TCP, no kernel — pure codec + async overhead
// =============================================================================

fn build_binary_wire(size: usize, count: u64) -> Vec<u8> {
    let payload = vec![0x42u8; size];
    let mut frame = Vec::new();
    frame.push(0x82); // FIN + Binary
    if size <= 125 {
        frame.push(size as u8);
    } else if size <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(size as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(size as u64).to_be_bytes());
    }
    frame.extend_from_slice(&payload);

    let mut wire = Vec::with_capacity(frame.len() * count as usize);
    for _ in 0..count {
        wire.extend_from_slice(&frame);
    }
    wire
}

/// Mock stream for our AsyncRead — always Ready, no IO.
mod mock_nexus {
    use nexus_async_rt::{AsyncRead, AsyncWrite};
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    pub struct MockReader<'a> {
        pub data: &'a [u8],
        pub pos: usize,
    }

    impl AsyncRead for MockReader<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            let remaining = &self.data[self.pos..];
            if remaining.is_empty() {
                return Poll::Ready(Ok(0));
            }
            let n = buf.len().min(remaining.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            Poll::Ready(Ok(n))
        }
    }

    impl AsyncWrite for MockReader<'_> {
        fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}

/// Mock stream for tokio's AsyncRead — always Ready, no IO.
mod mock_tokio {
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    pub struct MockReader<'a> {
        pub data: &'a [u8],
        pub pos: usize,
    }

    impl tokio::io::AsyncRead for MockReader<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let remaining = &self.data[self.pos..];
            let n = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..n]);
            self.pos += n;
            Poll::Ready(Ok(()))
        }
    }

    impl tokio::io::AsyncWrite for MockReader<'_> {
        fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}

fn inmemory_nexus_async_rt(wire: &[u8], msg_count: u64) -> (Duration, u64) {
    use nexus_net::ws;

    // Mock streams are always Ready — drive the async recv loop
    // inside a minimal block_on with a noop waker.
    let mock = mock_nexus::MockReader { data: wire, pos: 0 };
    let reader = ws::FrameReader::builder()
        .role(ws::Role::Client)
        .buffer_capacity(64 * 1024)
        .build();
    let mut client = ws::Client::from_parts(mock, reader, ws::FrameWriter::new(ws::Role::Client));

    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);

    // Drive the entire receive loop as one future.
    let fut = async {
        let start = Instant::now();
        let mut count = 0u64;
        while count < msg_count {
            match client.recv().await {
                Ok(Some(msg)) => {
                    black_box(&msg);
                    count += 1;
                }
                _ => break,
            }
        }
        (start.elapsed(), count)
    };
    let mut fut = std::pin::pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => panic!("mock future returned Pending"),
    }
}

fn inmemory_tokio_nexus_async_net(wire: &[u8], msg_count: u64) -> (Duration, u64) {
    use nexus_async_net::ws::WsStream;
    use nexus_net::ws::{FrameReader, FrameWriter, Role};

    let mock = mock_tokio::MockReader { data: wire, pos: 0 };
    let reader = FrameReader::builder()
        .role(Role::Client)
        .buffer_capacity(64 * 1024)
        .build();
    let mut ws = WsStream::from_parts(mock, reader, FrameWriter::new(Role::Client));

    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);

    let fut = async {
        let start = Instant::now();
        let mut count = 0u64;
        while count < msg_count {
            match ws.recv().await {
                Ok(Some(msg)) => {
                    black_box(&msg);
                    count += 1;
                }
                _ => break,
            }
        }
        (start.elapsed(), count)
    };
    let mut fut = std::pin::pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => panic!("mock future returned Pending"),
    }
}

fn inmemory_tokio_tungstenite(wire: &[u8], msg_count: u64) -> (Duration, u64) {
    use futures_util::StreamExt;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let mock = mock_tokio::MockReader { data: wire, pos: 0 };
        let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            mock,
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;

        let start = Instant::now();
        let mut count = 0u64;
        while count < msg_count {
            match ws.next().await {
                Some(Ok(msg)) => {
                    black_box(&msg);
                    count += 1;
                }
                _ => break,
            }
        }
        (start.elapsed(), count)
    })
}

fn noop_waker() -> std::task::Waker {
    use std::task::{RawWaker, RawWakerVTable};
    const VTABLE: RawWakerVTable =
        RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
    // SAFETY: no-op vtable, null data is never dereferenced.
    unsafe { std::task::Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

fn print_inmemory(label: &str, elapsed: Duration, count: u64, payload_size: usize) {
    let secs = elapsed.as_secs_f64();
    let ns_per_msg = elapsed.as_nanos() as f64 / count as f64;
    let msg_per_sec = count as f64 / secs;
    println!(
        "  {label:<30} {ns_per_msg:>5.0}ns ({:.1}M/s)",
        msg_per_sec / 1_000_000.0
    );
    let _ = payload_size; // used in the header
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    // =================================================================
    // 1. In-memory parse — isolates codec + async overhead, no kernel.
    //    This is the meaningful runtime comparison.
    // =================================================================

    println!();
    println!("=== In-Memory Parse ({INMEMORY_MSGS} msgs, binary frames) ===");
    println!("No TCP, no kernel — pure codec + async machinery.");
    println!();

    for &size in BINARY_SIZES {
        let wire = build_binary_wire(size, INMEMORY_MSGS);
        println!("  {size}B payload:");

        let (e1, c1) = inmemory_nexus_async_rt(&wire, INMEMORY_MSGS);
        print_inmemory("nexus-async-rt", e1, c1, size);

        let (e2, c2) = inmemory_tokio_nexus_async_net(&wire, INMEMORY_MSGS);
        print_inmemory("tokio + nexus-async-net", e2, c2, size);

        let (e3, c3) = inmemory_tokio_tungstenite(&wire, INMEMORY_MSGS);
        print_inmemory("tokio + tokio-tungstenite", e3, c3, size);

        println!();
    }

    // =================================================================
    // 2. TCP loopback — system-level validation.
    //    Kernel dominates (~95% of latency). These confirm the system
    //    works end-to-end but don't isolate runtime differences.
    // =================================================================

    println!("=== TCP Loopback Validation ===");
    println!("Kernel-dominated — confirms end-to-end correctness, not runtime speed.");
    println!();

    println!("--- Ping-Pong Latency ({ITERATIONS} iterations, {} bytes text) ---", PAYLOAD.len());
    println!();

    let h1 = bench_nexus_async_rt();
    print_histogram("nexus-async-rt + ws::Client", &h1);

    let h2 = bench_tokio_nexus_async_net();
    print_histogram("tokio + nexus-async-net", &h2);

    let h3 = bench_tokio_tungstenite();
    print_histogram("tokio + tokio-tungstenite", &h3);

    println!("--- Throughput ({THROUGHPUT_MSGS} msgs, {} bytes text, recv only) ---", PAYLOAD.len());
    println!();

    let (e1, c1) = throughput_nexus_async_rt();
    print_throughput("nexus-async-rt + ws::Client", e1, c1);

    let (e2, c2) = throughput_tokio_nexus_async_net();
    print_throughput("tokio + nexus-async-net", e2, c2);

    let (e3, c3) = throughput_tokio_tungstenite();
    print_throughput("tokio + tokio-tungstenite", e3, c3);
}
