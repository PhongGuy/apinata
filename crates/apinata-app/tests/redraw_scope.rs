//! A key typed into the Body editor redraws only the views on its way to the window, and
//! the views it leaves alone still follow their own inputs. Frames here are drawn the way
//! the platform draws them, without the harness's forced refresh, so a view reused from
//! the last frame shows exactly what the app would show.

use crate::git_support;
use crate::support;

use std::sync::Arc;
use std::time::Duration;

use apinata_app::app::AppState;
use apinata_app::cached_part::Same;
use apinata_app::command::{Command, Edit, EnvOp, Target, TreeOp};
use apinata_app::console_dock::ConsoleDock;
use apinata_app::last_response::{ResponseStore, clear_all, next_age_change, wall_now};
use apinata_app::request_chrome::ChromePart;
use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::response_pane::ResponsePane;
use apinata_app::send_model::Restored;
use apinata_app::settings::{Orientation, Settings, ThemeChoice};
use apinata_app::tab_area::DraggedTab;
use apinata_app::tabs::{Side as TabSide, TabId};
use apinata_app::url_unit::Side;
use apinata_core::{
    BodySpec, ItemPath, KeyValue, Outcome, RequestResult, Response, ScriptDecision, ScriptPhase,
    ScriptTrust, VarValue, Variable,
};
use git_support::{Sandbox, write as git_write};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, ElementId, Entity, InputEvent as _, Keystroke, Modifiers,
    MouseMoveEvent, Pixels, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px,
    size,
};
use support::sse_server::{Script, Step};
use support::*;

const KEYS: usize = 20;

// A text body to type into, and a URL whose undefined variable fails the send before it
// leaves the machine, so the response pane shows its error block.
const BODY_REQUEST: &str = "info:\n  name: j\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://127.0.0.1:9/{{nope}}\"\n  body:\n    type: text\n    data: 'x'\n";

// One frame as the platform draws it: views nobody notified are reused, not rebuilt.
fn draw(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
}

fn visible(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    draw(cx, win);
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

fn visible_id(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    draw(cx, win);
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

fn editor_of(cx: &mut TestAppContext, s: &Started, file: &str) -> Entity<RequestEditor> {
    let tab = open_tab(cx, &s.ws, &s.root, file);
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    area.read_with(cx, |a, _| a.editor(tab).cloned()).unwrap()
}

// Sends `file` from its toolbar and waits until the response pane has the result.
fn send_from_toolbar(cx: &mut TestAppContext, s: &Started, editor: &Entity<RequestEditor>) {
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    finished(cx, &model);
    draw(cx, s.win);
}

// The app with j.yml sent once, its Body section showing and the caret at the body's end.
fn typing_after_a_send(
    cx: &mut TestAppContext,
) -> (Started, Entity<RequestEditor>, tempfile::TempDir) {
    http_ready(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("j.yml", BODY_REQUEST)]);
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "j.yml");
    send_from_toolbar(cx, &s, &editor);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("body").click_at("body-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    draw(cx, s.win);
    (s, editor, col)
}

// Every key of a pilot `type` call in one update: the window draws before each key.
fn type_burst(cx: &mut TestAppContext, win: AnyWindowHandle) {
    type_keys(cx, win, KEYS);
}

fn type_keys(cx: &mut TestAppContext, win: AnyWindowHandle, keys: usize) {
    cx.update_window(win, |_, w, cx| {
        for _ in 0..keys {
            let mut key = Keystroke::parse("a").unwrap();
            key.key_char = Some("a".into());
            w.dispatch_keystroke(key, cx);
        }
    })
    .unwrap();
    draw(cx, win);
}

#[gpui_kit::test]
fn a_body_key_redraws_neither_the_sidebar_nor_the_response(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    let response = editor.read_with(cx, |e, _| e.response().clone());
    let before = (
        sidebar.read_with(cx, |v, _| v.renders()),
        response.read_with(cx, |v, _| v.renders()),
    );
    type_burst(cx, s.win);
    match &view_of(cx, &s.ws, &s.root, "j.yml").body {
        BodySpec::Text(t) => assert_eq!(t.len(), 1 + KEYS, "every key reached the body"),
        other => panic!("{other:?}"),
    }
    // The edits' snapshots may redraw them a few times; one redraw per key is the bug.
    let sidebar_renders = sidebar.read_with(cx, |v, _| v.renders()) - before.0;
    let response_renders = response.read_with(cx, |v, _| v.renders()) - before.1;
    assert!(
        sidebar_renders < KEYS / 2 && response_renders < KEYS / 2,
        "{KEYS} keys redrew the sidebar {sidebar_renders} times and the response {response_renders} times"
    );
}

#[gpui_kit::test]
fn the_sidebar_shows_a_new_request_while_the_body_is_typed_in(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    let create = TreeOp::CreateRequest {
        parent: ItemPath::root(),
        name: "fresh".into(),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Tree(create), cx));
    draw(cx, s.win);
    let id = row("fresh.yml");
    let found = cx
        .update_window(s.win, |_, w, _| w.try_find(id).is_some())
        .unwrap();
    assert!(found, "the new request has its sidebar row");
}

