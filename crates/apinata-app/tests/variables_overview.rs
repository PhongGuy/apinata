//! The Variables overview: a title bar button opens a read-only list of the active request's
//! variables grouped by scope, with shadowed rows marked and secrets masked.

use crate::support;

use apinata_app::command::Command;
use apinata_app::env_table::SECRET_HIDDEN;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const ROOT_VARS: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: collection.test\n    - name: shared\n      value: collection\n";
const DEV: &str =
    "name: dev\nvariables:\n  - name: shared\n    value: env\n  - secret: true\n    name: key\n";
const FOLDER: &str = "info:\n  name: a\n  type: folder\n  seq: 1\nrequest:\n  variables:\n    - name: shared\n      value: folder\n";

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT_VARS),
        ("environments/dev.yml", DEV),
        ("a/folder.yml", FOLDER),
        ("a/r.yml", &request("r", "http://127.0.0.1:9/{{host}}")),
    ])
}

#[gpui_kit::test]
fn the_button_opens_the_active_requests_variables_grouped_and_marked(cx: &mut TestAppContext) {
    let col = collection();
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    let key = apinata_core::Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "key");
    apinata_core::SecretStore::set(store.as_ref(), &key, "hunter2-value").unwrap();
    let s = started_with_store(cx, &col, store);
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a/r.yml"), false, cx));
    cx.run_until_parked();
    assert!(present(cx, s.win, "variables-button"));
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    let label = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    };
    // Folder first (innermost), then the environment and collection rows it shadows.
    assert_eq!(
        label(cx, ("var-overview-heading", 0)).as_deref(),
        Some("Folder")
    );
    assert_eq!(
        label(cx, ("var-overview-name", 0)).as_deref(),
        Some("shared")
    );
    assert_eq!(
        label(cx, ("var-overview-value", 0)).as_deref(),
        Some("folder")
    );
    assert!(!present(cx, s.win, ("var-overview-shadowed", 0usize)));
    let shadowed = (0usize..8)
        .filter(|i| present(cx, s.win, ("var-overview-shadowed", *i)))
        .count();
    assert_eq!(shadowed, 2);
    let all: Vec<_> = (0usize..8)
        .filter_map(|i| label(cx, ("var-overview-value", i)))
        .collect();
    assert!(all.iter().any(|v| v == SECRET_HIDDEN), "{all:?}");
    assert!(!all.iter().any(|v| v.contains("hunter2")), "{all:?}");
}

#[gpui_kit::test]
fn without_an_active_request_the_dialog_says_so(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "var-overview-empty"));
}

#[gpui_kit::test]
fn a_narrow_window_drops_the_button_but_the_palette_keeps_the_command(cx: &mut TestAppContext) {
    use gpui_kit::{VisualTestContext, px, size};
    let col = collection();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(400.), px(700.)));
    frame(cx, s.win);
    assert!(!present(cx, s.win, "variables-button"));
    assert!(
        apinata_app::commands::registry()
            .iter()
            .any(|c| c.id == "environment.variables_overview")
    );
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(700.)));
    frame(cx, s.win);
    assert!(present(cx, s.win, "variables-button"));
}

#[gpui_kit::test]
fn a_failed_lookup_opens_the_dialog_with_a_note(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::variables_overview::open_failed(w, cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "var-overview-failed"));
}

#[gpui_kit::test]
fn shadowed_rows_leave_the_scrollbar_its_gutter_in_a_narrow_window(cx: &mut TestAppContext) {
    use apinata_app::style::SCROLLBAR_GUTTER;
    use gpui_kit::{VisualTestContext, px, size};
    let col = collection();
    let s = started(cx, &col);
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a/r.yml"), false, cx));
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(440.), px(500.)));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let content = w.find("variables-overview").bounds();
        let limit = content.right() - SCROLLBAR_GUTTER + px(0.5);
        let label = w.find(("var-overview-shadowed", 2usize)).bounds();
        assert!(
            label.right() <= limit,
            "label {label:?} runs under the scrollbar of {content:?}"
        );
    })
    .unwrap();
}

