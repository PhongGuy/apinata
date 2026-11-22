//! `apinata import collection` keeps imported credentials out of the written files: the value
//! goes to the secret store and the report names the secret variable, never the value.

use crate::common::apinata;
use serde_json::Value;
use std::fs;
use std::path::Path;

const POSTMAN: &str = r#"{"info":{"name":"Shop","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
  "item":[{"name":"Create user","request":{"method":"POST","url":"https://x.test/u",
    "auth":{"type":"basic","basic":[{"key":"username","value":"bob"},{"key":"password","value":"cli-pw-s3cret"}]}}}]}"#;

/// Every file under `dir`, concatenated.
fn all_text(dir: &Path) -> String {
    let mut out = String::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.push_str(&all_text(&path));
        } else {
            out.push_str(&fs::read_to_string(&path).unwrap_or_default());
        }
    }
    out
}

/// Run the import of [`POSTMAN`] as JSON with `secrets` as the store file, plus `extra` args.
fn import(work: &Path, extra: &[&str]) -> (Value, String) {
    let input = work.join("shop.json");
    fs::write(&input, POSTMAN).unwrap();
    let into = work.join("into");
    fs::create_dir_all(&into).unwrap();
    let out = apinata()
        .env("APINATA_TEST_SECRETS_FILE", work.join("secrets.json"))
        .env("XDG_RUNTIME_DIR", work.join("xdg"))
        .args(["import", "collection"])
        .arg(&input)
        .arg("--into")
        .arg(&into)
        .args(["--format", "json"])
        .args(extra)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(!stdout.contains("cli-pw-s3cret") && !stderr.contains("cli-pw-s3cret"));
    let files = all_text(&into);
    assert!(
        !files.contains("cli-pw-s3cret"),
        "a value in a file:\n{files}"
    );
    assert!(files.contains("{{create_user_password}}"), "{files}");
    (serde_json::from_str(&stdout).unwrap(), files)
}

#[test]
fn an_imported_password_goes_to_the_store_and_the_report_names_it() {
    let work = tempfile::tempdir().unwrap();
    let (report, _) = import(work.path(), &[]);
    let stored = fs::read_to_string(work.path().join("secrets.json")).unwrap();
    let entries: Vec<Value> = serde_json::from_str(&stored).unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["variable"] == "create_user_password"
                && e["environment"] == "Environment"
                && e["value"] == "cli-pw-s3cret"),
        "{entries:?}"
    );
    let notes = report["notes"].to_string();
    assert!(notes.contains("`create_user_password`"), "{notes}");
    assert_eq!(report["secrets_to_enter"], serde_json::json!([]));
}

#[test]
fn without_the_keychain_the_password_is_left_to_enter() {
    let work = tempfile::tempdir().unwrap();
    let (report, _) = import(work.path(), &["--no-keychain"]);
    assert!(!work.path().join("secrets.json").exists());
    assert_eq!(
        report["secrets_to_enter"],
        serde_json::json!([{"environment": "Environment", "variable": "create_user_password"}])
    );
}

/// A collection with the environment `dev`, and the store file beside it.
fn collection_with_dev(work: &Path) -> std::path::PathBuf {
    let root = work.join("col");
    fs::create_dir_all(root.join("environments")).unwrap();
    fs::write(root.join("opencollection.yml"), crate::common::ROOT).unwrap();
    fs::write(root.join("environments/dev.yml"), "name: dev\n").unwrap();
    root
}

/// The stored values as `variable=value`, sorted.
fn stored(work: &Path) -> Vec<String> {
    let text = fs::read_to_string(work.join("secrets.json")).unwrap_or_else(|_| "[]".into());
    let entries: Vec<Value> = serde_json::from_str(&text).unwrap();
    let mut rows: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{}={}",
                e["variable"].as_str().unwrap(),
                e["value"].as_str().unwrap()
            )
        })
        .collect();
    rows.sort();
    rows
}

fn with_store(work: &Path) -> assert_cmd::Command {
    let mut cmd = apinata();
    cmd.env("APINATA_TEST_SECRETS_FILE", work.join("secrets.json"))
        .env("XDG_RUNTIME_DIR", work.join("xdg"));
    cmd
}

#[test]
fn a_second_request_import_with_another_password_never_overwrites_the_first() {
    let work = tempfile::tempdir().unwrap();
    let root = collection_with_dev(work.path());
    for (name, command) in [
        ("A", "curl -u a:first-pw http://localhost:8080/x"),
        ("B", "curl -u a:second-pw http://localhost:9090/x"),
    ] {
        with_store(work.path())
            .args(["import", "request", "--name", name, "--collection"])
            .arg(&root)
            .write_stdin(command)
            .assert()
            .code(0);
    }
    assert_eq!(
        stored(work.path()),
        [
            "localhost_password=first-pw",
            "localhost_password_2=second-pw"
        ]
    );
    let b = fs::read_to_string(root.join("B.yml")).unwrap();
    assert!(b.contains("{{localhost_password_2}}"), "{b}");
}

