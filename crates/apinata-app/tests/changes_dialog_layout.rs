//! Changes dialog layout details: the empty sections row, the diff's six-row floor.

use crate::git_support::{Sandbox, write};
use crate::h::*;
use crate::support::*;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext};

fn bounds(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Bounds<Pixels> {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

// `a.yml` has a changed URL (one chip); `notes.txt` is not a request (no chips).
fn with_and_without_chips(cx: &mut TestAppContext) -> Started {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("notes.txt", "one\n"),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    write(col.path(), "notes.txt", "two\n");
    let s = git_started(cx, &sb, &col);
    std::mem::forget(col);
    open_on(cx, &s, std::path::Path::new(&s.root));
    s
}

fn set_zoom(cx: &mut TestAppContext, zoom: f32) {
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = zoom));
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_empty_sections_row_holds_a_chip_s_height_without_a_chip(cx: &mut TestAppContext) {
    spacer_matches_chip(cx, 1.0);
}

#[gpui_kit::test]
fn the_spacer_matches_the_chip_at_200_percent(cx: &mut TestAppContext) {
    spacer_matches_chip(cx, 2.0);
}

fn spacer_matches_chip(cx: &mut TestAppContext, zoom: f32) {
    let s = with_and_without_chips(cx);
    set_zoom(cx, zoom);
    let names: Vec<_> = view(cx).read_with(cx, |v, _| {
        v.files().iter().map(|(p, _)| p.clone()).collect()
    });
    let at = |n: &str| names.iter().position(|f| f.ends_with(n)).unwrap();
    click(cx, &s, ("changes-file", at("a.yml")));
    idle(cx, &s);
    let chip = bounds(cx, &s, ("changes-section", 0usize)).size.height;
    let with = bounds(cx, &s, "changes-sections").size.height;
    click(cx, &s, ("changes-file", at("notes.txt")));
    idle(cx, &s);
    assert!(!present(cx, s.win, ("changes-section", 0usize)));
    let spacer = bounds(cx, &s, "changes-sections-spacer").size.height;
    assert_eq!(spacer, chip, "the spacer is a chip tall");
    let without = bounds(cx, &s, "changes-sections").size.height;
    assert_eq!(without, with, "the row keeps its height");
}

#[test]
fn the_gutter_is_sized_from_the_measured_digit_advance() {
    use apinata_app::body_view::digit_gutter;
    use gpui_kit::px;
    // A wide-digit font (0.7 em at 10 px) gets 7 px a digit, not an estimated 6.
    assert_eq!(digit_gutter(px(7.), 3, px(8.)), px(29.));
    assert_eq!(digit_gutter(px(6.), 1, px(8.)), px(14.));
}

#[gpui_kit::test]
fn the_diff_gutter_matches_the_measured_digit_width(cx: &mut TestAppContext) {
    use apinata_app::body_view::digit_gutter;
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{Font, px};
    let s = with_and_without_chips(cx);
    let at = view(cx).read_with(cx, |v, _| {
        v.files()
            .iter()
            .position(|(p, _)| p.ends_with("a.yml"))
            .unwrap()
    });
    click(cx, &s, ("changes-file", at));
    idle(cx, &s);
    let (advance, pad) = cx
        .update_window(s.win, |_, w, cx| {
            let theme = cx.theme();
            let font = Font {
                family: theme.mono_font_family.clone(),
                ..w.text_style().font()
            };
            let id = w.text_system().resolve_font(&font);
            let size = theme.mono_font_size;
            let advance = w.text_system().advance(id, size, '0').unwrap().width;
            (
                advance,
                apinata_app::style::resolve(apinata_app::style::SPACE_2, w),
            )
        })
        .unwrap();
    // Both cells are right-aligned and one gutter wide, so their texts end a gutter apart.
    let old = bounds(cx, &s, ("diff-old", 1usize));
    let new = bounds(cx, &s, ("diff-new", 1usize));
    let (got, want) = (new.right() - old.right(), digit_gutter(advance, 1, pad));
    assert!((got - want).abs() <= px(0.5), "{got:?} against {want:?}");
}

// A long diff in a window `w` by `h` px at `zoom`; returns the pane's row and the diff's height.
fn long_diff_at(cx: &mut TestAppContext, w: f32, h: f32, zoom: f32) -> (Pixels, Pixels) {
    use gpui_kit::{VisualTestContext, px, size};
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    let long: String = (0..80).map(|i| format!("# line {i}\n")).collect();
    write(col.path(), "a.yml", &(request("a", "http://x/b") + &long));
    let s = git_started(cx, &sb, &col);
    std::mem::forget(col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    set_zoom(cx, zoom);
    open_on(cx, &s, std::path::Path::new(&s.root));
    click(cx, &s, ("changes-file", 0usize));
    idle(cx, &s);
    let row = cx
        .update_window(s.win, |_, w, _| {
            apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w)
        })
        .unwrap();
    (row, bounds(cx, &s, "changes-diff").size.height)
}

fn assert_six_rows(cx: &mut TestAppContext, w: f32, h: f32, zoom: f32) {
    let (row, diff) = long_diff_at(cx, w, h, zoom);
    assert!(
        diff >= row * 6.,
        "{w}x{h} at {zoom}: the diff is {diff:?}, under six rows of {row:?}"
    );
}

#[gpui_kit::test]
fn the_diff_keeps_six_rows_beside_the_list_at_1000_by_500(cx: &mut TestAppContext) {
    assert_six_rows(cx, 1000., 500., 1.0);
}

#[gpui_kit::test]
fn the_diff_keeps_six_rows_at_1000_by_500_and_200_percent(cx: &mut TestAppContext) {
    assert_six_rows(cx, 1000., 500., 2.0);
}

#[gpui_kit::test]
fn the_diff_keeps_six_rows_stacked_at_500_by_500(cx: &mut TestAppContext) {
    assert_six_rows(cx, 500., 500., 1.0);
}
