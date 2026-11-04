//! Post-response variables written as Bruno writes them, from the
//! post-response parity corpus.

use apinata_core::{Collection, ItemPath, PostResponseVar, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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
    let path = research().join("2026-10-11-post-response-parity/cases.json");
    let all: Vec<Case> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let selected: Vec<Case> = all.into_iter().filter(|c| c.kind == kind).collect();
    assert!(!selected.is_empty(), "no {kind} cases");
    selected
}

fn expected(id: &str, file: &str) -> String {
    fs::read_to_string(
        research()
            .join("2026-10-11-post-response-parity/expected")
            .join(id)
            .join(file),
    )
    .unwrap()
}

fn disabled(name: &str, expression: &str) -> PostResponseVar {
    let mut var = PostResponseVar::new(name, expression);
    var.set_enabled(false);
    var
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
        let mut vars = r.post_response_vars();
        match case.id.as_str() {
            "prv-add" => {
                vars = vec![
                    PostResponseVar::new("token", "res.body.token"),
                    disabled("id", "res.body.id"),
                ]
            }
            "prv-add-ambiguous" => {
                vars = vec![
                    PostResponseVar::new("123", "true"),
                    PostResponseVar::new("null", "007"),
                ]
            }
            "prv-add-typed" => vars = vec![PostResponseVar::new("x", "res.status")],
            "prv-edit-expression" => vars[1].set_expression("res.body.data.id"),
            "prv-rename" => vars[0].set_name("session"),
            "prv-disable" => vars[0].set_enabled(false),
            "prv-enable" => vars[1].set_enabled(true),
            "prv-remove-first" => {
                vars.remove(0);
            }
            "prv-clear" => vars.clear(),
            "prv-untouched" => r.set_name("renamed"),
            other => panic!("no Apinata edit for {other}"),
        }
        if case.id != "prv-untouched" {
            r.set_post_response_vars(vars);
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

const OTHER: &str = "info:\n  name: o\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.a\n        method: jsonq\n      variable:\n        name: a\n        scope: runtime\n    - type: notify\n      phase: before-request\n";

#[test]
fn other_actions_survive_an_edit_and_a_clear() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("o.yml");
    fs::write(&path, OTHER).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    let mut vars = r.post_response_vars();
    vars[0].set_expression("res.body.b");
    r.set_post_response_vars(vars);
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("expression: res.body.b") && text.contains("type: notify"),
        "{text}"
    );
    assert!(
        text.find("set-variable").unwrap() < text.find("type: notify").unwrap(),
        "{text}"
    );

    r.set_post_response_vars(Vec::new());
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("set-variable") && text.contains("type: notify"),
        "{text}"
    );
}

const NOTIFY_FIRST: &str = "info:\n  name: o\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  actions:\n    - type: notify\n      phase: before-request\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.a\n        method: jsonq\n      variable:\n        name: a\n        scope: runtime\n";

#[test]
fn other_actions_keep_their_place() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("o.yml");
    fs::write(&path, NOTIFY_FIRST).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    let vars = r.post_response_vars();
    r.set_post_response_vars(vars.clone());
    assert!(!r.is_dirty(), "an unchanged list keeps the order");
    let mut more = vars;
    more[0].set_expression("res.body.b");
    more.push(PostResponseVar::new("c", "res.status"));
    r.set_post_response_vars(more);
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in {text}"))
    };
    assert!(
        at("type: notify") < at("res.body.b") && at("res.body.b") < at("name: c"),
        "{text}"
    );
}

#[test]
fn an_unchanged_list_leaves_the_file_clean() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("o.yml");
    fs::write(&path, OTHER).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    let vars = r.post_response_vars();
    r.set_post_response_vars(vars);
    assert!(!r.is_dirty());
}

#[test]
fn a_new_var_round_trips_through_the_reader() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("o.yml");
    fs::write(&path, OTHER).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    r.set_post_response_vars(vec![disabled("id", "res.body.id")]);
    r.save().unwrap();
    let back = RequestFile::load(&path).unwrap().post_response_vars();
    assert_eq!(back, [disabled("id", "res.body.id")]);
}

#[test]
fn config_edits_match_bruno() {
    let selected = cases("config");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        copy_tree(&research().join(&case.input), dir.path());
        let mut c = Collection::open(dir.path()).unwrap();
        let token = PostResponseVar::new("token", "res.body.token");
        match case.id.as_str() {
            "folder-prv-add" => {
                let f = c
                    .folder_file_mut(&ItemPath::new("folder_1").unwrap())
                    .unwrap();
                f.set_post_response_vars(vec![token]);
                f.save().unwrap();
            }
            "collection-prv-add" => {
                let f = c.file_mut();
                f.set_post_response_vars(vec![token, disabled("id", "res.body.id")]);
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

#[test]
fn config_vars_read_back_and_clear() {
    let dir = TempDir::new().unwrap();
    copy_tree(
        &research().join("2026-10-09-edit-parity/normalized/cli-docs"),
        dir.path(),
    );
    let mut c = Collection::open(dir.path()).unwrap();
    let before = fs::read_to_string(dir.path().join("opencollection.yml")).unwrap();
    let f = c.file_mut();
    f.set_post_response_vars(vec![PostResponseVar::new("a", "res.status")]);
    assert_eq!(
        f.post_response_vars(),
        [PostResponseVar::new("a", "res.status")]
    );
    f.set_post_response_vars(Vec::new());
    f.save().unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("opencollection.yml")).unwrap(),
        before
    );
}

#[test]
fn skip_hints_match_the_run_time_reasons() {
    let hint = |n: &str, e: &str| PostResponseVar::new(n, e).skip_hint();
    assert_eq!(hint("id", "res.body.id"), None);
    assert_eq!(hint("t", " res.headers['x-token'] "), None);
    assert_eq!(hint("s", "res.status"), None);
    assert_eq!(
        hint("id", "res('id')").as_deref(),
        Some("`res('id')` needs a JavaScript runtime")
    );
    assert_eq!(
        hint("id", "res.body.items.map(i => i.id)").as_deref(),
        Some("`res.body.items.map(i => i.id)` needs a JavaScript runtime")
    );
    assert_eq!(
        hint("id", "res.cookies.a").as_deref(),
        Some("`res.cookies.a` needs a JavaScript runtime")
    );
    assert_eq!(
        hint("", "res.body.id").as_deref(),
        Some("the variable has no name")
    );
    assert_eq!(
        hint("a b", "res.body.id").as_deref(),
        Some("`a b` is not a valid variable name; use ASCII letters, digits, `-`, `_` and `.`")
    );
}
