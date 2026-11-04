//! `apinata export`: the Postman file on stdout or in a file, environments, notes, exit codes.

use crate::common;

use common::{ROOT, apinata, check, get, write_tree};
use serde_json::{Value, json};
use std::path::Path;

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("a", 1, "http://x.test/a", &check("res.status", "eq", "200")),
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: http://x.test\n  - name: token\n    secret: true\n",
        ),
    ])
}

fn export(dir: &Path, out: &Path) -> std::process::Output {
    apinata()
        .arg("export")
        .arg(dir)
        .args(["--format", "postman", "--out"])
        .arg(out)
        .output()
        .expect("runs")
}

#[test]
fn stdout_holds_the_collection_and_stderr_the_notes() {
    let col = collection();
    let out = export(col.path(), Path::new("-"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(v["info"]["name"], "t");
    assert_eq!(v["item"][0]["name"], "a");
    assert_eq!(v["item"][0]["request"]["url"]["raw"], "http://x.test/a");
    assert!(
        stderr.contains("note: a / assertions: assertions have no place in the exported file"),
        "{stderr}"
    );
}

#[test]
fn out_writes_the_same_bytes_and_nothing_else() {
    let col = collection();
    let dest = tempfile::tempdir().unwrap();
    let file = dest.path().join("c.postman_collection.json");
    assert_eq!(export(col.path(), &file).status.code(), Some(0));
    assert_eq!(
        std::fs::read(&file).unwrap(),
        export(col.path(), Path::new("-")).stdout
    );
    let names: Vec<String> = std::fs::read_dir(dest.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["c.postman_collection.json"]);
}

// A new export gets the mode any other new file in that directory gets under the current
// umask; a replaced one keeps its mode.
#[cfg(unix)]
#[test]
fn out_gets_ordinary_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let col = collection();
    let dest = tempfile::tempdir().unwrap();
    let plain = dest.path().join("plain");
    std::fs::write(&plain, b"").unwrap();
    let file = dest.path().join("c.postman_collection.json");
    assert_eq!(export(col.path(), &file).status.code(), Some(0));
    assert_eq!(mode(&file), mode(&plain));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(export(col.path(), &file).status.code(), Some(0));
    assert_eq!(mode(&file), 0o640);
}

#[test]
fn env_dir_writes_each_environment_with_secrets_empty() {
    let col = collection();
    let dest = tempfile::tempdir().unwrap();
    let envs = dest.path().join("nested/envs");
    let out = apinata()
        .arg("export")
        .arg(col.path())
        .args(["--format", "postman", "--out"])
        .arg(dest.path().join("c.json"))
        .arg("--env-dir")
        .arg(&envs)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value =
        serde_json::from_slice(&std::fs::read(envs.join("dev.postman_environment.json")).unwrap())
            .unwrap();
    assert_eq!(v["name"], "dev");
    assert_eq!(
        v["values"],
        json!([
        {"key": "host", "value": "http://x.test", "type": "default", "enabled": true},
        {"key": "token", "value": "", "type": "secret", "enabled": true}])
    );
}

#[test]
fn failures_exit_3_and_write_nothing() {
    let col = collection();
    let dest = tempfile::tempdir().unwrap();
    let missing = dest.path().join("missing/c.json");
    let out = export(col.path(), &missing);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing"));
    assert!(!missing.exists());
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(export(empty.path(), Path::new("-")).status.code(), Some(3));
    let out = apinata()
        .arg("export")
        .arg(col.path())
        .args(["--format", "insomnia", "--out", "-"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn environments_whose_stems_collide_get_a_number() {
    // ` dev` sorts before `dev` and trims to the same stem, so `dev` takes the ` (2)`.
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev
variables: []
",
        ),
        (
            "environments/ dev.yml",
            "name: spaced
variables: []
",
        ),
    ]);
    let dest = tempfile::tempdir().unwrap();
    let out = apinata()
        .arg("export")
        .arg(col.path())
        .args(["--format", "postman", "--out"])
        .arg(dest.path().join("c.json"))
        .arg("--env-dir")
        .arg(dest.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let name = |file: &str| -> Value {
        let v: Value =
            serde_json::from_slice(&std::fs::read(dest.path().join(file)).expect(file)).unwrap();
        v["name"].clone()
    };
    assert_eq!(name("dev.postman_environment.json"), " dev");
    assert_eq!(name("dev (2).postman_environment.json"), "dev");
}

#[test]
fn load_errors_are_warnings_and_the_rest_still_exports() {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("a", 1, "http://x.test/a", &check("res.status", "eq", "200")),
        ),
        ("bad.yml", "info: [\n"),
    ]);
    let out = export(col.path(), Path::new("-"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        stderr
            .lines()
            .any(|l| l.starts_with("warning: ") && l.contains("bad.yml")),
        "{stderr}"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(v["item"].as_array().unwrap().len(), 1);
}

fn export_openapi(dir: &Path, out: &Path) -> std::process::Output {
    apinata()
        .arg("export")
        .arg(dir)
        .args(["--format", "openapi", "--out"])
        .arg(out)
        .output()
        .expect("runs")
}

#[test]
fn openapi_goes_to_stdout_with_notes_on_stderr() {
    let col = collection();
    let out = export_openapi(col.path(), Path::new("-"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(v["openapi"], "3.1.0");
    assert_eq!(v["info"]["title"], "t");
    assert_eq!(v["servers"], json!([{"url": "http://x.test"}]));
    assert_eq!(v["paths"]["/a"]["get"]["summary"], "a");
    assert!(
        stderr.contains(
            "note: collection / assertions: 1 item has assertions; OpenAPI has no place for them"
        ),
        "{stderr}"
    );
}

#[test]
fn openapi_out_writes_the_same_bytes() {
    let col = collection();
    let dest = tempfile::tempdir().unwrap();
    let file = dest.path().join("c.openapi.json");
    assert_eq!(export_openapi(col.path(), &file).status.code(), Some(0));
    assert_eq!(
        std::fs::read(&file).unwrap(),
        export_openapi(col.path(), Path::new("-")).stdout
    );
}

#[test]
fn env_dir_with_openapi_is_a_usage_error() {
    let col = collection();
    let dest = tempfile::tempdir().unwrap();
    let file = dest.path().join("c.openapi.json");
    let envs = dest.path().join("envs");
    let out = apinata()
        .arg("export")
        .arg(col.path())
        .args(["--format", "openapi", "--out"])
        .arg(&file)
        .arg("--env-dir")
        .arg(&envs)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "--env-dir does not apply to --format openapi; environments are carried as servers"
        ),
        "{stderr}"
    );
    assert!(!file.exists() && !envs.exists());
}

fn credential_collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x.test/a\"\n  headers:\n    - name: Authorization\n      value: Basic abc\n",
        ),
    ])
}

