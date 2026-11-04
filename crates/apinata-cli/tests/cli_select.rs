//! Which requests run: root discovery, folders recursively in tree order, single files, --bail,
//! and the environment flags.

use crate::common;

use common::{ROOT, apinata, check, column, get, run_json, write_tree};
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FOLDER_B: &str = "info:\n  name: b\n  type: folder\n  seq: 1\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folders_run_recursively_and_files_alone_from_any_directory() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let ok = format!("{}/ok", s.uri());
    let pass = check("res.status", "eq", "200");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("top.yml", &get("Top", 1, &ok, &pass)),
        ("a/y.yml", &get("Y", 1, &ok, &pass)),
        ("a/b/folder.yml", FOLDER_B),
        ("a/b/x.yml", &get("X", 1, &ok, &pass)),
        ("other/broken.yml", "info: [unclosed\n"),
    ]);
    let (code, report) = run_json(&dir.path().join("a"), &[]);
    assert_eq!(
        code, 0,
        "a load error outside the selection is not reported"
    );
    assert_eq!(column(&report, "path"), ["a/b/x.yml", "a/y.yml"]);
    assert_eq!(report["load_errors"], serde_json::json!([]));

    let out = apinata()
        .current_dir(dir.path().join("a/b"))
        .args(["run", "x.yml", "--format", "json"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(column(&report, "path"), ["a/b/x.yml"]);

    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 3);
    assert_eq!(column(&report, "path"), ["a/b/x.yml", "a/y.yml", "top.yml"]);
    assert_eq!(report["load_errors"][0]["path"], "other/broken.yml");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bail_skips_everything_after_the_first_failure() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let ok = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "1.yml",
            &get("One", 1, &ok, &check("res.status", "eq", "200")),
        ),
        (
            "2.yml",
            &get("Two", 2, &ok, &check("res.status", "eq", "201")),
        ),
        (
            "3.yml",
            &get("Three", 3, &ok, &check("res.status", "eq", "200")),
        ),
    ]);
    let (code, report) = run_json(dir.path(), &["--bail"]);
    assert_eq!(code, 1);
    assert_eq!(column(&report, "outcome"), ["passed", "failed", "skipped"]);
    let skipped = &report["results"][2];
    assert_eq!(skipped["status"], serde_json::Value::Null);
    assert_eq!(skipped["assertions"], serde_json::json!([]));
    assert_eq!(report["summary"]["skipped"], 1);
    assert_eq!(s.received_requests().await.expect("recorded").len(), 2);

    apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--bail")
        .assert()
        .code(1)
        .stdout(predicates::str::contains("3 (skipped by --bail)"));
    let (_, report) = run_json(dir.path(), &[]);
    assert_eq!(column(&report, "outcome"), ["passed", "failed", "passed"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_selects_an_environment_and_env_var_overrides_it() {
    let s = MockServer::start().await;
    Mock::given(path("/who"))
        .and(header("x-token", "from-env"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let env = "name: dev\nvariables:\n  - name: base\n    value: http://127.0.0.1:1\n  - name: token\n    value: from-env\n";
    let req = "info:\n  name: Who\n  type: http\nhttp:\n  method: GET\n  url: \"{{base}}/who\"\n  headers:\n    - name: X-Token\n      value: \"{{token}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
        ("who.yml", req),
    ]);
    let base = format!("base={}", s.uri());
    let (code, report) = run_json(dir.path(), &["--env", "dev", "--env-var", &base]);
    assert_eq!(code, 0, "{report}");
    let (code, _) = run_json(dir.path(), &["--env", "dev"]);
    assert_eq!(code, 2, "the environment's own base refuses connections");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_ancestor_folder_file_is_reported_for_a_selection_beneath_it() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let ok = format!("{}/ok", s.uri());
    let pass = check("res.status", "eq", "200");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", "info: [unclosed\n"),
        ("users/get-user.yml", &get("Get user", 1, &ok, &pass)),
        ("users/admin/list.yml", &get("List", 1, &ok, &pass)),
        ("users/admin/broken.yml", "info: [unclosed\n"),
        ("other/broken.yml", "info: [unclosed\n"),
    ]);
    let (code, report) = run_json(&dir.path().join("users/get-user.yml"), &[]);
    assert_eq!(code, 3, "{report}");
    let errors: Vec<_> = report["load_errors"]
        .as_array()
        .expect("load errors")
        .iter()
        .map(|e| e["path"].as_str().expect("path"))
        .collect();
    assert_eq!(errors, ["users/folder.yml"]);

    let (code, report) = run_json(&dir.path().join("users/admin"), &[]);
    assert_eq!(code, 3, "{report}");
    let mut errors: Vec<_> = report["load_errors"]
        .as_array()
        .expect("load errors")
        .iter()
        .map(|e| e["path"].as_str().expect("path").to_owned())
        .collect();
    errors.sort();
    assert_eq!(errors, ["users/admin/broken.yml", "users/folder.yml"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bail_skips_everything_after_an_errored_request() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let ok = format!("{}/ok", s.uri());
    let pass = check("res.status", "eq", "200");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("1.yml", &get("One", 1, &ok, &pass)),
        ("2.yml", &get("Two", 2, "http://127.0.0.1:1/refused", &pass)),
        ("3.yml", &get("Three", 3, &ok, &pass)),
    ]);
    let (code, report) = run_json(dir.path(), &["--bail"]);
    assert_eq!(code, 2, "{report}");
    assert_eq!(column(&report, "outcome"), ["passed", "errored", "skipped"]);
    assert_eq!(s.received_requests().await.expect("recorded").len(), 1);
}
