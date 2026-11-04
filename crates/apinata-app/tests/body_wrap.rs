//! The response body wraps long lines: a toggle in the bar, the Preferences default as its
//! start, and re-chunked rows for bodies too large for the editor.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::body_text::{EDITOR_MAX_LINES, ROW_WIDTH};
use apinata_app::body_view::{BodyView, Shown};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, SharedString, TestAppContext, WindowOptions,
};
use support::*;
use wiremock::ResponseTemplate;

fn host(
    cx: &mut TestAppContext,
    body: impl Into<Vec<u8>>,
    content_type: &str,
) -> (AnyWindowHandle, Entity<BodyView>) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(body.into(), content_type),
        "",
    );
    support::init(cx);
    open_view(cx, s)
}

fn open_view(cx: &mut TestAppContext, s: Sent) -> (AnyWindowHandle, Entity<BodyView>) {
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    (win, view)
}

fn long_json(len: usize) -> String {
    format!("{{\"k\":\"{}\"}}", "x".repeat(len))
}

fn present(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

#[gpui_kit::test]
fn the_editor_toggle_wraps_and_unwraps(cx: &mut TestAppContext) {
    let (win, view) = host(cx, long_json(3000), "application/json");
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert!(view.read_with(cx, |v, _| v.wrap()));
    assert!(present(cx, win, "body-wrap-toggle"));
    cx.update_window(win, |_, w, cx| w.click("body-wrap-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert!(!view.read_with(cx, |v, _| v.wrap()));
    cx.update_window(win, |_, w, cx| w.click("body-wrap-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert!(view.read_with(cx, |v, _| v.wrap()));
}

#[gpui_kit::test]
fn the_preference_sets_the_start_and_a_change_overrides_the_toggle(cx: &mut TestAppContext) {
    let sent = |cx: &mut TestAppContext| {
        send_once(
            cx,
            ResponseTemplate::new(200).set_body_raw(long_json(3000), "application/json"),
            "",
        )
    };
    let first = sent(cx);
    let second = sent(cx);
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let _app = started(cx, &col);
    let (win, view) = open_view(cx, first);
    let settings = cx.update(|cx| cx.global::<AppState>().settings.clone());
    settings.update(cx, |p, cx| p.update(cx, |s| s.wrap_editors = false));
    frame(cx, win);
    assert!(!view.read_with(cx, |v, _| v.wrap()));
    // A body opened under the off preference starts unwrapped.
    let (_, other) = open_view(cx, second);
    assert!(!other.read_with(cx, |v, _| v.wrap()));
    settings.update(cx, |p, cx| p.update(cx, |s| s.wrap_editors = true));
    frame(cx, win);
    assert!(view.read_with(cx, |v, _| v.wrap()));
}

#[gpui_kit::test]
fn binary_and_empty_bodies_offer_no_toggle(cx: &mut TestAppContext) {
    let (win, view) = host(
        cx,
        [0xff_u8, 0xfe, 0x00, 0x80].repeat(50),
        "application/octet-stream",
    );
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Binary(_)
    ));
    assert!(!present(cx, win, "body-wrap-toggle"));
}

fn big_single_line() -> String {
    "ab".repeat(150_000)
}

fn joined(rows: &[SharedString]) -> String {
    rows.iter().map(|r| r.as_ref()).collect()
}

#[gpui_kit::test]
fn large_rows_follow_the_column_budget_when_wrapping(cx: &mut TestAppContext) {
    let source = big_single_line();
    let (win, view) = host(cx, source.clone(), "text/plain");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    frame(cx, win);
    let (cols, rows) = view.read_with(cx, |v, _| (v.wrap_columns(), v.shown_rows().to_vec()));
    assert!(cols < ROW_WIDTH, "{cols}");
    assert!(rows.iter().all(|r| r.len() <= cols));
    assert_eq!(joined(&rows), source);
    // Only the first chunk of the line carries a number.
    let numbers = view.read_with(cx, |v, _| v.line_numbers().to_vec());
    assert_eq!(numbers[0], 1);
    assert!(numbers.len() > 1 && numbers[1..].iter().all(|n| *n == 0));
}

#[gpui_kit::test]
fn unwrapping_restores_the_fixed_chunks(cx: &mut TestAppContext) {
    let source = big_single_line();
    let (win, view) = host(cx, source.clone(), "text/plain");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let rows = view.read_with(cx, |v, _| v.shown_rows().to_vec());
    assert_eq!(view.read_with(cx, |v, _| v.wrap_columns()), ROW_WIDTH);
    assert_eq!(rows[0].len(), ROW_WIDTH);
    assert_eq!(joined(&rows), source);
}

#[gpui_kit::test]
fn find_still_matches_after_rechunking(cx: &mut TestAppContext) {
    let mut source = big_single_line();
    source.replace_range(5_000..5_006, "NEEDLE");
    let (win, view) = host(cx, source, "text/plain");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.open_find(w, cx);
            v.set_find_query("needle", w, cx);
        })
    })
    .unwrap();
    cx.executor()
        .advance_clock(apinata_app::body_view::SETTLE * 2);
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, cx| v.find_label(cx)),
        "1 of 1".to_string()
    );
}

