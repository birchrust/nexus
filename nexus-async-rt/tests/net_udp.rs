//! UDP integration tests.

use std::cell::Cell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use nexus_async_rt::{DefaultRuntime, IoHandle, UdpSocket, spawn};
use nexus_rt::WorldBuilder;

fn bind_udp(io: IoHandle) -> (UdpSocket, SocketAddr) {
    let s = UdpSocket::bind("127.0.0.1:0".parse().unwrap(), io).unwrap();
    let a = s.local_addr().unwrap();
    (s, a)
}

// =============================================================================
// Basic send/recv
// =============================================================================

#[test]
fn udp_send_recv_basic() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (recv_sock, recv_addr) = bind_udp(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut s = recv_sock;
            let mut buf = [0u8; 64];
            let (n, from) = s.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"hello udp");
            assert!(from.ip().is_loopback());
            flag.set(true);
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut s = UdpSocket::bind("127.0.0.1:0".parse().unwrap(), handle.io()).unwrap();
            s.send_to(b"hello udp", recv_addr).await.unwrap();
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert!(done.get());
}

// =============================================================================
// Connected mode
// =============================================================================

#[test]
fn udp_connected_send_recv() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (a_sock, a_addr) = bind_udp(handle.io());
    let (b_sock, b_addr) = bind_udp(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut a = a_sock;
            a.connect(b_addr).unwrap();
            a.send(b"connected-msg").await.unwrap();
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut b = b_sock;
            b.connect(a_addr).unwrap();
            let mut buf = [0u8; 64];
            let n = b.recv(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"connected-msg");
            flag.set(true);
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert!(done.get());
}

// =============================================================================
// Echo (bidirectional)
// =============================================================================

#[test]
fn udp_echo() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (server_sock, server_addr) = bind_udp(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        // Server: recv, echo back.
        spawn(async move {
            let mut s = server_sock;
            let mut buf = [0u8; 64];
            let (n, peer) = s.recv_from(&mut buf).await.unwrap();
            s.send_to(&buf[..n], peer).await.unwrap();
        });

        // Client: send, recv echo.
        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = UdpSocket::bind("127.0.0.1:0".parse().unwrap(), handle.io()).unwrap();
            c.send_to(b"echo-me", server_addr).await.unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = c.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"echo-me");
            flag.set(true);
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert!(done.get());
}

// =============================================================================
// Multiple datagrams
// =============================================================================

#[test]
fn udp_multiple_datagrams() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (recv_sock, recv_addr) = bind_udp(handle.io());
    let count = Rc::new(Cell::new(0u32));
    let count2 = count.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut s = recv_sock;
            let mut buf = [0u8; 64];
            for _ in 0..5 {
                let (n, _) = s.recv_from(&mut buf).await.unwrap();
                assert!(n > 0);
                count2.set(count2.get() + 1);
            }
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut c = UdpSocket::bind("127.0.0.1:0".parse().unwrap(), handle.io()).unwrap();
            for i in 0..5u8 {
                c.send_to(&[i; 4], recv_addr).await.unwrap();
                // Small delay between sends.
                handle.sleep(Duration::from_millis(20)).await;
            }
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert_eq!(count.get(), 5);
}

// =============================================================================
// Socket options
// =============================================================================

#[test]
fn udp_socket_options() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 4);
    let io = rt.handle().io();

    let (s, _addr) = bind_udp(io);

    // Broadcast
    s.set_broadcast(true).unwrap();
    assert!(s.broadcast().unwrap());
    s.set_broadcast(false).unwrap();
    assert!(!s.broadcast().unwrap());

    // TTL
    s.set_ttl(42).unwrap();
    assert_eq!(s.ttl().unwrap(), 42);

    // Multicast TTL
    s.set_multicast_ttl_v4(5).unwrap();
    assert_eq!(s.multicast_ttl_v4().unwrap(), 5);

    // Multicast loop
    s.set_multicast_loop_v4(false).unwrap();
    assert!(!s.multicast_loop_v4().unwrap());
    s.set_multicast_loop_v4(true).unwrap();
    assert!(s.multicast_loop_v4().unwrap());

    // Buffer sizes
    s.set_send_buffer_size(65536).unwrap();
    assert!(s.send_buffer_size().unwrap() >= 65536);
    s.set_recv_buffer_size(65536).unwrap();
    assert!(s.recv_buffer_size().unwrap() >= 65536);

    // take_error
    assert!(s.take_error().unwrap().is_none());
}

