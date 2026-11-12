//! The wide response header stays one row: the address gives way first, then the tab strip
//! scrolls, and below that the pane goes compact, so the body never jumps.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Pixels, SharedString, TestAppContext,
    VisualTestContext, px, size,
};
use support::*;
use wiremock::ResponseTemplate;
use wiremock::matchers::path;

// The busiest response header: the request redirects once and both hops set a cookie, so the
// response has a timing bar, an address and badges on Headers, Cookies and Timeline.
fn redirected(cx: &mut TestAppContext, width: f32) -> (Started, wiremock::MockServer) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    let hop = ResponseTemplate::new(302)
        .insert_header("location", "/final")
        .insert_header("set-cookie", "a=1; Path=/");
    let last = ResponseTemplate::new(200)
        .insert_header("set-cookie", "b=2; Path=/")
        .set_body_raw("{\"ok\":true}", "application/json");
    handle.block_on(
        wiremock::Mock::given(path("/ok"))
            .respond_with(hop)
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(path("/final"))
            .respond_with(last)
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    std::mem::forget(col);
    resize(cx, &s, width);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-address"));
    settle(cx, s.win);
    (s, server)
}

fn resize(cx: &mut TestAppContext, s: &Started, width: f32) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(800.)));
    settle(cx, s.win);
}

// The probes measure in one frame and the pane re-lays out in the next.
fn settle(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for _ in 0..3 {
        frame(cx, win);
    }
}

fn bounds(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

// The header facts and buttons on offer in the wide shape, left to right.
const FACTS: [&str; 9] = [
    "response-status",
    "response-time",
    "response-timing",
    "response-size",
    "response-address",
    "response-save-example",
    "response-export-har",
    "response-history",
    "response-compare",
];

fn on_offer(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<&'static str> {
    FACTS
        .into_iter()
        .filter(|id| present(cx, win, *id))
        .collect()
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// Every fact and button is centred inside the strip's band, so the header is one row.
fn assert_one_row(cx: &mut TestAppContext, win: AnyWindowHandle) {
    let strip = bounds(cx, win, "response-tab-strip");
    for id in on_offer(cx, win) {
        let b = bounds(cx, win, id);
        assert!(
            b.center().y > strip.top() && b.center().y < strip.bottom(),
            "{id} {b:?} is off the strip's row {strip:?}"
        );
    }
}

#[gpui_kit::test]
fn at_1280_the_header_is_one_row(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1280.);
    assert!(
        !present(cx, s.win, "response-more"),
        "1280 px is a wide pane"
    );
    assert_eq!(
        on_offer(cx, s.win),
        FACTS,
        "the busiest header offers every fact"
    );
    assert_one_row(cx, s.win);
}

// The row never wraps at any width, so the content under it keeps its place while the
// window is dragged across the width the old row wrapped at.
#[gpui_kit::test]
fn the_body_keeps_its_place_across_widths(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1280.);
    let top = bounds(cx, s.win, "body-filter").top();
    for width in (1240..=1440).step_by(8) {
        resize(cx, &s, width as f32);
        for _ in 0..3 {
            let now = bounds(cx, s.win, "body-filter").top();
            assert_eq!(now, top, "the body moved at {width} px");
        }
    }
}

