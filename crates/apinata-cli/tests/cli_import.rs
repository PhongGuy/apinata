//! `apinata import collection` and `apinata import environment`: Postman files written through core.

use crate::common;

use apinata_core::MemoryStore;
use apinata_core::import::{parse_collection, write_collection};
use common::{ROOT, apinata, run_json, write_tree};
use predicates::str::contains;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const DEV: &str = r#"{"name":"dev","values":[{"key":"host","value":"x.test","enabled":true},{"key":"token","value":"s3","type":"secret","enabled":true}],"_postman_variable_scope":"environment"}"#;
/// A Hoppscotch export holding one empty collection per name; `CON` is a reserved name the writer refuses.
fn hoppscotch(names: &[&str]) -> String {
    let each: Vec<String> = names
        .iter()
        .map(|n| format!(r#"{{"v":12,"name":"{n}","folders":[],"requests":[]}}"#))
        .collect();
    format!("[{}]", each.join(","))
}

const STAGE: &str = r#"{"name":"stage","values":[{"key":"host","value":"y.test"}]}"#;

fn shop() -> String {
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1"}},"event":[{{"listen":"test","script":{{"exec":["pm.test('ok')"]}}}}]}}]}}"#
    )
}

fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_collection_is_written_by_core_and_its_notes_listed() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection"])
        .arg(file(input.path(), "shop.json", &shop()))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let root = fs::canonicalize(into.path()).unwrap().join("Shop");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{}\n", root.display())
    );
    assert!(
        stderr.contains("note: converted Get user / event[0]"),
        "{stderr}"
    );
    assert!(!stderr.to_lowercase().contains("bruno"), "{stderr}");
    assert!(stderr.contains("(Get user.yml)"), "{stderr}");
    let core = tempfile::tempdir().unwrap();
    write_collection(
        core.path(),
        &parse_collection(None, shop().as_bytes()).unwrap(),
        &[],
        &MemoryStore::new(),
    )
    .unwrap();
    for rel in ["opencollection.yml", "Get user.yml"] {
        let read = |dir: &Path| fs::read_to_string(dir.join(rel)).unwrap();
        assert_eq!(read(&root), read(&core.path().join("Shop")), "{rel}");
    }
}

#[test]
fn a_second_import_takes_the_next_free_name() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let shop = file(input.path(), "shop.json", &shop());
    for _ in 0..2 {
        apinata()
            .args(["import", "collection"])
            .arg(&shop)
            .arg("--into")
            .arg(into.path())
            .assert()
            .code(0);
    }
    assert!(into.path().join("Shop - 1/opencollection.yml").is_file());
}

#[test]
fn environments_ride_along_and_secrets_without_a_keychain_need_input() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection", "--no-keychain", "--format", "json"])
        .arg(file(input.path(), "shop.json", &shop()))
        .arg("--env")
        .arg(file(input.path(), "dev.json", DEV))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["environments"], json!(["dev"]));
    assert_eq!(
        report["secrets_to_enter"],
        json!([{"environment": "dev", "variable": "token"}])
    );
    let notes = report["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n["kind"] == "needs-input" && n["file"] == "environments/dev.yml"),
        "{notes:?}"
    );
    let env = fs::read_to_string(into.path().join("Shop/environments/dev.yml")).unwrap();
    assert!(!env.contains("s3"));
}

#[test]
fn environments_import_into_an_existing_collection() {
    let input = tempfile::tempdir().unwrap();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["import", "environment", "--no-keychain"])
        .arg(file(input.path(), "dev.json", DEV))
        .arg(file(input.path(), "stage.json", STAGE))
        .arg("--collection")
        .arg(col.path())
        .assert()
        .code(0)
        .stdout("dev\nstage\n")
        .stderr(contains("note: needs-input values[1]"));
    for name in ["dev", "stage"] {
        assert!(
            col.path()
                .join(format!("environments/{name}.yml"))
                .is_file(),
            "{name}"
        );
    }
}

