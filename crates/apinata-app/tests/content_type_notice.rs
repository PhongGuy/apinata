//! A hand-set Content-Type the body mode cannot be sent under is named above the Headers and
//! the Body, with one click to take the body's type or drop the header.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::workspace::Workspace;
use apinata_core::{BodySpec, KeyValue};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;

const NOTICE: &str = "content-type-conflict";

const MULTIPART: &str = "\n    type: multipart-form\n    data:\n      - name: f\n        type: text\n        value: v\n";
const JSON: &str = "\n    type: json\n    data: \"{}\"\n";

/// A POST with one Content-Type header `value` and the body YAML `body`, written after `body:`.
fn file(value: &str, body: &str) -> String {
    format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0\"\n  headers:\n    - name: Content-Type\n      value: \"{value}\"\n    - name: X-A\n      value: \"1\"\n  body:{body}"
    )
}

struct Opened {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: PathBuf,
}

// The app over one file `a.yml`, its editor showing `section`.
fn opened(
    cx: &mut TestAppContext,
    text: &str,
    section: Section,
) -> (Opened, AnyWindowHandle, Entity<RequestEditor>) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", text)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    view.update(cx, |v, cx| v.show(section, cx));
    cx.run_until_parked();
    let s = Opened {
        _col: col,
        _state: state,
        ws,
        root,
    };
    (s, win, view)
}

fn notice(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(NOTICE)
            .map(|n| n.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

fn headers(cx: &mut TestAppContext, s: &Opened) -> Vec<(String, String)> {
    view_of(cx, &s.ws, &s.root, "a.yml")
        .headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect()
}

const MULTIPART_TEXT: &str = "The Content-Type header here (application/json) is sent instead of the body's multipart/form-data.";

#[gpui_kit::test]
fn a_multipart_body_under_a_json_header_is_named_above_the_headers(cx: &mut TestAppContext) {
    let (_s, win, _) = opened(cx, &file("application/json", MULTIPART), Section::Headers);
    assert_eq!(notice(cx, win).as_deref(), Some(MULTIPART_TEXT));
    assert!(present(cx, win, "content-type-use-body"));
    assert!(present(cx, win, "content-type-remove"));
}

#[gpui_kit::test]
fn the_body_section_names_it_too(cx: &mut TestAppContext) {
    let (_s, win, _) = opened(cx, &file("application/json", MULTIPART), Section::Body);
    assert_eq!(notice(cx, win).as_deref(), Some(MULTIPART_TEXT));
}

#[gpui_kit::test]
fn an_agreeing_header_shows_no_notice(cx: &mut TestAppContext) {
    let (_s, win, _) = opened(
        cx,
        &file("application/json; charset=utf-8", JSON),
        Section::Headers,
    );
    assert_eq!(notice(cx, win), None);
}

#[gpui_kit::test]
fn switching_the_body_mode_under_a_header_raises_the_notice(cx: &mut TestAppContext) {
    let (s, win, _) = opened(cx, &file("application/json", JSON), Section::Body);
    assert_eq!(notice(cx, win), None);
    let form = BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1")]);
    let command = Command::Edit {
        target: Target::Request(ip("a.yml")),
        edit: Edit::Body(form),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, command, cx));
    assert_eq!(
        notice(cx, win).as_deref(),
        Some(
            "The Content-Type header here (application/json) is sent instead of the body's application/x-www-form-urlencoded."
        )
    );
}

#[gpui_kit::test]
fn use_body_type_rewrites_the_header(cx: &mut TestAppContext) {
    let (s, win, _) = opened(cx, &file("application/json", MULTIPART), Section::Headers);
    click(cx, win, "content-type-use-body");
    assert_eq!(
        headers(cx, &s),
        [
            ("Content-Type".to_owned(), "multipart/form-data".to_owned()),
            ("X-A".to_owned(), "1".to_owned())
        ]
    );
    assert!(view_of(cx, &s.ws, &s.root, "a.yml").dirty);
    assert_eq!(notice(cx, win), None);
}

#[gpui_kit::test]
fn remove_header_drops_only_the_content_type(cx: &mut TestAppContext) {
    let (s, win, view) = opened(cx, &file("application/json", MULTIPART), Section::Headers);
    click(cx, win, "content-type-remove");
    assert_eq!(headers(cx, &s), [("X-A".to_owned(), "1".to_owned())]);
    assert_eq!(notice(cx, win), None);
    // The fix is one undo step.
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(headers(cx, &s).len(), 2);
}
