//! TCP integration tests.
//!
//! Every test creates its own Runtime + World. Tests bind to 127.0.0.1:0
//! (OS-assigned port) to avoid port conflicts.

use std::cell::Cell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use nexus_async_rt::{
    DefaultRuntime, IoHandle, TcpListener, TcpSocket, TcpStream, spawn,
};
use nexus_rt::WorldBuilder;

/// Helper: bind a listener and return (listener, addr).
fn bind_listener(io: IoHandle) -> (TcpListener, SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0".parse().unwrap(), io).unwrap();
    let a = l.local_addr().unwrap();
    (l, a)
}

// =============================================================================
// Basic connectivity
// =============================================================================

#[test]
fn tcp_echo_basic() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        // Server: accept, read, echo, close.
        spawn(async move {
            let mut listener = listener;
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 128];
            let n = s.read(&mut buf).await.unwrap();
            s.write_all(&buf[..n]).await.unwrap();
        });

        // Client: connect, write, read echo.
        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = TcpStream::connect(addr, io).unwrap();
            c.write_all(b"hello world").await.unwrap();
            let mut buf = [0u8; 128];
            let n = c.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"hello world");
            flag.set(true);
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

#[test]
fn tcp_multiple_clients() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 32);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let count = Rc::new(Cell::new(0u32));
    let count2 = count.clone();

    rt.block_on(async move {
        // Server: accept 3 connections, echo each.
        spawn(async move {
            let mut listener = listener;
            for _ in 0..3 {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 64];
                let n = s.read(&mut buf).await.unwrap();
                s.write_all(&buf[..n]).await.unwrap();
                count2.set(count2.get() + 1);
            }
        });

        // 3 clients, sequential (single-threaded — can't truly parallel).
        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            for i in 0..3u8 {
                let mut c = TcpStream::connect(addr, io).unwrap();
                let msg = [b'A' + i; 4];
                c.write_all(&msg).await.unwrap();
                let mut buf = [0u8; 64];
                let n = c.read(&mut buf).await.unwrap();
                assert_eq!(&buf[..n], &msg);
            }
        });

        handle.sleep(Duration::from_millis(1000)).await;
    });

    assert_eq!(count.get(), 3);
}

// =============================================================================
// Large transfer
// =============================================================================

#[test]
fn tcp_large_transfer() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    // 1MB of data.
    let data: Vec<u8> = (0..1_000_000).map(|i| (i % 251) as u8).collect();
    let expected = data.clone();

    rt.block_on(async move {
        // Server: read all, verify.
        let exp = expected;
        spawn(async move {
            let mut listener = listener;
            let (mut s, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = s.read(&mut buf).await.unwrap();
                if n == 0 {
                    break; // EOF
                }
                received.extend_from_slice(&buf[..n]);
            }
            assert_eq!(received.len(), exp.len());
            assert_eq!(received, exp);
            flag.set(true);
        });

        // Client: write all, close.
        let io = handle.io();
        let send_data = data;
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = TcpStream::connect(addr, io).unwrap();
            c.write_all(&send_data).await.unwrap();
            // Drop the stream to send FIN → server sees EOF.
        });

        handle.sleep(Duration::from_millis(2000)).await;
    });

    assert!(done.get(), "large transfer did not complete");
}

// =============================================================================
// Split
// =============================================================================

#[test]
fn tcp_split_borrowed() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut listener = listener;
            let (mut s, _) = listener.accept().await.unwrap();
            // Use borrowed split: read then write within same task.
            let (mut rd, mut wr) = s.split();
            let mut buf = [0u8; 64];
            use nexus_async_rt::AsyncRead;
            use nexus_async_rt::AsyncWrite;
            let n = std::future::poll_fn(|cx| {
                std::pin::Pin::new(&mut rd).poll_read(cx, &mut buf)
            })
            .await
            .unwrap();
            std::future::poll_fn(|cx| {
                std::pin::Pin::new(&mut wr).poll_write(cx, &buf[..n])
            })
            .await
            .unwrap();
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = TcpStream::connect(addr, io).unwrap();
            c.write_all(b"split").await.unwrap();
            let mut buf = [0u8; 64];
            let n = c.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"split");
            flag.set(true);
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

#[test]
fn tcp_into_split_reunite() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());

    rt.block_on(async move {
        spawn(async move {
            let mut listener = listener;
            let (s, _) = listener.accept().await.unwrap();
            let (read_half, write_half) = s.into_split();
            // Reunite should succeed — same stream.
            let _stream = read_half.reunite(write_half).unwrap();
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let _c = TcpStream::connect(addr, io).unwrap();
        });

        handle.sleep(Duration::from_millis(500)).await;
    });
}

// =============================================================================
// Socket options
// =============================================================================

