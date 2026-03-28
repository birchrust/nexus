//! Autobahn WebSocket conformance test.
//!
//! Runs against Autobahn's `fuzzingserver` via Docker.
//!
//! Prerequisites:
//!   docker pull crossbario/autobahn-testsuite
//!
//! Run:
//!   cargo test -p nexus-net --test autobahn -- --ignored --nocapture

use std::net::TcpStream;
use nexus_net::ws::{Message, OwnedMessage, WsStream};

const AUTOBAHN_HOST: &str = "127.0.0.1:9001";
const AGENT: &str = "nexus-net";

/// Connect to the Autobahn fuzzingserver and run all test cases.
///
/// The server must be started separately:
/// ```bash
/// docker run -it --rm \
///     -v "${PWD}/tests/autobahn:/config" \
///     -v "${PWD}/target/autobahn-reports:/reports" \
///     -p 9001:9001 \
///     crossbario/autobahn-testsuite \
///     wstest -m fuzzingserver -s /config/fuzzingserver.json
/// ```
#[test]
#[ignore]
fn autobahn_conformance() {
    // Get case count
    let case_count = get_case_count();
    println!("Autobahn: {case_count} test cases");

    // Run each case
    for case in 1..=case_count {
        print!("  Case {case}/{case_count}...");
        run_case(case);
        println!(" ok");
    }

    // Request update report
    update_reports();
    println!("Autobahn: reports generated. Check target/autobahn-reports/");
}

fn get_case_count() -> u32 {
    let tcp = TcpStream::connect(AUTOBAHN_HOST)
        .expect("failed to connect to Autobahn server — is it running?");
    let mut ws = WsStream::connect(tcp, AUTOBAHN_HOST, "/getCaseCount")
        .expect("handshake failed");

    match ws.next().expect("read failed").expect("no message") {
        Message::Text(s) => s.parse().expect("invalid case count"),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn run_case(case: u32) {
    let path = format!("/runCase?case={case}&agent={AGENT}");
    let tcp = TcpStream::connect(AUTOBAHN_HOST).expect("connect failed");
    let mut ws = WsStream::connect(tcp, AUTOBAHN_HOST, &path).expect("handshake failed");

    // Echo loop: echo back text/binary, respond to pings, handle close
    loop {
        let msg = match ws.next() {
            Ok(Some(msg)) => msg.into_owned(),
            Ok(None) => break,
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
                let _ = ws.close(nexus_net::ws::CloseCode::Normal, "");
                break;
            }
            _ => {}
        }
    }
}

fn update_reports() {
    let path = format!("/updateReports?agent={AGENT}");
    let tcp = TcpStream::connect(AUTOBAHN_HOST).expect("connect failed");
    let mut ws = WsStream::connect(tcp, AUTOBAHN_HOST, &path).expect("handshake failed");
    // Server closes after generating reports
    let _ = ws.next();
}
