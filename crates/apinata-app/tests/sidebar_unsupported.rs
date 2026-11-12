//! WebSocket and gRPC request files: their rows carry a kind tag and a tooltip, and opening one
//! says it cannot be sent yet instead of doing nothing. The file itself is never touched.

use crate::support;

use apinata_app::sidebar::marker_id;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

const GRPC: &str = "info:\n  name: Greeter\n  type: grpc\n  seq: 3\n";
// A graphql file core cannot read stays an opaque row, but GraphQL is sendable.
const BROKEN_GRAPHQL: &str = "info:\n  name: Query\n  type: graphql\n  seq: 4\n";
const WS_NOTE: &str = "WebSocket requests cannot be sent yet. The file is kept as it is.";
const GRPC_NOTE: &str = "gRPC requests cannot be sent yet. The file is kept as it is.";

fn harness(cx: &mut TestAppContext) -> SidebarHarness {
    let mut files = sidebar_files();
    files.push(("rpc.yml", GRPC.to_string()));
    files.push(("query.yml", BROKEN_GRAPHQL.to_string()));
    open_sidebar_with(cx, files)
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: gpui_kit::SharedString) -> String {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(id.clone())
            .and_then(|e| e.label().map(str::to_owned))
            .unwrap_or_else(|| panic!("{id} is not shown"))
    })
    .unwrap()
}

// The tooltip text shown after resting the pointer on the row at `path`, if any.
fn tooltip(cx: &mut TestAppContext, win: AnyWindowHandle, path: &str) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.hover(row(path), cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(marker_id("unsupported-tip", &row(path)))
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

// The text of the "cannot be sent yet" toast showing, if any.
fn note(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find("unsupported-note")
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn websocket_and_grpc_rows_show_a_kind_tag(cx: &mut TestAppContext) {
    let h = harness(cx);
    assert_eq!(label(cx, h.win, marker_id("method", &row("ws.yml"))), "WS");
    assert_eq!(
        label(cx, h.win, marker_id("method", &row("rpc.yml"))),
        "gRPC"
    );
    assert!(
        !shown(cx, h.win, marker_id("method", &row("query.yml"))),
        "a graphql file gets no unsupported tag"
    );
}

#[gpui_kit::test]
fn hovering_a_websocket_or_grpc_row_says_it_cannot_be_sent_yet(cx: &mut TestAppContext) {
    let h = harness(cx);
    assert_eq!(tooltip(cx, h.win, "ws.yml").as_deref(), Some(WS_NOTE));
    assert_eq!(tooltip(cx, h.win, "rpc.yml").as_deref(), Some(GRPC_NOTE));
    assert_eq!(tooltip(cx, h.win, "query.yml"), None);
}

#[gpui_kit::test]
fn clicking_a_websocket_row_says_it_cannot_be_sent_and_keeps_the_file(cx: &mut TestAppContext) {
    let h = harness(cx);
    let events = sidebar_events(cx, &h.view);
    let before = std::fs::read(h.root.join("ws.yml")).unwrap();
    assert_eq!(note(cx, h.win), None);
    cx.update_window(h.win, |_, w, cx| w.click(row("ws.yml"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(note(cx, h.win).as_deref(), Some(WS_NOTE));
    assert!(events.borrow().is_empty(), "no tab opens");
    assert_eq!(std::fs::read(h.root.join("ws.yml")).unwrap(), before);
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("ws.yml"))),
        "the row is selected"
    );
}

#[gpui_kit::test]
fn enter_on_a_grpc_row_says_it_cannot_be_sent(cx: &mut TestAppContext) {
    let h = harness(cx);
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&row("rpc.yml"));
        t.set_selected_index(ix, cx)
    });
    focus_tree(cx, &h);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(note(cx, h.win).as_deref(), Some(GRPC_NOTE));
    assert_eq!(
        std::fs::read_to_string(h.root.join("rpc.yml")).unwrap(),
        GRPC
    );
}

#[gpui_kit::test]
fn clicking_a_broken_graphql_row_shows_no_unsupported_note(cx: &mut TestAppContext) {
    let h = harness(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("query.yml"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(note(cx, h.win), None);
}