#[gpui_kit::test]
fn the_response_offer_follows_a_new_environment(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    let label = |cx: &mut TestAppContext| {
        draw(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            w.find("response-add-env").label().map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Create environment"));
    let create = Command::Env(EnvOp::Create { name: "qa".into() });
    s.ws.update(cx, |w, cx| w.send(&s.root, create, cx));
    assert_eq!(label(cx).as_deref(), Some("Add to environment"));
}

#[gpui_kit::test]
fn the_response_drops_ask_again_once_the_collection_is_unblocked(cx: &mut TestAppContext) {
    http_ready(cx);
    // A live server, so the pane shows the response with its warnings above the tabs.
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1)\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    let trust = s.files.path().join("script-trust.json");
    ScriptTrust::update(trust.clone(), |t| {
        t.set_decision(&s.root, Some(ScriptDecision::Blocked))
    })
    .unwrap();
    refresh_approvals(cx);
    let editor = editor_of(cx, &s, "a.yml");
    send_from_toolbar(cx, &s, &editor);
    assert!(visible(cx, s.win, "response-warning-ask-again"));
    ScriptTrust::update(trust, |t| t.set_decision(&s.root, None)).unwrap();
    refresh_approvals(cx);
    assert!(!visible(cx, s.win, "response-warning-ask-again"));
}

// Tells the approvals model its trust file changed, as the file watcher would.
fn refresh_approvals(cx: &mut TestAppContext) {
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
}

// Changes the settings the way Preferences does, then lets every observer run.
fn set_settings(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn height(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> Pixels {
    let id = id.into();
    draw(cx, win);
    cx.update_window(win, |_, w, _| w.find(id).bounds().size.height)
        .unwrap()
}

// The dock opened by its chord and drawn once.
fn open_dock(cx: &mut TestAppContext, s: &Started) -> Entity<ConsoleDock> {
    press_in_shell(cx, s, "ctrl-`");
    draw(cx, s.win);
    s.shell.read_with(cx, |sh, _| sh.dock().clone())
}

#[gpui_kit::test]
fn the_sidebar_follows_the_zoom(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    let before = height(cx, s.win, row("j.yml"));
    set_settings(cx, |p| p.zoom = 2.0);
    let after = height(cx, s.win, row("j.yml"));
    assert!(after > before * 1.5, "{before:?} -> {after:?}");
}

#[gpui_kit::test]
fn the_sidebar_follows_the_theme(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    let before = sidebar.read_with(cx, |v, _| v.renders());
    set_settings(cx, |p| p.theme = ThemeChoice::Dark);
    draw(cx, s.win);
    assert!(sidebar.read_with(cx, |v, _| v.renders()) > before);
}

#[gpui_kit::test]
fn the_dock_follows_the_zoom(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    open_dock(cx, &s);
    let before = height(cx, s.win, ("dock-tab-label", 0usize));
    set_settings(cx, |p| p.zoom = 2.0);
    let after = height(cx, s.win, ("dock-tab-label", 0usize));
    assert!(after > before * 1.5, "{before:?} -> {after:?}");
}

#[gpui_kit::test]
fn the_dock_follows_the_theme(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    let dock = open_dock(cx, &s);
    let before = dock.read_with(cx, |v, _| v.renders());
    set_settings(cx, |p| p.theme = ThemeChoice::Dark);
    draw(cx, s.win);
    assert!(dock.read_with(cx, |v, _| v.renders()) > before);
}

#[gpui_kit::test]
fn the_response_follows_the_zoom(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    let before = height(cx, s.win, "response-add-env");
    set_settings(cx, |p| p.zoom = 2.0);
    let after = height(cx, s.win, "response-add-env");
    assert!(after > before * 1.5, "{before:?} -> {after:?}");
}

#[gpui_kit::test]
fn the_response_follows_the_theme(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    let response = editor.read_with(cx, |e, _| e.response().clone());
    let before = response.read_with(cx, |v, _| v.renders());
    set_settings(cx, |p| p.theme = ThemeChoice::Dark);
    draw(cx, s.win);
    assert!(response.read_with(cx, |v, _| v.renders()) > before);
}

#[gpui_kit::test]
fn the_live_body_follows_the_response_font_offset(cx: &mut TestAppContext) {
    http_ready(cx);
    // No Finish step, so the stream stays open and its rows stay on screen.
    let server = sse(
        cx,
        Script::with_type("text/plain", vec![Step::Chunk("hi\n".into())]),
    );
    let col = sse_collection(&server);
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "a.yml");
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.row_count() > 0))
    });
    let before = height(cx, s.win, ("live-row", 0usize));
    set_settings(cx, |p| p.response_font_offset = 10.0);
    let after = height(cx, s.win, ("live-row", 0usize));
    assert!(after > before, "{before:?} -> {after:?}");
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

// The kept-response label of a.yml's pane, as last drawn.
fn saved_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> String {
    draw(cx, win);
    cx.update_window(win, |_, w, _| {
        w.find("response-saved").label().map(str::to_owned)
    })
    .unwrap()
    .unwrap_or_default()
}

