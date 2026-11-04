//! The filter's placeholder fits beside the key hint at the default sidebar width, the hint
//! steps aside before it meets the row's buttons, and a tooltip keeps the `tag:` and
//! `method:` terms discoverable.

use crate::support;

use apinata_app::sidebar::{FILTER_PLACEHOLDER, FILTER_TOOLTIP};
use gpui_kit::AppContext as _;
use gpui_kit::Focusable as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

#[test]
fn the_texts_are_pinned() {
    assert_eq!(FILTER_PLACEHOLDER, "Filter");
    assert_eq!(
        FILTER_TOOLTIP,
        "Filter by name. Use tag:name or method:get."
    );
}

#[test]
fn the_tooltip_names_both_terms() {
    assert!(FILTER_TOOLTIP.contains("tag:"));
    assert!(FILTER_TOOLTIP.contains("method:"));
}

// At the default sidebar width the placeholder fits in the room left of the key hint.
#[gpui_kit::test]
fn the_placeholder_fits_beside_the_key_hint_at_the_default_width(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let h = open_sidebar(cx);
    gpui_kit::VisualTestContext::from_window(h.win, cx).simulate_resize(size(px(240.), px(700.)));
    frame(cx, h.win);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        let field = w.find("sidebar-filter-input").bounds();
        let kbd = w.find("sidebar-filter-kbd").bounds();
        // The kit input's medium text is `text_sm`, 0.875 rem.
        let size = gpui_kit::rems(0.875).to_pixels(w.rem_size());
        let run = w.text_style().to_run(FILTER_PLACEHOLDER.len());
        let text = w
            .text_system()
            .shape_line(FILTER_PLACEHOLDER.into(), size, &[run], None)
            .width;
        // Room left of the hint, less the search icon and the padding before the text.
        let room = kbd.left() - field.left() - gpui_kit::rems(2.).to_pixels(w.rem_size());
        assert!(text <= room, "placeholder {text:?} in {room:?} ({field:?})");
    })
    .unwrap();
}

type B = gpui_kit::Bounds<gpui_kit::Pixels>;

// The filter row's laid-out boxes, the hint's only when shown, plus the placeholder's width
// and the room left of the hint for it (none without the hint).
struct Row {
    field: B,
    icon: B,
    kbd: Option<B>,
    collapse: B,
    text: gpui_kit::Pixels,
    room: Option<gpui_kit::Pixels>,
}

// A sidebar alone in a window, at `zoom`.
fn zoomed(cx: &mut TestAppContext, zoom: f32) -> SidebarHarness {
    let h = open_sidebar(cx);
    // The harness has no kit Root to turn the theme's font size into the rem, so the zoom's
    // rem is set on the window directly, as the Root would each frame.
    let settings = apinata_app::settings::Settings {
        zoom,
        ..Default::default()
    };
    cx.update_window(h.win, |_, w, cx| {
        apinata_app::theme::apply(&settings, w, cx);
        w.set_rem_size(apinata_app::zoom::ui_font_size(zoom));
    })
    .unwrap();
    cx.run_until_parked();
    h
}

// Lays `h` out `width` px wide and measures its filter row.
fn measure(cx: &mut TestAppContext, h: &SidebarHarness, width: gpui_kit::Pixels) -> Row {
    use gpui_kit::{px, size};
    gpui_kit::VisualTestContext::from_window(h.win, cx).simulate_resize(size(width, px(700.)));
    frame(cx, h.win);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        let field = w.find("sidebar-filter-input").bounds();
        let icon = w.find("sidebar-filter-icon").bounds();
        let kbd = w.try_find("sidebar-filter-kbd").map(|e| e.bounds());
        let collapse = w.find("sidebar-collapse-all").bounds();
        // The kit input's medium text is `text_sm`, 0.875 rem; it starts a quarter rem past
        // the search icon.
        let rem = w.rem_size();
        let run = w.text_style().to_run(FILTER_PLACEHOLDER.len());
        let text = w
            .text_system()
            .shape_line(
                FILTER_PLACEHOLDER.into(),
                gpui_kit::rems(0.875).to_pixels(rem),
                &[run],
                None,
            )
            .width;
        let room = kbd.map(|k| k.left() - icon.right() - gpui_kit::rems(0.25).to_pixels(rem));
        Row {
            field,
            icon,
            kbd,
            collapse,
            text,
            room,
        }
    })
    .unwrap()
}

// A fresh sidebar's filter row, `width` px wide at `zoom`.
fn filter_row(cx: &mut TestAppContext, zoom: f32, width: gpui_kit::Pixels) -> Row {
    let h = zoomed(cx, zoom);
    measure(cx, &h, width)
}

const ZOOMS: [f32; 3] = [apinata_app::zoom::ZOOM_MIN, 1.0, 2.0];

