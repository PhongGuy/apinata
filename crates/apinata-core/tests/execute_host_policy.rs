//! The host policy's pre-resolution rules: each hop's host is judged before any I/O for it.

use crate::common;

use apinata_core::{ExecError, ExecOptions, Executor};
use common::resolved_one;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// An executor whose policy trusts exactly `trusted` and records every host it is asked about.
fn guarded(trusted: &'static [&'static str]) -> (Executor, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let policy = move |host: &str| {
        log.lock().unwrap().push(host.to_owned());
        trusted.contains(&host)
    };
    let exec = Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client");
    (exec, asked)
}

/// Sends `url` through `exec`, failing the test if the send takes longer than two seconds,
/// which a dial to an unroutable TEST-NET address would.
async fn send(exec: &Executor, url: &str, top: &str) -> Result<u16, ExecError> {
    let req = resolved_one("GET", url, "", top);
    tokio::time::timeout(Duration::from_secs(2), exec.send(&req))
        .await
        .expect("refused without dialling")
        .map(|res| res.status())
}

async fn ok_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn loopback_literals_and_localhost_pass_without_asking_the_policy() {
    let server = ok_server().await;
    let port = server.address().port();
    let (exec, asked) = guarded(&[]);
    for url in [
        format!("http://127.0.0.1:{port}/ok"),
        format!("http://0x7f.0.0.1:{port}/ok"),
        format!("http://localhost:{port}/ok"),
    ] {
        assert_eq!(send(&exec, &url, "").await.expect(&url), 200, "{url}");
    }
    // Nothing listens on [::1]:1, so passing the policy shows as a connect failure.
    let err = send(&exec, "http://[::1]:1/", "").await.unwrap_err();
    assert!(matches!(err, ExecError::Connect { .. }), "{err:?}");
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_untrusted_name_is_refused_normalised_and_never_reached() {
    let (exec, asked) = guarded(&[]);
    let err = send(&exec, "http://API.Example.TEST.:1/x", "")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 0, host, .. } if host == "api.example.test"),
        "{err:?}"
    );
    assert_eq!(*asked.lock().unwrap(), ["api.example.test"]);
}

#[tokio::test]
async fn non_loopback_ip_literals_including_private_ranges_are_refused_unless_trusted() {
    let (exec, _) = guarded(&[]);
    for (url, host) in [
        ("http://192.0.2.1/", "192.0.2.1"),
        ("http://10.0.0.1/", "10.0.0.1"),
        ("http://169.254.169.254/latest", "169.254.169.254"),
        ("http://[fe80::1]/", "[fe80::1]"),
        ("http://[::ffff:10.0.0.1]/", "[::ffff:a00:1]"),
    ] {
        let err = send(&exec, url, "").await.unwrap_err();
        assert!(
            matches!(&err, ExecError::HostNotAllowed { hop: 0, host: h, .. } if h == host),
            "{url}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_trusted_ip_literal_reaches_the_connect() {
    let (exec, asked) = guarded(&["192.0.2.1"]);
    let err = send(&exec, "http://192.0.2.1/", "settings:\n  timeout: 300\n")
        .await
        .unwrap_err();
    assert!(
        matches!(err, ExecError::Timeout { .. } | ExecError::Connect { .. }),
        "{err:?}"
    );
    assert_eq!(*asked.lock().unwrap(), ["192.0.2.1"]);
}

#[tokio::test]
async fn a_redirect_to_an_untrusted_host_is_refused_at_its_hop_before_dialling() {
    let server = MockServer::start().await;
    Mock::given(path("/to-ip"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://192.0.2.1/x"))
        .mount(&server)
        .await;
    Mock::given(path("/to-name"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "http://evil.example/"))
        .mount(&server)
        .await;
    let (exec, _) = guarded(&[]);
    let err = send(&exec, &format!("{}/to-ip", server.uri()), "")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if host == "192.0.2.1"),
        "{err:?}"
    );
    let err = send(&exec, &format!("{}/to-name", server.uri()), "")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if host == "evil.example"),
        "{err:?}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn the_first_hop_is_judged_before_body_files_are_read() {
    let (exec, _) = guarded(&[]);
    let body = "  body:\n    type: file\n    data:\n      - filePath: missing.bin\n        contentType: application/octet-stream\n        selected: true\n";
    let req = resolved_one("POST", "http://evil.example/", body, "");
    let err = exec.send(&req).await.unwrap_err();
    assert!(
        matches!(err, ExecError::HostNotAllowed { hop: 0, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn without_a_policy_any_host_is_dialled() {
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let req = resolved_one(
        "GET",
        "http://192.0.2.1/",
        "",
        "settings:\n  timeout: 300\n",
    );
    let err = exec.send(&req).await.unwrap_err();
    assert!(
        matches!(err, ExecError::Timeout { .. } | ExecError::Connect { .. }),
        "{err:?}"
    );
}
