//! The pure logic of moving a table row: where a drop lands, the Alt step, and where focus
//! follows a row when an undo or redo moves it.

use apinata_app::row_move::{
    drop_side, follow, grip_color, move_target, real_move_target, real_step_target, reorder,
    reorder_pinned, step_target,
};
use apinata_app::tabs::Side;
use gpui_kit::{Bounds, point, px, size};

#[test]
fn a_drop_after_a_lower_row_lands_on_that_row() {
    assert_eq!(move_target(4, 0, 2, Side::After), Some(2));
    assert_eq!(move_target(4, 0, 2, Side::Before), Some(1));
}

#[test]
fn a_drop_above_an_earlier_row_lands_on_it() {
    assert_eq!(move_target(4, 3, 1, Side::Before), Some(1));
    assert_eq!(move_target(4, 3, 1, Side::After), Some(2));
}

#[test]
fn a_drop_that_leaves_the_row_in_place_is_no_move() {
    assert_eq!(move_target(3, 1, 1, Side::Before), None);
    assert_eq!(move_target(3, 1, 1, Side::After), None);
    assert_eq!(move_target(3, 1, 0, Side::After), None);
    assert_eq!(move_target(3, 1, 2, Side::Before), None);
}

#[test]
fn a_row_or_target_outside_the_table_is_no_move() {
    assert_eq!(move_target(3, 3, 0, Side::Before), None);
    assert_eq!(move_target(3, 0, 3, Side::Before), None);
}

#[test]
fn a_step_swaps_with_the_neighbour_and_stops_at_the_ends() {
    assert_eq!(step_target(3, 1, true), Some(0));
    assert_eq!(step_target(3, 1, false), Some(2));
    assert_eq!(step_target(3, 0, true), None);
    assert_eq!(step_target(3, 2, false), None);
    assert_eq!(step_target(0, 0, false), None);
}

#[test]
fn reorder_moves_one_element_to_its_destination() {
    let mut v = vec!['a', 'b', 'c', 'd'];
    reorder(&mut v, 0, 2);
    assert_eq!(v, ['b', 'c', 'a', 'd']);
    reorder(&mut v, 3, 0);
    assert_eq!(v, ['d', 'b', 'c', 'a']);
}

#[test]
fn the_upper_half_of_a_row_is_before_and_the_lower_after() {
    let b = Bounds::new(point(px(0.), px(100.)), size(px(80.), px(20.)));
    assert_eq!(drop_side(px(105.), b), Side::Before);
    assert_eq!(drop_side(px(115.), b), Side::After);
}

#[test]
fn focus_follows_a_row_an_undo_moved_back() {
    // Row 2 (c) was moved from 0; the undo shows the original order, c is back at 0.
    let current = ['b', 'a', 'c'];
    let incoming = ['a', 'b', 'c'];
    assert_eq!(
        follow(&current, &incoming, 0, |_| false, |a, b| a == b),
        Some(1)
    );
    assert_eq!(
        follow(&current, &incoming, 1, |_| false, |a, b| a == b),
        Some(0)
    );
}

#[test]
fn focus_stays_when_nothing_moved_or_the_load_is_another_edit() {
    let same = ['a', 'a', 'b'];
    assert_eq!(follow(&same, &same, 0, |_| false, |a, b| a == b), None);
    assert_eq!(
        follow(&['a', 'b'], &['a', 'x'], 0, |_| false, |a, b| a == b),
        None
    );
    assert_eq!(
        follow(&['a', 'b'], &['a', 'b', 'c'], 0, |_| false, |a, b| a == b),
        None
    );
    assert_eq!(
        follow(&['a', 'b'], &['b', 'a'], 5, |_| false, |a, b| a == b),
        None
    );
}

#[test]
fn a_step_skips_over_pinned_rows_to_the_next_real_row() {
    // q0, P, q2, P, q4: Alt+Up on q2 swaps with q0 across the pinned row, and so on.
    let pinned = [false, true, false, true, false];
    assert_eq!(real_step_target(&pinned, 2, true), Some(0));
    assert_eq!(real_step_target(&pinned, 2, false), Some(4));
    assert_eq!(real_step_target(&pinned, 0, true), None);
    assert_eq!(real_step_target(&pinned, 4, false), None);
    assert_eq!(
        real_step_target(&pinned, 1, true),
        None,
        "a pinned row never moves"
    );
}

#[test]
fn a_drop_is_measured_over_real_rows_only() {
    let pinned = [false, true, false];
    assert_eq!(real_move_target(&pinned, 0, 2, Side::After), Some(2));
    assert_eq!(real_move_target(&pinned, 2, 0, Side::Before), Some(0));
    assert_eq!(
        real_move_target(&pinned, 0, 1, Side::After),
        None,
        "onto a pinned row"
    );
    assert_eq!(
        real_move_target(&pinned, 1, 2, Side::After),
        None,
        "from a pinned row"
    );
    assert_eq!(
        real_move_target(&pinned, 2, 0, Side::After),
        None,
        "no change"
    );
}

#[test]
fn reorder_pinned_keeps_every_pinned_row_at_its_index() {
    let mut v = vec!['a', 'P', 'b', 'c'];
    reorder_pinned(&mut v, |c| *c == 'P', 2, 0);
    assert_eq!(v, ['b', 'P', 'a', 'c']);
    reorder_pinned(&mut v, |c| *c == 'P', 0, 3);
    assert_eq!(v, ['a', 'P', 'c', 'b']);
}

// The kit paints a ghost icon button in `secondary_foreground` but offers no way to read the
// painted colour, so the real check is legibility: the grip reads strongly against the page
// in every shipped theme, which the old `muted_foreground` grip did not in the light one.
#[gpui_kit::test]
fn the_grip_is_reads_strongly_against_the_page_in_every_theme(cx: &mut gpui_kit::TestAppContext) {
    use crate::support;
    use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
    use gpui_kit::component::ActiveTheme as _;
    for name in [DARK, LIGHT, HC_DARK, HC_LIGHT] {
        support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let page = t.background;
            assert!(support::contrast(grip_color(cx), page) >= 7.0, "{name}");
        });
    }
}

#[test]
fn focus_follows_a_row_moved_across_a_pinned_row() {
    // 'P' keeps slot 1; the real rows a, c swapped, so the undo shows c, P, a back as a, P, c.
    let current = ['c', 'P', 'a'];
    let incoming = ['a', 'P', 'c'];
    let pinned = |c: &char| *c == 'P';
    assert_eq!(
        follow(&current, &incoming, 0, pinned, |a, b| a == b),
        Some(2)
    );
    assert_eq!(
        follow(&current, &incoming, 2, pinned, |a, b| a == b),
        Some(0)
    );
    assert_eq!(follow(&current, &incoming, 1, pinned, |a, b| a == b), None);
}
