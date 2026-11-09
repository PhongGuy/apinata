//! Export to Postman from the collection menu and the palette, and its file helpers.

use crate::support;

use apinata_app::export::{
    default_file_name, exported_message, html_file_name, openapi_file_name, write_export,
};
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use std::path::Path;
use support::*;

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
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.input(text, cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// Answers the open save dialog with a fresh file; returns the exported request names.
fn export_to_chosen_file(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    assert!(cx.did_prompt_for_new_path());
    let before = toasts(cx, s);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("c.postman_collection.json");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).expect("written")).unwrap();
    assert_eq!(v["info"]["name"], "c");
    assert_eq!(toasts(cx, s), before + 1);
    v["item"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_default_file_name_is_the_sanitized_collection_name() {
    assert_eq!(
        default_file_name("My API"),
        "My API.postman_collection.json"
    );
    assert_eq!(default_file_name("a/b\\c"), "a_b_c.postman_collection.json");
    assert_eq!(default_file_name("  "), "untitled.postman_collection.json");
}

#[test]
fn the_message_counts_what_was_not_carried_over() {
    let file = Path::new("/tmp/c.json");
    assert_eq!(exported_message(file, 0), "Exported to /tmp/c.json");
    assert_eq!(
        exported_message(file, 1),
        "Exported to /tmp/c.json (1 item not carried over)"
    );
    assert_eq!(
        exported_message(file, 3),
        "Exported to /tmp/c.json (3 items not carried over)"
    );
}

#[test]
fn a_failed_write_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("missing/c.json");
    let err = write_export(&file, b"{}").unwrap_err();
    assert!(
        err.starts_with(&format!("cannot write {}: ", file.display())),
        "{err}"
    );
}

#[test]
fn only_the_collection_row_offers_export() {
    assert!(menu_entries(NodeKind::Collection).contains(&MenuEntry::ExportPostman));
    assert!(!menu_entries(NodeKind::Folder).contains(&MenuEntry::ExportPostman));
    assert!(!menu_entries(NodeKind::Request).contains(&MenuEntry::ExportPostman));
    assert_eq!(
        MenuEntry::ExportPostman.label(),
        "Export collection to Postman"
    );
}

#[gpui_kit::test]
fn the_collection_menu_exports_to_the_chosen_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::ExportPostman,
    );
    assert_eq!(export_to_chosen_file(cx, &s), ["a", "b"]);
}

#[gpui_kit::test]
fn the_palette_exports_the_active_collection(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    palette(cx, &s, "export collection");
    assert_eq!(export_to_chosen_file(cx, &s), ["a", "b"]);
}

// Answers the open save dialog with a fresh file; returns the exported operations' summaries.
fn export_openapi_to_chosen_file(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    assert!(cx.did_prompt_for_new_path());
    let before = toasts(cx, s);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("c.openapi.json");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).expect("written")).unwrap();
    assert_eq!(v["openapi"], "3.1.0");
    assert_eq!(v["info"]["title"], "c");
    assert_eq!(toasts(cx, s), before + 1);
    let mut summaries: Vec<String> = v["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|item| item.as_object().unwrap().values())
        .map(|op| op["summary"].as_str().unwrap().to_owned())
        .collect();
    summaries.sort();
    summaries
}

#[test]
fn the_openapi_file_name_is_the_sanitized_collection_name() {
    assert_eq!(openapi_file_name("My API"), "My API.openapi.json");
    assert_eq!(openapi_file_name("a/b\\c"), "a_b_c.openapi.json");
    assert_eq!(openapi_file_name("  "), "untitled.openapi.json");
}

#[test]
fn only_the_collection_row_offers_openapi_export() {
    assert!(menu_entries(NodeKind::Collection).contains(&MenuEntry::ExportOpenApi));
    assert!(!menu_entries(NodeKind::Folder).contains(&MenuEntry::ExportOpenApi));
    assert!(!menu_entries(NodeKind::Request).contains(&MenuEntry::ExportOpenApi));
    assert_eq!(
        MenuEntry::ExportOpenApi.label(),
        "Export collection to OpenAPI"
    );
}

#[gpui_kit::test]
fn the_collection_menu_exports_openapi_to_the_chosen_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::ExportOpenApi,
    );
    assert_eq!(export_openapi_to_chosen_file(cx, &s), ["a", "b"]);
}

