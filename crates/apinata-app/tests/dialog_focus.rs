//! Focus inside an open dialog survives the focused control going away: a removed row, a
//! locked button or a blur puts it back inside the dialog, so Tab and Escape keep working.

use crate::support;

use apinata_app::command::Command;
use apinata_app::cookie_jar::open_cookie_jar;
use apinata_app::executor::SharedExecutor;
use apinata_app::shell::OpenRuntimeVariables;
use apinata_core::{CookieInfo, ExecOptions, Executor};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::sync::Arc;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: key\n";

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request("r", "http://127.0.0.1:9/")),
    ])
}

// The started app, its window active so focus changes reach the focus-lost listeners.
fn app(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, s.win);
    s
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

// Tab moves focus off `id` and Shift+Tab brings it back, then Escape closes the dialog.
fn keys_alive_from(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    let id = id.into();
    press_key(cx, win, "tab");
    assert!(
        !holds_focus(cx, win, id.clone()),
        "Tab moves focus off {id}"
    );
    press_key(cx, win, "shift-tab");
    assert!(
        holds_focus(cx, win, id.clone()),
        "Shift+Tab comes back to {id}"
    );
    press_key(cx, win, "escape");
    assert!(!dialog_open(cx, win), "Escape closes the dialog");
}

// Opens the cookie jar over a fresh executor holding one cookie per domain in `domains`.
fn jar(cx: &mut TestAppContext, s: &Started, domains: &[&str]) -> Arc<Executor> {
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    for d in domains {
        exec.cookies()
            .insert(CookieInfo::new(*d, "k", "v"))
            .unwrap();
    }
    cx.update(|cx| cx.set_global(SharedExecutor(exec.clone())));
    cx.update_window(s.win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, s.win);
    exec
}

fn jar_domains(exec: &Executor) -> Vec<String> {
    exec.cookies()
        .list()
        .into_iter()
        .map(|c| c.domain)
        .collect()
}

#[gpui_kit::test]
fn deleting_the_last_domain_by_key_lands_on_the_first_control(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    let exec = jar(cx, &s, &["a.test"]);
    enter_on(cx, s.win, ("jar-domain-delete", 0usize));
    frame(cx, s.win);
    assert!(jar_domains(&exec).is_empty());
    assert!(holds_focus(cx, s.win, "jar-add"), "Add cookie takes focus");
    keys_alive_from(cx, s.win, "jar-add");
}

// A guard: the next domain's button takes the deleted one's place and keeps focus.
#[gpui_kit::test]
fn deleting_a_domain_with_a_successor_keeps_focus_on_the_next(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    let exec = jar(cx, &s, &["a.test", "b.test"]);
    enter_on(cx, s.win, ("jar-domain-delete", 0usize));
    frame(cx, s.win);
    assert_eq!(jar_domains(&exec), ["b.test"]);
    assert!(holds_focus(cx, s.win, ("jar-domain-delete", 0usize)));
}

// The trailing domain of two has no successor, so focus goes back to the jar's first control.
#[gpui_kit::test]
fn deleting_the_trailing_domain_lands_on_the_first_control(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    let exec = jar(cx, &s, &["a.test", "b.test"]);
    enter_on(cx, s.win, ("jar-domain-delete", 1usize));
    frame(cx, s.win);
    assert_eq!(jar_domains(&exec), ["a.test"]);
    assert!(holds_focus(cx, s.win, "jar-add"), "Add cookie takes focus");
    keys_alive_from(cx, s.win, "jar-add");
}

#[gpui_kit::test]
fn escape_closes_a_dialog_after_focus_was_blurred(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    assert!(dialog_open(cx, s.win));
    cx.update_window(s.win, |_, w, cx| w.blur(cx)).unwrap();
    frame(cx, s.win);
    assert!(
        holds_focus(cx, s.win, "search-input"),
        "focus is back on the query input"
    );
    press_key(cx, s.win, "escape");
    assert!(!dialog_open(cx, s.win), "Escape closes the dialog");
}

