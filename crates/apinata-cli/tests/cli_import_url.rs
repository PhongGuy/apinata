//! `apinata import collection <URL>`: a link is fetched through core, then imported like a file.
//! Servers are wiremock on 127.0.0.1; nothing leaves the machine.

use crate::common::apinata;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const OPENAPI: &str = r#"{"openapi":"3.0.0","info":{"title":"Pets","version":"1"},"servers":[{"url":"https://api.test"}],"paths":{"/pets":{"get":{"summary":"List pets","responses":{"200":{"description":"ok"}}}}}}"#;
const POSTMAN: &str = r#"{"info":{"name":"Shop","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[{"name":"Get user","request":{"method":"GET","url":"https://x.test/users/1"}}]}"#;

async fn serve(at: &str, body: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&s)
        .await;
    s
}

/// Every file under `dir` as (relative path, bytes), sorted.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push((
                    p.strip_prefix(base).unwrap().to_owned(),
                    fs::read(&p).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn import(source: &str, into: &Path, extra: &[&str]) -> std::process::Output {
    apinata()
        .args(["import", "collection", source, "--no-keychain", "--into"])
        .arg(into)
        .args(extra)
        .output()
        .unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_url_gives_the_same_files_as_the_saved_file() {
    let s = serve("/specs/petstore.json", OPENAPI).await;
    let saved = tempfile::tempdir().unwrap();
    let file = saved.path().join("petstore.json");
    fs::write(&file, OPENAPI).unwrap();
    let (from_url, from_file) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let url = format!("{}/specs/petstore.json", s.uri());
    let a = import(&url, from_url.path(), &[]);
    assert_eq!(a.status.code(), Some(0), "{}", stderr(&a));
    let b = import(file.to_str().unwrap(), from_file.path(), &[]);
    assert_eq!(b.status.code(), Some(0), "{}", stderr(&b));
    let (ta, tb) = (tree(from_url.path()), tree(from_file.path()));
    assert!(!ta.is_empty());
    assert_eq!(ta, tb);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_postman_url_is_detected_without_from() {
    let s = serve("/c/shop.json", POSTMAN).await;
    let into = tempfile::tempdir().unwrap();
    let out = import(&format!("{}/c/shop.json", s.uri()), into.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(into.path().join("Shop/Get user.yml").is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_url_is_recognised_case_insensitively() {
    let s = serve("/c/shop.json", POSTMAN).await;
    let into = tempfile::tempdir().unwrap();
    let url = s.uri().replacen("http://", "HTTP://", 1);
    let out = import(&format!("{url}/c/shop.json"), into.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn name_still_wins_over_the_default() {
    let har = r#"{"log":{"version":"1.2","entries":[{"request":{"method":"GET","url":"https://x.test/a","headers":[],"queryString":[]},"response":{"status":200}}]}}"#;
    let s = serve("/captures/session.har", har).await;
    let into = tempfile::tempdir().unwrap();
    let url = format!("{}/captures/session.har", s.uri());
    let out = import(&url, into.path(), &["--name", "Mine"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(into.path().join("Mine").is_dir());
    let into = tempfile::tempdir().unwrap();
    let out = import(&url, into.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(into.path().join("session").is_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_2xx_answer_names_the_status_and_writes_nothing() {
    let s = MockServer::start().await;
    Mock::given(path("/gone.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    let into = tempfile::tempdir().unwrap();
    let out = import(&format!("{}/gone.json", s.uri()), into.path(), &[]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("HTTP 404"), "{}", stderr(&out));
    assert!(tree(into.path()).is_empty());
}

#[test]
fn an_unreachable_port_is_a_fetch_failure() {
    // Bind then drop, so the port is known to be closed.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let into = tempfile::tempdir().unwrap();
    let out = import(&format!("http://127.0.0.1:{port}/x.json"), into.path(), &[]);
    assert_eq!(out.status.code(), Some(3));
    let text = stderr(&out);
    assert!(text.contains("cannot fetch"), "{text}");
    assert!(tree(into.path()).is_empty());
}

#[test]
fn other_schemes_are_refused() {
    let into = tempfile::tempdir().unwrap();
    for source in ["ftp://example.test/x.json", "file:///etc/hosts"] {
        let out = import(source, into.path(), &[]);
        assert_eq!(out.status.code(), Some(3), "{source}");
        let text = stderr(&out);
        assert!(text.contains("only http and https"), "{text}");
        assert!(tree(into.path()).is_empty());
    }
}

#[test]
fn a_relative_path_shaped_like_a_scheme_asks_for_the_full_path() {
    let into = tempfile::tempdir().unwrap();
    let out = import("a b://c", into.path(), &[]);
    assert_eq!(out.status.code(), Some(3));
    let text = stderr(&out);
    assert!(text.contains("full path of the file"), "{text}");
    assert!(!text.contains("only http and https"), "{text}");
    assert!(tree(into.path()).is_empty());
}

#[test]
fn a_bare_path_with_a_colon_still_reads_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a:b.json");
    fs::write(&file, POSTMAN).unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = import(file.to_str().unwrap(), into.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(into.path().join("Shop").is_dir());
}

#[test]
fn a_missing_file_still_says_it_cannot_read_it() {
    let into = tempfile::tempdir().unwrap();
    let out = import("no-such-file.json", into.path(), &[]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("cannot read"), "{}", stderr(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_by_still_applies_to_openapi_only() {
    let s = serve("/shop.json", POSTMAN).await;
    let into = tempfile::tempdir().unwrap();
    let out = import(
        &format!("{}/shop.json", s.uri()),
        into.path(),
        &["--group-by", "path"],
    );
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("--group-by"), "{}", stderr(&out));
}

#[test]
fn help_says_a_link_is_accepted() {
    let out = apinata()
        .args(["import", "collection", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(text.contains("http"), "{text}");
    assert!(text.contains("proxy"), "{text}");
    assert!(!text.contains("bruno"));
}

#[test]
fn a_url_with_credentials_is_refused_without_echoing_them() {
    let into = tempfile::tempdir().unwrap();
    let out = import("http://user:pw@127.0.0.1:9/x", into.path(), &[]);
    assert_eq!(out.status.code(), Some(3));
    let text = stderr(&out);
    assert!(text.contains("user name or password"), "{text}");
    assert!(!text.contains("user:") && !text.contains("pw@"), "{text}");
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert!(tree(into.path()).is_empty());
}

#[test]
fn a_transport_failure_names_neither_the_url_nor_the_port() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let into = tempfile::tempdir().unwrap();
    let out = import(
        &format!("http://127.0.0.1:{port}/secret-path.json"),
        into.path(),
        &[],
    );
    assert_eq!(out.status.code(), Some(3));
    let text = stderr(&out);
    assert!(!text.contains(&port.to_string()), "{text}");
    assert!(!text.contains("secret-path"), "{text}");
    assert!(!text.contains("127.0.0.1"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_to_another_scheme_is_refused() {
    let s = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "ftp://example.test/x"))
        .mount(&s)
        .await;
    let into = tempfile::tempdir().unwrap();
    let out = import(&format!("{}/a", s.uri()), into.path(), &[]);
    assert_eq!(out.status.code(), Some(3));
    assert!(
        stderr(&out).contains("only http and https"),
        "{}",
        stderr(&out)
    );
    assert!(tree(into.path()).is_empty());
}

// The network flags the link fetch shares with `run`.

/// A stand-in proxy on 127.0.0.1 that answers every request with the OpenAPI document and
/// records each request line, so a link that nobody serves still imports when it is proxied.
fn fake_proxy() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            log.lock()
                .expect("log")
                .push(head.split("\r\n").next().unwrap_or_default().to_owned());
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{OPENAPI}",
                OPENAPI.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (port, seen)
}

/// Nothing listens here, so only a proxy can answer a link to it.
const LINK: &str = "http://127.0.0.1:9/spec.json";

/// `import collection <source> <extra>` with HOME and XDG_* in a temp dir and `env` set.
fn import_with(
    source: &str,
    extra: &[&str],
    env: &[(&str, String)],
) -> (std::process::Output, tempfile::TempDir) {
    let (into, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut cmd = apinata();
    cmd.args(["import", "collection", source, "--no-keychain", "--into"])
        .arg(into.path())
        .args(extra)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("XDG_STATE_HOME", home.path().join("state"));
    for (name, value) in env {
        cmd.env(name, value);
    }
    (cmd.output().unwrap(), into)
}

#[test]
fn the_system_proxy_carries_the_link_and_noproxy_ignores_it() {
    let (port, seen) = fake_proxy();
    let env = [("HTTP_PROXY", format!("http://127.0.0.1:{port}"))];
    let (out, into) = import_with(LINK, &[], &env);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!tree(into.path()).is_empty());
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec![format!("GET {LINK} HTTP/1.1")]
    );
    seen.lock().unwrap().clear();
    let (out, _) = import_with(LINK, &["--noproxy"], &env);
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr(&out).contains("cannot fetch the link"),
        "{}",
        stderr(&out)
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_pac_file_routes_the_link() {
    let (port, seen) = fake_proxy();
    let dir = tempfile::tempdir().unwrap();
    let pac = dir.path().join("proxy.pac");
    fs::write(
        &pac,
        format!("function FindProxyForURL(url, host) {{ return \"PROXY 127.0.0.1:{port}\"; }}"),
    )
    .unwrap();
    let (out, into) = import_with(LINK, &["--proxy-pac", pac.to_str().unwrap()], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!tree(into.path()).is_empty());
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec![format!("GET {LINK} HTTP/1.1")]
    );
}

#[test]
fn a_missing_cacert_fails_an_https_link_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing-ca.pem");
    let (out, into) = import_with(
        "https://127.0.0.1:9/spec.json",
        &["--cacert", missing.to_str().unwrap()],
        &[],
    );
    assert_ne!(out.status.code(), Some(0));
    assert!(stderr(&out).contains("missing-ca.pem"), "{}", stderr(&out));
    assert!(tree(into.path()).is_empty());
}

#[test]
fn the_network_flags_are_checked_together() {
    let (out, _) = import_with(LINK, &["--ignore-truststore"], &[]);
    assert!(stderr(&out).contains("--cacert"), "{}", stderr(&out));
    let (out, _) = import_with(LINK, &["--noproxy", "--proxy-pac", "p.pac"], &[]);
    assert!(stderr(&out).contains("--noproxy"), "{}", stderr(&out));
    let (out, _) = import_with(LINK, &["--insecure", "--noproxy"], &[]);
    assert!(
        stderr(&out).contains("cannot fetch the link"),
        "{}",
        stderr(&out)
    );
}
