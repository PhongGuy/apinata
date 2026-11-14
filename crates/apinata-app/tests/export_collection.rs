//! Export to Postman from the collection menu and the palette, and its file helpers.

use crate::support;

use apinata_app::export::{
    default_file_name, exported_message, html_file_name, openapi_file_name, report_groups,
    report_summary, write_export,
};
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_core::export::ExportNote;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use std::path::{Path, PathBuf};
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

fn lost(source: &str, detail: &str) -> ExportNote {
    ExportNote::new(source, detail)
}

fn changed(source: &str, detail: &str) -> ExportNote {
    ExportNote::changed(source, detail)
}

#[test]
fn the_message_counts_only_what_was_not_carried_over() {
    let file = Path::new("/tmp/exports/c.json");
    let translated = changed("a / pre-request script", "script translated");
    assert_eq!(exported_message(file, &[]), "Exported to c.json");
    assert_eq!(
        exported_message(file, &[lost("a / tags", "t"), translated.clone()]),
        "Exported to c.json (1 item not carried over)"
    );
    assert_eq!(
        exported_message(file, &[lost("a", "x"), lost("b", "x"), lost("c", "x")]),
        "Exported to c.json (3 items not carried over)"
    );
    assert_eq!(
        exported_message(file, &[translated.clone(), translated.clone()]),
        "Exported to c.json (2 items changed to fit)"
    );
    assert_eq!(
        report_summary(file, &[lost("a / tags", "t"), translated.clone()]),
        "1 item was not carried over to c.json."
    );
    assert_eq!(
        report_summary(file, &[translated]),
        "Everything was carried over to c.json; 1 item was changed to fit."
    );
}

#[test]
fn the_report_lists_each_kind_once_with_its_places_under_it() {
    let asserts = "assertions have no place";
    let notes = [
        lost("Posts / List posts / assertions", asserts),
        lost("Posts / List posts / tags", "tags were left out: smoke"),
        changed(
            "Users / List users / pre-request script",
            "script `translated`",
        ),
        lost("Posts / Get post / assertions", asserts),
        lost("Users / Get user / assertions", asserts),
        lost("Users / Get user / tags", "tags were left out: smoke, data"),
        lost("Users / Get user / timeout", "the 5 ms timeout"),
    ];
    assert_eq!(
        report_groups(&notes),
        vec![
            (
                Some("Left out"),
                vec![
                    "Assertions have no place\nPosts / List posts / assertions\nPosts / Get post / assertions\nUsers / Get user / assertions".to_owned(),
                    "Tags were left out\nPosts / List posts / tags: smoke\nUsers / Get user / tags: smoke, data".to_owned(),
                    "The 5 ms timeout\nUsers / Get user / timeout".to_owned(),
                ]
            ),
            (
                Some("Changed"),
                vec!["Script translated\nUsers / List users / pre-request script".to_owned()]
            ),
        ]
    );
    // A note about a whole item names the item itself, not a member of it.
    let skipped = "only HTTP requests are exported";
    assert_eq!(
        report_groups(&[lost("f / g", skipped), lost("h / g", skipped)]),
        vec![(
            Some("Left out"),
            vec!["Only HTTP requests are exported\nf / g\nh / g".to_owned()]
        )]
    );
    assert_eq!(
        report_groups(&[lost("collection / headers", "h")]),
        vec![(Some("Left out"), vec!["H\ncollection / headers".to_owned()])]
    );
    // A name in backticks may hold ": " itself; it never splits the kind from its place.
    let broken = "environment `a: b` extends `c`, which does not exist";
    assert_eq!(
        report_groups(&[
            lost("collection / a: b", broken),
            lost("global / a: b", broken)
        ]),
        vec![(
            Some("Left out"),
            vec![
                "Environment a: b extends c, which does not exist\ncollection / a: b\nglobal / a: b"
                    .to_owned()
            ]
        )]
    );
}

#[test]
fn a_failed_write_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("missing/c.json");
    let err = write_export(&file, b"{}").unwrap_err();
    let folder = dir.path().join("missing");
    assert_eq!(
        err,
        format!(
            "cannot write {}: the folder {} does not exist",
            file.display(),
            folder.display()
        )
    );
}

