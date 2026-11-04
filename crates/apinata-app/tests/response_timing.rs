//! The timing bar in the response header row.

use crate::support;

use apinata_app::response_pane::ResponsePane;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
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
    frame(cx, win);
    (win, view)
}

#[gpui_kit::test]
fn a_measured_send_shows_the_bar_with_its_phases(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string("ok"), "");
    let (win, _view) = pane(cx, &s);
    assert!(present(cx, win, "response-timing"));
    // An IP literal host has no DNS phase, and a fresh connection is not "reused".
    assert!(present(cx, win, "response-timing-connect"));
    assert!(present(cx, win, "response-timing-wait"));
    assert!(!present(cx, win, "response-timing-dns"));
}
