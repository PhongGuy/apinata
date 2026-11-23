//! The Compare tab in a short window with the console dock open, and in a compact pane.

use crate::support;

use apinata_app::compare_source::CompareSource;
use apinata_app::keymap::ToggleConsole;
use apinata_app::response_pane::ResponsePane;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, ScrollDelta, TestAppContext, VisualTestContext, point, px, size,
};
use support::*;
use wiremock::ResponseTemplate;

const BODY: &str = "{\"items\":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30]}";

// Sends `a.yml` against a local server with the dock open, then compares the response with a
// saved example whose every array element differs, so the row list is longer than the pane.
fn compare_open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (Started, Entity<ResponsePane>, wiremock::MockServer) {
    sent_with(cx, (width, height), 200, "golden", true)
}

// As `compare_open`, answering with `status` and naming the example `example`; without
// `compare` the response is left on its Body tab.
fn sent_with(
    cx: &mut TestAppContext,
    (width, height): (f32, f32),
    status: u16,
    example: &str,
    compare: bool,
) -> (Started, Entity<ResponsePane>, wiremock::MockServer) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(status).set_body_string(BODY));
    let old_body = BODY.replace(|c: char| c.is_ascii_digit(), "0");
    let a = format!(
        "{}examples:\n  - name: {example}\n    response:\n      status: 200\n      body:\n        type: json\n        data: '{old_body}'\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    std::mem::forget(col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(height)));
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-status"));
    let pane = editor_of(cx, &s, tab).read_with(cx, |e, _| e.response().clone());
    if compare {
        pane.update(cx, |p, cx| p.compare_with(CompareSource::Example(0), cx));
        wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    }
    frame(cx, s.win);
    (s, pane, server)
}

#[gpui_kit::test]
fn the_compare_tab_scrolls_inside_a_short_window_with_the_dock_open(cx: &mut TestAppContext) {
    let (s, _pane, _server) = compare_open(cx, 1000., 500.);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let response = w.find("response").bounds();
        let summary = w.find("compare-summary").bounds();
        let list = w.find("compare-list").bounds();
        let dock = w.find("console-dock").bounds();
        // At 1000 px the pane may be compact; then the button lives in the More menu.
        let id = if w.try_find("response-compare").is_some() {
            "response-compare"
        } else {
            "response-more"
        };
        let button = w.find(id).bounds();
        assert!(
            button.left() >= response.left() && button.right() <= response.right(),
            "{id} {button:?} clipped by {response:?}"
        );
        assert!(
            summary.left() >= response.left()
                && summary.right() <= response.right()
                && summary.top() >= response.top()
        );
        assert!(
            list.top() >= summary.bottom(),
            "list {list:?} under summary {summary:?}"
        );
        assert!(
            list.bottom() <= response.bottom() && list.bottom() <= dock.top() + px(1.),
            "list {list:?} overlaps dock {dock:?}"
        );
        assert!(
            list.size.height >= px(60.),
            "starting a compare gave the list room: {list:?} response {response:?} summary {summary:?} dock {dock:?}"
        );
    })
    .unwrap();
    let before = cx
        .update_window(s.win, |_, w, _| w.find("compare-summary").bounds())
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.scroll(
            "compare-list",
            ScrollDelta::Pixels(point(px(0.), px(-200.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
    let after = cx
        .update_window(s.win, |_, w, _| w.find("compare-summary").bounds())
        .unwrap();
    assert_eq!(before, after, "scrolling moves rows, not the summary line");
}

#[gpui_kit::test]
fn the_compare_tab_keeps_its_controls_inside_a_compact_pane(cx: &mut TestAppContext) {
    let (s, _pane, _server) = compare_open(cx, 520., 700.);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let response = w.find("response").bounds();
        for id in ["compare-source", "compare-summary", "compare-close"] {
            let b = w.find(id).bounds();
            assert!(
                b.left() >= response.left() && b.right() <= response.right(),
                "{id} {b:?} outside {response:?}"
            );
        }
    })
    .unwrap();
}

// At 1280x800 beside the sidebar, with Compare chosen and the request pane at its top limit,
// the status facts give way to the tabs: every tab shows whole, none scrolled out at the left.
#[gpui_kit::test]
fn every_response_tab_shows_whole_with_compare_chosen_at_1280(cx: &mut TestAppContext) {
    let (s, _pane, _server) = compare_open(cx, 1280., 800.);
    // Opening the open request again only brings its tab forward.
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let ed = editor_of(cx, &s, tab);
    let split = ed.read_with(cx, |e, _| e.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, px(10.), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let strip = w.find("response-tab-strip").bounds();
        let tabs: Vec<_> = (0..16usize)
            .filter_map(|ix| w.try_find(("response-tab", ix)).map(|t| t.bounds()))
            .collect();
        assert!(tabs.len() >= 3, "{tabs:?}");
        for t in &tabs {
            assert!(
                t.left() >= strip.left() - px(0.5) && t.right() <= strip.right() + px(0.5),
                "tab {t:?} cut by the strip {strip:?}; tabs {tabs:?}"
            );
        }
    })
    .unwrap();
}

