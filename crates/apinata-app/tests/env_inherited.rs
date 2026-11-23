//! The environment editor's read-only block of inherited variables, the marks on own rows that
//! override them, and Override, which copies an inherited row into the environment.

use crate::support;

use apinata_app::inherited_section::{header_text, row_text};
use apinata_core::{Collection, InheritedState, MemoryStore, SecretStore, VarValue};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::sync::Arc;
use support::*;

type Editor = gpui_kit::Entity<apinata_app::env_editor::EnvEditor>;

const BASE: &str = "name: base\nvariables:\n  - name: deep\n    value: d\n  - name: off\n    value: x\n    disabled: true\n  - name: token\n    secret: true\n";
const MID: &str = "name: mid\nextends: base\nvariables:\n  - name: host\n    value: mid-host\n  - name: port\n    value:\n      type: number\n      data: \"8080\"\n";
const LEAF: &str = "name: leaf\nextends: mid\nvariables:\n  - name: host\n    value: leaf-host\n";

fn chain() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", BASE),
        ("environments/mid.yml", MID),
        ("environments/leaf.yml", LEAF),
    ])
}

fn pick(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &Editor, name: &str) {
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select(name, w, cx)))
        .unwrap();
    frame(cx, win);
}

fn override_row(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &Editor, name: &str) {
    ed.update(cx, |e, cx| e.override_inherited(name, cx));
    frame(cx, win);
}

#[gpui_kit::test]
fn the_block_lists_what_the_chain_gives_and_marks_overrides(cx: &mut TestAppContext) {
    let col = chain();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    pick(cx, win, &ed, "leaf");
    let view = env_view(cx, &ws, &root, "leaf");
    let names: Vec<&str> = view.inherited.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["deep", "host", "port", "token"]);
    let host = view.inherited.iter().find(|v| v.name == "host").unwrap();
    assert_eq!(host.state, InheritedState::OverriddenByOwn);
    assert_eq!(
        header_text("mid", view.inherited.len()),
        "Inherited from mid (4)"
    );
    assert!(present(cx, win, "env-inherited"));
    assert!(present(cx, win, ("env-inherited-row", 0usize)));
    // The own host row carries the mark; the inherited host row offers no Override.
    assert!(present(cx, win, ("env-row-overrides", 0usize)));
    assert!(!present(cx, win, ("env-inherited-override", 1usize)));
    assert!(present(cx, win, ("env-inherited-override", 0usize)));
    // Collapsing hides the rows but keeps the header.
    cx.update_window(win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click("env-inherited-toggle", cx)
    })
    .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "env-inherited"));
    assert!(!present(cx, win, ("env-inherited-row", 0usize)));
}

#[gpui_kit::test]
fn a_row_names_its_source_only_past_the_direct_parent(cx: &mut TestAppContext) {
    let col = chain();
    let (_state, ws, root) = open_dir(cx, &col);
    let view = env_view(cx, &ws, &root, "leaf");
    let text = |name: &str| {
        let var = view.inherited.iter().find(|v| v.name == name).unwrap();
        row_text(var, "mid")
    };
    assert!(text("deep").contains("from base"), "{}", text("deep"));
    assert!(!text("port").contains("from"), "{}", text("port"));
}

#[gpui_kit::test]
fn marks_follow_unsaved_edits(cx: &mut TestAppContext) {
    let col = chain();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    pick(cx, win, &ed, "leaf");
    override_row(cx, win, &ed, "deep");
    let view = env_view(cx, &ws, &root, "leaf");
    let deep = view.inherited.iter().find(|v| v.name == "deep").unwrap();
    assert_eq!(deep.state, InheritedState::OverriddenByOwn);
}

#[gpui_kit::test]
fn override_copies_a_plain_row_and_marks_the_environment_dirty(cx: &mut TestAppContext) {
    let col = chain();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    pick(cx, win, &ed, "leaf");
    override_row(cx, win, &ed, "port");
    let view = env_view(cx, &ws, &root, "leaf");
    assert!(view.dirty);
    let port = view.variables.iter().find(|v| v.name == "port").unwrap();
    assert!(!port.secret);
    let typed = VarValue::Typed {
        kind: "number".into(),
        data: "8080".into(),
    };
    assert!(port.value == typed, "the number type is kept");
    let rows = ed.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    assert!(rows.iter().any(|r| r.name == "port"), "{rows:?}");
}

#[gpui_kit::test]
fn an_overridden_secret_gets_an_empty_row_and_no_value(cx: &mut TestAppContext) {
    let col = chain();
    let store = Arc::new(MemoryStore::default());
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("base", "token");
    store.set(&key, "s3cret-value").unwrap();
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    pick(cx, win, &ed, "leaf");
    let view = env_view(cx, &ws, &root, "leaf");
    for var in &view.inherited {
        assert!(!row_text(var, "mid").contains("s3cret-value"));
    }
    override_row(cx, win, &ed, "token");
    let view = env_view(cx, &ws, &root, "leaf");
    let token = view.variables.iter().find(|v| v.name == "token").unwrap();
    assert!(token.secret);
    assert!(token.value == VarValue::Text(String::new()));
    let own_key = Collection::open(col.path())
        .unwrap()
        .secret_key("leaf", "token");
    assert_eq!(store.get(&own_key).unwrap(), None);
}

#[gpui_kit::test]
fn a_broken_chain_shows_its_warning(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nextends: ghost\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    let view = env_view(cx, &ws, &root, "dev");
    assert!(view.inherited.is_empty());
    let warning = view.inherited_warning.clone().expect("warns");
    assert!(warning.contains("ghost"), "{warning}");
    assert!(present(cx, win, "env-inherited-warning"));
}
