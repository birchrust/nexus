//! Integration tests for the tokio compatibility layer.
//!
//! Requires `tokio-compat` feature:
//! `cargo test -p nexus-async-rt --test tokio_compat --features tokio-compat`

#![cfg(feature = "tokio-compat")]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use nexus_async_rt::tokio_compat::with_tokio;
use nexus_async_rt::{Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

// hdrhistogram for latency tests
#[cfg(feature = "tokio-compat")]
use hdrhistogram::Histogram;

// =============================================================================
// Basic: tokio::time::sleep works from our executor
// =============================================================================

#[test]
fn tokio_sleep() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        // tokio::time::sleep driven by tokio's timer, waker bridges back.
        with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(10))).await;
        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// TCP: tokio TcpStream from our executor
// =============================================================================

#[test]
fn tokio_tcp_echo() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        // Start a tokio TCP listener on a background thread.
        // (We use std::thread because tokio::spawn needs tokio's scheduler,
        // which we're only using for the reactor, not task scheduling.)
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn a task that accepts and echoes.
        spawn_boxed(async move {
            let (mut stream, _) = with_tokio(|| listener.accept()).await.unwrap();
            let mut buf = [0u8; 64];
            let n = with_tokio(|| tokio::io::AsyncReadExt::read(&mut stream, &mut buf))
                .await
                .unwrap();
            with_tokio(|| tokio::io::AsyncWriteExt::write_all(&mut stream, &buf[..n]))
                .await
                .unwrap();
        });

        // Connect and send from another spawned task.
        let mut client = with_tokio(|| tokio::net::TcpStream::connect(addr))
            .await
            .unwrap();
        with_tokio(|| tokio::io::AsyncWriteExt::write_all(&mut client, b"hello"))
            .await
            .unwrap();

        let mut buf = [0u8; 64];
        let n = with_tokio(|| tokio::io::AsyncReadExt::read(&mut client, &mut buf))
            .await
            .unwrap();
        assert_eq!(&buf[..n], b"hello");

        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// Timeout: tokio::time::timeout wrapping a tokio future
// =============================================================================

#[test]
fn tokio_timeout_success() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        let result = with_tokio(|| {
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                tokio::time::sleep(std::time::Duration::from_millis(10)),
            )
        })
        .await;
        assert!(result.is_ok()); // Completed before timeout.
        flag.set(true);
    });

    assert!(done.get());
}

#[test]
fn tokio_timeout_expires() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        let result = with_tokio(|| {
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                tokio::time::sleep(std::time::Duration::from_secs(10)),
            )
        })
        .await;
        assert!(result.is_err()); // Timed out.
        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// Multiple await points in a single with_tokio block
// =============================================================================

#[test]
fn tokio_multi_await_block() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: single with_tokio block, multiple awaits inside.
        spawn_boxed(async move {
            with_tokio(|| async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 64];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                    .await
                    .unwrap();
                tokio::io::AsyncWriteExt::write_all(&mut stream, &buf[..n])
                    .await
                    .unwrap();
            })
            .await;
        });

        // Client: single block with multiple awaits.
        let result = with_tokio(|| async {
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut client, b"multi-await")
                .await
                .unwrap();
            let mut buf = [0u8; 64];
            let n = tokio::io::AsyncReadExt::read(&mut client, &mut buf)
                .await
                .unwrap();
            String::from_utf8(buf[..n].to_vec()).unwrap()
        })
        .await;

        assert_eq!(result, "multi-await");
        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// Spawned task uses with_tokio
// =============================================================================

#[test]
fn spawned_task_with_tokio() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    let check = done.clone();

    rt.block_on(async move {
        spawn_boxed(async move {
            with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(10))).await;
            flag.set(true);
        });

        // Wait for the spawned task.
        for _ in 0..100 {
            nexus_async_rt::yield_now().await;
            if check.get() {
                return;
            }
            with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(1))).await;
        }
        panic!("spawned task did not complete");
    });

    assert!(done.get());
}

// =============================================================================
// Latency: tokio TCP loopback through our executor
// =============================================================================