// While a response streams, the tab strip keeps its tabs' width beside the streaming facts:
// with the last tab chosen, every tab shows whole wherever the tabs alone fit the pane.
#[gpui_kit::test]
fn every_response_tab_shows_whole_while_streaming(cx: &mut TestAppContext) {
    use apinata_app::response_pane::ResponseTab;
    use support::sse_server::{Script, event};
    http_ready(cx);
    let events = (0..3).map(|i| event(&format!("event {i}"))).collect();
    let server = sse(cx, Script::sse(events));
    let col = sse_collection(&server);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let ed = editor_of(cx, &s, tab);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 3))
    });
    let pane = ed.read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    // At 200% the facts beside the strip are wide enough to squeeze it below its tabs.
    cx.update(|cx| {
        let p = cx.global::<apinata_app::app::AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = 2.0));
    });
    for width in [1280., 1000., 800., 640.] {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(800.)));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            assert!(
                w.try_find("response-streaming").is_some(),
                "{width}: not streaming"
            );
            let response = w.find("response").bounds();
            let strip = w.find("response-tab-strip").bounds();
            let tabs: Vec<_> = (0..16usize)
                .filter_map(|ix| w.try_find(("response-tab", ix)).map(|t| t.bounds()))
                .collect();
            let span = tabs.last().unwrap().right() - tabs[0].left();
            if span > response.size.width {
                return;
            }
            for t in &tabs {
                assert!(
                    t.left() >= strip.left() - px(0.5) && t.right() <= strip.right() + px(0.5),
                    "{width}: tab {t:?} cut by the strip {strip:?}; tabs {tabs:?}"
                );
            }
        })
        .unwrap();
    }
}

// Shows the response beside the request at `w` x `h` and `zoom`, then lays out three times.
fn sbs(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, zoom: f32) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| {
                st.response_orientation = Orientation::Horizontal;
                st.zoom = zoom;
            })
        });
    });
    cx.run_until_parked();
    for _ in 0..3 {
        frame(cx, s.win);
    }
}

// The request pane width its URL row needs at `zoom`.
fn url_need(zoom: f32) -> gpui_kit::Pixels {
    use apinata_app::style::*;
    at_zoom(URL_FIELD_MIN, zoom)
        + at_zoom(METHOD_SELECT, zoom).max(at_zoom(CUSTOM_METHOD, zoom))
        + at_zoom(SPACE_2, zoom)
        + at_zoom(REQUEST_BUTTONS_WIDTH, zoom)
        + at_zoom(URL_ROW_SLACK, zoom)
}

fn is_stacked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, _| {
        w.find("response").bounds().top() >= w.find("request-column").bounds().bottom() - px(1.)
    })
    .unwrap()
}

// Asserts every id lies horizontally within `pane`, with half a pixel of rounding.
fn all_within(w: &mut gpui_kit::Window, pane: &'static str, ids: &[&'static str]) {
    let pane_b = w.find(pane).bounds();
    for id in ids {
        let b = w.find(*id).bounds();
        assert!(
            b.left() >= pane_b.left() - px(0.5) && b.right() <= pane_b.right() + px(0.5),
            "{id} {b:?} leaves {pane} {pane_b:?}"
        );
    }
}

