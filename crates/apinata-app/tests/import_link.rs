//! Import collection from a link: the dialog behind the Import action, fetching over a local
//! wiremock server, handing the bytes to the same parse and write path as Browse.

use crate::import_collection::create_in;
use crate::support::*;

use apinata_app::import_link::{self, ImportLinkDialog, Phase};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_core::MemoryStore;
use apinata_core::import::write_collection;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::fs;
use std::path::{Path, PathBuf};
use wiremock::ResponseTemplate;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

const OPENAPI: &str = r#"{"openapi":"3.0.0","info":{"title":"Pets","version":"1"},"servers":[{"url":"https://pets.test"}],"paths":{"/pets":{"get":{"operationId":"listPets","summary":"List pets","responses":{"200":{"description":"ok"}}}}}}"#;

fn postman() -> String {
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1"}}}}]}}"#
    )
}

fn shell(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    crate::support::init(cx);
    http_ready(cx);
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

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

/// Opens the dialog from the Import action and types `link` into it.
fn open_with(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    link: &str,
) -> Entity<ImportLinkDialog> {
    switcher_click(cx, win, "switcher-import-collection");
    frame(cx, win);
    assert!(
        !cx.did_prompt_for_paths(),
        "the action opens the dialog, not the file prompt"
    );
    let view = cx.read(import_link::current).expect("dialog open");
    let (v, link) = (view.clone(), link.to_owned());
    cx.update_window(win, |_, w, cx| {
        v.update(cx, |d, cx| d.set_link(&link, w, cx))
    })
    .unwrap();
    view
}

fn phase(cx: &mut TestAppContext, view: &Entity<ImportLinkDialog>) -> Phase {
    view.read_with(cx, |d, _| d.phase().clone())
}

fn error(cx: &mut TestAppContext, view: &Entity<ImportLinkDialog>) -> String {
    view.read_with(cx, |d, _| d.error().unwrap_or_default().to_owned())
}

/// Waits until the import has written something into `into`; no folder prompt comes first.
fn landed(cx: &mut TestAppContext, view: &Entity<Shell>, into: &Path) {
    wait_for(cx, "the import to open", |cx| !opened(cx, view).is_empty());
    assert!(!cx.did_prompt_for_paths(), "the dialog already said where");
    assert!(opened(cx, view).iter().all(|r| r.starts_with(into)));
}

fn serve(cx: &mut TestAppContext, response: ResponseTemplate) -> wiremock::MockServer {
    mock_server(cx, response)
}

/// How many requests the mock server has seen so far.
fn received(cx: &mut TestAppContext, server: &wiremock::MockServer) -> usize {
    cx.update(|cx| apinata_app::tokio_bridge::handle(cx))
        .block_on(server.received_requests())
        .unwrap_or_default()
        .len()
}

fn files_of(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.push((rel, fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[gpui_kit::test]
fn an_openapi_link_writes_what_browse_writes(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let server = serve(cx, ResponseTemplate::new(200).set_body_string(OPENAPI));
    let dlg = open_with(cx, win, &format!("{}/pets.json", server.uri()));
    let into = tempfile::tempdir().unwrap();
    create_in(cx, into.path());
    click(cx, win, "import-link-import");
    assert_eq!(phase(cx, &dlg), Phase::Fetching);
    landed(cx, &view, into.path());
    let root = into.path().join("Pets");
    assert_eq!(opened(cx, &view), std::slice::from_ref(&root));
    assert!(!present(cx, win, "import-link-dialog"));

    // The same bytes through the core path Browse uses give identical files.
    let core = tempfile::tempdir().unwrap();
    let mut options = apinata_core::import::CollectionOptions::default();
    options.name = Some("pets".into());
    let bundles =
        apinata_core::import::parse_collections(None, OPENAPI.as_bytes(), &options).unwrap();
    write_collection(
        core.path(),
        &bundles[0].collection,
        &bundles[0].environments,
        &MemoryStore::new(),
    )
    .unwrap();
    assert_eq!(files_of(&root), files_of(&core.path().join("Pets")));
    drop(files);
}

#[gpui_kit::test]
fn a_postman_link_imports(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let server = serve(cx, ResponseTemplate::new(200).set_body_string(postman()));
    open_with(cx, win, &format!("{}/shop.json", server.uri()));
    let into = tempfile::tempdir().unwrap();
    create_in(cx, into.path());
    click(cx, win, "import-link-import");
    landed(cx, &view, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
    assert!(into.path().join("Shop/Get user.yml").is_file());
}

#[gpui_kit::test]
fn a_404_shows_an_error_and_keeps_the_dialog(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let server = serve(cx, ResponseTemplate::new(404));
    let dlg = open_with(cx, win, &format!("{}/missing.json", server.uri()));
    click(cx, win, "import-link-import");
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, &dlg), Phase::Failed(_))
    });
    assert!(error(cx, &dlg).contains("404"), "{}", error(cx, &dlg));
    assert!(present(cx, win, "import-link-error"));
    assert!(dialog(cx, win));
    assert!(!cx.did_prompt_for_paths());
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn an_unknown_body_shows_the_format_error(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let server = serve(
        cx,
        ResponseTemplate::new(200).set_body_string(r#"{"other":1}"#),
    );
    let dlg = open_with(cx, win, &format!("{}/x.json", server.uri()));
    click(cx, win, "import-link-import");
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, &dlg), Phase::Failed(_))
    });
    assert!(
        error(cx, &dlg).to_lowercase().contains("recognized"),
        "{}",
        error(cx, &dlg)
    );
    assert!(dialog(cx, win));
    assert!(!cx.did_prompt_for_paths());
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn an_ftp_link_shows_a_scheme_error_without_fetching(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    let server = serve(cx, ResponseTemplate::new(200));
    let dlg = open_with(cx, win, "ftp://example.test/c.json");
    click(cx, win, "import-link-import");
    // The text is refused before any work starts, so the phase is already final.
    assert!(
        matches!(phase(cx, &dlg), Phase::Failed(_)),
        "{:?}",
        phase(cx, &dlg)
    );
    assert!(error(cx, &dlg).contains("ftp"), "{}", error(cx, &dlg));
    assert!(dialog(cx, win));
    settle(cx);
    assert_eq!(received(cx, &server), 0, "no request left the app");
}

