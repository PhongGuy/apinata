//! "Add to environment": the intent, the editor applying it, and the error block's button.

use crate::support;

use apinata_app::command::{Command, EnvOp};
use apinata_app::env_editor::EnvEditor;
use apinata_app::response_pane::ResponsePane;
use apinata_app::tabs::TabKind;
use apinata_app::workspace::{EnvIntent, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, WindowOptions,
};
use std::path::Path;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: off\n    value: \"1\"\n    enabled: false\n";
const PROD: &str = "name: prod\n";

fn with_envs() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{nope}}")),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", PROD),
    ])
}

fn without_envs() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{nope}}")),
    ])
}

fn intent(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Option<EnvIntent> {
    ws.read_with(cx, |w, _| w.env_intent(root).cloned())
}

fn environments_tab_is_active(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> bool {
    ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .is_some_and(|e| e.kind == TabKind::Environments)
    })
}

#[gpui_kit::test]
fn the_intent_targets_the_selected_env_else_the_first_else_a_new_one(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    assert_eq!(
        intent(cx, &ws, &root),
        Some(EnvIntent {
            select: Some("dev".into()),
            add_var: "x".into(),
            create: false
        })
    );
    assert!(environments_tab_is_active(cx, &ws));
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("prod".into()), cx)
    });
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "y", cx));
    assert_eq!(
        intent(cx, &ws, &root),
        Some(EnvIntent {
            select: Some("prod".into()),
            add_var: "y".into(),
            create: false
        })
    );

    let bare = without_envs();
    let (_state2, ws2, root2) = open_dir(cx, &bare);
    ws2.update(cx, |w, cx| w.add_variable_to_environment(&root2, "x", cx));
    assert_eq!(
        intent(cx, &ws2, &root2),
        Some(EnvIntent {
            select: None,
            add_var: "x".into(),
            create: true
        })
    );
}

/// (name, value, enabled, unsaved) per row.
fn rows(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> Vec<(String, String, bool, bool)> {
    let table = ed.read_with(cx, |e, _| e.table().clone());
    table.read_with(cx, |t, cx| {
        t.rows(cx)
            .into_iter()
            .map(|r| (r.name, r.value, r.enabled, r.unsaved))
            .collect()
    })
}

fn value_focused(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &Entity<EnvEditor>,
    ix: usize,
) -> bool {
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(ix).cloned())
        .expect("a row");
    cx.update_window(win, |_, w, cx| {
        input.read(cx).focus_handle(cx).is_focused(w)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_new_editor_selects_the_env_and_adds_a_focused_unsaved_row(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("prod".into()), cx)
    });
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("prod")
    );
    let shown = rows(cx, &ed);
    let ix = shown.iter().position(|r| r.0 == "x").expect("the new row");
    assert_eq!(shown[ix], ("x".into(), String::new(), true, true));
    assert!(value_focused(cx, win, &ed, ix));
    assert_eq!(intent(cx, &ws, &root), None, "taken once");
}

#[gpui_kit::test]
fn an_open_editor_takes_the_intent_and_never_duplicates_a_row(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("dev")
    );
    let before = rows(cx, &ed).len();
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "off", cx));
    cx.run_until_parked();
    let shown = rows(cx, &ed);
    assert_eq!(shown.len(), before, "the disabled row is reused");
    let ix = shown.iter().position(|r| r.0 == "off").unwrap();
    assert!(value_focused(cx, win, &ed, ix));
    assert_eq!(intent(cx, &ws, &root), None);
}

#[gpui_kit::test]
fn with_no_environment_it_names_one_then_adds_the_row(cx: &mut TestAppContext) {
    let col = without_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    let (_win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("New environment name")
    );
    ed.update(cx, |e, cx| e.create("qa", cx));
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("qa")
    );
    assert!(rows(cx, &ed).iter().any(|r| r.0 == "x" && r.3));
}

fn pane_after_undefined(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    std::path::PathBuf,
    tempfile::TempDir,
) {
    http_ready(cx);
    let (state, ws, root) = open_dir(cx, col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (win, ws, root, state)
}

fn button_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("response-add-env").label().map(str::to_owned)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_block_offers_add_or_create_and_the_click_opens_the_editor(cx: &mut TestAppContext) {
    let col = with_envs();
    let (win, ws, root, _state) = pane_after_undefined(cx, &col);
    assert_eq!(button_label(cx, win).as_deref(), Some("Add to environment"));
    cx.update_window(win, |_, w, cx| w.click("response-add-env", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        intent(cx, &ws, &root),
        Some(EnvIntent {
            select: Some("dev".into()),
            add_var: "nope".into(),
            create: false
        })
    );
    assert!(environments_tab_is_active(cx, &ws));

    let bare = without_envs();
    let (win2, _ws2, _root2, _state2) = pane_after_undefined(cx, &bare);
    assert_eq!(
        button_label(cx, win2).as_deref(),
        Some("Create environment")
    );
}

#[gpui_kit::test]
fn closing_the_collection_drops_its_intent(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    ws.update(cx, |w, cx| w.close(&root, cx));
    assert_eq!(intent(cx, &ws, &root), None);
    // A root that is not open opens no tab, so it keeps no intent either.
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    assert_eq!(intent(cx, &ws, &root), None);
}

#[gpui_kit::test]
fn a_cancelled_create_forgets_the_variable(cx: &mut TestAppContext) {
    let col = without_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.naming_label()).is_some());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(ed.read_with(cx, |e, _| e.naming_label()), None);
    ed.update(cx, |e, cx| e.create("qa", cx));
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("qa")
    );
    assert!(!rows(cx, &ed).iter().any(|r| r.0 == "x"));
}

#[gpui_kit::test]
fn a_target_deleted_before_the_apply_keeps_the_row_and_shows_an_error(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    assert_eq!(
        intent(cx, &ws, &root).and_then(|i| i.select).as_deref(),
        Some("dev")
    );
    let delete = Command::Env(EnvOp::Delete { name: "dev".into() });
    ws.update(cx, |w, cx| w.send(&root, delete, cx));
    cx.run_until_parked();
    let (_win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("prod")
    );
    assert!(rows(cx, &ed).iter().any(|r| r.0 == "x" && r.3), "kept");
    let error = ed.read_with(cx, |e, _| e.error()).expect("an error");
    assert!(error.contains("dev"), "{error}");
    assert_eq!(intent(cx, &ws, &root), None);
}

#[gpui_kit::test]
fn a_create_that_cannot_be_sent_forgets_the_pending_variable(cx: &mut TestAppContext) {
    let col = without_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.add_variable_to_environment(&root, "x", cx));
    let (_win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(
        ed.read_with(cx, |e, _| e.pending_add().map(str::to_owned))
            .as_deref(),
        Some("x")
    );
    ws.update(cx, |w, cx| w.close(&root, cx));
    ed.update(cx, |e, cx| e.create("qa", cx));
    assert_eq!(
        ed.read_with(cx, |e, _| e.pending_add().map(str::to_owned)),
        None
    );
}
