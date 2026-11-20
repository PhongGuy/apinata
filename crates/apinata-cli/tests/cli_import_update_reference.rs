//! `apinata import collection --update` with no input, on a collection whose only OpenAPI
//! record is the reference app's. Links are wiremock on 127.0.0.1.

use crate::common::apinata;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use std::fs;
use std::path::{Path, PathBuf};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apinata-core/tests/fixtures/openapi")
        .join(name)
}

/// Import v1, then swap its `extensions` for a reference record naming `source`.
fn reference_only(parent: &Path, source: &str, group_by: &str) -> PathBuf {
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(parent)
        .assert()
        .code(0);
    let root = parent.join("Shop");
    let file = root.join("opencollection.yml");
    let text = fs::read_to_string(&file).unwrap();
    let mut kept = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.starts_with("extensions:") {
            skipping = true;
            continue;
        }
        if skipping && (line.starts_with(' ') || line.is_empty()) {
            continue;
        }
        skipping = false;
        kept.push_str(line);
        kept.push('\n');
    }
    let block = format!(
        "extensions:\n  bruno:\n    openapi:\n      - sourceUrl: {source}\n        groupBy: {group_by}\n        autoCheck: true\n        autoCheckInterval: 5\n"
    );
    fs::write(&file, format!("{kept}{block}")).unwrap();
    root
}

fn update(root: &Path) -> assert_cmd::assert::Assert {
    apinata()
        .args(["import", "collection", "--update", "--into"])
        .arg(root)
        .assert()
}

#[test]
fn a_recorded_file_source_is_adopted_and_grouped_by_path() {
    let parent = tempfile::tempdir().unwrap();
    let root = reference_only(parent.path(), "spec.json", "path");
    fs::copy(fixture("update-v2.json"), root.join("spec.json")).unwrap();
    update(&root)
        .code(0)
        .stdout(contains("added users/listUsers.yml"))
        .stdout(contains("differs people/getUser.yml (target, docs)"))
        .stdout(contains("pruned").not().or(contains("pruned: 0")));
    assert!(root.join("users/listUsers.yml").is_file());
    let text = fs::read_to_string(root.join("opencollection.yml")).unwrap();
    assert!(text.contains("      - sourceUrl: spec.json\n        groupBy: path\n        autoCheck: true\n        autoCheckInterval: 5\n"), "{text}");
    assert!(text.contains("  apinata:"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_link_is_fetched_and_grouped_by_tags() {
    let server = MockServer::start().await;
    Mock::given(path("/spec.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fs::read_to_string(fixture("update-v2.json")).unwrap()),
        )
        .mount(&server)
        .await;
    let parent = tempfile::tempdir().unwrap();
    let root = reference_only(
        parent.path(),
        &format!("{}/spec.json", server.uri()),
        "tags",
    );
    let out =
        tokio::task::spawn_blocking(move || update(&root).code(0).get_output().stdout.clone())
            .await
            .unwrap();
    let out = String::from_utf8_lossy(&out);
    assert!(out.contains("added people/listUsers.yml"), "{out}");
}

#[test]
fn a_missing_recorded_file_is_a_clear_error() {
    let parent = tempfile::tempdir().unwrap();
    let root = reference_only(parent.path(), "gone.json", "tags");
    update(&root)
        .failure()
        .stderr(contains("recorded spec").and(contains("gone.json")));
}

#[test]
fn an_unreachable_recorded_link_is_a_clear_error() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let parent = tempfile::tempdir().unwrap();
    let root = reference_only(
        parent.path(),
        &format!("http://127.0.0.1:{port}/spec.json"),
        "tags",
    );
    update(&root)
        .failure()
        .stderr(contains("recorded spec").and(contains("cannot fetch")));
}

#[test]
fn no_input_and_no_recorded_source_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(parent.path())
        .assert()
        .code(0);
    // Forget the source the import recorded, as a collection imported elsewhere would have none.
    let file = parent.path().join("Shop/opencollection.yml");
    let text = fs::read_to_string(&file).unwrap();
    let kept: String = text
        .lines()
        .filter(|l| !l.starts_with("      source: ") && !l.starts_with("      groupBy: "))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&file, kept).unwrap();
    update(&parent.path().join("Shop"))
        .failure()
        .stderr(contains("FILE_OR_URL"));
}

#[test]
fn an_import_without_update_still_needs_an_input() {
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .assert()
        .failure()
        .stderr(contains("FILE_OR_URL"));
}

#[test]
fn the_recorded_source_is_named_on_stderr() {
    let parent = tempfile::tempdir().unwrap();
    let root = reference_only(parent.path(), "spec.json", "tags");
    fs::copy(fixture("update-v2.json"), root.join("spec.json")).unwrap();
    let shown = root.join("spec.json").display().to_string();
    apinata()
        .args([
            "import",
            "collection",
            "--update",
            "--format",
            "json",
            "--into",
        ])
        .arg(&root)
        .assert()
        .code(0)
        .stderr(contains(format!(
            "Updating from the recorded source {shown}"
        )))
        .stdout(contains("Updating").not());
}

#[test]
fn a_folder_update_without_input_is_refused_and_changes_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let root = reference_only(parent.path(), "spec.json", "tags");
    fs::copy(fixture("update-v2.json"), root.join("spec.json")).unwrap();
    let before = snapshot(&root);
    apinata()
        .args([
            "import",
            "collection",
            "--update",
            "--folder",
            "people",
            "--into",
        ])
        .arg(&root)
        .assert()
        .failure()
        .stderr(contains("give a spec file or link to update a folder"));
    assert_eq!(snapshot(&root), before);
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}
