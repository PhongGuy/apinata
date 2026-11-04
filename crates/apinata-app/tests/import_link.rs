//! Import collection from a link: the dialog behind the Import action, fetching over a local
//! wiremock server, handing the bytes to the same parse and write path as Browse.

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

/// Answers the "Import into" folder prompt.
fn place(cx: &mut TestAppContext, into: &Path) {
    wait_for(cx, "the folder prompt", |cx| cx.did_prompt_for_paths());
    let into = into.to_path_buf();
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.directories && !o.files && !o.multiple);
        Some(vec![into])
    });
    cx.run_until_parked();
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
    click(cx, win, "import-link-import");
    assert_eq!(phase(cx, &dlg), Phase::Fetching);
    let into = tempfile::tempdir().unwrap();
    place(cx, into.path());
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
    click(cx, win, "import-link-import");
    let into = tempfile::tempdir().unwrap();
    place(cx, into.path());
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
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    let into = tempfile::tempdir().unwrap();
    place(cx, into.path());
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
    click(cx, win, "import-link-browse");
    assert!(cx.did_prompt_for_paths());
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![file])
    });
    cx.run_until_parked();
    let into = tempfile::tempdir().unwrap();
    place(cx, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
}

#[gpui_kit::test]
fn an_absolute_path_in_the_field_imports_like_browse(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let file = files.path().join("shop.json");
    fs::write(&file, postman()).unwrap();
    open_with(cx, win, file.to_str().unwrap());
    click(cx, win, "import-link-import");
    let into = tempfile::tempdir().unwrap();
    place(cx, into.path());
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
    click(cx, win, "import-link-import");
    let into = tempfile::tempdir().unwrap();
    place(cx, into.path());
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