#[test]
fn row_offsets_map_back_into_the_text_at_any_chunking() {
    use apinata_app::body_node::row_offset;
    use apinata_app::body_text::chunk_lines;
    let text = format!(
        "{{\n  \"a\": \"{}\",\r\n  \"b\": [1, 2]\n}}",
        "é".repeat(40)
    );
    for width in [7, 16, 1000] {
        let rows = chunk_lines(&text, width);
        let mut at = 0;
        for (ix, row) in rows.rows.iter().enumerate() {
            let offset = row_offset(&text, &rows.rows, ix).expect("a row maps back");
            let blank = row.len() - row.trim_start().len();
            assert_eq!(offset, at + blank, "width {width} row {ix}");
            assert!(text[offset..].starts_with(row.trim_start()), "{width}");
            // Move to the next row: past the line break only when this row ends its line.
            at += row.len();
            if rows.line_nos.get(ix + 1).is_none_or(|n| *n != 0) {
                at += if text[at..].starts_with("\r\n") { 2 } else { 1 };
            }
        }
    }
}

/// Re-cutting must stay far under a frame budget's worth of seconds even in a debug build;
/// measured at 11 ms for 4 MB on one line and 8 ms for 100k lines.
#[test]
fn rechunking_big_bodies_is_fast() {
    use apinata_app::body_text::chunk_lines;
    use std::time::Instant;
    let one_line = format!("[{}]", vec!["1234567"; 500_000].join(","));
    let many_lines = (0..100_000)
        .map(|i| format!("  \"key{i}\": {i},"))
        .collect::<Vec<_>>()
        .join("\n");
    for (name, text) in [("4 MB one line", one_line), ("100k lines", many_lines)] {
        let cut = chunk_lines(&text, ROW_WIDTH);
        let started = Instant::now();
        let narrow = cut.rechunk(96);
        let wrapped = started.elapsed();
        let started = Instant::now();
        let back = narrow.rechunk(ROW_WIDTH);
        let unwrapped = started.elapsed();
        assert!(narrow.len() >= cut.len(), "{name}");
        assert_eq!(back, cut);
        assert!(wrapped.as_secs() < 2 && unwrapped.as_secs() < 2);
    }
}

fn many_long_lines() -> String {
    (0..20_500)
        .map(|i| format!("line {i} {}", "w".repeat(300)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pins the combined defence: replacing the body cancels the pending re-cut, and the
/// generation guard drops any result that still lands. Breaking only one does not fail it.
#[gpui_kit::test(iterations = 8)]
fn a_new_body_arriving_mid_rechunk_wins(cx: &mut TestAppContext) {
    let a = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("ab".repeat(4_500_000), "text/plain"),
        "",
    );
    let other = "cd".repeat(160_000);
    let b = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(other.clone(), "text/plain"),
        "",
    );
    support::init(cx);
    let (win, view) = open_view(cx, a);
    // Unwrap and draw once so a re-cut is queued, then replace the body before it lands.
    let rb = b.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx));
        w.render_frame(cx);
        view.update(cx, |v, cx| v.show(Some(rb), w, cx));
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    let (rows, cols) = view.read_with(cx, |v, _| (v.shown_rows().to_vec(), v.wrap_columns()));
    assert_eq!(joined(&rows), other);
    assert!(rows.iter().all(|r| r.len() <= cols));
}

