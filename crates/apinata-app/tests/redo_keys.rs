//! Redo is bound to Ctrl+Shift+Z (Cmd+Shift+Z on macOS) in every code editor.

use crate::support;

use apinata_app::body_section::BodySection;
use apinata_app::script_section::ScriptSection;
use apinata_app::snapshot::ScriptCode;
use apinata_core::BodySpec;
use apinata_core::ScriptPhase::PreRequest;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::path::PathBuf;
use support::*;

fn redo_chord() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-shift-z"
    }
}

#[gpui_kit::test]
fn ctrl_shift_z_redoes_in_the_body_editor(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    // Section hosts carry only the kit's keys; the app's own keymap adds Ctrl+Shift+Z.
    cx.update(apinata_app::keymap::init);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::Json(String::new()), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    replace_code(cx, win, None, "body-code", "abc");
    let code = s.read_with(cx, |s, _| s.code().clone());
    let value = |cx: &mut TestAppContext| code.read_with(cx, |c, _| c.value().to_string());
    assert_eq!(value(cx), "abc");
    cx.update_window(win, |_, w, cx| w.press(&key("z"), cx))
        .unwrap();
    assert_ne!(value(cx), "abc");
    cx.update_window(win, |_, w, cx| w.press(redo_chord(), cx))
        .unwrap();
    assert_eq!(value(cx), "abc");
}

#[gpui_kit::test]
fn ctrl_shift_z_redoes_in_the_script_editor(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| ScriptSection::new(&[PreRequest], w, cx));
    // Section hosts carry only the kit's keys; the app's own keymap adds Ctrl+Shift+Z.
    cx.update(apinata_app::keymap::init);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&ScriptCode::default(), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    replace_code(cx, win, None, "script-pre-request-code", "abc");
    let value = |cx: &mut TestAppContext| s.read_with(cx, |s, cx| s.texts(cx)[0].clone());
    assert_eq!(value(cx), "abc");
    cx.update_window(win, |_, w, cx| w.press(&key("z"), cx))
        .unwrap();
    assert_ne!(value(cx), "abc");
    cx.update_window(win, |_, w, cx| w.press(redo_chord(), cx))
        .unwrap();
    assert_eq!(value(cx), "abc");
}
