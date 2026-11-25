//! HTTP over a Unix socket: `http://unix:/path/to.sock:/resource` sends over the socket file,
//! the host policy is asked about `unix:/path`, redirects and curl export keep the socket.

use crate::common;
use crate::network_support;

use apinata_core::{
    ContextOptions, ExecError, ExecOptions, Executor, GlobalProxy, NetworkSettings, PacSource,
    ResolveError, normalize_host, to_curl,
};
use common::{executor, resolved_one};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use network_support::{custom_proxy, resolve_in, root_with};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::net::UnixListener;
use wiremock::matchers::path as at_path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A hyper server on a socket in `dir`, named `name`. `/start` sets a cookie and redirects
/// to `location`; anything else answers with its path and query, the Host header, and any
/// Authorization or Cookie it was sent.
fn serve(dir: &Path, name: &str, location: Option<&str>) -> PathBuf {
    let path = dir.join(name);
    let listener = UnixListener::bind(&path).expect("bind");
    let location = location.map(str::to_owned);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let location = location.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req: hyper::Request<Incoming>| {
                    let location = location.clone();
                    async move {
                        let header = |name: &str| {
                            req.headers()
                                .get(name)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned)
                        };
                        let reply = match (req.uri().path(), location) {
                            ("/start", Some(to)) => hyper::Response::builder()
                                .status(302)
                                .header("location", to)
                                .header("set-cookie", "sid=1; Path=/")
                                .body(String::new()),
                            _ => {
                                let mut text = format!(
                                    "{} host={}",
                                    req.uri(),
                                    header("host").unwrap_or_default()
                                );
                                for name in ["authorization", "cookie"] {
                                    if let Some(value) = header(name) {
                                        text.push_str(&format!(" {name}={value}"));
                                    }
                                }
                                hyper::Response::builder().status(200).body(text)
                            }
                        };
                        Ok::<_, Infallible>(reply.expect("response"))
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    path
}

fn socket_url(socket: &Path, rest: &str) -> String {
    format!("http://unix:{}:{rest}", socket.display())
}

/// An executor whose policy trusts exactly `trusted` and records every host it is asked about.
fn guarded(trusted: Vec<String>) -> (Executor, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let policy = move |host: &str| {
        log.lock().unwrap().push(host.to_owned());
        trusted.iter().any(|t| t == host)
    };
    let exec = Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client");
    (exec, asked)
}

#[tokio::test]
async fn a_get_is_sent_over_the_socket_with_its_path_and_query() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", None);
    let url = socket_url(&socket, "/v1.47/info?all=1");
    let res = executor()
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("sent over the socket");
    assert_eq!(res.status(), 200);
    assert_eq!(res.text(), "/v1.47/info?all=1 host=localhost");
}

#[tokio::test]
async fn a_percent_encoded_socket_path_is_decoded() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("a:b");
    std::fs::create_dir(&sub).unwrap();
    let socket = serve(&sub, "d.sock", None);
    let encoded = socket.display().to_string().replace(':', "%3A");
    let url = format!("http://unix:{encoded}:/ping");
    let res = executor()
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("sent over the socket");
    assert_eq!(res.text(), "/ping host=localhost");
}

#[tokio::test]
async fn a_socket_url_without_a_request_path_is_refused() {
    let err = executor()
        .send(&resolved_one(
            "GET",
            "http://unix:/tmp/none.sock:info",
            "",
            "",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. } if message.contains("Unix socket")),
        "{err:?}"
    );
}

#[tokio::test]
async fn https_over_a_socket_is_refused_with_a_clear_error() {
    let err = executor()
        .send(&resolved_one(
            "GET",
            "https://unix:/tmp/none.sock:/info",
            "",
            "",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. } if message.contains("HTTPS")),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_untrusted_socket_is_refused_and_named_to_the_policy() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", None);
    let (exec, asked) = guarded(Vec::new());
    let err = exec
        .send(&resolved_one("GET", &socket_url(&socket, "/x"), "", ""))
        .await
        .unwrap_err();
    let key = format!("unix:{}", socket.display());
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 0, host, .. } if *host == key),
        "{err:?}"
    );
    assert_eq!(*asked.lock().unwrap(), [key]);
}

