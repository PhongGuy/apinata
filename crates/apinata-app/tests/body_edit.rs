//! The response body is read-only until Edit is chosen; scratch edits feed copy and save.

use crate::support;

use apinata_app::body_view::BodyView;
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Focusable, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            cx.new(|cx| BodyView::new(w, cx))
        })
    })
    .unwrap()
}

fn show(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, s: &Sent) {
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
}

fn value(cx: &mut TestAppContext, ed: &Entity<EditorState>) -> String {
    ed.read_with(cx, |e, _| e.value().to_string())
}

fn type_into(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &Entity<EditorState>, text: &str) {
    cx.update_window(win, |_, w, cx| {
        ed.read(cx).focus_handle(cx).focus(w, cx);
        w.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn json() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json")
}

#[gpui_kit::test]
fn the_body_is_read_only_until_edit_is_chosen(cx: &mut TestAppContext) {
    let s = send_once(cx, json(), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    let shown = value(cx, &ed);
    assert!(!view.read_with(cx, |v, _| v.editing()));
    assert!(present(cx, win, "body-view") && present(cx, win, "body-edit"));
    type_into(cx, win, &ed, "x");
    assert_eq!(value(cx, &ed), shown, "View ignores typing");
    click(cx, win, "body-edit");
    assert!(view.read_with(cx, |v, _| v.editing()));
    type_into(cx, win, &ed, "x");
    assert!(value(cx, &ed).contains('x'), "Edit takes typing");
    click(cx, win, "body-view");
    assert!(!view.read_with(cx, |v, _| v.editing()));
}

#[gpui_kit::test]
fn copy_and_save_take_the_edited_text(cx: &mut TestAppContext) {
    let s = send_once(cx, json(), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    click(cx, win, "body-edit");
    type_into(cx, win, &ed, "x");
    let edited = value(cx, &ed);
    click(cx, win, "body-copy");
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some(edited.as_str()));
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("body.json");
    click(cx, win, "body-save");
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), edited);
}

#[gpui_kit::test]
fn unedited_save_stays_as_received(cx: &mut TestAppContext) {
    let s = send_once(cx, json(), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    click(cx, win, "body-edit");
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("body.json");
    click(cx, win, "body-save");
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"a\":1}");
}

#[gpui_kit::test]
fn a_new_response_replaces_the_scratch_edits(cx: &mut TestAppContext) {
    let s = send_once(cx, json(), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    let shown = value(cx, &ed);
    click(cx, win, "body-edit");
    type_into(cx, win, &ed, "x");
    show(cx, win, &view, &s);
    assert_eq!(value(cx, &ed), shown);
    assert!(
        view.read_with(cx, |v, _| v.editing()),
        "the switch keeps its choice"
    );
    click(cx, win, "body-copy");
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("{\n  \"a\": 1\n}"));
}

#[gpui_kit::test]
fn a_long_body_offers_no_edit(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_string(vec!["line"; 30_000].join("\n")),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert!(!present(cx, win, "body-edit"));
}

#[gpui_kit::test]
fn switching_pretty_and_raw_asks_before_dropping_edits(cx: &mut TestAppContext) {
    use apinata_app::body_view::BodyMode;
    let s = send_once(cx, json(), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    click(cx, win, "body-raw");
    assert!(!confirm_open(cx), "no edits, no question");
    assert_eq!(view.read_with(cx, |v, _| v.mode()), BodyMode::Raw);
    click(cx, win, "body-pretty");
    click(cx, win, "body-edit");
    type_into(cx, win, &ed, "x");
    let edited = value(cx, &ed);
    click(cx, win, "body-pretty");
    assert!(!confirm_open(cx), "the mode already shown never asks");
    click(cx, win, "body-raw");
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(view.read_with(cx, |v, _| v.mode()), BodyMode::Pretty);
    assert_eq!(value(cx, &ed), edited, "Cancel keeps the edits");
    click(cx, win, "body-raw");
    answer_confirm(cx, "confirm-discard");
    assert_eq!(view.read_with(cx, |v, _| v.mode()), BodyMode::Raw);
    assert_eq!(value(cx, &ed), "{\"a\":1}");
}
