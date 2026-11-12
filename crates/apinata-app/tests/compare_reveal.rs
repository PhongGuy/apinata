//! Compare finds header changes the mask hides, shows them masked, and reveals them with the
//! Headers tab's eye; a side without the real value never makes a false change or shows one.

use crate::support;
use apinata_app::body_view::SCRUBBED_TIP;
use apinata_app::compare_source::CompareSource;
use apinata_app::last_response::{ResponseStore, install};
use apinata_app::response_pane::ResponsePane;
use apinata_app::send_model::SendState;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext};
use std::time::{Duration, SystemTime};
use support::*;

struct Fx {
    win: AnyWindowHandle,
    pane: Entity<ResponsePane>,
    _keep: Box<dyn std::any::Any>,
}

pub(crate) fn template(headers: &[(&str, &str)]) -> wiremock::ResponseTemplate {
    let mut t = wiremock::ResponseTemplate::new(200).set_body_string("{}");
    for (name, value) in headers {
        t = t.append_header(*name, *value);
    }
    t
}

// Two live sends of `a.yml` in one session, answered with `first` then `second` as headers,
// compared against the previous send with the Compare tab open.
fn live_pair(cx: &mut TestAppContext, first: &[(&str, &str)], second: &[(&str, &str)]) -> Fx {
    http_ready(cx);
    let server = mock_server(cx, template(first));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    for (sends, headers) in [(1usize, first), (2, second)] {
        if sends == 2 {
            handle.block_on(server.reset());
            handle.block_on(
                wiremock::Mock::given(wiremock::matchers::any())
                    .respond_with(template(headers))
                    .mount(&server),
            );
        }
        let r = root.clone();
        model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
        finished(cx, &model);
        wait_real(cx, |cx| {
            pane.read_with(cx, |p, _| p.history().len() == sends)
        });
    }
    open_compare(cx, win, &pane);
    Fx {
        win,
        pane,
        _keep: Box::new((server, col, state, editor, ws)),
    }
}

pub(crate) fn open_compare(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    pane: &Entity<ResponsePane>,
) {
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Previous, cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, win);
    // Set-Cookie changes on every send, so its row sits in the volatile fold.
    if let Some(ix) = (0..40usize).find(|&ix| present(cx, win, ("compare-volatile", ix))) {
        click(cx, win, ("compare-volatile", ix));
    }
}

pub(crate) fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    frame(cx, win);
}

pub(crate) fn label(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

// Every drawn compare row's label, by row index.
fn row_labels(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<(usize, String)> {
    let drawn: Vec<usize> = (0..60usize)
        .filter(|&ix| present(cx, win, ("compare-row", ix)))
        .collect();
    drawn
        .into_iter()
        .map(|ix| (ix, label(cx, win, ("compare-row", ix))))
        .collect()
}

// The index of the header row for `name`, whose label starts "name:".
pub(crate) fn header_row(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    name: &str,
) -> Option<usize> {
    let prefix = format!("{}:", name.to_ascii_lowercase());
    row_labels(cx, win)
        .into_iter()
        .find(|(_, l)| l.to_ascii_lowercase().starts_with(&prefix))
        .map(|(ix, _)| ix)
}

fn assert_hidden(cx: &mut TestAppContext, fx: &Fx, secrets: &[&str]) {
    let mut texts: Vec<String> = row_labels(cx, fx.win).into_iter().map(|(_, l)| l).collect();
    texts.push(label(cx, fx.win, "compare-summary"));
    let rows = fx.pane.read_with(cx, |p, _| p.compare_rows());
    texts.extend(rows.iter().map(|r| format!("{r:?}")));
    for text in texts {
        for s in secrets {
            assert!(!text.contains(s), "{s} leaks in {text}");
        }
    }
}

#[gpui_kit::test]
fn a_changed_cookie_is_found_masked_and_revealed_by_its_eye(cx: &mut TestAppContext) {
    let fx = live_pair(
        cx,
        &[("set-cookie", "sid=aaa; Path=/")],
        &[("set-cookie", "sid=bbb; Path=/")],
    );
    let ix = header_row(cx, fx.win, "set-cookie").expect("a Set-Cookie row");
    assert!(label(cx, fx.win, ("compare-row", ix)).contains("sid="));
    assert_hidden(cx, &fx, &["aaa", "bbb"]);
    assert!(
        present(cx, fx.win, ("compare-reveal", ix)),
        "the row has an eye"
    );
    // The list's rows share one height, so the eye must fit inside its row.
    cx.update_window(fx.win, |_, w, _| {
        let (row, eye) = (
            w.find(("compare-row", ix)).bounds(),
            w.find(("compare-eye", ix)).bounds(),
        );
        assert!(
            eye.top() >= row.top() && eye.bottom() <= row.bottom() && eye.right() <= row.right(),
            "{eye:?} inside {row:?}"
        );
    })
    .unwrap();
    click(cx, fx.win, ("compare-eye", ix));
    let shown = label(cx, fx.win, ("compare-row", ix));
    assert!(
        shown.contains("sid=aaa; Path=/") && shown.contains("sid=bbb; Path=/"),
        "{shown}"
    );
    assert!(present(cx, fx.win, ("compare-hide", ix)));
    click(cx, fx.win, ("compare-eye", ix));
    assert_hidden(cx, &fx, &["aaa", "bbb"]);
    assert!(present(cx, fx.win, ("compare-reveal", ix)));
}

#[gpui_kit::test]
fn the_same_cookie_on_both_sides_makes_no_row(cx: &mut TestAppContext) {
    let fx = live_pair(
        cx,
        &[("set-cookie", "sid=aaa")],
        &[("set-cookie", "sid=aaa")],
    );
    let c = fx.pane.read_with(cx, |p, _| p.comparison().unwrap());
    assert!(
        !c.headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("set-cookie")),
        "{:?}",
        c.headers
    );
    assert_eq!(header_row(cx, fx.win, "set-cookie"), None);
}

#[gpui_kit::test]
fn an_added_url_header_with_a_password_is_masked_until_revealed(cx: &mut TestAppContext) {
    let fx = live_pair(cx, &[], &[("location", "http://u:pw0rd@example.test/x")]);
    let ix = header_row(cx, fx.win, "location").expect("a Location row");
    assert!(label(cx, fx.win, ("compare-row", ix)).contains("example.test"));
    assert_hidden(cx, &fx, &["pw0rd"]);
    click(cx, fx.win, ("compare-eye", ix));
    assert!(label(cx, fx.win, ("compare-row", ix)).contains("u:pw0rd@example.test"));
}

pub(crate) fn kept(headers: &[(&str, &str)]) -> RequestResult {
    let mut response = Response::new(200).with_header("content-type", "application/json");
    for (name, value) in headers {
        response = response.with_header(*name, *value);
    }
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response.with_text_body("{}")),
        Vec::new(),
    )
}

