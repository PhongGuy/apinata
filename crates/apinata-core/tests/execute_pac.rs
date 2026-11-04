//! PAC routing in the executor: each hop asks the script, the collection and `--noproxy` still
//! win, failures go direct, and a host policy judges the target, then the chosen proxy.

use crate::network_support;

use std::sync::Arc;
use std::time::Duration;

use apinata_core::{
    ContextOptions, ExecError, ExecOptions, Executor, GlobalProxy, NetworkSettings, PacSource,
};
use network_support::{custom_proxy, http_proxy, pki, resolve_in, root_with, tls_server};
use tempfile::TempDir;
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

/// A PAC file in its own directory, so each test has its own cache key.
fn pac_file(body: &str) -> (TempDir, PacSource) {
    let dir = TempDir::new().expect("tempdir");
    let file = dir.path().join("proxy.pac");
    std::fs::write(&file, body).expect("pac");
    let source = PacSource::parse(file.to_str().expect("utf-8 path"), None).expect("source");
    (dir, source)
}

fn find(body: &str) -> String {
    format!("function FindProxyForURL(url, host) {{ {body} }}")
}

fn settings(source: PacSource) -> NetworkSettings {
    NetworkSettings::new().proxy(GlobalProxy::Pac { source })
}

fn pac_exec(source: PacSource) -> Executor {
    Executor::new(ExecOptions::new().network(settings(source))).expect("executor")
}

fn pac_lines(res: &apinata_core::Response) -> Vec<Option<String>> {
    res.timeline().iter().map(|hop| hop.pac.clone()).collect()
}

#[tokio::test]
async fn the_script_routes_a_hop_through_the_proxy_it_names() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let url = format!("{}/ping?q=1", origin.uri());
    let (_dir, source) = pac_file(&find(&format!(
        "return url === {url:?} && host === \"127.0.0.1\" ? \"PROXY 127.0.0.1:{}\" : \"DIRECT\";",
        proxy.port
    )));
    let r = resolve_in(PLAIN, &url, ContextOptions::new());
    let res = pac_exec(source).send(r.request()).await.expect("sent");
    assert_eq!(res.text(), "pong");
    assert_eq!(proxy.seen.lines(), vec![format!("GET {url} HTTP/1.1")]);
    assert_eq!(
        pac_lines(&res),
        [Some(format!("http://127.0.0.1:{}", proxy.port))]
    );
}

#[tokio::test]
async fn a_redirect_hop_asks_the_script_again_and_uses_its_own_proxy() {
    let last = origin().await;
    let first = MockServer::start().await;
    let next = format!("http://localhost:{}/ping", last.address().port());
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", next.as_str()))
        .mount(&first)
        .await;
    let (one, two) = (http_proxy(None).await, http_proxy(None).await);
    let (_dir, source) = pac_file(&find(&format!(
        "return host === \"localhost\" ? \"PROXY 127.0.0.1:{}\" : \"PROXY 127.0.0.1:{}\";",
        two.port, one.port
    )));
    let url = format!("{}/start", first.uri());
    let r = resolve_in(PLAIN, &url, ContextOptions::new());
    let res = pac_exec(source).send(r.request()).await.expect("sent");
    assert_eq!(res.text(), "pong");
    assert_eq!(one.seen.lines(), vec![format!("GET {url} HTTP/1.1")]);
    assert_eq!(two.seen.lines(), vec![format!("GET {next} HTTP/1.1")]);
    assert_eq!(
        pac_lines(&res),
        [
            Some(format!("http://127.0.0.1:{}", one.port)),
            Some(format!("http://127.0.0.1:{}", two.port))
        ]
    );
}