#[gpui_kit::test]
fn enter_in_the_link_field_starts_the_import(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let server = serve(cx, ResponseTemplate::new(200).set_body_string(postman()));
    open_with(cx, win, &format!("{}/shop.json", server.uri()));
    let into = tempfile::tempdir().unwrap();
    create_in(cx, into.path());
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    landed(cx, &view, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
    assert_eq!(received(cx, &server), 1);
}

#[gpui_kit::test]
fn enter_while_fetching_starts_no_second_fetch(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    // Stalled far past the checks: the wait and settle below can take seconds
    // under load, and the cancel button only exists while the fetch is pending.
    let slow = ResponseTemplate::new(200)
        .set_body_string(OPENAPI)
        .set_delay(std::time::Duration::from_secs(30));
    let server = serve(cx, slow);
    let dlg = open_with(cx, win, &format!("{}/pets.json", server.uri()));
    click(cx, win, "import-link-import");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    assert_eq!(phase(cx, &dlg), Phase::Fetching);
    wait_for(cx, "the request to arrive", |cx| received(cx, &server) == 1);
    settle(cx);
    assert_eq!(received(cx, &server), 1);
    click(cx, win, "import-link-cancel");
}

#[gpui_kit::test]
fn cancel_during_a_slow_fetch_drops_it_and_writes_nothing(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let slow = ResponseTemplate::new(200)
        .set_body_string(OPENAPI)
        .set_delay(std::time::Duration::from_millis(1500));
    let server = serve(cx, slow);
    let dlg = open_with(cx, win, &format!("{}/pets.json", server.uri()));
    click(cx, win, "import-link-import");
    assert_eq!(phase(cx, &dlg), Phase::Fetching);
    wait_for(cx, "the request to arrive", |cx| received(cx, &server) == 1);
    click(cx, win, "import-link-cancel");
    assert!(!dialog(cx, win));
    // Long enough for the delayed answer to have arrived had the fetch survived.
    settle(cx);
    settle(cx);
    assert!(
        !cx.did_prompt_for_paths(),
        "a dropped fetch never asks where to write"
    );
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn the_import_button_is_disabled_while_fetching(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    // Stalled far past the checks: the wait and settle below can take seconds
    // under load, and the cancel button only exists while the fetch is pending.
    let slow = ResponseTemplate::new(200)
        .set_body_string(OPENAPI)
        .set_delay(std::time::Duration::from_secs(30));
    let server = serve(cx, slow);
    let dlg = open_with(cx, win, &format!("{}/pets.json", server.uri()));
    click(cx, win, "import-link-import");
    click(cx, win, "import-link-import");
    assert_eq!(phase(cx, &dlg), Phase::Fetching);
    // The request is made on a background task, so wait for it, then give a
    // second fetch time to show up before counting.
    wait_for(cx, "the request to arrive", |cx| received(cx, &server) == 1);
    settle(cx);
    assert_eq!(
        received(cx, &server),
        1,
        "a second click starts no second fetch"
    );
    click(cx, win, "import-link-cancel");
}

#[gpui_kit::test]
fn browse_runs_the_file_prompt(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    open_with(cx, win, "");
    let into = tempfile::tempdir().unwrap();
    create_in(cx, into.path());
    click(cx, win, "import-link-browse");
    assert!(cx.did_prompt_for_paths());
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![file])
    });
    cx.run_until_parked();
    landed(cx, &view, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
}