// Between the compact threshold and the full row the address gives way: still one row,
// the address cut with its whole text in a tooltip, every button whole inside the pane.
#[gpui_kit::test]
fn a_narrower_pane_cuts_the_address_and_keeps_every_button(cx: &mut TestAppContext) {
    let (s, server) = redirected(cx, 2400.);
    let natural = bounds(cx, s.win, "response-address").size.width;
    resize(cx, &s, 1130.);
    assert!(!present(cx, s.win, "response-more"), "still a wide pane");
    assert_eq!(
        on_offer(cx, s.win),
        FACTS,
        "every fact and button is on offer"
    );
    assert_one_row(cx, s.win);
    let address = bounds(cx, s.win, "response-address");
    assert!(
        address.size.width < natural,
        "{address:?} is not cut from {natural:?}"
    );
    let pane = bounds(cx, s.win, "response");
    for id in on_offer(cx, s.win) {
        let b = bounds(cx, s.win, id);
        assert!(
            b.left() >= pane.left() && b.right() <= pane.right() + px(0.5),
            "{id} {b:?} clipped by {pane:?}"
        );
    }
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover("response-address", cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(
        label(cx, s.win, "response-address-tooltip"),
        Some(server.address().to_string())
    );
}

// Narrower still, once the address is cut away entirely the tabs scroll behind their
// fades, and only then does the pane fold to compact.
#[gpui_kit::test]
fn the_tabs_scroll_before_the_pane_goes_compact(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1280.);
    let mut scrolled = false;
    for width in (880..=1280).rev().step_by(8) {
        resize(cx, &s, width as f32);
        if present(cx, s.win, "response-more") {
            break;
        }
        assert_one_row(cx, s.win);
        if present(cx, s.win, "response-tab-overflow") {
            let address = bounds(cx, s.win, "response-address").size.width;
            assert_eq!(
                address,
                px(0.),
                "the tabs scroll at {width} px beside an address"
            );
            scrolled = true;
        }
    }
    assert!(scrolled, "the strip never overflowed while wide");
}

// The right edge of the last tab's content: its label or its badge, whichever ends later,
// plus the tab's 2 px inner padding.
fn tabs_end(cx: &mut TestAppContext, win: AnyWindowHandle) -> Pixels {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0usize..16)
            .flat_map(|ix| {
                [
                    w.try_find(("response-tab", ix)),
                    w.try_find(("response-count", ix)),
                ]
            })
            .flatten()
            .map(|e| e.bounds().right())
            .fold(Pixels::ZERO, Pixels::max)
            + px(2.)
    })
    .unwrap()
}

// At 1280 the whole row fits with room to spare: the address uncut, every tab whole, and at
// least 40 px between the row's width and what its content needs.
#[gpui_kit::test]
fn at_1280_the_row_fits_whole_with_room_to_spare(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 2400.);
    let natural = bounds(cx, s.win, "response-address").size.width;
    resize(cx, &s, 1280.);
    let address = bounds(cx, s.win, "response-address").size.width;
    assert_eq!(address, natural, "the address is cut at 1280");
    assert!(
        !present(cx, s.win, "response-tab-overflow"),
        "a tab is scrolled out"
    );
    let strip = bounds(cx, s.win, "response-tab-strip");
    let spare = strip.right() - tabs_end(cx, s.win);
    assert!(spare >= px(40.), "only {spare:?} to spare at 1280");
}

fn set_zoom(cx: &mut TestAppContext, s: &Started, zoom: f32) {
    use apinata_app::app::AppState;
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = zoom));
    });
    cx.run_until_parked();
    settle(cx, s.win);
}

// The header's floor follows the zoom: a pane wide at 1x folds at 2x and is wide again
// once the zoom is back at 1x.
#[gpui_kit::test]
fn a_zoom_round_trip_leaves_the_pane_wide(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1100.);
    assert!(!present(cx, s.win, "response-more"), "wide at 1x");
    set_zoom(cx, &s, 2.0);
    assert!(present(cx, s.win, "response-more"), "compact at 2x");
    set_zoom(cx, &s, 1.0);
    assert!(
        !present(cx, s.win, "response-more"),
        "stuck compact back at 1x"
    );
}

