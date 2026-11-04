//! The public conversion plan: detection, the file lists and every refusal.

use apinata_core::legacy::{
    ConvertError, ConvertTarget, convert, is_legacy_collection, plan_conversion,
};
use apinata_core::{Collection, OpenError, OpenOptions};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const BRUNO_JSON: &str = r#"{"version": "1", "name": "Legacy", "type": "collection"}"#;
const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";
const MALFORMED: &str = "meta {\n  name: a\n";

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn legacy(root: &Path) {
    write(root, "bruno.json", BRUNO_JSON);
    write(root, "a.bru", REQUEST);
    write(root, "f/folder.bru", "meta {\n  name: f\n}\n");
    write(root, "f/b.bru", &REQUEST.replace("name: a", "name: b"));
    write(
        root,
        "environments/Local.bru",
        "vars {\n  host: http://localhost\n}\n",
    );
    write(root, ".env", "TOKEN=x\n");
    write(root, "body.json", "{}\n");
}

fn paths(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn detection_needs_bruno_json_and_no_opencollection_yml() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    assert!(!is_legacy_collection(root));
    write(root, "a.bru", REQUEST);
    assert!(
        !is_legacy_collection(root),
        "a .bru file alone is not a collection"
    );
    write(root, "bruno.json", BRUNO_JSON);
    assert!(is_legacy_collection(root));
    write(
        root,
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: y\n",
    );
    assert!(!is_legacy_collection(root), "opencollection.yml wins");
}

#[test]
fn a_plan_lists_what_is_created_removed_and_copied_and_writes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("legacy");
    legacy(&root);
    let before = snapshot(&root);
    let plan = plan_conversion(&root).unwrap();
    assert_eq!(plan.root(), root.as_path());
    assert_eq!(plan.name(), "Legacy");
    assert_eq!(
        plan.created(),
        paths(&[
            "a.yml",
            "environments/Local.yml",
            "f/b.yml",
            "f/folder.yml",
            "opencollection.yml"
        ])
    );
    assert_eq!(
        plan.removed_in_place(),
        paths(&[
            "a.bru",
            "bruno.json",
            "environments/Local.bru",
            "f/b.bru",
            "f/folder.bru"
        ])
    );
    assert_eq!(plan.copied(), paths(&[".env", "body.json"]));
    assert_eq!(snapshot(&root), before);
}

#[test]
fn skipped_directories_and_symlinks_are_neither_converted_nor_copied() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("c");
    write(
        &root,
        "bruno.json",
        r#"{"version": "1", "name": "c", "type": "collection", "ignore": ["vendor"]}"#,
    );
    write(&root, "a.bru", REQUEST);
    write(&root, "keep/k.txt", "k\n");
    write(&root, "node_modules/m.js", "m\n");
    write(&root, "node_modules/n.bru", MALFORMED);
    write(&root, ".git/config", "g\n");
    write(&root, "vendor/v.txt", "v\n");
    write(&root, "deep/vendor/w.bru", MALFORMED);
    #[cfg(unix)]
    {
        write(t.path(), "outside.txt", "o\n");
        std::os::unix::fs::symlink(t.path().join("outside.txt"), root.join("link.txt")).unwrap();
    }
    let plan = plan_conversion(&root).unwrap();
    assert_eq!(plan.created(), paths(&["a.yml", "opencollection.yml"]));
    assert_eq!(plan.removed_in_place(), paths(&["a.bru", "bruno.json"]));
    assert_eq!(plan.copied(), paths(&["keep/k.txt"]));
}

#[test]
fn the_name_falls_back_to_the_directory_name() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Old API");
    write(
        &root,
        "bruno.json",
        r#"{"version": "1", "type": "collection"}"#,
    );
    write(&root, "a.bru", REQUEST);
    assert_eq!(plan_conversion(&root).unwrap().name(), "Old API");
}