#[gpui_kit::test]
fn the_palette_exports_the_active_collection_to_openapi(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    palette(cx, &s, "export collection to openapi");
    assert_eq!(export_openapi_to_chosen_file(cx, &s), ["a", "b"]);
}

// Answers the open save dialog with a fresh file; returns the page's request names, sorted.
fn export_html_to_chosen_file(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    assert!(cx.did_prompt_for_new_path());
    let before = toasts(cx, s);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("c.docs.html");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let page = std::fs::read_to_string(&file).expect("written");
    assert!(page.starts_with("<!doctype html>\n"), "{page}");
    assert!(page.contains("<title>c</title>"), "{page}");
    assert_eq!(toasts(cx, s), before + 1);
    let mut names: Vec<String> = page
        .split("<h3><span class=\"method m-get\">GET</span> ")
        .skip(1)
        .map(|rest| rest[..rest.find("</h3>").unwrap()].to_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_html_file_name_is_the_sanitized_collection_name() {
    assert_eq!(html_file_name("My API"), "My API.docs.html");
    assert_eq!(html_file_name("a/b\\c"), "a_b_c.docs.html");
    assert_eq!(html_file_name("  "), "untitled.docs.html");
}

#[test]
fn only_the_collection_row_offers_html_export() {
    assert!(menu_entries(NodeKind::Collection).contains(&MenuEntry::ExportHtml));
    assert!(!menu_entries(NodeKind::Folder).contains(&MenuEntry::ExportHtml));
    assert!(!menu_entries(NodeKind::Request).contains(&MenuEntry::ExportHtml));
    assert_eq!(
        MenuEntry::ExportHtml.label(),
        "Export collection to HTML documentation"
    );
}

#[gpui_kit::test]
fn the_collection_menu_exports_html_to_the_chosen_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    choose_menu(cx, s.win, "", NodeKind::Collection, MenuEntry::ExportHtml);
    assert_eq!(export_html_to_chosen_file(cx, &s), ["a", "b"]);
}

#[gpui_kit::test]
fn the_palette_exports_the_active_collection_to_html(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    palette(cx, &s, "export collection to html");
    assert_eq!(export_html_to_chosen_file(cx, &s), ["a", "b"]);
}

#[gpui_kit::test]
fn a_yaml_file_name_makes_the_openapi_export_yaml(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::ExportOpenApi,
    );
    assert!(cx.did_prompt_for_new_path());
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("c.openapi.yaml");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let text = std::fs::read_to_string(&file).expect("written");
    assert!(text.starts_with("openapi: 3.1.0\n"), "{text}");
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_err());
}

fn scripted() -> tempfile::TempDir {
    let a = request("a", "http://x/0")
        + "runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('k', 1);\"\n";
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

// Answers the save prompt with a fresh file; returns the request's first pre-request line.
fn saved_pre_request(cx: &mut TestAppContext) -> String {
    assert!(cx.did_prompt_for_new_path());
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("c.postman_collection.json");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).expect("written")).unwrap();
    v["item"][0]["event"][0]["script"]["exec"][0]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn only_the_collection_row_offers_export_keeping_scripts() {
    let entry = MenuEntry::ExportPostmanKeepScripts;
    let entries = menu_entries(NodeKind::Collection);
    assert!(entries.contains(&entry));
    assert!(!menu_entries(NodeKind::Folder).contains(&entry));
    assert!(!menu_entries(NodeKind::Request).contains(&entry));
    assert_eq!(
        entry.label(),
        "Export collection to Postman (keep scripts as written)"
    );
    let at = |e| entries.iter().position(|x| *x == e).unwrap();
    assert_eq!(at(entry), at(MenuEntry::ExportPostman) + 1);
}

#[gpui_kit::test]
fn postman_export_translates_scripts(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::ExportPostman,
    );
    assert_eq!(saved_pre_request(cx), "pm.variables.set('k', 1);");
}

#[gpui_kit::test]
fn the_keep_scripts_entry_exports_them_as_written(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::ExportPostmanKeepScripts,
    );
    assert_eq!(saved_pre_request(cx), "bru.setVar('k', 1);");
}

#[gpui_kit::test]
fn the_palette_exports_keeping_scripts_as_written(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    palette(cx, &s, "keep scripts as written");
    assert_eq!(saved_pre_request(cx), "bru.setVar('k', 1);");
}

#[gpui_kit::test]
fn the_palette_still_exports_translated_scripts(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    palette(cx, &s, "export collection");
    assert_eq!(saved_pre_request(cx), "pm.variables.set('k', 1);");
}