// Neither the field nor its key hint reaches under the row's buttons, at the narrowest and
// the default sidebar width, at the smallest zoom, 100% and 200%.
#[gpui_kit::test]
fn the_filter_field_stays_left_of_the_buttons(cx: &mut TestAppContext) {
    use apinata_app::style::{SIDEBAR_WIDTH, at_zoom, sidebar_min_at};
    // Every case is measured before any assertion, so one failure reports them all.
    let mut bad = Vec::new();
    for zoom in ZOOMS {
        for (name, width) in [
            ("min", sidebar_min_at(zoom)),
            ("default", at_zoom(SIDEBAR_WIDTH, zoom)),
        ] {
            let Row {
                field,
                icon,
                kbd,
                collapse,
                ..
            } = filter_row(cx, zoom, width);
            let fits = field.right() <= collapse.left()
                && icon.right() <= field.right()
                && kbd.is_none_or(|k| {
                    k.right() <= collapse.left()
                        && k.left() >= field.left()
                        && k.right() <= field.right()
                });
            if !fits {
                bad.push(format!(
                    "z={zoom} {name}: field {field:?} icon {icon:?} kbd {kbd:?} collapse {collapse:?}"
                ));
            }
        }
    }
    assert!(bad.is_empty(), "overlaps:\n{}", bad.join("\n"));
}

// Wherever the hint shows, from the narrowest to the default width, the placeholder fits in
// the room left of it.
#[gpui_kit::test]
fn the_placeholder_fits_wherever_the_hint_shows(cx: &mut TestAppContext) {
    use apinata_app::style::{SIDEBAR_WIDTH, at_zoom, sidebar_min_at};
    let mut bad = Vec::new();
    for zoom in ZOOMS {
        let h = zoomed(cx, zoom);
        let (lo, hi) = (sidebar_min_at(zoom), at_zoom(SIDEBAR_WIDTH, zoom));
        let mut width = lo;
        while width <= hi {
            let row = measure(cx, &h, width);
            if let Some(room) = row.room
                && row.text > room
            {
                bad.push(format!(
                    "z={zoom} w={width:?}: text {:?} room {room:?} field {:?}",
                    row.text, row.field
                ));
            }
            width += gpui_kit::px(1.);
        }
    }
    assert!(bad.is_empty(), "squeezed:\n{}", bad.join("\n"));
}

// The hint still shows at the default width at 100%.
#[gpui_kit::test]
fn the_hint_shows_at_the_default_width(cx: &mut TestAppContext) {
    let row = filter_row(
        cx,
        1.0,
        apinata_app::style::at_zoom(apinata_app::style::SIDEBAR_WIDTH, 1.0),
    );
    assert!(row.kbd.is_some());
}

// The hint steps aside on a narrowing and comes back on a widening of the same sidebar.
#[gpui_kit::test]
fn the_hint_comes_back_when_the_sidebar_widens(cx: &mut TestAppContext) {
    use apinata_app::style::{SIDEBAR_WIDTH, at_zoom, sidebar_min_at};
    use gpui_kit::{px, size};
    let h = open_sidebar(cx);
    let at = |cx: &mut TestAppContext, width| {
        gpui_kit::VisualTestContext::from_window(h.win, cx).simulate_resize(size(width, px(700.)));
        frame(cx, h.win);
        frame(cx, h.win);
        present(cx, h.win, "sidebar-filter-kbd")
    };
    assert!(!at(cx, sidebar_min_at(1.0)));
    assert!(at(cx, at_zoom(SIDEBAR_WIDTH, 1.0)));
}

// At the narrowest sidebar, the text area beside the search icon holds the six-letter
// placeholder whole, so typed text shows too, at the smallest zoom, 100% and 200%.
#[gpui_kit::test]
fn the_text_area_holds_the_placeholder_at_the_narrowest_width(cx: &mut TestAppContext) {
    let mut bad = Vec::new();
    for zoom in ZOOMS {
        let h = zoomed(cx, zoom);
        let row = measure(cx, &h, apinata_app::style::sidebar_min_at(zoom));
        let filter = h.view.read_with(cx, |s, _| s.filter().clone());
        let area = filter.read_with(cx, |f, _| f.input_bounds());
        if area.size.width < row.text {
            bad.push(format!(
                "z={zoom}: text area {area:?} under the placeholder's {:?} (field {:?})",
                row.text, row.field
            ));
        }
    }
    assert!(bad.is_empty(), "cramped:\n{}", bad.join("\n"));
}

// At the narrowest sidebar a click on the field focuses it, at every zoom.
#[gpui_kit::test]
fn a_click_focuses_the_field_at_the_narrowest_width(cx: &mut TestAppContext) {
    let mut bad = Vec::new();
    for zoom in ZOOMS {
        let h = zoomed(cx, zoom);
        let row = measure(cx, &h, apinata_app::style::sidebar_min_at(zoom));
        click_with(
            cx,
            h.win,
            "sidebar-filter-input",
            gpui_kit::Modifiers::default(),
        );
        frame(cx, h.win);
        let filter = h.view.read_with(cx, |s, _| s.filter().clone());
        let focused = cx
            .update_window(h.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
            .unwrap();
        if !focused {
            bad.push(format!(
                "z={zoom}: field {:?} icon {:?}",
                row.field, row.icon
            ));
        }
    }
    assert!(bad.is_empty(), "unfocused:\n{}", bad.join("\n"));
}
