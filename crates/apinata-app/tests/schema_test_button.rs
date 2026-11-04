//! The response toolbar's "Schema test" button: offered for a JSON body, locked with a
//! reason otherwise, and emitting the unfiltered response text.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use apinata_app::body_view::{BodyView, SchemaTest};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

const JSON: &str = r#"{"user":{"name":"ann"},"n":1}"#;
const TIP: &str =
    "Add a test that checks the response against a schema inferred from the full body";

fn shown(
    cx: &mut TestAppContext,
    response: Response,
    compact: bool,
) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    view.update(cx, |v, cx| v.set_compact(compact, cx));
    let r = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    (win, view)
}

fn json(body: &str) -> Response {
    Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body(body)
}

fn watch(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Rc<RefCell<Vec<String>>> {
    let got: Rc<RefCell<Vec<String>>> = Default::default();
    let sink = got.clone();
    let sub = cx.update(|cx| {
        cx.subscribe(view, move |_, e: &SchemaTest, _| {
            sink.borrow_mut().push(e.0.clone())
        })
    });
    std::mem::forget(sub);
    got
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.click("body-schema-test", cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_json_body_enables_the_button_and_a_click_emits_the_body(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, json(JSON), false);
    assert_eq!(view.read_with(cx, |v, _| v.schema_test_lock()), None);
    assert_eq!(view.read_with(cx, |v, _| v.schema_test_tooltip()), TIP);
    let got = watch(cx, &view);
    click(cx, win);
    assert_eq!(*got.borrow(), [JSON.to_string()]);
}

#[gpui_kit::test]
fn the_button_stays_in_a_compact_pane(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, json(JSON), true);
    let got = watch(cx, &view);
    click(cx, win);
    assert_eq!(got.borrow().len(), 1);
}

#[gpui_kit::test]
fn a_filter_does_not_change_the_emitted_text(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, json(JSON), false);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.n", w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, _| v.applied_filter().map(str::to_owned))
            .as_deref(),
        Some("$.n")
    );
    let got = watch(cx, &view);
    click(cx, win);
    assert_eq!(*got.borrow(), [JSON.to_string()]);
}

#[gpui_kit::test]
fn a_text_body_is_locked_with_a_reason_and_emits_nothing(cx: &mut TestAppContext) {
    let r = Response::new(200)
        .with_header("content-type", "text/plain")
        .with_text_body("hello");
    let (win, view) = shown(cx, r, false);
    let reason = view.read_with(cx, |v, _| v.schema_test_lock());
    assert_eq!(reason, Some("Needs a JSON response"));
    let got = watch(cx, &view);
    click(cx, win);
    assert!(got.borrow().is_empty());
}

#[gpui_kit::test]
fn a_cut_body_is_locked_with_the_size_limit_reason(cx: &mut TestAppContext) {
    let r = json(JSON).with_truncated(9);
    let (win, view) = shown(cx, r, false);
    let reason = view.read_with(cx, |v, _| v.schema_test_lock());
    assert_eq!(reason, Some("The body was cut at the size limit"));
    let got = watch(cx, &view);
    click(cx, win);
    assert!(got.borrow().is_empty());
}
