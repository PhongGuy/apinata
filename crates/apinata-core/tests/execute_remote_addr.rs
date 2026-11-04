//! The peer address of each hop: the server's for a direct send, the proxy's through one.

use crate::common;
use crate::network_support::{custom_proxy, http_proxy, resolve_in, root_with};

use apinata_core::ContextOptions;
use common::{executor, resolved_one};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn a_direct_send_records_the_servers_address() {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let req = resolved_one("GET", &format!("{}/ok", server.uri()), "", "");
    let res = executor().send(&req).await.expect("sent");
    let want = *server.address();
    assert_eq!(res.remote_addr(), Some(want));
    assert!(!res.via_proxy());
    assert_eq!(res.peer_label(), Some(want.to_string()));
    assert_eq!(res.timeline()[0].remote_addr(), Some(want));
}

#[tokio::test]
async fn every_redirect_hop_records_its_own_address() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(path("/go"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/end", second.uri())),
        )
        .mount(&first)
        .await;
    Mock::given(path("/end"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&second)
        .await;
    let req = resolved_one("GET", &format!("{}/go", first.uri()), "", "");
    let res = executor().send(&req).await.expect("sent");
    let hops = res.timeline();
    assert_eq!(hops.len(), 2);
    assert_eq!(hops[0].remote_addr(), Some(*first.address()));
    assert_eq!(hops[1].remote_addr(), Some(*second.address()));
    assert_eq!(res.remote_addr(), Some(*second.address()));
}

#[tokio::test]
async fn through_a_proxy_the_address_is_the_proxys_and_says_so() {
    let origin = MockServer::start().await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&origin)
        .await;
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
    let res = executor().send(r.request()).await.expect("sent");
    assert_eq!(res.remote_addr().map(|a| a.port()), Some(proxy.port));
    assert!(res.via_proxy());
    assert_eq!(
        res.peer_label(),
        Some(format!("proxy 127.0.0.1:{}", proxy.port))
    );
}
