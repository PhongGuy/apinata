//! `apinata item rename-example` and `rm-example`: a request's saved examples renamed and
//! removed by name, refused for an unknown, taken or ambiguous name, and a removed example
//! no longer served by `apinata mock`.

use crate::cli_item::Copy;
use crate::cli_mock::Mock;
use crate::common;

use std::fs;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use wiremock::matchers::method;
use wiremock::{MockServer, ResponseTemplate};

const USERS: &str = "info:\n  name: r\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://localhost/r\n\nexamples:\n  - name: ok\n    response:\n      status: 200\n  - name: missing\n    response:\n      status: 404\n";

fn fixture(text: &str) -> Copy {
    Copy::of(&write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", text),
    ]))
}

fn disk(c: &Copy) -> String {
    fs::read_to_string(c.root().join("r.yml")).expect("file")
}

#[test]
fn rename_example_renames_it_in_the_file() {
    let c = fixture(USERS);
    c.item(&["rename-example", "r.yml", "ok", "fine"])
        .code(0)
        .stdout("renamed example \"ok\" of r.yml to \"fine\"\n");
    assert_eq!(disk(&c), USERS.replace("name: ok", "name: fine"));
    c.item(&[
        "--format",
        "json",
        "rename-example",
        "r.yml",
        "fine",
        "good",
    ])
    .code(0)
    .stdout("{\"path\":\"r.yml\",\"name\":\"good\"}\n");
}

#[test]
fn rm_example_removes_only_that_example() {
    let c = fixture(USERS);
    c.item(&["rm-example", "r.yml", "ok"])
        .code(0)
        .stdout("removed example \"ok\" from r.yml\n");
    let text = disk(&c);
    assert!(
        !text.contains("name: ok") && text.contains("name: missing"),
        "{text}"
    );
    c.item(&["--format", "json", "rm-example", "r.yml", "missing"])
        .code(0)
        .stdout("{\"path\":\"r.yml\",\"removed\":\"missing\"}\n");
    assert!(!disk(&c).contains("examples:"), "{}", disk(&c));
}

#[test]
fn an_unknown_example_or_request_exits_3_and_changes_nothing() {
    let c = fixture(USERS);
    c.item(&["rm-example", "r.yml", "gone"])
        .code(3)
        .stderr(contains("r.yml has no example named `gone`"));
    c.item(&["rename-example", "r.yml", "gone", "x"])
        .code(3)
        .stderr(contains("r.yml has no example named `gone`"));
    c.item(&["rm-example", "nope.yml", "ok"])
        .code(3)
        .stderr(contains("nope.yml"));
    assert_eq!(disk(&c), USERS);
}

#[test]
fn a_rename_onto_a_taken_name_exits_3_and_changes_nothing() {
    let c = fixture(USERS);
    c.item(&["rename-example", "r.yml", "ok", "missing"])
        .code(3)
        .stderr(contains("r.yml already has an example named `missing`"));
    assert_eq!(disk(&c), USERS);
}

#[test]
fn an_ambiguous_example_name_exits_3_and_changes_nothing() {
    let text = USERS.replace("name: missing", "name: ok");
    let c = fixture(&text);
    for args in [
        &["rm-example", "r.yml", "ok"][..],
        &["rename-example", "r.yml", "ok", "x"][..],
    ] {
        c.item(args).code(3).stderr(contains(
            "2 examples of r.yml are named `ok`, so none was changed; rename one in the app \
             or edit the file, then try again",
        ));
    }
    assert_eq!(disk(&c), text);
}

#[test]
fn the_help_lists_both_actions() {
    apinata()
        .args(["item", "--help"])
        .assert()
        .code(0)
        .stdout(contains("rename-example"))
        .stdout(contains("rm-example"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_removed_example_is_no_longer_served_by_the_mock() {
    let server = MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(201).set_body_string("saved"))
        .mount(&server)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ping\"\n",
        server.uri()
    );
    let c = fixture(&request);
    let mut run = apinata();
    for (k, v) in c.envs() {
        run.env(k, v);
    }
    run.arg("run")
        .arg(c.root().join("r.yml"))
        .args(["--no-keychain", "--save-example", "kept"])
        .assert()
        .code(0);
    let root = c.root();
    let served = Mock::start(&[root.to_str().unwrap()]).get("/ping");
    assert!(served.starts_with("HTTP/1.1 201"), "{served}");

    c.item(&["rm-example", "r.yml", "kept"]).code(0);
    let after = Mock::start(&[root.to_str().unwrap()]).get("/ping");
    assert!(!after.starts_with("HTTP/1.1 201"), "{after}");
    assert!(!after.contains("saved"), "{after}");
}
