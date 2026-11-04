//! Fetching an import source from a URL, against local wiremock and raw TCP servers.

use crate::network_support::{http_proxy, pki, tls_server};
use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionOptions, FetchError, SourceFetch, default_import_name, fetch_source,
    parse_collections, write_collection,
};
use apinata_core::{ExecOptions, Executor, GlobalProxy, NetworkSettings, PacSource, ResolvedProxy};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn opts() -> SourceFetch {
    SourceFetch::default()
}

async fn serving(route: &str, response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(route))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn redirect(server: &MockServer, from: &str, to: &str) {
    Mock::given(path(from))
        .respond_with(ResponseTemplate::new(302).insert_header("location", to))
        .mount(server)
        .await;
}

#[test]
fn the_defaults_are_thirty_seconds_sixty_four_mebibytes_and_five_redirects() {
    let o = opts();
    assert_eq!(o.timeout, Duration::from_secs(30));
    assert_eq!(o.max_bytes, 64 << 20);
    assert_eq!(o.max_redirects, 5);
}

#[tokio::test]
async fn a_2xx_body_is_returned_as_bytes() {
    let server = serving(
        "/spec",
        ResponseTemplate::new(200).set_body_bytes(b"a\xffb".to_vec()),
    )
    .await;
    let got = fetch_source(&format!("{}/spec", server.uri()), &opts()).await;
    assert_eq!(got, Ok(b"a\xffb".to_vec()));
}

#[tokio::test]
async fn a_non_2xx_status_is_its_own_error() {
    let server = serving("/spec", ResponseTemplate::new(404)).await;
    let got = fetch_source(&format!("{}/spec", server.uri()), &opts()).await;
    assert_eq!(got, Err(FetchError::Status(404)));
}

#[tokio::test]
async fn a_body_announced_over_the_cap_is_refused() {
    let server = serving(
        "/spec",
        ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 100]),
    )
    .await;
    let mut o = opts();
    o.max_bytes = 10;
    let got = fetch_source(&format!("{}/spec", server.uri()), &o).await;
    assert_eq!(got, Err(FetchError::TooLarge { max_bytes: 10 }));
}

#[tokio::test]
async fn a_streamed_body_with_no_length_is_cut_at_the_cap() {
    // A chunked response announces no length, so only the streamed count can stop it.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let _ = socket.write_all(head.as_bytes()).await;
        for _ in 0..20 {
            if socket.write_all(b"8\r\nxxxxxxxx\r\n").await.is_err() {
                return;
            }
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    let mut o = opts();
    o.max_bytes = 30;
    let got = fetch_source(&format!("http://{addr}/spec"), &o).await;
    assert_eq!(got, Err(FetchError::TooLarge { max_bytes: 30 }));
}

#[tokio::test]
async fn a_slow_server_times_out() {
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(5));
    let server = serving("/spec", slow).await;
    let mut o = opts();
    o.timeout = Duration::from_millis(200);
    let got = fetch_source(&format!("{}/spec", server.uri()), &o).await;
    assert_eq!(got, Err(FetchError::Timeout));
}

#[tokio::test]
async fn redirects_are_followed_including_relative_ones() {
    let server = serving("/final", ResponseTemplate::new(200).set_body_string("ok")).await;
    redirect(&server, "/a", "/b").await;
    redirect(&server, "/b", &format!("{}/final", server.uri())).await;
    let got = fetch_source(&format!("{}/a", server.uri()), &opts()).await;
    assert_eq!(got, Ok(b"ok".to_vec()));
}

#[tokio::test]
async fn the_sixth_redirect_is_refused() {
    let server = serving("/p5", ResponseTemplate::new(200).set_body_string("ok")).await;
    for i in 0..5 {
        redirect(&server, &format!("/p{i}"), &format!("/p{}", i + 1)).await;
    }
    redirect(&server, "/q0", "/p0").await;
    // Five redirects reach the body; one more does not.
    let five = fetch_source(&format!("{}/p0", server.uri()), &opts()).await;
    assert_eq!(five, Ok(b"ok".to_vec()));
    let six = fetch_source(&format!("{}/q0", server.uri()), &opts()).await;
    assert_eq!(six, Err(FetchError::TooManyRedirects));
}