// The app with a.yml showing a response kept `age` ago.
fn showing_a_kept_response(
    cx: &mut TestAppContext,
    age: Duration,
) -> (Started, Entity<ResponsePane>, tempfile::TempDir) {
    let col = two_requests();
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "a.yml");
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("kept")),
        Vec::new(),
    );
    let saved_at = cx.update(|cx| wall_now(cx)) - age;
    let restored = Restored {
        saved_at,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.restore(Arc::new(result), restored, cx));
    let response = editor.read_with(cx, |e, _| e.response().clone());
    draw(cx, s.win);
    (s, response, col)
}

#[gpui_kit::test]
fn a_kept_response_age_keeps_ticking(cx: &mut TestAppContext) {
    let (s, response, _col) = showing_a_kept_response(cx, Duration::from_secs(50));
    let label = saved_label(cx, s.win);
    assert!(label.ends_with("(just now)"), "{label}");
    cx.executor().advance_clock(Duration::from_secs(30));
    let label = saved_label(cx, s.win);
    assert!(label.ends_with("(1 minute ago)"), "{label}");
    // The next change is a minute away, so half a minute more redraws nothing.
    let renders = response.read_with(cx, |v, _| v.renders());
    cx.executor().advance_clock(Duration::from_secs(20));
    draw(cx, s.win);
    assert_eq!(response.read_with(cx, |v, _| v.renders()), renders);
}

#[gpui_kit::test]
fn a_pane_without_an_age_label_never_ticks(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    let response = editor.read_with(cx, |e, _| e.response().clone());
    let renders = response.read_with(cx, |v, _| v.renders());
    cx.executor().advance_clock(Duration::from_secs(7200));
    draw(cx, s.win);
    assert_eq!(response.read_with(cx, |v, _| v.renders()), renders);
}

#[test]
fn an_age_label_changes_at_its_own_granularity() {
    let at = |s: u64| next_age_change(Duration::from_secs(s));
    assert_eq!(at(0), Duration::from_secs(60));
    assert_eq!(at(59), Duration::from_secs(1));
    assert_eq!(at(60), Duration::from_secs(60));
    assert_eq!(at(150), Duration::from_secs(30));
    assert_eq!(at(3_600), Duration::from_secs(3_600));
    assert_eq!(at(5_400), Duration::from_secs(1_800));
    assert_eq!(at(86_400 + 10), Duration::from_secs(86_390));
    assert_eq!(
        next_age_change(Duration::from_millis(59_500)),
        Duration::from_millis(500)
    );
}

// The app with j.yml's last send failed and one older send of it kept `age` ago, so the
// pane offers "Previous runs" but draws no "N ago" label of its own.
fn history_after_a_failed_send(
    cx: &mut TestAppContext,
    age: Duration,
) -> (Started, Entity<ResponsePane>, tempfile::TempDir) {
    http_ready(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("j.yml", BODY_REQUEST)]);
    let saved_at = cx.update(|cx| wall_now(cx)) - age;
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |paths| {
        let kept = RequestResult::from_parts(
            ip("j.yml"),
            "j",
            Outcome::Passed,
            Some(Response::new(200).with_text_body("kept")),
            Vec::new(),
        );
        ResponseStore::new(paths.responses.clone())
            .save(&root, &ip("j.yml"), &kept, saved_at)
            .unwrap();
    });
    let editor = editor_of(cx, &s, "j.yml");
    let response = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        response.read_with(cx, |p, _| p.history().len() == 1)
    });
    send_from_toolbar(cx, &s, &editor);
    assert!(visible(cx, s.win, "response-history"));
    assert!(!visible(cx, s.win, "response-saved"));
    (s, response, col)
}

