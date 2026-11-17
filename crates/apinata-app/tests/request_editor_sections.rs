use crate::support;

use apinata_app::auth_section::{AuthField, AuthMode};
use apinata_app::body_section::BodyMode;
use apinata_app::command::{Command, Edit, Target};
use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::settings_section::TimeoutMode;
use apinata_core::{AuthSpec, BodySpec};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, FocusHandle, TestAppContext};
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0\"\n  body:\n    type: json\n    data: \"{}\"\n  auth:\n    type: bearer\n    token: t\nsettings:\n  encodeUrl: true\n  timeout: 0\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: true\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)])
}

fn open_section(cx: &mut TestAppContext, win: AnyWindowHandle, s: Section) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", s as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
}

fn body_code(cx: &mut TestAppContext, ed: &Entity<RequestEditor>) -> (String, usize) {
    let code = ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    code.read_with(cx, |c, _| (c.value().to_string(), c.cursor()))
}

#[gpui_kit::test]
fn the_sections_show_the_snapshot(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert_eq!(
        ed.read_with(cx, |e, cx| e.body().read(cx).mode()),
        Some(BodyMode::Json)
    );
    assert_eq!(body_code(cx, &ed).0, "{}");
    let token = ed.read_with(cx, |e, cx| {
        e.auth().read(cx).input(AuthField::Token).clone()
    });
    assert_eq!(token.read_with(cx, |i, _| i.value().to_string()), "t");
    assert_eq!(
        ed.read_with(cx, |e, cx| e.settings().read(cx).timeout_mode()),
        TimeoutMode::Unlimited
    );
}

#[gpui_kit::test]
fn body_typing_reaches_the_draft_and_the_echo_keeps_the_caret(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    open_section(cx, win, Section::Body);
    replace_code(cx, win, Some("body"), "body-code", "{\"a\": 1");
    cx.update_window(win, |_, w, cx| w.press("left", cx))
        .unwrap();
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.body, BodySpec::Json("{\"a\": 1}".into()));
    assert!(view.dirty);
    assert_eq!(body_code(cx, &ed), ("{\"a\": 1}".into(), "{\"a\": ".len()));
}

#[gpui_kit::test]
fn section_edits_pin_a_preview_tab_and_reach_the_draft(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", true);
    open_section(cx, win, Section::Auth);
    type_at_end(cx, win, Some("auth"), "auth-token", "2");
    cx.run_until_parked();
    assert!(!ws.read_with(cx, |w, _| w.tabs().active().unwrap().preview));
    open_section(cx, win, Section::Settings);
    cx.update_window(win, |_, w, cx| {
        w.within("settings").click("set-follow-redirects", cx)
    })
    .unwrap();
    cx.run_until_parked();
    open_section(cx, win, Section::Docs);
    replace_code(cx, win, Some("docs"), "docs-code", "notes");
    cx.run_until_parked();
    open_section(cx, win, Section::Assertions);
    type_at_end(
        cx,
        win,
        Some("assertions"),
        ("as-expr", 0usize),
        "res.status",
    );
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.auth, AuthSpec::Bearer { token: "t2".into() });
    assert!(!view.settings.follow_redirects);
    assert_eq!(view.docs, "notes");
    assert_eq!(view.assertions.len(), 1);
}

#[gpui_kit::test]
fn an_outside_edit_reloads_idle_sections(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let target = || Target::Request(ip("a.yml"));
    for edit in [
        Edit::Docs("from outside".into()),
        Edit::Body(BodySpec::Text("t".into())),
        Edit::Auth(AuthSpec::None),
    ] {
        ws.update(cx, |w, cx| {
            w.send(
                &root,
                Command::Edit {
                    target: target(),
                    edit,
                },
                cx,
            )
        });
    }
    cx.run_until_parked();
    let docs = ed.read_with(cx, |e, cx| e.docs().read(cx).code().clone());
    assert_eq!(
        docs.read_with(cx, |c, _| c.value().to_string()),
        "from outside"
    );
    assert_eq!(
        ed.read_with(cx, |e, cx| e.body().read(cx).mode()),
        Some(BodyMode::Text)
    );
    assert_eq!(body_code(cx, &ed).0, "t");
    assert_eq!(
        ed.read_with(cx, |e, cx| e.auth().read(cx).mode()),
        Some(AuthMode::None)
    );
}