#[tokio::test]
async fn a_redirect_to_a_non_http_target_is_a_scheme_error() {
    let server = MockServer::start().await;
    redirect(&server, "/a", "ftp://example.test/spec").await;
    let got = fetch_source(&format!("{}/a", server.uri()), &opts()).await;
    assert!(matches!(got, Err(FetchError::Scheme(_))), "{got:?}");
}

#[tokio::test]
async fn a_redirect_target_with_credentials_is_refused_without_echoing_them() {
    let server = MockServer::start().await;
    redirect(&server, "/a", "http://user:s3cret@example.test/spec").await;
    let got = fetch_source(&format!("{}/a", server.uri()), &opts()).await;
    let Err(FetchError::Scheme(message)) = got else {
        panic!("{got:?}");
    };
    assert!(message.contains("user name or password"), "{message}");
    assert!(
        !message.contains("user:") && !message.contains("s3cret"),
        "{message}"
    );
}

// A 30x with no Location header has nowhere to go, so it is reported as a plain status error.
#[tokio::test]
async fn a_redirect_without_a_location_is_its_status() {
    let server = serving("/a", ResponseTemplate::new(302)).await;
    let got = fetch_source(&format!("{}/a", server.uri()), &opts()).await;
    assert_eq!(got, Err(FetchError::Status(302)));
}

#[tokio::test]
async fn the_host_policy_judges_each_hop_and_names_its_index() {
    let server = MockServer::start().await;
    redirect(&server, "/a", "http://elsewhere.test/spec").await;
    let o = opts().host_policy(|_| false);
    let got = fetch_source(&format!("{}/a", server.uri()), &o).await;
    assert_eq!(
        got,
        Err(FetchError::HostNotAllowed {
            host: "elsewhere.test".into(),
            hop: 1
        })
    );
}

#[tokio::test]
async fn the_host_policy_judges_the_given_url_as_hop_zero() {
    let o = opts().host_policy(|_| false);
    let got = fetch_source("http://elsewhere.test/spec", &o).await;
    assert_eq!(
        got,
        Err(FetchError::HostNotAllowed {
            host: "elsewhere.test".into(),
            hop: 0
        })
    );
}

#[tokio::test]
async fn loopback_bypasses_a_refusing_host_policy() {
    let server = serving("/spec", ResponseTemplate::new(200).set_body_string("ok")).await;
    let o = opts().host_policy(|_| false);
    let got = fetch_source(&format!("{}/spec", server.uri()), &o).await;
    assert_eq!(got, Ok(b"ok".to_vec()));
}

#[tokio::test]
async fn other_schemes_and_credentials_in_the_url_are_refused() {
    for url in [
        "ftp://example.test/spec",
        "file:///etc/passwd",
        "http://user:s3cret@127.0.0.1:1/spec",
        "http://user@127.0.0.1:1/spec",
        "not a url",
    ] {
        let got = fetch_source(url, &opts()).await;
        let Err(FetchError::Scheme(message)) = got else {
            panic!("{url}: {got:?}");
        };
        assert!(!message.contains("s3cret"), "{message}");
    }
}

#[tokio::test]
async fn no_cookie_or_authorization_header_is_sent() {
    let server = MockServer::start().await;
    Mock::given(path("/set"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "/spec")
                .insert_header("set-cookie", "sid=1"),
        )
        .mount(&server)
        .await;
    Mock::given(path("/spec"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    fetch_source(&format!("{}/set", server.uri()), &opts())
        .await
        .expect("fetches");
    let seen = server.received_requests().await.unwrap();
    assert_eq!(seen.len(), 2);
    for request in seen {
        assert!(!request.headers.contains_key("cookie"));
        assert!(!request.headers.contains_key("authorization"));
    }
}

const SPEC: &str = r#"{"openapi":"3.0.0","info":{"title":"Pets","version":"1"},
"servers":[{"url":"https://api.example.test"}],
"paths":{"/pets":{"get":{"tags":["pets"],"summary":"List pets","responses":{"200":{"description":"ok"}}}}}}"#;

/// Every file under `dir` as relative path to bytes.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(name, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn written(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let bundles = parse_collections(None, bytes, &CollectionOptions::default()).expect("parses");
    let parent = tempfile::TempDir::new().unwrap();
    for bundle in &bundles {
        write_collection(
            parent.path(),
            &bundle.collection,
            &bundle.environments,
            &MemoryStore::new(),
        )
        .expect("writes");
    }
    tree(parent.path())
}

