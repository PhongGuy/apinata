//! The Body section's Beautify button and command: a whitespace-only re-layout of a JSON or
//! XML body that undoes like any edit and reports a parse error without touching the text.

use crate::support;

use apinata_app::body_section::{BodyMode, BodySection, JSON_CHECK_MAX_BYTES};
use apinata_app::command::Edit;
use apinata_core::BodySpec;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;

type Host = Entity<SectionHost<BodySection>>;

fn body(cx: &mut TestAppContext, spec: BodySpec) -> (AnyWindowHandle, Host, Entity<BodySection>) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host, s)
}

fn text(cx: &mut TestAppContext, s: &Entity<BodySection>) -> String {
    let code = s.read_with(cx, |s, _| s.code().clone());
    code.read_with(cx, |c, _| c.value().to_string())
}

fn beautify(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<BodySection>) {
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.beautify(w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn last_edit(cx: &mut TestAppContext, host: &Host) -> Option<BodySpec> {
    host.read_with(cx, |h, _| match h.edits.last() {
        Some(Edit::Body(b)) => Some(b.clone()),
        _ => None,
    })
}

fn shown(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

const PRETTY: &str = "{\n  \"b\": 1.50,\n  \"a\": [\n    {{x}},\n    1\n  ]\n}";

#[gpui_kit::test]
fn the_button_beautifies_a_json_body_and_edits_the_draft(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Json("{\"b\":1.50,\"a\":[{{x}},1]}".into()));
    assert!(shown(cx, win, "body-beautify"));
    cx.update_window(win, |_, w, cx| w.click("body-beautify", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &s), PRETTY);
    assert_eq!(last_edit(cx, &host), Some(BodySpec::Json(PRETTY.into())));
    assert!(!shown(cx, win, "body-beautify-error"));
}

#[gpui_kit::test]
fn an_xml_body_is_beautified_too(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Xml("<r a=\"{{id}}\"><t>{{n}}</t></r>".into()));
    beautify(cx, win, &s);
    let want = "<r a=\"{{id}}\">\n  <t>{{n}}</t>\n</r>";
    assert_eq!(text(cx, &s), want);
    assert_eq!(last_edit(cx, &host), Some(BodySpec::Xml(want.into())));
}

#[gpui_kit::test]
fn invalid_json_keeps_the_text_and_shows_only_the_live_marker(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Json("{\"a\": }".into()));
    assert!(shown(cx, win, "body-json-error"));
    beautify(cx, win, &s);
    assert_eq!(text(cx, &s), "{\"a\": }");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    assert!(shown(cx, win, "body-json-error"));
    assert!(!shown(cx, win, "body-beautify-error"));
    assert!(s.read_with(cx, |s, _| s.beautify_error().is_none()));
}

#[gpui_kit::test]
fn invalid_xml_still_shows_the_beautify_error_until_the_next_edit(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Xml("<a><b></a>".into()));
    beautify(cx, win, &s);
    assert_eq!(text(cx, &s), "<a><b></a>");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    assert!(shown(cx, win, "body-beautify-error"));
    replace_code(cx, win, None, "body-code", "<a/>");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(!shown(cx, win, "body-beautify-error"));
}

#[gpui_kit::test]
fn oversize_invalid_json_still_shows_the_beautify_error(cx: &mut TestAppContext) {
    let big = format!("{{\"a\": \"{}\" }}}}", "x".repeat(JSON_CHECK_MAX_BYTES));
    let (win, _host, s) = body(cx, BodySpec::Json(big));
    assert!(!shown(cx, win, "body-json-error"));
    beautify(cx, win, &s);
    assert!(shown(cx, win, "body-beautify-error"));
}

#[gpui_kit::test]
fn undo_restores_the_text_before_beautify(cx: &mut TestAppContext) {
    let original = "{\"a\":[1,2]}";
    let (win, host, s) = body(cx, BodySpec::Json(original.into()));
    beautify(cx, win, &s);
    assert_ne!(text(cx, &s), original);
    cx.update_window(win, |_, w, cx| {
        w.click_at("body-code", code_at(), cx);
        w.press(&key("z"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &s), original);
    assert_eq!(last_edit(cx, &host), Some(BodySpec::Json(original.into())));
}

#[gpui_kit::test]
fn the_button_is_for_json_and_xml_only(cx: &mut TestAppContext) {
    let (win, _host, s) = body(cx, BodySpec::Text("{\"a\":1}".into()));
    assert!(!shown(cx, win, "body-beautify"));
    beautify(cx, win, &s);
    assert_eq!(text(cx, &s), "{\"a\":1}");
    assert!(!shown(cx, win, "body-beautify-error"));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(BodyMode::Json, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(shown(cx, win, "body-beautify"));
}

#[gpui_kit::test]
fn beautify_on_an_empty_or_blank_body_is_a_no_op(cx: &mut TestAppContext) {
    for spec in [
        BodySpec::Json(String::new()),
        BodySpec::Json("  \n\t ".into()),
        BodySpec::Xml(String::new()),
        BodySpec::Xml(" \n ".into()),
    ] {
        let (win, host, s) = body(cx, spec);
        let before = text(cx, &s);
        beautify(cx, win, &s);
        assert_eq!(text(cx, &s), before);
        assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
        assert!(s.read_with(cx, |s, _| s.beautify_error().is_none()));
        assert!(!shown(cx, win, "body-beautify-error"));
    }
}

fn minify(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<BodySection>) {
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.minify(w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn the_minify_button_strips_whitespace_and_edits_the_draft(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Json(PRETTY.into()));
    assert!(shown(cx, win, "body-minify"));
    cx.update_window(win, |_, w, cx| w.click("body-minify", cx))
        .unwrap();
    cx.run_until_parked();
    let want = "{\"b\":1.50,\"a\":[{{x}},1]}";
    assert_eq!(text(cx, &s), want);
    assert_eq!(last_edit(cx, &host), Some(BodySpec::Json(want.into())));
}

#[gpui_kit::test]
fn undo_restores_the_text_before_minify_in_one_step(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Json(PRETTY.into()));
    minify(cx, win, &s);
    assert_ne!(text(cx, &s), PRETTY);
    cx.update_window(win, |_, w, cx| {
        w.click_at("body-code", code_at(), cx);
        w.press(&key("z"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &s), PRETTY);
    assert_eq!(last_edit(cx, &host), Some(BodySpec::Json(PRETTY.into())));
}

#[gpui_kit::test]
fn minify_on_invalid_json_shows_only_the_live_marker(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Json("{\"a\": }".into()));
    minify(cx, win, &s);
    assert_eq!(text(cx, &s), "{\"a\": }");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    assert!(shown(cx, win, "body-json-error"));
    assert!(!shown(cx, win, "body-beautify-error"));
}

#[gpui_kit::test]
fn minify_on_invalid_xml_shows_the_error_and_keeps_the_text(cx: &mut TestAppContext) {
    let (win, _host, s) = body(cx, BodySpec::Xml("<a><b></a>".into()));
    minify(cx, win, &s);
    assert_eq!(text(cx, &s), "<a><b></a>");
    assert!(shown(cx, win, "body-beautify-error"));
}

#[gpui_kit::test]
fn minify_on_an_empty_body_is_a_no_op_and_text_mode_has_no_button(cx: &mut TestAppContext) {
    let (win, host, s) = body(cx, BodySpec::Json("  \n ".into()));
    minify(cx, win, &s);
    assert_eq!(text(cx, &s), "  \n ");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    let (win, _host, _s) = body(cx, BodySpec::Text("{ }".into()));
    assert!(!shown(cx, win, "body-minify"));
}