fn export_html(dir: &Path, out: &Path) -> std::process::Output {
    apinata()
        .arg("export")
        .arg(dir)
        .args(["--format", "html", "--out"])
        .arg(out)
        .output()
        .expect("runs")
}

#[test]
fn html_goes_to_stdout_with_notes_on_stderr() {
    let col = credential_collection();
    let out = export_html(col.path(), Path::new("-"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let page = String::from_utf8(out.stdout).expect("UTF-8");
    assert!(page.starts_with("<!doctype html>\n"), "{page}");
    assert!(page.contains("<title>t</title>"), "{page}");
    assert!(
        page.contains("<a href=\"#r-1\"><span class=\"method m-get\">GET</span> a</a>"),
        "{page}"
    );
    assert!(
        stderr.contains("note: a / headers: header Authorization holds a literal credential, written into the page"),
        "{stderr}"
    );
}

#[test]
fn html_out_writes_the_same_bytes() {
    let col = credential_collection();
    let dest = tempfile::tempdir().unwrap();
    let file = dest.path().join("c.docs.html");
    assert_eq!(export_html(col.path(), &file).status.code(), Some(0));
    assert_eq!(
        std::fs::read(&file).unwrap(),
        export_html(col.path(), Path::new("-")).stdout
    );
}

#[test]
fn env_dir_with_html_is_a_usage_error() {
    let col = credential_collection();
    let dest = tempfile::tempdir().unwrap();
    let file = dest.path().join("c.docs.html");
    let envs = dest.path().join("envs");
    let out = apinata()
        .arg("export")
        .arg(col.path())
        .args(["--format", "html", "--out"])
        .arg(&file)
        .arg("--env-dir")
        .arg(&envs)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "--env-dir does not apply to --format html; the page leaves environments out"
        ),
        "{stderr}"
    );
    assert!(!file.exists() && !envs.exists());
}
