//! A pooled connection the server closed after its first answer is retried once on a fresh
//! connection for an idempotent method, and never for a method that could repeat a side effect.

use crate::common::{executor, resolved_one};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ANSWER: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok";

struct Counts {
    connections: AtomicUsize,
    /// Requests that reached the first connection after its answer, which it drops unanswered.
    stale_attempts: AtomicUsize,
}

/// Connection 1 answers one request, then closes on the next one without replying; every later
/// connection answers each request it gets.
async fn server() -> (u16, Arc<Counts>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let counts = Arc::new(Counts {
        connections: AtomicUsize::new(0),
        stale_attempts: AtomicUsize::new(0),
    });
    let seen = Arc::clone(&counts);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let n = seen.connections.fetch_add(1, Ordering::SeqCst) + 1;
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let mut answered = 0;
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {}
                    }
                    if n == 1 && answered == 1 {
                        seen.stale_attempts.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                    answered += 1;
                    if sock.write_all(ANSWER).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, counts)
}

#[tokio::test]
async fn a_get_on_a_dead_pooled_connection_is_retried_on_a_fresh_one() {
    let (port, counts) = server().await;
    let r = resolved_one("GET", &format!("http://127.0.0.1:{port}/"), "", "");
    let exec = executor();
    exec.send(&r).await.expect("first send");
    exec.send(&r).await.expect("second send retried");
    assert_eq!(counts.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_post_on_a_dead_pooled_connection_is_not_retried() {
    let (port, counts) = server().await;
    let under = "  body:\n    type: text\n    data: \"x\"\n";
    let r = resolved_one("POST", &format!("http://127.0.0.1:{port}/"), under, "");
    let exec = executor();
    exec.send(&r).await.expect("first send");
    exec.send(&r).await.expect_err("second send fails");
    assert_eq!(counts.stale_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(counts.connections.load(Ordering::SeqCst), 1);
}
