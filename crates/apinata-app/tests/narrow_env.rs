//! At the narrowest window every control of both environment editors can be reached, and a
//! wide window shows no sideways scrolling at all.

use crate::env_orphans::{Listing, global_editor_of, seeded};
use crate::support;

use apinata_app::app::AppState;
use apinata_app::style::{
    DIALOG_SIDE_PADDINGS, ENV_EDITOR_LIST_WIDTH, ENV_PANE_MIN, GLOBAL_ENV_DIALOG_WIDTH,
    SCROLLBAR_GUTTER, resolve,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Focusable as _, Pixels, Rems, ScrollDelta,
    TestAppContext, point, px, size,
};
use std::sync::Arc;
use support::*;

const NARROW: f32 = 320.;
const X_BAR: &str = "env-editor-scrollbar-x";
const GLOBAL_X_BAR: &str = "global-env-scrollbar-x";
const SCROLL: &str = "env-editor";
const GLOBAL_SCROLL: &str = "global-env-scroll";
const FILE: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";
const GLOBAL_FILE: &str = "name: g\nvariables:\n  - name: host\n    value: a\n";

fn resize(cx: &mut TestAppContext, win: AnyWindowHandle, w: f32, h: f32) {
    gpui_kit::VisualTestContext::from_window(win, cx).simulate_resize(size(px(w), px(h)));
    for _ in 0..4 {
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

fn resolved(cx: &mut TestAppContext, win: AnyWindowHandle, rems: Rems) -> Pixels {
    cx.update_window(win, |_, w, _| resolve(rems, w)).unwrap()
}

fn wheel(cx: &mut TestAppContext, win: AnyWindowHandle, scroller: &'static str, by: (f32, f32)) {
    let delta = ScrollDelta::Pixels(point(px(by.0), px(by.1)));
    cx.update_window(win, |_, w, cx| w.scroll(scroller, delta, cx))
        .unwrap();
    for _ in 0..3 {
        frame(cx, win);
    }
}

// Wheels `scroller` just far enough to show `id` whole, as a person would, then checks it.
fn reach(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    scroller: &'static str,
    id: impl Into<ElementId> + Clone,
) {
    // A wheel event moves one axis, the one it leans to.
    wheel(cx, win, scroller, (10_000., 0.));
    wheel(cx, win, scroller, (0., 10_000.));
    let (view, at) = (bounds(cx, win, scroller), bounds(cx, win, id.clone()));
    let right = (at.right() - view.right()).max(px(0.));
    let down = (at.bottom() - view.bottom()).max(px(0.));
    wheel(cx, win, scroller, (-(right / px(1.)), 0.));
    wheel(cx, win, scroller, (0., -(down / px(1.))));
    let (view, at) = (bounds(cx, win, scroller), bounds(cx, win, id.clone()));
    let inside = at.left() >= view.left() - px(1.)
        && at.right() <= view.right() + px(1.)
        && at.top() >= view.top() - px(1.)
        && at.bottom() <= view.bottom() + px(1.);
    assert!(inside, "{:?}: {at:?} is outside {view:?}", id.into());
}

fn collection(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", FILE)]);
    let (store, _key) = seeded(&col);
    let s = started_with_store(cx, &col, store);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    (col, s)
}

#[gpui_kit::test]
fn every_collection_control_can_be_reached_at_the_narrowest_width(cx: &mut TestAppContext) {
    let (_col, s) = collection(cx);
    resize(cx, s.win, NARROW, 700.);
    assert!(present(cx, s.win, X_BAR));
    let ids: [ElementId; 14] = [
        "env-save".into(),
        "env-more".into(),
        "env-title".into(),
        "env-swatches".into(),
        "env-swatch-none".into(),
        "env-extends".into(),
        "env-create".into(),
        ("env-name", 0usize).into(),
        ("env-value", 0usize).into(),
        ("env-secret", 0usize).into(),
        ("env-delete", 0usize).into(),
        ("env-orphan-remove", 0usize).into(),
        ("env-orphan-row", 0usize).into(),
        ("env-item", 0usize).into(),
    ];
    for id in ids {
        reach(cx, s.win, SCROLL, id);
    }
}

#[gpui_kit::test]
fn the_far_edge_keeps_the_gutter_past_the_last_control(cx: &mut TestAppContext) {
    let (_col, s) = collection(cx);
    resize(cx, s.win, NARROW, 700.);
    wheel(cx, s.win, SCROLL, (-10_000., 0.));
    let view = bounds(cx, s.win, SCROLL);
    let pane = bounds(cx, s.win, "env-pane");
    assert!(
        pane.right() <= view.right() - SCROLLBAR_GUTTER + px(1.),
        "{pane:?} runs under the scrollbar of {view:?}"
    );
}

#[gpui_kit::test]
fn the_sideways_bar_never_hides_the_bottom_of_the_page(cx: &mut TestAppContext) {
    let (_col, s) = collection(cx);
    resize(cx, s.win, NARROW, 300.);
    wheel(cx, s.win, SCROLL, (0., -10_000.));
    let bar = bounds(cx, s.win, X_BAR);
    let view = bounds(cx, s.win, SCROLL);
    let last = bounds(cx, s.win, ("env-orphan-row", 0usize));
    assert!(
        view.bottom() <= bar.top() + px(1.),
        "{view:?} under {bar:?}"
    );
    assert!(
        last.bottom() <= bar.top() + px(1.),
        "{last:?} under {bar:?}"
    );
}

#[gpui_kit::test]
fn a_wide_collection_editor_has_no_sideways_scrolling(cx: &mut TestAppContext) {
    let (_col, s) = collection(cx);
    resize(cx, s.win, 1000., 700.);
    assert!(!present(cx, s.win, X_BAR));
    wheel(cx, s.win, SCROLL, (-500., 0.));
    let list = bounds(cx, s.win, "env-list");
    assert_eq!(list.size.width, resolved(cx, s.win, ENV_EDITOR_LIST_WIDTH));
    let view = bounds(cx, s.win, SCROLL);
    let pane = bounds(cx, s.win, "env-pane");
    assert_eq!(view.right() - pane.right(), SCROLLBAR_GUTTER);
}

#[gpui_kit::test]
fn the_empty_state_still_centres(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    resize(cx, s.win, 1000., 700.);
    assert!(!present(cx, s.win, X_BAR));
    let (pane, group) = (
        bounds(cx, s.win, "env-empty-pane"),
        bounds(cx, s.win, "env-empty"),
    );
    let (mid_pane, mid_group) = (pane.center().x, group.center().x);
    assert!((mid_pane - mid_group).abs() <= px(1.), "{pane:?} {group:?}");
}

// The app with the global environment `g`, in a window at the narrowest width and `zoom`.
fn zoomed_globals(
    cx: &mut TestAppContext,
    zoom: f32,
) -> (tempfile::TempDir, tempfile::TempDir, Started) {
    let col = two_requests();
    let globals = write_tree(&[("g.yml", GLOBAL_FILE)]);
    let s = started(cx, &col);
    let dir = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = zoom));
    });
    resize(cx, s.win, NARROW, 700.);
    (col, globals, s)
}

