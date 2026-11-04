//! `workspace.yml` as a read-only view: entries, validity, unknown keys and environments.

use crate::common::write_tree;
use apinata_core::{WorkspaceError, WorkspaceFile, is_workspace_dir};

const VALID: &str = "opencollection: 1.0.0
info:
  name: Team
  type: workspace
collections:
  - name: Api
    path: collections/api
  - name: Abs
    path: /srv/abs
    remote: https://example.test/x.git
    color: red
  - name: ''
    path: nope
  - name: NoPath
specs:
  - name: Spec
    path: specs/a.yml
docs: hello
theme: dark
";

#[test]
fn reads_a_valid_workspace() {
    let dir = write_tree(&[("workspace.yml", VALID)]);
    assert!(is_workspace_dir(dir.path()));
    let ws = WorkspaceFile::open(dir.path()).expect("open");
    assert_eq!(ws.name(), "Team");
    assert_eq!(ws.dir(), dir.path());
    assert_eq!(ws.collections().len(), 2);
    assert_eq!(ws.skipped(), 2);
    assert_eq!(ws.docs(), "hello");
    assert_eq!(ws.specs().len(), 1);
    assert_eq!(ws.specs()[0].name(), "Spec");
    assert_eq!(ws.specs()[0].path(), "specs/a.yml");
}

#[test]
fn resolves_relative_and_absolute_paths() {
    let dir = write_tree(&[("workspace.yml", VALID)]);
    let ws = WorkspaceFile::open(dir.path()).expect("open");
    let [api, abs] = ws.collections() else {
        panic!("two collections")
    };
    assert_eq!(api.name(), "Api");
    assert_eq!(api.path(), "collections/api");
    assert_eq!(api.root(), dir.path().join("collections/api"));
    assert_eq!(api.remote(), None);
    assert_eq!(abs.root(), std::path::Path::new("/srv/abs"));
    assert_eq!(abs.remote(), Some("https://example.test/x.git"));
}

#[test]
fn keeps_unknown_keys() {
    let dir = write_tree(&[("workspace.yml", VALID)]);
    let ws = WorkspaceFile::open(dir.path()).expect("open");
    assert_eq!(ws.extra(), [("theme".to_string(), "dark".to_string())]);
    assert_eq!(
        ws.collections()[1].extra(),
        [("color".to_string(), "red".to_string())]
    );
}

#[test]
fn specs_that_is_a_map_reads_empty() {
    let dir = write_tree(&[(
        "workspace.yml",
        "info: {name: W, type: workspace}\nspecs: {a: b}\n",
    )]);
    let ws = WorkspaceFile::open(dir.path()).expect("open");
    assert!(ws.specs().is_empty());
    assert!(ws.collections().is_empty());
}

#[test]
fn rejects_what_is_not_a_workspace() {
    let none = write_tree(&[("a.txt", "x")]);
    assert!(!is_workspace_dir(none.path()));
    assert!(matches!(
        WorkspaceFile::open(none.path()),
        Err(WorkspaceError::NotAWorkspace { .. })
    ));
    for text in [
        "- a\n- b\n",
        "info: {name: W, type: collection}\n",
        "info: {type: workspace}\n",
        "info: [unclosed\n",
    ] {
        let dir = write_tree(&[("workspace.yml", text)]);
        assert!(
            matches!(
                WorkspaceFile::open(dir.path()),
                Err(WorkspaceError::Malformed { .. })
            ),
            "{text}"
        );
    }
}

#[test]
fn loads_environments_from_the_environments_dir() {
    let dir = write_tree(&[
        ("workspace.yml", "info: {name: W, type: workspace}\n"),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - name: host\n    value: h\n",
        ),
    ]);
    let ws = WorkspaceFile::open(dir.path()).expect("open");
    let envs = ws.environments().expect("envs");
    assert_eq!(envs.environments().len(), 1);
    assert_eq!(envs.environments()[0].name(), "prod");
}
