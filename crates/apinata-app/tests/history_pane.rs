//! The response pane lists a request's kept sends and shows a picked one.

use crate::support;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::body_text::{format_size, format_time};
use apinata_app::last_response::{
    KeptSend, ResponseStore, StoreOp, clear_all, enqueue, install, local_labels,
};
use apinata_app::response_pane::{HistoryLine, history_row};
use apinata_app::send_model::{SendModel, SendState};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn kept(status: u16) -> RequestResult {
    let response = Response::new(status)
        .with_text_body("k")
        .with_response_time(Duration::from_millis(7));
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

fn shown(cx: &mut TestAppContext, model: &Entity<SendModel>) -> Option<u16> {
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    })
}

fn send(method: &str, url: &str) -> KeptSend {
    KeptSend {
        seq: 0,
        saved_at: at(1_760_000_000_000),
        status: 201,
        time: Duration::from_millis(7),
        size: 1,
        method: method.into(),
        url: url.into(),
    }
}

#[test]
fn a_row_names_method_and_url_then_status_time_size_and_age() {
    let now = at(1_760_000_120_000);
    let k = send("GET", "https://api.example.com/items?page=2&q=a%20b");
    let summary = format!(
        "201 · {} · {} · 2 minutes ago",
        format_time(Duration::from_millis(7)),
        format_size(1)
    );
    let target = "GET https://api.example.com/items?page=2&q=a%20b".to_owned();
    let tip = format!("{target}\n{}", local_labels(k.saved_at, now).1);
    assert_eq!(
        history_row(&k, now),
        HistoryLine {
            target,
            summary,
            tip
        }
    );
}

#[test]
fn a_send_without_a_url_reads_as_its_summary() {
    let now = at(1_760_000_120_000);
    let line = history_row(&send("", ""), now);
    assert_eq!(line.target, "");
    assert_eq!(line.tip, local_labels(at(1_760_000_000_000), now).1);
    assert_eq!(send("", "https://h/p").target(), "https://h/p");
}

#[test]
fn an_index_from_before_urls_were_kept_still_lists_its_sends() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().to_path_buf());
    let root = Path::new("/c");
    let r = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(200)
                .with_text_body("k")
                .with_url("https://h/p?q=1"),
        ),
        Vec::new(),
    );
    store.save(root, &ip("a.yml"), &r, at(1)).unwrap();
    let kept = store.history(root, &ip("a.yml"));
    assert_eq!(
        (kept[0].method.as_str(), kept[0].url.as_str()),
        ("", "https://h/p?q=1")
    );
    let path = dir.path().join("index.json");
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for e in index["entries"].as_array_mut().unwrap() {
        for s in e["sends"].as_array_mut().unwrap() {
            let s = s.as_object_mut().unwrap();
            s.remove("method");
            s.remove("url");
        }
    }
    std::fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
    let kept = store.history(root, &ip("a.yml"));
    assert_eq!(kept.len(), 1);
    assert_eq!((kept[0].method.as_str(), kept[0].url.as_str()), ("", ""));
}

#[gpui_kit::test]
fn a_live_send_keeps_its_method_and_full_url_and_names_its_row(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("live"));
    let url = format!("{}/ok?page=2&q=x", server.uri());
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request("a", &url))]);
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len()) == 1);
    let k = pane.read_with(cx, |p, _| p.history()[0].clone());
    assert_eq!((k.method.as_str(), k.url.as_str()), ("GET", url.as_str()));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    frame(cx, win);
    let name = cx
        .update_window(win, |_, w, _| {
            w.within("popup-menu")
                .find(("history-row", 0usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap()
        .expect("an accessible name");
    assert!(name.starts_with(&format!("GET {url} · 200 · ")), "{name}");
}

#[gpui_kit::test]
fn the_pane_lists_kept_sends_and_shows_a_picked_one(cx: &mut TestAppContext) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    for (i, status) in [201u16, 202, 203].into_iter().enumerate() {
        store
            .save(&root, &ip("a.yml"), &kept(status), at(1_000 + i as u64))
            .unwrap();
    }
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 3) && shown(cx, &model) == Some(203)
    });
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    frame(cx, win);
    // Rows run newest first, so the third is send 0.
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(2usize, cx))
        .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    assert_eq!(shown(cx, &model), Some(201));
    frame(cx, win);
    assert!(present(cx, win, "response-saved"));
    assert!(present(cx, win, "response-latest"));
    cx.update_window(win, |_, w, cx| w.click("response-latest", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    assert_eq!(shown(cx, &model), Some(203));
    frame(cx, win);
    assert!(!present(cx, win, "response-latest"));
}

#[gpui_kit::test]
fn a_send_adds_to_the_listed_history(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("live"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.save(&root, &ip("a.yml"), &kept(201), at(1)).unwrap();
    cx.update(|cx| install(store.clone(), cx));
    let (_win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().first().map(|k| k.status)) == Some(200)
    });
    assert_eq!(pane.read_with(cx, |p, _| p.history().len()), 2);
}