fn global(cx: &mut TestAppContext, vars: &str) -> crate::env_orphans::GlobalHarness {
    global_editor_of(cx, Arc::new(Listing::default()), vars)
}

#[gpui_kit::test]
fn every_global_control_can_be_reached_at_the_narrowest_width(cx: &mut TestAppContext) {
    let (_c, _g, _s, win, _ed, _key) = global(cx, GLOBAL_FILE);
    resize(cx, win, NARROW, 700.);
    assert!(present(cx, win, GLOBAL_X_BAR));
    let dialog = bounds(cx, win, GLOBAL_SCROLL);
    assert!(dialog.right() <= px(NARROW), "{dialog:?}");
    let ids: [ElementId; 10] = [
        "global-env-rename".into(),
        "global-env-duplicate".into(),
        "global-env-delete".into(),
        "global-env-save".into(),
        "global-env-create".into(),
        ("env-name", 0usize).into(),
        ("env-value", 0usize).into(),
        ("env-secret", 0usize).into(),
        ("env-delete", 0usize).into(),
        ("env-orphan-remove", 0usize).into(),
    ];
    for id in ids {
        reach(cx, win, GLOBAL_SCROLL, id);
    }
    // The row is wider than the dialog's body, so each edge comes into view in its turn.
    let row = ("env-orphan-row", 0usize);
    wheel(cx, win, GLOBAL_SCROLL, (-10_000., 0.));
    let view = bounds(cx, win, GLOBAL_SCROLL);
    assert!(bounds(cx, win, row).right() <= view.right() + px(1.));
    wheel(cx, win, GLOBAL_SCROLL, (10_000., 0.));
    assert!(bounds(cx, win, row).left() >= view.left() - px(1.));
}

