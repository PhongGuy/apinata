//! Under a host policy the proxy is judged like any host before I/O, and a secret in its host
//! or port refuses the send.

use crate::network_support;

use apinata_core::{ContextOptions, ExecError, ExecOptions, Executor};
use network_support::{custom_proxy, http_proxy, resolve_in, root_with, socks5_proxy};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn guarded() -> (Executor, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let policy = move |host: &str| {
        log.lock().unwrap().push(host.to_owned());
        false
    };
    (
        Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("executor"),
        asked,
    )
}

async fn origin() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    server
}

async fn send(exec: &Executor, req: &apinata_core::ResolvedRequest) -> Result<String, ExecError> {
    tokio::time::timeout(Duration::from_secs(2), exec.send(req))
        .await
        .expect("decided without dialling")
        .map(|res| res.text().to_owned())
}

#[tokio::test]
async fn an_untrusted_proxy_is_refused_before_any_connection() {
    let origin = origin().await;
    let url = format!("{}/ping", origin.uri());
    for host in ["proxy.corp.example", "192.0.2.10"] {
        let (exec, asked) = guarded();
        let r = resolve_in(
            &root_with(&custom_proxy("http", host, "3128", None, "")),
            &url,
            ContextOptions::new(),
        );
        let err = send(&exec, r.request()).await.expect_err(host);
        let want = format!("http://{host}:3128");
        assert!(
            matches!(&err, ExecError::ProxyNotAllowed { hop: 0, host: h, proxy, .. } if h == host && *proxy == want),
            "{err:?}"
        );
        assert!(asked.lock().unwrap().iter().any(|h| h == host));
    }
    assert_eq!(
        origin.received_requests().await.expect("recording").len(),
        0
    );
}

#[tokio::test]
async fn a_loopback_proxy_needs_no_trust() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let r = resolve_in(
        &root_with(&custom_proxy(
            "http",
            "127.0.0.1",
            &proxy.port.to_string(),
            None,
            "",
        )),
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    let (exec, _) = guarded();
    assert_eq!(send(&exec, r.request()).await.expect("sent"), "pong");
    assert_eq!(proxy.seen.lines().len(), 1);
}

#[tokio::test]
async fn a_bypassed_hop_never_asks_about_the_proxy() {
    let origin = origin().await;
    let r = resolve_in(
        &root_with(&custom_proxy(
            "http",
            "proxy.corp.example",
            "3128",
            None,
            "127.0.0.1",
        )),
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    let (exec, asked) = guarded();
    assert_eq!(send(&exec, r.request()).await.expect("direct"), "pong");
    assert!(
        !asked
            .lock()
            .unwrap()
            .iter()
            .any(|h| h == "proxy.corp.example")
    );
}

#[tokio::test]
async fn a_secret_in_the_proxy_host_refuses_only_under_a_policy() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let root = root_with(&custom_proxy(
        "http",
        "{{process.env.H}}",
        &proxy.port.to_string(),
        None,
        "",
    ));
    let options = || ContextOptions::new().process_env([("H", "127.0.0.1")]);
    let url = format!("{}/ping", origin.uri());
    let r = resolve_in(&root, &url, options());
    let (exec, asked) = guarded();
    assert!(matches!(
        send(&exec, r.request()).await,
        Err(ExecError::SecretInProxy)
    ));
    assert!(proxy.seen.lines().is_empty() && asked.lock().unwrap().is_empty());
    let open = Executor::new(ExecOptions::new()).expect("executor");
    assert_eq!(
        send(&open, r.request())
            .await
            .expect("no policy, no refusal"),
        "pong"
    );
}

#[tokio::test]
async fn a_secret_only_in_the_credentials_is_fine() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let root = root_with(&custom_proxy(
        "http",
        "127.0.0.1",
        &proxy.port.to_string(),
        Some(("u", "{{process.env.PW}}")),
        "",
    ));
    let r = resolve_in(
        &root,
        &format!("{}/ping", origin.uri()),
        ContextOptions::new().process_env([("PW", "pw")]),
    );
    let (exec, _) = guarded();
    assert_eq!(send(&exec, r.request()).await.expect("sent"), "pong");
}