async fn any_ok() -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_save_never_overwrites_an_earlier_saved_secret() {
    let server = any_ok().await;
    let work = tempfile::tempdir().unwrap();
    let root = collection_with_dev(work.path());
    for (file, password) in [("A.yml", "first-pw"), ("B.yml", "second-pw")] {
        with_store(work.path())
            .current_dir(work.path())
            .args(["send", "--save"])
            .arg(root.join(file))
            .arg("--curl")
            .arg(format!("curl -u a:{password} {}/x", server.uri()))
            .assert()
            .code(0);
    }
    assert_eq!(
        stored(work.path()),
        [
            "127_0_0_1_password=first-pw",
            "127_0_0_1_password_2=second-pw"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_save_stores_the_value_given_with_env_var() {
    let server = any_ok().await;
    let work = tempfile::tempdir().unwrap();
    let root = collection_with_dev(work.path());
    with_store(work.path())
        .current_dir(work.path())
        .args(["send", "--save"])
        .arg(root.join("K.yml"))
        .args(["--env-var", "127_0_0_1_x_api_key=given-key"])
        .arg("--curl")
        .arg(format!(
            "curl -H 'X-Api-Key: parsed-key' {}/k",
            server.uri()
        ))
        .assert()
        .code(0);
    assert_eq!(stored(work.path()), ["127_0_0_1_x_api_key=given-key"]);
    let sent = &server.received_requests().await.unwrap()[0];
    assert_eq!(sent.headers.get("x-api-key").unwrap(), "given-key");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_without_env_uses_the_environment_the_import_chose() {
    let server = wiremock::MockServer::start().await;
    // `bob:run-pw` as Basic credentials.
    wiremock::Mock::given(wiremock::matchers::header(
        "authorization",
        "Basic Ym9iOnJ1bi1wdw==",
    ))
    .respond_with(wiremock::ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("shop.json");
    let text = POSTMAN
        .replace("https://x.test/u", &format!("{}/u", server.uri()))
        .replace("cli-pw-s3cret", "run-pw");
    fs::write(&input, text).unwrap();
    let into = work.path().join("into");
    fs::create_dir_all(&into).unwrap();
    with_store(work.path())
        .args(["import", "collection"])
        .arg(&input)
        .arg("--into")
        .arg(&into)
        .assert()
        .code(0);
    with_store(work.path())
        .arg("run")
        .arg(into.join("Shop"))
        .assert()
        .code(0);
}

/// Text-format stdout and stderr of `apinata <args>` in `work`, with the platform store and no
/// session bus, so the keychain cannot answer.
#[cfg(target_os = "linux")]
fn with_unreachable_keychain(work: &Path, args: &[&str]) -> String {
    let out = apinata()
        .env_remove("APINATA_TEST_NO_KEYCHAIN")
        .env("XDG_RUNTIME_DIR", work.join("xdg"))
        .current_dir(work)
        .args(args)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "{all}");
    all
}

/// The keychain lines carry no platform text, no claim of a kept value, and there is one.
#[cfg(target_os = "linux")]
fn assert_one_plain_keychain_note(all: &str, names: &[&str]) {
    for raw in [
        "zbus",
        "os error",
        "secret store unavailable",
        "kept in the secret store",
    ] {
        assert!(!all.contains(raw), "{raw}:\n{all}");
    }
    let lines: Vec<&str> = all.lines().filter(|l| l.contains("keychain")).collect();
    assert_eq!(lines.len(), 1, "{all}");
    assert!(lines[0].contains("did not answer"), "{all}");
    for name in names {
        assert!(lines[0].contains(&format!("`{name}`")), "{name}:\n{all}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn with_no_keychain_answering_each_import_says_so_once_in_plain_words() {
    let work = tempfile::tempdir().unwrap();
    let w = work.path();
    fs::create_dir_all(w.join("xdg")).unwrap();
    fs::write(w.join("shop.json"), POSTMAN).unwrap();
    let all = with_unreachable_keychain(w, &["import", "collection", "shop.json", "--into", "new"]);
    assert_one_plain_keychain_note(&all, &["create_user_password"]);
    assert!(!all_text(&w.join("new")).contains("cli-pw-s3cret"));

    let root = w.join("new/Shop");
    fs::write(w.join("c.txt"), "curl -u a:req-pw-s3cret https://x.test/z").unwrap();
    let all = with_unreachable_keychain(
        w,
        &[
            "import",
            "request",
            "c.txt",
            "--collection",
            root.to_str().unwrap(),
        ],
    );
    assert_one_plain_keychain_note(&all, &["x_test_password"]);

    let env = r#"{"name":"E","values":[{"key":"k","value":"env-s3cret","type":"secret","enabled":true}]}"#;
    fs::write(w.join("e.json"), env).unwrap();
    let all = with_unreachable_keychain(
        w,
        &[
            "import",
            "environment",
            "e.json",
            "--collection",
            root.to_str().unwrap(),
        ],
    );
    assert_one_plain_keychain_note(&all, &["k"]);
    assert!(!all_text(&root).contains("req-pw-s3cret") && !all_text(&root).contains("env-s3cret"));
}
