//! Environment export from the environment editor and the palette.

use crate::support;

use apinata_app::export::{environment_exported_message, environment_file_name};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use std::path::Path;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)])
}

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
}

fn palette(cx: &mut TestAppContext, s: &Started, text: &str) {
    focus_shell(cx, s);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// Answers the save dialog with a fresh file and returns the written JSON.
fn exported(cx: &mut TestAppContext, s: &Started) -> serde_json::Value {
    assert!(cx.did_prompt_for_new_path());
    let before = toasts(cx, s);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("dev.postman_environment.json");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(toasts(cx, s), before + 1);
    serde_json::from_slice(&std::fs::read(&file).expect("written")).unwrap()
}

fn assert_dev(v: &serde_json::Value) {
    assert_eq!(v["name"], "dev");
    let values = v["values"].as_array().unwrap();
    let host = values.iter().find(|x| x["key"] == "host").unwrap();
    assert_eq!(host["value"], "x");
    let token = values.iter().find(|x| x["key"] == "token").unwrap();
    assert_eq!(
        (token["value"].as_str(), token["type"].as_str()),
        (Some(""), Some("secret"))
    );
}

#[test]
fn the_file_name_and_message() {
    assert_eq!(environment_file_name("dev"), "dev.postman_environment.json");
    assert_eq!(environment_file_name("a/b"), "a_b.postman_environment.json");
    let file = Path::new("/tmp/dev.json");
    assert_eq!(
        environment_exported_message(file, false),
        "Exported to /tmp/dev.json"
    );
    assert_eq!(
        environment_exported_message(file, true),
        "Exported to /tmp/dev.json (unsaved edits not included)"
    );
}

#[gpui_kit::test]
fn the_environment_editor_exports_the_selected_environment(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("e"));
    cx.run_until_parked();
    frame(cx, s.win);
    env_menu(cx, s.win, apinata_app::env_editor::ENV_MENU_EXPORT);
    assert_dev(&exported(cx, &s));
}

#[gpui_kit::test]
fn the_palette_exports_the_active_environment(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    palette(cx, &s, "export environment");
    assert_dev(&exported(cx, &s));
}

#[gpui_kit::test]
fn the_palette_without_an_active_environment_says_so(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    let before = toasts(cx, &s);
    palette(cx, &s, "export environment");
    assert!(!cx.did_prompt_for_new_path());
    assert_eq!(toasts(cx, &s), before + 1);
}