fn print_histogram(name: &str, hist: &Histogram<u64>) {
    println!("\n=== {name} ({} samples) ===", hist.len());
    println!("  p50:    {:>8} ns", hist.value_at_quantile(0.50));
    println!("  p90:    {:>8} ns", hist.value_at_quantile(0.90));
    println!("  p99:    {:>8} ns", hist.value_at_quantile(0.99));
    println!("  p99.9:  {:>8} ns", hist.value_at_quantile(0.999));
    println!("  max:    {:>8} ns", hist.max());
    println!("  mean:   {:>8.1} ns", hist.mean());
}

#[test]
#[ignore]
fn tokio_compat_tcp_latency() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    const WARMUP: usize = 1_000;
    const ITERS: usize = 10_000;

    let hist_cell = Rc::new(Cell::new(None::<Histogram<u64>>));
    let hist_ref = hist_cell.clone();

    rt.block_on(async move {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Echo server in a spawned task.
        spawn_boxed(async move {
            with_tokio(|| async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 64];
                loop {
                    match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            tokio::io::AsyncWriteExt::write_all(&mut stream, &buf[..n])
                                .await
                                .unwrap();
                        }
                        Err(_) => break,
                    }
                }
            })
            .await;
        });

        let mut client = with_tokio(|| tokio::net::TcpStream::connect(addr))
            .await
            .unwrap();
        client.set_nodelay(true).unwrap();

        let msg = b"ping1234"; // 8 bytes
        let mut buf = [0u8; 8];

        // Warmup
        for _ in 0..WARMUP {
            with_tokio(|| tokio::io::AsyncWriteExt::write_all(&mut client, msg))
                .await
                .unwrap();
            with_tokio(|| tokio::io::AsyncReadExt::read_exact(&mut client, &mut buf))
                .await
                .unwrap();
        }

        // Measure
        let mut hist = Histogram::<u64>::new(3).unwrap();
        for _ in 0..ITERS {
            let start = Instant::now();
            with_tokio(|| tokio::io::AsyncWriteExt::write_all(&mut client, msg))
                .await
                .unwrap();
            with_tokio(|| tokio::io::AsyncReadExt::read_exact(&mut client, &mut buf))
                .await
                .unwrap();
            let elapsed = start.elapsed().as_nanos() as u64;
            hist.record(elapsed).unwrap();
        }

        print_histogram("tokio-compat TCP echo RTT", &hist);
        hist_ref.set(Some(hist));
    });

    assert!(hist_cell.take().is_some());
}

// =============================================================================
// Stress: many concurrent with_tokio tasks
// =============================================================================

#[test]
#[ignore]
fn tokio_compat_stress_concurrent_sleeps() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let count = Rc::new(Cell::new(0u32));
    let count_ref = count.clone();

    rt.block_on(async move {
        for _ in 0..100 {
            let c = count_ref.clone();
            spawn_boxed(async move {
                with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(1))).await;
                c.set(c.get() + 1);
            });
        }

        for _ in 0..500 {
            nexus_async_rt::yield_now().await;
            if count.get() >= 100 {
                return;
            }
            with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(1))).await;
        }
        panic!("only {}/100 tasks completed", count.get());
    });
}

