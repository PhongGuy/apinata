//! Import collection: a file prompt, a staged write through core into the dialog's Create in
//! folder, the collection opened, a report.

use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::{import_link, import_report};
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

/// Points the open Import dialog's Create in folder at `into`.
pub fn create_in(cx: &mut TestAppContext, into: &Path) {
    let dialog = cx.read(import_link::current).expect("dialog open");
    let into = into.to_path_buf();
    dialog.update(cx, |d, cx| d.set_parent(into, cx));
}

/// Creates in `into`, answers the file prompt, then imports; no folder prompt follows.
pub fn answer(cx: &mut TestAppContext, file: PathBuf, into: PathBuf) {
    assert!(cx.did_prompt_for_paths());
    create_in(cx, &into);
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![file])
    });
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_paths(), "no folder prompt follows");
    submit(cx);
}

/// Presses the open Import dialog's Import button.
pub fn submit(cx: &mut TestAppContext) {
    assert!(cx.read(import_link::current).is_some(), "dialog open");
    let win = cx
        .update(|cx| cx.active_window().or_else(|| cx.windows().first().copied()))
        .expect("a window");
    cx.update_window(win, |_, w, cx| w.click("import-link-import", cx))
        .unwrap();
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
    submit(cx);
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
    assert!(
        present(cx, win, "import-link-dialog"),
        "the dialog stays for another try"
    );
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
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1","auth":{{"type":"asap"}}}}}}]}}"#
    )
}

fn report_of(text: &str) -> apinata_core::import::ImportReport {
    let into = tempfile::tempdir().unwrap();
    let imported = parse_collection(None, text.as_bytes()).unwrap();
    write_collection(into.path(), &imported, &[], &MemoryStore::new())
        .unwrap()
        .1
}

// Every line of every group, in group order.
fn all_lines(report: &apinata_core::import::ImportReport) -> Vec<String> {
    import_report::groups(report)
        .into_iter()
        .flat_map(|(_, lines)| lines)
        .collect()
}

#[test]
fn report_lines_name_each_note_without_backticks() {
    let lines = all_lines(&report_of(&shop_with_oauth()));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("asap authentication is not supported")),
        "{lines:?}"
    );
    assert!(lines.iter().all(|l| !l.contains('`')), "{lines:?}");
    assert!(lines.iter().all(|l| !l.starts_with("Dropped")), "{lines:?}");
    assert!(
        lines.iter().all(|l| !l.to_lowercase().contains("bruno")),
        "{lines:?}"
    );
}

#[test]
fn each_kind_of_note_is_one_line_with_its_places_and_files_under_it() {
    let report = report_of(&shop_with_oauth_requests(2));
    let asap: Vec<_> = report
        .notes
        .iter()
        .filter(|n| n.detail.starts_with("`asap`"))
        .collect();
    assert_eq!(asap.len(), 2, "{:?}", report.notes);
    let place = |n: &apinata_core::import::ImportNote| {
        format!(
            "{}, in {}",
            n.place().replace('`', ""),
            n.file.as_deref().unwrap()
        )
    };
    let expected = format!(
        "asap authentication is not supported; set to none\n{}\n{}",
        place(asap[0]),
        place(asap[1])
    );
    assert!(
        expected.contains("\nrequest Get user 1, auth, in "),
        "{expected}"
    );
    let lines = all_lines(&report);
    assert!(lines.contains(&expected), "{lines:?}");
    assert!(lines.iter().all(|l| !l.contains(" (")), "{lines:?}");
}

#[test]
fn a_name_in_backticks_keeps_its_case_and_only_outside_words_are_capitalised() {
    use import_report::{KindNote, kind_lines};
    let note = |detail| KindNote {
        detail,
        place: "p".to_owned(),
        file: None,
    };
    assert_eq!(
        kind_lines([
            note("`api_key` is not a defined security scheme"),
            note("no `Server` given"),
        ]),
        [
            "api_key is not a defined security scheme\np",
            "No Server given\np"
        ]
    );
}

