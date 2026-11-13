//! The status bar toggles the sidebar and the response layout, at the designed height.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::keymap::ToggleOrientation;
use apinata_app::settings::Orientation;
use apinata_app::style::{STATUS_BAR_HEIGHT, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

fn orientation(cx: &mut TestAppContext) -> Orientation {
    cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .response_orientation
    })
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.within("status-bar").click(id, cx))
        .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_status_bar_toggles_the_sidebar_and_the_layout(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    click(cx, &s, "toggle-sidebar");
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    click(cx, &s, "toggle-sidebar");
    assert!(s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    assert_eq!(orientation(cx), Orientation::Vertical);
    click(cx, &s, "toggle-layout");
    assert_eq!(orientation(cx), Orientation::Horizontal);
}

#[gpui_kit::test]
fn the_status_bar_has_the_spec_height(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let height = cx
        .update_window(s.win, |_, w, _| w.find("status-bar").bounds().size.height)
        .unwrap();
    assert_eq!(height, at_zoom(STATUS_BAR_HEIGHT, 1.0));
}

#[gpui_kit::test]
fn the_status_bar_buttons_fit_inside_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let bar = w.find("status-bar").bounds();
        for id in ["toggle-sidebar", "toggle-layout"] {
            let b = w.within("status-bar").find(id).bounds();
            assert!(
                b.top() >= bar.top() && b.bottom() <= bar.bottom(),
                "{id}: {b:?} in {bar:?}"
            );
        }
    })
    .unwrap();
}

// The layout button names the layout a click switches to.
#[gpui_kit::test]
fn the_layout_button_names_the_layout_it_switches_to(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let label = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("status-bar")
                .find("toggle-layout")
                .label()
                .map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Switch to side by side layout"));
    click(cx, &s, "toggle-layout");
    assert_eq!(label(cx).as_deref(), Some("Switch to stacked layout"));
}

const TOO_NARROW: &str = "Side by side needs a wider window";

// Opens `a.yml` and sizes the window to `width` with `saved` as the chosen layout.
fn sized(cx: &mut TestAppContext, width: f32, saved: Orientation) -> Started {
    let col = two_requests();
    let s = started(cx, &col);
    std::mem::forget(col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    resize(cx, &s, width, saved);
    s
}

fn resize(cx: &mut TestAppContext, s: &Started, width: f32, saved: Orientation) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(600.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| st.response_orientation = saved)
        });
    });
    for _ in 0..3 {
        frame(cx, s.win);
    }
}

// The layout button's name and whether it is locked.
fn layout_button(cx: &mut TestAppContext, s: &Started) -> (Option<String>, bool) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let button = w.within("status-bar").find("toggle-layout");
        let name = button.label().map(str::to_owned);
        let locked = w.within("toggle-layout").try_find("locked").is_some();
        (name, locked)
    })
    .unwrap()
}

fn stacked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, _| {
        w.find("response").bounds().top() >= w.find("request-column").bounds().bottom() - px(1.)
    })
    .unwrap()
}

// At 800 px side by side cannot fit, so the button is off and its tooltip, which also names
// it, says why; neither a click nor the shortcut flips a choice that would change nothing.
#[gpui_kit::test]
fn at_800_px_the_layout_button_explains_why_the_view_stays_stacked(cx: &mut TestAppContext) {
    let s = sized(cx, 800., Orientation::Vertical);
    assert!(stacked(cx, &s));
    assert_eq!(layout_button(cx, &s), (Some(TOO_NARROW.into()), true));
    click(cx, &s, "toggle-layout");
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleOrientation);
    frame(cx, s.win);
    assert_eq!(orientation(cx), Orientation::Vertical);
    assert!(stacked(cx, &s));
}

// A side-by-side choice kept through a narrow window applies once the window is wide enough.
#[gpui_kit::test]
fn widening_applies_side_by_side_when_it_was_chosen(cx: &mut TestAppContext) {
    let s = sized(cx, 800., Orientation::Horizontal);
    assert!(stacked(cx, &s));
    assert_eq!(layout_button(cx, &s), (Some(TOO_NARROW.into()), true));
    resize(cx, &s, 1800., Orientation::Horizontal);
    assert!(!stacked(cx, &s));
    assert_eq!(
        layout_button(cx, &s),
        (Some("Switch to stacked layout".into()), false)
    );
}

// With room for both panes the button switches the layout as before.
#[gpui_kit::test]
fn a_wide_window_keeps_the_layout_button_as_it_was(cx: &mut TestAppContext) {
    let s = sized(cx, 1800., Orientation::Vertical);
    assert_eq!(
        layout_button(cx, &s),
        (Some("Switch to side by side layout".into()), false)
    );
    click(cx, &s, "toggle-layout");
    for _ in 0..2 {
        frame(cx, s.win);
    }
    assert_eq!(orientation(cx), Orientation::Horizontal);
    assert!(!stacked(cx, &s));
}
