//! Client certificates and custom CAs against the local TLS server and the test PKI.

use crate::network_support;

use apinata_core::{
    ConnectCause, ContextOptions, ExecError, ExecOptions, Executor, FileAccess, GlobalProxy,
    NetworkSettings,
};
use network_support::{
    PASSPHRASE, closed_port, pem_certificates, pki, resolve_in, root_with, tls_server,
};
use std::path::PathBuf;

/// The certificate fixtures live outside each test's collection, so every context reads anywhere.
fn anywhere() -> ContextOptions {
    ContextOptions::new().file_access(FileAccess::Anywhere)
}

fn settings() -> NetworkSettings {
    NetworkSettings::new().proxy(GlobalProxy::Off)
}

fn trusting_the_ca() -> Executor {
    Executor::new(ExecOptions::new().network(settings().ca_file(Some(pki("ca.pem")))))
        .expect("executor")
}

fn path(name: &str) -> String {
    pki(name).display().to_string()
}

/// A collection whose one PEM certificate for 127.0.0.1 is `cert` with `key`.
fn with_identity(cert: &str, key: &str, passphrase: &str) -> String {
    root_with(&pem_certificates(&[("127.0.0.1", cert, key, passphrase)]))
}

fn certificate_error(err: &ExecError) -> (PathBuf, String) {
    match err {
        ExecError::Certificate { path, message, .. } => (path.clone(), message.clone()),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_pem_pair_completes_mutual_tls() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let r = resolve_in(
        &with_identity(&path("client.pem"), &path("client.key"), ""),
        &url,
        anywhere(),
    );
    let res = trusting_the_ca()
        .send(r.request())
        .await
        .expect("handshake");
    assert_eq!((res.status(), res.text()), (200, "ok"));
}

#[tokio::test]
async fn an_encrypted_pkcs8_key_opens_with_its_passphrase() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let root = with_identity(
        &path("client.pem"),
        &path("client-enc.key"),
        "{{process.env.PP}}",
    );
    let r = resolve_in(&root, &url, anywhere().process_env([("PP", PASSPHRASE)]));
    assert_eq!(
        trusting_the_ca()
            .send(r.request())
            .await
            .expect("handshake")
            .text(),
        "ok"
    );
}

#[tokio::test]
async fn key_problems_fail_before_any_connection() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let cases = [
        ("client-enc.key", "wrong", "wrong passphrase"),
        ("client-enc.key", "", "passphrase"),
        ("client-trad.key", PASSPHRASE, "openssl pkcs8 -topk8"),
        ("missing.key", "", ""),
        ("ca.pem", "", "no private key"),
    ];
    for (key, passphrase, says) in cases {
        let r = resolve_in(
            &with_identity(&path("client.pem"), &path(key), passphrase),
            &url,
            anywhere(),
        );
        let err = trusting_the_ca().send(r.request()).await.expect_err(key);
        let (at, message) = certificate_error(&err);
        assert_eq!(at, pki(key), "{key}");
        assert!(message.contains(says), "{key}: {message}");
    }
    let r = resolve_in(
        &with_identity(&path("client.key"), &path("client.key"), ""),
        &url,
        anywhere(),
    );
    let (at, message) = certificate_error(
        &trusting_the_ca()
            .send(r.request())
            .await
            .expect_err("no cert"),
    );
    assert_eq!(
        (at, message.contains("no certificate")),
        (pki("client.key"), true)
    );
    assert_eq!(server.connections(), 0);
}

/// A collection whose one PKCS#12 certificate for 127.0.0.1 is `file` with `passphrase`.
fn with_pkcs12(file: &str, passphrase: &str) -> String {
    root_with(&format!(
        "  clientCertificates:\n    - domain: \"127.0.0.1\"\n      type: pkcs12\n      pkcs12FilePath: '{file}'\n      passphrase: \"{passphrase}\"\n"
    ))
}

#[tokio::test]
async fn a_pkcs12_bundle_completes_mutual_tls_with_a_resolved_passphrase() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    for file in ["client.p12", "client-legacy.p12"] {
        let r = resolve_in(
            &with_pkcs12(&path(file), "{{process.env.PP}}"),
            &url,
            anywhere().process_env([("PP", PASSPHRASE)]),
        );
        let res = trusting_the_ca().send(r.request()).await.expect(file);
        assert_eq!((res.status(), res.text()), (200, "ok"), "{file}");
    }
}

