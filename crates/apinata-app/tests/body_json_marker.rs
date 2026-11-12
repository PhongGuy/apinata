//! The Body section's live JSON validity marker: a line and column under the editor while the
//! JSON body does not parse, gone once it does, and absent for other modes.

use crate::support;

use apinata_app::body_section::{BodyMode, BodySection, JSON_CHECK_MAX_BYTES};
use apinata_core::BodySpec;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;

fn body(cx: &mut TestAppContext, spec: BodySpec) -> (AnyWindowHandle, Entity<BodySection>) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    settle(cx, win);
    (win, s)
}

fn settle(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn edit(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<BodySection>, text: &str) {
    let code = s.read_with(cx, |s, _| s.code().clone());
    cx.update_window(win, |_, w, cx| {
        code.update(cx, |c, cx| c.replace_all(text.to_string(), w, cx))
    })
    .unwrap();
    settle(cx, win);
}

fn marker(cx: &mut TestAppContext, s: &Entity<BodySection>) -> Option<String> {
    s.read_with(cx, |s, _| s.json_error())
}

fn shown(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, _| w.try_find("body-json-error").is_some())
        .unwrap()
}

#[gpui_kit::test]
fn an_empty_json_body_has_no_marker(cx: &mut TestAppContext) {
    let (win, s) = body(cx, BodySpec::Json(String::new()));
    assert_eq!(marker(cx, &s), None);
    assert!(!shown(cx, win));
}

#[gpui_kit::test]
fn an_invalid_edit_shows_line_and_column_and_a_fix_clears_it(cx: &mut TestAppContext) {
    let (win, s) = body(cx, BodySpec::Json(String::new()));
    edit(cx, win, &s, "{\n\"b\": }");
    assert_eq!(
        marker(cx, &s).as_deref(),
        Some("Line 2, column 6: A value is missing here")
    );
    assert!(shown(cx, win));
    edit(cx, win, &s, "{\n\"b\": 1}");
    assert_eq!(marker(cx, &s), None);
    assert!(!shown(cx, win));
}

#[gpui_kit::test]
fn a_loaded_invalid_body_shows_the_marker(cx: &mut TestAppContext) {
    let (win, s) = body(cx, BodySpec::Json("[1,".into()));
    assert_eq!(
        marker(cx, &s).as_deref(),
        Some("Line 1, column 4: The JSON ends where a value should be")
    );
    assert!(shown(cx, win));
}

#[gpui_kit::test]
fn other_body_types_have_no_marker(cx: &mut TestAppContext) {
    let (win, s) = body(cx, BodySpec::Text("{\"a\": }".into()));
    assert_eq!(marker(cx, &s), None);
    assert!(!shown(cx, win));
    let (win, s) = body(cx, BodySpec::Xml("<a>{".into()));
    assert_eq!(marker(cx, &s), None);
    assert!(!shown(cx, win));
}

#[gpui_kit::test]
fn switching_the_mode_to_and_from_json_adds_and_drops_the_marker(cx: &mut TestAppContext) {
    let (win, s) = body(cx, BodySpec::Json("{\"a\": }".into()));
    assert!(marker(cx, &s).is_some());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(BodyMode::Text, w, cx))
    })
    .unwrap();
    settle(cx, win);
    assert_eq!(marker(cx, &s), None);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(BodyMode::Json, w, cx))
    })
    .unwrap();
    settle(cx, win);
    assert!(marker(cx, &s).is_some());
}

#[gpui_kit::test]
fn a_body_with_placeholders_only_has_no_marker(cx: &mut TestAppContext) {
    let (win, s) = body(
        cx,
        BodySpec::Json("{\"id\": {{id}}, \"s\": \"{{n}}\"}".into()),
    );
    assert_eq!(marker(cx, &s), None);
    assert!(!shown(cx, win));
}

#[gpui_kit::test]
fn a_body_over_the_size_bound_is_not_checked(cx: &mut TestAppContext) {
    assert_eq!(JSON_CHECK_MAX_BYTES, 2 * 1024 * 1024);
    let big = format!("[{}", "1,".repeat(JSON_CHECK_MAX_BYTES / 2));
    let (_, s) = body(cx, BodySpec::Json(big));
    assert_eq!(marker(cx, &s), None);
}