// Side by side with Compare chosen, a drag toward the response stops where Compare, its
// neighbour, the status chip and the close button still show whole; toward the request it
// still stops at the URL row.
#[gpui_kit::test]
fn dragging_right_keeps_the_compare_header_whole_side_by_side(cx: &mut TestAppContext) {
    for (w, zoom) in [(1280., 1.0), (2400., 2.0)] {
        let (s, _pane, _server) = compare_open(cx, 1280., 800.);
        let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
        sbs(cx, &s, w, 800., zoom);
        let ed = editor_of(cx, &s, tab);
        let split = ed.read_with(cx, |e, _| e.split().clone());
        let drag = |cx: &mut TestAppContext, to: gpui_kit::Pixels| {
            cx.update_window(s.win, |_, w, cx| {
                split.update(cx, |sp, cx| sp.resize_panel(0, to, w, cx))
            })
            .unwrap();
            for _ in 0..3 {
                frame(cx, s.win);
            }
        };
        drag(cx, px(5000.));
        assert!(!is_stacked(cx, &s), "{w}@{zoom}: stacked after the drag");
        cx.update_window(s.win, |_, win, cx| {
            win.render_frame(cx);
            let response = win.find("response").bounds();
            let strip = win.find("response-tab-strip").bounds();
            let tabs: Vec<_> = (0..16usize)
                .filter_map(|ix| win.try_find(("response-tab", ix)).map(|t| t.bounds()))
                .collect();
            assert!(tabs.len() >= 3, "{tabs:?}");
            let fades: Vec<_> = [
                "response-tab-strip-fade-left",
                "response-tab-strip-fade-right",
            ]
            .into_iter()
            .filter_map(|id| win.try_find(id).map(|f| f.bounds()))
            .collect();
            for t in &tabs[tabs.len() - 2..] {
                assert!(
                    t.left() >= strip.left() - px(0.5) && t.right() <= strip.right() + px(0.5),
                    "{w}@{zoom}: tab {t:?} cut by the strip {strip:?}"
                );
                assert!(
                    t.left() >= response.left() - px(0.5)
                        && t.right() <= response.right() + px(0.5),
                    "{w}@{zoom}: tab {t:?} outside the response {response:?}"
                );
                for f in &fades {
                    assert!(
                        t.right() <= f.left() + px(1.) || t.left() >= f.right() - px(1.),
                        "{w}@{zoom}: tab {t:?} under a fade {f:?}; strip {strip:?}"
                    );
                }
            }
            all_within(win, "response", &["response-status", "compare-close"]);
            // The floor counts the overflow button as a constant; it must cover the button.
            if let Some(menu) = win.try_find("response-tab-overflow") {
                let room =
                    apinata_app::style::at_zoom(apinata_app::style::TAB_OVERFLOW_WIDTH, zoom);
                assert!(
                    menu.bounds().size.width <= room,
                    "{w}@{zoom}: {menu:?} over {room:?}"
                );
            }
        })
        .unwrap();
        drag(cx, px(10.));
        assert!(
            !is_stacked(cx, &s),
            "{w}@{zoom}: stacked after the drag left"
        );
        let request = split.read_with(cx, |sp, _| sp.sizes()[0]);
        assert!(
            request >= url_need(zoom) - px(1.),
            "{w}@{zoom}: {request:?} < {:?}",
            url_need(zoom)
        );
    }
}

