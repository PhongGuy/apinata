//! The address a response came from: in the status row and on every Timeline hop row.

use crate::support;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
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

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_status_row_shows_the_servers_address(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let (win, _view) = pane(cx, &s);
    assert_eq!(
        label(cx, win, "response-address"),
        Some(s.server.address().to_string())
    );
}

#[gpui_kit::test]
fn each_timeline_hop_row_shows_its_address(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::start());
    let redirect = ResponseTemplate::new(302).insert_header("location", "/final");
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/ok"))
            .respond_with(redirect)
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/final"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    let s = Sent {
        server,
        col,
        state,
        ws,
        root,
        model,
        result,
    };
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    let want = s.server.address().to_string();
    assert_eq!(
        view.read_with(cx, |p, cx| p.hop_peers(cx)),
        [Some(want.clone()), Some(want.clone())]
    );
    for ix in 0..2usize {
        let shown = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.within(("hop", ix))
                    .find("hop-peer")
                    .label()
                    .map(str::to_owned)
            })
            .unwrap();
        assert_eq!(shown, Some(want.clone()), "hop {ix}");
    }
}
