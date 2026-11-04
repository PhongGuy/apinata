//! The focus and navigation commands: Focus URL, Next and Previous section, Focus response
//! and Open environment selector, all through the real keymap.

use crate::support;

use std::sync::Arc;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::tabs::TabId;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use support::*;
use tempfile::TempDir;

const LONG: usize = 30_000;

fn long_json() -> String {
    let items: Vec<String> = (0..LONG).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

// A started app with request a open in the active tab, one frame drawn.
fn app(cx: &mut TestAppContext, col: &TempDir) -> (Started, TabId, Entity<RequestEditor>) {
    let s = started(cx, col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, tab);
    (s, tab, ed)
}

fn press(cx: &mut TestAppContext, s: &Started, ks: &str) {
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.simulate_keystrokes(ks);
    v.run_until_parked();
    frame(cx, s.win);
}

fn url_focused(cx: &mut TestAppContext, s: &Started, ed: &Entity<RequestEditor>) -> bool {
    let handle = ed.read_with(cx, |e, cx| e.url_focus(cx));
    cx.update_window(s.win, |_, w, _| handle.is_focused(w))
        .unwrap()
}

fn focus(cx: &mut TestAppContext, s: &Started, handle: gpui_kit::FocusHandle) {
    cx.update_window(s.win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn section(cx: &mut TestAppContext, ed: &Entity<RequestEditor>) -> Section {
    ed.read_with(cx, |e, _| e.section())
}

#[gpui_kit::test]
fn focus_url_works_from_the_sidebar(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    frame(cx, s.win);
    assert!(!url_focused(cx, &s, &ed));
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed));
}

#[gpui_kit::test]
fn focus_url_works_from_a_table_cell(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("params").click(("kv-name", 0usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(!url_focused(cx, &s, &ed));
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed));
}

#[gpui_kit::test]
fn focus_url_works_from_a_code_editor_and_from_the_url_itself(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    ed.update(cx, |e, cx| e.show(Section::Docs, cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("docs").click_at("docs-code", code_at(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(!url_focused(cx, &s, &ed));
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed));
    press(cx, &s, &key("l"));
    assert!(
        url_focused(cx, &s, &ed),
        "from the URL input it stays there"
    );
}

fn with_response(cx: &mut TestAppContext, ed: &Entity<RequestEditor>) {
    with_body(cx, ed, 200, "application/json", long_json());
}

fn with_body(
    cx: &mut TestAppContext,
    ed: &Entity<RequestEditor>,
    status: u16,
    content_type: &str,
    body: String,
) {
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(status)
                .with_header("content-type", content_type)
                .with_text_body(body),
        ),
        Vec::new(),
    );
    let restored = apinata_app::send_model::Restored {
        saved_at: std::time::UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.restore(Arc::new(result), restored, cx));
}

#[gpui_kit::test]
fn focus_url_works_from_the_response_pane(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    with_response(cx, &ed);
    frame(cx, s.win);
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    let handle = body.read_with(cx, |b, _| b.body_focus().clone());
    focus(cx, &s, handle);
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed));
}

#[gpui_kit::test]
fn the_section_commands_cycle_and_wrap(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    press(cx, &s, &key("l"));
    assert_eq!(section(cx, &ed), Section::Params);
    for want in Section::ALL.iter().skip(1).chain([&Section::Params]) {
        press(cx, &s, &key("alt-right"));
        assert_eq!(section(cx, &ed), *want);
    }
    press(cx, &s, &key("alt-left"));
    assert_eq!(section(cx, &ed), Section::Settings, "backward wraps");
    press(cx, &s, &key("alt-left"));
    assert_eq!(section(cx, &ed), Section::Docs);
}

#[gpui_kit::test]
fn the_section_commands_keep_word_navigation_in_inputs(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    press(cx, &s, &key("l"));
    press(cx, &s, "alt-left");
    assert_eq!(
        section(cx, &ed),
        Section::Params,
        "plain alt-left is the input's"
    );
}

#[gpui_kit::test]
fn focus_response_lands_on_the_body_and_down_moves_the_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    with_response(cx, &ed);
    frame(cx, s.win);
    press(cx, &s, &key("l"));
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    assert_eq!(body.read_with(cx, |b, _| b.current_row()), None);
    press(cx, &s, &key("alt-r"));
    let handle = body.read_with(cx, |b, _| b.body_focus().clone());
    assert!(
        cx.update_window(s.win, |_, w, _| handle.is_focused(w))
            .unwrap()
    );
    press(cx, &s, "down");
    assert!(body.read_with(cx, |b, _| b.current_row()).is_some());
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed), "focus stays reachable");
}

// Whether the window has any focus owner at all.
fn something_focused(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.focused(cx).is_some())
        .unwrap()
}

#[gpui_kit::test]
fn focus_response_focuses_an_editor_mode_body(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    with_body(
        cx,
        &ed,
        404,
        "text/html",
        "<html><body>Not found</body></html>".into(),
    );
    frame(cx, s.win);
    press(cx, &s, &key("l"));
    press(cx, &s, &key("alt-r"));
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    assert_eq!(
        body.read_with(cx, |b, _| b.shown()),
        apinata_app::body_view::Shown::Editor
    );
    let editor = body.read_with(cx, |b, cx| b.editor().read(cx).focus_handle(cx));
    assert!(
        cx.update_window(s.win, |_, w, _| editor.is_focused(w))
            .unwrap(),
        "the body's editor owns focus"
    );
    // The chords that need a focus owner keep working afterwards.
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed));
}

