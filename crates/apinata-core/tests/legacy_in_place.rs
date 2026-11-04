//! In-place conversion: the result equals the new-folder result, and refusals change nothing.

use apinata_core::Collection;
use apinata_core::legacy::{ConvertError, ConvertTarget, convert, plan_conversion};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn legacy(root: &Path) {
    write(
        root,
        "bruno.json",
        r#"{"version": "1", "name": "Legacy", "type": "collection"}"#,
    );
    write(root, "a.bru", REQUEST);
    write(root, "f/folder.bru", "meta {\n  name: f\n}\n");
    write(root, "f/b.bru", &REQUEST.replace("name: a", "name: b"));
    write(
        root,
        "environments/Local.bru",
        "vars {\n  host: http://localhost\n}\n",
    );
    write(root, ".env", "TOKEN=x\n");
}

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
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
fn in_place_produces_exactly_the_new_folder_result() {
    let src = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let reference = convert(&plan, ConvertTarget::NewFolder(other.path().join("x"))).unwrap();
    assert_eq!(convert(&plan, ConvertTarget::InPlace).unwrap(), src.path());
    assert_eq!(files(src.path()), files(&reference));
    let c = Collection::open(src.path()).unwrap();
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn a_target_created_after_the_plan_is_refused() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    write(src.path(), "f/b.yml", "mine\n");
    let before = files(src.path());
    let Err(ConvertError::TargetExists { paths }) = convert(&plan, ConvertTarget::InPlace) else {
        panic!("expected TargetExists");
    };
    assert_eq!(paths, [PathBuf::from("f/b.yml")]);
    assert_eq!(files(src.path()), before);
}

#[test]
fn a_source_edited_after_the_plan_is_refused() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    write(
        src.path(),
        "f/b.bru",
        &REQUEST.replace("name: a", "name: c"),
    );
    let before = files(src.path());
    let Err(ConvertError::SourceChanged { path }) = convert(&plan, ConvertTarget::InPlace) else {
        panic!("expected SourceChanged");
    };
    assert_eq!(path, Path::new("f/b.bru"));
    assert_eq!(files(src.path()), before);
}

#[test]
fn left_over_lists_the_legacy_files_still_present_after_a_failed_delete() {
    let src = tempfile::tempdir().unwrap();
    legacy(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let io = |rel: &str| ConvertError::Io {
        path: src.path().join(rel),
        message: "denied".into(),
    };
    fs::remove_file(src.path().join("a.bru")).unwrap();
    let left = plan.left_over(&io("f/b.bru")).expect("a failed delete");
    assert_eq!(
        left,
        [
            "bruno.json",
            "environments/Local.bru",
            "f/b.bru",
            "f/folder.bru"
        ]
        .map(PathBuf::from)
    );
    assert_eq!(
        plan.left_over(&io("a.yml")),
        None,
        "a failed write rolls back"
    );
    let other = ConvertError::SourceChanged {
        path: "a.bru".into(),
    };
    assert_eq!(plan.left_over(&other), None);
}