#[gpui_kit::test]
fn the_top_line_survives_a_width_change(cx: &mut TestAppContext) {
    let (win, view) = host(cx, many_long_lines(), "text/plain");
    query(cx, win, &view, "line 9000 ");
    frame(cx, win);
    let before = view.read_with(cx, |v, _| v.top_line()).expect("laid out");
    assert!(before > 8000, "{before}");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.wrap_columns()), ROW_WIDTH);
    assert_eq!(view.read_with(cx, |v, _| v.top_line()), Some(before));
}

#[gpui_kit::test]
fn unwrapped_rows_scroll_sideways(cx: &mut TestAppContext) {
    let (win, view) = host(cx, big_single_line(), "text/plain");
    let wrapped = view.read_with(cx, |v, _| v.rows_max_scroll());
    assert_eq!(wrapped.x, gpui_kit::px(0.));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert!(view.read_with(cx, |v, _| v.rows_max_scroll()).x > gpui_kit::px(0.));
}

#[gpui_kit::test]
fn find_keeps_its_place_across_a_rechunk(cx: &mut TestAppContext) {
    let body = (0..60)
        .map(|i| format!("{} needle{i}", "a".repeat(5_000)))
        .collect::<Vec<_>>()
        .join("\n")
        + &"\nz".repeat(EDITOR_MAX_LINES);
    let (win, view) = host(cx, body, "text/plain");
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 60");
    for _ in 0..6 {
        cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.find_next(w, cx)))
            .unwrap();
    }
    assert_eq!(label(cx, &view), "7 of 60");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(label(cx, &view), "7 of 60");
}

#[gpui_kit::test]
fn find_matches_across_a_chunk_seam_once(cx: &mut TestAppContext) {
    let mut source = big_single_line();
    source.replace_range(ROW_WIDTH - 3..ROW_WIDTH + 3, "NEEDLE");
    let (win, view) = host(cx, source, "text/plain");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 1");
}

fn query(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, q: &str) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.open_find(w, cx);
            v.set_find_query(q, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
}

fn label(cx: &mut TestAppContext, view: &Entity<BodyView>) -> String {
    view.read_with(cx, |v, cx| v.find_label(cx))
}

fn chunks(text: &str, width: usize) -> apinata_app::body_text::Chunked {
    apinata_app::body_text::chunk_lines(text, width)
}

type Found = (
    Vec<(usize, std::ops::Range<usize>)>,
    Vec<(usize, usize, std::ops::Range<usize>)>,
);

fn found(text: &str, width: usize, q: &str, cs: bool) -> Found {
    let r = apinata_app::body_find::find_in_chunked(&chunks(text, width), q, cs);
    (r.matches, r.tails)
}

#[test]
fn find_in_chunked_respects_case() {
    assert_eq!(
        found("aBc ABC", 100, "abc", false).0,
        [(0, 0..3), (0, 4..7)]
    );
    assert_eq!(found("aBc ABC", 100, "ABC", true).0, [(0, 4..7)]);
}

#[test]
fn find_in_chunked_matches_in_the_first_and_last_chunk() {
    let (m, t) = found("xxabxxxxxxxxxxxxxxxxab", 8, "ab", true);
    assert_eq!(m, [(0, 2..4), (2, 4..6)]);
    assert!(t.is_empty());
}

#[test]
fn find_in_chunked_spans_three_rows_as_one_match() {
    // Rows of 4: "0123" "4567" "89ab"; the match covers 2..10.
    let (m, t) = found("0123456789ab", 4, "23456789", true);
    assert_eq!(m, [(0, 2..4)]);
    assert_eq!(t, [(0, 1, 0..4), (0, 2, 0..2)]);
}

#[test]
fn find_in_chunked_does_not_overlap_matches() {
    assert_eq!(found("aaa", 100, "aa", true).0, [(0, 0..2)]);
    // "aaaa" cut at 3 is "aaa" "a"; the second match runs over the seam.
    let (m, t) = found("aaaa", 3, "aa", true);
    assert_eq!(m, [(0, 0..2), (0, 2..3)]);
    assert_eq!(t, [(1, 1, 0..1)]);
}

