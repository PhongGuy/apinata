//! The editors' word wrap: a Preferences default, a per-editor toggle, a live default change.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::body_section::{BodyMode, BodySection};
use apinata_app::script_section::ScriptSection;
use apinata_app::settings::Settings;
use apinata_core::BodySpec;
use apinata_core::ScriptPhase::{PostResponse, PreRequest, Tests};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::path::PathBuf;
use support::*;

#[test]
fn wrap_defaults_on_and_round_trips() {
    assert!(Settings::default().wrap_editors);
    let off = Settings {
        wrap_editors: false,
        ..Settings::default()
    };
    let json = serde_json::to_string(&off).unwrap();
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert!(!back.wrap_editors);
}

#[test]
fn an_old_settings_file_without_the_field_wraps() {
    let old: Settings = serde_json::from_str(r#"{"autosave":true}"#).unwrap();
    assert!(old.wrap_editors);
}

#[gpui_kit::test]
fn the_body_toggle_flips_that_editor(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::Json("{}".into()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(BodyMode::Json));
    assert!(s.read_with(cx, |s, _| s.wrap()));
    cx.update_window(win, |_, w, cx| w.click("body-wrap-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert!(!s.read_with(cx, |s, _| s.wrap()));
}

#[gpui_kit::test]
fn each_script_phase_toggles_on_its_own(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("script-wrap-toggle", cx))
        .unwrap();
    frame(cx, win);
    s.read_with(cx, |s, _| {
        assert!(!s.wrap(PreRequest) && s.wrap(PostResponse));
    });
}

#[gpui_kit::test]
fn the_tests_editor_has_a_toggle(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| ScriptSection::new(&[Tests], w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("script-wrap-toggle", cx))
        .unwrap();
    assert!(!s.read_with(cx, |s, _| s.wrap(Tests)));
}

#[gpui_kit::test]
fn changing_the_default_reaches_open_editors(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let st = started(cx, &col);
    let (win, ed) = editor_for(cx, &st.ws, &st.root, "a.yml", false);
    frame(cx, win);
    let (body, script) = ed.read_with(cx, |e, _| (e.body().clone(), e.script().clone()));
    assert!(body.read_with(cx, |b, _| b.wrap()));
    let settings = cx.update(|cx| cx.global::<AppState>().settings.clone());
    settings.update(cx, |p, cx| p.update(cx, |s| s.wrap_editors = false));
    frame(cx, win);
    assert!(!body.read_with(cx, |b, _| b.wrap()));
    assert!(!script.read_with(cx, |s, _| s.wrap(PreRequest)));
}