#[gpui_kit::test]
fn the_global_table_keeps_its_minimum_at_the_narrowest_width(cx: &mut TestAppContext) {
    let (_c, _g, _s, win, _ed, _key) = global(cx, GLOBAL_FILE);
    resize(cx, win, NARROW, 700.);
    let row = bounds(cx, win, ("env-row", 0usize));
    let want = resolved(cx, win, ENV_PANE_MIN);
    assert!(row.size.width >= want - px(1.), "{row:?}");
}

#[gpui_kit::test]
fn the_sideways_bar_stays_in_the_dialogs_room(cx: &mut TestAppContext) {
    let mut vars = "name: g\nvariables:\n".to_string();
    for i in 0..30 {
        vars += &format!("  - name: v{i}\n    value: x\n");
    }
    let (_c, _g, _s, win, _ed, _key) = global(cx, &vars);
    resize(cx, win, NARROW, 400.);
    let (view, bar) = (
        bounds(cx, win, GLOBAL_SCROLL),
        bounds(cx, win, GLOBAL_X_BAR),
    );
    let room = cx
        .update_window(win, |_, w, _| {
            apinata_app::dialog_frame::titled_body_room(w)
        })
        .unwrap();
    assert!(
        view.bottom() <= bar.top() + px(1.),
        "{view:?} under {bar:?}"
    );
    assert!(
        bar.bottom() - view.top() <= room + px(1.),
        "{view:?} {bar:?}"
    );
}

#[gpui_kit::test]
fn a_wide_global_dialog_keeps_its_width_and_has_no_sideways_scrolling(cx: &mut TestAppContext) {
    let (_c, _g, _s, win, _ed, _key) = global(cx, GLOBAL_FILE);
    resize(cx, win, 1000., 700.);
    assert!(!present(cx, win, GLOBAL_X_BAR));
    let view = bounds(cx, win, GLOBAL_SCROLL);
    // The kit's side paddings and its border.
    let want = resolved(cx, win, GLOBAL_ENV_DIALOG_WIDTH) - DIALOG_SIDE_PADDINGS - px(2.);
    assert_eq!(view.size.width, want);
}

// At 200 percent the window is half as wide in layout terms, so the title bar has no room for
// the global select; its compact button takes over and the dialog stays reachable from it.
#[gpui_kit::test]
fn the_global_dialog_opens_from_the_title_bar_at_200_percent_at_the_narrowest_width(
    cx: &mut TestAppContext,
) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    let (button, width) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let bar = w.within("title-bar");
            (
                bar.find("global-environments").bounds(),
                w.viewport_size().width,
            )
        })
        .unwrap();
    assert!(
        button.left() >= px(0.) && button.right() <= width,
        "{button:?}"
    );
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("global-environments", cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "global-env-editor"));
}

