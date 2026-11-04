//! Proxy credentials and certificate passphrases never appear in an error's Display or Debug,
//! nor in a resolved request's Debug, on every listed failure.

use crate::network_support;

use apinata_core::{ContextOptions, ExecError, ExecOptions, Executor};
use network_support::{
    closed_port, custom_proxy, http_proxy, pem_certificates, pki, resolve_in, root_with,
    socks5_proxy, tls_server,
};

const USER: &str = "user-SEKRIT";
const PASS: &str = "pw SEKRIT/1";

fn assert_clean(text: &str) {
    for needle in ["SEKRIT", "pw%20", "user-"] {
        assert!(!text.contains(needle), "{needle} leaked: {text}");
    }
}

fn shown(err: &ExecError) -> String {
    format!("{err} {err:?}")
}

fn proxied(protocol: &str, port: u16, url: &str) -> network_support::Resolved {
    resolve_in(
        &root_with(&custom_proxy(
            protocol,
            "127.0.0.1",
            &port.to_string(),
            Some((USER, PASS)),
            "",
        )),
        url,
        ContextOptions::new(),
    )
}

#[tokio::test]
async fn an_unreachable_proxy_names_it_without_credentials() {
    let port = closed_port();
    let r = proxied("http", port, "http://127.0.0.1:9/x");
    assert_clean(&format!("{:?}", r.request()));
    let err = Executor::new(ExecOptions::new())
        .unwrap()
        .send(r.request())
        .await
        .expect_err("unreachable");
    let ExecError::Proxy { proxy, .. } = &err else {
        panic!("{err:?}")
    };
    assert_eq!(proxy, &format!("http://127.0.0.1:{port}"));
    assert_clean(&shown(&err));
}

#[tokio::test]
async fn a_connect_answered_407_is_a_proxy_error() {
    let server = tls_server(false).await;
    let proxy = http_proxy(Some("Basic nope")).await;
    let r = proxied(
        "http",
        proxy.port,
        &format!("https://127.0.0.1:{}/", server.port),
    );
    let err = Executor::new(ExecOptions::new().insecure(true))
        .unwrap()
        .send(r.request())
        .await
        .expect_err("407");
    assert!(matches!(err, ExecError::Proxy { .. }), "{err:?}");
    assert_clean(&shown(&err));
}

#[tokio::test]
async fn a_forward_proxy_407_is_an_ordinary_response() {
    let proxy = http_proxy(Some("Basic nope")).await;
    let r = proxied("http", proxy.port, "http://127.0.0.1:9/x");
    let res = Executor::new(ExecOptions::new())
        .unwrap()
        .send(r.request())
        .await
        .expect("a response");
    assert_eq!(res.status(), 407);
}

#[tokio::test]
async fn a_socks_rejection_is_a_proxy_error() {
    let proxy = socks5_proxy(Some(("other", "creds"))).await;
    let r = proxied("socks5", proxy.port, "http://127.0.0.1:9/x");
    let err = Executor::new(ExecOptions::new())
        .unwrap()
        .send(r.request())
        .await
        .expect_err("rejected");
    assert!(matches!(err, ExecError::Proxy { .. }), "{err:?}");
    assert_clean(&shown(&err));
}

#[tokio::test]
async fn a_wrong_passphrase_never_echoes_it() {
    let root = root_with(&pem_certificates(&[(
        "127.0.0.1",
        &pki("client.pem").display().to_string(),
        &pki("client-enc.key").display().to_string(),
        "pp-SEKRIT",
    )]));
    let r = resolve_in(
        &root,
        &format!("https://127.0.0.1:{}/", closed_port()),
        ContextOptions::new(),
    );
    assert_clean(&format!("{:?}", r.request()));
    let err = Executor::new(ExecOptions::new())
        .unwrap()
        .send(r.request())
        .await
        .expect_err("wrong passphrase");
    assert_clean(&shown(&err));
}

#[test]
fn an_invalid_proxy_with_credentials_reports_no_secret() {
    let root = root_with(&custom_proxy(
        "http",
        "127.0.0.1",
        "99999",
        Some((USER, PASS)),
        "",
    ));
    let err = resolve_in(&root, "http://a.test/", ContextOptions::new())
        .result
        .expect_err("bad port");
    assert_clean(&format!("{err} {err:?}"));
}
