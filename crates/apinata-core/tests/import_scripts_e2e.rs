//! A Postman collection with scripts, imported and written: the translated code lands in the
//! right YAML sections, unsupported calls stay and are reported, and the written tree carries no
//! approval marker or script hash (`write_collection` has no trust parameter, so approval cannot
//! happen at this layer).

use apinata_core::MemoryStore;
use apinata_core::import::{NoteKind, parse_postman, write_collection};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn imported_scripts_are_translated_placed_and_carry_no_approval_marker() {
    let bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman/own/scripts-subset.json"),
    )
    .expect("fixture");
    let imported = parse_postman(&bytes).expect("parses");
    let parent = TempDir::new().expect("tempdir");
    let (collection, report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    let root = collection.root().to_path_buf();

    let root_yml = read(&root, "opencollection.yml");
    assert!(root_yml.contains("scripts:"), "root scripts: {root_yml}");
    assert!(root_yml.contains("bru.setEnvVar"), "{root_yml}");
    assert!(root_yml.contains("test("), "{root_yml}");
    assert!(!root_yml.contains("pm."), "{root_yml}");

    let folder = read(&root, "Group/folder.yml");
    assert!(folder.contains("bru.setEnvVar(\"folder_pre\""), "{folder}");

    let checked = read(&root, "Group/Checked.yml");
    assert!(checked.contains("runtime:"), "{checked}");
    assert!(checked.contains("bru.setEnvVar(\"token\""), "{checked}");
    assert!(
        checked.contains("expect(res.getStatus()).to.equal(200)"),
        "{checked}"
    );
    assert!(checked.contains("res.getBody()"), "{checked}");
    assert!(checked.contains("test(\"has id\""), "{checked}");
    assert!(!checked.contains("pm."), "{checked}");

    let unsupported = read(&root, "Group/Unsupported.yml");
    assert!(unsupported.contains("pm.visualizer.set("), "{unsupported}");
    assert!(
        report.notes.iter().any(|n| n.kind == NoteKind::Converted
            && n.detail.contains("pm.visualizer.set")
            && n.file.as_deref() == Some("Group/Unsupported.yml")),
        "{:?}",
        report.notes
    );

    // write_collection takes no trust path, so the written tree must carry no approval marker
    // and no script hash. A hash is 64 lowercase hex digits; no such run may appear anywhere.
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            assert!(!text.contains("script_trust"), "{}", path.display());
            assert!(!text.contains("approved"), "{}", path.display());
            assert!(!has_hex64_run(&text), "{}", path.display());
        }
    }
}

/// True when `text` holds a run of at least 64 consecutive lowercase hex digits.
fn has_hex64_run(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        run = if matches!(c, '0'..='9' | 'a'..='f') {
            run + 1
        } else {
            0
        };
        if run >= 64 {
            return true;
        }
    }
    false
}
