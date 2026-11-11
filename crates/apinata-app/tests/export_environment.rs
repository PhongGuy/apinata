//! Environment export from the environment editor, the palette and the global environments
//! dialog: one environment to a file, or every environment to a folder.

use crate::support;

use apinata_app::export::{
    environment_exported_message, environment_file_name, environments_exported_message,
};
use apinata_app::global_env_editor::open_global_env_editor;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use std::collections::BTreeMap;
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
    palette(cx, &s, "export current environment");
    assert_dev(&exported(cx, &s));
}

#[gpui_kit::test]
fn the_palette_without_an_active_environment_says_so(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    let before = toasts(cx, &s);
    palette(cx, &s, "export current environment");
    assert!(!cx.did_prompt_for_new_path());
    assert_eq!(toasts(cx, &s), before + 1);
}

const PROD: &str = "name: prod\nvariables:\n  - name: base\n    value: https://p\n";

fn two() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", PROD),
    ])
}

fn files_in(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            (name, std::fs::read(e.path()).unwrap())
        })
        .collect()
}

// What `apinata export --env-dir` writes for the same environments: core's file set.
fn cli_files(files: Vec<apinata_core::export::EnvironmentFile>) -> BTreeMap<String, Vec<u8>> {
    files
        .into_iter()
        .map(|f| (f.file_name, f.export.json.into_bytes()))
        .collect()
}

fn collection_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let c = apinata_core::Collection::open(root).unwrap();
    cli_files(apinata_core::export::postman_environment_files(&c))
}

// Answers the folder prompt with a fresh folder already holding a stale `dev` file, and
// returns what the folder holds afterwards.
fn exported_to_folder(cx: &mut TestAppContext, s: &Started) -> BTreeMap<String, Vec<u8>> {
    assert!(cx.did_prompt_for_paths());
    let before = toasts(cx, s);
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("dev.postman_environment.json"), "stale").unwrap();
    let chosen = out.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen.clone()]));
    cx.run_until_parked();
    assert_eq!(toasts(cx, s), before + 1);
    files_in(out.path())
}

fn assert_dev_file(files: &BTreeMap<String, Vec<u8>>) {
    let dev = &files["dev.postman_environment.json"];
    assert_dev(&serde_json::from_slice(dev).unwrap());
}

#[test]
fn the_folder_message_counts_the_files() {
    let dir = Path::new("/tmp/out");
    assert_eq!(
        environments_exported_message(dir, 1, false),
        "Exported 1 environment to /tmp/out"
    );
    assert_eq!(
        environments_exported_message(dir, 2, true),
        "Exported 2 environments to /tmp/out (unsaved edits not included)"
    );
}

#[gpui_kit::test]
fn the_environment_editor_exports_every_environment_to_a_folder(cx: &mut TestAppContext) {
    let col = two();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("e"));
    cx.run_until_parked();
    frame(cx, s.win);
    env_menu(cx, s.win, apinata_app::env_editor::ENV_MENU_EXPORT_ALL);
    let files = exported_to_folder(cx, &s);
    assert_eq!(files, collection_files(col.path()));
    assert_eq!(files.len(), 2);
    assert_dev_file(&files);
}

#[gpui_kit::test]
fn the_palette_exports_every_environment_to_a_folder(cx: &mut TestAppContext) {
    let col = two();
    let s = started(cx, &col);
    palette(cx, &s, "export all environments");
    let files = exported_to_folder(cx, &s);
    assert_eq!(files, collection_files(col.path()));
    assert_dev_file(&files);
}

#[gpui_kit::test]
fn the_palette_without_environments_says_so(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let before = toasts(cx, &s);
    palette(cx, &s, "export all environments");
    assert!(!cx.did_prompt_for_paths());
    assert_eq!(toasts(cx, &s), before + 1);
}

#[gpui_kit::test]
fn the_global_environments_dialog_exports_every_global(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    let globals = write_tree(&[("dev.yml", DEV), ("prod.yml", PROD)]);
    let path = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(path, cx));
    cx.run_until_parked();
    let ws = s.ws.clone();
    let editor = cx
        .update_window(s.win, |_, w, cx| open_global_env_editor(ws, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("dev", w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("global-env-export", cx))
        .unwrap();
    cx.run_until_parked();
    let files = exported_to_folder(cx, &s);
    let g = apinata_core::GlobalEnvironments::open(globals.path()).unwrap();
    assert_eq!(
        files,
        cli_files(apinata_core::export::postman_global_environment_files(&g))
    );
    assert_eq!(files.len(), 2);
    assert_dev_file(&files);
}
