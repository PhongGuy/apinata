//! Import collection: two prompts, a staged write through core, the collection opened, a report.

use crate::support;

use apinata_app::import_report;
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_core::MemoryStore;
use apinata_core::import::{parse_collection, write_collection};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext,
    WindowOptions,
};
use std::fs;
use std::path::{Path, PathBuf};
use support::*;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// A Postman collection `Shop` with one request; `script` adds a test script, imported with a note.
fn shop(script: bool) -> String {
    let event = if script {
        r#","event":[{"listen":"test","script":{"exec":["pm.test('ok')"]}}]"#
    } else {
        ""
    };
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1"}}{event}}}]}}"#
    )
}

fn shell(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (files, win, view)
}

fn opened(cx: &mut TestAppContext, view: &Entity<Shell>) -> Vec<PathBuf> {
    view.read_with(cx, |v, cx| {
        v.workspace()
            .read(cx)
            .collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect()
    })
}

fn dialog(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

pub fn input(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("shop.json");
    fs::write(&path, text).unwrap();
    path
}

/// Opens the Import dialog and takes its Browse button to the file prompt.
pub fn click_import(cx: &mut TestAppContext, win: AnyWindowHandle) {
    switcher_click(cx, win, "switcher-import-collection");
    cx.update_window(win, |_, w, cx| w.click("import-link-browse", cx))
        .unwrap();
    cx.run_until_parked();
}

/// Answers the file prompt, then the directory prompt.
pub fn answer(cx: &mut TestAppContext, file: PathBuf, into: PathBuf) {
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![file])
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.directories && !o.files && !o.multiple);
        Some(vec![into])
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_import_is_written_by_core_opened_and_reported(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(
        cx,
        input(files.path(), &shop(true)),
        into.path().to_path_buf(),
    );
    let root = into.path().join("Shop");
    assert_eq!(opened(cx, &view), std::slice::from_ref(&root));
    assert!(dialog(cx, win), "notes open the report");
    assert!(view.read_with(cx, |v, _| {
        v.last_import().is_some_and(|r| !r.notes.is_empty())
    }));
    let core = tempfile::tempdir().unwrap();
    let imported = parse_collection(None, shop(true).as_bytes()).unwrap();
    write_collection(core.path(), &imported, &[], &MemoryStore::new()).unwrap();
    for rel in ["opencollection.yml", "Get user.yml"] {
        let read = |dir: &Path| fs::read_to_string(dir.join(rel)).unwrap();
        assert_eq!(read(&root), read(&core.path().join("Shop")), "{rel}");
    }
}

#[gpui_kit::test]
fn a_clean_import_opens_without_a_dialog(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(
        cx,
        input(files.path(), &shop(false)),
        into.path().to_path_buf(),
    );
    assert_eq!(opened(cx, &view).len(), 1);
    assert!(!dialog(cx, win));
}

#[gpui_kit::test]
fn an_unrecognized_file_is_refused_before_asking_where(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    click_import(cx, win);
    assert!(cx.did_prompt_for_paths());
    let file = input(files.path(), r#"{"other":1}"#);
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories);
        Some(vec![file])
    });
    cx.run_until_parked();
    assert!(
        !cx.did_prompt_for_paths(),
        "no folder prompt for a file that cannot import"
    );
    assert!(dialog(cx, win), "the failure shows at once");
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn a_cancelled_prompt_imports_nothing(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    click_import(cx, win);
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert!(opened(cx, &view).is_empty());
    assert!(!dialog(cx, win));
}