#[gpui_kit::test]
fn an_absolute_path_in_the_field_imports_like_browse(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    open_with(cx, win, file.to_str().unwrap());
    let into = tempfile::tempdir().unwrap();
    create_in(cx, into.path());
    click(cx, win, "import-link-import");
    landed(cx, &view, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
    assert!(into.path().join("Shop/Get user.yml").is_file());
}

#[gpui_kit::test]
fn a_file_url_in_the_field_imports_like_browse(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("my shop.json");
    fs::write(&file, postman()).unwrap();
    let url = url::Url::from_file_path(&file).unwrap();
    open_with(cx, win, url.as_str());
    let into = tempfile::tempdir().unwrap();
    create_in(cx, into.path());
    click(cx, win, "import-link-import");
    landed(cx, &view, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
}

#[gpui_kit::test]
fn a_missing_path_shows_a_clear_error(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("gone.json");
    let dlg = open_with(cx, win, file.to_str().unwrap());
    click(cx, win, "import-link-import");
    assert!(
        matches!(phase(cx, &dlg), Phase::Failed(_)),
        "{:?}",
        phase(cx, &dlg)
    );
    let msg = error(cx, &dlg);
    assert!(msg.contains("No file at that path"), "{msg}");
    assert!(msg.contains("gone.json"), "{msg}");
    assert!(dialog(cx, win));
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn text_that_is_neither_a_link_nor_a_path_shows_the_expected_error(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    let dlg = open_with(cx, win, "just some words");
    click(cx, win, "import-link-import");
    assert!(
        matches!(phase(cx, &dlg), Phase::Failed(_)),
        "{:?}",
        phase(cx, &dlg)
    );
    let msg = error(cx, &dlg);
    assert!(msg.contains("Expected an http or https link"), "{msg}");
}

/// Counts how often the dialog tells its view to redraw. A state change that skips the notify
/// leaves the old text on screen until some later event repaints, so each change must add one.
fn notifies(
    cx: &mut TestAppContext,
    view: &Entity<ImportLinkDialog>,
) -> std::rc::Rc<std::cell::Cell<usize>> {
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    cx.update(|cx| {
        cx.observe(view, move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    count
}

#[gpui_kit::test]
fn the_fetching_state_and_the_refused_error_each_notify_the_view(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    // A stalled server: the request arrives and the answer never does within the test.
    let stalled = ResponseTemplate::new(200)
        .set_body_string(OPENAPI)
        .set_delay(std::time::Duration::from_secs(30));
    let server = serve(cx, stalled);
    let dlg = open_with(cx, win, &format!("{}/pets.json", server.uri()));
    let count = notifies(cx, &dlg);
    let before = count.get();
    cx.update_window(win, |_, w, cx| w.click("import-link-import", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(phase(cx, &dlg), Phase::Fetching);
    assert!(count.get() > before, "starting the fetch notifies");
    click(cx, win, "import-link-cancel");

    // A closed port: bind, note the address, drop the listener so the connection is refused.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let dlg = open_with(cx, win, &format!("http://{closed}/pets.json"));
    let count = notifies(cx, &dlg);
    cx.update_window(win, |_, w, cx| w.click("import-link-import", cx))
        .unwrap();
    // Sampled before anything is parked: a refusal can land at once, and only the start has notified so far.
    let fetching = count.get();
    wait_for(cx, "the refusal", |cx| {
        matches!(phase(cx, &dlg), Phase::Failed(_))
    });
    assert!(count.get() > fetching, "the error notifies when it arrives");
    assert!(present(cx, win, "import-link-error"));
}

#[gpui_kit::test]
fn enter_on_the_focused_update_mode_button_switches_the_mode(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    cx.update(apinata_app::keymap::init);
    open_with(cx, win, "");
    enter_on(cx, win, "import-mode-update");
    frame(cx, win);
    assert!(dialog(cx, win), "the dialog stays open");
    assert!(
        present(cx, win, "import-update-collection-heading"),
        "Enter pressed Update"
    );
}

#[gpui_kit::test]
fn the_dialog_lists_every_collection_format_under_its_title(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    open_with(cx, win, "");
    assert!(present(cx, win, "import-link-formats"));
    let text = import_link::formats_text();
    for format in apinata_core::import::CollectionFormat::ALL {
        assert!(text.contains(format.label()), "{text}");
    }
    let (formats, mode) = cx
        .update_window(win, |_, w, _| {
            (
                w.find("import-link-formats").bounds().top(),
                w.find("import-mode").bounds().top(),
            )
        })
        .unwrap();
    assert!(formats < mode, "the formats come first, under the title");
}

#[gpui_kit::test]
fn an_environment_file_points_at_import_environment(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("dev.json");
    fs::write(
        &file,
        r#"{"name":"Dev","values":[{"key":"host","value":"x.test","enabled":true}],"_postman_variable_scope":"environment"}"#,
    )
    .unwrap();
    let dlg = open_with(cx, win, file.to_str().unwrap());
    click(cx, win, "import-link-import");
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, &dlg), Phase::Failed(_))
    });
    let msg = error(cx, &dlg);
    assert!(msg.contains("environment"), "{msg}");
    assert!(msg.contains("Import environment"), "{msg}");
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn the_mode_is_one_segmented_control(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    let dlg = open_with(cx, win, "");
    assert!(
        present(cx, win, "import-mode"),
        "one track holds both modes"
    );
    let (new, update) = cx
        .update_window(win, |_, w, _| {
            (
                w.find("import-mode-new").bounds(),
                w.find("import-mode-update").bounds(),
            )
        })
        .unwrap();
    assert_eq!(new.top(), update.top(), "side by side, not stacked");
    click(cx, win, "import-mode-update");
    assert_eq!(
        dlg.read_with(cx, |d, _| d.mode()),
        import_link::Mode::Update
    );
}

#[gpui_kit::test]
fn the_link_placeholder_reads_as_an_example(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    let dlg = open_with(cx, win, "");
    let hint = dlg.read_with(cx, |d, cx| d.placeholder(cx));
    assert!(hint.starts_with("e.g. "), "{hint}");
}

/// Whether `id` sits on the link field's row.
fn beside_the_link(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let (field, button) = (w.find("import-link").bounds(), w.find(id).bounds());
        button.top() < field.bottom() && button.bottom() > field.top()
    })
    .unwrap()
}

#[gpui_kit::test]
fn browse_sits_beside_the_link_in_both_modes(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    open_with(cx, win, "");
    assert!(beside_the_link(cx, win, "import-link-browse"));
    click(cx, win, "import-mode-update");
    assert!(beside_the_link(cx, win, "import-update-browse"));
    assert!(
        !present(cx, win, "import-link-browse"),
        "one Browse at a time"
    );
}

#[gpui_kit::test]
fn create_in_names_the_folder_and_change_picks_another(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx);
    let dlg = open_with(cx, win, "");
    assert!(present(cx, win, "import-link-parent"));
    let into = tempfile::tempdir().unwrap();
    click(cx, win, "import-link-parent-change");
    assert!(cx.did_prompt_for_paths());
    let pick = into.path().to_path_buf();
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.directories && !o.files && !o.multiple);
        Some(vec![pick])
    });
    cx.run_until_parked();
    let parent = dlg.read_with(cx, |d, _| d.parent().map(Path::to_path_buf));
    assert_eq!(parent.as_deref(), Some(into.path()));
    click(cx, win, "import-mode-update");
    assert!(
        !present(cx, win, "import-link-parent"),
        "an update creates nothing"
    );
}