#[test]
fn only_the_collection_row_offers_export() {
    assert!(menu_entries(NodeKind::Collection).contains(&MenuEntry::ExportPostman));
    assert!(!menu_entries(NodeKind::Folder).contains(&MenuEntry::ExportPostman));
    assert!(!menu_entries(NodeKind::Request).contains(&MenuEntry::ExportPostman));
    assert_eq!(
        MenuEntry::ExportPostman.label(),
        "Export collection to Postman, converting scripts…"
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
        "Export collection to OpenAPI…"
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
        "Export collection to HTML documentation…"
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
        "Export collection to Postman, keeping scripts as written…"
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
fn the_palette_exports_copying_scripts_unchanged(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    palette(cx, &s, "keeping scripts as written");
    assert_eq!(saved_pre_request(cx), "bru.setVar('k', 1);");
}

#[gpui_kit::test]
fn the_palette_still_exports_translated_scripts(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    palette(cx, &s, "export collection");
    assert_eq!(saved_pre_request(cx), "pm.variables.set('k', 1);");
}

// One HTTP request plus `n` gRPC items, which a Postman export leaves out.
fn with_grpc_items(n: usize) -> tempfile::TempDir {
    let mut files = vec![
        ("opencollection.yml".to_owned(), ROOT.to_owned()),
        ("a.yml".to_owned(), request("a", "http://x/a")),
    ];
    for i in 1..=n {
        let text = format!("info:\n  name: g{i}\n  type: grpc\n  seq: {}\n", i + 1);
        files.push((format!("g{i:02}.yml"), text));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    write_tree(&files)
}

// `n` HTTP requests, each with its own timeout, so the report has `n` lines that do not group.
fn with_timed_requests(n: usize) -> tempfile::TempDir {
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    for i in 1..=n {
        let text = request(&format!("r{i:02}"), "http://x/a").replacen(
            "  seq: 1\n",
            &format!("  seq: {i}\nsettings:\n  timeout: {i}\n"),
            1,
        );
        files.push((format!("r{i:02}.yml"), text));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    write_tree(&files)
}

// Exports to Postman through the collection menu into a fresh file and returns its path.
fn export_postman(cx: &mut TestAppContext, s: &Started, out: &tempfile::TempDir) -> PathBuf {
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::ExportPostman,
    );
    let file = out.path().join("c.postman_collection.json");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    file
}

// What an element shows in a freshly drawn frame: its value, or else its label.
fn shown(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) -> String {
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let found = w.find(id.clone());
        found
            .value()
            .or(found.label())
            .map(str::to_owned)
            .unwrap_or_default()
    })
    .unwrap()
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[gpui_kit::test]
fn show_details_gathers_the_items_the_export_left_out(cx: &mut TestAppContext) {
    let col = with_grpc_items(2);
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    export_postman(cx, &s, &out);
    assert!(
        present(cx, s.win, "export-details"),
        "the toast offers details"
    );
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click("export-details", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(dialog_open(cx, &s));
    assert_eq!(
        shown(cx, &s, "export-summary"),
        "2 items were not carried over to c.postman_collection.json."
    );
    assert_eq!(
        shown(cx, &s, ("export-note", 0usize)),
        "Only HTTP requests are exported; this gRPC item was left out\ng1\ng2"
    );
    assert!(!present(cx, s.win, ("export-note", 1usize)));
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click("export-report-close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn a_clean_export_offers_no_details(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    export_postman(cx, &s, &out);
    assert!(toasts(cx, &s) >= 1, "the export toasts");
    assert!(!present(cx, s.win, "export-details"));
}

// The toast's action opens the folder that holds the exported file.
#[gpui_kit::test]
fn the_export_toast_shows_the_file_in_its_folder(cx: &mut TestAppContext) {
    use apinata_app::open_external::{self, FileOpener};
    use std::sync::{Arc, Mutex};
    let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let log = opened.clone();
    let opener: FileOpener = Arc::new(move |p, _| log.lock().unwrap().push(p.to_path_buf()));
    let base = tempfile::tempdir().unwrap();
    let b = base.path().to_path_buf();
    cx.update(|cx| open_external::install(opener, b, cx));
    let col = two_requests();
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    export_postman(cx, &s, &out);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click("export-show-folder", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(*opened.lock().unwrap(), vec![out.path().to_path_buf()]);
}

// A kind's places sit on their own lines under it, indented past its first letter.
#[gpui_kit::test]
fn the_report_lists_the_places_under_their_kind(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = with_grpc_items(2);
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    export_postman(cx, &s, &out);
    cx.update_window(s.win, |_, w, cx| w.click("export-details", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let note = w.find(("export-note", 0usize)).bounds();
        let within = w.within(("export-note", 0usize));
        let kind = within.find("report-kind");
        assert_eq!(
            kind.label(),
            Some("Only HTTP requests are exported; this gRPC item was left out")
        );
        let places: Vec<_> = (0..2usize)
            .map(|i| within.find(("report-place", i)))
            .collect();
        assert_eq!(places[0].label(), Some("g1"));
        assert_eq!(places[1].label(), Some("g2"));
        let (kind, g1, g2) = (kind.bounds(), places[0].bounds(), places[1].bounds());
        assert!(note.contains(&g2.center()), "{g2:?} in {note:?}");
        assert!(g1.top() >= kind.bottom() && g2.top() >= g1.bottom());
        assert!(g1.left() > kind.left(), "{g1:?} indented under {kind:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_long_export_report_scrolls_to_its_last_item(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{InputEvent as _, ScrollDelta, ScrollEvent, point};
    let col = with_timed_requests(60);
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    export_postman(cx, &s, &out);
    cx.update_window(s.win, |_, w, cx| w.click("export-details", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, s.win, ("export-note", 59usize)));
    // The last item's bottom against the top of the Close button below the list.
    let gap = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let note = w.find(("export-note", 59usize)).bounds();
            let close = w.find("export-report-close").bounds();
            close.top() - note.bottom()
        })
        .unwrap()
    };
    assert!(gap(cx) < gpui_kit::px(0.), "the long list starts clipped");
    cx.update_window(s.win, |_, w, cx| {
        let position = w.find(("export-note", 0usize)).bounds().center();
        w.dispatch_event(
            ScrollEvent {
                position,
                delta: ScrollDelta::Lines(point(0., -1000.)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        gap(cx) >= gpui_kit::px(0.),
        "the last item scrolled into view"
    );
}

#[gpui_kit::test]
fn the_export_report_keeps_its_lines_clear_of_the_scrollbar(cx: &mut TestAppContext) {
    use apinata_app::style::SCROLLBAR_GUTTER;
    use gpui_kit::test::TestWindowExt as _;
    let col = with_timed_requests(60);
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    export_postman(cx, &s, &out);
    cx.update_window(s.win, |_, w, cx| w.click("export-details", cx))
        .unwrap();
    cx.run_until_parked();
    let (view, note) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let view = w.find("export-report-scroll").bounds();
            (view, w.find(("export-note", 0usize)).bounds())
        })
        .unwrap();
    assert_eq!(
        view.right() - note.right(),
        SCROLLBAR_GUTTER,
        "{note:?} in {view:?}"
    );
}

const TRANSLATE: &str = "Export collection to Postman, converting scripts…";
const UNCHANGED: &str = "Export collection to Postman, keeping scripts as written…";

#[gpui_kit::test]
fn the_collection_menu_says_what_each_postman_export_does_with_scripts(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(row(""), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let labels = cx
        .update_window(s.win, |_, w, _| {
            [
                MenuEntry::ExportPostman,
                MenuEntry::ExportPostmanKeepScripts,
            ]
            .map(|e| {
                let ix = menu_index(w, e.label());
                let found = w.within("popup-menu").find(ix);
                found.label().unwrap_or_default().to_owned()
            })
        })
        .unwrap();
    assert_eq!(labels, [TRANSLATE, UNCHANGED]);
}

#[gpui_kit::test]
fn the_palette_says_what_each_postman_export_does_with_scripts(cx: &mut TestAppContext) {
    use apinata_app::palette::{index_path, palette_items};
    use apinata_app::shell::{ExportCollection, ExportCollectionPostmanKeepScripts};
    use gpui_kit::Action;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("postman", cx))
        .unwrap();
    frame(cx, s.win);
    let items = palette_items();
    let labels = cx
        .update_window(s.win, |_, w, _| {
            [
                &ExportCollection as &dyn Action,
                &ExportCollectionPostmanKeepScripts,
            ]
            .map(|action| {
                let ix = items
                    .iter()
                    .position(|e| e.action.partial_eq(action))
                    .expect("listed");
                let found = w.within("command-palette").find(index_path(&items, ix));
                found.label().unwrap_or_default().to_owned()
            })
        })
        .unwrap();
    assert_eq!(labels, [TRANSLATE, UNCHANGED]);
}

// Each export asks for a file first, so its label ends with an ellipsis.
#[test]
fn every_export_that_asks_for_a_file_ends_with_an_ellipsis() {
    let commands = apinata_app::commands::registry();
    for id in [
        "collection.export",
        "collection.export_keep_scripts",
        "collection.export_openapi",
        "collection.export_html",
        "request.export_har",
    ] {
        let command = commands.iter().find(|c| c.id == id).expect(id);
        assert!(command.label.ends_with('…'), "{id}: {}", command.label);
    }
}
