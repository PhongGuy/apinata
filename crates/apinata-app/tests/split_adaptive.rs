//! The response collapses to a strip until its tab first sends or restores a kept result;
//! from then on the split stays open for that tab.

use crate::support;

use apinata_app::response_pane::RESPONSE_HINT;
use apinata_app::style::{RESPONSE_STRIP_WIDTH, ROW_HEIGHT, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px};
use support::*;

#[gpui_kit::test]
fn a_restored_result_sets_the_flag(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    assert!(!model.read_with(cx, |m, _| m.ever_shown()));
    give_result(cx, &ed);
    assert!(model.read_with(cx, |m, _| m.ever_shown()));
    assert_eq!(RESPONSE_HINT, "Send a request to see the response");
}

#[gpui_kit::test]
fn a_cancelled_send_keeps_the_pane_open(cx: &mut TestAppContext) {
    http_ready(cx);
    let slow = wiremock::ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30));
    let server = mock_server(cx, slow);
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    assert!(present(cx, win, "response-hint"));
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    assert!(
        !present(cx, win, "response-hint"),
        "the pane opens at the send"
    );
    model.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    frame(cx, win);
    assert!(model.read_with(cx, |m, _| m.ever_shown()));
    assert!(
        !present(cx, win, "response-hint"),
        "a cancel never collapses it"
    );
}

#[gpui_kit::test]
fn with_no_result_the_response_is_one_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let height = w.viewport_size().height;
        let response = w.find("response").bounds();
        let request = w.find("request-column").bounds();
        assert!(
            response.size.height <= at_zoom(ROW_HEIGHT, 1.0) * 2.,
            "{response:?}"
        );
        assert!(request.size.height >= height * 0.6, "{request:?}");
        assert!(w.try_find("response-hint").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn side_by_side_the_strip_is_a_narrow_column(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| st.response_orientation = Orientation::Horizontal)
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let response = w.find("response").bounds();
        let request = w.find("request-column").bounds();
        assert!(
            response.size.width <= at_zoom(RESPONSE_STRIP_WIDTH, 1.0) + px(0.5),
            "{response:?}"
        );
        assert!(request.size.width > response.size.width * 2., "{request:?}");
    })
    .unwrap();
}

// The Send key chip in the side-by-side column is as wide as its text, not the whole column.
#[gpui_kit::test]
fn side_by_side_the_send_key_chip_fits_its_text(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    let col = two_requests();
    let s = started(cx, &col);
    cx.update(apinata_app::keymap::init);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| st.response_orientation = Orientation::Horizontal)
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let response = w.find("response").bounds();
        let kbd = w.find("response-strip-kbd").bounds();
        assert!(
            kbd.size.width < response.size.width * 0.75,
            "{kbd:?} in {response:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_result_opens_the_split_evenly(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    give_result(cx, &ed);
    frame(cx, win);
    frame(cx, win);
    let sizes = ed.read_with(cx, |e, cx| e.split().read(cx).sizes().clone());
    assert_eq!(sizes.len(), 2);
    assert!((sizes[0] - sizes[1]).abs() <= px(1.), "{sizes:?}");
    assert!(!present(cx, win, "response-hint"));
}

#[gpui_kit::test]
fn the_strip_follows_each_tabs_own_result(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, a);
    give_result(cx, &ed);
    frame(cx, s.win);
    assert!(!present(cx, s.win, "response-hint"));
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "response-hint"),
        "b has no result of its own"
    );
}

// A never-sent tab's strip names Send's key as the idle pane does, so the hint shows from the
// first open and not only after a result is cleared.
#[gpui_kit::test]
fn a_fresh_tabs_strip_shows_the_send_key(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    cx.update(apinata_app::keymap::init);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    assert!(present(cx, win, "response-hint"));
    let hinted = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.within("response")
                .try_find("response-strip-kbd")
                .is_some()
        })
        .unwrap();
    assert!(hinted, "the strip reads Send a request ... Ctrl+Enter");
}
