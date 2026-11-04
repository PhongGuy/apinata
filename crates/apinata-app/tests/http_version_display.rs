//! Each hop's negotiated version in the response timeline and the console dock.

use crate::support;

use apinata_app::executor;
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::{ExecOptions, Executor, HttpVersionPolicy, SessionLog};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

fn pane(cx: &mut TestAppContext, s: &Sent) -> (AnyWindowHandle, Entity<ResponsePane>) {
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn first_hop_line(cx: &mut TestAppContext, win: AnyWindowHandle) -> String {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.within(("hop", 0usize))
            .find("hop-line")
            .label()
            .unwrap_or_default()
            .to_owned()
    })
    .unwrap()
}

/// Sends `a.yml` once through a shared executor under `policy`.
fn send_under(cx: &mut TestAppContext, policy: HttpVersionPolicy) -> Sent {
    http_ready(cx);
    let exec = Executor::new(ExecOptions::new().http_version(policy)).unwrap();
    cx.update(|cx| cx.set_global(executor::SharedExecutor(Arc::new(exec))));
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    Sent {
        server,
        col,
        state,
        ws,
        root,
        model,
        result,
    }
}

#[gpui_kit::test]
fn the_timeline_shows_http1_by_default(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    assert_eq!(
        view.read_with(cx, |p, cx| p.hop_versions(cx)),
        [Some("HTTP/1.1".to_owned())]
    );
    let line = first_hop_line(cx, win);
    assert!(line.contains(" 200 · HTTP/1.1 · "), "{line}");
}

#[gpui_kit::test]
fn the_timeline_shows_http2_for_an_h2_hop(cx: &mut TestAppContext) {
    let s = send_under(cx, HttpVersionPolicy::Http2PriorKnowledge);
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    assert_eq!(
        view.read_with(cx, |p, cx| p.hop_versions(cx)),
        [Some("HTTP/2".to_owned())]
    );
    let line = first_hop_line(cx, win);
    assert!(line.contains(" 200 · HTTP/2 · "), "{line}");
}

#[gpui_kit::test]
fn the_console_detail_shows_each_hops_version(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    for (policy, shown) in [
        (HttpVersionPolicy::Http1Only, " 200 · HTTP/1.1 · "),
        (HttpVersionPolicy::Http2PriorKnowledge, " 200 · HTTP/2 · "),
    ] {
        let log = SessionLog::new();
        let exec = Executor::new(
            ExecOptions::new()
                .http_version(policy)
                .session_log(log.clone()),
        )
        .unwrap();
        send_direct(cx, &exec, &server.uri());
        let e = log.network_since(0).entries.remove(0);
        let detail = apinata_app::console_dock::detail_text(&e);
        assert!(
            detail
                .lines()
                .any(|l| l.starts_with("GET ") && l.contains(shown)),
            "{policy}: {detail}"
        );
    }
}
