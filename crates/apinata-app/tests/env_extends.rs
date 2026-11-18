//! The environment editor's Extends picker offers only parents core would accept, marks a
//! parent the file already names that core refuses, and Save refuses a looping environment.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

type Editor = gpui_kit::Entity<apinata_app::env_editor::EnvEditor>;

fn choices(cx: &mut TestAppContext, ed: &Editor) -> Vec<(String, Option<String>)> {
    ed.read_with(cx, |e, _| e.parent_choices())
}

fn pick(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &Editor, name: &str) {
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select(name, w, cx)))
        .unwrap();
    frame(cx, win);
}

fn save(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &Editor) {
    ed.update(cx, |e, cx| e.save(cx));
    frame(cx, win);
}

fn some(name: &str) -> (String, Option<String>) {
    (name.to_owned(), Some(name.to_owned()))
}

fn none() -> (String, Option<String>) {
    ("None".to_owned(), None)
}

#[gpui_kit::test]
fn a_parent_that_would_loop_is_not_offered(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", "name: base\n"),
        ("environments/dev.yml", "name: dev\nextends: base\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    pick(cx, win, &ed, "base");
    assert_eq!(choices(cx, &ed), vec![none(), some("prod")]);
    cx.update_window(win, |_, w, cx| w.click("env-extends", cx))
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "env-extends-option-prod"));
    assert!(
        !present(cx, win, "env-extends-option-dev"),
        "dev extends base"
    );
    assert!(!present(cx, win, "env-extends-option-base"), "base itself");
    pick(cx, win, &ed, "dev");
    assert_eq!(choices(cx, &ed), vec![none(), some("base"), some("prod")]);
}

#[gpui_kit::test]
fn a_looping_parent_is_refused_as_an_edit(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", "name: base\n"),
        ("environments/dev.yml", "name: dev\nextends: base\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    pick(cx, win, &ed, "base");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_extends(Some("dev".into()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(env_view(cx, &ws, &root, "base").extends, None);
    let error = ed.read_with(cx, |e, _| e.error()).unwrap_or_default();
    assert!(error.contains("base -> dev -> base"), "{error}");
}

#[gpui_kit::test]
fn choosing_writes_extends_and_clearing_removes_it(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_extends(Some("prod".into()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    save(cx, win, &ed);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("extends: prod"), "{text}");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_extends(None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    save(cx, win, &ed);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("extends"), "{text}");
}

#[gpui_kit::test]
fn a_missing_parent_shows_as_missing_and_can_be_changed(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nextends: ghost\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(
        choices(cx, &ed),
        vec![
            none(),
            some("prod"),
            ("ghost (missing)".to_owned(), Some("ghost".to_owned()))
        ]
    );
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_extends(Some("prod".into()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    save(cx, win, &ed);
    let text = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert!(text.contains("extends: prod"), "{text}");
    assert_eq!(choices(cx, &ed), vec![none(), some("prod")]);
}

#[gpui_kit::test]
fn a_file_that_already_loops_is_marked_and_save_refuses(cx: &mut TestAppContext) {
    let dev = "name: dev\nextends: prod\nvariables:\n  - name: host\n    value: x\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("environments/prod.yml", "name: prod\nextends: dev\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(
        choices(cx, &ed),
        vec![none(), ("prod (loops)".to_owned(), Some("prod".to_owned()))]
    );
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    frame(cx, win);
    save(cx, win, &ed);
    let error = ed.read_with(cx, |e, _| e.error()).unwrap_or_default();
    assert!(error.contains("dev -> prod -> dev"), "{error}");
    let text = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert_eq!(text, dev, "a refused save leaves the file alone");
    assert!(env_view(cx, &ws, &root, "dev").dirty);
}