#[gpui_kit::test]
fn every_global_control_can_be_reached_at_200_percent_at_the_narrowest_width(
    cx: &mut TestAppContext,
) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("global-environments", cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(("global-env-item", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    let dialog = bounds(cx, s.win, GLOBAL_SCROLL);
    assert!(dialog.right() <= px(NARROW), "{dialog:?}");
    let ids: [ElementId; 7] = [
        "global-env-rename".into(),
        "global-env-duplicate".into(),
        "global-env-delete".into(),
        "global-env-save".into(),
        ("env-name", 0usize).into(),
        ("env-value", 0usize).into(),
        ("env-delete", 0usize).into(),
    ];
    for id in ids {
        reach(cx, s.win, GLOBAL_SCROLL, id);
    }
    // The list column alone is wider than the dialog here, so its button shows in part, with
    // its label in view once the list is scrolled back.
    wheel(cx, s.win, GLOBAL_SCROLL, (10_000., 0.));
    let (view, create) = (
        bounds(cx, s.win, GLOBAL_SCROLL),
        bounds(cx, s.win, "global-env-create"),
    );
    assert!(view.contains(&create.center()), "{create:?} in {view:?}");
}

// The button sits in the select's place and runs into neither neighbour, down to the narrowest
// bar the title bar is swept at.
#[gpui_kit::test]
fn the_global_button_keeps_its_place_in_the_bar(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    for width in [NARROW, 280., 240.] {
        resize(cx, s.win, width, 700.);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let bar = w.within("title-bar");
            let [env, button, manage] = ["env-control", "global-environments", "environments"]
                .map(|id| bar.find(id).bounds());
            assert!(
                env.right() <= button.left() + px(0.5),
                "{width}: {env:?} {button:?}"
            );
            assert!(
                button.right() <= manage.left() + px(0.5),
                "{width}: {button:?} {manage:?}"
            );
            assert!(
                button.size.width > px(0.) && button.left() >= px(0.),
                "{width}"
            );
        })
        .unwrap();
    }
}

// A wide bar keeps the full select, so the button is no second way in there.
#[gpui_kit::test]
fn the_global_select_shows_instead_of_the_button_at_100_percent(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 1.0);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let bar = w.within("title-bar");
        assert!(bar.try_find("global-environments").is_none());
        assert!(bar.find("global-env-selector").bounds().size.width > px(0.));
    })
    .unwrap();
}

fn palette_reachable(cx: &mut TestAppContext, s: &Started) -> bool {
    for _ in 0..3 {
        frame(cx, s.win);
    }
    cx.update_window(s.win, |_, w, cx| {
        w.is_action_available(&apinata_app::palette::OpenCommandPalette, cx)
    })
    .unwrap()
}

fn focus_global_select(cx: &mut TestAppContext, s: &Started) {
    let select = s
        .shell
        .read_with(cx, |sh, _| sh.global_env_selector().clone());
    cx.update_window(s.win, |_, w, cx| {
        let handle = select.read(cx).select().focus_handle(cx);
        handle.focus(w, cx)
    })
    .unwrap();
    frame(cx, s.win);
}

fn set_zoom(cx: &mut TestAppContext, zoom: f32) {
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = zoom));
    });
}

// The select leaves the bar with focus on it, and the app's keys must not leave with it.
#[gpui_kit::test]
fn narrowing_past_the_button_width_keeps_the_keys_with_the_select_focused(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    resize(cx, s.win, 640., 700.);
    focus_global_select(cx, &s);
    assert!(palette_reachable(cx, &s));
    resize(cx, s.win, NARROW, 700.);
    assert!(palette_reachable(cx, &s));
}

#[gpui_kit::test]
fn zooming_past_the_button_width_keeps_the_keys_with_the_select_focused(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 1.0);
    focus_global_select(cx, &s);
    set_zoom(cx, 2.0);
    assert!(palette_reachable(cx, &s));
}

// Tabs onto the button; the bar is short, so a full round finds it.
fn focus_global_button(cx: &mut TestAppContext, s: &Started) {
    for _ in 0..40 {
        let on_it = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.within("title-bar").find("global-environments").focused()
            })
            .unwrap();
        if on_it == Some(true) {
            return;
        }
        cx.update_window(s.win, |_, w, cx| w.press("tab", cx))
            .unwrap();
    }
    panic!("Tab never reached the global environments button");
}

