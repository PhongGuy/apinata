//! Script edits written as Bruno writes them, from the
//! script-edit parity corpus.

use apinata_core::{Collection, ItemPath, RequestFile, ScriptPhase, script_code};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use ScriptPhase::{PostResponse, PreRequest, Tests};

const TESTS: &str = "test(\"ok\", function() {\n  expect(res.status).to.equal(200);\n});";

fn research() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    kind: String,
    input: String,
    files: Vec<String>,
}

fn cases(kind: &str) -> Vec<Case> {
    let path = research().join("2026-10-11-script-edit-parity/cases.json");
    let all: Vec<Case> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let selected: Vec<Case> = all.into_iter().filter(|c| c.kind == kind).collect();
    assert!(!selected.is_empty(), "no {kind} cases");
    selected
}

fn expected(id: &str, file: &str) -> String {
    fs::read_to_string(
        research()
            .join("2026-10-11-script-edit-parity/expected")
            .join(id)
            .join(file),
    )
    .unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Every mismatch at once, so one run shows the whole picture.
fn report(failures: Vec<String>, total: usize) {
    assert!(
        failures.is_empty(),
        "{} of {total} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn request_edits_match_bruno() {
    let selected = cases("request");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(&case.files[0]);
        fs::copy(research().join(&case.input), &path).unwrap();
        let mut r = RequestFile::load(&path).unwrap();
        match case.id.as_str() {
            "script-add-pre" => r.set_script(PreRequest, "bru.setVar(\"t\", Date.now());"),
            "script-add-all" => {
                r.set_script(PreRequest, "const t = Date.now();\nbru.setVar(\"t\", t);");
                r.set_script(PostResponse, "bru.setVar(\"id\", res.body.id);");
                r.set_script(Tests, TESTS);
            }
            "script-trim" => r.set_script(PreRequest, "\n\n  bru.setVar(\"a\", 1);  \n\n"),
            "script-quote" => r.set_script(PreRequest, "bru.setVar('k', \"v: #1\");"),
            "script-edit-post" => {
                r.set_script(PostResponse, "bru.setVar(\"id\", res.body.data.id);")
            }
            "script-clear-pre" => r.set_script(PreRequest, ""),
            "script-clear-all" => {
                for phase in [PreRequest, PostResponse, Tests] {
                    r.set_script(phase, "");
                }
            }
            "script-untouched" => r.set_name("renamed"),
            other => panic!("no Apinata edit for {other}"),
        }
        r.save().unwrap();
        let actual = fs::read_to_string(&path).unwrap();
        let want = expected(&case.id, &case.files[0]);
        if actual != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {actual:?}",
                case.id
            ));
        }
    }
    report(failures, selected.len());
}

#[test]
fn config_edits_match_bruno() {
    let selected = cases("config");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        copy_tree(&research().join(&case.input), dir.path());
        let mut c = Collection::open(dir.path()).unwrap();
        match case.id.as_str() {
            "folder-script-add" => {
                let f = c
                    .folder_file_mut(&ItemPath::new("folder_1").unwrap())
                    .unwrap();
                f.set_script(PreRequest, "bru.setVar(\"f\", 1);");
                f.save().unwrap();
            }
            "collection-script-add" => {
                let f = c.file_mut();
                f.set_script(PreRequest, "bru.setVar(\"c\", 1);");
                f.set_script(
                    Tests,
                    "test(\"c\", function() {\n  expect(1).to.equal(1);\n});",
                );
                f.save().unwrap();
            }
            other => panic!("no Apinata edit for {other}"),
        }
        for file in &case.files {
            let actual = fs::read_to_string(dir.path().join(file)).unwrap();
            let want = expected(&case.id, file);
            if actual != want {
                failures.push(format!(
                    "{} {file}\n  expected {want:?}\n  actual   {actual:?}",
                    case.id
                ));
            }
        }
    }
    report(failures, selected.len());
}

const MIXED: &str = "info:\n  name: m\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  scripts:\n    - type: x-custom\n      code: keep()\n    - type: before-request\n      code: first()\n    - type: before-request\n      code: second()\n    - type: tests\n      code: t()\n";

fn load(text: &str) -> (TempDir, PathBuf, RequestFile) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("m.yml");
    fs::write(&path, text).unwrap();
    let r = RequestFile::load(&path).unwrap();
    (dir, path, r)
}

#[test]
fn the_getter_reads_the_first_entry_of_each_type() {
    let (_d, _p, r) = load(MIXED);
    assert_eq!(r.script(PreRequest), "first()");
    assert_eq!(r.script(PostResponse), "");
    assert_eq!(r.script(Tests), "t()");
}

#[test]
fn an_unchanged_script_leaves_the_file_clean() {
    let (_d, _p, mut r) = load(MIXED);
    for phase in [PreRequest, PostResponse, Tests] {
        let code = r.script(phase);
        r.set_script(phase, code);
    }
    assert!(!r.is_dirty());
}

#[test]
fn only_the_first_entry_changes_and_others_stay_in_place() {
    let (_d, path, mut r) = load(MIXED);
    r.set_script(PreRequest, "changed()");
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let at = |s: &str| {
        text.find(s)
            .unwrap_or_else(|| panic!("{s} missing in {text}"))
    };
    assert!(
        at("keep()") < at("changed()")
            && at("changed()") < at("second()")
            && at("second()") < at("t()"),
        "{text}"
    );
    assert!(!text.contains("first()"), "{text}");
}

#[test]
fn a_new_entry_goes_between_earlier_and_later_phases() {
    let (_d, path, mut r) = load(
        "info:\n  name: m\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  scripts:\n    - type: tests\n      code: t()\n",
    );
    r.set_script(PostResponse, "post()");
    r.set_script(PreRequest, "pre()");
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let at = |s: &str| text.find(s).unwrap();
    assert!(
        at("pre()") < at("post()") && at("post()") < at("t()"),
        "{text}"
    );
    assert_eq!(
        RequestFile::load(&path).unwrap().script(PostResponse),
        "post()"
    );
}

#[test]
fn script_code_trims_as_javascript_does() {
    assert_eq!(script_code("\n  a();  \n"), "a();");
    assert_eq!(script_code("\u{feff}\u{a0}a()\u{2028}"), "a()");
    assert_eq!(script_code(" \t\n"), "");
}
