//! Small body details: the copy label under a filter, the invalid-JSON note, the save
//! suggestion from Content-Disposition, and the toast after opening a page in the browser.

use crate::support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use apinata_app::body_view::{BodyView, OPENED_PAGE_NOTE, opened_note};
use apinata_app::open_external::{self, FileOpener};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

fn result(headers: &[(&str, &str)], body: &str, truncated: bool) -> Arc<RequestResult> {
    let mut r = Response::new(200);
    for (name, value) in headers {
        r = r.with_header(*name, *value);
    }
    let mut r = r.with_text_body(body);
    if truncated {
        let n = r.body().len() as u64;
        r = r.with_truncated(n);
    }
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "Get file",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ))
}

fn host(cx: &mut TestAppContext, r: Arc<RequestResult>) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_copy_button_names_the_result_while_a_filter_is_set(cx: &mut TestAppContext) {
    let (win, view) = host(
        cx,
        result(
            &[("content-type", "application/json")],
            r#"{"a":[1]}"#,
            false,
        ),
    );
    assert_eq!(label(cx, win, "body-copy-button"), "Copy body");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.a", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(label(cx, win, "body-copy-button"), "Copy result");
}

const BROKEN: &str = "{\n  \"a\": 1\n  \"b\": 2\n}";
const BROKEN_NOTE: &str = "Not valid JSON: expected `,` or `}` at line 3, column 3";

#[gpui_kit::test]
fn an_unparsable_json_body_says_where_it_breaks(cx: &mut TestAppContext) {
    let (win, view) = host(
        cx,
        result(&[("content-type", "application/json")], BROKEN, false),
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.json_problem().map(str::to_owned))
            .as_deref(),
        Some(BROKEN_NOTE)
    );
    assert_eq!(label(cx, win, "body-json-invalid"), BROKEN_NOTE);
}

#[gpui_kit::test]
fn a_valid_or_cut_json_body_has_no_note(cx: &mut TestAppContext) {
    let (_win, view) = host(
        cx,
        result(&[("content-type", "application/json")], r#"{"a":1}"#, false),
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.json_problem().map(str::to_owned)),
        None
    );
    let (_win, view) = host(
        cx,
        result(&[("content-type", "application/json")], BROKEN, true),
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.json_problem().map(str::to_owned)),
        None
    );
}

#[gpui_kit::test]
fn the_full_download_suggests_the_disposition_name(cx: &mut TestAppContext) {
    let (_win, view) = host(
        cx,
        result(
            &[
                ("content-type", "application/octet-stream"),
                ("content-disposition", "attachment; filename=\"blob.bin\""),
            ],
            "abc",
            true,
        ),
    );
    assert_eq!(view.read_with(cx, |v, _| v.save_suggestion()), "blob.bin");
    let (_win, view) = host(
        cx,
        result(&[("content-type", "application/pdf")], "abc", true),
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.save_suggestion()),
        "Get file.pdf"
    );
}

#[gpui_kit::test]
fn opening_a_page_in_the_browser_confirms_with_a_toast(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let log = seen.clone();
    let opener: FileOpener = Arc::new(move |path, _| log.lock().unwrap().push(path.to_path_buf()));
    let root = base.path().to_path_buf();
    cx.update(|cx| open_external::install(opener, root, cx));
    cx.run_until_parked();
    let (win, _view) = host(
        cx,
        result(&[("content-type", "text/html")], "<p>hi</p>", false),
    );
    let count = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let before = count(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-open", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].starts_with(base.path()), "{seen:?}");
    assert_eq!(count(cx), before + 1);
}

#[test]
fn only_a_page_open_names_what_the_browser_blocks() {
    assert_eq!(
        opened_note(true),
        Some("Opened in your browser (scripts and remote content blocked)")
    );
    assert_eq!(opened_note(true), Some(OPENED_PAGE_NOTE));
    assert_eq!(opened_note(false), None);
}

#[gpui_kit::test]
fn opening_a_binary_body_shows_no_toast(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let log = seen.clone();
    let opener: FileOpener = Arc::new(move |path, _| log.lock().unwrap().push(path.to_path_buf()));
    let root = base.path().to_path_buf();
    cx.update(|cx| open_external::install(opener, root, cx));
    cx.run_until_parked();
    let (win, _view) = host(
        cx,
        result(&[("content-type", "application/pdf")], "%PDF-1.4", false),
    );
    let count = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let before = count(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-open", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(seen.lock().unwrap()[0].starts_with(base.path()));
    assert_eq!(count(cx), before);
}
