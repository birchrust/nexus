//! Autobahn WebSocket conformance test.
//!
//! Runs against Autobahn's `fuzzingserver` via Docker/Podman.
//!
//! Prerequisites:
//!   docker pull crossbario/autobahn-testsuite
//!
//! Run:
//!   cargo test -p nexus-net --test autobahn -- --ignored --nocapture

use std::net::TcpStream;
use nexus_net::ws::{
    CloseCode, Message, OwnedMessage, ProtocolError, WsError, WsStream,
};

const AUTOBAHN_HOST: &str = "127.0.0.1:9001";
const AGENT: &str = "nexus-net";

fn make_ws(path: &str) -> WsStream<TcpStream> {
    let tcp = TcpStream::connect(AUTOBAHN_HOST).expect("connect failed");
    let url = format!("ws://{AUTOBAHN_HOST}{path}");
    nexus_net::ws::WsStreamBuilder::new()
        .buffer_capacity(16 * 1024 * 1024 + 4096) // 16MB + header room
        .max_frame_size(16 * 1024 * 1024)
        .max_message_size(16 * 1024 * 1024)
        .write_buffer_capacity(16 * 1024 * 1024 + 4096) // match read capacity for echo
        .connect(tcp, &url)
        .expect("handshake failed")
}

#[test]
#[ignore]
fn autobahn_conformance() {
    let case_count = get_case_count();
    println!("Autobahn: {case_count} test cases");

    for case in 1..=case_count {
        print!("  Case {case}/{case_count}...");
        run_case(case);
        println!(" ok");
    }

    update_reports();
    println!("Autobahn: reports generated. Check target/autobahn-reports/");
}

fn get_case_count() -> u32 {
    let mut ws = make_ws("/getCaseCount");
    match ws.next().expect("read failed").expect("no message") {
        Message::Text(s) => s.parse().expect("invalid case count"),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn run_case(case: u32) {
    let path = format!("/runCase?case={case}&agent={AGENT}");
    let mut ws = make_ws(&path);

    loop {
        let msg = match ws.next() {
            Ok(Some(msg)) => msg.into_owned(),
            Ok(None) => break,
            Err(WsError::Protocol(ProtocolError::InvalidUtf8)) => {
                let _ = ws.close(CloseCode::InvalidPayload, "invalid UTF-8");
                break;
            }
            Err(WsError::Protocol(_)) => {
                let _ = ws.close(CloseCode::Protocol, "protocol error");
                break;
            }
            Err(_) => break,
        };

        match msg {
            OwnedMessage::Text(s) => {
                if ws.send_text(&s).is_err() { break; }
            }
            OwnedMessage::Binary(b) => {
                if ws.send_binary(&b).is_err() { break; }
            }
            OwnedMessage::Ping(p) => {
                if ws.send_pong(&p).is_err() { break; }
            }
            OwnedMessage::Close(_) => {
                let _ = ws.close(CloseCode::Normal, "");
                break;
            }
            _ => {}
        }
    }
}

fn update_reports() {
    let path = format!("/updateReports?agent={AGENT}");
    let mut ws = make_ws(&path);
    let _ = ws.next();
}