fn focused(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, h: &FocusHandle) -> bool {
    cx.update_window(win, |_, w, _| h.is_focused(w)).unwrap()
}

fn press(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(win, keys);
    cx.run_until_parked();
}

// A whole key press, down and up: the kit activates the focused tab on release.
fn activate(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
}

// The section tab holding keyboard focus, as an index into `Section::ALL`, after a frame.
fn focused_section(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &Entity<RequestEditor>,
) -> Option<usize> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        ed.read(cx).focused_section(w, cx)
    })
    .unwrap()
}

// Tabs forward from the URL until focus enters the section strip, and says where it landed.
fn tab_into_sections(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &Entity<RequestEditor>,
) -> usize {
    let url = ed.read_with(cx, |e, cx| e.url_focus(cx));
    cx.update_window(win, |_, w, cx| url.focus(w, cx)).unwrap();
    for _ in 0..3 {
        press(cx, win, "tab");
        if let Some(ix) = focused_section(cx, win, ed) {
            return ix;
        }
    }
    panic!("Tab from the url never reached the section strip");
}

#[gpui_kit::test]
fn tab_walks_the_method_the_url_and_the_section_strip(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let url = ed.read_with(cx, |e, cx| e.url_focus(cx));
    let method = ed.read_with(cx, |e, cx| e.method_focus(cx));
    cx.update_window(win, |_, w, cx| url.focus(w, cx)).unwrap();
    press(cx, win, "shift-tab");
    assert!(
        focused(cx, win, &method),
        "shift-tab from the url reaches the method"
    );
    press(cx, win, "tab");
    assert!(
        focused(cx, win, &url),
        "tab from the method reaches the url"
    );
    assert_eq!(
        tab_into_sections(cx, win, &ed),
        0,
        "Tab lands on the chosen section"
    );
    press(cx, win, "right");
    assert_eq!(focused_section(cx, win, &ed), Some(1));
    assert_eq!(
        ed.read_with(cx, |e, _| e.section()),
        Section::ALL[0],
        "an arrow moves focus without choosing"
    );
    activate(cx, win, "enter");
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::ALL[1]);
    press(cx, win, "left");
    activate(cx, win, "space");
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::ALL[0]);
    press(cx, win, "tab");
    assert_eq!(
        focused_section(cx, win, &ed),
        None,
        "one Tab leaves the strip"
    );
    press(cx, win, "shift-tab");
    assert_eq!(focused_section(cx, win, &ed), Some(0), "Shift+Tab returns");
    press(cx, win, "shift-tab");
    assert_eq!(
        focused_section(cx, win, &ed),
        None,
        "Shift+Tab leaves the strip backwards"
    );
}

// The kit draws keyboard focus as a 2px ring inset in its tab, which encloses our padded box,
// so a content dot at least that far inside the box never lies under the ring.
#[gpui_kit::test]
fn a_section_tab_focus_ring_clears_its_content_dot(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let ix = Section::Auth as usize;
    let (tab, dot) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let tab = w.find(("section-tab", ix)).bounds();
            (tab, w.find(("section-mode", ix)).bounds())
        })
        .unwrap();
    let ring = gpui_kit::px(2.);
    assert!(
        tab.left() + ring <= dot.left()
            && tab.right() - ring >= dot.right()
            && tab.top() + ring <= dot.top()
            && tab.bottom() - ring >= dot.bottom(),
        "the tab {tab:?} holds the dot {dot:?} clear of the ring"
    );
}