// The first "Previous runs" row's name in a freshly opened menu, which is closed again.
fn first_history_row(cx: &mut TestAppContext, win: AnyWindowHandle) -> String {
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    draw(cx, win);
    let name = cx
        .update_window(win, |_, w, _| {
            w.within("popup-menu")
                .find(("history-row", 0usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap()
        .unwrap_or_default();
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    draw(cx, win);
    name
}

#[gpui_kit::test]
fn a_closed_history_menu_arms_no_age_timer(cx: &mut TestAppContext) {
    let (s, response, _col) = history_after_a_failed_send(cx, Duration::from_secs(30));
    let renders = response.read_with(cx, |v, _| v.renders());
    cx.executor().advance_clock(Duration::from_secs(7200));
    draw(cx, s.win);
    assert_eq!(response.read_with(cx, |v, _| v.renders()), renders);
}

#[gpui_kit::test]
fn history_rows_read_the_time_the_menu_opens(cx: &mut TestAppContext) {
    let (s, _response, _col) = history_after_a_failed_send(cx, Duration::from_secs(30));
    let row = first_history_row(cx, s.win);
    assert!(row.ends_with("just now"), "{row}");
    cx.executor().advance_clock(Duration::from_secs(60));
    let row = first_history_row(cx, s.win);
    assert!(row.ends_with("1 minute ago"), "{row}");
}

#[gpui_kit::test]
fn clearing_history_of_a_pane_without_a_key_redraws_it(cx: &mut TestAppContext) {
    let (s, _response, _col) = history_after_a_failed_send(cx, Duration::from_secs(30));
    std::fs::remove_file(s.root.join("j.yml")).unwrap();
    cx.run_until_parked();
    draw(cx, s.win);
    cx.update(clear_all);
    assert!(!visible(cx, s.win, "response-history"));
}

// The request editor's chrome: breadcrumb, method, custom method, URL, buttons and section
// bar, each its own cached view. A body key redraws none of them, and each follows every
// input it reads with nothing but a plain draw.

// What a drawn element says: its label, or its value for an input.
fn drawn(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    draw(cx, win);
    cx.update_window(win, |_, w, _| {
        let found = w.find(id.clone());
        found
            .value()
            .or(found.label())
            .map(str::to_owned)
            .unwrap_or_default()
    })
    .unwrap()
}

fn shows(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    draw(cx, win);
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

fn width(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> Pixels {
    let id = id.into();
    draw(cx, win);
    cx.update_window(win, |_, w, _| w.find(id).bounds().size.width)
        .unwrap()
}

fn chrome_renders(cx: &mut TestAppContext, editor: &Entity<RequestEditor>) -> Vec<usize> {
    ChromePart::ALL
        .iter()
        .map(|&p| {
            let view = editor.read_with(cx, |e, _| e.chrome(p).clone());
            view.read_with(cx, |v, _| v.renders())
        })
        .collect()
}

fn edit_j(cx: &mut TestAppContext, s: &Started, target: Target, edit: Edit) {
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Edit { target, edit }, cx)
    });
    cx.run_until_parked();
}

fn j() -> Target {
    Target::Request(ip("j.yml"))
}

#[gpui_kit::test]
fn a_body_key_redraws_none_of_the_request_chrome(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    let before = chrome_renders(cx, &editor);
    type_burst(cx, s.win);
    let after = chrome_renders(cx, &editor);
    // The counts measured: the URL follows the edits' variable sets and the buttons show
    // Save once the body is dirty; nothing else redraws.
    let most = [0, 0, 0, 3, 2, 0];
    for (((part, b), a), most) in ChromePart::ALL.iter().zip(before).zip(after).zip(most) {
        assert!(a - b <= most, "{KEYS} keys redrew {part:?} {} times", a - b);
    }
}

// The editor's variable set as `(allocation, revision)`.
fn variable_set(cx: &mut TestAppContext, editor: &Entity<RequestEditor>) -> (usize, u64) {
    editor.read_with(cx, |e, _| {
        let set = e.variables().expect("a set");
        (Arc::as_ptr(set) as usize, set.revision())
    })
}

#[gpui_kit::test]
fn a_body_key_keeps_the_variable_set_and_the_url(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    type_keys(cx, s.win, 1);
    let set = variable_set(cx, &editor);
    let url = chrome_renders(cx, &editor)[ChromePart::Url as usize];
    type_keys(cx, s.win, 1);
    assert_eq!(variable_set(cx, &editor), set);
    assert_eq!(chrome_renders(cx, &editor)[ChromePart::Url as usize], url);
}

#[gpui_kit::test]
fn a_request_variable_edit_replaces_the_variable_set(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    let (before, _) = variable_set(cx, &editor);
    let nope = Variable::new("nope", VarValue::Text("x".into()));
    edit_j(cx, &s, j(), Edit::Vars(vec![nope]));
    draw(cx, s.win);
    let (after, _) = variable_set(cx, &editor);
    assert_ne!(after, before);
    assert!(editor.read_with(cx, |e, _| e.variables().unwrap().get("nope").is_some()));
}

#[gpui_kit::test]
fn the_breadcrumb_follows_a_rename_of_the_request(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    // The name is plain text, so its drawn width is what tells the names apart.
    let before = width(cx, s.win, "breadcrumb-file");
    edit_j(cx, &s, j(), Edit::Name("a much longer name".into()));
    let after = width(cx, s.win, "breadcrumb-file");
    assert!(after > before * 4., "{before:?} -> {after:?}");
}

#[gpui_kit::test]
fn the_breadcrumb_follows_a_rename_of_the_collection(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    assert_eq!(drawn(cx, s.win, "breadcrumb-collection"), "c");
    edit_j(cx, &s, Target::Collection, Edit::Name("Shop".into()));
    assert_eq!(drawn(cx, s.win, "breadcrumb-collection"), "Shop");
}

#[gpui_kit::test]
fn the_breadcrumb_follows_a_rename_of_its_folder(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("Old")),
        ("f/k.yml", &request("k", "http://127.0.0.1:9/k")),
    ]);
    let s = started(cx, &col);
    let _editor = editor_of(cx, &s, "f/k.yml");
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    type_burst(cx, s.win);
    assert_eq!(drawn(cx, s.win, ("breadcrumb-folder", 0usize)), "Old");
    edit_j(cx, &s, Target::Folder(ip("f")), Edit::Name("New".into()));
    assert_eq!(drawn(cx, s.win, ("breadcrumb-folder", 0usize)), "New");
}

#[gpui_kit::test]
fn the_method_follows_an_edit_from_elsewhere(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    assert!(drawn(cx, s.win, "method").contains("POST"));
    edit_j(cx, &s, j(), Edit::Method("PUT".into()));
    assert!(drawn(cx, s.win, "method").contains("PUT"));
}