#[test]
fn find_in_chunked_ignores_an_empty_query_and_empty_rows() {
    assert_eq!(found("abc", 2, "", false), (vec![], vec![]));
    assert_eq!(found("", 4, "a", false), (vec![], vec![]));
    let (m, _) = found("\n\nab\n", 4, "ab", true);
    assert_eq!(m, [(2, 0..2)]);
}

#[test]
fn find_in_chunked_keeps_ranges_on_char_boundaries_at_a_seam() {
    // Width 3 cuts "aéé" into "aé" "é"; the needle "éé" crosses that seam.
    let text = "aéé";
    let c = chunks(text, 3);
    assert_eq!(c.rows.len(), 2);
    let r = apinata_app::body_find::find_in_chunked(&c, "éé", true);
    assert_eq!(r.matches, [(0, 1..3)]);
    assert_eq!(r.tails, [(0, 1, 0..2)]);
    for (row, range) in r.matches.iter().map(|(r, g)| (*r, g.clone())) {
        assert!(
            c.rows[row].is_char_boundary(range.start) && c.rows[row].is_char_boundary(range.end)
        );
    }
}

#[test]
fn the_widest_row_is_the_widest_in_cells_not_in_bytes() {
    // 900 narrow cells in 900 bytes against 330 double-width cells (660) in 990 bytes.
    let text = format!("{}\n{}", "a".repeat(900), "\u{4f60}".repeat(330));
    assert_eq!(chunks(&text, ROW_WIDTH).widest(), 0);
    let text = format!("{}\n{}", "a".repeat(500), "\u{4f60}".repeat(330));
    assert_eq!(chunks(&text, ROW_WIDTH).widest(), 1);
}

// A text/plain body of many short lines, so the rows list shows it, with `line` at row `at`.
fn rows_body_with(at: usize, line: &str) -> String {
    let mut lines = vec!["z".to_owned(); EDITOR_MAX_LINES + 10];
    lines[at] = line.to_owned();
    lines.join("\n")
}

fn needle_line(col: usize) -> String {
    format!("{}needle{}", "x".repeat(col), "y".repeat(1500))
}

fn unwrap_view(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    cx.run_until_parked();
    frame(cx, win);
}

fn advance(cx: &mut TestAppContext, win: AnyWindowHandle) -> f32 {
    cx.update_window(win, |_, w, cx| {
        f32::from(apinata_app::body_view::mono_advance(
            BodyView::font_size(cx),
            w,
            cx,
        ))
    })
    .unwrap()
}

// Whether the rows list shows the column span `start..end` of its row with the gutter ahead.
fn rows_show_columns(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<BodyView>,
    start: usize,
    end: usize,
) -> bool {
    let adv = advance(cx, win);
    let (offset, width) = view.read_with(cx, |v, _| {
        (
            f32::from(v.rows_scroll_offset().x),
            f32::from(v.rows_viewport_width()),
        )
    });
    let (left, right) = (-offset, -offset + width);
    start as f32 * adv >= left && end as f32 * adv <= right
}

#[gpui_kit::test]
fn the_rows_scroll_to_the_end_of_their_widest_row(cx: &mut TestAppContext) {
    // The CJK row has more bytes (990) but fewer cells (660) than the ASCII row (700).
    let mut lines = vec!["z".to_owned(); EDITOR_MAX_LINES + 10];
    lines[5] = "a".repeat(700);
    lines[7] = "\u{4f60}".repeat(330);
    let (win, view) = host(cx, lines.join("\n"), "text/plain");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    unwrap_view(cx, win, &view);
    let adv = advance(cx, win);
    let (max, width) = view.read_with(cx, |v, _| {
        (
            f32::from(v.rows_max_scroll().x),
            f32::from(v.rows_viewport_width()),
        )
    });
    assert!(
        max + width >= 700. * adv,
        "{max} + {width} < {}",
        700. * adv
    );
}

#[gpui_kit::test]
fn a_toggle_keeps_the_rows_match_in_view(cx: &mut TestAppContext) {
    let (win, view) = host(cx, rows_body_with(5000, &needle_line(900)), "text/plain");
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 1");
    unwrap_view(cx, win, &view);
    assert_eq!(label(cx, &view), "1 of 1");
    assert!(rows_show_columns(cx, win, &view, 900, 906));
    let top = view.read_with(cx, |v, _| v.top_line()).unwrap();
    assert!((4980..=5001).contains(&top), "{top}");
}