#[gpui_kit::test]
fn a_pane_without_a_target_offers_no_history(cx: &mut TestAppContext) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.save(&root, &ip("a.yml"), &kept(201), at(1)).unwrap();
    cx.update(|cx| install(store, cx));
    let model = sender(cx, &ws);
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::response_pane::ResponsePane::new(model.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |p, _| p.history().is_empty()));
    frame(cx, win);
    assert!(!present(cx, win, "response-history"));
}

#[gpui_kit::test]
fn clearing_saved_responses_empties_a_live_panes_history(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("live"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (_win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    cx.update(clear_all);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().is_empty()));
}

#[gpui_kit::test]
fn a_failed_send_keeps_the_history_dropdown(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for("http://127.0.0.1:1", "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.save(&root, &ip("a.yml"), &kept(201), at(1)).unwrap();
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert!(shown(cx, &model).is_none(), "the send failed");
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    assert!(present(cx, win, "response-history"));
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    assert_eq!(shown(cx, &model), Some(201));
}

// Whether each "Previous runs" row of a freshly opened menu is checked, newest first.
fn row_checks(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, rows: usize) -> Vec<bool> {
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    frame(cx, win);
    let checks = cx
        .update_window(win, |_, w, _| {
            let menu = w.within("popup-menu");
            (0..rows)
                .map(|ix| menu.find(ix).checked() == Some(true))
                .collect::<Vec<_>>()
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, win);
    checks
}

#[gpui_kit::test]
fn the_run_on_screen_carries_the_check(cx: &mut TestAppContext) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    for (i, status) in [201u16, 202, 203].into_iter().enumerate() {
        store
            .save(&root, &ip("a.yml"), &kept(status), at(1_000 + i as u64))
            .unwrap();
    }
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 3) && shown(cx, &model) == Some(203)
    });
    frame(cx, win);
    // The newest kept run is restored on open, so the first row is the one shown.
    assert_eq!(row_checks(cx, win, 3), [true, false, false]);
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    frame(cx, win);
    assert_eq!(row_checks(cx, win, 3), [false, false, true]);
}

#[gpui_kit::test]
fn a_live_send_checks_its_own_row(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("live"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.save(&root, &ip("a.yml"), &kept(201), at(1)).unwrap();
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 2)
            && model.read_with(cx, |m, _| m.kept_as().is_some())
    });
    frame(cx, win);
    assert_eq!(row_checks(cx, win, 2), [true, false]);
}

#[gpui_kit::test]
fn a_failed_send_on_screen_checks_no_row(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for("http://127.0.0.1:1", "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.save(&root, &ip("a.yml"), &kept(201), at(1)).unwrap();
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    frame(cx, win);
    assert_eq!(row_checks(cx, win, 1), [false]);
}

#[gpui_kit::test]
fn a_long_target_gives_way_to_the_whole_summary(cx: &mut TestAppContext) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let url = format!("https://api.example.com/{}", "segment/".repeat(40));
    let long = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("k").with_url(url)),
        Vec::new(),
    );
    store.save(&root, &ip("a.yml"), &long, at(1)).unwrap();
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    frame(cx, win);
    let (item, row, summary) = cx
        .update_window(win, |_, w, _| {
            let menu = w.within("popup-menu");
            (
                menu.find(0usize).bounds(),
                menu.find(("history-row", 0usize)).bounds(),
                menu.find(("history-summary", 0usize)).bounds(),
            )
        })
        .unwrap();
    assert!(
        row.right() <= item.right(),
        "row {row:?} past item {item:?}"
    );
    assert!(
        summary.right() <= row.right() && summary.left() >= row.left(),
        "summary {summary:?} outside row {row:?}"
    );
}

