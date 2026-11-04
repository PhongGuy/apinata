//! Proxy routing in the executor: the collection layer, the global layer, the bypass list per
//! hop, SOCKS5, and one client per rule so proxies never share a connection.

use crate::network_support;

use apinata_core::{
    ContextOptions, ExecOptions, Executor, GlobalProxy, NetworkSettings, ResolvedProxy,
};
use network_support::{custom_proxy, http_proxy, resolve_in, root_with, socks5_proxy, tls_server};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

async fn origin() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    server
}

fn via(protocol: &str, port: u16, auth: Option<(&str, &str)>, bypass: &str) -> String {
    root_with(&custom_proxy(
        protocol,
        "127.0.0.1",
        &port.to_string(),
        auth,
        bypass,
    ))
}

fn direct() -> Executor {
    Executor::new(ExecOptions::new()).expect("executor")
}

async fn received(server: &MockServer) -> usize {
    server.received_requests().await.expect("recording").len()
}

#[tokio::test]
async fn a_custom_http_proxy_carries_plain_http_with_credentials() {
    let origin = origin().await;
    let proxy = http_proxy(Some("Basic dTpwIHc=")).await;
    let url = format!("{}/ping", origin.uri());
    let r = resolve_in(
        &via("http", proxy.port, Some(("u", "p w")), ""),
        &url,
        ContextOptions::new(),
    );
    let res = direct().send(r.request()).await.expect("sent");
    assert_eq!((res.status(), res.text()), (200, "pong"));
    assert_eq!(
        proxy.seen.lines(),
        vec![
            format!("GET {url} HTTP/1.1"),
            "auth: Basic dTpwIHc=".to_owned()
        ]
    );
}

#[tokio::test]
async fn an_https_target_is_tunnelled_with_connect() {
    let server = tls_server(false).await;
    let proxy = http_proxy(None).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let r = resolve_in(
        &via("http", proxy.port, None, ""),
        &url,
        ContextOptions::new(),
    );
    let exec = Executor::new(ExecOptions::new().insecure(true)).expect("executor");
    let res = exec.send(r.request()).await.expect("sent");
    assert_eq!((res.status(), res.text()), (200, "ok"));
    assert_eq!(
        proxy.seen.lines(),
        vec![format!("CONNECT 127.0.0.1:{} HTTP/1.1", server.port)]
    );
}

#[tokio::test]
async fn socks5_carries_the_request_with_credentials() {
    let origin = origin().await;
    let proxy = socks5_proxy(Some(("u", "p"))).await;
    let url = format!("{}/ping", origin.uri());
    let r = resolve_in(
        &via("socks5", proxy.port, Some(("u", "p")), ""),
        &url,
        ContextOptions::new(),
    );
    let res = direct().send(r.request()).await.expect("sent");
    assert_eq!(res.text(), "pong");
    assert_eq!(
        proxy.seen.lines(),
        vec![
            "auth: u:p".to_owned(),
            format!("connect 127.0.0.1:{}", origin.address().port())
        ]
    );
}

#[tokio::test]
async fn the_bypass_list_sends_matching_hosts_directly() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let r = resolve_in(
        &via("http", proxy.port, None, "127.0.0.1"),
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    assert_eq!(
        direct().send(r.request()).await.expect("sent").text(),
        "pong"
    );
    assert!(proxy.seen.lines().is_empty());
    assert_eq!(received(&origin).await, 1);
}

#[tokio::test]
async fn a_redirect_hop_is_routed_afresh() {
    let last = origin().await;
    let first = MockServer::start().await;
    let next = format!("http://localhost:{}/ping", last.address().port());
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", next.as_str()))
        .mount(&first)
        .await;
    let proxy = http_proxy(None).await;
    let url = format!("{}/start", first.uri());
    let r = resolve_in(
        &via("http", proxy.port, None, "localhost"),
        &url,
        ContextOptions::new(),
    );
    assert_eq!(
        direct().send(r.request()).await.expect("sent").text(),
        "pong"
    );
    assert_eq!(proxy.seen.lines(), vec![format!("GET {url} HTTP/1.1")]);
    assert_eq!(received(&last).await, 1);
}

#[tokio::test]
async fn collection_modes_and_the_global_layer_combine() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let url = format!("{}/ping", origin.uri());
    let manual = GlobalProxy::Manual {
        proxy: ResolvedProxy::new("http", "127.0.0.1", &proxy.port.to_string()).expect("proxy"),
        bypass: String::new(),
    };
    let with_manual =
        Executor::new(ExecOptions::new().network(NetworkSettings::new().proxy(manual.clone())))
            .expect("executor");
    let inherit = resolve_in(PLAIN, &url, ContextOptions::new());
    with_manual
        .send(inherit.request())
        .await
        .expect("inherit uses the global proxy");
    assert_eq!(proxy.seen.lines().len(), 1);

    let off = resolve_in(
        &root_with(
            "  proxy:\n    inherit: false\n    config:\n      hostname: h\n    disabled: true\n",
        ),
        &url,
        ContextOptions::new(),
    );
    with_manual
        .send(off.request())
        .await
        .expect("off goes direct");
    direct()
        .send(inherit.request())
        .await
        .expect("no global layer goes direct");
    let custom = resolve_in(
        &via("http", proxy.port, None, ""),
        &url,
        ContextOptions::new(),
    );
    let forced = Executor::new(
        ExecOptions::new().network(NetworkSettings::new().proxy(manual).force_no_proxy(true)),
    )
    .expect("executor");
    forced
        .send(custom.request())
        .await
        .expect("--noproxy beats the collection");
    assert_eq!(proxy.seen.lines().len(), 1);
    assert_eq!(received(&origin).await, 4);
}

#[tokio::test]
async fn two_proxies_on_one_executor_never_share_a_tunnel() {
    let server = tls_server(false).await;
    let (one, two) = (http_proxy(None).await, http_proxy(None).await);
    let url = format!("https://127.0.0.1:{}/", server.port);
    let first = resolve_in(
        &via("http", one.port, None, ""),
        &url,
        ContextOptions::new(),
    );
    let second = resolve_in(
        &via("http", two.port, None, ""),
        &url,
        ContextOptions::new(),
    );
    let exec = Executor::new(ExecOptions::new().insecure(true)).expect("executor");
    for _ in 0..2 {
        assert_eq!(
            exec.send(first.request()).await.expect("first").text(),
            "ok"
        );
        assert_eq!(
            exec.send(second.request()).await.expect("second").text(),
            "ok"
        );
    }
    // Each proxy opened one tunnel and its client reused it; neither borrowed the other's.
    assert_eq!(one.seen.lines().len(), 1, "{:?}", one.seen.lines());
    assert_eq!(two.seen.lines().len(), 1, "{:?}", two.seen.lines());
    assert_eq!(server.connections(), 2);
}

#[tokio::test]
async fn a_proxied_hop_is_marked_via_proxy_with_a_connect_phase() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let manual = GlobalProxy::Manual {
        proxy: ResolvedProxy::new("http", "127.0.0.1", &proxy.port.to_string()).expect("proxy"),
        bypass: String::new(),
    };
    let exec = Executor::new(ExecOptions::new().network(NetworkSettings::new().proxy(manual)))
        .expect("executor");
    let r = resolve_in(
        PLAIN,
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    let res = exec.send(r.request()).await.expect("sent");
    let t = res.timeline()[0].timing.clone().expect("timing");
    assert!(t.via_proxy && t.connect.is_some() && !t.reused, "{t:?}");
}