#[test]
fn bad_input_exits_3_and_writes_nothing() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let other = file(input.path(), "other.json", r#"{"other":1}"#);
    let v1 = file(
        input.path(),
        "v1.json",
        r#"{"id":"x","name":"c","order":[],"requests":[]}"#,
    );
    let collection = |f: &Path, extra: &[&str]| {
        apinata()
            .args(["import", "collection"])
            .arg(f)
            .args(extra)
            .arg("--into")
            .arg(into.path())
            .assert()
            .code(3)
    };
    collection(&other, &[]).stderr(contains("error: not a recognized collection format"));
    collection(&v1, &["--from", "postman"]).stderr(contains("error: "));
    collection(&input.path().join("missing.json"), &[]).stderr(contains("error: "));
    collection(
        &file(input.path(), "shop.json", &shop()),
        &["--env", other.to_str().unwrap()],
    )
    .stderr(contains("not a recognized environment format"));
    assert_eq!(fs::read_dir(into.path()).unwrap().count(), 0);
    apinata()
        .args(["import", "collection"])
        .arg(&other)
        .arg("--into")
        .arg(&other)
        .assert()
        .code(3);
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["import", "environment"])
        .arg(file(input.path(), "stage.json", STAGE))
        .arg(file(input.path(), "bad.json", r#"{"info":{}}"#))
        .arg("--collection")
        .arg(col.path())
        .assert()
        .code(3);
    assert!(!col.path().join("environments").exists());
}

#[test]
fn a_missing_into_directory_is_created_with_its_parents() {
    let input = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let into = base.path().join("a").join("b");
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "shop.json", &shop()))
        .arg("--into")
        .arg(&into)
        .assert()
        .code(0);
    assert!(into.join("Shop").join("opencollection.yml").is_file());
}

#[test]
fn an_into_that_is_a_file_is_refused() {
    let input = tempfile::tempdir().unwrap();
    let target = file(input.path(), "target", "x");
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "shop.json", &shop()))
        .arg("--into")
        .arg(&target)
        .assert()
        .failure()
        .stderr(contains("not a directory"));
}

#[test]
fn a_failed_parse_creates_no_missing_into_directory() {
    let input = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let into = base.path().join("a").join("b");
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "other.json", r#"{"other":1}"#))
        .arg("--into")
        .arg(&into)
        .assert()
        .failure();
    assert!(!base.path().join("a").exists());
}

#[test]
fn an_env_mismatch_creates_no_missing_into_directory() {
    let input = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    apinata()
        .args([
            "import",
            "collection",
            "--no-keychain",
            "--from",
            "hoppscotch",
        ])
        .arg(file(
            input.path(),
            "two.json",
            &hoppscotch(&["Alpha", "Beta"]),
        ))
        .arg("--env")
        .arg(file(input.path(), "dev.json", DEV))
        .arg("--into")
        .arg(base.path().join("a").join("b"))
        .assert()
        .failure()
        .stderr(contains("--env needs an input with one collection"));
    assert!(!base.path().join("a").exists());
}

#[test]
fn an_ancestor_that_is_a_file_fails_and_is_left_untouched() {
    let input = tempfile::tempdir().unwrap();
    let blocker = file(input.path(), "blocker", "keep");
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "shop.json", &shop()))
        .arg("--into")
        .arg(blocker.join("sub"))
        .assert()
        .failure()
        .stderr(contains("cannot create"));
    assert_eq!(fs::read_to_string(&blocker).unwrap(), "keep");
}

#[test]
fn a_first_write_failure_removes_only_the_directories_it_created() {
    let input = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let into = base.path().join("a").join("b");
    apinata()
        .args([
            "import",
            "collection",
            "--no-keychain",
            "--from",
            "hoppscotch",
        ])
        .arg(file(input.path(), "con.json", &hoppscotch(&["CON"])))
        .arg("--into")
        .arg(&into)
        .assert()
        .failure();
    assert!(base.path().is_dir());
    assert!(!base.path().join("a").exists());
}

