//! `apinata network hosts`: list, set and remove a collection's host overrides, and a run that
//! sends through one.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use std::fs;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn hosts(dir: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    apinata()
        .args(["network", "hosts"])
        .args(args)
        .arg("--collection")
        .arg(dir)
        .assert()
}

fn root_text(dir: &std::path::Path) -> String {
    fs::read_to_string(dir.join("opencollection.yml")).expect("root")
}

#[test]
fn set_adds_a_row_and_list_shows_it() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    hosts(dir.path(), &["set", "API.Fake.test", "127.0.0.1:8080"])
        .code(0)
        .stdout(contains("api.fake.test -> 127.0.0.1:8080"));
    let text = root_text(dir.path());
    assert!(text.contains("hostOverrides"), "{text}");
    assert!(text.contains("host: api.fake.test"), "{text}");
    let out = hosts(dir.path(), &["list"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "api.fake.test -> 127.0.0.1:8080\n"
    );
}

#[test]
fn set_replaces_the_row_of_the_same_host_and_can_disable_it() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    hosts(dir.path(), &["set", "a.test", "127.0.0.1"]).code(0);
    hosts(dir.path(), &["set", "A.test", "127.0.0.2", "--disabled"]).code(0);
    let out = hosts(dir.path(), &["list"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "a.test -> 127.0.0.2 (disabled)\n"
    );
}

#[test]
fn remove_deletes_the_row_and_the_emptied_sections() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    hosts(dir.path(), &["set", "a.test", "127.0.0.1"]).code(0);
    hosts(dir.path(), &["remove", "a.test"])
        .code(0)
        .stdout(contains("removed a.test"));
    assert!(!root_text(dir.path()).contains("extensions"));
    hosts(dir.path(), &["remove", "a.test"])
        .code(0)
        .stdout(contains("was not listed"));
}

#[test]
fn a_bad_host_or_address_exits_3_and_writes_nothing() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    hosts(dir.path(), &["set", "a.test", "nope"])
        .code(3)
        .stderr(contains("not an IP address"));
    hosts(dir.path(), &["set", "a.test", "0.0.0.0"]).code(3);
    hosts(dir.path(), &["set", "*.test", "127.0.0.1"]).code(3);
    assert_eq!(root_text(dir.path()), ROOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_sends_through_the_override_and_verbose_shows_it() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let url = format!("http://fake.test:{}/ok", s.address().port());
    let request = format!(
        "info:\n  name: A\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
    hosts(dir.path(), &["set", "fake.test", "127.0.0.1"]).code(0);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--verbose"])
        .assert()
        .code(0)
        .get_output()
        .stderr
        .clone();
    let err = String::from_utf8(out).unwrap();
    assert!(err.contains("(override fake.test -> 127.0.0.1)"), "{err}");
}
