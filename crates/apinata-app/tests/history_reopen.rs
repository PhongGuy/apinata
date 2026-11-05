//! Choosing an unsaved draft's send opens a new draft that continues its history.

use crate::support;

use std::time::{Duration, UNIX_EPOCH};

use apinata_app::history_palette::OpenHistoryEntry;
use apinata_app::last_response::{Limits, ResponseStore, SendExtra};
use apinata_app::send_model::SendState;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext, VisualTestContext};
use support::*;

const KEY: &str = "untitled-17600000000000000001.draft";
const GONE: &str = "untitled-17600000000000000002.draft";
const BAD: &str = "untitled-17600000000000000003.draft";
const MASKED: &str = "untitled-17600000000000000004.draft";
const MASKED_SNAP: &str = r#"{"info":{"name":"Masked","type":"http","seq":1},"http":{"method":"GET","url":"http://127.0.0.1:9/m","headers":[{"name":"X-API-Key","value":"[redacted]"}]}}"#;
const SNAP: &str = r#"{"info":{"name":"Kept draft","type":"http","seq":1},"http":{"method":"POST","url":"http://127.0.0.1:9/kept"}}"#;

fn kept(item: &str, status: u16) -> RequestResult {
    let response = Response::new(status).with_text_body("k");
    RequestResult::from_parts(ip(item), "x", Outcome::Passed, Some(response), Vec::new())
}

fn seeded(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let root = col.path().to_path_buf();
    started_with(cx, col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        let save = |item: &str, status, ms, request: Option<&str>| {
            let extra = SendExtra {
                request: request.map(str::to_owned),
                origin: None,
            };
            let at = UNIX_EPOCH + Duration::from_millis(ms);
            store
                .save_send(
                    &root,
                    &ip(item),
                    &kept(item, status),
                    &[],
                    at,
                    Limits::default(),
                    &extra,
                )
                .unwrap()
        };
        save(KEY, 201, 1_000, Some(SNAP));
        save(KEY, 202, 2_000, Some(SNAP));
        save(GONE, 203, 3_000, None);
        save(BAD, 204, 4_000, Some("not json"));
        save(MASKED, 205, 5_000, Some(MASKED_SNAP));
    })
}

fn choose(cx: &mut TestAppContext, s: &Started, path: &str, seq: u64) {
    let shell = s.shell.clone();
    let action = OpenHistoryEntry {
        root: s.root.clone(),
        path: ip(path),
        seq,
    };
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| shell.read(cx).focus_handle(cx).focus(w, cx));
    v.run_until_parked();
    v.update(|w, cx| w.dispatch_action(Box::new(action), cx));
    v.run_until_parked();
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn choosing_a_draft_entry_opens_a_new_draft_showing_that_send(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    choose(cx, &s, KEY, 0);
    wait_real(cx, |cx| {
        s.ws.read_with(cx, |w, _| {
            w.tabs()
                .active()
                .and_then(|e| e.item().cloned())
                .is_some_and(|p| p.is_draft())
        })
    });
    let (tab, item) = s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().unwrap();
        (e.id, e.item().cloned().unwrap())
    });
    assert_ne!(item, ip(KEY), "a fresh draft path");
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.tabs().history_key(tab).cloned()),
        Some(ip(KEY))
    );
    let view = view_of(cx, &s.ws, &s.root, item.as_str());
    assert_eq!(
        (view.method.as_str(), view.url.as_str()),
        ("POST", "http://127.0.0.1:9/kept")
    );
    assert!(view.draft);
    frame(cx, s.win);
    let editor = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("the draft's editor");
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.viewing()) == Some(0)
            && pane.read_with(cx, |p, _| p.history().len() == 2)
    });
    let status = model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    });
    assert_eq!(status, Some(201));
    model.update(cx, |m, cx| m.back_to_latest(cx));
    let latest = model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    });
    assert_eq!(latest, Some(202), "the newest kept send is the latest");
}

#[gpui_kit::test]
fn a_draft_entry_with_no_readable_snapshot_opens_nothing_and_says_so(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    for (n, path) in [GONE, BAD].into_iter().enumerate() {
        let before = toasts(cx, &s);
        choose(cx, &s, path, 0);
        wait_real(cx, |cx| toasts(cx, &s) == before + 1);
        assert!(
            s.ws.read_with(cx, |w, _| w.tabs().entries().is_empty()),
            "case {n}"
        );
    }
}

#[gpui_kit::test]
fn a_saved_entry_still_opens_its_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        store
            .save(&root, &ip("a.yml"), &kept("a.yml", 207), UNIX_EPOCH)
            .unwrap();
    });
    choose(cx, &s, "a.yml", 0);
    wait_real(cx, |cx| tab_names(cx, &s.ws) == ["a.yml"]);
}

// The kept copy lost its credentials, so the reopened draft says they must be entered again
// rather than letting the placeholder go out unnoticed.
#[gpui_kit::test]
fn a_draft_entry_with_hidden_credentials_opens_and_says_to_enter_them(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let before = toasts(cx, &s);
    choose(cx, &s, MASKED, 0);
    wait_real(cx, |cx| {
        s.ws.read_with(cx, |w, _| {
            w.tabs()
                .active()
                .and_then(|e| e.item().cloned())
                .is_some_and(|p| p.is_draft())
        })
    });
    wait_real(cx, |cx| toasts(cx, &s) == before + 1);
    assert!(apinata_app::history_palette::DRAFT_MASKED.contains("[redacted]"));
}

#[gpui_kit::test]
fn a_draft_entry_without_hidden_credentials_opens_quietly(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let before = toasts(cx, &s);
    choose(cx, &s, KEY, 0);
    wait_real(cx, |cx| {
        s.ws.read_with(cx, |w, _| {
            w.tabs()
                .active()
                .and_then(|e| e.item().cloned())
                .is_some_and(|p| p.is_draft())
        })
    });
    cx.run_until_parked();
    assert_eq!(toasts(cx, &s), before);
}
