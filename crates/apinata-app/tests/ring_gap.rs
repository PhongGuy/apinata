//! A visible gap of one ring width between a ringed control and its nearest neighbour: their
//! bounds lie at least twice the ring's room apart, so the band never meets the neighbour.

use crate::git_support::Sandbox;
use crate::h::*;
use crate::support::*;

use apinata_app::request_editor::Section;
use apinata_app::style::FOCUS_RING_ROOM;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Pixels, TestAppContext, VisualTestContext, px, size,
};

// The space from `upper`'s bottom edge down to `lower`'s top edge, in the last frame.
fn gap_below(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    upper: &'static str,
    lower: &'static str,
) -> Pixels {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(lower).bounds().top() - w.find(upper).bounds().bottom()
    })
    .unwrap()
}

fn assert_gap(gap: Pixels, what: &str) {
    assert!(
        gap >= FOCUS_RING_ROOM * 2.,
        "{what}: {gap:?} apart, under twice the ring's room"
    );
}

#[gpui_kit::test]
fn the_body_select_clears_the_section_strip_above(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    ed.update(cx, |e, cx| e.show(Section::Body, cx));
    frame(cx, win);
    let gap = gap_below(cx, win, "section-strip", "body-mode");
    assert_gap(gap, "section strip to body select");
}

#[gpui_kit::test]
fn the_body_select_clears_the_body_editor_below(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(request("a", "http://x") + "  body:\n    type: json\n    data: '{}'\n"),
        ),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    ed.update(cx, |e, cx| e.show(Section::Body, cx));
    frame(cx, win);
    let gap = gap_below(cx, win, "body-mode", "body-code");
    assert_gap(gap, "body select to body editor");
}

// The Changes dialog at `w` by `h`, a long diff loaded, with its list's ring drawn.
fn changes_at(cx: &mut TestAppContext, w: f32, h: f32) -> Started {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    std::mem::forget(col);
    std::mem::forget(sb);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    open_on(cx, &s, std::path::Path::new(&s.root));
    click(cx, &s, ("changes-file", 0usize));
    idle(cx, &s);
    frame(cx, s.win);
    s
}

#[gpui_kit::test]
fn the_changes_list_clears_the_message_beside_the_diff(cx: &mut TestAppContext) {
    let s = changes_at(cx, 1000., 500.);
    let gap = gap_below(cx, s.win, "changes-files-ring", "changes-message");
    assert_gap(gap, "list to message, side by side");
}

// Stacked, the diff pane's chip row is the first box under the list.
#[gpui_kit::test]
fn the_changes_list_clears_the_diff_chips_when_stacked(cx: &mut TestAppContext) {
    let s = changes_at(cx, 500., 500.);
    let gap = gap_below(cx, s.win, "changes-files-ring", "changes-sections");
    assert_gap(gap, "list to diff chips, stacked");
}