#[tokio::test]
async fn a_trusted_socket_is_sent_to() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", None);
    let (exec, _) = guarded(vec![format!("unix:{}", socket.display())]);
    let res = exec
        .send(&resolved_one("GET", &socket_url(&socket, "/x"), "", ""))
        .await
        .expect("trusted");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn a_relative_redirect_stays_on_the_socket() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", Some("/final?n=2"));
    let res = executor()
        .send(&resolved_one(
            "GET",
            &socket_url(&socket, "/start"),
            AUTH,
            "",
        ))
        .await
        .expect("followed");
    // The same socket keeps the credentials, and its cookie is neither kept nor sent.
    assert_eq!(
        res.text(),
        "/final?n=2 host=localhost authorization=Bearer t"
    );
    // The URL parser drops the empty port, so the final URL reads `http://unix/...`.
    assert_eq!(
        res.url(),
        format!("http://unix{}:/final?n=2", socket.display())
    );
}

#[tokio::test]
async fn an_absolute_redirect_is_judged_as_usual() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", Some("http://elsewhere.test/x"));
    let (exec, asked) = guarded(vec![format!("unix:{}", socket.display())]);
    let err = exec
        .send(&resolved_one("GET", &socket_url(&socket, "/start"), "", ""))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if host == "elsewhere.test"),
        "{err:?}"
    );
    assert!(asked.lock().unwrap().contains(&"elsewhere.test".to_owned()));
}

#[tokio::test]
async fn a_protocol_relative_redirect_leaves_the_socket() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", Some("//elsewhere.test/x"));
    let (exec, _) = guarded(vec![format!("unix:{}", socket.display())]);
    let err = exec
        .send(&resolved_one("GET", &socket_url(&socket, "/start"), "", ""))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if host == "elsewhere.test"),
        "{err:?}"
    );
}

#[test]
fn copy_as_curl_names_the_socket() {
    let r = resolved_one(
        "GET",
        "http://unix:/var/run/docker.sock:/v1.47/info?all=1",
        "",
        "",
    );
    assert_eq!(
        to_curl(&r),
        [
            "curl --request 'GET'",
            "--unix-socket '/var/run/docker.sock'",
            "--url 'http://localhost/v1.47/info?all=1'",
        ]
        .join(" \\\n  ")
    );
}

#[test]
fn a_socket_trust_key_keeps_its_path() {
    assert_eq!(
        normalize_host("unix:/var/run/docker.sock").as_deref(),
        Some("unix:/var/run/docker.sock")
    );
    // A plain host named unix stays a host, so trusting it never trusts a socket.
    assert_eq!(normalize_host("unix").as_deref(), Some("unix"));
}

const AUTH: &str = "  headers:\n    - name: Authorization\n      value: \"Bearer t\"\n";

#[tokio::test]
async fn a_tcp_redirect_into_a_socket_is_judged_by_the_socket() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", None);
    let origin = MockServer::start().await;
    Mock::given(at_path("/go"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", socket_url(&socket, "/x")),
        )
        .mount(&origin)
        .await;
    let (exec, _) = guarded(Vec::new());
    let err = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/go", origin.uri()),
            "",
            "",
        ))
        .await
        .unwrap_err();
    let key = format!("unix:{}", socket.display());
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if *host == key),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_redirect_to_another_socket_is_judged_by_that_socket() {
    let dir = TempDir::new().unwrap();
    let b = dir.path().join("b.sock");
    let a = serve(dir.path(), "a.sock", Some(&socket_url(&b, "/x")));
    let (exec, _) = guarded(vec![format!("unix:{}", a.display())]);
    let err = exec
        .send(&resolved_one("GET", &socket_url(&a, "/start"), "", ""))
        .await
        .unwrap_err();
    let key = format!("unix:{}", b.display());
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if *host == key),
        "{err:?}"
    );
}