// A split 20 px short of the URL row and the measured response minimum stacks; 20 px more
// than both shows side by side. The split is the tab area less the editor's insets.
#[gpui_kit::test]
fn a_room_short_of_both_minimums_stacks(cx: &mut TestAppContext) {
    use apinata_app::request_editor::TabAreaRoom;
    use apinata_app::style::{PANE_INSET, SPACE_2, at_zoom};
    for zoom in [1.0, 2.0] {
        let (s, pane, _server) = compare_open(cx, 1280., 800.);
        sbs(cx, &s, 1280., 800., zoom);
        // At 200% a 1280 px window cannot hold both panes.
        assert_eq!(is_stacked(cx, &s), zoom > 1.5, "1280x800 at {zoom}");
        let need = pane
            .read_with(cx, |p, _| p.min_width())
            .expect("the response minimum is measured after a layout");
        let both = url_need(zoom) + need + at_zoom(SPACE_2, zoom) + at_zoom(PANE_INSET, zoom) * 2.;
        // A narrow zoomed window hides the sidebar, so it is measured in a wide one.
        sbs(cx, &s, 2400., 800., zoom);
        let sidebar = px(2400.) - cx.update(|cx| cx.global::<TabAreaRoom>().0);
        for (delta, want_stacked) in [(-20., true), (20., false)] {
            let width = f32::from(both + sidebar) + delta;
            sbs(cx, &s, width, 800., zoom);
            let now = cx.update(|cx| cx.global::<TabAreaRoom>().0);
            assert!(
                (now - (both + px(delta))).abs() < px(1.),
                "{zoom}: room {now:?} for {both:?} {delta}"
            );
            assert_eq!(
                is_stacked(cx, &s),
                want_stacked,
                "{zoom}: room {now:?}, url {:?} + response {need:?}",
                url_need(zoom)
            );
        }
    }
}

// Drags the divider of the editor showing `a.yml` toward the response, then lays out thrice.
fn drag_right(cx: &mut TestAppContext, s: &Started) {
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let split = editor_of(cx, s, tab).read_with(cx, |e, _| e.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, px(5000.), w, cx))
    })
    .unwrap();
    for _ in 0..3 {
        frame(cx, s.win);
    }
}

// An example with a long name keeps the comparison's source menu and close button inside a
// response dragged to its limit, and the room still shows both panes.
#[gpui_kit::test]
fn a_long_example_name_keeps_the_compare_controls_whole_side_by_side(cx: &mut TestAppContext) {
    let name = "the response saved after the second checkout run with every discount applied";
    for (w, zoom) in [(1280., 1.0), (2400., 2.0)] {
        let (s, _pane, _server) = sent_with(cx, (1280., 800.), 200, name, true);
        sbs(cx, &s, w, 800., zoom);
        drag_right(cx, &s);
        assert!(!is_stacked(cx, &s), "{w}@{zoom}: stacked");
        cx.update_window(s.win, |_, win, _| {
            all_within(win, "response", &["compare-source", "compare-close"]);
            let cap =
                apinata_app::style::at_zoom(apinata_app::style::COMPARE_SOURCE_MAX_WIDTH, zoom);
            let source = win.find("compare-source").bounds();
            assert!(
                source.size.width <= cap + px(0.5),
                "{w}@{zoom}: {source:?} over {cap:?}"
            );
        })
        .unwrap();
    }
}

// The longest standard reason phrase keeps the status chip whole in a response dragged to
// its limit.
#[gpui_kit::test]
fn a_long_status_keeps_its_chip_whole_side_by_side(cx: &mut TestAppContext) {
    for (w, zoom) in [(1280., 1.0), (2400., 2.0)] {
        let (s, _pane, _server) = sent_with(cx, (1280., 800.), 511, "golden", false);
        sbs(cx, &s, w, 800., zoom);
        drag_right(cx, &s);
        assert!(!is_stacked(cx, &s), "{w}@{zoom}: stacked");
        cx.update_window(s.win, |_, win, _| {
            all_within(win, "response", &["response-status"])
        })
        .unwrap();
    }
}

// A long example name is cut on the source button, so hovering it shows the whole label.
#[gpui_kit::test]
fn the_compare_source_tooltip_shows_the_whole_label(cx: &mut TestAppContext) {
    let name = "a very long example name that the source button cannot show whole";
    let (s, _pane, _server) = sent_with(cx, (1280., 800.), 200, name, true);
    let label = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    };
    let whole = label(cx, "compare-source").expect("the source button is named");
    assert!(whole.contains(name), "{whole}");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover("compare-source", cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(label(cx, "compare-source-tooltip"), Some(whole));
}
