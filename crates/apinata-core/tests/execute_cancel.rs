//! Cancelling a send through a CancelHandle.

use crate::common;

use apinata_core::{CancelHandle, ExecError, ResolvedRequest};
use common::{executor, resolved_one};
use std::time::{Duration, Instant};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `/slow` answers after two seconds; `/login` sets a cookie and redirects to `/slow`.
async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "/slow")
                .insert_header("set-cookie", "sid=1; Path=/"),
        )
        .mount(&server)
        .await;
    server
}

/// Cancels `handle` once `server` has received `n` requests, so the send is provably waiting
/// on the server; polls every 10 ms.
async fn cancel_once_received(server: &MockServer, n: usize, handle: &CancelHandle) {
    while server.received_requests().await.expect("recorded").len() < n {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.cancel();
}

#[tokio::test]
async fn cancelling_stops_a_send_waiting_on_the_server() {
    let server = server().await;
    let req = resolved_one("GET", &format!("{}/slow", server.uri()), "", "");
    let cancel = CancelHandle::new();
    assert!(!cancel.is_cancelled());
    let exec = executor();
    let started = Instant::now();
    let (result, ()) = tokio::join!(
        exec.send_with(&req, &cancel),
        cancel_once_received(&server, 1, &cancel)
    );
    assert!(matches!(result, Err(ExecError::Cancelled)), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert!(cancel.is_cancelled());
    assert_eq!(ExecError::Cancelled.to_string(), "cancelled");
}

#[tokio::test]
async fn a_cancelled_handle_sends_nothing() {
    let server = server().await;
    let req = resolved_one("GET", &format!("{}/slow", server.uri()), "", "");
    let cancel = CancelHandle::new();
    cancel.clone().cancel();
    let result = executor().send_with(&req, &cancel).await;
    assert!(matches!(result, Err(ExecError::Cancelled)), "{result:?}");
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn cookies_from_hops_that_finished_stay_after_a_cancel() {
    let server = server().await;
    let req = resolved_one("GET", &format!("{}/login", server.uri()), "", "");
    let exec = executor();
    let cancel = CancelHandle::new();
    // The second request is the hop to /slow, so the first hop's cookie is stored by then.
    let (result, ()) = tokio::join!(
        exec.send_with(&req, &cancel),
        cancel_once_received(&server, 2, &cancel)
    );
    assert!(matches!(result, Err(ExecError::Cancelled)), "{result:?}");
    let names: Vec<String> = exec.cookies().list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["sid"]);
}

#[test]
fn send_with_is_send_so_it_can_be_spawned() {
    fn assert_send<T: Send>(_: &T) {}
    let exec = executor();
    let req: ResolvedRequest = resolved_one("GET", "http://127.0.0.1:9/", "", "");
    let cancel = CancelHandle::new();
    let fut = exec.send_with(&req, &cancel);
    assert_send(&fut);
}

/// Bodies whose file read never finishes by itself: a named pipe nothing writes to.
#[cfg(unix)]
mod file_reads {
    use super::*;
    use common::{at, setup};
    use std::path::Path;

    const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

    /// Opening the pipe read-write never blocks, and it releases a reader waiting on it.
    fn release(pipe: &Path) {
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe);
    }

    /// POSTs a body (`under_http`) naming the pipe `pipe.bin`, cancels after 200 ms, and
    /// checks the send gives up at once without sending anything.
    async fn expect_cancel_during_file_read(under_http: &str) {
        let server = MockServer::start().await;
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let text = format!(
            "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{}/\"\n{under_http}",
            server.uri()
        );
        let (dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
        let pipe = dir.path().join("pipe.bin");
        let made = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .expect("mkfifo runs");
        assert!(made.success(), "mkfifo failed");
        let req = at(&c, &ctx, "r.yml").expect("resolves");
        // A read that blocks this test's only thread would hang it; the helper releases the
        // pipe after five seconds so a blocking read fails the test instead of hanging it.
        let fallback = pipe.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            release(&fallback);
        });
        let exec = executor();
        let cancel = CancelHandle::new();
        let started = Instant::now();
        let (result, ()) = tokio::join!(exec.send_with(&req, &cancel), async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            cancel.cancel();
        });
        let elapsed = started.elapsed();
        // The abandoned read ends once released, so the runtime can shut down.
        release(&pipe);
        assert!(matches!(result, Err(ExecError::Cancelled)), "{result:?}");
        assert!(
            elapsed < Duration::from_secs(2),
            "the cancel took {elapsed:?}"
        );
        let seen = server.received_requests().await.expect("recorded");
        assert!(seen.is_empty(), "{} requests sent", seen.len());
    }

    #[tokio::test]
    async fn cancelling_interrupts_reading_a_file_body() {
        expect_cancel_during_file_read(
            "  body:\n    type: file\n    data:\n      - filePath: pipe.bin\n        contentType: x/y\n        selected: true\n",
        )
        .await;
    }

    #[tokio::test]
    async fn cancelling_interrupts_reading_a_multipart_file() {
        expect_cancel_during_file_read(
            "  body:\n    type: multipart-form\n    data:\n      - name: up\n        type: file\n        value: [pipe.bin]\n",
        )
        .await;
    }
}