#[test]
#[ignore]
fn tokio_compat_stress_rapid_tcp() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    rt.block_on(async {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        spawn_boxed(async move {
            with_tokio(|| async {
                loop {
                    match listener.accept().await {
                        Ok((mut stream, _)) => {
                            let mut buf = [0u8; 64];
                            match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                                Ok(n) if n > 0 => {
                                    let _ =
                                        tokio::io::AsyncWriteExt::write_all(&mut stream, &buf[..n])
                                            .await;
                                }
                                _ => {}
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .await;
        });

        for i in 0u32..100 {
            with_tokio(|| async {
                let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
                let msg = i.to_le_bytes();
                tokio::io::AsyncWriteExt::write_all(&mut client, &msg)
                    .await
                    .unwrap();
                let mut buf = [0u8; 4];
                tokio::io::AsyncReadExt::read_exact(&mut client, &mut buf)
                    .await
                    .unwrap();
                assert_eq!(buf, msg);
            })
            .await;
        }
    });
}

// =============================================================================
// Latency: pure waker bridge (no IO, no TCP)
// =============================================================================

/// Measures the pure waker bridge cost.
///
/// A background thread sends on a tokio oneshot channel, which fires
/// the waker immediately (no timer, no IO). The waker goes through
/// our cross-thread inbox → eventfd → our executor re-polls.
///
/// The background thread uses a sync barrier to coordinate — it sends
/// right after we start awaiting, so we measure the full round-trip:
/// register waker → Pending → sender fires waker → inbox push →
/// eventfd poke → our poll loop → re-poll → Ready.
#[test]
#[ignore]
fn tokio_compat_waker_bridge_latency() {
    use std::sync::{Arc, Barrier};

    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    const WARMUP: usize = 1_000;
    const ITERS: usize = 50_000;

    let hist_cell = Rc::new(Cell::new(None::<Histogram<u64>>));
    let hist_ref = hist_cell.clone();

    // Long-lived sender thread with per-iteration barrier.
    // Barrier ensures background thread has the oneshot sender
    // and is about to send BEFORE we start timing.
    let (coord_tx, coord_rx) =
        std::sync::mpsc::channel::<(tokio::sync::oneshot::Sender<()>, Arc<Barrier>)>();

    std::thread::spawn(move || {
        while let Ok((tx, barrier)) = coord_rx.recv() {
            barrier.wait(); // sync with receiver
            let _ = tx.send(()); // fire immediately
        }
    });

    // Use block_on_busy — never parks in epoll, drains inbox every iteration.
    rt.block_on_busy(async move {
        let mut hist_park = Histogram::<u64>::new(3).unwrap();

        for i in 0..(WARMUP + ITERS) {
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let barrier = Arc::new(Barrier::new(2));

            coord_tx.send((tx, barrier.clone())).unwrap();
            barrier.wait();

            let start = Instant::now();
            let _ = with_tokio(|| rx).await;
            let elapsed = start.elapsed().as_nanos() as u64;

            if i >= WARMUP {
                hist_park.record(elapsed).unwrap();
            }
        }

        print_histogram(
            "tokio-compat waker bridge (busy spin, no epoll)",
            &hist_park,
        );
        hist_ref.set(Some(hist_park));
    });

    assert!(hist_cell.take().is_some());
}

// =============================================================================
// Integration: bidirectional TCP conversation
// =============================================================================

#[test]
fn tokio_tcp_bidirectional_conversation() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: multi-round conversation.
        spawn_boxed(async move {
            with_tokio(|| async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 256];

                for round in 0u32..10 {
                    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                        .await
                        .unwrap();
                    let msg = std::str::from_utf8(&buf[..n]).unwrap();
                    assert_eq!(msg, format!("ping-{round}"));

                    let reply = format!("pong-{round}");
                    tokio::io::AsyncWriteExt::write_all(&mut stream, reply.as_bytes())
                        .await
                        .unwrap();
                }
            })
            .await;
        });

        // Client: send ping, receive pong, 10 rounds.
        with_tokio(|| async {
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();

            for round in 0u32..10 {
                let msg = format!("ping-{round}");
                tokio::io::AsyncWriteExt::write_all(&mut client, msg.as_bytes())
                    .await
                    .unwrap();

                let mut buf = [0u8; 256];
                let n = tokio::io::AsyncReadExt::read(&mut client, &mut buf)
                    .await
                    .unwrap();
                let reply = std::str::from_utf8(&buf[..n]).unwrap();
                assert_eq!(reply, format!("pong-{round}"));
            }
        })
        .await;

        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// Integration: concurrent TCP clients
// =============================================================================

#[test]
fn tokio_tcp_concurrent_clients() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let count = Rc::new(Cell::new(0u32));
    let count_ref = count.clone();

    rt.block_on(async move {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: accept multiple connections, echo each.
        spawn_boxed(async move {
            with_tokio(|| async {
                for _ in 0..5 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut buf = [0u8; 64];
                    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                        .await
                        .unwrap();
                    tokio::io::AsyncWriteExt::write_all(&mut stream, &buf[..n])
                        .await
                        .unwrap();
                }
            })
            .await;
        });

        // 5 concurrent client tasks.
        for i in 0u32..5 {
            let c = count_ref.clone();
            spawn_boxed(async move {
                with_tokio(|| async {
                    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
                    let msg = i.to_le_bytes();
                    tokio::io::AsyncWriteExt::write_all(&mut client, &msg)
                        .await
                        .unwrap();
                    let mut buf = [0u8; 4];
                    tokio::io::AsyncReadExt::read_exact(&mut client, &mut buf)
                        .await
                        .unwrap();
                    assert_eq!(buf, msg);
                })
                .await;
                c.set(c.get() + 1);
            });
        }

        // Wait for all clients.
        for _ in 0..200 {
            if count.get() >= 5 {
                return;
            }
            nexus_async_rt::yield_now().await;
        }
        panic!("only {}/5 clients completed", count.get());
    });
}