#[tokio::test]
async fn a_fetched_spec_imports_the_same_files_as_the_saved_one() {
    let server = serving(
        "/openapi.json",
        ResponseTemplate::new(200).set_body_string(SPEC),
    )
    .await;
    let fetched = fetch_source(&format!("{}/openapi.json", server.uri()), &opts())
        .await
        .expect("fetches");
    let files = written(&fetched);
    assert!(!files.is_empty());
    assert_eq!(files, written(SPEC.as_bytes()));
}

#[test]
fn the_default_name_is_the_last_path_stem_else_the_host() {
    let table = [
        ("https://example.test/api/openapi.json", Some("openapi")),
        (
            "https://example.test/api/petstore.yaml?x=1#f",
            Some("petstore"),
        ),
        ("https://example.test/api/v1.2.json", Some("v1.2")),
        ("https://example.test/api/my%20api.json", Some("my api")),
        ("https://example.test/api/", Some("api")),
        ("https://example.test/spec", Some("spec")),
        ("https://example.test/", Some("example.test")),
        ("https://example.test", Some("example.test")),
        ("HTTP://Example.Test/a/b.json", Some("b")),
        ("not a url", None),
    ];
    for (url, want) in table {
        assert_eq!(default_import_name(url).as_deref(), want, "{url}");
    }
}

// Fetching through the shared network settings.

fn manual(port: u16, bypass: &str) -> GlobalProxy {
    GlobalProxy::Manual {
        proxy: ResolvedProxy::new("http", "127.0.0.1", &port.to_string()).expect("proxy"),
        bypass: bypass.into(),
    }
}

fn through(proxy: GlobalProxy) -> SourceFetch {
    SourceFetch::default().network(NetworkSettings::new().proxy(proxy))
}

async fn origin() -> MockServer {
    serving("/spec", ResponseTemplate::new(200).set_body_string("body")).await
}

#[tokio::test]
async fn a_manual_proxy_carries_the_fetch() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let url = format!("{}/spec", origin.uri());
    let got = fetch_source(&url, &through(manual(proxy.port, ""))).await;
    assert_eq!(got, Ok(b"body".to_vec()));
    assert_eq!(proxy.seen.lines(), vec![format!("GET {url} HTTP/1.1")]);
}

#[tokio::test]
async fn force_no_proxy_beats_a_manual_proxy() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let settings = NetworkSettings::new()
        .proxy(manual(proxy.port, ""))
        .force_no_proxy(true);
    let o = SourceFetch::default().network(settings);
    let got = fetch_source(&format!("{}/spec", origin.uri()), &o).await;
    assert_eq!(got, Ok(b"body".to_vec()));
    assert!(proxy.seen.lines().is_empty());
}

#[tokio::test]
async fn the_bypass_list_sends_a_named_host_direct() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let got = fetch_source(
        &format!("{}/spec", origin.uri()),
        &through(manual(proxy.port, "127.0.0.1")),
    )
    .await;
    assert_eq!(got, Ok(b"body".to_vec()));
    assert!(proxy.seen.lines().is_empty());
}

fn pac_for(body: &str) -> (tempfile::TempDir, GlobalProxy) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = dir.path().join("proxy.pac");
    std::fs::write(
        &file,
        format!("function FindProxyForURL(url, host) {{ {body} }}"),
    )
    .expect("pac");
    let source = PacSource::parse(file.to_str().expect("utf-8"), None).expect("source");
    (dir, GlobalProxy::Pac { source })
}

#[tokio::test]
async fn a_pac_script_routes_the_fetch() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let url = format!("{}/spec", origin.uri());
    let (_dir, pac) = pac_for(&format!("return \"PROXY 127.0.0.1:{}\";", proxy.port));
    let got = fetch_source(&url, &through(pac)).await;
    assert_eq!(got, Ok(b"body".to_vec()));
    assert_eq!(proxy.seen.lines(), vec![format!("GET {url} HTTP/1.1")]);
}

#[tokio::test]
async fn a_pac_script_answering_direct_sends_direct() {
    let origin = origin().await;
    let (_dir, pac) = pac_for("return \"DIRECT\";");
    let got = fetch_source(&format!("{}/spec", origin.uri()), &through(pac)).await;
    assert_eq!(got, Ok(b"body".to_vec()));
}