#[gpui_kit::test]
fn arrows_home_and_end_move_focus_along_the_section_strip(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let last = Section::ALL.len() - 1;
    assert_eq!(tab_into_sections(cx, win, &ed), 0);
    for (key, ix) in [
        ("right", 1),
        ("left", 0),
        ("left", last),
        ("right", 0),
        ("end", last),
        ("home", 0),
    ] {
        press(cx, win, key);
        assert_eq!(
            focused_section(cx, win, &ed),
            Some(ix),
            "{key} lands on section {ix}"
        );
    }
    assert_eq!(
        ed.read_with(cx, |e, _| e.section()),
        Section::ALL[0],
        "moving focus chooses nothing"
    );
}

const HEADERS: &str = "info:\n  name: h\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n  headers:\n    - name: A\n      value: \"1\"\n    - name: B\n      value: \"2\"\n    - name: C\n      value: \"3\"\n";

// Removing a header with its button leaves focus in the table, so the save key still works.
// `target` is the focused row's index once the removed row is gone.
fn header_delete_then_save(cx: &mut TestAppContext, ix: usize, target: usize) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("h.yml", HEADERS)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "h.yml", false);
    open_section(cx, win, Section::Headers);
    cx.update_window(win, |_, w, cx| w.click(("kv-delete", ix), cx))
        .unwrap();
    cx.run_until_parked();
    let on_target = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("kv-name", target)).focused()
        })
        .unwrap();
    assert_eq!(on_target, Some(true), "row {target}'s name has focus");
    assert!(view_of(cx, &ws, &root, "h.yml").dirty);
    press(cx, win, &key("s"));
    let saved = view_of(cx, &ws, &root, "h.yml");
    assert!(!saved.dirty, "the save key saved the request");
    assert_eq!(saved.headers.len(), 2);
}

#[gpui_kit::test]
fn deleting_a_header_keeps_the_save_key_working(cx: &mut TestAppContext) {
    header_delete_then_save(cx, 1, 1);
}

#[gpui_kit::test]
fn deleting_the_last_header_keeps_the_save_key_working(cx: &mut TestAppContext) {
    header_delete_then_save(cx, 2, 1);
}

// A section's table in bulk mode: Tab goes section tabs, then the bulk toggle, then the text.
#[gpui_kit::test]
fn tab_from_the_section_tabs_reaches_the_bulk_toggle_then_its_editor(cx: &mut TestAppContext) {
    for section in [Section::Params, Section::Headers] {
        let col = two_requests();
        let (_state, ws, root) = open_dir(cx, &col);
        let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
        open_section(cx, win, section);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click("kv-bulk-toggle", cx)
        })
        .unwrap();
        cx.run_until_parked();
        let table = ed.read_with(cx, |e, _| e.table(section).expect("a table").clone());
        let text = table.read_with(cx, |t, cx| {
            gpui_kit::Focusable::focus_handle(t.bulk_editor().expect("bulk on").read(cx), cx)
        });
        let chosen = section as usize;
        assert_eq!(
            tab_into_sections(cx, win, &ed),
            chosen,
            "{section:?}: Tab enters the tabs on the chosen section"
        );
        press(cx, win, "tab");
        let on_toggle = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.find("kv-bulk-toggle").focused()
            })
            .unwrap();
        assert_eq!(on_toggle, Some(true), "{section:?}: Tab reaches the toggle");
        press(cx, win, "tab");
        assert!(focused(cx, win, &text), "{section:?}: then the bulk text");
        // The editor keeps Shift+Tab for outdent, so its leave chord steps back out.
        press(cx, win, "ctrl-shift-tab");
        press(cx, win, "shift-tab");
        assert_eq!(
            focused_section(cx, win, &ed),
            Some(chosen),
            "{section:?}: and back to the tabs"
        );
        std::mem::forget(col);
    }
}