// An editor over `a.yml` with `count` kept sends, the oldest shown, so the menu's last row is
// the checked one; `height` is the window's.
fn viewing_oldest_of(
    cx: &mut TestAppContext,
    count: u64,
    height: f32,
) -> (gpui_kit::AnyWindowHandle, tempfile::TempDir) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    for i in 0..count {
        store
            .save(&root, &ip("a.yml"), &kept(200), at(1_000 + i))
            .unwrap();
    }
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(height)));
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == count as usize)
    });
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    for _ in 0..3 {
        frame(cx, win);
    }
    std::mem::forget(state);
    (win, col)
}

// The bounds of the open menu and of its first item.
fn menu_and_first(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
) -> (
    gpui_kit::Bounds<gpui_kit::Pixels>,
    gpui_kit::Bounds<gpui_kit::Pixels>,
) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let whole = w.find("popup-menu").bounds();
        (whole, w.within("popup-menu").find(0usize).bounds())
    })
    .unwrap()
}

#[gpui_kit::test]
fn three_runs_open_without_a_scrollbar_and_keep_the_top_padding(cx: &mut TestAppContext) {
    let (win, _col) = viewing_oldest_of(cx, 3, 800.);
    let (menu, first) = menu_and_first(cx, win);
    assert!(
        first.top() > menu.top(),
        "the first row {first:?} keeps the padding under the menu's top {menu:?}"
    );
    let bar = gpui_kit::VisualTestContext::from_window(win, cx).debug_bounds("scrollbar-overlay");
    assert_ne!(bar, Some(menu), "a list that fits draws no scrollbar");
}

// The kit draws a row's check 12 px after the row and 12 px wide.
const CHECK_COLUMN: f32 = 24.;

#[gpui_kit::test]
fn a_scrolling_menu_keeps_the_checks_clear_of_its_scrollbar(cx: &mut TestAppContext) {
    let (win, _col) = viewing_oldest_of(cx, 20, 500.);
    let menu = menu_and_first(cx, win).0;
    let bar = gpui_kit::VisualTestContext::from_window(win, cx).debug_bounds("scrollbar-overlay");
    assert_eq!(bar, Some(menu), "twenty runs in a short window scroll");
    let rows: Vec<_> = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let menu = w.within("popup-menu");
            (0..20usize)
                .filter_map(|ix| menu.try_find(("history-row", ix)).map(|r| r.bounds()))
                .collect()
        })
        .unwrap();
    assert!(!rows.is_empty());
    let gutter = menu.right() - apinata_app::style::SCROLLBAR_GUTTER;
    for row in rows {
        assert!(
            row.right() + gpui_kit::px(CHECK_COLUMN) <= gutter,
            "the check after {row:?} reaches into the scrollbar's gutter at {gutter:?}"
        );
    }
}

// Picking the newest kept send shows it as the latest: nothing older is on screen, so there
// is no Back to latest.
#[gpui_kit::test]
fn picking_the_newest_send_offers_no_back_to_latest(cx: &mut TestAppContext) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    for (i, status) in [201u16, 202].into_iter().enumerate() {
        store
            .save(&root, &ip("a.yml"), &kept(status), at(1_000 + i as u64))
            .unwrap();
    }
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 2) && shown(cx, &model) == Some(202)
    });
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    pane.update(cx, |p, cx| p.show_history(1, cx));
    wait_real(cx, |cx| shown(cx, &model) == Some(202));
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    frame(cx, win);
    assert!(!present(cx, win, "response-latest"));
    assert_eq!(row_checks(cx, win, 2), [true, false]);
}

// The only send, picked right after it ran, stays the live result with its timeline.
#[gpui_kit::test]
fn picking_the_live_send_keeps_it_live(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500).set_body_string("live"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 1)
            && model.read_with(cx, |m, _| m.kept_as().is_some())
    });
    pane.update(cx, |p, cx| p.show_history(0, cx));
    // The store queue answers in order, so a read queued after the pick marks its end.
    let (tx, rx) = async_channel::bounded(1);
    let (r, item) = (root.clone(), ip("a.yml"));
    cx.update(|cx| {
        enqueue(
            StoreOp::History {
                root: r,
                item,
                reply: tx,
            },
            cx,
        )
    });
    wait_real(cx, |_| !rx.is_empty());
    cx.run_until_parked();
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    assert!(
        model.read_with(cx, |m, _| m.kept_as().is_some()),
        "still the live result"
    );
    frame(cx, win);
    assert!(!present(cx, win, "response-latest"));
}
