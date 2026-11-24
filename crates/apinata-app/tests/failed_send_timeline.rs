//! After a send that got no response, the Timeline lists the hops that completed and the
//! request left unanswered, masked as an answered send's are, and the Console stays reachable.

use crate::support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::RequestResult;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, WindowOptions,
};
use support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "lit-tok-7731";
const REQ: &str = "unanswered-request-header";

struct Fx {
    win: AnyWindowHandle,
    view: Entity<ResponsePane>,
    result: Arc<RequestResult>,
    started: Started,
    _keep: (MockServer, tempfile::TempDir),
}

// A GET that is redirected once, then times out waiting on the slow second hop.
fn shown(cx: &mut TestAppContext) -> Fx {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(path("/start"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/slow"))
            .mount(&server),
    );
    handle.block_on(
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server),
    );
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/start\"\n  headers:\n    - name: Authorization\n      value: Bearer {TOKEN}\n    - name: x-plain\n      value: hello\nsettings:\n  timeout: 300\n",
        server.uri()
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let started = started_with(cx, &col, |p| {
        apinata_app::persist::save_json(&p.settings, &test_settings()).unwrap()
    });
    let model = sender(cx, &started.ws);
    assert!(model.update(cx, |m, cx| m.send(&started.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.response.is_none());
    assert!(result.error.is_some());
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    frame(cx, win);
    Fx {
        win,
        view,
        result,
        started,
        _keep: (server, col),
    }
}

fn label(cx: &mut TestAppContext, fx: &Fx, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

fn row_of(cx: &mut TestAppContext, fx: &Fx, name: &str) -> usize {
    (0..40)
        .find(|&ix| {
            present(cx, fx.win, ("unanswered-request-header-name", ix))
                && label(cx, fx, ("unanswered-request-header-name", ix)) == name
        })
        .unwrap_or_else(|| panic!("no {name} row in {REQ}"))
}

#[gpui_kit::test]
fn the_timeline_lists_the_completed_hop_then_the_unanswered_request(cx: &mut TestAppContext) {
    let fx = shown(cx);
    assert!(present(cx, fx.win, ("hop", 0usize)));
    assert!(!present(cx, fx.win, ("hop", 1usize)));
    let hop = label(cx, &fx, "hop-line");
    assert!(hop.contains("302"), "{hop}");
    assert!(hop.contains("/start"), "{hop}");
    let line = label(cx, &fx, "unanswered-line");
    assert!(line.starts_with("GET "), "{line}");
    assert!(line.contains("/slow"), "{line}");
    assert!(line.contains("No response"), "{line}");
}

#[gpui_kit::test]
fn the_unanswered_request_headers_are_masked_like_an_answered_hop(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let sent = fx.result.unanswered.as_ref().expect("unanswered");
    let row = row_of(cx, &fx, "authorization");
    let shown = label(cx, &fx, ("unanswered-request-header-value", row));
    assert!(!shown.contains(TOKEN), "{shown}");
    let (name, value) = &sent.headers[row];
    assert_eq!(shown, fx.result.redact_header(name, value));
    let plain = row_of(cx, &fx, "x-plain");
    assert_eq!(
        label(cx, &fx, ("unanswered-request-header-value", plain)),
        "hello"
    );
}

#[gpui_kit::test]
fn the_failure_still_shows_and_the_console_is_reachable(cx: &mut TestAppContext) {
    let fx = shown(cx);
    // Off the Body tab the failure keeps a one-line banner; Body shows it whole.
    assert!(present(cx, fx.win, "response-stream-error"));
    fx.view
        .update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    frame(cx, fx.win);
    let tabs = fx.view.read_with(cx, |p, cx| p.shown_tabs(cx));
    assert!(tabs.contains(&ResponseTab::Timeline));
    assert!(tabs.contains(&ResponseTab::Console));
}

fn files_under(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[gpui_kit::test]
fn nothing_of_the_sent_headers_is_written_to_disk(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let data = fx.started.files.path().to_path_buf();
    // The kept failure lands on the store's worker, so wait for its list to appear.
    wait_real(cx, |_| {
        let mut all = Vec::new();
        files_under(&data, &mut all);
        all.iter().any(|p| p.ends_with("failures.json"))
    });
    let mut all = Vec::new();
    files_under(&data, &mut all);
    for file in all {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(TOKEN), "{}", file.display());
        assert!(!text.contains("x-plain"), "{}", file.display());
    }
}

#[gpui_kit::test]
fn on_the_body_tab_the_error_stands_alone_and_links_to_the_timeline(cx: &mut TestAppContext) {
    let fx = shown(cx);
    fx.view.update(cx, |p, cx| p.set_tab(ResponseTab::Body, cx));
    frame(cx, fx.win);
    // The error keeps the whole pane, so a short window still reaches Retry.
    assert!(present(cx, fx.win, "response-error"));
    assert!(!present(cx, fx.win, ("response-tab", 0usize)));
    assert!(present(cx, fx.win, "response-error-console"));
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("response-error-timeline", cx)
    })
    .unwrap();
    frame(cx, fx.win);
    assert_eq!(fx.view.read_with(cx, |p, _| p.tab()), ResponseTab::Timeline);
    assert!(present(cx, fx.win, "unanswered-line"));
}