// =============================================================================
// Integration: tokio timeout on slow server
// =============================================================================

#[test]
fn tokio_timeout_on_slow_server() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: accept but never respond (simulate slow/dead server).
        spawn_boxed(async move {
            with_tokio(|| async {
                let (_stream, _) = listener.accept().await.unwrap();
                // Hold connection open, never write.
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            })
            .await;
        });

        // Client: connect with timeout. Should timeout, not hang.
        let result = with_tokio(|| async {
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            let mut buf = [0u8; 64];
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                tokio::io::AsyncReadExt::read(&mut client, &mut buf),
            )
            .await
        })
        .await;

        assert!(result.is_err()); // Elapsed — timeout fired.
        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// Integration: mixed nexus IO + tokio futures
// =============================================================================

#[test]
fn mixed_nexus_and_tokio() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        // Use our local channel (nexus primitive) alongside tokio sleep.
        let (tx, rx) = nexus_async_rt::channel::local::channel::<u32>(16);

        spawn_boxed(async move {
            for i in 0..5 {
                // Tokio timer between sends.
                with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(5))).await;
                tx.send(i).await.unwrap();
            }
        });

        let mut received = Vec::new();
        for _ in 0..5 {
            let val = rx.recv().await.unwrap();
            received.push(val);
        }
        assert_eq!(received, vec![0, 1, 2, 3, 4]);
        flag.set(true);
    });

    assert!(done.get());
}

// =============================================================================
// Fuzz: rapid with_tokio creation/drop
// =============================================================================

#[test]
#[ignore]
fn fuzz_rapid_with_tokio() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    rt.block_on(async {
        // Rapidly create and await with_tokio futures.
        // Tests that the leaked EnterGuard + cross-thread waker
        // creation/drop don't leak or corrupt state.
        for _ in 0..10_000 {
            with_tokio(|| tokio::time::sleep(std::time::Duration::ZERO)).await;
        }
    });
}

#[test]
#[ignore]
fn fuzz_concurrent_tasks_with_tokio() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    let count = Rc::new(Cell::new(0u32));
    let count_ref = count.clone();

    rt.block_on(async move {
        // Spawn 50 tasks, each doing 100 tokio sleeps.
        for _ in 0..50 {
            let c = count_ref.clone();
            spawn_boxed(async move {
                for _ in 0..100 {
                    with_tokio(|| tokio::time::sleep(std::time::Duration::ZERO)).await;
                }
                c.set(c.get() + 1);
            });
        }

        // Wait for all.
        loop {
            if count.get() >= 50 {
                return;
            }
            with_tokio(|| tokio::time::sleep(std::time::Duration::from_millis(1))).await;
        }
    });
}

#[test]
#[ignore]
fn fuzz_tcp_connect_storm() {
    // Many rapid TCP connections through the bridge.
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::new(&mut world);

    rt.block_on(async {
        let listener = with_tokio(|| tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: accept everything, echo, close.
        spawn_boxed(async move {
            with_tokio(|| async {
                loop {
                    match listener.accept().await {
                        Ok((mut stream, _)) => {
                            let mut buf = [0u8; 8];
                            match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                                Ok(n) if n > 0 => {
                                    let _ =
                                        tokio::io::AsyncWriteExt::write_all(&mut stream, &buf[..n])
                                            .await;
                                }
                                _ => {}
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .await;
        });

        // Rapid connect/send/recv/close — 200 connections.
        for i in 0u32..200 {
            with_tokio(|| async {
                let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
                let msg = i.to_le_bytes();
                tokio::io::AsyncWriteExt::write_all(&mut client, &msg)
                    .await
                    .unwrap();
                let mut buf = [0u8; 4];
                tokio::io::AsyncReadExt::read_exact(&mut client, &mut buf)
                    .await
                    .unwrap();
                assert_eq!(buf, msg);
            })
            .await;
        }
    });
}
