//! `--insecure`: certificates are verified by default and accepted when asked.

use crate::common;

use apinata_core::{ConnectCause, ExecError, ExecOptions, Executor};
use common::resolved_one;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// A self-signed certificate for `localhost` and `127.0.0.1`, valid until 2126.
const CERT: &str = "
-----BEGIN CERTIFICATE-----
MIIBoTCCAUegAwIBAgIUFIBqvfCcF05xnBExaqLFzAG96MYwCgYIKoZIzj0EAwIw
FzEVMBMGA1UEAwwMYXBpbmF0YS10ZXN0MCAXDTI2MTAwOTA1MDMxM1oYDzIxMjYw
OTE1MDUwMzEzWjAXMRUwEwYDVQQDDAxhcGluYXRhLXRlc3QwWTATBgcqhkjOPQIB
BggqhkjOPQMBBwNCAAT4LqVaRLaBVS+l6zG3D0hJhiOKySg8X6J3r+7NoCxM/7Ua
jl+teuS/Hv8sS4b5DrjNwdg5S4yigSdWk56f/SWBo28wbTAdBgNVHQ4EFgQUsYA1
fOv6AaZgpupbIYQppcsxadYwHwYDVR0jBBgwFoAUsYA1fOv6AaZgpupbIYQppcsx
adYwDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEw
CgYIKoZIzj0EAwIDSAAwRQIgW78PGr1EL47OJvRV50d7nfIDyL+PTV1QciVzwFeG
pZoCIQCQ5L5QFFNh13q0EVPIK+kzXpBaCRlz989/cAVRwiaWKg==
-----END CERTIFICATE-----
";
/// The certificate's private key; it protects nothing.
const KEY: &str = "
-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg7OQzuCwTpdphOUYm
iWiOvC4cK98jZlaqF0EQgsdGN7ahRANCAAT4LqVaRLaBVS+l6zG3D0hJhiOKySg8
X6J3r+7NoCxM/7Uajl+teuS/Hv8sS4b5DrjNwdg5S4yigSdWk56f/SWB
-----END PRIVATE KEY-----
";

/// Serve `ok` over TLS to every connection on a random port; returns the base URL.
async fn https_server() -> String {
    serve(
        CertificateDer::from_pem_slice(CERT.as_bytes()).expect("cert"),
        PrivateKeyDer::from_pem_slice(KEY.as_bytes()).expect("key"),
    )
    .await
}

/// Like `https_server`, with the certificate and key read from the network fixtures by stem.
async fn https_server_fixture(stem: &str) -> String {
    let dir = format!("{}/tests/fixtures/network", env!("CARGO_MANIFEST_DIR"));
    serve(
        CertificateDer::from_pem_file(format!("{dir}/{stem}.pem")).expect("cert"),
        PrivateKeyDer::from_pem_file(format!("{dir}/{stem}.key")).expect("key"),
    )
    .await
}

async fn serve(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> String {
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("tls config");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.expect("accept");
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A failed handshake is the verifying client hanging up; nothing to answer.
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = vec![0; 4096];
                let _ = tls.read(&mut buf).await;
                let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
                let _ = tls.write_all(reply).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    format!("https://127.0.0.1:{port}/")
}

#[tokio::test]
async fn self_signed_certificates_fail_unless_insecure() {
    let url = https_server().await;
    let req = resolved_one("GET", &url, "", "");
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let err = strict.send(&req).await.expect_err("untrusted certificate");
    assert!(matches!(err, ExecError::Connect { .. }), "{err:?}");
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure.send(&req).await.expect("accepted");
    assert_eq!((res.status(), res.text()), (200, "ok"));
}

#[tokio::test]
async fn an_untrusted_certificate_is_a_tls_cause() {
    let url = https_server().await;
    let req = resolved_one("GET", &url, "", "");
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let err = strict.send(&req).await.expect_err("untrusted certificate");
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::TlsUnknownIssuer,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_https_hop_reports_connect_and_tls_as_one_phase() {
    let url = https_server().await;
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("accepted");
    let t = res.timeline()[0].timing.clone().expect("timing");
    assert_eq!(t.connect_kind, apinata_core::ConnectKind::ConnectAndTls);
    assert_eq!(t.dns, None, "{t:?}");
    assert!(t.connect.is_some() && !t.reused && !t.via_proxy, "{t:?}");
}

/// A self-signed certificate that is also a CA (openssl's default) is refused by rustls as a
/// CA used as an end entity; the cause must still be the untrusted-certificate one.
#[tokio::test]
async fn a_self_signed_ca_certificate_is_an_unknown_issuer_unless_insecure() {
    let url = https_server_fixture("selfsigned-ca-leaf").await;
    let req = resolved_one("GET", &url, "", "");
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let err = strict.send(&req).await.expect_err("untrusted certificate");
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::TlsUnknownIssuer,
                ..
            }
        ),
        "{err:?}"
    );
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure.send(&req).await.expect("accepted");
    assert_eq!((res.status(), res.text()), (200, "ok"));
}
