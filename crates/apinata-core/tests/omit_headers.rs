//! `settings.omitHeaders`: read and written as Bruno does, and honoured when the request is sent.

use crate::common;

use apinata_core::{RequestFile, default_headers};
use common::{executor, resolved_one};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const CORPUS: &str = "2026-10-19-omit-headers-parity";

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference")
        .join(CORPUS)
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    fixture: String,
    names: Vec<String>,
}

#[test]
fn omit_header_edits_match_bruno() {
    let cases: Vec<Case> =
        serde_json::from_str(&fs::read_to_string(corpus().join("cases.json")).unwrap()).unwrap();
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for case in &cases {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("r.yml");
        fs::copy(corpus().join("normalized").join(&case.fixture), &path).unwrap();
        let mut file = RequestFile::load(&path).unwrap();
        file.set_omitted_headers(case.names.clone());
        file.save().unwrap();
        let want =
            fs::read_to_string(corpus().join("expected").join(format!("{}.yml", case.id))).unwrap();
        let got = fs::read_to_string(&path).unwrap();
        if got != want {
            failures.push(format!("{}:\n--- got\n{got}--- want\n{want}", case.id));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn load(text: &str) -> (TempDir, PathBuf, RequestFile) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    fs::write(&path, text).unwrap();
    let file = RequestFile::load(&path).unwrap();
    (dir, path, file)
}

const HEAD: &str = "info:\n  name: r\n  type: http\n\nhttp:\n  method: GET\n  url: u\n";

#[test]
fn names_read_trimmed_and_non_strings_dropped() {
    let (_d, _p, file) = load(&format!(
        "{HEAD}\nsettings:\n  omitHeaders:\n    - \" User-Agent \"\n    - 5\n    - \"\"\n    - accept\n"
    ));
    assert_eq!(file.omitted_headers(), ["User-Agent", "accept"]);
    let (_d, _p, file) = load(HEAD);
    assert!(file.omitted_headers().is_empty());
}

#[test]
fn a_file_without_settings_gains_only_the_key_and_loses_the_block_again() {
    let (_d, path, mut file) = load(HEAD);
    file.set_omitted_headers(["User-Agent"]);
    file.save().unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{HEAD}\nsettings:\n  omitHeaders:\n    - User-Agent\n")
    );
    file.set_omitted_headers(Vec::<String>::new());
    file.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), HEAD);
}

#[test]
fn unchanged_names_leave_the_file_clean() {
    let text = format!("{HEAD}\nsettings:\n  omitHeaders:\n    - User-Agent\n");
    let (_d, _p, mut file) = load(&text);
    file.set_omitted_headers(["  User-Agent "]);
    assert!(!file.is_dirty());
}

#[test]
fn the_default_list_names_what_core_adds() {
    let list = default_headers();
    let pairs: Vec<(&str, &str)> = list
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("User-Agent", concat!("apinata/", env!("CARGO_PKG_VERSION"))),
            ("Accept", "application/json, text/plain, */*"),
            ("Accept-Encoding", "gzip,deflate,br"),
            ("Connection", "keep-alive"),
        ]
    );
}

const OMIT_UA: &str = "settings:\n  omitHeaders:\n    - User-Agent\n";

async fn sent_with(under_http: &str, top: &str) -> Vec<(String, String)> {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    executor()
        .send(&resolved_one("GET", &server.uri(), under_http, top))
        .await
        .expect("sent");
    let received = server.received_requests().await.expect("recorded");
    received[0]
        .headers
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_str().unwrap().to_owned()))
        .collect()
}

fn has(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
}

#[tokio::test]
async fn an_omitted_default_is_not_sent_and_the_other_is() {
    let sent = sent_with("", OMIT_UA).await;
    assert!(!has(&sent, "user-agent"), "{sent:?}");
    assert!(has(&sent, "accept"), "{sent:?}");
}

#[tokio::test]
async fn an_omitted_accept_falls_back_to_the_stacks_neutral_value() {
    // The HTTP stack always sends an Accept, so omitting ours leaves its `*/*`.
    let sent = sent_with(
        "",
        "settings:\n  omitHeaders:\n    - User-Agent\n    - Accept\n",
    )
    .await;
    let accept: Vec<_> = sent.iter().filter(|(n, _)| n == "accept").collect();
    assert_eq!(accept, [&("accept".to_owned(), "*/*".to_owned())]);
    assert!(!has(&sent, "user-agent"), "{sent:?}");
}

#[tokio::test]
async fn names_match_case_insensitively_and_trimmed() {
    let sent = sent_with(
        "",
        "settings:\n  omitHeaders:\n    - \" aCCEPT \"\n    - user-agent\n",
    )
    .await;
    assert!(!has(&sent, "user-agent"), "{sent:?}");
    assert!(
        sent.contains(&("accept".to_owned(), "*/*".to_owned())),
        "{sent:?}"
    );
}

#[tokio::test]
async fn a_header_the_user_set_wins_over_omitting_it() {
    let own = "  headers:\n    - name: User-Agent\n      value: mine/1\n";
    let sent = sent_with(own, OMIT_UA).await;
    let agents: Vec<_> = sent.iter().filter(|(n, _)| n == "user-agent").collect();
    assert_eq!(agents, [&("user-agent".to_owned(), "mine/1".to_owned())]);
}

#[tokio::test]
async fn host_is_never_omitted() {
    let sent = sent_with("", "settings:\n  omitHeaders:\n    - Host\n").await;
    assert!(has(&sent, "host"), "{sent:?}");
    // Host is not one of the defaults core adds, so the defaults are untouched too.
    assert!(has(&sent, "user-agent") && has(&sent, "accept"), "{sent:?}");
}

#[tokio::test]
async fn the_recorded_request_headers_match_the_wire() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let res = executor()
        .send(&resolved_one("GET", &server.uri(), "", OMIT_UA))
        .await
        .expect("sent");
    let hop = &res.timeline()[0];
    let names: Vec<_> = hop
        .request_headers
        .iter()
        .map(|(n, _)| n.to_ascii_lowercase())
        .collect();
    assert!(names.contains(&"accept".to_owned()), "{names:?}");
    assert!(!names.contains(&"user-agent".to_owned()), "{names:?}");
}

#[tokio::test]
async fn the_resolved_request_exposes_its_omitted_names() {
    let req = resolved_one("GET", "http://x.test", "", OMIT_UA);
    assert_eq!(req.omitted_headers(), ["User-Agent"]);
}