#[tokio::test]
async fn credentials_are_dropped_on_a_redirect_to_another_socket() {
    // Another socket is another origin, so with forwarding off the credentials stay behind.
    let dir = TempDir::new().unwrap();
    let b = serve(dir.path(), "b.sock", None);
    let a = serve(dir.path(), "a.sock", Some(&socket_url(&b, "/x")));
    let res = executor()
        .send(&resolved_one(
            "GET",
            &socket_url(&a, "/start"),
            AUTH,
            "settings:\n  forwardAuthorizationHeader: false\n",
        ))
        .await
        .expect("followed");
    assert_eq!(res.text(), "/x host=localhost");
}

#[tokio::test]
async fn a_collection_proxy_is_skipped_for_a_socket() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", None);
    // Nothing listens on the proxy; the policy would also refuse it if it were asked.
    let root = root_with(&custom_proxy("http", "192.0.2.1", "9", None, ""));
    let r = resolve_in(&root, &socket_url(&socket, "/x"), ContextOptions::new());
    let (exec, asked) = guarded(vec![format!("unix:{}", socket.display())]);
    let res = exec.send(r.request()).await.expect("sent directly");
    assert_eq!(res.status(), 200);
    assert!(!res.timeline()[0].via_proxy());
    assert_eq!(
        *asked.lock().unwrap(),
        [format!("unix:{}", socket.display())]
    );
}

#[tokio::test]
async fn a_pac_script_is_not_asked_about_a_socket() {
    let dir = TempDir::new().unwrap();
    let socket = serve(dir.path(), "d.sock", None);
    let pac = dir.path().join("proxy.pac");
    std::fs::write(
        &pac,
        "function FindProxyForURL(url, host) { return \"PROXY 192.0.2.1:9\"; }",
    )
    .unwrap();
    let source = PacSource::parse(pac.to_str().unwrap(), None).expect("source");
    let exec = Executor::new(
        ExecOptions::new().network(NetworkSettings::new().proxy(GlobalProxy::Pac { source })),
    )
    .expect("executor");
    let res = exec
        .send(&resolved_one("GET", &socket_url(&socket, "/x"), "", ""))
        .await
        .expect("sent directly");
    assert_eq!(res.status(), 200);
    assert_eq!(res.timeline()[0].pac, None);
}

#[tokio::test]
async fn a_socket_path_with_trailing_space_is_refused() {
    let err = executor()
        .send(&resolved_one(
            "GET",
            "http://unix:/run/app.sock%20:/x",
            "",
            "",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. } if message.contains("spaces")),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_explicit_port_on_the_socket_host_is_refused() {
    // The parser drops a default port, so `:80` is refused while the written URL is resolved.
    let r = resolve_in(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        "http://unix:80/run/a.sock:/x",
        ContextOptions::new(),
    );
    assert!(
        matches!(&r.result, Err(ResolveError::InvalidUrl { .. })),
        "{:?}",
        r.result
    );
    let err = executor()
        .send(&resolved_one(
            "GET",
            "http://unix:8080/run/a.sock:/x",
            "",
            "",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. } if message.contains("no port")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_socket_url_survives_encode_url() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("a:b");
    std::fs::create_dir(&sub).unwrap();
    let socket = serve(&sub, "d.sock", None);
    let encoded = socket.display().to_string().replace(':', "%3A");
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("http://unix:{encoded}:/v1/a b"),
            "",
            "settings:\n  encodeUrl: true\n",
        ))
        .await
        .expect("sent over the socket");
    assert_eq!(res.text(), "/v1/a%20b host=localhost");
}
