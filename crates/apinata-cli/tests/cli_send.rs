//! `apinata send`: one request from flags or a curl command, sent without a collection.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::prelude::PredicateBooleanExt as _;
use std::fs;
use tempfile::TempDir;
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn serving(text: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/hello"))
        .respond_with(ResponseTemplate::new(200).set_body_string(text))
        .mount(&s)
        .await;
    s
}

/// An empty working directory, so a stray write shows up as a file in it.
fn cwd() -> TempDir {
    TempDir::new().expect("tempdir")
}

fn send(dir: &TempDir) -> assert_cmd::Command {
    let mut cmd = apinata();
    cmd.current_dir(dir.path()).arg("send").arg("--no-keychain");
    cmd
}

fn is_empty(dir: &TempDir) -> bool {
    fs::read_dir(dir.path()).unwrap().next().is_none()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_get_prints_the_body_and_exits_0() {
    let s = serving("hi there").await;
    let dir = cwd();
    send(&dir)
        .arg(format!("{}/hello", s.uri()))
        .assert()
        .code(0)
        .stdout("hi there");
    assert!(is_empty(&dir));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn method_header_and_data_reach_the_server() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/items"))
        .and(header("x-token", "abc"))
        .and(body_string("name=widget"))
        .respond_with(ResponseTemplate::new(201).set_body_string("made"))
        .expect(1)
        .mount(&s)
        .await;
    let dir = cwd();
    send(&dir)
        .args(["-X", "POST", "-H", "x-token: abc", "-d", "name=widget"])
        .arg(format!("{}/items", s.uri()))
        .assert()
        .code(0)
        .stdout("made");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_curl_argument_is_sent() {
    let s = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(header("x-a", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_string("put"))
        .expect(1)
        .mount(&s)
        .await;
    let dir = cwd();
    send(&dir)
        .arg("--curl")
        .arg(format!("curl -X PUT -H 'x-a: 1' {}/z", s.uri()))
        .assert()
        .code(0)
        .stdout("put");
    assert!(is_empty(&dir));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_curl_command_on_stdin_is_sent() {
    let s = serving("from stdin").await;
    let dir = cwd();
    send(&dir)
        .args(["--curl", "-"])
        .write_stdin(format!("curl {}/hello", s.uri()))
        .assert()
        .code(0)
        .stdout("from stdin");
}

#[test]
fn curl_combined_with_a_url_or_request_flags_exits_3() {
    for extra in [
        vec!["http://127.0.0.1:1/x"],
        vec!["-X", "POST"],
        vec!["-H", "a: b"],
        vec!["-d", "x"],
    ] {
        let dir = cwd();
        send(&dir)
            .args(["--curl", "curl http://127.0.0.1:1/x"])
            .args(&extra)
            .assert()
            .code(3);
        assert!(is_empty(&dir));
    }
}

#[test]
fn a_bad_url_or_no_request_at_all_exits_3() {
    for url in ["not a url", "http://"] {
        let dir = cwd();
        send(&dir).arg(url).assert().code(3);
        assert!(is_empty(&dir));
    }
    send(&cwd()).assert().code(3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_var_fills_a_placeholder() {
    let s = serving("filled").await;
    let dir = cwd();
    send(&dir)
        .args(["--env-var", "who=hello"])
        .arg(format!("{}/{{{{who}}}}", s.uri()))
        .assert()
        .code(0)
        .stdout("filled");
}

#[test]
fn an_unresolved_placeholder_is_an_error_not_a_literal() {
    // As in `run`, an undefined variable is a request that errored: exit 2, nothing sent.
    let dir = cwd();
    send(&dir)
        .arg("http://127.0.0.1:1/{{nope}}")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("`nope` is not defined"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i_shows_the_status_line_and_headers() {
    let s = serving("body").await;
    let dir = cwd();
    let out = send(&dir)
        .arg("-i")
        .arg(format!("{}/hello", s.uri()))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.trim_end().ends_with("body"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o_writes_the_body_to_a_file() {
    let s = serving("saved body").await;
    let dir = cwd();
    let target = dir.path().join("out.bin");
    send(&dir)
        .arg("-o")
        .arg(&target)
        .arg(format!("{}/hello", s.uri()))
        .assert()
        .code(0);
    assert_eq!(fs::read_to_string(&target).unwrap(), "saved body");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_on_status_exits_1_like_run() {
    let s = MockServer::start().await;
    Mock::given(path("/hello"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    let dir = cwd();
    send(&dir)
        .arg("--fail-on-status")
        .arg(format!("{}/hello", s.uri()))
        .assert()
        .code(1);
}

#[test]
fn a_refused_connection_exits_2_like_run() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    send(&cwd())
        .arg(format!("http://127.0.0.1:{port}/x"))
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_writes_a_request_that_run_then_sends() {
    let s = serving("again").await;
    let coll = write_tree(&[("opencollection.yml", ROOT)]);
    let dir = cwd();
    let file = coll.path().join("hello.yml");
    send(&dir)
        .arg("--save")
        .arg(&file)
        .arg(format!("{}/hello", s.uri()))
        .assert()
        .code(0)
        .stdout("again");
    assert!(file.is_file());
    assert!(is_empty(&dir));
    apinata()
        .arg("run")
        .arg(&file)
        .args(["--body", "--no-keychain"])
        .assert()
        .code(0)
        .stdout("again");
}

#[test]
fn save_outside_a_collection_exits_3_and_sends_nothing() {
    let outside = cwd();
    let file = outside.path().join("hello.yml");
    send(&cwd())
        .arg("--save")
        .arg(&file)
        .arg("http://127.0.0.1:1/hello")
        .assert()
        .code(3);
    assert!(!file.exists());
}

#[test]
fn save_onto_an_existing_file_exits_3_and_leaves_it() {
    let coll = write_tree(&[("opencollection.yml", ROOT), ("hello.yml", "keep me")]);
    send(&cwd())
        .arg("--save")
        .arg(coll.path().join("hello.yml"))
        .arg("http://127.0.0.1:1/hello")
        .assert()
        .code(3);
    assert_eq!(
        fs::read_to_string(coll.path().join("hello.yml")).unwrap(),
        "keep me"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_from_a_file_is_read_relative_to_the_working_directory() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string("from a file"))
        .respond_with(ResponseTemplate::new(200).set_body_string("got it"))
        .expect(2)
        .mount(&s)
        .await;
    let dir = cwd();
    fs::write(dir.path().join("f.txt"), "from a file").unwrap();
    send(&dir)
        .args(["-d", "@f.txt"])
        .arg(format!("{}/up", s.uri()))
        .assert()
        .code(0)
        .stdout("got it");
    send(&dir)
        .args(["--curl"])
        .arg(format!("curl --data-binary @f.txt {}/up", s.uri()))
        .assert()
        .code(0)
        .stdout("got it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quotes_and_spaces_in_headers_and_data_arrive_intact() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("x-note", "it's \"quoted\" $HOME `x`"))
        .and(header("content-type", "text/plain"))
        .and(body_string("a='b c'&d=$(e)"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&s)
        .await;
    let dir = cwd();
    send(&dir)
        .args([
            "-H",
            "content-type: text/plain",
            "-H",
            "x-note: it's \"quoted\" $HOME `x`",
            "-d",
            "a='b c'&d=$(e)",
        ])
        .arg(format!("{}/q", s.uri()))
        .assert()
        .code(0)
        .stdout("ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flag_requests_print_no_curl_conversion_note() {
    let s = serving("x").await;
    let dir = cwd();
    send(&dir)
        .args(["-d", "plain text"])
        .arg(format!("{}/hello", s.uri()))
        .assert()
        .stderr(predicates::str::contains("curl flag").not());
}

#[test]
fn the_working_directory_stays_empty_on_every_error_path() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = cwd();
    send(&dir)
        .arg(format!("http://127.0.0.1:{port}/x"))
        .assert()
        .code(2);
    send(&dir)
        .arg("http://127.0.0.1:1/{{nope}}")
        .assert()
        .code(2);
    send(&dir).args(["--curl", "curl"]).assert().code(3);
    assert!(is_empty(&dir));
}

/// A server that expects `needle` somewhere in the body of a POST to /up, exactly `hits` times.
async fn expecting_body(needle: &str, hits: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/up"))
        .and(wiremock::matchers::body_string_contains(needle))
        .respond_with(ResponseTemplate::new(200).set_body_string("got it"))
        .expect(hits)
        .mount(&s)
        .await;
    s
}

/// Run a saved request from `elsewhere`, which holds nothing the request could point at.
fn run_saved(file: &std::path::Path, elsewhere: &TempDir) {
    apinata()
        .current_dir(elsewhere.path())
        .arg("run")
        .arg(file)
        .args(["--body", "--no-keychain"])
        .assert()
        .code(0)
        .stdout("got it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_keeps_a_body_file_inside_the_collection_relative() {
    let s = expecting_body("inside payload", 2).await;
    let coll = write_tree(&[
        ("opencollection.yml", ROOT),
        ("data/f.txt", "inside payload"),
    ]);
    let file = coll.path().join("up.yml");
    send(&coll)
        .args(["-d", "@data/f.txt", "--save"])
        .arg(&file)
        .arg(format!("{}/up", s.uri()))
        .assert()
        .code(0)
        .stdout("got it");
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("data/f.txt"), "{saved}");
    assert!(
        !saved.contains(coll.path().to_str().unwrap()),
        "an absolute path was saved: {saved}"
    );
    run_saved(&file, &cwd());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_keeps_a_body_file_outside_the_collection_absolute() {
    let s = expecting_body("outside payload", 2).await;
    let coll = write_tree(&[("opencollection.yml", ROOT)]);
    let dir = cwd();
    fs::write(dir.path().join("o.txt"), "outside payload").unwrap();
    let file = coll.path().join("up.yml");
    send(&dir)
        .args(["-d", "@o.txt", "--save"])
        .arg(&file)
        .arg(format!("{}/up", s.uri()))
        .assert()
        .code(0)
        .stdout("got it");
    let saved = fs::read_to_string(&file).unwrap();
    let absolute = dir.path().join("o.txt");
    assert!(saved.contains(absolute.to_str().unwrap()), "{saved}");
    run_saved(&file, &cwd());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multipart_file_part_is_sent_and_saved_relative() {
    let s = expecting_body("part payload", 2).await;
    let coll = write_tree(&[("opencollection.yml", ROOT), ("data/p.txt", "part payload")]);
    let file = coll.path().join("form.yml");
    send(&coll)
        .arg("--save")
        .arg(&file)
        .arg("--curl")
        .arg(format!("curl -F f=@data/p.txt {}/up", s.uri()))
        .assert()
        .code(0)
        .stdout("got it");
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("data/p.txt"), "{saved}");
    assert!(
        !saved.contains(coll.path().to_str().unwrap()),
        "an absolute path was saved: {saved}"
    );
    run_saved(&file, &cwd());
}

/// A server that expects every one of `needles` in the body of a POST to /up, `hits` times.
async fn expecting_parts(needles: &[&str], hits: u64) -> MockServer {
    let s = MockServer::start().await;
    let mut mock = Mock::given(method("POST")).and(path("/up"));
    for needle in needles {
        mock = mock.and(wiremock::matchers::body_string_contains(*needle));
    }
    mock.respond_with(ResponseTemplate::new(200).set_body_string("got it"))
        .expect(hits)
        .mount(&s)
        .await;
    s
}

// curl names one file per `-F`, so several files under one name arrive as repeated parts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_file_parts_under_one_name_are_all_saved_relative() {
    let s = expecting_parts(&["first payload", "second payload"], 2).await;
    let coll = write_tree(&[
        ("opencollection.yml", ROOT),
        ("data/a.txt", "first payload"),
        ("data/b.txt", "second payload"),
    ]);
    let file = coll.path().join("form.yml");
    send(&coll)
        .arg("--save")
        .arg(&file)
        .arg("--curl")
        .arg(format!(
            "curl -F f=@data/a.txt -F f=@data/b.txt {}/up",
            s.uri()
        ))
        .assert()
        .code(0)
        .stdout("got it");
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("data/a.txt"), "{saved}");
    assert!(saved.contains("data/b.txt"), "{saved}");
    assert!(
        !saved.contains(coll.path().to_str().unwrap()),
        "an absolute path was saved: {saved}"
    );
    run_saved(&file, &cwd());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_parts_keep_the_inside_file_relative_and_the_outside_one_absolute() {
    let s = expecting_parts(&["inside part", "outside part"], 2).await;
    let coll = write_tree(&[("opencollection.yml", ROOT), ("data/in.txt", "inside part")]);
    let away = cwd();
    let outside = away.path().join("out.txt");
    fs::write(&outside, "outside part").unwrap();
    let file = coll.path().join("form.yml");
    send(&coll)
        .arg("--save")
        .arg(&file)
        .arg("--curl")
        .arg(format!(
            "curl -F a=@data/in.txt -F b=@{} {}/up",
            outside.display(),
            s.uri()
        ))
        .assert()
        .code(0)
        .stdout("got it");
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("data/in.txt"), "{saved}");
    assert!(saved.contains(outside.to_str().unwrap()), "{saved}");
    assert!(
        !saved.contains(coll.path().to_str().unwrap()),
        "the inside file was saved absolute: {saved}"
    );
    run_saved(&file, &cwd());
}

// `--save` resolves the root through the link while the part names the link path, so only
// the canonical comparison can see the file is inside.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_root_reached_through_a_symlink_still_saves_parts_relative() {
    let s = expecting_body("linked payload", 2).await;
    let coll = write_tree(&[
        ("opencollection.yml", ROOT),
        ("data/p.txt", "linked payload"),
    ]);
    let links = cwd();
    let link = links.path().join("coll");
    std::os::unix::fs::symlink(coll.path(), &link).unwrap();
    let file = link.join("form.yml");
    send(&cwd())
        .arg("--save")
        .arg(&file)
        .arg("--curl")
        .arg(format!(
            "curl -F f=@{}/data/p.txt {}/up",
            link.display(),
            s.uri()
        ))
        .assert()
        .code(0)
        .stdout("got it");
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("data/p.txt"), "{saved}");
    for absolute in [&link, &coll.path().to_path_buf()] {
        assert!(
            !saved.contains(absolute.to_str().unwrap()),
            "an absolute path was saved: {saved}"
        );
    }
    run_saved(&file, &cwd());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zstd_reply_prints_the_decoded_body() {
    // `hello zstd` compressed by the `zstd` command line tool.
    let wire = [
        0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x68, 0x51, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20,
        0x7a, 0x73, 0x74, 0x64, 0xcf, 0xdb, 0x60, 0x9c,
    ];
    let s = MockServer::start().await;
    Mock::given(path("/zst"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "zstd")
                .set_body_raw(wire.to_vec(), "text/plain"),
        )
        .mount(&s)
        .await;
    let dir = cwd();
    send(&dir)
        .arg(format!("{}/zst", s.uri()))
        .assert()
        .code(0)
        .stdout("hello zstd");
}