// Shows the older of two sends, a kept one: its header has the saved chip and Back to
// latest, and Export as HAR is locked for want of the request.
fn viewing_older(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| {
        present(cx, s.win, "response-address") && !present(cx, s.win, "response-cancel")
    });
    settle(cx, s.win);
    let menu = match present(cx, s.win, "response-more") {
        true => "response-more",
        false => "response-history",
    };
    assert_eq!(
        menu, "response-history",
        "the older send is picked while wide"
    );
    cx.update_window(s.win, |_, w, cx| w.click(menu, cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(1usize, cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-saved"));
    settle(cx, s.win);
}

// The floor is the shown facts': once the kept send's longer facts fold the pane, going
// back to the shorter live facts unfolds it at the same width.
#[gpui_kit::test]
fn a_fact_going_away_while_compact_unfolds_the_pane(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1280.);
    viewing_older(cx, &s);
    let mut width = 1280.;
    while !present(cx, s.win, "response-more") {
        width -= 8.;
        assert!(width > 900., "the kept send never folded the pane");
        resize(cx, &s, width);
    }
    cx.update_window(s.win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, s.win);
    let latest = cx
        .update_window(s.win, |_, w, _| {
            (0..8usize)
                .find(|&ix| w.within("popup-menu").find(ix).label() == Some("Back to latest"))
        })
        .unwrap()
        .expect("Back to latest is listed");
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(latest, cx))
        .unwrap();
    wait_real(cx, |cx| !present(cx, s.win, "response-saved"));
    settle(cx, s.win);
    assert!(
        !present(cx, s.win, "response-more"),
        "the live facts fit at {width} px but the pane stayed compact"
    );
}

// A compact pane keeps the body bar one row: the schema test joins Copy and Save in More
// actions, and picking it there adds the test as the button does.
#[gpui_kit::test]
fn a_compact_pane_offers_the_schema_test_in_more_actions(cx: &mut TestAppContext) {
    use apinata_app::body_view::SchemaTest;
    let (s, _server) = redirected(cx, 1280.);
    resize(cx, &s, 880.);
    assert!(present(cx, s.win, "response-more"), "880 px is compact");
    assert!(!present(cx, s.win, "body-schema-test"));
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let ed = editor_of(cx, &s, tab);
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    let got: std::rc::Rc<std::cell::Cell<usize>> = Default::default();
    let sink = got.clone();
    let sub =
        cx.update(|cx| cx.subscribe(&body, move |_, _: &SchemaTest, _| sink.set(sink.get() + 1)));
    std::mem::forget(sub);
    cx.update_window(s.win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, s.win);
    let item = cx
        .update_window(s.win, |_, w, _| {
            (0..8usize)
                .find(|&ix| w.within("popup-menu").find(ix).label() == Some("Add schema test"))
        })
        .unwrap()
        .expect("Add schema test is listed");
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(item, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(got.get(), 1);
}

// A locked Export as HAR names itself before the reason, as its icon carries no text.
#[gpui_kit::test]
fn a_locked_export_har_tooltip_names_the_action(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1600.);
    viewing_older(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover("response-export-har", cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(
        label(cx, s.win, "response-export-har-tooltip"),
        Some(format!(
            "Export HAR: {}",
            apinata_app::har_export::NO_REQUEST
        ))
    );
}

// The icon controls of the header and the body bar that `ui::glyph` saw and that show.
fn shown_glyphs(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
) -> Vec<(&'static str, SharedString)> {
    let recorded = apinata_app::ui::recorded_glyphs();
    let mut shown: Vec<_> = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            recorded
                .into_iter()
                .filter(|(id, _)| w.try_find(*id).is_some_and(|e| e.visible()))
                .collect()
        })
        .unwrap();
    shown.sort();
    shown
}

