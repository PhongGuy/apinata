//! The HTTP version policy: its spellings and the ALPN protocols each policy offers over TLS.

use crate::common;

use apinata_core::{
    ExecOptions, Executor, GlobalProxy, HttpVersion, HttpVersionPolicy, NetworkSettings, PacSource,
};
use common::resolved_one;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::LazyConfigAcceptor;
use tokio_rustls::rustls::server::Acceptor;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

type Offers = Arc<Mutex<Vec<Vec<String>>>>;

/// A TLS listener on 127.0.0.1 that records the ALPN protocols of every ClientHello, then
/// hangs up, so the client's handshake fails only after its offer is recorded.
async fn alpn_recorder() -> (u16, Offers) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen: Offers = Arc::default();
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let Ok(start) = LazyConfigAcceptor::new(Acceptor::default(), tcp).await else {
                    return;
                };
                let offer: Vec<String> = start
                    .client_hello()
                    .alpn()
                    .map(|list| {
                        list.map(|p| String::from_utf8_lossy(p).into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                log.lock().expect("seen").push(offer);
            });
        }
    });
    (port, seen)
}

/// The offers `seen` holds, all equal, as one list.
fn one_offer(seen: &Offers) -> Vec<String> {
    let seen = seen.lock().expect("seen").clone();
    assert!(!seen.is_empty(), "no ClientHello arrived");
    assert!(seen.iter().all(|o| *o == seen[0]), "{seen:?}");
    seen[0].clone()
}

/// The ALPN list one GET under `opts` offers to the recorder.
async fn offered(opts: ExecOptions) -> Vec<String> {
    let (port, seen) = alpn_recorder().await;
    let exec = Executor::new(opts).expect("executor");
    let req = resolved_one("GET", &format!("https://127.0.0.1:{port}/"), "", "");
    let sent = tokio::time::timeout(Duration::from_secs(10), exec.send(&req))
        .await
        .expect("the handshake ends");
    assert!(sent.is_err(), "the recorder never completes a handshake");
    one_offer(&seen)
}

#[test]
fn each_policy_has_one_spelling() {
    assert_eq!(HttpVersionPolicy::default(), HttpVersionPolicy::Http1Only);
    for (policy, text) in [
        (HttpVersionPolicy::Http1Only, "1.1"),
        (HttpVersionPolicy::Auto, "auto"),
        (HttpVersionPolicy::Http2PriorKnowledge, "2-prior-knowledge"),
    ] {
        assert_eq!(policy.to_string(), text);
        assert_eq!(text.parse::<HttpVersionPolicy>(), Ok(policy));
    }
    for bad in ["", "2", "h2", "HTTP/1.1", "Auto", " auto"] {
        assert!(bad.parse::<HttpVersionPolicy>().is_err(), "{bad:?}");
    }
}

#[test]
fn versions_display_as_http_names() {
    assert_eq!(HttpVersion::Http10.to_string(), "HTTP/1.0");
    assert_eq!(HttpVersion::Http11.to_string(), "HTTP/1.1");
    assert_eq!(HttpVersion::Http2.to_string(), "HTTP/2");
}

#[tokio::test]
async fn the_default_offers_only_http1() {
    assert_eq!(offered(ExecOptions::new()).await, ["http/1.1"]);
}

#[tokio::test]
async fn each_policy_offers_its_alpn_protocols() {
    for (policy, alpn) in [
        (HttpVersionPolicy::Http1Only, &["http/1.1"][..]),
        (HttpVersionPolicy::Auto, &["h2", "http/1.1"][..]),
        (HttpVersionPolicy::Http2PriorKnowledge, &["h2"][..]),
    ] {
        let offer = offered(ExecOptions::new().http_version(policy)).await;
        assert_eq!(offer, alpn, "{policy}");
    }
}

#[tokio::test]
async fn the_pac_fetch_offers_only_http1_under_auto() {
    let (port, seen) = alpn_recorder().await;
    let target = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&target)
        .await;
    let source =
        PacSource::parse(&format!("https://127.0.0.1:{port}/proxy.pac"), None).expect("pac source");
    let exec = Executor::new(
        ExecOptions::new()
            .http_version(HttpVersionPolicy::Auto)
            .network(NetworkSettings::new().proxy(GlobalProxy::Pac { source })),
    )
    .expect("executor");
    let req = resolved_one("GET", &format!("{}/", target.uri()), "", "");
    // The fetch fails, so the hop goes direct or fails; only the fetch's offer matters here.
    let _ = tokio::time::timeout(Duration::from_secs(20), exec.send(&req))
        .await
        .expect("the send ends");
    assert_eq!(one_offer(&seen), ["http/1.1"]);
}
