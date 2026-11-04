//! The URL row: a fixed method select, Send beside the URL, and a spinner on Cancel.

use crate::support;

use apinata_app::style::{METHOD_SELECT, at_zoom};
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

#[gpui_kit::test]
fn the_method_is_fixed_and_send_shares_the_url_line(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let method = w.find("method").bounds();
        let url = w.find("url").bounds();
        let send = w.find("send").bounds();
        assert_eq!(method.size.width, at_zoom(METHOD_SELECT, 1.0));
        assert!((url.center().y - send.center().y).abs() < gpui_kit::px(1.));
        assert!(method.right() <= url.left() && url.right() <= send.left());
    })
    .unwrap();
}

#[gpui_kit::test]
fn cancel_shows_a_spinner_while_sending(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert!(!present(cx, win, "cancel-spinner"));
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    assert!(present(cx, win, "cancel-spinner"));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    cx.update_window(win, |_, w, cx| w.click("cancel", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert!(!present(cx, win, "cancel-spinner"));
}
