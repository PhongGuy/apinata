//! An open comparison survives the history refresh a save brings, scroll offset included.

use crate::support;

use apinata_app::compare_source::CompareSource;
use apinata_app::last_response::{ResponseStore, install};
use apinata_app::response_pane::{ResponsePane, refresh_all_history};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, ScrollDelta, TestAppContext, VisualTestContext, point, px, size,
};
use support::*;
use wiremock::ResponseTemplate;

const BODY: &str = "{\"items\":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30]}";

#[gpui_kit::test]
fn saving_the_request_keeps_a_comparison_with_the_previous_send(cx: &mut TestAppContext) {
    http_ready(cx);
    // Two paths so the two sends differ in every array element and the row list is long.
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string(BODY));
    let old_body = BODY.replace(|c: char| c.is_ascii_digit(), "0");
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/old"))
            .respond_with(ResponseTemplate::new(200).set_body_string(old_body))
            .with_priority(1)
            .mount(&server),
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", &format!("{}/old", server.uri()))),
    ]);
    let s = started(cx, &col);
    // A short window, so the long row list overflows the pane and can scroll.
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(500.)));
    frame(cx, s.win);
    let store = tempfile::tempdir().unwrap();
    cx.update(|cx| install(ResponseStore::new(store.path().join("responses")), cx));
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = editor_of(cx, &s, tab);
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let send = |cx: &mut TestAppContext, n: usize| {
        cx.update_window(s.win, |_, w, cx| w.click("send", cx))
            .unwrap();
        wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == n));
    };
    send(cx, 1);
    let root = s.root.clone();
    let new_url = format!("{}/new", server.uri());
    s.ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", &new_url), cx));
    frame(cx, s.win);
    send(cx, 2);
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Previous, cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.scroll(
            "compare-list",
            ScrollDelta::Pixels(point(px(0.), px(-200.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
    let before = offset(cx, &pane);
    assert!(before.y < px(-20.), "the list scrolled: {before:?}");

    // Save the edited request, as the user does; the save refreshes the history list.
    cx.update_window(s.win, |_, w, cx| editor.update(cx, |e, cx| e.save(w, cx)))
        .unwrap();
    // The history list is re-read after the save (the store queue orders it behind the save).
    cx.update(refresh_all_history);
    settle(cx);
    frame(cx, s.win);

    assert_eq!(
        pane.read_with(cx, |p, _| p.compare_source()),
        Some(CompareSource::Previous),
        "the comparison stays open on the same source"
    );
    assert!(pane.read_with(cx, |p, _| p.comparison().is_some()));
    assert_eq!(offset(cx, &pane), before, "the save kept the scroll offset");
}

// A comparison with a long row list, scrolled down, then the history list re-read.
#[gpui_kit::test]
fn a_recompute_with_the_same_source_keeps_the_scroll_offset(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string(BODY));
    let old_body = BODY.replace(|c: char| c.is_ascii_digit(), "0");
    let a = format!(
        "{}examples:\n  - name: golden\n    response:\n      status: 200\n      body:\n        type: json\n        data: '{old_body}'\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    // A short window, so the long row list overflows the pane and can scroll.
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(500.)));
    frame(cx, s.win);
    let store = tempfile::tempdir().unwrap();
    cx.update(|cx| install(ResponseStore::new(store.path().join("responses")), cx));
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-status"));
    let pane = editor_of(cx, &s, tab).read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Example(0), cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.scroll(
            "compare-list",
            ScrollDelta::Pixels(point(px(0.), px(-200.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
    let before = offset(cx, &pane);
    assert!(before.y < px(-20.), "the list scrolled: {before:?}");

    cx.update(refresh_all_history);
    settle(cx);
    frame(cx, s.win);

    assert_eq!(
        pane.read_with(cx, |p, _| p.compare_source()),
        Some(CompareSource::Example(0))
    );
    assert_eq!(
        offset(cx, &pane),
        before,
        "the recompute kept the scroll offset"
    );
}

fn offset(
    cx: &mut TestAppContext,
    pane: &Entity<ResponsePane>,
) -> gpui_kit::Point<gpui_kit::Pixels> {
    pane.read_with(cx, |p, _| p.compare_scroll_offset().expect("a Compare tab"))
}