#[test]
fn every_unreadable_file_is_reported_on_one_line_each() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("legacy");
    legacy(&root);
    write(&root, "bad/one.bru", MALFORMED);
    write(&root, "two.bru", "nonsense {\n}\n");
    let Err(ConvertError::Parse { errors }) = plan_conversion(&root) else {
        panic!("expected Parse");
    };
    let got: Vec<PathBuf> = errors.iter().map(|e| e.path.clone()).collect();
    assert_eq!(got, paths(&["bad/one.bru", "two.bru"]));
    assert!(
        errors.iter().all(|e| e.line >= 1 && !e.message.is_empty()),
        "{errors:?}"
    );
    let shown = errors[0].to_string();
    assert!(
        shown.starts_with(&format!("bad/one.bru:{}: ", errors[0].line)),
        "{shown}"
    );
    assert!(!shown.contains('\n'), "{shown}");
    let summary = ConvertError::Parse { errors }.to_string();
    assert_eq!(summary, "2 of the legacy files could not be read");
}

#[test]
fn invalid_bruno_json_is_the_one_parse_error() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "bruno.json", "{");
    write(t.path(), "a.bru", REQUEST);
    write(t.path(), "bad.bru", MALFORMED);
    let Err(ConvertError::Parse { errors }) = plan_conversion(t.path()) else {
        panic!("expected Parse");
    };
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].path, Path::new("bruno.json"));
    assert!(
        errors[0].line >= 1,
        "a1 reports serde_json's line: {errors:?}"
    );
    assert!(
        errors[0].to_string().starts_with("bruno.json:"),
        "{}",
        errors[0]
    );
}

#[test]
fn targets_already_in_the_tree_are_refused_together() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("legacy");
    legacy(&root);
    write(&root, "a.yml", "x\n");
    write(&root, "f/b.yml", "x\n");
    let Err(ConvertError::TargetExists { paths: got }) = plan_conversion(&root) else {
        panic!("expected TargetExists");
    };
    assert_eq!(got, paths(&["a.yml", "f/b.yml"]));
    write(
        &root,
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: y\n",
    );
    let Err(ConvertError::TargetExists { paths: got }) = plan_conversion(&root) else {
        panic!("expected TargetExists");
    };
    assert_eq!(got, paths(&["a.yml", "f/b.yml", "opencollection.yml"]));
}

#[test]
fn a_folder_without_bruno_json_is_not_legacy() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "a.bru", REQUEST);
    let Err(ConvertError::NotLegacy { root }) = plan_conversion(t.path()) else {
        panic!("expected NotLegacy");
    };
    assert_eq!(root, t.path());
}

#[test]
fn opening_a_legacy_root_says_it_needs_converting() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("legacy");
    legacy(&root);
    let err = Collection::open(&root).unwrap_err();
    assert!(
        matches!(&err, OpenError::LegacyFormat { root: r } if *r == root),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        format!(
            "{} is in the legacy .bru format; convert it to open it",
            root.display()
        )
    );
    assert!(matches!(
        Collection::open_with(&root, &OpenOptions::new()),
        Err(OpenError::LegacyFormat { .. })
    ));
    let bare = tempfile::tempdir().unwrap();
    write(bare.path(), "a.bru", REQUEST);
    assert!(matches!(
        Collection::open(bare.path()),
        Err(OpenError::NotACollection { .. })
    ));
}

#[test]
fn a_root_opencollection_bru_is_refused_rather_than_merged_into_the_root() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("legacy");
    legacy(&root);
    write(&root, "opencollection.bru", REQUEST);
    write(&root, "sub/opencollection.bru", REQUEST);
    let Err(ConvertError::Parse { errors }) = plan_conversion(&root) else {
        panic!("expected Parse");
    };
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].path, Path::new("opencollection.bru"));
    assert_eq!(errors[0].line, 0);
    assert!(
        errors[0].message.contains("opencollection.yml"),
        "{errors:?}"
    );
    // Only the root file is refused: a nested one is an ordinary request.
    fs::remove_file(root.join("opencollection.bru")).unwrap();
    let plan = plan_conversion(&root).unwrap();
    assert!(
        plan.created()
            .contains(&PathBuf::from("sub/opencollection.yml")),
        "{:?}",
        plan.created()
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_fails_the_plan_instead_of_dropping_its_files() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    legacy(t.path());
    let locked = t.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("x.bru"), "").unwrap();
    let _g = crate::common::ModeGuard::set(&locked, 0o000);
    // A superuser reads through the mode bits, so there is nothing to test.
    let readable = fs::read_dir(&locked).is_ok();
    let result = plan_conversion(t.path());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return;
    }
    let err = result.unwrap_err();
    assert!(
        matches!(&err, ConvertError::Parse { errors } if errors[0].path == Path::new("locked")),
        "{err:?}"
    );
}

