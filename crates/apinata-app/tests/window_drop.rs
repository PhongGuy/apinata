//! Dropping folders and files from the desktop onto the window opens or imports them.

use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::tabs::TabKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, ExternalPaths, FileDropEvent, InputEvent as _,
    TestAppContext, WindowOptions, point, px,
};
use std::fs;
use std::path::{Path, PathBuf};
use support::*;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn postman() -> String {
    format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{{"name":"Get user","request":{{"method":"GET","url":"https://x.test/users/1"}}}}]}}"#
    )
}

const OPENAPI: &str = r#"{"openapi":"3.0.0","info":{"title":"Pets","version":"1"},"paths":{"/pets":{"get":{"summary":"List","responses":{"200":{"description":"ok"}}}}}}"#;

const HAR: &str = r#"{"log":{"version":"1.2","creator":{"name":"t","version":"1"},"entries":[{"startedDateTime":"2026-01-01T00:00:00.000Z","time":1,"request":{"method":"GET","url":"http://127.0.0.1:9/a","httpVersion":"HTTP/1.1","headers":[],"queryString":[],"cookies":[],"headersSize":-1,"bodySize":0},"response":{"status":200,"statusText":"OK","httpVersion":"HTTP/1.1","headers":[],"cookies":[],"content":{"size":0,"mimeType":"text/plain"},"redirectURL":"","headersSize":-1,"bodySize":0},"cache":{},"timings":{"send":0,"wait":0,"receive":0}}]}}"#;

const CURL: &str = "curl -X POST http://127.0.0.1:9/users";

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

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

fn entered(cx: &mut TestAppContext, win: AnyWindowHandle, paths: &[PathBuf]) {
    let paths = ExternalPaths(paths.iter().cloned().collect());
    cx.update_window(win, |_, w, cx| {
        let position = point(px(300.), px(300.));
        let event = FileDropEvent::Entered { position, paths };
        w.dispatch_event(event.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
}

fn submit(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        let position = point(px(300.), px(300.));
        let submit = FileDropEvent::Submit { position };
        w.dispatch_event(submit.to_platform_input(), cx);
        w.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
        w.dispatch_event(FileDropEvent::Ended.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn drop_on(cx: &mut TestAppContext, win: AnyWindowHandle, paths: &[PathBuf]) {
    entered(cx, win, paths);
    submit(cx, win);
}

fn toasts(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn dialog(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

/// Answers the "Import into" directory prompt.
fn answer_into(cx: &mut TestAppContext, into: &Path) {
    assert!(cx.did_prompt_for_paths(), "the import asks where to write");
    let into = into.to_path_buf();
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.directories && !o.files);
        Some(vec![into])
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_dropped_folder_opens_as_a_collection(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let col = collection();
    drop_on(cx, win, &[col.path().to_path_buf()]);
    assert_eq!(opened(cx, &view), vec![col.path().to_path_buf()]);
}

#[gpui_kit::test]
fn a_dropped_postman_file_is_imported_where_the_user_says(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    let file = write(files.path(), "shop.json", &postman());
    drop_on(cx, win, &[file]);
    answer_into(cx, into.path());
    assert_eq!(opened(cx, &view), vec![into.path().join("Shop")]);
}

#[gpui_kit::test]
fn a_dropped_openapi_file_is_imported(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    drop_on(cx, win, &[write(files.path(), "pets.json", OPENAPI)]);
    answer_into(cx, into.path());
    assert_eq!(opened(cx, &view).len(), 1);
}

#[gpui_kit::test]
fn a_dropped_har_file_is_imported(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let into = tempfile::tempdir().unwrap();
    drop_on(cx, win, &[write(files.path(), "trace.har", HAR)]);
    answer_into(cx, into.path());
    assert_eq!(opened(cx, &view).len(), 1);
}

#[gpui_kit::test]
fn a_dropped_curl_file_becomes_a_draft_in_the_open_collection(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let file = write(s.files.path(), "call.sh", CURL);
    drop_on(cx, s.win, &[file]);
    let item =
        s.ws.read_with(cx, |w, _| match w.tabs().active().map(|t| t.kind.clone()) {
            Some(TabKind::Request(item)) => item,
            other => panic!("a request tab is active: {other:?}"),
        });
    assert!(item.is_draft(), "{item:?}");
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&s.root, &item, cx))
            .expect("the draft");
    assert_eq!(view.url.as_str(), "http://127.0.0.1:9/users");
}

#[gpui_kit::test]
fn a_dropped_curl_file_without_a_collection_becomes_a_scratch_draft(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let file = write(s.files.path(), "call.sh", CURL);
    drop_on(cx, s.win, &[file]);
    let (root, item) = active_draft(cx, &s.ws);
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    let view = s.ws.read_with(cx, |w, cx| w.request(&root, &item, cx));
    assert_eq!(view.unwrap().url.as_str(), "http://127.0.0.1:9/users");
}

#[gpui_kit::test]
fn an_unknown_file_shows_an_error_and_prompts_for_nothing(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    drop_on(cx, win, &[write(files.path(), "notes.txt", "hello there")]);
    assert_eq!(toasts(cx, win), 1, "the failure shows at once");
    assert!(!cx.did_prompt_for_paths());
    assert!(!dialog(cx, win));
    assert!(opened(cx, &view).is_empty());
}

#[gpui_kit::test]
fn each_of_several_dropped_paths_is_handled(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx);
    let (col, into) = (collection(), tempfile::tempdir().unwrap());
    let paths = [
        col.path().to_path_buf(),
        write(files.path(), "notes.txt", "hello there"),
        write(files.path(), "shop.json", &postman()),
    ];
    drop_on(cx, win, &paths);
    answer_into(cx, into.path());
    let mut got = opened(cx, &view);
    got.sort();
    let mut want = vec![col.path().to_path_buf(), into.path().join("Shop")];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(toasts(cx, win), 1, "only the unknown file is refused");
}

#[gpui_kit::test]
fn the_overlay_shows_while_files_hover_and_goes_when_they_leave(cx: &mut TestAppContext) {
    let (files, win, _view) = shell(cx);
    let file = write(files.path(), "shop.json", &postman());
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("drop-overlay").is_some()
        })
        .unwrap()
    };
    assert!(!shown(cx));
    entered(cx, win, &[file]);
    assert!(shown(cx));
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx)
    })
    .unwrap();
    assert!(!shown(cx));
}
