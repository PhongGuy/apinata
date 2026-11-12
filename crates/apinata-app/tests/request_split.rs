//! The request and the response share a resizable split: even at first, never smaller
//! than the minimum.

use crate::support;

use apinata_app::style::PANE_MIN;
use gpui_kit::AppContext as _;
use gpui_kit::{TestAppContext, px};
use support::*;

#[gpui_kit::test]
fn request_and_response_split_evenly(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    give_result(cx, &ed);
    frame(cx, win);
    let split = ed.read_with(cx, |e, _| e.split().clone());
    let sizes = split.read_with(cx, |s, _| s.sizes().clone());
    assert_eq!(sizes.len(), 2);
    assert!((sizes[0] - sizes[1]).abs() <= px(1.), "{sizes:?}");
    assert!(present(cx, win, "response"));
    assert!(present(cx, win, "url"));
    assert!(present(cx, win, "request-column"));
}

#[gpui_kit::test]
fn neither_panel_shrinks_below_the_minimum(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    give_result(cx, &ed);
    frame(cx, win);
    let split = ed.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |s, _| s.sizes()[0] + s.sizes()[1]);
    cx.update_window(win, |_, w, cx| {
        split.update(cx, |s, cx| s.resize_panel(0, px(10.), w, cx))
    })
    .unwrap();
    frame(cx, win);
    let first = split.read_with(cx, |s, _| s.sizes()[0]);
    assert!(first >= PANE_MIN, "{first:?}");
    cx.update_window(win, |_, w, cx| {
        split.update(cx, |s, cx| s.resize_panel(0, total, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(split.read_with(cx, |s, _| s.sizes()[1]) >= PANE_MIN);
}

// At 200% a request pane dragged to its minimum still shows the whole section bar, and the
// body below it keeps its minimum height.
#[gpui_kit::test]
fn the_section_bar_shows_at_the_minimum_split_at_200_percent(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(800.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = 2.0));
    });
    cx.run_until_parked();
    frame(cx, s.win);
    let ed = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .expect("an editor")
            .clone()
    });
    give_result(cx, &ed);
    frame(cx, s.win);
    let split = ed.read_with(cx, |e, _| e.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |s, cx| s.resize_panel(0, px(10.), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let column = w.find("request-column").bounds();
        let strip = w.find("section-strip").bounds();
        assert!(
            strip.top() >= column.top() - px(0.5) && strip.bottom() <= column.bottom() + px(0.5),
            "section bar {strip:?} outside {column:?}"
        );
        let body = w.find("section-body").bounds();
        assert!(body.size.height >= PANE_MIN - px(0.5), "{body:?}");
    })
    .unwrap();
}

// Shows the window at 2400x800 with the response beside the request at 200%, where each
// pane is narrower than its fixed-width controls laid out in one row. Narrower windows stack
// at this zoom, since the request pane could not hold the URL at 16 rem.
fn side_by_side_at_200(cx: &mut TestAppContext, s: &Started) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(2400.), px(800.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.response_orientation = Orientation::Horizontal;
                s.zoom = 2.0;
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
}

// Sends `a.yml` once against a JSON server, so the response header and body toolbar show.
fn sent(
    cx: &mut TestAppContext,
) -> (
    Started,
    apinata_app::tabs::TabId,
    tempfile::TempDir,
    wiremock::MockServer,
) {
    use gpui_kit::test::TestWindowExt as _;
    http_ready(cx);
    let json = wiremock::ResponseTemplate::new(200).set_body_raw("{\"id\":42}", "application/json");
    let server = mock_server(cx, json);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "body-save"));
    (s, tab, col, server)
}

// Asserts every id lies horizontally within `pane`, with half a pixel of rounding.
fn all_within(w: &mut gpui_kit::Window, pane: &'static str, ids: &[&'static str]) {
    use gpui_kit::test::TestWindowExt as _;
    let pane_b = w.find(pane).bounds();
    for id in ids {
        let b = w.find(*id).bounds();
        assert!(
            b.left() >= pane_b.left() - px(0.5) && b.right() <= pane_b.right() + px(0.5),
            "{id} {b:?} leaves {pane} {pane_b:?}"
        );
    }
}