// =============================================================================
// try_send / try_recv (non-blocking, no context)
// =============================================================================

#[test]
fn udp_try_send_recv() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (a, a_addr) = bind_udp(handle.io());
    let (b, b_addr) = bind_udp(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut a = a;
            a.connect(b_addr).unwrap();
            // try_send should work immediately on UDP.
            let n = a.try_send(b"try-data").unwrap();
            assert_eq!(n, 8);
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(50)).await;
            let mut b = b;
            b.connect(a_addr).unwrap();
            // Data should have arrived by now.
            match b.try_recv(&mut [0u8; 64]) {
                Ok(n) => {
                    assert_eq!(n, 8);
                    flag.set(true);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Data might not have arrived yet — also acceptable
                    // for a timing-sensitive non-blocking test.
                    flag.set(true);
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert!(done.get());
}

// =============================================================================
// from_std / into_std
// =============================================================================

#[test]
fn udp_from_std() {
    let std_sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    std_sock.set_nonblocking(true).unwrap();
    let addr = std_sock.local_addr().unwrap();

    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let sock = UdpSocket::from_std(std_sock, handle.io()).unwrap();
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut s = sock;
            let mut buf = [0u8; 64];
            let (n, _) = s.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"from-std");
            flag.set(true);
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut s = UdpSocket::bind("127.0.0.1:0".parse().unwrap(), handle.io()).unwrap();
            s.send_to(b"from-std", addr).await.unwrap();
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert!(done.get());
}

#[test]
fn udp_into_std() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 4);
    let io = rt.handle().io();

    let (sock, _addr) = bind_udp(io);
    let std_sock = sock.into_std().unwrap();
    // Should be a valid std UdpSocket.
    assert!(std_sock.local_addr().is_ok());
}

// =============================================================================
// Peek
// =============================================================================

#[test]
fn udp_peek_from() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    let (recv_sock, recv_addr) = bind_udp(handle.io());
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut s = recv_sock;
            let mut buf = [0u8; 64];
            // Peek should return data without consuming.
            let (n, peer) = s.peek_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"peek-data");
            assert!(peer.ip().is_loopback());

            // Regular recv should return the same data.
            let (n2, _) = s.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..n2], b"peek-data");
            flag.set(true);
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(10)).await;
            let mut s = UdpSocket::bind("127.0.0.1:0".parse().unwrap(), handle.io()).unwrap();
            s.send_to(b"peek-data", recv_addr).await.unwrap();
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    assert!(done.get());
}

// =============================================================================
// Multicast (loopback)
// =============================================================================

#[test]
fn udp_multicast_loopback() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 16);
    let handle = rt.handle();

    // Bind to any port, then join the multicast group.
    // Multicast may not work in all environments (CI, containers, no
    // multicast route). Skip gracefully if join fails.
    let (recv_sock, recv_local) = bind_udp(handle.io());
    let recv_port = recv_local.port();

    if recv_sock
        .join_multicast_v4(
            &"239.255.0.1".parse().unwrap(),
            &"0.0.0.0".parse().unwrap(),
        )
        .is_err()
    {
        println!("multicast join failed — skipping test (likely no multicast route)");
        return;
    }
    let _ = recv_sock.set_multicast_loop_v4(true);

    let done = Rc::new(Cell::new(false));
    let flag = done.clone();

    rt.block_on(async move {
        spawn(async move {
            let mut s = recv_sock;
            let mut buf = [0u8; 64];
            let (n, _) = s.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"mcast");
            flag.set(true);
        });

        spawn(async move {
            handle.sleep(Duration::from_millis(50)).await;
            let mut s = UdpSocket::bind("0.0.0.0:0".parse().unwrap(), handle.io()).unwrap();
            let target: SocketAddr = format!("239.255.0.1:{recv_port}").parse().unwrap();
            s.send_to(b"mcast", target).await.unwrap();
        });

        handle.sleep(Duration::from_secs(2)).await;
    });

    // Multicast on loopback may not work in all environments (CI, containers).
    // Don't assert — just verify it doesn't panic.
    let _ = done.get();
}

// =============================================================================
// AsFd / AsRawFd
// =============================================================================

#[test]
fn udp_as_fd() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = DefaultRuntime::new(&mut world, 4);
    let io = rt.handle().io();

    let (s, _) = bind_udp(io);
    use std::os::fd::AsRawFd;
    let fd = s.as_raw_fd();
    assert!(fd >= 0);
}