#[gpui_kit::test]
fn a_good_file_lands_in_the_create_in_folder_without_a_second_prompt(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    click_import(cx, win);
    let into = tempfile::tempdir().unwrap();
    let parent = into.path().to_path_buf();
    create_in(cx, &parent);
    let file = input(files.path(), &shop(false));
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    cx.run_until_parked();
    submit(cx);
    assert!(!cx.did_prompt_for_paths(), "the dialog already said where");
    assert_eq!(opened(cx, &view), vec![parent.join("Shop")]);
    assert!(!present(cx, win, "import-link-dialog"));
}

#[test]
fn a_script_note_reads_as_its_owner_and_script() {
    let text = format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"event":[{{"listen":"prerequest","script":{{"exec":["pm.environment.set('a', 1);"]}}}}],"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1"}}}}]}}"#
    );
    let lines = all_lines(&report_of(&text));
    assert!(
        lines.iter().any(|l| l.starts_with(
            "1 Postman call was converted to Apinata's script API\nCollection pre-request script, in "
        )),
        "{lines:?}"
    );
    let trust = "Imported scripts ask before they first run in the app or the MCP server, and apinata run runs them as your own command\n";
    assert_eq!(
        lines.iter().filter(|l| l.starts_with(trust)).count(),
        1,
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .all(|l| !l.contains("event[") && !l.contains("pm call")),
        "{lines:?}"
    );
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
fn the_summary_counts_requests_folders_and_environments() {
    assert_eq!(
        import_report::summary(&report_of(&shop_with_oauth())),
        "Imported 1 request."
    );
    let mut report = apinata_core::import::ImportReport::default();
    report.items = ["Users", "Users/a.yml", "Users/b.yml", "Users/c.yml"]
        .map(|p| apinata_core::ItemPath::new(p).unwrap())
        .to_vec();
    report.requests = 3;
    assert_eq!(
        import_report::summary(&report),
        "Imported 3 requests in 1 folder."
    );
    report.environments = vec!["Live".to_owned()];
    assert_eq!(
        import_report::summary(&report),
        "Imported 3 requests in 1 folder and 1 environment."
    );
    let mut envs = apinata_core::import::ImportReport::default();
    envs.environments = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(import_report::summary(&envs), "Imported 2 environments.");
}

#[test]
fn a_kind_that_repeats_its_heading_is_left_out() {
    let text = format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"{{{{host}}}}/users/1"}}}}]}}"#
    );
    let groups = import_report::groups(&report_of(&text));
    let (_, lines) = groups
        .iter()
        .find(|(heading, _)| *heading == "Needs a value")
        .expect("a Needs a value group");
    let rows: Vec<&str> = lines[0].lines().collect();
    assert_eq!(rows[0], "", "no kind row: {lines:?}");
    assert!(
        rows[1].starts_with("collection: host; nothing imported defines it"),
        "{lines:?}"
    );
}

