//! HTTP over a Unix socket: `http://unix:/path/to.sock:/resource` sends over the socket file,
//! the host policy is asked about `unix:/path`, redirects and curl export keep the socket.

use crate::common;

use apinata_core::{ExecError, ExecOptions, Executor, normalize_host, to_curl};
use common::{executor, resolved_one};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::net::UnixListener;

/// A hyper server on a socket in `dir`, named `name`. `/start` redirects to `location`;
/// anything else answers with its path and query and the Host header it was sent.
fn serve(dir: &Path, name: &str, location: Option<&'static str>) -> PathBuf {
    let path = dir.join(name);
    let listener = UnixListener::bind(&path).expect("bind");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let service = service_fn(move |req: hyper::Request<Incoming>| async move {
                    let reply = match (req.uri().path(), location) {
                        ("/start", Some(to)) => hyper::Response::builder()
                            .status(302)
                            .header("location", to)
                            .body(String::new()),
                        _ => {
                            let host = req
                                .headers()
                                .get("host")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_owned();
                            hyper::Response::builder()
                                .status(200)
                                .body(format!("{} host={host}", req.uri()))
                        }
                    };
                    Ok::<_, Infallible>(reply.expect("response"))
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
    let sub = dir.path().join("my dir");
    std::fs::create_dir(&sub).unwrap();
    let socket = serve(&sub, "d.sock", None);
    let encoded = socket.display().to_string().replace(' ', "%20");
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
        .send(&resolved_one("GET", &socket_url(&socket, "/start"), "", ""))
        .await
        .expect("followed");
    assert_eq!(res.text(), "/final?n=2 host=localhost");
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
