// SPDX-License-Identifier: GPL-2.0-or-later
//! The web listener does not let a client hold a connection by never finishing its request:
//! about 1,100 such connections once used every file descriptor of the daemon, and its next
//! polling cycle could open no socket and write no log (2026-10-21).

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use linkscope_web::{HEADER_TIMEOUT, LiveState, serve_listener};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn a_request_whose_headers_never_end_is_closed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve_listener(
        listener,
        Arc::new(RwLock::new(LiveState::default())),
    ));

    // A request that answers at once still works.
    let mut ok = tokio::net::TcpStream::connect(addr).await.unwrap();
    ok.write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    ok.read_to_string(&mut reply).await.unwrap();
    assert!(
        reply.starts_with("HTTP/1.1 200") && reply.ends_with("ok"),
        "{reply}"
    );

    // Headers started and never finished: the server hangs up after HEADER_TIMEOUT.
    let mut slow = tokio::net::TcpStream::connect(addr).await.unwrap();
    slow.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .await
        .unwrap();
    let started = Instant::now();
    let mut buf = [0u8; 1024];
    let read =
        tokio::time::timeout(HEADER_TIMEOUT + Duration::from_secs(5), slow.read(&mut buf)).await;
    let waited = started.elapsed();
    assert!(read.is_ok(), "still open after {waited:?}");
    assert!(
        waited >= HEADER_TIMEOUT - Duration::from_secs(1),
        "closed too early: {waited:?}"
    );
}