// The read-only overview leads to the editable runtime variables dialog.
#[gpui_kit::test]
fn the_overview_offers_to_edit_the_runtime_variables(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a/r.yml"), false, cx));
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "var-overview-edit-runtime"));
    assert!(!present(cx, s.win, "runtime-vars"));
    cx.update_window(s.win, |_, w, cx| w.click("var-overview-edit-runtime", cx))
        .unwrap();
    frame(cx, s.win);
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "runtime-vars"));
    assert!(!present(cx, s.win, "variables-overview"));
}

// The overview's scroll box clips on every side, flush with the full-width button.
#[gpui_kit::test]
fn the_edit_button_keeps_its_ring_inside_the_scroll_box(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a/r.yml"), false, cx));
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let clip = w.find("variables-overview-scroll").bounds();
        let b = w.find("var-overview-edit-runtime").bounds();
        assert!(
            b.left() - FOCUS_RING_ROOM >= clip.left(),
            "{b:?} in {clip:?}"
        );
        assert!(
            b.right() + FOCUS_RING_ROOM <= clip.right(),
            "{b:?} in {clip:?}"
        );
        assert!(
            b.bottom() + FOCUS_RING_ROOM <= clip.bottom(),
            "{b:?} in {clip:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_on_the_focused_edit_button_opens_the_runtime_variables(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a/r.yml"), false, cx));
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    enter_on(cx, s.win, "var-overview-edit-runtime");
    frame(cx, s.win);
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "runtime-vars"),
        "Enter pressed the button"
    );
}

// The overview over a request whose sensitive runtime value is masked, and that row's eye.
fn overview_with_secret(cx: &mut TestAppContext) -> (tempfile::TempDir, Started, usize) {
    let col = collection();
    let s = started(cx, &col);
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a/r.yml"), false, cx));
    cx.run_until_parked();
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("open");
    vars.set_text("key", "hidden-one");
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    let eye = (0usize..8)
        .find(|i| present(cx, s.win, ("var-overview-reveal", *i)))
        .expect("an eye on the sensitive row");
    (col, s, eye)
}

fn overview_open(cx: &mut TestAppContext, s: &Started) -> bool {
    present(cx, s.win, "variables-overview")
}

fn press(cx: &mut TestAppContext, s: &Started, key: &str) {
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn enter_after_clicking_an_eye_keeps_the_overview_open(cx: &mut TestAppContext) {
    let (_col, s, eye) = overview_with_secret(cx);
    cx.update_window(s.win, |_, w, cx| w.click(("var-overview-eye", eye), cx))
        .unwrap();
    frame(cx, s.win);
    assert!(
        present(cx, s.win, ("var-overview-hide", eye)),
        "the click revealed"
    );
    press(cx, &s, "enter");
    assert!(
        overview_open(cx, &s),
        "Enter after a click keeps the dialog"
    );
    assert!(
        present(cx, s.win, ("var-overview-hide", eye)),
        "Enter on the dialog host toggles nothing"
    );
}

#[gpui_kit::test]
fn enter_right_after_opening_keeps_the_overview_open(cx: &mut TestAppContext) {
    let (_col, s, _eye) = overview_with_secret(cx);
    press(cx, &s, "enter");
    assert!(
        overview_open(cx, &s),
        "Enter with nothing focused keeps the dialog"
    );
}

#[gpui_kit::test]
fn enter_on_a_focused_eye_reveals_and_keeps_the_overview_open(cx: &mut TestAppContext) {
    let (_col, s, eye) = overview_with_secret(cx);
    enter_on(cx, s.win, ("var-overview-eye", eye));
    frame(cx, s.win);
    assert!(
        present(cx, s.win, ("var-overview-hide", eye)),
        "Enter revealed"
    );
    assert!(overview_open(cx, &s), "Enter on the eye keeps the dialog");
}

#[gpui_kit::test]
fn escape_still_closes_the_overview(cx: &mut TestAppContext) {
    let (_col, s, eye) = overview_with_secret(cx);
    cx.update_window(s.win, |_, w, cx| w.click(("var-overview-eye", eye), cx))
        .unwrap();
    frame(cx, s.win);
    press(cx, &s, "escape");
    assert!(!overview_open(cx, &s), "Escape closes");
}

#[gpui_kit::test]
fn enter_on_the_focused_x_closes_the_overview(cx: &mut TestAppContext) {
    let (_col, s, _eye) = overview_with_secret(cx);
    enter_on(cx, s.win, "close");
    frame(cx, s.win);
    assert!(!overview_open(cx, &s), "Enter on the x closes");
}
