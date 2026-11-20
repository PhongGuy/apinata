//! `apinata import collection --update` with no input reuses the file the import came from.

use crate::common::apinata;
use predicates::str::contains;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apinata-core/tests/fixtures/openapi")
        .join(name)
}

#[test]
fn an_update_with_no_input_reads_the_file_the_import_came_from() {
    let parent = tempfile::tempdir().unwrap();
    let spec = parent.path().join("api.json");
    fs::copy(fixture("update-v1.json"), &spec).unwrap();
    apinata()
        .args([
            "import",
            "collection",
            "--no-keychain",
            "--group-by",
            "path",
        ])
        .arg(&spec)
        .arg("--into")
        .arg(parent.path())
        .assert()
        .code(0);
    let root = parent.path().join("Shop");
    fs::copy(fixture("update-v2.json"), &spec).unwrap();
    apinata()
        .args(["import", "collection", "--update", "--into"])
        .arg(&root)
        .assert()
        .code(0)
        .stderr(contains("api.json"))
        .stdout(contains("added users/listUsers.yml"));
}