// Each icon control in the header and the body bar has a glyph of its own, in the busiest
// header live, compact and while an older send shows. The id lists catch a control that bypasses the
// record; a compact pane folds the rest of the header into `response-more`.
#[gpui_kit::test]
fn no_two_shown_icon_controls_share_a_glyph(cx: &mut TestAppContext) {
    const WIDE: &[&str] = &[
        "body-copy-button",
        "body-save-button",
        "response-compare",
        "response-export-har",
        "response-history",
        "response-save-example",
    ];
    const COMPACT: &[&str] = &["response-history", "response-more"];
    let (s, _server) = redirected(cx, 1280.);
    let mut shapes = vec![("live", WIDE, recorded(cx, s.win))];
    resize(cx, &s, 880.);
    assert!(present(cx, s.win, "response-more"), "880 px is compact");
    shapes.push(("compact", COMPACT, recorded(cx, s.win)));
    resize(cx, &s, 1280.);
    viewing_older(cx, &s);
    shapes.push(("older", WIDE, recorded(cx, s.win)));
    for (shape, want, shown) in shapes {
        let ids: Vec<&str> = shown.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, want, "{shape}");
        let mut glyphs: Vec<&str> = shown.iter().map(|(_, g)| g.as_str()).collect();
        glyphs.sort();
        glyphs.dedup();
        assert_eq!(glyphs.len(), shown.len(), "{shape}: {shown:?}");
    }
}

// The shown glyphs of a fresh record, so a control left from an earlier shape does not count.
fn recorded(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<(&'static str, SharedString)> {
    apinata_app::ui::clear_recorded_glyphs();
    settle(cx, win);
    shown_glyphs(cx, win)
}

// An older send at 1280 stacked keeps the header one row, with Back to latest whole.
#[gpui_kit::test]
fn viewing_an_older_send_at_1280_keeps_back_to_latest_whole(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1600.);
    viewing_older(cx, &s);
    let natural = bounds(cx, s.win, "response-latest").size.width;
    resize(cx, &s, 1280.);
    assert!(
        !present(cx, s.win, "response-more"),
        "1280 px is a wide pane"
    );
    assert_one_row(cx, s.win);
    let latest = bounds(cx, s.win, "response-latest");
    let pane = bounds(cx, s.win, "response");
    let strip = bounds(cx, s.win, "response-tab-strip");
    assert_eq!(latest.size.width, natural, "Back to latest is cut");
    assert!(
        latest.left() >= pane.left() && latest.right() <= pane.right(),
        "{latest:?} leaves {pane:?}"
    );
    assert!(
        latest.center().y > strip.top() && latest.center().y < strip.bottom(),
        "{latest:?} is off the strip's row {strip:?}"
    );
}

// GPUI's svg paints only with a text colour of its own, never an inherited one, so the
// dropdowns' chevron needs one or its half of the button stays blank.
#[gpui_kit::test]
fn the_dropdown_chevron_paints_in_the_buttons_colour(cx: &mut TestAppContext) {
    use gpui_kit::Styled as _;
    use gpui_kit::component::ActiveTheme as _;
    init(cx);
    let (painted, foreground) = cx.update(|cx| {
        let mut chevron = apinata_app::response_pane::chevron(cx);
        (chevron.style().text.color, cx.theme().secondary_foreground)
    });
    assert_eq!(painted, Some(foreground));
}

// Hovering Previous runs or Compare shows its name, as the icon carries no text.
#[gpui_kit::test]
fn the_icon_dropdowns_show_their_names_on_hover(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1280.);
    for (id, tip, name) in [
        (
            "response-history",
            "response-history-tooltip",
            "Previous runs",
        ),
        ("response-compare", "response-compare-tooltip", "Compare"),
    ] {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.hover("response-body", cx)
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(800));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.hover(id, cx)
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(800));
        frame(cx, s.win);
        assert_eq!(label(cx, s.win, tip), Some(name.to_owned()), "{id}");
    }
}

// History and More actions sit side by side in a compact pane; a chevron on both read as two
// copies of one control, so More actions shows the overflow ellipsis instead.
#[gpui_kit::test]
fn more_actions_is_not_a_second_chevron_beside_history(cx: &mut TestAppContext) {
    let (s, _server) = redirected(cx, 1280.);
    resize(cx, &s, 880.);
    let shown = recorded(cx, s.win);
    let glyph = |id: &str| shown.iter().find(|(i, _)| *i == id).map(|(_, g)| g.clone());
    assert_eq!(
        glyph("response-more"),
        Some(gpui_kit::assets::IconName::Ellipsis.path())
    );
    assert!(glyph("response-history").is_some(), "{shown:?}");
}