#[tokio::test]
async fn a_direct_answer_connects_directly() {
    let origin = origin().await;
    let (_dir, source) = pac_file(&find("return \"DIRECT\";"));
    let r = resolve_in(
        PLAIN,
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    let res = pac_exec(source).send(r.request()).await.expect("sent");
    assert_eq!(res.text(), "pong");
    assert_eq!(pac_lines(&res), [Some("DIRECT".to_owned())]);
}

#[tokio::test]
async fn the_collection_and_noproxy_override_pac() {
    let origin = origin().await;
    let (named, own) = (http_proxy(None).await, http_proxy(None).await);
    let (_dir, source) = pac_file(&find(&format!(
        "return \"PROXY 127.0.0.1:{}\";",
        named.port
    )));
    let url = format!("{}/ping", origin.uri());
    let exec = pac_exec(source.clone());
    let custom = resolve_in(
        &root_with(&custom_proxy(
            "http",
            "127.0.0.1",
            &own.port.to_string(),
            None,
            "",
        )),
        &url,
        ContextOptions::new(),
    );
    let res = exec.send(custom.request()).await.expect("custom");
    assert_eq!(pac_lines(&res), [None]);
    let off = resolve_in(
        &root_with(
            "  proxy:\n    inherit: false\n    config:\n      hostname: h\n    disabled: true\n",
        ),
        &url,
        ContextOptions::new(),
    );
    let res = exec.send(off.request()).await.expect("off");
    assert_eq!(pac_lines(&res), [None]);
    let forced = Executor::new(ExecOptions::new().network(settings(source).force_no_proxy(true)))
        .expect("executor");
    let inherit = resolve_in(PLAIN, &url, ContextOptions::new());
    let res = forced.send(inherit.request()).await.expect("forced");
    assert_eq!(pac_lines(&res), [None]);
    assert!(named.seen.lines().is_empty());
    assert_eq!(own.seen.lines().len(), 1);
    assert_eq!(
        origin.received_requests().await.expect("recording").len(),
        3
    );
}

#[tokio::test]
async fn a_failing_script_sends_every_hop_directly() {
    let server = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/ping"))
        .mount(&server)
        .await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    let (_dir, source) = pac_file(&find("throw new Error(\"boom\");"));
    let r = resolve_in(
        PLAIN,
        &format!("{}/start", server.uri()),
        ContextOptions::new(),
    );
    let res = pac_exec(source)
        .send(r.request())
        .await
        .expect("sent directly");
    assert_eq!(res.text(), "pong");
    let failed = Some("script failed: boom".to_owned());
    assert_eq!(pac_lines(&res), [failed.clone(), failed]);
}

#[tokio::test]
async fn a_missing_pac_file_sends_directly() {
    let origin = origin().await;
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("absent.pac");
    let source = PacSource::parse(missing.to_str().expect("utf-8"), None).expect("source");
    let r = resolve_in(
        PLAIN,
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    let res = pac_exec(source)
        .send(r.request())
        .await
        .expect("sent directly");
    assert_eq!(res.text(), "pong");
    let line = res.timeline()[0].pac.clone().unwrap_or_default();
    assert!(
        line.starts_with("could not be fetched: ") && line.contains("absent.pac"),
        "{line}"
    );
}

#[tokio::test]
async fn the_pac_fetch_uses_the_executors_tls_settings() {
    let server = tls_server(false).await;
    let source = PacSource::parse(&format!("https://127.0.0.1:{}/tls.pac", server.port), None)
        .expect("source");
    let origin = origin().await;
    let r = resolve_in(
        PLAIN,
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    // The default roots refuse the test CA, so the fetch fails; failures are not cached.
    let res = pac_exec(source.clone())
        .send(r.request())
        .await
        .expect("direct");
    let line = res.timeline()[0].pac.clone().unwrap_or_default();
    assert!(line.starts_with("could not be fetched: "), "{line}");
    // Trusting the CA, the body `ok` arrives; it is not a script, so evaluation fails instead.
    let trusting =
        Executor::new(ExecOptions::new().network(settings(source).ca_file(Some(pki("ca.pem")))))
            .expect("executor");
    let res = trusting.send(r.request()).await.expect("direct");
    let line = res.timeline()[0].pac.clone().unwrap_or_default();
    assert!(line.starts_with("script failed: "), "{line}");
}

#[tokio::test]
async fn the_target_is_judged_before_any_pac_fetch() {
    let pac_server = MockServer::start().await;
    Mock::given(path("/proxy.pac"))
        .respond_with(ResponseTemplate::new(200).set_body_string(find("return \"DIRECT\";")))
        .mount(&pac_server)
        .await;
    let source =
        PacSource::parse(&format!("{}/proxy.pac", pac_server.uri()), None).expect("source");
    let exec = Executor::new(
        ExecOptions::new()
            .network(settings(source))
            .host_policy(Arc::new(|_: &str| false)),
    )
    .expect("executor");
    let r = resolve_in(PLAIN, "http://api.example.test/ping", ContextOptions::new());
    let err = exec.send(r.request()).await.expect_err("refused");
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 0, host, .. } if host == "api.example.test"),
        "{err:?}"
    );
    assert!(
        pac_server
            .received_requests()
            .await
            .expect("recording")
            .is_empty()
    );
}

#[tokio::test]
async fn a_pac_chosen_socks_route_is_judged_by_the_host_policy() {
    let origin = origin().await;
    for (answer, expected) in [
        (
            "SOCKS5H proxy.corp.example:1080",
            "socks5h://proxy.corp.example:1080",
        ),
        (
            "SOCKS4A proxy.corp.example:1080",
            "socks4a://proxy.corp.example:1080",
        ),
    ] {
        let (_dir, source) = pac_file(&find(&format!("return \"{answer}\";")));
        let exec = Executor::new(
            ExecOptions::new()
                .network(settings(source))
                .host_policy(Arc::new(|_: &str| false)),
        )
        .expect("executor");
        let r = resolve_in(
            PLAIN,
            &format!("{}/ping", origin.uri()),
            ContextOptions::new(),
        );
        let err = tokio::time::timeout(Duration::from_secs(5), exec.send(r.request()))
            .await
            .expect("decided without dialling")
            .expect_err("refused");
        assert!(
            matches!(&err, ExecError::ProxyNotAllowed { hop: 0, host, proxy, .. }
                if host == "proxy.corp.example" && proxy == expected),
            "{answer}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_pac_chosen_proxy_is_judged_by_the_host_policy() {
    let origin = origin().await;
    let (_dir, source) = pac_file(&find("return \"PROXY 192.0.2.10:3128\";"));
    let exec = Executor::new(
        ExecOptions::new()
            .network(settings(source))
            .host_policy(Arc::new(|_: &str| false)),
    )
    .expect("executor");
    let r = resolve_in(
        PLAIN,
        &format!("{}/ping", origin.uri()),
        ContextOptions::new(),
    );
    let err = tokio::time::timeout(Duration::from_secs(5), exec.send(r.request()))
        .await
        .expect("decided without dialling")
        .expect_err("refused");
    assert!(
        matches!(&err, ExecError::ProxyNotAllowed { hop: 0, host, proxy, .. }
            if host == "192.0.2.10" && proxy == "http://192.0.2.10:3128"),
        "{err:?}"
    );
}