// A new pane over two kept sends: the baseline comes from the store and the side on screen is
// restored, so neither has the real values the mask hid.
fn kept_pair(cx: &mut TestAppContext, sends: [RequestResult; 2]) -> Fx {
    let files = [
        ("opencollection.yml", ROOT.to_owned()),
        ("a.yml", request("a", "http://x/0")),
    ];
    let col = write_tree(
        &files
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let t0 = SystemTime::now() - Duration::from_secs(400);
    for (i, send) in sends.iter().enumerate() {
        store
            .save(
                &root,
                &ip("a.yml"),
                send,
                t0 + Duration::from_millis(i as u64),
            )
            .unwrap();
    }
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 2)
            && model.read_with(cx, |m, _| matches!(m.state(), SendState::Done(_)))
    });
    open_compare(cx, win, &pane);
    Fx {
        win,
        pane,
        _keep: Box::new((col, state, editor, ws)),
    }
}

#[gpui_kit::test]
fn kept_sides_show_no_false_change_and_lock_the_eye(cx: &mut TestAppContext) {
    let fx = kept_pair(
        cx,
        [
            kept(&[("set-cookie", "sid=aaa")]),
            kept(&[("set-cookie", "sid=bbb"), ("authorization", "Bearer t0ken")]),
        ],
    );
    assert_eq!(
        header_row(cx, fx.win, "set-cookie"),
        None,
        "no false change"
    );
    let ix = header_row(cx, fx.win, "authorization").expect("an Authorization row");
    assert_hidden(cx, &fx, &["t0ken", "aaa", "bbb"]);
    assert!(present(cx, fx.win, ("compare-reveal", ix)), "a locked eye");
    let before = label(cx, fx.win, ("compare-row", ix));
    click(cx, fx.win, ("compare-eye", ix));
    assert_eq!(
        label(cx, fx.win, ("compare-row", ix)),
        before,
        "nothing to reveal"
    );
    assert!(!present(cx, fx.win, ("compare-hide", ix)));
}

