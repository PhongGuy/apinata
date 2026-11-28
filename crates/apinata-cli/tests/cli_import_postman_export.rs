//! `apinata import postman-export`: a whole Postman data export in one command, one collection
//! per Postman collection, each with every exported environment.

use crate::common::apinata;
use serde_json::Value;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use zip::write::SimpleFileOptions;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const ARCHIVE: &str = r#"{"collection":{"c1":true,"c2":true},"environment":{"e1":true}}"#;
const SECRET: &str = "tok-s3cret-value";

fn collection(name: &str) -> String {
    format!(
        r#"{{"info":{{"name":"{name}","schema":"{V21}"}},"item":[{{"name":"r","request":{{"method":"GET","url":"https://x.test/r"}}}}]}}"#
    )
}

fn environment() -> String {
    format!(
        r#"{{"name":"dev","values":[{{"key":"host","value":"x.test","enabled":true}},{{"key":"token","value":"{SECRET}","type":"secret","enabled":true}}],"_postman_variable_scope":"environment"}}"#
    )
}

/// Two collections with the same name, one environment, and `extra` entries.
fn export_zip(path: &Path, extra: &[(&str, &str)]) {
    let mut entries = vec![
        ("export/archive.json".to_owned(), ARCHIVE.to_owned()),
        ("export/collection/c1.json".to_owned(), collection("Shop")),
        ("export/collection/c2.json".to_owned(), collection("Shop")),
        ("export/environment/e1.json".to_owned(), environment()),
    ];
    entries.extend(
        extra
            .iter()
            .map(|(n, t)| ((*n).to_owned(), (*t).to_owned())),
    );
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, text) in &entries {
        zip.start_file(name.as_str(), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

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

/// Run the import of `zip` into `into` with the file-backed test store, returning the exit code,
/// stdout and stderr.
fn import(work: &Path, zip: &Path, into: &Path, extra: &[&str]) -> (Option<i32>, String, String) {
    let out = apinata()
        .env("APINATA_TEST_SECRETS_FILE", work.join("secrets.json"))
        .env("XDG_RUNTIME_DIR", work.join("xdg"))
        .args(["import", "postman-export"])
        .arg(zip)
        .arg("--into")
        .arg(into)
        .args(extra)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn every_collection_is_written_with_the_environments_and_the_report_matches() {
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("export.zip");
    export_zip(&zip, &[]);
    let into = work.path().join("into");
    let (code, stdout, stderr) = import(work.path(), &zip, &into, &["--format", "json"]);
    assert_eq!(code, Some(0), "{stderr}");
    let written = names(&into);
    assert_eq!(written.len(), 2, "{written:?}");
    let reports: Vec<Value> = serde_json::Deserializer::from_str(&stdout)
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let mut reported: Vec<String> = reports
        .iter()
        .map(|r| {
            assert_eq!(r["environments"], serde_json::json!(["dev"]), "{r}");
            let root = r["collection"].as_str().unwrap();
            Path::new(root)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    reported.sort();
    assert_eq!(reported, written);
    for name in &written {
        assert!(
            into.join(name)
                .join("environments")
                .join("dev.yml")
                .is_file()
        );
    }
    let files = all_text(&into);
    assert!(
        !files.contains(SECRET),
        "a secret value in a file:\n{files}"
    );
}

#[test]
fn without_the_keychain_each_collection_asks_for_the_secret() {
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("export.zip");
    export_zip(&zip, &[]);
    let into = work.path().join("into");
    let (code, stdout, stderr) = import(
        work.path(),
        &zip,
        &into,
        &["--format", "json", "--no-keychain"],
    );
    assert_eq!(code, Some(0), "{stderr}");
    let asks: Vec<Value> = serde_json::Deserializer::from_str(&stdout)
        .into_iter()
        .map(|r: Result<Value, _>| r.unwrap()["secrets_to_enter"].clone())
        .collect();
    let want = serde_json::json!([{"environment": "dev", "variable": "token"}]);
    assert_eq!(asks, [want.clone(), want]);
}

#[test]
fn unreadable_entries_are_listed_and_exit_3_after_the_rest_is_written() {
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("export.zip");
    export_zip(&zip, &[("export/collection/bad.json", "{not json")]);
    let into = work.path().join("into");
    let (code, stdout, stderr) = import(work.path(), &zip, &into, &[]);
    assert_eq!(code, Some(3), "{stdout}\n{stderr}");
    assert_eq!(names(&into).len(), 2);
    assert!(
        stderr.contains("not imported: export/collection/bad.json"),
        "{stderr}"
    );
}

#[test]
fn entry_names_never_place_files_outside_into() {
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("export.zip");
    let climbing = collection("Climb");
    export_zip(
        &zip,
        &[
            ("../../escape.json", &climbing),
            ("/abs/escape.json", &climbing),
        ],
    );
    let into = work.path().join("into");
    let (code, _, stderr) = import(work.path(), &zip, &into, &[]);
    assert_eq!(code, Some(0), "{stderr}");
    let beside: Vec<String> = names(work.path())
        .into_iter()
        .filter(|n| !["export.zip", "into", "secrets.json", "xdg"].contains(&n.as_str()))
        .collect();
    assert!(beside.is_empty(), "written outside --into: {beside:?}");
    assert_eq!(names(&into).len(), 4);
}

#[test]
fn a_zip_that_is_not_an_export_writes_nothing() {
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("plain.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&zip).unwrap());
    writer
        .start_file("c.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(collection("Shop").as_bytes()).unwrap();
    writer.finish().unwrap();
    let into = work.path().join("into");
    let (code, _, stderr) = import(work.path(), &zip, &into, &[]);
    assert_eq!(code, Some(3));
    assert!(stderr.contains("not a Postman data export"), "{stderr}");
    assert!(!into.exists());
}

#[test]
fn a_bare_archive_json_reads_the_folder_it_sits_in() {
    let work = tempfile::tempdir().unwrap();
    let export = work.path().join("export");
    fs::create_dir_all(export.join("collection")).unwrap();
    fs::write(export.join("archive.json"), ARCHIVE).unwrap();
    fs::write(export.join("collection/c1.json"), collection("Shop")).unwrap();
    let into = work.path().join("into");
    let out = apinata()
        .current_dir(&export)
        .args(["import", "postman-export", "archive.json", "--into"])
        .arg(&into)
        .output()
        .unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert_eq!(names(&into), ["Shop"]);
}

#[test]
fn skipped_entry_names_print_without_control_characters_or_values() {
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("export.zip");
    let leaky = format!(r#"{{"info":{{"name":"x","schema":"{V21}"}},"item":"{SECRET}"}}"#);
    export_zip(
        &zip,
        &[
            ("export/\u{1b}[2Jevil.json", "{not json"),
            ("export/collection/leaky.json", &leaky),
        ],
    );
    let into = work.path().join("into");
    let (code, stdout, stderr) = import(work.path(), &zip, &into, &[]);
    assert_eq!(code, Some(3), "{stdout}\n{stderr}");
    assert!(stderr.contains("evil.json"), "{stderr}");
    assert!(!stderr.contains('\u{1b}'), "{stderr:?}");
    assert!(
        !stderr.contains(SECRET) && !stdout.contains(SECRET),
        "{stderr}"
    );
}
