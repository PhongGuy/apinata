//! The Vim key map in the code editors: the Preferences choice, Normal and Insert modes, and
//! keys that must still reach the editor or the app.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::body_section::BodySection;
use apinata_app::settings::{EditorKeymap, Settings};
use apinata_app::vim::VimMode;
use apinata_core::BodySpec;
use apinata_core::ScriptPhase::PreRequest;
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext};
use std::path::PathBuf;
use support::*;

#[test]
fn the_key_map_defaults_to_default_and_round_trips() {
    assert_eq!(Settings::default().editor_keymap, EditorKeymap::Default);
    let vim = Settings {
        editor_keymap: EditorKeymap::Vim,
        ..Settings::default()
    };
    let json = serde_json::to_string(&vim).unwrap();
    assert!(json.contains(r#""editor_keymap":"vim""#), "{json}");
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.editor_keymap, EditorKeymap::Vim);
}

#[test]
fn an_unknown_key_map_reads_as_default_and_keeps_other_settings() {
    let s: Settings = serde_json::from_str(r#"{"editor_keymap":"emacs","autosave":true}"#).unwrap();
    assert_eq!(s.editor_keymap, EditorKeymap::Default);
    assert!(s.autosave);
}

struct Body {
    win: gpui_kit::AnyWindowHandle,
    host: Entity<SectionHost<BodySection>>,
    body: Entity<BodySection>,
}

// A body section holding `text`, with the Vim key map on or off and its editor focused.
fn body(cx: &mut TestAppContext, text: &str, vim: bool) -> Body {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    cx.update(apinata_app::keymap::init);
    let body = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        body.update(cx, |s, cx| s.load(&BodySpec::Text(text.into()), w, cx));
        let vim_state = body.read(cx).vim().clone();
        vim_state.update(cx, |v, cx| v.set_enabled(vim, cx));
        w.activate_window();
        let code = body.read(cx).code().clone();
        code.read(cx).focus_handle(cx).focus(w, cx);
        code.update(cx, |c, cx| c.set_selected_range(0..0, cx));
    })
    .unwrap();
    frame(cx, win);
    Body { win, host, body }
}

fn keys(cx: &mut TestAppContext, b: &Body, keys: &str) {
    cx.simulate_keystrokes(b.win, keys);
    frame(cx, b.win);
}

fn text(cx: &mut TestAppContext, b: &Body) -> String {
    b.body
        .read_with(cx, |s, cx| s.code().read(cx).value().to_string())
}

fn cursor(cx: &mut TestAppContext, b: &Body) -> usize {
    b.body.read_with(cx, |s, cx| s.code().read(cx).cursor())
}

fn mode(cx: &mut TestAppContext, b: &Body) -> Option<VimMode> {
    b.body.read_with(cx, |s, cx| s.vim().read(cx).mode())
}

#[gpui_kit::test]
fn normal_mode_moves_and_deletes_without_typing(cx: &mut TestAppContext) {
    let b = body(cx, "hello\nworld", true);
    assert_eq!(mode(cx, &b), Some(VimMode::Normal));
    keys(cx, &b, "l l j");
    assert_eq!(cursor(cx, &b), 8);
    keys(cx, &b, "x");
    assert_eq!(text(cx, &b), "hello\nwold");
    keys(cx, &b, "z q enter backspace tab");
    assert_eq!(text(cx, &b), "hello\nwold");
    keys(cx, &b, "g g d d");
    assert_eq!(text(cx, &b), "wold");
}

#[gpui_kit::test]
fn a_vim_edit_reaches_the_section_as_an_edit(cx: &mut TestAppContext) {
    let b = body(cx, "abc", true);
    keys(cx, &b, "x");
    let edits = b.host.read_with(cx, |h, _| h.edits.len());
    assert!(edits > 0, "the section saw no edit");
}

#[gpui_kit::test]
fn insert_mode_types_and_escape_returns_to_normal(cx: &mut TestAppContext) {
    let b = body(cx, "ab", true);
    keys(cx, &b, "A");
    assert_eq!(mode(cx, &b), Some(VimMode::Insert));
    keys(cx, &b, "x y");
    assert_eq!(text(cx, &b), "abxy");
    keys(cx, &b, "escape");
    assert_eq!(mode(cx, &b), Some(VimMode::Normal));
    assert_eq!(cursor(cx, &b), 3);
    keys(cx, &b, "x");
    assert_eq!(text(cx, &b), "abx");
}

#[gpui_kit::test]
fn modified_keys_still_reach_the_editor_in_normal_mode(cx: &mut TestAppContext) {
    let b = body(cx, "abc", true);
    keys(cx, &b, "ctrl-a");
    let selected = b
        .body
        .read_with(cx, |s, cx| s.code().read(cx).selected_range());
    assert_eq!(selected, 0..3);
}

#[gpui_kit::test]
fn u_undoes_the_last_vim_edit(cx: &mut TestAppContext) {
    let b = body(cx, "abc", true);
    keys(cx, &b, "x");
    assert_eq!(text(cx, &b), "bc");
    keys(cx, &b, "u");
    assert_eq!(text(cx, &b), "abc");
}

#[gpui_kit::test]
fn the_default_key_map_types_every_key(cx: &mut TestAppContext) {
    let b = body(cx, "", false);
    assert_eq!(mode(cx, &b), None);
    keys(cx, &b, "x j");
    assert_eq!(text(cx, &b), "xj");
}

#[gpui_kit::test]
fn turning_vim_off_leaves_the_editor_typing(cx: &mut TestAppContext) {
    let b = body(cx, "", true);
    cx.update(|cx| {
        let vim = b.body.read(cx).vim().clone();
        vim.update(cx, |v, cx| v.set_enabled(false, cx));
    });
    keys(cx, &b, "x");
    assert_eq!(text(cx, &b), "x");
}

#[gpui_kit::test]
fn the_preference_reaches_open_editors(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let st = started(cx, &col);
    let (win, ed) = editor_for(cx, &st.ws, &st.root, "a.yml", false);
    frame(cx, win);
    let (body, script, docs) = ed.read_with(cx, |e, _| {
        (e.body().clone(), e.script().clone(), e.docs().clone())
    });
    let modes = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            [
                body.read(cx).vim().read(cx).mode(),
                script
                    .read(cx)
                    .vim(PreRequest)
                    .and_then(|v| v.read(cx).mode()),
                docs.read(cx).vim().read(cx).mode(),
            ]
        })
    };
    assert_eq!(modes(cx), [None; 3]);
    let settings = cx.update(|cx| cx.global::<AppState>().settings.clone());
    settings.update(cx, |p, cx| {
        p.update(cx, |s| s.editor_keymap = EditorKeymap::Vim)
    });
    frame(cx, win);
    assert_eq!(modes(cx), [Some(VimMode::Normal); 3]);
}