#[test]
fn notes_group_under_plain_headings() {
    let report = report_of(&shop_with_oauth());
    let groups = import_report::groups(&report);
    assert_eq!(groups[0].0, "Changed");
    assert!(groups.iter().all(|(_, lines)| !lines.is_empty()));
    // Each note is one place row under its kind's line.
    let places: usize = groups
        .iter()
        .flat_map(|(_, l)| l)
        .map(|l| l.lines().count() - 1)
        .sum();
    assert_eq!(places, report.notes.len());
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
                r#"{{"name":"Get user {i}","request":{{"method":"GET","url":"https://x.test/users/{i}","auth":{{"type":"asap"}}}}}}"#
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
    let v9 = "https://schema.getpostman.com/json/collection/v9.0.0/collection.json";
    fs::write(
        &file,
        format!(r#"{{"info":{{"name":"c","schema":"{v9}"}},"item":[]}}"#),
    )
    .unwrap();
    let Err(message) = apinata_app::shell::parse_bundles(&file, false) else {
        panic!("an unsupported schema parses");
    };
    assert!(
        message.contains(&format!("schema \"{v9}\"")) && !message.contains('`'),
        "{message}"
    );
    fs::write(&file, r#"{"id":"x","name":"c","order":[],"requests":[]}"#).unwrap();
    let Err(message) = apinata_app::shell::parse_bundles(&file, false) else {
        panic!("a v1 file parses");
    };
    assert!(
        message.contains("older Postman format") && !message.contains('`'),
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
    assert_eq!(selected.as_deref(), Some("pets.test"));
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

// A note's file is shown in full however deep it sits, wrapping inside its place's width.
#[gpui_kit::test]
fn a_long_file_in_the_report_wraps_in_full(cx: &mut TestAppContext) {
    let request = r#"{"name":"Get user","request":{"method":"GET","url":"https://x.test/u","auth":{"type":"asap"}}}"#;
    let mut item = request.to_owned();
    for i in 0..8 {
        item = format!(r#"{{"name":"a-rather-long-folder-name-{i}","item":[{item}]}}"#);
    }
    let text = format!(r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{item}]}}"#);
    let (files, win, _view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(cx, input(files.path(), &text), into.path().to_path_buf());
    assert!(dialog(cx, win));
    let (file, kind, place, name) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let note = w.within(("import-note", 0usize));
            let file = note.find(("report-file", 0usize));
            let name = file.label().unwrap_or_default().to_owned();
            let file = file.bounds();
            let kind = note.find("report-kind").bounds();
            let place = note.find(("report-place", 0usize)).bounds();
            (file, kind, place, name)
        })
        .unwrap();
    assert!(name.ends_with("/Get user.yml"), "{name}");
    assert!(
        file.size.height > kind.size.height,
        "{name} wraps onto more lines"
    );
    assert!(file.right() <= place.right(), "{file:?} stays in {place:?}");
}

/// The first note's first place row and its file, laid out after importing `text`.
fn place_and_file(
    text: &str,
    cx: &mut TestAppContext,
) -> (
    String,
    gpui_kit::Bounds<gpui_kit::Pixels>,
    gpui_kit::Bounds<gpui_kit::Pixels>,
) {
    let (files, win, _view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, win);
    answer(cx, input(files.path(), text), into.path().to_path_buf());
    assert!(dialog(cx, win));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        // The first note about a request's own file, past the collection's.
        let ix = (0usize..20)
            .find(|&i| {
                w.try_find(("import-note", i)).is_some_and(|n| {
                    let label = n.label().unwrap_or_default();
                    label.contains(".yml") && !label.contains("opencollection.yml")
                })
            })
            .expect("a note about a request");
        let note = w.within(("import-note", ix));
        let place = note.find(("report-place", 0usize));
        let label = place.label().unwrap_or_default().to_owned();
        (
            label,
            place.bounds(),
            note.find(("report-file", 0usize)).bounds(),
        )
    })
    .unwrap()
}

// A description that wraps hands its file to the next line, where the phrase continues,
// rather than leaving it alone beside the first line.
#[gpui_kit::test]
fn a_wrapped_report_place_is_followed_by_its_file(cx: &mut TestAppContext) {
    let long = "Fetch the complete order history of a customer ".repeat(4);
    let text = shop(true).replace("Get user", long.trim());
    let (label, place, file) = place_and_file(&text, cx);
    assert!(label.contains(", in "), "{label}");
    assert!(
        file.top() > place.top(),
        "{label}: {file:?} sits under the description in {place:?}"
    );
    assert_eq!(
        file.bottom(),
        place.bottom(),
        "{label}: the file ends the row"
    );
    assert_eq!(
        file.left(),
        place.left(),
        "{label}: the file starts its line"
    );
}

// A short description keeps its file beside it on the same line.
#[gpui_kit::test]
fn a_short_report_place_keeps_its_file_beside_it(cx: &mut TestAppContext) {
    let (label, place, file) = place_and_file(&shop(true), cx);
    assert!(label.contains(", in "), "{label}");
    assert_eq!(file.top(), place.top(), "{label}: one line");
    assert_eq!(file.bottom(), place.bottom(), "{label}: one line");
    assert!(
        file.left() > place.left(),
        "{label}: the file follows the description"
    );
}