/// The Create in folder the settings remember, shared with the clone dialog.
fn saved_folder(cx: &mut TestAppContext) -> Option<PathBuf> {
    cx.update(|cx| {
        cx.global::<apinata_app::app::AppState>()
            .settings
            .read(cx)
            .get()
            .clone_parent
            .clone()
    })
}

/// Opens the Import dialog in a started app.
fn open_started(cx: &mut TestAppContext, s: &Started) -> Entity<ImportLinkDialog> {
    switcher_click(cx, s.win, "switcher-import-collection");
    frame(cx, s.win);
    cx.read(import_link::current).expect("dialog open")
}

/// Puts `link` in the dialog's field and presses Import.
fn import_link_text(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    dlg: &Entity<ImportLinkDialog>,
    link: &str,
) {
    let (d, link) = (dlg.clone(), link.to_owned());
    cx.update_window(win, |_, w, cx| {
        d.update(cx, |d, cx| d.set_link(&link, w, cx))
    })
    .unwrap();
    click(cx, win, "import-link-import");
}

/// Waits for the dialog to show a failure and answers its text.
fn failed(cx: &mut TestAppContext, dlg: &Entity<ImportLinkDialog>) -> String {
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, dlg), Phase::Failed(_))
    });
    error(cx, dlg)
}