#[gpui_kit::test]
fn the_custom_method_shows_what_is_typed(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click("custom-method", cx);
        w.input("PURGE", cx);
    })
    .unwrap();
    assert_eq!(drawn(cx, s.win, "custom-method"), "PURGE");
}

#[gpui_kit::test]
fn the_url_follows_an_edit_from_elsewhere(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    edit_j(cx, &s, j(), Edit::Url("http://127.0.0.1:9/moved".into()));
    assert_eq!(drawn(cx, s.win, "url"), "http://127.0.0.1:9/moved");
}

// The URL's tooltip for `{{nope}}` (characters 19 to 27 of BODY_REQUEST's URL), hovered and
// drawn without a refresh.
fn nope_tip(cx: &mut TestAppContext, s: &Started, editor: &Entity<RequestEditor>) -> String {
    draw(cx, s.win);
    let input = editor.read_with(cx, |e, _| e.url_input().clone());
    let at = input
        .read_with(cx, |i, _| i.range_to_bounds(&(19..27)))
        .expect("laid out")
        .center();
    cx.update_window(s.win, |_, w, cx| {
        let event = MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        };
        w.dispatch_event(event.to_platform_input(), cx);
    })
    .unwrap();
    draw(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find("var-tooltip")
            .and_then(|t| t.label().map(str::to_owned))
            .unwrap_or_default()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_url_follows_the_environment(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: nope\n    value: here\n",
        ),
        ("j.yml", BODY_REQUEST),
    ]);
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "j.yml");
    wait_for(cx, "variables", |cx| {
        editor.read_with(cx, |e, _| e.variables().is_some())
    });
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
    })
    .unwrap();
    draw(cx, s.win);
    type_burst(cx, s.win);
    assert_eq!(nope_tip(cx, &s, &editor), "Undefined variable");
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    wait_for(cx, "the environment's variables", |cx| {
        editor.read_with(cx, |e, _| {
            e.variables().is_some_and(|v| v.get("nope").is_some())
        })
    });
    assert_eq!(nope_tip(cx, &s, &editor), "Environment · here");
}

#[gpui_kit::test]
fn save_follows_the_dirty_state_and_the_autosave_setting(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    assert!(!visible(cx, s.win, "save-request"));
    type_burst(cx, s.win);
    assert!(visible(cx, s.win, "save-request"), "typing made it dirty");
    set_settings(cx, |p| p.autosave = true);
    assert!(!visible(cx, s.win, "save-request"), "autosave writes it");
    set_settings(cx, |p| p.autosave = false);
    assert!(visible(cx, s.win, "save-request"));
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Save(j()), cx));
    cx.run_until_parked();
    assert!(!visible(cx, s.win, "save-request"), "saved");
}

#[gpui_kit::test]
fn the_send_slot_follows_a_send_in_flight(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", &format!("{}/slow", server.uri()))),
    ]);
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "a.yml");
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    type_burst(cx, s.win);
    assert!(visible(cx, s.win, "send"));
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_sending()));
    assert!(visible(cx, s.win, "cancel"));
    assert!(!visible(cx, s.win, "send"));
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
    assert!(visible(cx, s.win, "send"));
}

#[gpui_kit::test]
fn the_section_bar_follows_the_counts_and_marks(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    let headers = ("section-count", Section::Headers as usize);
    let script = ("section-mode", Section::Script as usize);
    assert!(!shows(cx, s.win, headers));
    assert!(!shows(cx, s.win, script));
    edit_j(cx, &s, j(), Edit::Headers(vec![KeyValue::new("X-A", "1")]));
    assert!(shows(cx, s.win, headers));
    edit_j(
        cx,
        &s,
        j(),
        Edit::Script(ScriptPhase::PreRequest, "bru.x = 1".into()),
    );
    assert!(shows(cx, s.win, script));
}

#[gpui_kit::test]
fn the_section_bar_follows_a_section_switch(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    let bar = editor.read_with(cx, |e, _| e.chrome(ChromePart::Sections).clone());
    let renders = bar.read_with(cx, |v, _| v.renders());
    editor.update(cx, |e, cx| e.show(Section::Headers, cx));
    draw(cx, s.win);
    assert!(bar.read_with(cx, |v, _| v.renders()) > renders);
    let mut selected = |ix: Section| {
        cx.update_window(s.win, |_, w, _| {
            w.within("sections")
                .find(ElementId::Integer(ix as u64))
                .selected()
        })
        .unwrap()
    };
    assert_eq!(selected(Section::Headers), Some(true));
    assert_eq!(selected(Section::Body), Some(false));
}

#[gpui_kit::test]
fn the_request_chrome_follows_the_zoom(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    let ids: [ElementId; 4] = [
        "breadcrumb".into(),
        "url".into(),
        "method".into(),
        "section-strip".into(),
    ];
    let before: Vec<_> = ids.iter().map(|id| height(cx, s.win, id.clone())).collect();
    set_settings(cx, |p| p.zoom = 2.0);
    for (id, b) in ids.iter().zip(before) {
        let a = height(cx, s.win, id.clone());
        assert!(a > b * 1.3, "{id:?}: {b:?} -> {a:?}");
    }
}

