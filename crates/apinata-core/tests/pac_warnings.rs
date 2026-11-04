//! PAC fallbacks reach the caller: `send_reporting` keeps one warning per cause even when the
//! send fails or is cancelled, and the runner lists them with the request's other warnings.

use crate::network_support;
use crate::script_support;

use std::time::Duration;

use apinata_core::{
    CancelHandle, ContextOptions, ExecError, ExecOptions, Executor, GlobalProxy, NetworkSettings,
    Outcome, PacSource, PacWarning, RunOptions, Warning,
};
use network_support::{closed_port, resolve_in};
use script_support::{send_full, write_tree};
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn throwing_pac() -> (TempDir, PacSource) {
    let dir = TempDir::new().expect("tempdir");
    let file = dir.path().join("proxy.pac");
    std::fs::write(
        &file,
        "function FindProxyForURL(url, host) { throw new Error('boom'); }",
    )
    .expect("pac");
    let source = PacSource::parse(file.to_str().expect("utf-8 path"), None).expect("source");
    (dir, source)
}

fn exec_options(source: PacSource) -> ExecOptions {
    ExecOptions::new().network(NetworkSettings::new().proxy(GlobalProxy::Pac { source }))
}

fn is_boom(warnings: &[PacWarning]) -> bool {
    matches!(warnings, [PacWarning::Script { message }] if message == "boom")
}

#[test]
fn the_warning_names_the_cause_and_the_fallback() {
    let cases = [
        (
            PacWarning::Timeout,
            "proxy auto-config ran past its time limit; connected directly",
        ),
        (
            PacWarning::Script {
                message: "boom".into(),
            },
            "proxy auto-config script failed: boom; connected directly",
        ),
        (
            PacWarning::Fetch {
                message: "file:///x.pac: not found".into(),
            },
            "proxy auto-config could not be fetched: file:///x.pac: not found; connected directly",
        ),
        (
            PacWarning::Answer {
                answer: "FTP a:1".into(),
            },
            "proxy auto-config answered \"FTP a:1\", which is not a usable proxy; connected directly",
        ),
    ];
    for (warning, text) in cases {
        assert_eq!(Warning::ProxyAutoConfig(warning).to_string(), text);
    }
}

#[tokio::test]
async fn one_warning_per_cause_across_redirect_hops() {
    let server = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/ping"))
        .mount(&server)
        .await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    let (_dir, source) = throwing_pac();
    let exec = Executor::new(exec_options(source)).expect("executor");
    let r = resolve_in(
        PLAIN,
        &format!("{}/start", server.uri()),
        ContextOptions::new(),
    );
    let (res, warnings) = exec.send_reporting(r.request(), &CancelHandle::new()).await;
    let res = res.expect("sent directly");
    assert_eq!((res.text(), res.timeline().len()), ("pong", 2));
    assert!(is_boom(&warnings), "{warnings:?}");
}

#[tokio::test]
async fn warnings_survive_a_failed_send() {
    let (_dir, source) = throwing_pac();
    let exec = Executor::new(exec_options(source)).expect("executor");
    let url = format!("http://127.0.0.1:{}/", closed_port());
    let r = resolve_in(PLAIN, &url, ContextOptions::new());
    let (res, warnings) = exec.send_reporting(r.request(), &CancelHandle::new()).await;
    assert!(res.is_err());
    assert!(is_boom(&warnings), "{warnings:?}");
}

#[tokio::test]
async fn warnings_survive_a_cancel() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    // Connections are held open and never answered, so only the cancel ends the send.
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    let (_dir, source) = throwing_pac();
    let exec = Executor::new(exec_options(source)).expect("executor");
    let r = resolve_in(
        PLAIN,
        &format!("http://127.0.0.1:{port}/"),
        ContextOptions::new(),
    );
    let cancel = CancelHandle::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let (res, warnings) = exec.send_reporting(r.request(), &cancel).await;
    assert!(matches!(res, Err(ExecError::Cancelled)), "{res:?}");
    assert!(is_boom(&warnings), "{warnings:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_runner_lists_pac_fallbacks_with_the_request_warnings() {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let request = |base: &str| {
        format!(
            "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{base}/ok\"\n"
        )
    };
    let boom = Warning::ProxyAutoConfig(PacWarning::Script {
        message: "boom".to_owned(),
    });
    let (_pac, source) = throwing_pac();

    let dir = write_tree(&[
        ("opencollection.yml", PLAIN),
        ("r.yml", &request(&server.uri())),
    ]);
    let (result, _) = send_full(
        dir.path(),
        "r.yml",
        RunOptions::new(),
        ContextOptions::new(),
        exec_options(source.clone()),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed);
    assert_eq!(result.warnings, std::slice::from_ref(&boom));

    let closed = format!("http://127.0.0.1:{}", closed_port());
    let failing = write_tree(&[("opencollection.yml", PLAIN), ("r.yml", &request(&closed))]);
    let (result, _) = send_full(
        failing.path(),
        "r.yml",
        RunOptions::new(),
        ContextOptions::new(),
        exec_options(source),
    )
    .await;
    assert!(result.error.is_some());
    assert!(result.warnings.contains(&boom), "{:?}", result.warnings);
}