#[gpui_kit::test]
fn the_url_row_fits_its_pane_side_by_side_at_200_percent(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // At 100% in the default window the row stays one line.
    cx.update_window(s.win, |_, w, _| {
        let url = w.find("url").bounds();
        let send = w.find("send-slot").bounds();
        assert_eq!(url.top(), send.top(), "url {url:?} send {send:?}");
    })
    .unwrap();
    side_by_side_at_200(cx, &s);
    cx.update_window(s.win, |_, w, _| {
        all_within(w, "request-column", &["method", "url", "send-slot"]);
        let url = w.find("url").bounds();
        let floor = apinata_app::style::at_zoom(apinata_app::style::URL_FIELD_MIN, 2.0);
        assert!(
            url.size.width >= floor - px(0.5),
            "url too narrow to use: {url:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_response_header_fits_its_pane_side_by_side_at_200_percent(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (s, _tab, _col, _server) = sent(cx);
    side_by_side_at_200(cx, &s);
    cx.update_window(s.win, |_, w, _| {
        all_within(
            w,
            "response",
            &[
                "response-tab-strip",
                "response-status",
                "response-time",
                "response-size",
                "response-history",
                "response-more",
            ],
        );
        let strip = w.find("response-tab-strip").bounds();
        assert!(strip.size.width >= px(100.), "tabs squeezed out: {strip:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_body_toolbar_fits_its_pane_side_by_side_at_200_percent(cx: &mut TestAppContext) {
    let (s, _tab, _col, _server) = sent(cx);
    side_by_side_at_200(cx, &s);
    cx.update_window(s.win, |_, w, _| {
        all_within(
            w,
            "response",
            &[
                "body-filter",
                "body-view",
                "body-edit",
                "body-pretty",
                "body-raw",
                "body-format",
            ],
        );
    })
    .unwrap();
}

// Side by side at 200% with the response squeezed under its header (its floor pinned to the
// panel minimum, since the measured floor keeps the toolbar whole) the Pretty and Raw pair is
// wider than the pane and overhangs the request; the pane must clip the overhang in paint
// and in hit testing, so a click over the request never reaches Pretty.
#[gpui_kit::test]
fn the_response_is_clipped_to_its_panel_at_the_minimum_split_at_200_percent(
    cx: &mut TestAppContext,
) {
    use apinata_app::body_view::BodyMode;
    use gpui_kit::test::TestWindowExt as _;
    let (s, tab, _col, _server) = sent(cx);
    side_by_side_at_200(cx, &s);
    let ed = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .expect("an editor")
            .clone()
    });
    ed.read_with(cx, |e, cx| {
        e.response().read(cx).pin_min_width(Some(PANE_MIN))
    });
    frame(cx, s.win);
    let split = ed.read_with(cx, |e, _| e.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |s, cx| s.resize_panel(0, px(2000.), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("body-raw", cx))
        .unwrap();
    frame(cx, s.win);
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    assert_eq!(body.read_with(cx, |b, _| b.mode()), BodyMode::Raw);
    let (column, pretty) = cx
        .update_window(s.win, |_, w, _| {
            (w.find("request-column").bounds(), w.find("body-pretty"))
        })
        .unwrap();
    let b = pretty.bounds();
    assert!(
        b.left() < column.right(),
        "the case needs Pretty over the request {column:?}: {b:?}"
    );
    assert!(pretty.visible(), "Pretty is clipped out of reach: {b:?}");
    // Click the part of Pretty that lies over the request column.
    let offset = gpui_kit::point((column.right() - b.left()) / 2., b.size.height / 2.);
    cx.update_window(s.win, |_, w, cx| w.click_at("body-pretty", offset, cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(
        body.read_with(cx, |b, _| b.mode()),
        BodyMode::Raw,
        "a click over the request {column:?} reached Pretty at {b:?}"
    );
}

// A window too short for two minimum panels stacked: each panel's minimum gives way to
// what fits, so the response is not pushed under the status bar.
#[gpui_kit::test]
fn both_panels_stay_above_the_status_bar_when_the_room_is_short(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, tab);
    give_result(cx, &ed);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(360.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.zoom = 2.0;
                s.response_orientation = Orientation::Vertical;
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let status = w.find("status-bar").bounds();
        for id in ["request-column", "response"] {
            let b = w.find(id).bounds();
            assert!(
                b.bottom() <= status.top() + px(0.5),
                "{id} {b:?} runs under the status bar {status:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_url_row_buttons_wrap_below_the_url_in_a_narrow_pane_at_200_percent(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/changed"), cx)
    });
    frame(cx, s.win);
    side_by_side_at_200(cx, &s);
    // The side-by-side pane no longer gets this narrow; the buttons still wrap below the
    // URL where even the stacked pane is short of the row.
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(640.), px(800.)));
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        all_within(
            w,
            "request-column",
            &[
                "method",
                "url",
                "save-request",
                "send-slot",
                "generate-code",
            ],
        );
        // The URL takes the rest of its line and is never narrower than the method.
        let url = w.find("url").bounds();
        let method = w.find("method").bounds();
        let column = w.find("request-column").bounds();
        let gap = apinata_app::style::at_zoom(apinata_app::style::SPACE_2, 2.0);
        assert!(
            url.size.width >= method.size.width && url.right() >= column.right() - gap,
            "url {url:?} takes the rest of its line in {column:?}"
        );
        let save = w.find("save-request").bounds();
        let send = w.find("send-slot").bounds();
        let code = w.find("generate-code").bounds();
        // Save wraps with Send and Generate code, first in that group, so the URL keeps the line.
        assert!(
            save.top() >= url.bottom() && send.top() >= url.bottom(),
            "the group wraps below the url {url:?}: save {save:?} send {send:?}"
        );
        assert!(
            save.right() <= send.left() && send.right() <= code.left(),
            "save {save:?} send {send:?} code {code:?}"
        );
    })
    .unwrap();
}

type Rows = std::collections::HashMap<&'static str, gpui_kit::Bounds<gpui_kit::Pixels>>;

// Drags the split to its top limit, then shrinks the window to `shrink` px tall if given, and
// returns the bounds of the request rows and the response controls by id, both panel sizes,
// and the editor row plus pane gutter in px.
fn at_top_limit(
    cx: &mut TestAppContext,
    horizontal: bool,
    zoom: f32,
    banner: bool,
    shrink: Option<f32>,
) -> (Rows, Vec<gpui_kit::Pixels>, gpui_kit::Pixels) {
    use apinata_app::app::AppState;
    use apinata_app::command::Command;
    use apinata_app::settings::Orientation;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let (w, h) = if horizontal {
        (2400., 800.)
    } else {
        (1280., 800.)
    };
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.zoom = zoom;
                if horizontal {
                    s.response_orientation = Orientation::Horizontal;
                }
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    let ed = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .expect("an editor")
            .clone()
    });
    give_result(cx, &ed);
    frame(cx, s.win);
    if banner {
        let path = col.path().join("a.yml");
        type_at_end(cx, s.win, None, "url", "/mine");
        cx.run_until_parked();
        std::fs::write(&path, request("a", "http://x/disk")).unwrap();
        s.ws.update(cx, |w, cx| {
            w.send(&s.root, Command::Reload(vec![path.clone()]), cx)
        });
        frame(cx, s.win);
        assert!(present(cx, s.win, "banner-changed"));
    }
    let split = ed.read_with(cx, |e, _| e.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |s, cx| s.resize_panel(0, px(10.), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    if let Some(h) = shrink {
        gpui_kit::VisualTestContext::from_window(s.win, cx)
            .simulate_resize(gpui_kit::size(px(w), px(h)));
        frame(cx, s.win);
    }
    let sizes = split.read_with(cx, |s, _| s.sizes().to_vec());
    let mut ids = vec![
        "request-column",
        "breadcrumb",
        "method",
        "url",
        "send-slot",
        "section-strip",
        "section-body",
        "response",
    ];
    if banner {
        ids.push("banner-changed");
    }
    cx.update_window(s.win, |_, w, _| {
        let mut b: std::collections::HashMap<_, _> =
            ids.iter().map(|&id| (id, w.find(id).bounds())).collect();
        b.insert("tab", w.find(("response-tab", 0usize)).bounds());
        let row = apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w)
            + apinata_app::style::resolve(apinata_app::style::SPACE_2, w);
        (b, sizes, row)
    })
    .unwrap()
}

