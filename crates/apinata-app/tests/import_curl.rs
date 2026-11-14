//! Import curl: a dialog that parses a curl command into a new draft.

use crate::support;

use apinata_app::request_editor::URL_PLACEHOLDER;
use apinata_app::session::Session;
use apinata_app::shell::{ImportCurl, Shell};
use apinata_app::tabs::TabKind;
use apinata_core::ItemPath;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, WindowOptions,
};
use support::*;

fn import_curl(cx: &mut TestAppContext, win: AnyWindowHandle, shell: &Entity<Shell>) {
    let shell = shell.clone();
    cx.update_window(win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(ImportCurl), cx);
    })
    .unwrap();
    frame(cx, win);
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn tab_count(cx: &mut TestAppContext, s: &Started) -> usize {
    s.ws.read_with(cx, |w, _| w.tabs().entries().len())
}

#[gpui_kit::test]
fn a_curl_command_becomes_a_new_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    import_curl(cx, s.win, &s.shell);
    assert!(present(cx, s.win, "import-curl-dialog"));
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "curl -X POST http://127.0.0.1:9/users",
    );
    click(cx, s.win, "import-curl-import");
    assert!(!dialog_open(cx, s.win));
    let item =
        s.ws.read_with(cx, |w, _| match w.tabs().active().map(|t| t.kind.clone()) {
            Some(TabKind::Request(item)) => item,
            other => panic!("a request tab is active: {other:?}"),
        });
    assert!(item.is_draft(), "{item:?}");
    assert_eq!(item.parent(), Some(ItemPath::root()));
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&s.root, &item, cx))
            .expect("the draft");
    assert_eq!(
        (view.method.as_str(), view.url.as_str()),
        ("POST", "http://127.0.0.1:9/users")
    );
}

#[gpui_kit::test]
fn text_that_is_not_curl_keeps_the_dialog_open_with_an_error(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = tab_count(cx, &s);
    import_curl(cx, s.win, &s.shell);
    assert!(!present(cx, s.win, "import-curl-error"));
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "wget http://127.0.0.1:9/users",
    );
    click(cx, s.win, "import-curl-import");
    assert!(dialog_open(cx, s.win));
    assert!(present(cx, s.win, "import-curl-error"));
    assert_eq!(tab_count(cx, &s), before);
}

#[gpui_kit::test]
fn cancel_imports_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = tab_count(cx, &s);
    import_curl(cx, s.win, &s.shell);
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "curl http://127.0.0.1:9/a",
    );
    click(cx, s.win, "import-curl-cancel");
    assert!(!dialog_open(cx, s.win));
    assert_eq!(tab_count(cx, &s), before);
}

#[gpui_kit::test]
fn without_a_collection_the_import_dialog_opens_on_the_scratch(cx: &mut TestAppContext) {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| {
        w.set_scratch_dir(state.path().join("scratch"), cx)
    });
    let (win, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws.clone(), w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    import_curl(cx, win, &shell);
    frame(cx, win);
    assert!(dialog_open(cx, win), "the dialog opens, no toast");
    assert!(ws.read_with(cx, |w, _| {
        w.collections().iter().any(|c| w.is_scratch(c.root()))
    }));
}

#[gpui_kit::test]
fn the_url_placeholder_offers_the_paste(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let input = ed.read_with(cx, |e, _| e.url_input().clone());
    assert_eq!(URL_PLACEHOLDER, "Enter URL or paste a curl command");
    assert_eq!(
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string()),
        URL_PLACEHOLDER
    );
}

// The empty editor says what to paste, with an example, so the first open is not a blank box.
#[gpui_kit::test]
fn the_empty_editor_shows_an_example_command(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    import_curl(cx, s.win, &s.shell);
    let hint = cx
        .update_window(s.win, |_, w, cx| {
            use gpui_kit::test::TestQueryExt as _;
            w.render_frame(cx);
            w.within("import-curl-input")
                .elements()
                .into_iter()
                .find_map(|e| e.placeholder().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        hint.as_deref(),
        Some(
            "Paste a curl command, for example curl https://api.example.com/users -H 'Accept: application/json'"
        )
    );
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    frame(cx, win);
}

fn focused_on(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).focused()
    })
    .unwrap()
        == Some(true)
}

// Ctrl+Tab leaves the editor for Cancel; Tab then reaches Import.
fn curl_with_focus_on(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    import_curl(cx, s.win, &s.shell);
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "curl -X POST http://127.0.0.1:9/users",
    );
    // Ctrl+Tab leaves the editor for Import; Cancel is the stop before it.
    press(cx, s.win, "ctrl-tab");
    if id == "import-curl-cancel" {
        press(cx, s.win, "shift-tab");
    }
    assert!(focused_on(cx, s.win, id), "{id} has focus");
}

#[gpui_kit::test]
fn ctrl_tab_then_enter_from_the_editor_imports(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = tab_count(cx, &s);
    import_curl(cx, s.win, &s.shell);
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "curl -X POST http://127.0.0.1:9/users",
    );
    frame(cx, s.win);
    platform_press(cx, s.win, "ctrl-tab");
    next_frame(cx, s.win);
    assert!(
        ring_on(cx, s.win, "import-curl-import"),
        "Ctrl+Tab rings Import"
    );
    platform_press(cx, s.win, "enter");
    next_frame(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Enter imported and closed");
    assert_eq!(tab_count(cx, &s), before + 1, "the draft opened");
}

#[gpui_kit::test]
fn enter_on_the_focused_import_button_imports(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = tab_count(cx, &s);
    curl_with_focus_on(cx, &s, "import-curl-import");
    press(cx, s.win, "enter");
    assert!(!dialog_open(cx, s.win), "Import closes the dialog");
    assert_eq!(tab_count(cx, &s), before + 1, "Enter imported the draft");
}

#[gpui_kit::test]
fn enter_on_the_focused_cancel_button_imports_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = tab_count(cx, &s);
    curl_with_focus_on(cx, &s, "import-curl-cancel");
    press(cx, s.win, "enter");
    assert!(!dialog_open(cx, s.win));
    assert_eq!(tab_count(cx, &s), before);
}

#[gpui_kit::test]
fn enter_outside_the_editor_keeps_the_pasted_command(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    import_curl(cx, s.win, &s.shell);
    replace_code(
        cx,
        s.win,
        None,
        "import-curl-input",
        "curl http://127.0.0.1:9/users",
    );
    // The dialog's own Enter binding, as it fires when no field takes Enter itself.
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(
            Box::new(gpui_kit::base::actions::Confirm { secondary: false }),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, s.win), "the dialog stays open");
}