#[gpui_kit::test]
fn widening_past_the_button_width_keeps_the_keys_with_the_button_focused(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    focus_global_button(cx, &s);
    resize(cx, s.win, 640., 700.);
    assert!(palette_reachable(cx, &s));
}

#[gpui_kit::test]
fn zooming_back_past_the_button_width_keeps_the_keys_with_the_button_focused(
    cx: &mut TestAppContext,
) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    focus_global_button(cx, &s);
    set_zoom(cx, 1.0);
    assert!(palette_reachable(cx, &s));
}

// Opens the global dialog with Enter on the button, so the dialog will hand focus back to it.
fn open_dialog_from_the_button(cx: &mut TestAppContext, s: &Started) {
    focus_global_button(cx, s);
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "global-env-editor"));
}

fn open_dialog_from_the_select(cx: &mut TestAppContext, s: &Started) {
    focus_global_select(cx, s);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::shell::OpenGlobalEnvironments), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "global-env-editor"));
}

fn escape_the_dialog(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "global-env-editor"));
}

// The element the dialog would give focus back to leaves the bar while the dialog is open, so
// its close has nothing drawn to return to.
#[gpui_kit::test]
fn widening_under_the_dialog_keeps_the_keys_when_it_closes(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    open_dialog_from_the_button(cx, &s);
    resize(cx, s.win, 640., 700.);
    assert!(present(cx, s.win, "global-env-editor"), "the dialog stays");
    escape_the_dialog(cx, &s);
    assert!(palette_reachable(cx, &s));
}

#[gpui_kit::test]
fn zooming_out_under_the_dialog_keeps_the_keys_when_it_closes(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    open_dialog_from_the_button(cx, &s);
    set_zoom(cx, 1.0);
    frame(cx, s.win);
    escape_the_dialog(cx, &s);
    assert!(palette_reachable(cx, &s));
}

#[gpui_kit::test]
fn narrowing_under_the_dialog_keeps_the_keys_when_it_closes(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    resize(cx, s.win, 640., 700.);
    open_dialog_from_the_select(cx, &s);
    resize(cx, s.win, NARROW, 700.);
    escape_the_dialog(cx, &s);
    assert!(palette_reachable(cx, &s));
}

#[gpui_kit::test]
fn zooming_in_under_the_dialog_keeps_the_keys_when_it_closes(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 1.0);
    open_dialog_from_the_select(cx, &s);
    set_zoom(cx, 2.0);
    frame(cx, s.win);
    escape_the_dialog(cx, &s);
    assert!(palette_reachable(cx, &s));
}

// The dialog keeps focus through the crossing: its own Escape still closes it, as the routes
// above show, and a key typed into it stays there rather than reaching the shell.
#[gpui_kit::test]
fn a_crossing_under_the_dialog_leaves_focus_in_the_dialog(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    open_dialog_from_the_button(cx, &s);
    resize(cx, s.win, 640., 700.);
    let in_dialog = cx
        .update_window(s.win, |_, w, _| {
            (w.context_stack().iter()).any(|c| format!("{c:?}").contains("Dialog"))
        })
        .unwrap();
    assert!(in_dialog, "focus stays in the dialog");
}

// A control that is still drawn keeps focus through the crossing.
#[gpui_kit::test]
fn a_crossing_leaves_a_drawn_focused_control_alone(cx: &mut TestAppContext) {
    let (_col, _globals, s) = zoomed_globals(cx, 2.0);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    frame(cx, s.win);
    resize(cx, s.win, 640., 700.);
    let in_url = cx
        .update_window(s.win, |_, w, _| {
            (w.context_stack().iter()).any(|c| format!("{c:?}").contains("Input"))
        })
        .unwrap();
    assert!(in_url, "focus stays in the URL input");
}