// Asserts the request rows run top to bottom without overlapping: breadcrumb, banner (when
// shown), URL row, section strip, section body.
fn assert_rows_stack(b: &Rows, banner: bool, case: &str) {
    let mut layers = vec![&["breadcrumb"][..]];
    if banner {
        layers.push(&["banner-changed"]);
    }
    layers.extend([
        &["method", "url", "send-slot"][..],
        &["section-strip"],
        &["section-body"],
    ]);
    for pair in layers.windows(2) {
        for upper in pair[0] {
            for lower in pair[1] {
                let (u, l) = (b[upper], b[lower]);
                assert!(
                    u.bottom() <= l.top() + px(0.5),
                    "{case}: {upper} {u:?} overlaps {lower} {l:?}"
                );
            }
        }
    }
}

// Each case: the request rows sit inside their column in order without overlapping, the first
// response tab sits inside its pane, and a stacked request pane is as small as its rows allow.
#[gpui_kit::test]
fn the_top_limit_keeps_breadcrumb_method_url_and_response_tabs_visible(cx: &mut TestAppContext) {
    for horizontal in [false, true] {
        for zoom in [1.0, 2.0] {
            for banner in [false, true] {
                let (b, sizes, row) = at_top_limit(cx, horizontal, zoom, banner, None);
                let first = sizes[0];
                let case = format!("h={horizontal} z={zoom} banner={banner}");
                let eps = px(0.5);
                let column = b["request-column"];
                for name in ["breadcrumb", "method", "url"] {
                    let c = b[name];
                    assert!(
                        c.top() >= column.top() - eps && c.bottom() <= column.bottom() + eps,
                        "{case}: {name} {c:?} clipped by {column:?}"
                    );
                }
                assert_rows_stack(&b, banner, &case);
                let (tab, response) = (b["tab"], b["response"]);
                assert!(
                    tab.top() >= response.top() - eps
                        && tab.bottom() <= response.bottom() + eps
                        && tab.left() >= response.left() - eps
                        && tab.right() <= response.right() + eps,
                    "{case}: tab {tab:?} clipped by {response:?}"
                );
                // Stacked, the pane holds every row above the body plus one editor row and
                // its gutter, and no more than a few px over that.
                if !horizontal {
                    let need = b["section-body"].top() - b["breadcrumb"].top() + row;
                    assert!(
                        first >= need - eps && first <= need + px(4.),
                        "{case}: request pane {first:?}, rows need {need:?}"
                    );
                }
            }
        }
    }
}

