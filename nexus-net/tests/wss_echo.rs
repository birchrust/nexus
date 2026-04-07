//! Integration test: WebSocket over TLS against public echo servers.
//!
//! Verifies the full stack: TLS handshake → HTTP upgrade → WS framing.
//! Requires network access. Skipped in normal `cargo test` and CI.
//!
//! **Run after any changes to:** `tls/`, `ws/stream.rs`,
//! `ws/handshake.rs`, `http/`, or buffer primitives.
//!
//! ```bash
//! cargo test -p nexus-net --features tls --test wss_echo -- --ignored --nocapture
//! ```

#![cfg(feature = "tls")]

use nexus_net::tls::TlsConfig;
use nexus_net::ws::{Client, CloseCode, Message};
use std::net::TcpStream;

fn connect_echo(host: &str, port: u16, url: &str) -> Client<TcpStream> {
    let tls_config = TlsConfig::new().expect("TLS config with system certs");
    let tcp = TcpStream::connect((host, port)).expect("TCP connect");
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    nexus_net::ws::ClientBuilder::new()
        .tls(&tls_config)
        .write_buffer_capacity(64 * 1024)
        .connect_with(tcp, url)
        .expect("WSS connect + upgrade")
}

#[test]
#[ignore = "requires network access to ws.postman-echo.com"]
fn postman_echo_text() {
    let mut ws = connect_echo("ws.postman-echo.com", 443, "wss://ws.postman-echo.com/raw");

    // Text echo
    ws.send_text("Hello from nexus-net!").unwrap();
    match ws.recv().unwrap().unwrap() {
        Message::Text(s) => assert_eq!(s, "Hello from nexus-net!"),
        other => panic!("expected Text echo, got {other:?}"),
    }

    // Multiple messages in sequence (FIFO over TLS)
    for i in 0..10 {
        let msg = format!("message {i}");
        ws.send_text(&msg).unwrap();
        match ws.recv().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, msg),
            other => panic!("expected Text echo #{i}, got {other:?}"),
        }
    }

    // Larger payload
    let big = "x".repeat(4096);
    ws.send_text(&big).unwrap();
    match ws.recv().unwrap().unwrap() {
        Message::Text(s) => assert_eq!(s.len(), 4096),
        other => panic!("expected Text echo, got {other:?}"),
    }

    // Clean close
    ws.close(CloseCode::Normal, "done").unwrap();
    println!("ws.postman-echo.com: PASS (text echo, FIFO x10, 4KB payload)");
}