#[tokio::test]
async fn pkcs12_problems_fail_before_any_connection() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let cases = [
        ("client.p12", "hunter2-not-it", "wrong passphrase"),
        ("client.p12", "", "wrong passphrase"),
        ("cert-only.p12", PASSPHRASE, "no private key"),
        ("client.pem", PASSPHRASE, "not a PKCS#12 file"),
        ("missing.p12", PASSPHRASE, ""),
    ];
    for (file, passphrase, says) in cases {
        let r = resolve_in(&with_pkcs12(&path(file), passphrase), &url, anywhere());
        let err = trusting_the_ca().send(r.request()).await.expect_err(file);
        let (at, message) = certificate_error(&err);
        assert_eq!(at, pki(file), "{file}");
        assert!(message.contains(says), "{file}: {message}");
        assert!(
            passphrase.is_empty() || !message.contains(passphrase),
            "{message}"
        );
    }
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn without_a_matching_certificate_the_server_refuses() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let root = root_with(&pem_certificates(&[(
        "other.test",
        &path("client.pem"),
        &path("client.key"),
        "",
    )]));
    let r = resolve_in(&root, &url, anywhere());
    let err = trusting_the_ca()
        .send(r.request())
        .await
        .expect_err("refused");
    assert!(err.to_string().contains("CertificateRequired"), "{err}");
}

#[tokio::test]
async fn an_expired_certificate_is_reported_by_the_server_alert() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let r = resolve_in(
        &with_identity(&path("expired-client.pem"), &path("client.key"), ""),
        &url,
        anywhere(),
    );
    let err = trusting_the_ca()
        .send(r.request())
        .await
        .expect_err("expired");
    assert!(err.to_string().contains("CertificateExpired"), "{err}");
}

#[tokio::test]
async fn an_edited_certificate_file_takes_effect_on_the_next_send() {
    let server = tls_server(true).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let cert = dir.path().join("c.pem");
    std::fs::copy(pki("client.pem"), &cert).expect("copy");
    let r = resolve_in(
        &with_identity(&cert.display().to_string(), &path("client.key"), ""),
        &url,
        anywhere(),
    );
    let exec = trusting_the_ca();
    assert_eq!(exec.send(r.request()).await.expect("first").text(), "ok");
    std::fs::copy(pki("expired-client.pem"), &cert).expect("swap");
    let err = exec
        .send(r.request())
        .await
        .expect_err("swapped for the expired one");
    assert!(err.to_string().contains("CertificateExpired"), "{err}");
}

#[tokio::test]
async fn the_custom_ca_is_merged_or_used_alone() {
    let server = tls_server(false).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let r = resolve_in(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        &url,
        anywhere(),
    );
    let unknown = Executor::new(ExecOptions::new().network(settings())).expect("executor");
    let err = unknown.send(r.request()).await.expect_err("untrusted");
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::TlsUnknownIssuer,
                ..
            }
        ) && err.to_string().contains("UnknownIssuer"),
        "{err}"
    );
    assert_eq!(
        trusting_the_ca()
            .send(r.request())
            .await
            .expect("merged")
            .text(),
        "ok"
    );
    let alone = Executor::new(
        ExecOptions::new().network(
            settings()
                .ca_file(Some(pki("ca.pem")))
                .keep_default_roots(false),
        ),
    )
    .expect("executor");
    assert_eq!(alone.send(r.request()).await.expect("alone").text(), "ok");
}

#[tokio::test]
async fn a_broken_ca_file_fails_the_send_not_the_executor() {
    let url = format!("https://127.0.0.1:{}/", closed_port());
    let r = resolve_in(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        &url,
        anywhere(),
    );
    for (file, says) in [("missing.pem", ""), ("client.key", "no certificate")] {
        let exec = Executor::new(ExecOptions::new().network(settings().ca_file(Some(pki(file)))))
            .expect("built lazily");
        let (at, message) = certificate_error(&exec.send(r.request()).await.expect_err(file));
        assert_eq!(at, pki(file));
        assert!(message.contains(says), "{file}: {message}");
    }
}
