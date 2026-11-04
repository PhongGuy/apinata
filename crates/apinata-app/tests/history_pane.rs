//! The response pane lists a request's kept sends and shows a picked one.

use crate::support;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::body_text::{format_size, format_time};
use apinata_app::last_response::{KeptSend, ResponseStore, clear_all, install, local_labels};
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