#[gpui_kit::test]
fn focus_response_from_another_response_tab_shows_the_body(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    with_body(cx, &ed, 404, "text/html", "<p>Not found</p>".into());
    let pane = ed.read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| {
        p.set_tab(apinata_app::response_pane::ResponseTab::Headers, cx)
    });
    frame(cx, s.win);
    press(cx, &s, &key("alt-r"));
    frame(cx, s.win);
    let body = pane.read_with(cx, |p, _| p.body().clone());
    let editor = body.read_with(cx, |b, cx| b.editor().read(cx).focus_handle(cx));
    assert!(
        cx.update_window(s.win, |_, w, _| editor.is_focused(w))
            .unwrap()
    );
}

#[gpui_kit::test]
fn focus_response_with_no_response_keeps_a_focus_owner(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    press(cx, &s, &key("l"));
    press(cx, &s, &key("alt-r"));
    assert!(something_focused(cx, &s), "focus is never lost");
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed));
}

#[gpui_kit::test]
fn the_environment_selector_command_opens_the_dropdown(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, _) = app(cx, &col);
    assert!(!present(cx, s.win, "env-none"));
    press(cx, &s, &key("alt-e"));
    assert!(present(cx, s.win, "env-none"));
}

const IDS: [&str; 5] = [
    "view.focus_url",
    "view.next_section",
    "view.previous_section",
    "view.focus_response",
    "environment.open_selector",
];

#[gpui_kit::test]
fn the_commands_are_in_the_palette_and_the_keyboard_section(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let palette = apinata_app::palette::palette_items();
    for id in IDS {
        let label = apinata_app::commands::label(id);
        assert!(palette.iter().any(|e| e.label == label), "{id}");
    }
    open_prefs(cx, &s);
    for id in IDS {
        assert!(present(cx, s.win, format!("key-{id}")), "{id}");
    }
    assert_eq!(key_label(cx, &s, "view.focus_url"), Some(secondary("l")));
    assert_eq!(
        key_label(cx, &s, "view.next_section"),
        Some(secondary("alt-right"))
    );
}

#[gpui_kit::test]
fn the_commands_can_be_rebound(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    edit_keys(cx, |o| {
        o.insert("view.focus_url".into(), vec!["secondary-k".into()]);
        o.insert("view.next_section".into(), vec!["secondary-alt-n".into()]);
    });
    press(cx, &s, &key("l"));
    assert!(!url_focused(cx, &s, &ed), "the old chord is gone");
    press(cx, &s, &key("k"));
    assert!(url_focused(cx, &s, &ed));
    press(cx, &s, &key("alt-n"));
    assert_eq!(section(cx, &ed), Section::Body);
}

// A section switch must not strand the keys: the next chord still has to reach the shell.
#[gpui_kit::test]
fn next_section_from_an_empty_headers_cell_keeps_working(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    ed.update(cx, |e, cx| e.show(Section::Headers, cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("headers").click(("kv-value", 0usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    press(cx, &s, &key("alt-right"));
    assert_eq!(section(cx, &ed), Section::Auth);
    press(cx, &s, &key("alt-right"));
    assert_eq!(section(cx, &ed), Section::Vars, "the second press lands");
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed), "focus-url still works");
}

#[gpui_kit::test]
fn previous_section_from_the_body_editor_keeps_working(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ed) = app(cx, &col);
    ed.update(cx, |e, cx| e.show(Section::Body, cx));
    let body = ed.read_with(cx, |e, _| e.body().clone());
    cx.update_window(s.win, |_, w, cx| {
        body.update(cx, |b, cx| {
            b.set_mode(apinata_app::body_section::BodyMode::Json, w, cx)
        })
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("body").click_at("body-code", code_at(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    press(cx, &s, &key("alt-left"));
    assert_eq!(section(cx, &ed), Section::Params);
    press(cx, &s, &key("alt-left"));
    assert_eq!(section(cx, &ed), Section::Settings, "wraps");
    press(cx, &s, &key("l"));
    assert!(url_focused(cx, &s, &ed), "focus-url still works");
}

// Ctrl-comma opens Preferences through the real keymap.
#[gpui_kit::test]
fn the_preferences_chord_opens_preferences(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, _) = app(cx, &col);
    press(cx, &s, &key(","));
    let open = cx
        .update_window(s.win, |_, w, _| w.try_find("preferences").is_some())
        .unwrap();
    assert!(open, "preferences opened");
}

// Commands that swap or relayout the editor must leave the keys working afterwards.
#[gpui_kit::test]
fn tab_and_layout_commands_from_a_table_cell_keep_the_keys_working(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _, ea) = app(cx, &col);
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    let eb = editor_of(cx, &s, b);
    for chord in ["ctrl-pageup".to_string(), key("j"), key("w")] {
        cx.update_window(s.win, |_, w, cx| {
            w.within("params").click(("kv-name", 0usize), cx)
        })
        .ok();
        frame(cx, s.win);
        press(cx, &s, &chord);
        press(cx, &s, &key("l"));
        assert!(
            url_focused(cx, &s, &ea) || url_focused(cx, &s, &eb),
            "{chord} then focus-url works"
        );
    }
}