#[test]
fn a_later_write_failure_keeps_the_directories_holding_written_collections() {
    let input = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let into = base.path().join("a").join("b");
    apinata()
        .args([
            "import",
            "collection",
            "--no-keychain",
            "--from",
            "hoppscotch",
        ])
        .arg(file(
            input.path(),
            "two.json",
            &hoppscotch(&["Alpha", "CON"]),
        ))
        .arg("--into")
        .arg(&into)
        .assert()
        .failure();
    assert!(into.join("Alpha").join("opencollection.yml").is_file());
}

#[test]
fn a_closed_port_fetch_creates_no_missing_into_directory() {
    let base = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(format!("http://127.0.0.1:{port}/c.json"))
        .arg("--into")
        .arg(base.path().join("a").join("b"))
        .assert()
        .failure();
    assert!(!base.path().join("a").exists());
}

#[test]
fn an_unparseable_environment_is_named_and_nothing_is_written() {
    let input = tempfile::tempdir().unwrap();
    let stage = file(input.path(), "stage.json", STAGE);
    let bad = file(input.path(), "broken-env.json", r#"{"info":{}}"#);
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["import", "environment"])
        .arg(&stage)
        .arg(&bad)
        .arg("--collection")
        .arg(col.path())
        .assert()
        .code(3)
        .stderr(contains("broken-env.json"))
        .stderr(contains("not a recognized environment format"));
    assert!(!col.path().join("environments").exists());

    let into = tempfile::tempdir().unwrap();
    apinata()
        .args(["import", "collection"])
        .arg(file(input.path(), "shop.json", &shop()))
        .arg("--env")
        .arg(&bad)
        .arg("--into")
        .arg(into.path())
        .assert()
        .code(3)
        .stderr(contains("broken-env.json"));
    assert_eq!(fs::read_dir(into.path()).unwrap().count(), 0);
}

#[test]
fn a_write_failure_keeps_earlier_environments_and_names_both_sets() {
    let input = tempfile::tempdir().unwrap();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let out = apinata()
        .args(["import", "environment", "--no-keychain"])
        .arg(file(input.path(), "dev.json", DEV))
        .arg(file(
            input.path(),
            "dots.json",
            r#"{"name":"...","values":[]}"#,
        )) // A7
        .arg(file(input.path(), "stage.json", STAGE))
        .arg("--collection")
        .arg(col.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        (out.status.code(), out.stdout.is_empty()),
        (Some(3), true),
        "{stderr}"
    );
    let tail: Vec<_> = stderr.lines().rev().take(2).collect();
    assert_eq!(
        tail,
        ["not written: ..., stage", "written: dev"],
        "{stderr}"
    );
    assert!(col.path().join("environments/dev.yml").is_file());
    assert!(!col.path().join("environments/stage.yml").exists());
}

#[test]
fn help_names_no_other_tool() {
    for sub in ["collection", "environment"] {
        let out = apinata().args(["import", sub, "--help"]).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(
            !text.contains("bruno") && !text.contains("no-keychain"),
            "{sub}: {text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imported_postman_schema_test_passes_and_fails_under_run() {
    let s = MockServer::start().await;
    for (route, body) in [("/good", r#"{"id":1}"#), ("/bad", r#"{"id":"one"}"#)] {
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&s)
            .await;
    }
    let item = |name: &str, route: &str| {
        json!({"name": name, "request": {"method": "GET", "url": format!("{}{route}", s.uri())},
        "event": [{"listen": "test", "script": {"exec": [
            "pm.test('schema', () => pm.response.to.have.jsonSchema({ type: 'object', properties: { id: { type: 'integer' } } }));"
        ]}}]})
    };
    let collection = json!({"info": {"name": "Schema", "schema": V21},
        "item": [item("Good", "/good"), item("Bad", "/bad")]});
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection"])
        .arg(file(input.path(), "schema.json", &collection.to_string()))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (code, report) = run_json(&into.path().join("Schema"), &[]);
    assert_eq!(code, 1, "{report}");
    let statuses: Vec<(String, Value)> = report["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap_or("").to_owned(),
                r["tests"][0]["status"].clone(),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("Good".to_owned(), json!("pass")),
            ("Bad".to_owned(), json!("fail"))
        ],
        "{report}"
    );
}