#[gpui_kit::test]
fn the_request_chrome_follows_the_theme(cx: &mut TestAppContext) {
    let (s, editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    let before = chrome_renders(cx, &editor);
    set_settings(cx, |p| p.theme = ThemeChoice::Dark);
    draw(cx, s.win);
    let after = chrome_renders(cx, &editor);
    for ((part, b), a) in ChromePart::ALL.iter().zip(before).zip(after) {
        assert!(a > b, "{part:?} did not redraw");
    }
}

#[gpui_kit::test]
fn the_method_menu_opens_with_a_plain_draw(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    type_burst(cx, s.win);
    let expanded = |cx: &mut TestAppContext| {
        draw(cx, s.win);
        cx.update_window(s.win, |_, w, _| w.find("method").expanded())
            .unwrap()
    };
    assert_eq!(expanded(cx), Some(false));
    cx.update_window(s.win, |_, w, cx| w.click("method", cx))
        .unwrap();
    assert_eq!(expanded(cx), Some(true));
}

#[gpui_kit::test]
fn the_url_locks_while_a_params_edit_is_in_flight(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a?q=1")),
    ]);
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "a.yml");
    draw(cx, s.win);
    type_at_end(cx, s.win, Some("params"), ("kv-value", 0usize), "9");
    let locked = |cx: &mut TestAppContext| editor.read_with(cx, |e, cx| e.locked(Side::Url, cx));
    assert!(locked(cx));
    // The click's focus change refreshes the window, so the locked URL is drawn regardless.
    cx.update_window(s.win, |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    // The lock shows only as the field's fill and text colour, which snapshots do not carry,
    // so the unlock is seen as the URL part drawing again with no refresh.
    let url = editor.read_with(cx, |e, _| e.chrome(ChromePart::Url).clone());
    let renders = url.read_with(cx, |v, _| v.renders());
    draw(cx, s.win);
    assert!(!locked(cx));
    assert!(url.read_with(cx, |v, _| v.renders()) > renders);
}

// The shell's chrome: the tab strip, the title bar and the status bar, each drawn by a view of
// its own that a body key leaves alone and that still follows every input it reads.

fn shell_chrome_renders(cx: &mut TestAppContext, s: &Started) -> [usize; 3] {
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let strip = area.read_with(cx, |a, _| a.strip().clone());
    let (title, status) = s.shell.read_with(cx, |sh, _| {
        (sh.title_part().clone(), sh.status_part().clone())
    });
    [
        strip.read_with(cx, |v, _| v.renders()),
        title.read_with(cx, |v, _| v.renders()),
        status.read_with(cx, |v, _| v.renders()),
    ]
}

// a.yml and j.yml open, j.yml active with its Body focused and the caret at the end.
fn two_tabs_typing(cx: &mut TestAppContext) -> (Started, [TabId; 2], tempfile::TempDir) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("b.yml", &request("b", "http://127.0.0.1:9/b")),
        ("j.yml", BODY_REQUEST),
    ]);
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    let j = open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("body").click_at("body-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    draw(cx, s.win);
    (s, [a, j], col)
}

#[gpui_kit::test]
fn a_body_key_redraws_neither_the_tab_strip_nor_the_bars(cx: &mut TestAppContext) {
    let (s, _editor, _col) = typing_after_a_send(cx);
    let before = shell_chrome_renders(cx, &s);
    type_burst(cx, s.win);
    let after = shell_chrome_renders(cx, &s);
    // The counts measured: the strip draws the dirty mark the first key sets; the bars never
    // redraw.
    let parts = [("strip", 2), ("title", 0), ("status", 0)];
    for (((part, most), b), a) in parts.into_iter().zip(before).zip(after) {
        assert!(
            a - b <= most,
            "{KEYS} keys redrew the {part} {} times",
            a - b
        );
    }
}

#[gpui_kit::test]
fn the_tab_strip_follows_a_rename(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    // The name is plain text, so its drawn width is what tells the names apart.
    let before = width(cx, s.win, ("tab-name", 1usize));
    edit_j(cx, &s, j(), Edit::Name("a much longer name".into()));
    let after = width(cx, s.win, ("tab-name", 1usize));
    assert!(after > before * 4., "{before:?} -> {after:?}");
}

#[gpui_kit::test]
fn the_tab_strip_follows_the_dirty_state(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    assert!(!shows(cx, s.win, ("dirty", 1usize)));
    type_burst(cx, s.win);
    assert!(shows(cx, s.win, ("dirty", 1usize)), "typing made it dirty");
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Save(j()), cx));
    cx.run_until_parked();
    assert!(!shows(cx, s.win, ("dirty", 1usize)), "saved");
}

#[gpui_kit::test]
fn the_tab_strip_follows_the_active_tab(cx: &mut TestAppContext) {
    let (s, [a, _], _col) = two_tabs_typing(cx);
    let target = Target::Request(ip("a.yml"));
    edit_j(cx, &s, target, Edit::Url("http://127.0.0.1:9/b".into()));
    type_burst(cx, s.win);
    // An inactive dirty tab holds its dot where the close button goes.
    assert!(!shows(cx, s.win, ("tab-close", 0usize)));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(a)));
    assert!(shows(cx, s.win, ("tab-close", 0usize)));
}