const ROOT_DOCS: &str = "docs {\n  ROOTDOCS-MARKER\n}\n";

/// Converts into a new folder and returns the text of its `opencollection.yml`.
fn converted_root(plan: &apinata_core::legacy::ConversionPlan, out: &Path) -> String {
    let dir = convert(plan, ConvertTarget::NewFolder(out.join("out"))).unwrap();
    fs::read_to_string(dir.join("opencollection.yml")).unwrap()
}

#[test]
fn a_plain_root_collection_bru_is_merged_and_removed() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("c");
    write(&root, "bruno.json", BRUNO_JSON);
    write(&root, "collection.bru", ROOT_DOCS);
    let plan = plan_conversion(&root).unwrap();
    assert_eq!(
        plan.removed_in_place(),
        paths(&["bruno.json", "collection.bru"])
    );
    assert!(plan.copied().is_empty());
    assert!(converted_root(&plan, t.path()).contains("ROOTDOCS-MARKER"));
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_collection_bru_is_merged_but_never_removed_or_copied() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("c");
    write(&root, "bruno.json", BRUNO_JSON);
    write(t.path(), "outside.bru", ROOT_DOCS);
    std::os::unix::fs::symlink(t.path().join("outside.bru"), root.join("collection.bru")).unwrap();
    let plan = plan_conversion(&root).unwrap();
    assert_eq!(plan.removed_in_place(), paths(&["bruno.json"]));
    assert!(plan.copied().is_empty());
    assert!(converted_root(&plan, t.path()).contains("ROOTDOCS-MARKER"));
}

#[test]
fn an_ignored_root_collection_bru_is_merged_but_never_removed_or_copied() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("c");
    write(
        &root,
        "bruno.json",
        r#"{"version": "1", "name": "c", "type": "collection", "ignore": ["collection.bru"]}"#,
    );
    write(&root, "collection.bru", ROOT_DOCS);
    let plan = plan_conversion(&root).unwrap();
    assert_eq!(plan.removed_in_place(), paths(&["bruno.json"]));
    assert!(plan.copied().is_empty());
    assert!(converted_root(&plan, t.path()).contains("ROOTDOCS-MARKER"));
}

#[cfg(unix)]
#[test]
fn a_dangling_root_collection_bru_symlink_is_ignored() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("c");
    write(&root, "bruno.json", BRUNO_JSON);
    std::os::unix::fs::symlink(t.path().join("missing.bru"), root.join("collection.bru")).unwrap();
    let plan = plan_conversion(&root).unwrap();
    assert_eq!(plan.removed_in_place(), paths(&["bruno.json"]));
    assert!(!converted_root(&plan, t.path()).contains("ROOTDOCS-MARKER"));
}

#[cfg(unix)]
#[test]
fn an_unparsable_symlinked_root_collection_bru_names_collection_bru() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("c");
    write(&root, "bruno.json", BRUNO_JSON);
    write(t.path(), "outside.bru", MALFORMED);
    std::os::unix::fs::symlink(t.path().join("outside.bru"), root.join("collection.bru")).unwrap();
    let ConvertError::Parse { errors } = plan_conversion(&root).unwrap_err() else {
        panic!("expected a parse error");
    };
    assert_eq!(errors[0].path, Path::new("collection.bru"));
}
