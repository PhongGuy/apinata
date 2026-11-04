//! "Assert this value" in the response body's node menu: it appends an enabled assertion
//! row to the request, once, and leaves the tab dirty with the new row focused.

use crate::support;

use apinata_app::body_node::{ASSERT_VALUE, NodeCopy, node_entries};
use apinata_app::request_editor::Section;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

const A: &str =
    "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n";
const JSON: &str = r#"{"user":{"name":"ann","tags":["a"]},"n":1.50,"z":null}"#;

fn entry(needle: &str) -> Option<NodeCopy> {
    node_entries(JSON, JSON.find(needle).unwrap(), true)
        .into_iter()
        .find(|e| e.label == ASSERT_VALUE)
}

#[test]
fn scalars_offer_the_entry_and_containers_do_not() {
    for needle in ["ann", "1.50", "null"] {
        assert!(entry(needle).is_some(), "{needle}");
    }
    assert!(entry("\"tags\"").is_none());
    assert!(entry("{\"name").is_none());
    let off = node_entries(JSON, JSON.find("ann").unwrap(), false);
    assert!(off.iter().all(|e| e.label != ASSERT_VALUE));
}

#[test]
fn the_entry_carries_the_matching_assertion() {
    let a = entry("ann").unwrap().assertion.unwrap();
    assert_eq!(
        (a.expression(), a.operator(), a.value()),
        ("res.body.user.name", "eq", Some("ann"))
    );
    let a = entry("null").unwrap().assertion.unwrap();
    assert_eq!((a.expression(), a.operator()), ("res.body.z", "isNull"));
}

#[gpui_kit::test]
fn choosing_it_adds_one_focused_row_and_a_repeat_adds_none(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    let entry = entry("ann").unwrap();
    for _ in 0..2 {
        cx.update_window(win, |_, w, cx| {
            body.update(cx, |b, cx| b.activate_node(&entry, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }
    let view = view_of(cx, &ws, &root, "a.yml");
    let rows: Vec<_> = view
        .assertions
        .iter()
        .map(|a| (a.expression(), a.operator(), a.value(), a.enabled()))
        .collect();
    assert_eq!(rows, [("res.body.user.name", "eq", Some("ann"), true)]);
    assert!(view.dirty);
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Assertions);
    let input = ed.read_with(cx, |e, cx| {
        e.assertions()
            .read(cx)
            .expression_input(0)
            .cloned()
            .unwrap()
    });
    let focused = cx
        .update_window(win, |_, w, cx| input.focus_handle(cx).is_focused(w))
        .unwrap();
    assert!(focused, "the new row's expression holds focus");
}