#[gpui_kit::test]
fn the_key_starts_an_import(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    cx.update(apinata_app::keymap::init);
    {
        let mut v = VisualTestContext::from_window(win, cx);
        v.update(|w, cx| view.read(cx).focus_handle(cx).focus(w, cx));
        v.simulate_keystrokes(&key("shift-i"));
        v.run_until_parked();
    }
    assert!(
        present(cx, win, "import-link-dialog"),
        "the chord opens the dialog"
    );
    cx.update_window(win, |_, w, cx| w.click("import-link-browse", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
}

/// `Shop` whose one request uses an auth type the import does not support.
pub fn shop_with_oauth() -> String {
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1","auth":{{"type":"hawk"}}}}}}]}}"#
    )
}

fn report_of(text: &str) -> apinata_core::import::ImportReport {
    let into = tempfile::tempdir().unwrap();
    let imported = parse_collection(None, text.as_bytes()).unwrap();
    write_collection(into.path(), &imported, &[], &MemoryStore::new())
        .unwrap()
        .1
}

#[test]
fn report_lines_name_each_note_without_backticks() {
    let report = report_of(&shop_with_oauth());
    let lines = import_report::lines(&report);
    assert_eq!(lines.len(), report.notes.len());
    assert!(
        lines.iter().any(
            |l| l.starts_with("Get user") && l.contains("hawk authentication is not supported")
        ),
        "{lines:?}"
    );
    assert!(lines.iter().all(|l| !l.contains('`')), "{lines:?}");
    assert!(lines.iter().all(|l| !l.starts_with("Dropped")), "{lines:?}");
    assert!(
        lines.iter().all(|l| !l.to_lowercase().contains("bruno")),
        "{lines:?}"
    );
}

#[gpui_kit::test]
fn cancelling_the_folder_prompt_writes_nothing(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    click_import(cx, win);
    let file = input(files.path(), &shop(true));
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "a good file asks where it goes");
    cx.simulate_path_prompt_response(|o| {
        assert!(o.directories && !o.files);
        None
    });
    cx.run_until_parked();
    assert!(opened(cx, &view).is_empty());
    assert!(!dialog(cx, win));
}

#[test]
fn failure_text_starts_with_a_capital() {
    assert_eq!(
        import_report::failure_text("not a recognized collection format; supported: x"),
        "Not a recognized collection format; supported: x"
    );
    assert_eq!(import_report::failure_text("élan"), "Élan");
    assert_eq!(import_report::failure_text(""), "");
    assert_eq!(import_report::failure_text("Already"), "Already");
}

#[test]
fn the_summary_counts_items_environments_and_notes() {
    let report = report_of(&shop_with_oauth());
    let n = report.notes.len();
    let notes = if n == 1 {
        "1 note".to_string()
    } else {
        format!("{n} notes")
    };
    assert_eq!(
        import_report::summary(&report),
        format!("1 item, 0 environments imported; {notes}.")
    );
}

#[test]
fn notes_group_under_plain_headings() {
    let report = report_of(&shop_with_oauth());
    let groups = import_report::groups(&report);
    assert_eq!(groups[0].0, "Changed");
    assert!(groups.iter().all(|(_, lines)| !lines.is_empty()));
    let total: usize = groups.iter().map(|(_, l)| l.len()).sum();
    assert_eq!(total, report.notes.len());
}