// A window shrunk after the split sits at its top limit: the rows still stack without
// overlapping and each pane keeps its floor. Where the rows plus RESPONSE_KEPT no longer fit
// the new room, the request minimum gives way so the response keeps RESPONSE_KEPT.
#[gpui_kit::test]
fn a_window_shrunk_at_the_top_limit_keeps_rows_apart_and_the_response_floor(
    cx: &mut TestAppContext,
) {
    use apinata_app::style::RESPONSE_KEPT;
    // (zoom, window height, whether the cap binds in that room)
    for (zoom, height, capped) in [
        (1.0, 520., false),
        (1.0, 480., true),
        (2.0, 640., true),
        (2.0, 560., false),
    ] {
        let (b, sizes, _) = at_top_limit(cx, false, zoom, false, Some(height));
        let case = format!("z={zoom} h={height}");
        assert_rows_stack(&b, false, &case);
        let floor = PANE_MIN.min((sizes[0] + sizes[1]) * 0.25) - px(0.5);
        assert!(sizes[0] >= floor && sizes[1] >= floor, "{case}: {sizes:?}");
        if capped {
            assert!(
                (sizes[1] - RESPONSE_KEPT).abs() <= px(0.5),
                "{case}: response {:?}, want {RESPONSE_KEPT:?}; sizes {sizes:?}",
                sizes[1]
            );
        }
    }
}

// Stacked, the response's first tab starts where the request content does, a ring's room
// inside the response pane's clipping edge.
#[gpui_kit::test]
fn the_response_content_lines_up_with_the_request_content(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    use gpui_kit::test::TestWindowExt as _;
    let (s, _tab, _col, _server) = sent(cx);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let method = w.find("method").bounds();
        let response = w.find("response").bounds();
        let tab = w.find(("response-tab", 0usize)).bounds();
        // The kit's strip sets its first tab 2 px in, as it did before the ring room.
        assert!(
            (tab.left() - method.left()).abs() <= px(2.),
            "{tab:?} against {method:?}"
        );
        assert!(
            tab.left() - FOCUS_RING_ROOM >= response.left(),
            "{tab:?} in {response:?}"
        );
        let body = w.find("response-body").bounds();
        assert!(
            (body.left() - method.left()).abs() < px(1.),
            "{body:?} against {method:?}"
        );
    })
    .unwrap();
}
