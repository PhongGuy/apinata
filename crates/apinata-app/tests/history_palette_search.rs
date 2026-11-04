//! The History palette filters its sends by name, URL, method and status as the query is typed.

use crate::support;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::last_response::{Limits, ResponseStore, STORE_LIMIT};
use apinata_app::send_model::SendState;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn kept(item: &str, status: u16, url: &str) -> RequestResult {
    let response = Response::new(status).with_text_body("k").with_url(url);
    RequestResult::from_parts(ip(item), "x", Outcome::Passed, Some(response), Vec::new())
}

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("List orders", "http://x/a")),
        ("b.yml", &request("Create post", "http://x/b")),
        ("c.yml", &request("Get item", "http://x/c")),
    ])
}

// Rows, newest first: List orders 404 /orders/9, Get item 500 /posts/7,
// Create post 201 /articles, List orders 200 /orders.
fn seeded(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let root = col.path().to_path_buf();
    started_with(cx, col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        let sends = [
            ("a.yml", 200, "https://api.example.com/orders"),
            ("b.yml", 201, "https://api.example.com/articles"),
            ("c.yml", 500, "https://api.example.com/posts/7"),
            ("a.yml", 404, "https://api.example.com/orders/9"),
        ];
        for (i, (item, status, url)) in sends.into_iter().enumerate() {
            let t = at(1_000 * (i as u64 + 1));
            store
                .save(&root, &ip(item), &kept(item, status, url), t)
                .unwrap();
        }
    })
}

fn open_history_palette(cx: &mut TestAppContext, s: &Started) {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
}

// Types `query` into the palette's field and renders exactly one frame.
fn search(cx: &mut TestAppContext, s: &Started, query: &str) {
    cx.update_window(s.win, |_, w, cx| w.input(query, cx))
        .unwrap();
    frame(cx, s.win);
}

// The (name, target) of every listed row, top down, as the last frame drew them.
fn listed(cx: &mut TestAppContext, s: &Started) -> Vec<(String, String)> {
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("history-palette");
        (0usize..)
            .map_while(|r| {
                // A highlighted name carries the match id instead.
                let name = palette
                    .try_find(("history-name", r))
                    .or_else(|| palette.try_find(("history-name-match", r)))?
                    .label()?
                    .to_owned();
                let target = palette
                    .try_find(("history-target", r))
                    .and_then(|t| t.label().map(str::to_owned))
                    .unwrap_or_default();
                Some((name, target))
            })
            .collect()
    })
    .unwrap()
}

fn names(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    listed(cx, s).into_iter().map(|(n, _)| n).collect()
}

#[gpui_kit::test]
fn the_empty_query_lists_every_send(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    assert_eq!(
        names(cx, &s),
        ["List orders", "Get item", "Create post", "List orders"]
    );
}

#[gpui_kit::test]
fn digits_keep_the_sends_with_that_status_or_url(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    search(cx, &s, "500");
    assert_eq!(names(cx, &s), ["Get item"]);
}

#[gpui_kit::test]
fn a_path_keeps_the_sends_to_it(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    search(cx, &s, "/orders");
    let got = listed(cx, &s);
    let targets: Vec<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(
        targets,
        [
            "https://api.example.com/orders/9",
            "https://api.example.com/orders"
        ]
    );
}

#[gpui_kit::test]
fn a_method_word_skips_urls_that_contain_it(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    search(cx, &s, "post");
    assert_eq!(names(cx, &s), ["Create post"]);
}

#[gpui_kit::test]
fn a_status_class_keeps_its_sends(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    search(cx, &s, "4xx");
    assert_eq!(listed(cx, &s).len(), 1);
    assert!(
        drawn(cx, &s, ("history-detail-match", 0usize)),
        "the status is highlighted"
    );
    assert_eq!(
        listed(cx, &s)[0].1,
        "https://api.example.com/orders/9",
        "the 404 send"
    );
}

#[gpui_kit::test]
fn a_name_hit_is_highlighted(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    assert!(!drawn(cx, &s, ("history-name-match", 0usize)));
    search(cx, &s, "orders");
    assert!(drawn(cx, &s, ("history-name-match", 0usize)));
}

// Whether the last frame drew `id` inside the palette.
fn drawn(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) -> bool {
    let id = id.into();
    cx.update_window(s.win, |_, w, _| {
        w.within("history-palette").try_find(id).is_some()
    })
    .unwrap()
}

// Enter pressed before the next frame draws the filtered rows still opens the first match,
// not the first row of the list drawn for the previous query.
#[gpui_kit::test]
fn enter_before_the_next_frame_opens_the_first_match(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    // A test flush draws after every update, so the keys share one update: Enter lands
    // before any frame draws the rows for the new query.
    cx.update_window(s.win, |_, w, cx| {
        for key in ["p", "o", "s", "t"] {
            let mut stroke = gpui_kit::Keystroke::parse(key).unwrap();
            stroke.key_char = Some(key.to_owned());
            w.dispatch_keystroke(stroke, cx);
        }
        w.dispatch_keystroke(gpui_kit::Keystroke::parse("enter").unwrap(), cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
}

#[gpui_kit::test]
fn a_name_outside_ascii_matches_in_any_case(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("Übersicht café", "http://x/a")),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, move |paths| {
        ResponseStore::new(paths.responses.clone())
            .save(
                &root,
                &ip("a.yml"),
                &kept("a.yml", 200, "https://h/a"),
                at(1_000),
            )
            .unwrap();
    });
    open_history_palette(cx, &s);
    search(cx, &s, "ÜBER CAFÉ");
    assert_eq!(names(cx, &s), ["Übersicht café"]);
    assert!(drawn(cx, &s, ("history-name-match", 0usize)));
}

// Down then Enter opens the second filtered row, the oldest /orders send, not the second
// row of the unfiltered list.
#[gpui_kit::test]
fn arrows_and_enter_confirm_the_filtered_send(cx: &mut TestAppContext) {
    let col = collection();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    search(cx, &s, "/orders");
    cx.simulate_keystrokes(s.win, "down enter");
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "the History dialog closed");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    let editor = s.shell.read_with(cx, |sh, cx| {
        let tab = sh.workspace().read(cx).tabs().active().unwrap().id;
        sh.tab_area().read(cx).editor(tab).cloned().unwrap()
    });
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    let status = model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    });
    assert_eq!(status, Some(200));
}

// The newest 100 sends are all to /filler; the search still reaches the oldest, to /needle.
#[gpui_kit::test]
fn a_match_older_than_the_newest_hundred_is_found(cx: &mut TestAppContext) {
    let col = collection();
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        let limits = Limits {
            per_request: 200,
            total_bytes: STORE_LIMIT,
        };
        let urls = std::iter::once("https://h/needle".to_owned())
            .chain((0..110).map(|i| format!("https://h/filler/{i}")));
        for (i, url) in urls.enumerate() {
            let r = kept("a.yml", 200, &url);
            store
                .save_with(&root, &ip("a.yml"), &r, at(1_000 + i as u64), limits)
                .unwrap();
        }
    });
    open_history_palette(cx, &s);
    search(cx, &s, "needle");
    let got = listed(cx, &s);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, "https://h/needle");
}