#[gpui_kit::test]
fn the_dialog_opens_in_the_saved_clone_folder(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let saved = tempfile::tempdir().unwrap();
    let dir = saved.path().to_path_buf();
    cx.update(|cx| {
        let settings = cx.global::<apinata_app::app::AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |s| s.clone_parent = Some(dir)));
    });
    let dlg = open_started(cx, &s);
    let parent = dlg.read_with(cx, |d, _| d.parent().map(Path::to_path_buf));
    assert_eq!(parent.as_deref(), Some(saved.path()));
}

#[gpui_kit::test]
fn change_is_remembered_only_once_an_import_lands(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let dlg = open_started(cx, &s);
    let into = tempfile::tempdir().unwrap();
    click(cx, s.win, "import-link-parent-change");
    let pick = into.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![pick]));
    cx.run_until_parked();
    assert_eq!(saved_folder(cx), None, "a pick alone saves nothing");
    let file = s.files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    import_link_text(cx, s.win, &dlg, file.to_str().unwrap());
    wait_for(cx, "the import to open", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.collections()
                .iter()
                .any(|c| c.root() == into.path().join("Shop"))
        })
    });
    assert_eq!(saved_folder(cx).as_deref(), Some(into.path()));
}

#[gpui_kit::test]
fn without_a_folder_the_import_says_so_and_keeps_the_link(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    let dlg = open_with(cx, win, "");
    dlg.update(cx, |d, cx| d.forget_parent_for_test(cx));
    import_link_text(cx, win, &dlg, file.to_str().unwrap());
    let msg = failed(cx, &dlg);
    assert!(msg.contains("Choose a folder"), "{msg}");
    assert!(dialog(cx, win));
    assert_eq!(
        dlg.read_with(cx, |d, cx| d.link(cx)),
        file.to_str().unwrap()
    );
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn a_missing_chosen_folder_is_named_and_not_recreated(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    let gone = files.path().join("unmounted");
    let dlg = open_with(cx, win, "");
    create_in(cx, &gone);
    import_link_text(cx, win, &dlg, file.to_str().unwrap());
    let msg = failed(cx, &dlg);
    assert!(msg.contains("unmounted"), "{msg}");
    assert!(msg.contains("no longer exists"), "{msg}");
    assert!(!gone.exists(), "a chosen folder is never recreated");
    assert!(dialog(cx, win));
    assert_eq!(
        dlg.read_with(cx, |d, cx| d.link(cx)),
        file.to_str().unwrap()
    );
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn the_built_in_default_folder_is_made_when_missing(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    let default = files.path().join("home/Apinata");
    let dlg = open_with(cx, win, "");
    let d = default.clone();
    dlg.update(cx, |dlg, cx| dlg.set_default_for_test(d, cx));
    import_link_text(cx, win, &dlg, file.to_str().unwrap());
    wait_for(cx, "the import to open", |cx| !opened(cx, &view).is_empty());
    assert_eq!(opened(cx, &view), vec![default.join("Shop")]);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_write_keeps_the_dialog_and_its_link(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (files, win, view) = shell(cx);
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    let locked = files.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let dlg = open_with(cx, win, "");
    create_in(cx, &locked);
    import_link_text(cx, win, &dlg, file.to_str().unwrap());
    let msg = failed(cx, &dlg);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!msg.is_empty());
    assert!(dialog(cx, win), "the dialog stays for another try");
    assert_eq!(
        dlg.read_with(cx, |d, cx| d.link(cx)),
        file.to_str().unwrap()
    );
    assert!(opened(cx, &view).is_empty());
}
