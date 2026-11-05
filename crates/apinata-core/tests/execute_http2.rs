//! The negotiated HTTP version of each hop: h2c, h2 over TLS, fallbacks and redirects.

use crate::common;
use crate::network_support;

use apinata_core::{
    ConnectKind, ExecError, ExecOptions, Executor, GlobalProxy, HttpVersion, HttpVersionPolicy,
    NetworkSettings,
};
use common::resolved_one;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use network_support::{pki, tls_server};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use wiremock::matchers::{any, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use HttpVersionPolicy::{Auto, Http1Only, Http2PriorKnowledge};

/// An executor under `policy` that trusts the test CA and uses no proxy.
fn executor(policy: HttpVersionPolicy) -> Executor {
    let network = NetworkSettings::new()
        .proxy(GlobalProxy::Off)
        .ca_file(Some(pki("ca.pem")));
    Executor::new(ExecOptions::new().http_version(policy).network(network)).expect("executor")
}

/// A wiremock server answering every request 200; its auto server speaks h1 and h2c.
async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&s)
        .await;
    s
}

/// Each hop's version for one GET of `url`.
async fn versions(exec: &Executor, url: &str) -> Vec<Option<HttpVersion>> {
    let res = exec
        .send(&resolved_one("GET", url, "", ""))
        .await
        .expect("sent");
    res.timeline().iter().map(|h| h.version()).collect()
}

/// An HTTPS server on 127.0.0.1 with the test PKI, offering ALPN `h2, http/1.1` and serving
/// both through hyper-util's auto builder: 302 to `location` when given, else 200 `ok`.
async fn h2_tls_server(location: Option<String>) -> u16 {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(pki("server.pem"))
        .expect("server.pem")
        .collect::<Result<_, _>>()
        .expect("certs");
    let key = PrivateKeyDer::from_pem_file(pki("server.key")).expect("server.key");
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("tls config");
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, location) = (acceptor.clone(), location.clone());
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |_: hyper::Request<Incoming>| {
                    let location = location.clone();
                    async move {
                        let reply = match location {
                            Some(to) => hyper::Response::builder()
                                .status(302)
                                .header("location", to)
                                .body(String::new()),
                            None => hyper::Response::builder().status(200).body("ok".to_owned()),
                        };
                        Ok::<_, Infallible>(reply.expect("response"))
                    }
                });
                let _ = auto::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });
    port
}

/// A plain TCP origin on 127.0.0.1 that answers anything with HTTP/1.1 bytes and closes.
async fn http1_only_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = tcp.read(&mut buf).await;
                let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
                let _ = tcp.write_all(reply).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    port
}

#[tokio::test]
async fn prior_knowledge_speaks_h2c_on_plain_http() {
    let s = ok_server().await;
    let url = format!("{}/", s.uri());
    assert_eq!(
        versions(&executor(Http2PriorKnowledge), &url).await,
        [Some(HttpVersion::Http2)]
    );
}

#[tokio::test]
async fn plain_http_stays_http1_by_default_and_under_auto() {
    let s = ok_server().await;
    let url = format!("{}/", s.uri());
    for policy in [Http1Only, Auto] {
        assert_eq!(
            versions(&executor(policy), &url).await,
            [Some(HttpVersion::Http11)],
            "{policy}"
        );
    }
}

#[tokio::test]
async fn tls_negotiates_h2_under_auto_and_prior_knowledge_only() {
    let port = h2_tls_server(None).await;
    let url = format!("https://127.0.0.1:{port}/");
    for (policy, version) in [
        (Http1Only, HttpVersion::Http11),
        (Auto, HttpVersion::Http2),
        (Http2PriorKnowledge, HttpVersion::Http2),
    ] {
        let res = executor(policy)
            .send(&resolved_one("GET", &url, "", ""))
            .await
            .expect("sent");
        assert_eq!((res.status(), res.text()), (200, "ok"), "{policy}");
        assert_eq!(res.timeline()[0].version(), Some(version), "{policy}");
    }
}

#[tokio::test]
async fn auto_uses_http1_when_the_tls_server_offers_no_h2() {
    let server = tls_server(false).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    assert_eq!(
        versions(&executor(Auto), &url).await,
        [Some(HttpVersion::Http11)]
    );
}

#[tokio::test]
async fn a_redirect_from_h2_tls_to_plain_http_keeps_each_hops_version() {
    let plain = MockServer::start().await;
    Mock::given(path("/plain"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&plain)
        .await;
    let port = h2_tls_server(Some(format!("{}/plain", plain.uri()))).await;
    let res = executor(Auto)
        .send(&resolved_one(
            "GET",
            &format!("https://127.0.0.1:{port}/"),
            "",
            "",
        ))
        .await
        .expect("sent");
    let hops: Vec<(u16, Option<HttpVersion>)> = res
        .timeline()
        .iter()
        .map(|h| (h.status, h.version()))
        .collect();
    assert_eq!(
        hops,
        [
            (302, Some(HttpVersion::Http2)),
            (200, Some(HttpVersion::Http11))
        ]
    );
}

#[tokio::test]
async fn prior_knowledge_against_an_http1_only_server_is_a_transport_error() {
    let port = http1_only_server().await;
    let url = format!("http://127.0.0.1:{port}/");
    let err = tokio::time::timeout(
        Duration::from_secs(10),
        executor(Http2PriorKnowledge).send(&resolved_one("GET", &url, "", "")),
    )
    .await
    .expect("fails promptly")
    .expect_err("h2 preface refused");
    match err {
        ExecError::Transport { url, .. } => assert!(url.contains("127.0.0.1"), "{url}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_session_log_keeps_each_hops_version() {
    let s = ok_server().await;
    let log = apinata_core::SessionLog::new();
    let exec = Executor::new(
        ExecOptions::new()
            .http_version(Http2PriorKnowledge)
            .session_log(log.clone()),
    )
    .expect("executor");
    exec.send(&resolved_one("GET", &format!("{}/", s.uri()), "", ""))
        .await
        .expect("sent");
    let entry = log.network_since(0).entries.remove(0);
    assert_eq!(entry.hops[0].version(), Some(HttpVersion::Http2));
}

#[tokio::test]
async fn a_second_h2_send_reuses_the_connection_with_no_connect_phase() {
    let port = h2_tls_server(None).await;
    let req = resolved_one("GET", &format!("https://127.0.0.1:{port}/"), "", "");
    let exec = executor(Auto);
    let mut hops = Vec::new();
    for _ in 0..2 {
        let res = exec.send(&req).await.expect("sent");
        let hop = &res.timeline()[0];
        assert_eq!(hop.version(), Some(HttpVersion::Http2));
        hops.push(hop.timing.clone().expect("timing"));
    }
    let (first, second) = (&hops[0], &hops[1]);
    assert!(first.connect.is_some() && !first.reused, "{first:?}");
    assert_eq!(first.connect_kind, ConnectKind::ConnectAndTls);
    // Reuse means no lookup or connect was measured, so neither appears as a zero.
    assert!(second.reused, "{second:?}");
    assert_eq!((second.dns, second.connect), (None, None), "{second:?}");
}
