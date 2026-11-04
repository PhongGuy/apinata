//! Opening collections: Bruno ordering, load errors, and what the walk skips.

use apinata_core::{Collection, OpenError, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bruno")
        .join(name)
}

fn write_tree(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    dir
}

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn request(name: &str, seq: Option<u64>) -> String {
    let seq = seq.map(|s| format!("  seq: {s}\n")).unwrap_or_default();
    format!(
        "info:\n  name: {name}\n  type: http\n{seq}http:\n  method: GET\n  url: https://example.test/{name}\n"
    )
}

fn folder_yml(seq: Option<u64>) -> String {
    let seq = seq.map(|s| format!("  seq: {s}\n")).unwrap_or_default();
    format!("info:\n  name: f\n  type: folder\n{seq}")
}

fn names(reqs: &[RequestFile]) -> Vec<&str> {
    reqs.iter().map(RequestFile::name).collect()
}

#[test]
fn opens_cli_docs_fixture_in_bruno_order() {
    let c = Collection::open(fixture("cli-docs")).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    assert_eq!(c.name(), "ymlcollection");
    let dirs: Vec<&str> = c.tree().folders().iter().map(|f| f.dir_name()).collect();
    assert_eq!(dirs, ["folder_1", "folder_2"]);
    assert_eq!(names(c.tree().requests()), ["request_1", "request_2"]);
    assert_eq!(
        c.tree().folders()[0].requests()[0].url(),
        "https://api.example.com/folder-1/one"
    );
    assert_eq!(c.tree().folders()[1].seq(), Some(2));
    assert!(c.environments().is_empty());
}

#[test]
fn opens_bruno_tests_fixture() {
    let c = Collection::open(fixture("bruno-tests")).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let dirs: Vec<&str> = c.tree().folders().iter().map(|f| f.dir_name()).collect();
    assert_eq!(dirs, ["response-time", "variable-datatypes"]);
    assert_eq!(
        names(c.tree().folders()[0].requests()),
        ["delayed headers", "delayed body"]
    );
    assert_eq!(
        names(c.tree().folders()[1].requests()),
        ["typed vars", "form-urlencoded", "multipart"]
    );
    let envs: Vec<&str> = c.environments().iter().map(|e| e.name()).collect();
    assert_eq!(envs, ["Local", "Prod"]);
}

#[test]
fn opens_env_inheritance_fixture() {
    let c = Collection::open(fixture("env-inheritance")).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    assert_eq!(c.tree().requests().len(), 4);
    let envs: Vec<&str> = c.environments().iter().map(|e| e.name()).collect();
    assert_eq!(envs, ["base", "dev", "qa", "scripted", "staging"]);
}

#[test]
fn folders_follow_brunos_name_then_sequence_order() {
    let (b, d, none) = (folder_yml(Some(1)), folder_yml(Some(2)), folder_yml(None));
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("b/folder.yml", &b),
        ("a/folder.yml", &none),
        ("c/folder.yml", &none),
        ("d/folder.yml", &d),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let dirs: Vec<&str> = c.tree().folders().iter().map(|f| f.dir_name()).collect();
    // sortByNameThenSequence splices seq items into the name-sorted list at index seq - 1.
    assert_eq!(dirs, ["b", "d", "a", "c"]);
}

#[test]
fn requests_sort_by_seq_then_file_name_with_missing_seq_last() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r_b.yml", &request("b", Some(2))),
        ("r_a.yml", &request("a", Some(2))),
        ("r_c.yml", &request("c", Some(1))),
        ("r_d.yml", &request("d", None)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let files: Vec<&str> = c
        .tree()
        .requests()
        .iter()
        .map(RequestFile::file_name)
        .collect();
    assert_eq!(files, ["r_c.yml", "r_a.yml", "r_b.yml", "r_d.yml"]);
}

#[test]
fn a_broken_file_is_a_load_error_and_the_rest_loads() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("good.yml", &request("good", Some(1))),
        ("bad.yml", "info:\n  name: a\ninfo:\n  name: b\n"),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert_eq!(names(c.tree().requests()), ["good"]);
    assert_eq!(c.load_errors().len(), 1);
    assert!(c.load_errors()[0].path().ends_with("bad.yml"));
}

#[test]
fn a_broken_folder_file_keeps_the_folder_and_reports_the_error() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", "info: [\n"),
        ("f/r.yml", &request("r", Some(1))),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert_eq!(c.tree().folders().len(), 1);
    assert_eq!(c.tree().folders()[0].dir_name(), "f");
    assert_eq!(c.tree().folders()[0].seq(), None);
    assert_eq!(names(c.tree().folders()[0].requests()), ["r"]);
    assert_eq!(c.load_errors().len(), 1);
    assert!(c.load_errors()[0].path().ends_with("folder.yml"));
}

#[test]
fn a_directory_without_a_root_file_is_not_a_collection() {
    let dir = TempDir::new().expect("tempdir");
    assert!(matches!(
        Collection::open(dir.path()),
        Err(OpenError::NotACollection { .. })
    ));
}

#[test]
fn an_unparseable_root_file_is_an_open_error() {
    let dir = write_tree(&[("opencollection.yml", "a: [\n")]);
    assert!(matches!(
        Collection::open(dir.path()),
        Err(OpenError::InvalidRoot { .. })
    ));
}

#[test]
fn the_walk_skips_what_bruno_skips() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("folder.yml", &folder_yml(None)),
        (".git/x.yml", &request("git", None)),
        ("node_modules/y.yml", &request("nm", None)),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
        ("README.md", "# hi\n"),
        (".env", "A=1\n"),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.tree().folders().is_empty());
    assert!(c.tree().requests().is_empty());
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let envs: Vec<&str> = c.environments().iter().map(|e| e.name()).collect();
    assert_eq!(envs, ["dev"]);
}

#[test]
fn equal_folder_seqs_share_one_slot_like_bruno() {
    let (one, two) = (folder_yml(Some(1)), folder_yml(Some(2)));
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", &one),
        ("b/folder.yml", &one),
        ("c/folder.yml", &two),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let dirs: Vec<&str> = c.tree().folders().iter().map(|f| f.dir_name()).collect();
    assert_eq!(dirs, ["a", "b", "c"]);
}

#[test]
fn a_whole_number_float_seq_counts_as_an_integer() {
    let req = request("r", None).replace("type: http\n", "type: http\n  seq: 3.0\n");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", "info:\n  name: f\n  seq: 2.0\n"),
        ("r.yml", &req),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert_eq!(c.tree().folders()[0].seq(), Some(2));
    assert_eq!(c.tree().requests()[0].seq(), Some(3));
}

#[test]
fn an_unreadable_environments_path_is_a_load_error() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("environments", "not a dir")]);
    let c = Collection::open(dir.path()).expect("opens");
    assert_eq!(c.load_errors().len(), 1);
    assert!(c.load_errors()[0].path().ends_with("environments"));
}

#[test]
fn a_dotdot_root_path_still_yields_a_directory_name() {
    let dir = write_tree(&[
        ("opencollection.yml", "opencollection: \"1.0.0\"\n"),
        ("sub/x.txt", ""),
    ]);
    let c = Collection::open(dir.path().join("sub").join("..")).expect("opens");
    assert!(!c.name().is_empty());
    assert!(!c.name().contains(".."));
}
