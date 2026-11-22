//! `apinata import` errors and notes name the file or link and say what is wrong in plain words.

use crate::common::apinata;
use std::fs;
use std::path::Path;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn shop() -> String {
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1"}},"event":[{{"listen":"test","script":{{"exec":["pm.test('ok')"]}}}}]}}]}}"#
    )
}

// Runs `import collection <source> --into <into>`, returning the exit code and stderr.
fn import(source: &str, into: &Path, stdin: Option<String>) -> (Option<i32>, String) {
    let mut cmd = apinata();
    cmd.args(["import", "collection", source, "--into"])
        .arg(into);
    if let Some(text) = stdin {
        cmd.write_stdin(text);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_missing_file_is_named_without_an_os_error_code() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.json");
    let (code, stderr) = import(missing.to_str().unwrap(), &dir.path().join("new"), None);
    assert_eq!(code, Some(3), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: cannot read {}: there is no file at that path\n",
            missing.display()
        )
    );
}

#[test]
fn a_folder_given_as_the_file_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stderr) = import(dir.path().to_str().unwrap(), &dir.path().join("new"), None);
    assert_eq!(code, Some(3), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: cannot read {}: it is a folder, not a file\n",
            dir.path().display()
        )
    );
}

#[test]
fn truncated_json_names_the_file_and_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("truncated.json");
    fs::write(&file, "{\"info\": {\"name\": \"x\",\n  \"item\": [").unwrap();
    let (code, stderr) = import(file.to_str().unwrap(), &dir.path().join("new"), None);
    assert_eq!(code, Some(3), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: cannot import {}: the JSON ends before the closing ] (line 2, column 12)\n",
            file.display()
        )
    );
    assert!(!dir.path().join("new").exists());
}

#[test]
fn a_link_that_cannot_be_reached_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let link = format!("http://127.0.0.1:{port}/c.json");
    let (code, stderr) = import(&link, &dir.path().join("new"), None);
    assert_eq!(code, Some(3), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: cannot import from http://127.0.0.1:{port}: 127.0.0.1 refused the connection\n"
        )
    );
}

#[test]
fn a_link_is_named_by_its_origin_only() {
    let dir = tempfile::tempdir().unwrap();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let link = format!("http://127.0.0.1:{port}/signed/s3cret-path.json?token=s3cret#s3cret-part");
    let (_, stderr) = import(&link, &dir.path().join("new"), None);
    assert!(!stderr.contains("s3cret"), "{stderr}");
    assert!(
        stderr.contains(&format!("cannot import from http://127.0.0.1:{port}: ")),
        "{stderr}"
    );
}

#[test]
fn a_dash_reads_the_collection_from_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stderr) = import("-", dir.path(), Some(shop()));
    assert_eq!(code, Some(0), "{stderr}");
    assert!(dir.path().join("Shop/Get user.yml").is_file(), "{stderr}");
}

#[test]
fn a_note_names_the_request_instead_of_the_event_index() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let file = input.path().join("shop.json");
    fs::write(&file, shop()).unwrap();
    let (code, stderr) = import(file.to_str().unwrap(), into.path(), None);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stderr.contains("note: converted request `Get user`: post-response script:"),
        "{stderr}"
    );
    assert!(!stderr.contains("event["), "{stderr}");
}
