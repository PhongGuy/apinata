//! The request timeout is a per-hop deadline over headers and body, restarted for each hop.

use crate::common::{executor, resolved_one};
use apinata_core::ExecError;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Serves every connection: read the request head, wait `delay`, write `head`, then run `after`.
async fn server<F, Fut>(delay: Duration, head: String, after: F) -> u16
where
    F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let after = Arc::new(after);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (after, head) = (Arc::clone(&after), head.clone());
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                tokio::time::sleep(delay).await;
                if sock.write_all(head.as_bytes()).await.is_ok() {
                    after(sock).await;
                }
            });
        }
    });
    port
}

const CHUNKED: &str =
    "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\n\r\n";

/// Writes one byte as a chunk every 100 ms, `count` times, then ends the body.
async fn trickle(mut sock: TcpStream, count: u32) {
    for _ in 0..count {
        if sock.write_all(b"1\r\nx\r\n").await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = sock.write_all(b"0\r\n\r\n").await;
}

fn req(port: u16, timeout_ms: Option<u64>) -> apinata_core::ResolvedRequest {
    let top = timeout_ms.map_or(String::new(), |t| format!("settings:\n  timeout: {t}\n"));
    resolved_one("GET", &format!("http://127.0.0.1:{port}/"), "", &top)
}

#[tokio::test]
async fn a_body_that_stalls_after_headers_times_out() {
    let head = "HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\npartial".to_owned();
    // The socket moves into the future so it stays open while the body stalls.
    let port = server(Duration::ZERO, head, |sock| async move {
        let _open = sock;
        std::future::pending::<()>().await
    })
    .await;
    let started = Instant::now();
    let err = executor()
        .send(&req(port, Some(300)))
        .await
        .expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_trickling_body_hits_the_total_deadline_not_an_idle_one() {
    let port = server(Duration::ZERO, CHUNKED.to_owned(), |s| trickle(s, 50)).await;
    let started = Instant::now();
    let err = executor()
        .send(&req(port, Some(400)))
        .await
        .expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
    assert!(started.elapsed() >= Duration::from_millis(350));
}

#[tokio::test]
async fn without_a_timeout_a_slow_body_completes() {
    let port = server(Duration::ZERO, CHUNKED.to_owned(), |s| trickle(s, 6)).await;
    let res = executor().send(&req(port, None)).await.expect("sent");
    assert_eq!(res.text(), "xxxxxx");
}

#[tokio::test]
async fn each_redirect_hop_gets_a_fresh_deadline() {
    // Each hop answers after 1 s against a 1.6 s timeout; the two together exceed it.
    let last = server(
        Duration::from_secs(1),
        "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok".to_owned(),
        |_s| std::future::pending(),
    )
    .await;
    let first = server(
        Duration::from_secs(1),
        format!(
            "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:{last}/\r\ncontent-length: 0\r\n\r\n"
        ),
        |_s| std::future::pending(),
    )
    .await;
    let res = executor()
        .send(&req(first, Some(1600)))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(res.timeline().len(), 2);
}

#[tokio::test]
async fn a_stalled_pac_fetch_counts_inside_the_request_timeout() {
    use apinata_core::{ExecOptions, Executor, GlobalProxy, NetworkSettings, PacSource};
    // The PAC host accepts and then says nothing, so its fetch would run to its own 5 s limit.
    let pac_port = server(Duration::from_secs(60), String::new(), |_| async {}).await;
    let source =
        PacSource::parse(&format!("http://127.0.0.1:{pac_port}/proxy.pac"), None).expect("source");
    let exec = Executor::new(
        ExecOptions::new().network(NetworkSettings::new().proxy(GlobalProxy::Pac { source })),
    )
    .expect("executor");
    let started = Instant::now();
    let err = exec.send(&req(1, Some(2000))).await.expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn pac_time_counts_inside_an_ntlm_hops_timeout() {
    use apinata_core::{ExecOptions, Executor, GlobalProxy, NetworkSettings, PacSource};
    // The PAC fetch takes 1.5 s and the origin another 1.5 s to answer the probe: each fits
    // the 2 s timeout alone, so only a deadline shared with the PAC lookup times the hop out.
    let script = "function FindProxyForURL(u, h) { return 'DIRECT'; }";
    let pac_port = server(
        Duration::from_millis(1500),
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/x-ns-proxy-autoconfig\r\ncontent-length: {}\r\n\r\n{script}",
            script.len()
        ),
        |_| async {},
    )
    .await;
    let origin = server(
        Duration::from_millis(1500),
        "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok".to_owned(),
        |_s| std::future::pending(),
    )
    .await;
    let source =
        PacSource::parse(&format!("http://127.0.0.1:{pac_port}/proxy.pac"), None).expect("source");
    let exec = Executor::new(
        ExecOptions::new().network(NetworkSettings::new().proxy(GlobalProxy::Pac { source })),
    )
    .expect("executor");
    let ntlm = "  auth:\n    type: ntlm\n    username: u\n    password: p\n    domain: D\n";
    let request = resolved_one(
        "GET",
        &format!("http://127.0.0.1:{origin}/"),
        ntlm,
        "settings:\n  timeout: 2000\n",
    );
    let started = Instant::now();
    let err = exec.send(&request).await.expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}

fn capped(secs_ms: u64) -> apinata_core::Executor {
    apinata_core::Executor::new(
        apinata_core::ExecOptions::new().timeout_cap(Some(Duration::from_millis(secs_ms))),
    )
    .expect("executor")
}

#[tokio::test]
async fn a_cap_bounds_a_request_with_no_timeout_and_names_itself() {
    let port = server(
        Duration::from_secs(3),
        "HTTP/1.1 200 OK\r\n\r\n".into(),
        |_| async {},
    )
    .await;
    let started = Instant::now();
    let err = capped(500)
        .send(&req(port, None))
        .await
        .expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
    assert!(err.to_string().contains("run-wide timeout of"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_shorter_own_timeout_beats_the_cap_and_keeps_the_plain_message() {
    let port = server(
        Duration::from_secs(3),
        "HTTP/1.1 200 OK\r\n\r\n".into(),
        |_| async {},
    )
    .await;
    let err = capped(2000)
        .send(&req(port, Some(300)))
        .await
        .expect_err("times out");
    assert!(err.to_string().ends_with(": timed out"), "{err}");
}

#[tokio::test]
async fn a_cap_bounds_a_later_redirect_hop_and_names_itself() {
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    Mock::given(path("/"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/slow"))
        .mount(&s)
        .await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&s)
        .await;
    let url = format!("{}/", s.uri());
    let started = Instant::now();
    let err = capped(500)
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect_err("second hop times out");
    assert!(
        matches!(&err, ExecError::Timeout { url, .. } if url.ends_with("/slow")),
        "{err:?}"
    );
    assert!(err.to_string().contains("run-wide timeout of"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
}