// The store is emptied and refilled behind the pane's back, so the seq of a send made live
// earlier now names a different kept send; its cached real values must never be used.
#[gpui_kit::test]
fn a_reused_seq_never_reveals_the_earlier_sends_values(cx: &mut TestAppContext) {
    use apinata_app::last_response::{Limits, SendExtra, StoreOp, enqueue};
    http_ready(cx);
    let server = mock_server(cx, template(&[("set-cookie", "sid=aaa")]));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let r = root.clone();
    model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
    finished(cx, &model);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    // Seq 0 now holds another send, kept masked.
    let other = std::sync::Arc::new(kept(&[("set-cookie", "sid=zzz")]));
    cx.update(|cx| {
        enqueue(StoreOp::Clear, cx);
        enqueue(
            StoreOp::Save {
                root: root.clone(),
                item: ip("a.yml"),
                result: other,
                secrets: Vec::new(),
                saved_at: SystemTime::now() - Duration::from_secs(60),
                limits: Limits::default(),
                extra: SendExtra::default(),
                kept: None,
            },
            cx,
        );
    });
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(server.reset());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(template(&[("set-cookie", "sid=bbb")]))
            .mount(&server),
    );
    let r = root.clone();
    model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
    finished(cx, &model);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 2));
    open_compare(cx, win, &pane);
    let fx = Fx {
        win,
        pane,
        _keep: Box::new((server, col, state, editor, ws)),
    };
    assert_hidden(cx, &fx, &["aaa", "zzz", "bbb"]);
    if let Some(ix) = header_row(cx, fx.win, "set-cookie") {
        click(cx, fx.win, ("compare-eye", ix));
        assert_hidden(cx, &fx, &["aaa", "zzz"]);
        assert!(
            !present(cx, fx.win, ("compare-hide", ix)),
            "the eye is locked"
        );
    }
}

// The locked eye explains itself in the Headers tab's words, which promise nothing a new send
// cannot keep: a send never gives a kept baseline its real value back.
#[gpui_kit::test]
fn the_locked_eye_says_why_in_the_headers_tabs_words(cx: &mut TestAppContext) {
    let fx = kept_pair(cx, [kept(&[]), kept(&[("authorization", "Bearer t0ken")])]);
    let ix = header_row(cx, fx.win, "authorization").expect("an Authorization row");
    assert_eq!(label(cx, fx.win, ("compare-eye", ix)), SCRUBBED_TIP);
    assert!(!SCRUBBED_TIP.contains("again"), "{SCRUBBED_TIP}");
}

// With enough rows to scroll and the pointer over the list, the overlay scrollbar shows at the
// list's right edge; the eye at the end of a row stays left of its track.
#[gpui_kit::test]
fn the_eye_stays_clear_of_the_scrollbar(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..60).map(|i| format!("x-h{i:02}")).collect();
    let first: Vec<(&str, &str)> = std::iter::once(("authorization", "Bearer one"))
        .chain(names.iter().map(|n| (n.as_str(), "a")))
        .collect();
    let second: Vec<(&str, &str)> = std::iter::once(("authorization", "Bearer two"))
        .chain(names.iter().map(|n| (n.as_str(), "b")))
        .collect();
    let fx = live_pair(cx, &first, &second);
    let ix = header_row(cx, fx.win, "authorization").expect("an Authorization row");
    cx.update_window(fx.win, |_, w, cx| {
        let list = w.find("compare-list").bounds();
        w.simulate_mouse_move(list.center(), cx);
    })
    .unwrap();
    frame(cx, fx.win);
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        let list = w.find("compare-list").bounds();
        let row = w.find(("compare-row", ix)).bounds();
        assert!(
            (0..60usize).any(|i| w.try_find(("compare-row", i)).is_none()),
            "the rows overflow the list, so it scrolls"
        );
        let eye = w.find(("compare-eye", ix)).bounds();
        // gpui-base 0.9.0 `scrollbar::WIDTH`, the overlay scrollbar's width.
        let track = list.right() - gpui_kit::px(16.);
        assert!(
            eye.right() <= track,
            "the eye {eye:?} ends left of the scrollbar track at {track:?} (row {row:?})"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_long_changed_header_ends_inside_its_row_and_tips_in_full(cx: &mut TestAppContext) {
    let (old, new) = ("a".repeat(300), "b".repeat(300));
    let fx = live_pair(cx, &[("x-long", &old)], &[("x-long", &new)]);
    let ix = header_row(cx, fx.win, "x-long").expect("an x-long row");
    let full = format!("x-long: {old} -> {new}");
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("compare-row", ix), cx)
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(800));
    frame(cx, fx.win);
    let tip = cx
        .update_window(fx.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("kv-cut-tooltip")
                .map(|t| t.label().unwrap_or_default().to_owned())
        })
        .unwrap();
    assert_eq!(tip, Some(full));
    cx.update_window(fx.win, |_, w, _| {
        let row = w.find(("compare-row", ix)).bounds();
        let text = w.find(("compare-text", ix)).bounds();
        assert!(text.right() <= row.right(), "{text:?} inside {row:?}");
    })
    .unwrap();
}
