//! Connect failures are classified from the real error chains and read back as one shared hint.

use crate::common::resolved_one;
use crate::network_support::{closed_port, pki, tls_server, tls_server_using};
use apinata_core::{
    ConnectCause, ConnectHint, ExecError, ExecOptions, Executor, GlobalProxy, NetworkSettings,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn settings() -> NetworkSettings {
    NetworkSettings::new().proxy(GlobalProxy::Off)
}

/// The failure of a GET to `url` under `options`, which must be a connect failure.
async fn connect_failure(url: &str, options: ExecOptions) -> (String, String, ConnectCause) {
    let exec = Executor::new(options).expect("executor");
    let err = exec
        .send(&resolved_one("GET", url, "", ""))
        .await
        .expect_err("no connection");
    match err {
        ExecError::Connect {
            url,
            message,
            cause,
            ..
        } => (url, message, cause),
        other => panic!("{other:?}"),
    }
}

/// A plain-http server on 127.0.0.1: it answers whatever it reads with an HTTP 400.
async fn plain_http_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = tcp.read(&mut buf).await;
                let _ = tcp
                    .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    port
}

#[tokio::test]
async fn a_closed_port_is_refused() {
    let url = format!("http://127.0.0.1:{}/x", closed_port());
    let (_, _, cause) = connect_failure(&url, ExecOptions::new()).await;
    assert_eq!(cause, ConnectCause::Refused);
}

#[tokio::test]
async fn https_to_a_plain_http_server_is_the_wrong_protocol() {
    let port = plain_http_server().await;
    let url = format!("https://127.0.0.1:{port}/x");
    let (_, message, cause) = connect_failure(&url, ExecOptions::new()).await;
    assert_eq!(cause, ConnectCause::TlsWrongProtocol, "{message}");
}

#[tokio::test]
async fn an_untrusted_issuer_is_classified_as_such() {
    let server = tls_server(false).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let (_, message, cause) = connect_failure(&url, ExecOptions::new().network(settings())).await;
    assert_eq!(cause, ConnectCause::TlsUnknownIssuer, "{message}");
}

#[tokio::test]
async fn an_expired_certificate_from_a_trusted_ca_is_expired() {
    let server = tls_server_using("expired-server.pem", "expired-server.key", false).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let options = ExecOptions::new().network(settings().ca_file(Some(pki("expired-ca.pem"))));
    let (_, message, cause) = connect_failure(&url, options).await;
    assert_eq!(cause, ConnectCause::TlsExpired, "{message}");
}

#[test]
fn the_hint_reads_per_cause() {
    let text = |url: &str, cause| {
        let h = ConnectHint::new(url, cause);
        (h.title().to_owned(), h.sentence().to_owned())
    };
    let cases = [
        (
            "http://127.0.0.1:1/x",
            ConnectCause::Refused,
            "Connection refused",
            "Nothing is listening at 127.0.0.1:1. Is the server running?",
        ),
        (
            "https://api.example.test/x",
            ConnectCause::Dns,
            "DNS lookup failed",
            "The host api.example.test could not be found. Check the URL for typos.",
        ),
        (
            "https://self.test/",
            ConnectCause::Tls,
            "TLS error",
            "The secure connection to self.test failed.",
        ),
        (
            "https://self.test/",
            ConnectCause::TlsUnknownIssuer,
            "TLS error",
            "The certificate of self.test is not trusted (unknown issuer or self-signed). \
             Trust its CA or turn certificate verification off.",
        ),
        (
            "https://127.0.0.1:8080/",
            ConnectCause::TlsWrongProtocol,
            "TLS error",
            "127.0.0.1 did not answer TLS. The URL may need http:// instead of https://.",
        ),
        (
            "https://old.test/",
            ConnectCause::TlsExpired,
            "TLS error",
            "The certificate of old.test has expired.",
        ),
        (
            "http://h.test:8080/",
            ConnectCause::Other,
            "Could not connect",
            "The connection to h.test failed.",
        ),
    ];
    for (url, cause, title, sentence) in cases {
        assert_eq!(
            text(url, cause),
            (title.into(), sentence.into()),
            "{cause:?}"
        );
    }
}

#[test]
fn the_line_carries_url_title_sentence_and_transport_detail() {
    let h = ConnectHint::new("http://127.0.0.1:1/x", ConnectCause::Refused);
    assert_eq!(
        h.line("http://127.0.0.1:1/x", "tcp connect error"),
        "http://127.0.0.1:1/x: Connection refused: Nothing is listening at 127.0.0.1:1. \
         Is the server running? (tcp connect error)"
    );
    assert_eq!(
        h.text(),
        "Connection refused: Nothing is listening at 127.0.0.1:1. Is the server running?"
    );
}

#[test]
fn only_connect_errors_have_a_hint() {
    let connect = ExecError::connect("http://a.test/", "refused", ConnectCause::Refused);
    let hint = connect.connect_hint().expect("hint");
    assert_eq!(hint.title(), "Connection refused");
    assert_eq!(
        connect.hinted_line().as_deref(),
        Some(
            "http://a.test/: Connection refused: Nothing is listening at a.test:80. Is the server running? (refused)"
        )
    );
    let timeout = ExecError::timeout("http://a.test/");
    assert!(timeout.connect_hint().is_none());
    assert!(timeout.hinted_line().is_none());
}