#[gpui_kit::test]
fn removing_the_last_runtime_row_by_key_keeps_keys_alive(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("open");
    vars.set_text("id", "1");
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(OpenRuntimeVariables);
    frame(cx, s.win);
    enter_on(cx, s.win, ("runtime-var-delete", 0usize));
    frame(cx, s.win);
    assert!(vars.is_empty());
    assert!(
        holds_focus(cx, s.win, "runtime-vars-add-name"),
        "the add row's name field takes focus"
    );
    keys_alive_from(cx, s.win, "runtime-vars-add-name");
}

// Space on a focused overview eye flips the value, and focus stays on the eye.
#[gpui_kit::test]
fn space_on_the_overview_eye_keeps_focus(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("r.yml"), false, cx));
    cx.run_until_parked();
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("open");
    vars.set_text("key", "hidden-one");
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, ("var-overview-reveal", 0usize)));
    tab_to(cx, s.win, ("var-overview-eye", 0usize));
    press_key(cx, s.win, "space");
    assert!(present(cx, s.win, ("var-overview-hide", 0usize)));
    assert!(
        holds_focus(cx, s.win, ("var-overview-eye", 0usize)),
        "the flipped eye holds focus"
    );
}

fn drawn_value(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).value().map(str::to_owned)
    })
    .unwrap()
    .unwrap_or_default()
}

fn runtime_dialog(cx: &mut TestAppContext, s: &Started, rows: &[(&str, &str)]) {
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("open");
    for (name, value) in rows {
        vars.set_text(*name, *value);
    }
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(OpenRuntimeVariables);
    frame(cx, s.win);
}

#[gpui_kit::test]
fn removing_the_trailing_runtime_row_by_key_rings_the_first_field(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    runtime_dialog(cx, &s, &[("a", "1"), ("b", "2")]);
    platform_enter_on(cx, s.win, ("runtime-var-delete", 1usize));
    assert!(
        ring_on(cx, s.win, ("runtime-var-value", 0usize)),
        "the first row's value shows its ring"
    );
    platform_press(cx, s.win, "x");
    assert!(
        drawn_value(cx, s.win, ("runtime-var-value", 0usize)).contains('x'),
        "typing reaches the first row's value"
    );
}

#[gpui_kit::test]
fn removing_the_only_runtime_row_by_key_rings_the_add_name(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    runtime_dialog(cx, &s, &[("a", "1")]);
    platform_enter_on(cx, s.win, ("runtime-var-delete", 0usize));
    assert!(
        ring_on(cx, s.win, "runtime-vars-add-name"),
        "the add name shows its ring"
    );
    platform_press(cx, s.win, "x");
    assert_eq!(
        drawn_value(cx, s.win, "runtime-vars-add-name"),
        "x",
        "typing reaches it"
    );
    platform_press(cx, s.win, "tab");
    next_frame(cx, s.win);
    assert!(
        ring_on(cx, s.win, "runtime-vars-add-value"),
        "one Tab reaches the second control"
    );
}

#[gpui_kit::test]
fn deleting_the_only_domain_by_key_rings_add_cookie_for_space(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    jar(cx, &s, &["a.test"]);
    platform_enter_on(cx, s.win, ("jar-domain-delete", 0usize));
    assert!(ring_on(cx, s.win, "jar-add"), "Add cookie shows its ring");
    platform_press(cx, s.win, "space");
    assert!(present(cx, s.win, "jar-form"), "Space opens the add form");
}

#[gpui_kit::test]
fn deleting_the_trailing_domain_by_key_tabs_to_the_second_control(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    jar(cx, &s, &["a.test", "b.test"]);
    platform_enter_on(cx, s.win, ("jar-domain-delete", 1usize));
    assert!(ring_on(cx, s.win, "jar-add"), "Add cookie shows its ring");
    platform_press(cx, s.win, "tab");
    next_frame(cx, s.win);
    assert!(
        ring_on(cx, s.win, "jar-search"),
        "one Tab reaches the search"
    );
}

#[gpui_kit::test]
fn add_cookie_by_key_focuses_the_domain_field(cx: &mut TestAppContext) {
    let col = collection();
    let s = app(cx, &col);
    jar(cx, &s, &[]);
    platform_enter_on(cx, s.win, "jar-add");
    assert!(
        ring_on(cx, s.win, "jar-form-domain"),
        "the domain field shows its ring"
    );
    platform_press(cx, s.win, "x");
    assert_eq!(
        drawn_value(cx, s.win, "jar-form-domain"),
        "x",
        "typing reaches it"
    );
}
