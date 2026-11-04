//! curl without a collection lands in the scratch.

use crate::support;

use apinata_app::request_editor::URL_PLACEHOLDER;
use apinata_app::shell::{ImportCurl, SwitchBranch};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

#[gpui_kit::test]
fn import_curl_with_no_collection_makes_a_scratch_draft(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(ImportCurl), cx);
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "curl -X POST http://127.0.0.1:9/u",
    );
    cx.update_window(s.win, |_, w, cx| w.click("import-curl-import", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    let (root, item) = active_draft(cx, &s.ws);
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&root, &item, cx))
            .unwrap();
    assert_eq!(
        (view.method.as_str(), view.url.as_str()),
        ("POST", "http://127.0.0.1:9/u")
    );
}

#[gpui_kit::test]
fn a_dropped_curl_command_uses_the_same_root(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let request = apinata_core::import::parse_request(None, "curl http://127.0.0.1:9/d")
        .map(|p| p.request)
        .unwrap();
    let root = s.ws.update(cx, |w, cx| w.draft_root(cx)).unwrap();
    s.ws.update(cx, |w, cx| w.new_draft_from_import(&root, request, cx));
    cx.run_until_parked();
    let (r, item) = active_draft(cx, &s.ws);
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&r, &item, cx))
            .unwrap();
    assert_eq!(view.url, "http://127.0.0.1:9/d");
}

#[gpui_kit::test]
fn a_curl_paste_in_the_scratch_url_bar_fills_the_draft(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &s.ws);
    let (win, _editor) = editor_for(cx, &s.ws, &root, item.as_str(), false);
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
        "curl -X PUT http://127.0.0.1:9/p".to_owned(),
    ));
    cx.update_window(win, |_, w, cx| {
        w.click("url", cx);
        w.press(&secondary("v"), cx);
    })
    .unwrap();
    frame(cx, win);
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&root, &item, cx))
            .unwrap();
    assert_eq!(
        (view.method.as_str(), view.url.as_str()),
        ("PUT", "http://127.0.0.1:9/p")
    );
    assert_eq!(URL_PLACEHOLDER, "Enter URL or paste a curl command");
}

#[gpui_kit::test]
fn the_branch_picker_ignores_a_lone_scratch(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    let before = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(SwitchBranch), cx);
    })
    .unwrap();
    frame(cx, s.win);
    let after = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    assert_eq!(after, before + 1, "a toast says to open a collection first");
}