#[gpui_kit::test]
fn the_tab_strip_follows_the_hover(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    let target = Target::Request(ip("a.yml"));
    edit_j(cx, &s, target, Edit::Url("http://127.0.0.1:9/b".into()));
    type_burst(cx, s.win);
    assert!(!shows(cx, s.win, ("tab-close", 0usize)));
    cx.update_window(s.win, |_, w, cx| w.hover(("tab", 0usize), cx))
        .unwrap();
    assert!(shows(cx, s.win, ("tab-close", 0usize)));
}

#[gpui_kit::test]
fn the_tab_strip_follows_pinning_and_the_preview(cx: &mut TestAppContext) {
    let (s, [a, _], _col) = two_tabs_typing(cx);
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a.yml"), true, cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(1)));
    draw(cx, s.win);
    type_burst(cx, s.win);
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(a, true)));
    assert!(shows(cx, s.win, ("tab-pin", 0usize)));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(a, false)));
    assert!(!shows(cx, s.win, ("tab-pin", 0usize)));
    // A new preview tab is drawn in italics until it is kept.
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("b.yml"), true, cx));
    assert!(shows(cx, s.win, ("preview", 2usize)));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("b.yml"), false, cx));
    assert!(!shows(cx, s.win, ("preview", 2usize)));
}

#[gpui_kit::test]
fn the_tab_strip_follows_a_drag_over_it(cx: &mut TestAppContext) {
    let (s, [a, j], _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    assert!(!shows(cx, s.win, "tab-drop-indicator"));
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let dragged = DraggedTab::new(j, s.root.clone(), "j".into());
    area.update(cx, |t, cx| {
        t.hover_drop(&dragged, a, Some(TabSide::Before), cx)
    });
    assert!(shows(cx, s.win, "tab-drop-indicator"));
    area.update(cx, |t, cx| t.hover_drop(&dragged, a, None, cx));
    assert!(!shows(cx, s.win, "tab-drop-indicator"));
}

#[gpui_kit::test]
fn the_tab_strip_follows_the_method(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    // The tag keeps its width and only its text and colour change, which snapshots do not
    // carry, so the change is seen as the strip drawing again with no refresh.
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let strip = area.read_with(cx, |a, _| a.strip().clone());
    draw(cx, s.win);
    let renders = strip.read_with(cx, |v, _| v.renders());
    edit_j(cx, &s, j(), Edit::Method("DELETE".into()));
    draw(cx, s.win);
    assert!(strip.read_with(cx, |v, _| v.renders()) > renders);
}

#[gpui_kit::test]
fn the_tab_strip_follows_a_git_change(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("j.yml", BODY_REQUEST),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "init");
    http_ready(cx);
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    type_burst(cx, s.win);
    assert!(!shows(cx, s.win, ("tab-git", 0usize)));
    assert_eq!(title_branch(cx, &s).as_deref(), Some("main"));
    git_write(
        col.path(),
        "a.yml",
        &request("a", "http://127.0.0.1:9/changed"),
    );
    s.ws.update(cx, |w, cx| w.refresh_all_git(cx));
    wait_for(cx, "the change", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.git_changes(&s.root).is_some_and(|c| !c.files.is_empty())
        })
    });
    assert!(shows(cx, s.win, ("tab-git", 0usize)));
    assert_eq!(title_branch(cx, &s).as_deref(), Some("main, 1 changed"));
}

fn title_branch(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    draw(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.within("title-bar")
            .try_find("git-label")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_tab_strip_keeps_the_active_tab_in_view(cx: &mut TestAppContext) {
    http_ready(cx);
    let names: Vec<String> = (0..12).map(|i| format!("request-number-{i}.yml")).collect();
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    for n in &names {
        files.push((n.clone(), request(n, "http://127.0.0.1:9/x")));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let col = write_tree(&refs);
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(900.), px(700.)));
    for n in &names {
        open_tab(cx, &s.ws, &s.root, n);
    }
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    type_burst(cx, s.win);
    let in_view = |cx: &mut TestAppContext, ix: usize| {
        draw(cx, s.win);
        draw(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            let strip = w.find("tab-strip").bounds();
            let tab = w.find(("tab", ix)).bounds();
            tab.left() >= strip.left() && tab.right() <= strip.right()
        })
        .unwrap()
    };
    assert!(in_view(cx, 11), "the last opened tab is active and shown");
    assert!(!in_view(cx, 0), "the strip overflows");
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    assert!(
        in_view(cx, 0),
        "a switch scrolls the strip to the active tab"
    );
    // Wheeled away, a pick of the active tab in the overflow menu brings it back.
    let strip = cx
        .update_window(s.win, |_, w, _| w.find("tab-strip").bounds())
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        let event = ScrollEvent {
            position: strip.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            ..Default::default()
        };
        w.dispatch_event(event.to_platform_input(), cx);
    })
    .unwrap();
    assert!(!in_view(cx, 0), "the wheel moved it out");
    cx.update_window(s.win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    draw(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    assert!(in_view(cx, 0), "the menu pick revealed it again");
}

#[gpui_kit::test]
fn the_title_bar_follows_a_rename_of_the_collection(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    assert!(drawn(cx, s.win, "collection-switcher").contains('c'));
    edit_j(cx, &s, Target::Collection, Edit::Name("Shop".into()));
    assert!(drawn(cx, s.win, "collection-switcher").contains("Shop"));
}

#[gpui_kit::test]
fn the_title_bar_follows_the_environment(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: nope\n    value: here\n",
        ),
        ("j.yml", BODY_REQUEST),
    ]);
    let s = started(cx, &col);
    let _editor = editor_of(cx, &s, "j.yml");
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    type_burst(cx, s.win);
    assert!(!drawn(cx, s.win, "env-select").contains("dev"));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    assert!(drawn(cx, s.win, "env-select").contains("dev"));
}

