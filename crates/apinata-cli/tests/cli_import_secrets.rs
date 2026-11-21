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