#[tokio::test]
async fn a_secret_proxy_password_inside_the_target_host_is_not_a_secret_host() {
    let root = root_with(&custom_proxy(
        "http",
        "127.0.0.1",
        "3128",
        Some(("u", "{{process.env.PW}}")),
        "",
    ));
    let r = resolve_in(
        &root,
        "https://api.example.test/",
        ContextOptions::new().process_env([("PW", "example")]),
    );
    let (exec, _) = guarded();
    let err = send(&exec, r.request())
        .await
        .expect_err("untrusted target");
    assert!(
        matches!(&err, ExecError::HostNotAllowed { host, .. } if host == "api.example.test"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_literal_proxy_password_inside_the_target_host_is_not_a_secret_host() {
    let root = root_with(&custom_proxy(
        "http",
        "127.0.0.1",
        "3128",
        Some(("u", "example")),
        "",
    ));
    let r = resolve_in(&root, "https://api.example.test/", ContextOptions::new());
    let (exec, _) = guarded();
    let err = send(&exec, r.request())
        .await
        .expect_err("untrusted target");
    assert!(
        matches!(&err, ExecError::HostNotAllowed { host, .. } if host == "api.example.test"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_local_dns_socks_proxy_gets_the_target_resolved_by_the_guard() {
    let origin = origin().await;
    let port = origin.address().port();
    let proxy = socks5_proxy(None).await;
    let r = resolve_in(
        &root_with(&custom_proxy(
            "socks5",
            "127.0.0.1",
            &proxy.port.to_string(),
            None,
            "",
        )),
        &format!("http://localhost:{port}/ping"),
        ContextOptions::new(),
    );
    let (exec, asked) = guarded();
    assert_eq!(send(&exec, r.request()).await.expect("sent"), "pong");
    assert!(
        asked.lock().unwrap().is_empty(),
        "loopback proxy and local target need no trust"
    );
    assert_eq!(proxy.seen.lines(), [format!("connect 127.0.0.1:{port}")]);
}

#[test]
fn a_refused_proxy_names_itself() {
    let err =
        ExecError::proxy_not_allowed(1, "proxy.corp.example", "http://proxy.corp.example:3128");
    assert_eq!(
        err.to_string(),
        "host `proxy.corp.example` is not allowed (hop 1, via proxy http://proxy.corp.example:3128)"
    );
}

#[tokio::test]
async fn a_target_the_guard_refuses_through_a_local_dns_socks_proxy_names_the_proxy() {
    let proxy = socks5_proxy(None).await;
    let r = resolve_in(
        &root_with(&custom_proxy(
            "socks5",
            "127.0.0.1",
            &proxy.port.to_string(),
            None,
            "",
        )),
        "http://api.example.test/ping",
        ContextOptions::new(),
    );
    // Yes to the pre-check by name, no when the guard resolver asks again for the SOCKS dial.
    let asked = Arc::new(Mutex::new(0usize));
    let count = Arc::clone(&asked);
    let policy = move |_: &str| {
        let mut n = count.lock().unwrap();
        *n += 1;
        *n == 1
    };
    let exec = Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("executor");
    let err = send(&exec, r.request()).await.expect_err("refused");
    let want = format!("socks5://127.0.0.1:{}", proxy.port);
    assert!(
        matches!(&err, ExecError::ProxyNotAllowed { hop: 0, host, proxy, .. } if host == "api.example.test" && *proxy == want),
        "{err:?}"
    );
    assert_eq!(*asked.lock().unwrap(), 2, "the pre-check and the resolver");
    assert!(proxy.seen.lines().is_empty(), "nothing was dialled");
}
