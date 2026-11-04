//! The opened split takes the window's remembered ratio; a drag sets it, an expand or a
//! window resize never does, and the divider moves even in a short room.

use crate::support;

use apinata_app::style::PANE_MIN;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size};
use support::*;

// The request pane's share of the laid-out split; fails the test when the split has no panels.
fn share(cx: &mut TestAppContext, ed: &Entity<apinata_app::request_editor::RequestEditor>) -> f32 {
    let s = ed.read_with(cx, |e, cx| e.split().read(cx).sizes().clone());
    let [request, response] = s.as_slice() else {
        panic!("the split is not laid out as two panels: {s:?}")
    };
    f32::from(*request) / f32::from(*request + *response)
}

#[gpui_kit::test]
fn the_ratio_starts_even(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    assert_eq!(ws.read_with(cx, |w, _| w.split_ratio()), 0.5);
}

#[gpui_kit::test]
fn a_drag_sets_the_ratio_every_tab_opens_at(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed_a = editor_of(cx, &s, a);
    give_result(cx, &ed_a);
    frame(cx, s.win);
    frame(cx, s.win);
    let split = ed_a.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |sp, _| sp.sizes()[0] + sp.sizes()[1]);
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, total * 0.3, w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    let ratio = s.ws.read_with(cx, |w, _| w.split_ratio());
    assert!((ratio - 0.3).abs() < 0.02, "{ratio}");
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    let ed_b = editor_of(cx, &s, b);
    give_result(cx, &ed_b);
    frame(cx, s.win);
    frame(cx, s.win);
    let shown = share(cx, &ed_b);
    assert!(
        (shown - ratio).abs() < 0.02,
        "b opened at {shown}, ratio {ratio}"
    );
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.split_ratio()),
        ratio,
        "an expand never writes"
    );
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1100.), px(700.)));
    frame(cx, s.win);
    frame(cx, s.win);
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.split_ratio()),
        ratio,
        "a resize never writes"
    );
}

#[gpui_kit::test]
fn a_drag_after_an_orientation_change_still_sets_the_ratio(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    let col = two_requests();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, a);
    give_result(cx, &ed);
    frame(cx, s.win);
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| st.response_orientation = Orientation::Horizontal)
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    // The orientation change replaces the split state, so this is the new entity.
    let split = ed.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |sp, _| sp.sizes()[0] + sp.sizes()[1]);
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, total * 0.7, w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    let ratio = s.ws.read_with(cx, |w, _| w.split_ratio());
    assert!(
        (ratio - 0.7).abs() < 0.02,
        "the drag after the switch was not stored: {ratio}"
    );
    assert!((share(cx, &ed) - ratio).abs() < 0.02);
}

#[gpui_kit::test]
fn the_divider_moves_when_the_room_is_short(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    let col = two_requests();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(360.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| {
                st.zoom = 2.0;
                st.response_orientation = Orientation::Vertical;
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    let ed = editor_of(cx, &s, a);
    give_result(cx, &ed);
    frame(cx, s.win);
    frame(cx, s.win);
    let split = ed.read_with(cx, |e, _| e.split().clone());
    let room = split.read_with(cx, |sp, _| sp.container_size());
    assert!(
        room < PANE_MIN * 2.,
        "the case needs a short room: {room:?}"
    );
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, px(1.), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    let first = split.read_with(cx, |sp, _| sp.sizes()[0]);
    assert!(
        first < room / 2. - px(1.),
        "the divider is pinned: {first:?} of {room:?}"
    );
    assert!(
        first >= room * 0.25 - px(0.5),
        "{first:?} under a quarter of {room:?}"
    );
}

#[gpui_kit::test]
fn a_tab_hidden_during_a_resize_opens_at_the_ratio(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed_a = editor_of(cx, &s, a);
    give_result(cx, &ed_a);
    frame(cx, s.win);
    frame(cx, s.win);
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    let ed_b = editor_of(cx, &s, b);
    give_result(cx, &ed_b);
    for _ in 0..3 {
        frame(cx, s.win);
    }
    // B is hidden while the window grows, so its split keeps the old room.
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(a)));
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1800.), px(1200.)));
    frame(cx, s.win);
    frame(cx, s.win);
    let split = ed_a.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |sp, _| sp.sizes()[0] + sp.sizes()[1]);
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, total * 0.3, w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    let ratio = s.ws.read_with(cx, |w, _| w.split_ratio());
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(b)));
    for _ in 0..4 {
        frame(cx, s.win);
    }
    let shown = share(cx, &ed_b);
    assert!((shown - ratio).abs() < 0.02, "b at {shown} not {ratio}");
}

// A drag in one editor tells every other open editor at once, without waiting for a render.
#[gpui_kit::test]
fn a_drag_notifies_the_other_open_editors(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed_a = editor_of(cx, &s, a);
    give_result(cx, &ed_a);
    frame(cx, s.win);
    frame(cx, s.win);
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    let ed_b = editor_of(cx, &s, b);
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(a)));
    frame(cx, s.win);
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = seen.clone();
    let _sub = cx.update(|cx| cx.observe(&ed_b, move |_, _| counter.set(counter.get() + 1)));
    let split = ed_a.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |sp, _| sp.sizes()[0] + sp.sizes()[1]);
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, total * 0.3, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(seen.get() > 0, "the hidden editor was not told of the drag");
}