#[gpui_kit::test]
fn the_title_bar_follows_the_window_width(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    assert!(shows(cx, s.win, "variables-button"));
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(400.), px(700.)));
    assert!(!shows(cx, s.win, "variables-button"));
}

#[gpui_kit::test]
fn the_status_bar_follows_the_response_layout(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    let status = s.shell.read_with(cx, |sh, _| sh.status_part().clone());
    let renders = status.read_with(cx, |v, _| v.renders());
    // The layout icon is all that changes, which snapshots do not carry, so the change is
    // seen as the bar drawing again with no refresh.
    set_settings(cx, |p| p.response_orientation = Orientation::Horizontal);
    draw(cx, s.win);
    assert!(status.read_with(cx, |v, _| v.renders()) > renders);
}

// The collection's watch state moves with no workspace notify and only the shell redrawn, so
// the status bar's own unwatched input is all that can make it draw the marker.
#[gpui_kit::test]
fn the_status_bar_follows_the_unwatched_collections_alone(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    assert!(!visible_id(cx, s.win, ("unwatched", 0usize)));
    let status = s.shell.read_with(cx, |sh, _| sh.status_part().clone());
    let renders = status.read_with(cx, |v, _| v.renders());
    s.ws.update(cx, |w, _| {
        w.set_watch_failed_silently(&s.root, Some("limit".into()))
    });
    s.shell.update(cx, |_, cx| cx.notify());
    assert!(visible_id(cx, s.win, ("unwatched", 0usize)));
    assert!(status.read_with(cx, |v, _| v.renders()) > renders);
}

// The lock changes with no model or editor notify and only the shell redrawn; the URL part
// must draw again from its lock input alone.
#[gpui_kit::test]
fn the_url_follows_its_lock_alone(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a?q=1")),
    ]);
    let s = started(cx, &col);
    let editor = editor_of(cx, &s, "a.yml");
    draw(cx, s.win);
    type_at_end(cx, s.win, Some("params"), ("kv-value", 0usize), "9");
    let locked = |cx: &mut TestAppContext| editor.read_with(cx, |e, cx| e.locked(Side::Url, cx));
    assert!(locked(cx));
    // Drawn without parking, so the edit stays in flight and the URL part is drawn locked.
    cx.update_window(s.win, |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    let url = editor.read_with(cx, |e, _| e.chrome(ChromePart::Url).clone());
    let renders = url.read_with(cx, |v, _| v.renders());
    editor.update(cx, |e, _| e.set_sender_silently(Side::Url));
    assert!(!locked(cx));
    s.shell.update(cx, |_, cx| cx.notify());
    cx.update_window(s.win, |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
    assert!(url.read_with(cx, |v, _| v.renders()) > renders);
}

#[gpui_kit::test]
fn the_shell_chrome_follows_the_zoom(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    let ids: [ElementId; 3] = ["tab-strip".into(), "title-row".into(), "status-bar".into()];
    let before: Vec<_> = ids.iter().map(|id| height(cx, s.win, id.clone())).collect();
    set_settings(cx, |p| p.zoom = 2.0);
    for (id, b) in ids.iter().zip(before) {
        let a = height(cx, s.win, id.clone());
        assert!(a > b * 1.3, "{id:?}: {b:?} -> {a:?}");
    }
}

#[gpui_kit::test]
fn the_shell_chrome_follows_the_theme(cx: &mut TestAppContext) {
    let (s, _tabs, _col) = two_tabs_typing(cx);
    type_burst(cx, s.win);
    let before = shell_chrome_renders(cx, &s);
    set_settings(cx, |p| p.theme = ThemeChoice::Dark);
    draw(cx, s.win);
    let after = shell_chrome_renders(cx, &s);
    for ((part, b), a) in ["strip", "title", "status"].iter().zip(before).zip(after) {
        assert!(a > b, "the {part} did not redraw");
    }
}

#[test]
fn a_freed_value_never_shares_an_identity_with_a_new_one() {
    // The allocator hands a freed block straight back, so an address alone would alias.
    for _ in 0..100 {
        let old = Arc::new(1u64);
        let seen = Same::of(&old);
        drop(old);
        let new = Arc::new(2u64);
        assert!(seen != Same::of(&new));
        assert!(Same::of(&new) == Same::of(&new));
    }
}