#[test]
fn tcp_socket_options_on_stream() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut listener = listener;
            let (s, _) = listener.accept().await.unwrap();

            // Nodelay
            s.set_nodelay(true).unwrap();
            assert!(s.nodelay().unwrap());
            s.set_nodelay(false).unwrap();
            assert!(!s.nodelay().unwrap());

            // Keepalive
            s.set_keepalive(true).unwrap();
            assert!(s.keepalive().unwrap());

            // TTL
            s.set_ttl(64).unwrap();
            assert_eq!(s.ttl().unwrap(), 64);

            // Buffer sizes (kernel may round up)
            s.set_send_buffer_size(32768).unwrap();
            assert!(s.send_buffer_size().unwrap() >= 32768);
            s.set_recv_buffer_size(32768).unwrap();
            assert!(s.recv_buffer_size().unwrap() >= 32768);

            // Linger
            s.set_linger(Some(Duration::from_secs(5))).unwrap();
            let linger = s.linger().unwrap();
            assert!(linger.is_some());

            // take_error should return None (no error)
            assert!(s.take_error().unwrap().is_none());

            flag.set(true);
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let _c = TcpStream::connect(addr, io).unwrap();
            handle.sleep(Duration::from_millis(100)).await;
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

#[test]
fn tcp_socket_builder_bind_listen() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let socket = TcpSocket::new_v4().unwrap();
    socket.set_reuseaddr(true).unwrap();
    assert!(socket.reuseaddr().unwrap());
    socket.set_nodelay(true).unwrap();
    assert!(socket.nodelay().unwrap());
    socket.set_send_buffer_size(65536).unwrap();
    assert!(socket.send_buffer_size().unwrap() >= 65536);

    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let mut listener = socket.listen(128, handle.io()).unwrap();
    let addr = listener.local_addr().unwrap();

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 16];
            let n = s.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"via-socket");
            flag.set(true);
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = TcpStream::connect(addr, io).unwrap();
            c.write_all(b"via-socket").await.unwrap();
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

// =============================================================================
// try_read / try_write
// =============================================================================

#[test]
fn tcp_try_read_write() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut listener = listener;
            let (s, _) = listener.accept().await.unwrap();
            // try_write should work on a connected stream.
            match s.try_write(b"data") {
                Ok(n) => assert!(n > 0),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Also acceptable — buffer might not be ready yet.
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
            flag.set(true);
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let _c = TcpStream::connect(addr, io).unwrap();
            handle.sleep(Duration::from_millis(100)).await;
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

// =============================================================================
// from_std / into_std
// =============================================================================

#[test]
fn tcp_from_std() {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr = std_listener.local_addr().unwrap();

    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let mut listener =
        TcpListener::from_std(std_listener, handle.io()).unwrap();
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 16];
            let n = s.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"from_std");
            flag.set(true);
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = TcpStream::connect(addr, io).unwrap();
            c.write_all(b"from_std").await.unwrap();
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

// =============================================================================
// into_std
// =============================================================================

#[test]
fn tcp_into_std() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut listener = listener;
            let (s, _) = listener.accept().await.unwrap();
            // Convert to std and verify it's valid.
            let std_stream = s.into_std().unwrap();
            assert!(std_stream.peer_addr().is_ok());
            flag.set(true);
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let _c = TcpStream::connect(addr, io).unwrap();
            handle.sleep(Duration::from_millis(100)).await;
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

// =============================================================================
// Error paths
// =============================================================================

#[test]
fn tcp_connect_refused() {
    // Connect to a port that's definitely not listening.
    // Bind a listener then drop it immediately to get a known-closed port.
    let tmp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed_addr = tmp_listener.local_addr().unwrap();
    drop(tmp_listener); // port is now closed

    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let io = handle.io();
            // Non-blocking connect may succeed (EINPROGRESS) or fail immediately.
            let connect_result = TcpStream::connect(closed_addr, io);
            match connect_result {
                Err(_) => {
                    // Connect failed immediately — that's valid.
                    flag.set(true);
                }
                Ok(mut c) => {
                    // Connect returned Ok (in progress). Wait for it to
                    // settle, then the first read/write detects failure.
                    handle.sleep(Duration::from_millis(50)).await;
                    let result = c.write(b"test").await;
                    assert!(result.is_err(), "expected connection refused");
                    flag.set(true);
                }
            }
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

#[test]
fn tcp_read_after_peer_close() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (listener, addr) = bind_listener(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        // Server: accept, immediately close.
        spawn(async move {
            let mut listener = listener;
            let (_s, _) = listener.accept().await.unwrap();
            // s dropped — sends FIN to client.
        });

        let io = handle.io();
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = TcpStream::connect(addr, io).unwrap();
            handle.sleep(Duration::from_millis(50)).await;
            // Read should return 0 (EOF) since server closed.
            let mut buf = [0u8; 64];
            let n = c.read(&mut buf).await.unwrap();
            assert_eq!(n, 0, "expected EOF");
            flag.set(true);
        });

        handle.sleep(Duration::from_millis(500)).await;
    });

    assert!(done.get());
}

// =============================================================================
// Listener TTL
// =============================================================================

#[test]
fn tcp_listener_ttl() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 4);
    let handle = rt.handle();

    let (listener, _addr) = bind_listener(handle.io());
    listener.set_ttl(42).unwrap();
    assert_eq!(listener.ttl().unwrap(), 42);
}