#[gpui_kit::test]
fn find_next_scrolls_unwrapped_rows_sideways(cx: &mut TestAppContext) {
    let mut lines = vec!["z".to_owned(); EDITOR_MAX_LINES + 10];
    lines[100] = "needle".into();
    lines[100 + 3000] = needle_line(900);
    let (win, view) = host(cx, lines.join("\n"), "text/plain");
    unwrap_view(cx, win, &view);
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 2");
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.find_next(w, cx)))
        .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(label(cx, &view), "2 of 2");
    assert!(rows_show_columns(cx, win, &view, 900, 906));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.find_previous(w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(label(cx, &view), "1 of 2");
    assert!(rows_show_columns(cx, win, &view, 0, 6));
}

// Whether the editor shows the byte range `range` of its text.
fn editor_shows(
    cx: &mut TestAppContext,
    view: &Entity<BodyView>,
    range: std::ops::Range<usize>,
) -> bool {
    let editor = view.read_with(cx, |v, _| v.editor().clone());
    editor.read_with(cx, |e, _| {
        let view = e.input_bounds();
        e.range_to_bounds(&range).is_some_and(|b| {
            b.left() >= view.left()
                && b.right() <= view.right()
                && b.top() >= view.top()
                && b.bottom() <= view.bottom()
        })
    })
}

fn editor_body() -> (String, usize) {
    let mut lines: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
    lines[30] = format!("{}needle", "x".repeat(3000));
    let at = lines[..30].iter().map(|l| l.len() + 1).sum::<usize>() + 3000;
    (lines.join("\n"), at)
}

#[gpui_kit::test]
fn a_toggle_keeps_the_editor_match_in_view(cx: &mut TestAppContext) {
    let (body, at) = editor_body();
    let (win, view) = host(cx, body, "text/plain");
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    query(cx, win, &view, "needle");
    frame(cx, win);
    frame(cx, win);
    assert_eq!(label(cx, &view), "1 of 1");
    assert!(editor_shows(cx, &view, at..at + 6));
    unwrap_view(cx, win, &view);
    assert_eq!(label(cx, &view), "1 of 1");
    assert!(editor_shows(cx, &view, at..at + 6));
}

// A regression pin, not red-first: the kit already scrolls the editor sideways to a stepped match.
#[gpui_kit::test]
fn find_next_scrolls_the_unwrapped_editor_sideways(cx: &mut TestAppContext) {
    let (body, at) = editor_body();
    let (win, view) = host(cx, format!("needle\n{body}"), "text/plain");
    unwrap_view(cx, win, &view);
    query(cx, win, &view, "needle");
    frame(cx, win);
    assert_eq!(label(cx, &view), "1 of 2");
    assert!(editor_shows(cx, &view, 0..6));
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.find_next(w, cx)))
        .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(label(cx, &view), "2 of 2");
    assert!(editor_shows(cx, &view, at + 7..at + 13));
}

#[gpui_kit::test]
fn an_unwrapped_line_past_the_editor_limit_shows_as_rows(cx: &mut TestAppContext) {
    let (win, view) = host(cx, long_json(21_000), "application/json");
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    unwrap_view(cx, win, &view);
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(true, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
}

#[gpui_kit::test]
fn an_unwrapped_short_line_stays_in_the_editor(cx: &mut TestAppContext) {
    let (win, view) = host(cx, long_json(3_000), "application/json");
    unwrap_view(cx, win, &view);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
}

#[gpui_kit::test]
fn find_keeps_its_place_when_a_wide_line_swaps_between_editor_and_rows(cx: &mut TestAppContext) {
    let body = format!("{}needle{}", "x".repeat(20_500), "y".repeat(400));
    let (win, view) = host(cx, body, "text/plain");
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 1");
    unwrap_view(cx, win, &view);
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    assert_eq!(label(cx, &view), "1 of 1");
    assert!(rows_show_columns(cx, win, &view, 500, 506));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(true, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert_eq!(label(cx, &view), "1 of 1");
    assert!(editor_shows(cx, &view, 20_500..20_506));
}