#[tokio::test]
async fn the_host_policy_judges_the_proxy_too() {
    let origin = origin().await;
    let proxy = GlobalProxy::Manual {
        proxy: ResolvedProxy::new("http", "192.0.2.10", "3128").expect("proxy"),
        bypass: String::new(),
    };
    let o = through(proxy).host_policy(|_| false);
    let got = tokio::time::timeout(
        Duration::from_secs(5),
        fetch_source(&format!("{}/spec", origin.uri()), &o),
    )
    .await
    .expect("decided without dialling");
    assert_eq!(
        got,
        Err(FetchError::HostNotAllowed {
            host: "192.0.2.10".into(),
            hop: 0
        })
    );
}

#[tokio::test]
async fn an_unknown_ca_fails_and_a_given_ca_succeeds() {
    let server = tls_server(false).await;
    let url = format!("https://127.0.0.1:{}/", server.port);
    let Err(FetchError::Transport(message)) = fetch_source(&url, &opts()).await else {
        panic!("the untrusted certificate must fail the fetch");
    };
    assert!(message.contains("UnknownIssuer"), "{message}");
    let trusting = NetworkSettings::new()
        .proxy(GlobalProxy::Off)
        .ca_file(Some(pki("ca.pem")));
    let o = SourceFetch::default().network(trusting.clone());
    assert_eq!(fetch_source(&url, &o).await, Ok(b"ok".to_vec()));
    let o = SourceFetch::default().network(trusting.keep_default_roots(false));
    assert_eq!(fetch_source(&url, &o).await, Ok(b"ok".to_vec()));
}

#[tokio::test]
async fn a_proxy_tunnels_the_fetch_to_a_tls_origin() {
    let server = tls_server(false).await;
    let proxy = http_proxy(None).await;
    let settings = NetworkSettings::new()
        .proxy(manual(proxy.port, ""))
        .ca_file(Some(pki("ca.pem")));
    let o = SourceFetch::default().network(settings);
    let got = fetch_source(&format!("https://127.0.0.1:{}/", server.port), &o).await;
    assert_eq!(got, Ok(b"ok".to_vec()));
    assert_eq!(
        proxy.seen.lines(),
        vec![format!("CONNECT 127.0.0.1:{} HTTP/1.1", server.port)]
    );
}

#[tokio::test]
async fn the_executor_hands_over_its_network_settings_and_insecure_flag() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let tls = tls_server(false).await;
    let exec = Executor::new(
        ExecOptions::new()
            .network(NetworkSettings::new().proxy(manual(proxy.port, "")))
            .insecure(true),
    )
    .expect("executor");
    let o = exec.source_fetch();
    assert!(o.insecure);
    let url = format!("{}/spec", origin.uri());
    assert_eq!(fetch_source(&url, &o).await, Ok(b"body".to_vec()));
    assert_eq!(proxy.seen.lines(), vec![format!("GET {url} HTTP/1.1")]);
    // Insecure carries over, so the unknown issuer is accepted.
    let got = fetch_source(&format!("https://127.0.0.1:{}/", tls.port), &o).await;
    assert_eq!(got, Ok(b"ok".to_vec()));
}

/// Runs only inside the child of the next test, where the environment names a proxy.
#[tokio::test]
#[ignore = "run by the parent test with a proxy in the environment"]
async fn child_default_fetch_with_proxy_env() {
    let origin = origin().await;
    let url = format!("{}/spec", origin.uri());
    let got = fetch_source(&url, &SourceFetch::default()).await;
    assert_eq!(got, Ok(b"body".to_vec()));
}

#[tokio::test]
async fn the_default_fetch_ignores_the_system_proxy_environment() {
    let proxy = http_proxy(None).await;
    let url = format!("http://127.0.0.1:{}", proxy.port);
    let out = std::process::Command::new(std::env::current_exe().expect("exe"))
        .args([
            "import_fetch::child_default_fetch_with_proxy_env",
            "--exact",
            "--ignored",
        ])
        .env("HTTP_PROXY", &url)
        .env("http_proxy", &url)
        .env("ALL_PROXY", &url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .expect("child");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(proxy.seen.lines().is_empty());
}