#[gpui_kit::test]
fn the_report_dialog_shows_a_summary_and_closes_with_its_button(cx: &mut TestAppContext) {
    let (files, win, _view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(
        cx,
        input(files.path(), &shop_with_oauth()),
        into.path().to_path_buf(),
    );
    assert!(dialog(cx, win));
    for id in ["import-summary", "import-report-close"] {
        assert!(present(cx, win, id), "{id}");
    }
    assert!(present(cx, win, ("import-group", 0usize)));
    assert!(present(cx, win, ("import-note", 0usize)));
    cx.update_window(win, |_, w, cx| w.click("import-report-close", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!dialog(cx, win));
}

/// `Shop` with `n` requests, each using an auth type the import does not support.
fn shop_with_oauth_requests(n: usize) -> String {
    let items: Vec<String> = (1..=n)
        .map(|i| {
            format!(
                r#"{{"name":"Get user {i}","request":{{"method":"GET","url":"https://x.test/users/{i}","auth":{{"type":"hawk"}}}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{}]}}"#,
        items.join(",")
    )
}

#[gpui_kit::test]
fn the_wheel_scrolls_a_long_report_to_its_last_note(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, ScrollDelta, ScrollEvent, point};
    let (files, win, _view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(
        cx,
        input(files.path(), &shop_with_oauth_requests(60)),
        into.path().to_path_buf(),
    );
    assert!(dialog(cx, win));
    let last = (0usize..)
        .take_while(|&ix| present(cx, win, ("import-note", ix)))
        .last()
        .unwrap();
    // The last note's bottom against the top of the Close button below the notes.
    let gap = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let note = w.find(("import-note", last)).bounds();
            let close = w.find("import-report-close").bounds();
            close.top() - note.bottom()
        })
        .unwrap()
    };
    assert!(gap(cx) < gpui_kit::px(0.), "the long report starts clipped");
    cx.update_window(win, |_, w, cx| {
        let position = w.find(("import-note", 0usize)).bounds().center();
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
        "the last note scrolled into view"
    );
}

#[test]
fn a_file_failure_starts_with_the_error_and_ends_with_the_path() {
    assert_eq!(
        import_report::failure_at(
            std::path::Path::new("/tmp/x/bad.json"),
            "missing field `name`"
        ),
        "Missing field \"name\" (/tmp/x/bad.json)"
    );
}

#[test]
fn a_backtick_in_the_failure_path_stays_as_typed() {
    assert_eq!(
        import_report::failure_at(Path::new("/tmp/a`b/bad.json"), "missing field `name`"),
        "Missing field \"name\" (/tmp/a`b/bad.json)"
    );
}

#[test]
fn an_open_failure_names_its_path_once() {
    let root = Path::new("/tmp/a`b");
    let cases = [
        (
            "/tmp/a`b: No such file or directory (os error 2)",
            "No such file or directory (os error 2) (/tmp/a`b)",
        ),
        (
            "/tmp/a`b/opencollection.yml: unknown field `x`",
            "Unknown field \"x\" (/tmp/a`b/opencollection.yml)",
        ),
        (
            "/tmp/a`b is not a collection: no `opencollection.yml`",
            "/tmp/a`b is not a collection: no \"opencollection.yml\"",
        ),
        (
            "an internal error stopped the open",
            "An internal error stopped the open (/tmp/a`b)",
        ),
    ];
    for (message, shown) in cases {
        assert_eq!(import_report::open_failure(root, message), shown);
    }
}

#[test]
fn an_unparsable_import_shows_plain_quotes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("c.json");
    fs::write(&file, r#"{"id":"x","name":"c","order":[],"requests":[]}"#).unwrap();
    let Err(message) = apinata_app::shell::parse_bundles(&file) else {
        panic!("an unsupported schema parses");
    };
    assert!(
        message.contains("schema \"") && !message.contains('`'),
        "{message}"
    );
}

#[gpui_kit::test]
fn the_report_has_one_close_at_the_right(cx: &mut TestAppContext) {
    let (files, win, _view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(
        cx,
        input(files.path(), &shop_with_oauth()),
        into.path().to_path_buf(),
    );
    assert!(dialog(cx, win));
    assert!(!present(cx, win, "close"), "no corner close beside Close");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let footer = w.find("dialog-footer").bounds();
        assert_eq!(
            w.find("import-report-close").bounds().right(),
            footer.right()
        );
    })
    .unwrap();
}

/// An OpenAPI spec named `Pets` with one root server per listed `(name, url)`.
fn pets(servers: &[(&str, &str)]) -> String {
    let servers: Vec<String> = servers
        .iter()
        .map(|(name, url)| {
            let name = if name.is_empty() {
                String::new()
            } else {
                format!(r#","description":"{name}""#)
            };
            format!(r#"{{"url":"{url}"{name}}}"#)
        })
        .collect();
    format!(
        r#"{{"openapi":"3.0.0","info":{{"title":"Pets","version":"1"}},"servers":[{}],"paths":{{"/pets":{{"get":{{"summary":"List","responses":{{"200":{{"description":"ok"}}}}}}}}}}}}"#,
        servers.join(",")
    )
}

/// Imports `text` through the Import dialog into a fresh folder; answers the collection's
/// selected environment.
fn import_selection(text: &str, cx: &mut TestAppContext) -> Option<String> {
    let (files, win, view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(cx, input(files.path(), text), into.path().to_path_buf());
    let root = into.path().join("Pets");
    assert_eq!(opened(cx, &view), std::slice::from_ref(&root));
    view.read_with(cx, |v, cx| {
        v.workspace().read(cx).selected_environment(&root, cx)
    })
}

#[gpui_kit::test]
fn an_import_with_one_environment_selects_it(cx: &mut TestAppContext) {
    let selected = import_selection(&pets(&[("", "https://pets.test")]), cx);
    assert_eq!(selected.as_deref(), Some("Environment 1"));
}

#[gpui_kit::test]
fn an_import_with_several_environments_selects_the_first_one_imported(cx: &mut TestAppContext) {
    let text = pets(&[("Staging", "https://s.test"), ("Live", "https://l.test")]);
    assert_eq!(import_selection(&text, cx).as_deref(), Some("Staging"));
}

#[gpui_kit::test]
fn an_import_without_environments_selects_none(cx: &mut TestAppContext) {
    assert_eq!(import_selection(&pets(&[]), cx), None);
}
