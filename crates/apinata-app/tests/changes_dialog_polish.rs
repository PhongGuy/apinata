//! Changes dialog polish: the commit notice lifetime, partly staged rows, the staged count,
//! the bulk button's tab stop and the failure notice's height cap.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::changes_dialog::Notice;
use apinata_app::git_status::GitStage::{Both, Staged, StagedGone, Unstaged};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext, VisualTestContext, px, size};
use h::*;
use support::*;

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

fn set_message(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| {
        current_view(cx).update(cx, |v, cx| v.set_message(text, w, cx))
    })
    .unwrap();
}

fn current_view(
    cx: &mut gpui_kit::App,
) -> gpui_kit::Entity<apinata_app::changes_dialog::ChangesView> {
    apinata_app::changes_dialog::current(cx).expect("the dialog is open")
}

#[gpui_kit::test]
fn the_committed_notice_clears_on_the_next_stage(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    let done = view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(matches!(done, Some(Notice::Committed(_))));
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    assert_eq!(view(cx).read_with(cx, |v, _| v.notice().cloned()), None);
}

#[gpui_kit::test]
fn the_summary_counts_only_fully_staged_files(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    sb.git(col.path(), &["add", "a.yml"]);
    write(col.path(), "a.yml", &request("a", "http://x/again"));
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Both, Unstaged]);
    assert_eq!(
        label(cx, &s, "changes-summary").as_deref(),
        Some("0 staged of 2")
    );
}

#[gpui_kit::test]
fn the_bulk_button_is_a_tab_stop_and_space_activates_it(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 0usize));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.press("tab", cx))
        .unwrap();
    frame(cx, s.win);
    let on_button = cx
        .update_window(s.win, |_, w, _| w.find("changes-toggle-all").focused())
        .unwrap();
    assert_eq!(on_button, Some(true), "Tab from the list reaches Stage all");
    cx.update_window(s.win, |_, w, cx| w.press("space", cx))
        .unwrap();
    idle(cx, &s);
    wait_for(cx, "all staged", |cx| stages(cx) == [Staged, Staged]);
}

#[gpui_kit::test]
fn the_bulk_button_is_a_tab_stop_and_enter_activates_it(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 0usize));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.press("tab", cx))
        .unwrap();
    frame(cx, s.win);
    let on_button = cx
        .update_window(s.win, |_, w, _| w.find("changes-toggle-all").focused())
        .unwrap();
    assert_eq!(on_button, Some(true), "Tab from the list reaches Stage all");
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    idle(cx, &s);
    wait_for(cx, "all staged", |cx| stages(cx) == [Staged, Staged]);
}

#[gpui_kit::test]
fn a_partly_staged_row_keeps_its_file_name_whole(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let dir = "a-rather-long-directory-name/with-a-nested-folder";
    let other = "another-rather-long-directory/with-a-nested-folder";
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            (&format!("{dir}/request.yml"), &request("a", "http://x/a")),
            (&format!("{other}/request.yml"), &request("b", "http://x/b")),
        ],
    );
    write(
        col.path(),
        &format!("{dir}/request.yml"),
        &request("a", "http://x/1"),
    );
    sb.git(col.path(), &["add", "."]);
    write(
        col.path(),
        &format!("{dir}/request.yml"),
        &request("a", "http://x/2"),
    );
    write(
        col.path(),
        &format!("{other}/request.yml"),
        &request("b", "http://x/3"),
    );
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Both, Unstaged]);
    let partly = bounds(cx, &s, ("changes-file-name", 0usize));
    let plain = bounds(cx, &s, ("changes-file-name", 1usize));
    assert_eq!(
        partly.size.width, plain.size.width,
        "the name is not squeezed"
    );
    assert!(plain.size.width > px(40.), "{plain:?}");
    let label = bounds(cx, &s, ("changes-partly", 0usize));
    // The label has its own line under the name, so neither is clipped by the other.
    assert!(
        label.top() >= partly.bottom() - px(1.),
        "{partly:?} {label:?}"
    );
    // A name that does not fit beside its directory takes the line under it.
    let dir_b = bounds(cx, &s, ("changes-file-dir", 0usize));
    assert!(
        dir_b.bottom() <= partly.top() + px(1.),
        "{dir_b:?} {partly:?}"
    );
}

// Fails a commit with a long hook notice in a window `height` px tall, then checks the layout.
#[cfg(unix)]
fn long_failure_in(cx: &mut TestAppContext, height: f32) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let body = format!(
        "#!/bin/sh\n{}exit 1\n",
        "echo 'lint failed on a long line'  >&2\n".repeat(40)
    );
    h::hook(col.path(), "pre-commit", &body);
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(height)));
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    let failed = view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(matches!(failed, Some(Notice::Failed(_))));
    let row = bounds(cx, &s, ("changes-file", 0usize)).size.height;
    let diff = bounds(cx, &s, "changes-diff").size.height;
    assert!(diff >= row * 4., "diff {diff:?} row {row:?}");
    let commit = bounds(cx, &s, "changes-commit");
    assert!(commit.bottom() <= px(height), "{commit:?}");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_long_failure_notice_leaves_the_diff_four_rows_in_a_small_window(cx: &mut TestAppContext) {
    long_failure_in(cx, 500.);
}

// At 400 px the notice cap is what keeps the footer on screen; uncapped it ends near 491.
#[cfg(unix)]
#[gpui_kit::test]
fn a_long_failure_notice_is_capped_in_a_very_short_window(cx: &mut TestAppContext) {
    long_failure_in(cx, 400.);
}

// A modified file with part of the change staged, in a window `height` px tall.
fn partly_staged_in(cx: &mut TestAppContext, height: f32) -> Started {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("users/list.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "users/list.yml", &request("a", "http://x/1"));
    sb.git(col.path(), &["add", "."]);
    write(col.path(), "users/list.yml", &request("a", "http://x/2"));
    let s = git_started(cx, &sb, &col);
    // The collection must outlive the dialog it feeds.
    std::mem::forget(col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(height)));
    open_on(cx, &s, std::path::Path::new(&s.root));
    assert_eq!(stages(cx), [Both]);
    s
}

fn assert_partly_label_fits(cx: &mut TestAppContext, height: f32) {
    let s = partly_staged_in(cx, height);
    let row = bounds(cx, &s, ("changes-file", 0usize));
    let label = bounds(cx, &s, ("changes-partly", 0usize));
    assert!(
        label.left() >= row.left() && label.right() <= row.right(),
        "at {height}px the label {label:?} sticks out of its row {row:?}"
    );
    // The text is its natural width: a clipped label would be narrower than "partly staged".
    assert!(label.size.width >= px(60.), "label squeezed: {label:?}");
}

#[gpui_kit::test]
fn the_partly_staged_label_fits_its_row_at_1000_by_500(cx: &mut TestAppContext) {
    assert_partly_label_fits(cx, 500.);
}

#[gpui_kit::test]
fn the_partly_staged_label_fits_its_row_at_1000_by_400(cx: &mut TestAppContext) {
    assert_partly_label_fits(cx, 400.);
}

// The footer buttons stay inside the dialog frame, whatever the failure notice's length.
#[cfg(unix)]
#[gpui_kit::test]
fn the_footer_stays_inside_the_dialog_with_a_long_notice_at_1000_by_400(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let body = format!(
        "#!/bin/sh\n{}exit 1\n",
        "echo 'lint failed on a long line'  >&2\n".repeat(80)
    );
    h::hook(col.path(), "pre-commit", &body);
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(400.)));
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    let notice = view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(matches!(notice, Some(Notice::Failed(_))));
    let dialog = bounds(cx, &s, "changes-dialog");
    let window = Bounds::from_corners(
        gpui_kit::point(px(0.), px(0.)),
        gpui_kit::point(px(1000.), px(400.)),
    );
    for id in ["changes-commit", "changes-close"] {
        let b = bounds(cx, &s, id);
        assert!(
            b.bottom() <= dialog.bottom() && b.bottom() <= window.bottom(),
            "{id} {b:?} below the dialog {dialog:?} in window {window:?}"
        );
    }
}

const LONG_DIR: &str = "requests";
const LONG_NAME: &str = "a-request-with-a-very-long-descriptive-file-name-for-the-list.yml";

// A left and right edge in pixels.
type Span = (f32, f32);

// Opens Changes in an 800 px window over a changed `a.yml` and, with `long`, a long-named
// change; returns the short name's span, the long name's span if any, and the list's span.
fn name_spans(cx: &mut TestAppContext, long: bool) -> (Span, Option<Span>, Span) {
    let sb = Sandbox::new();
    let path = format!("{LONG_DIR}/{LONG_NAME}");
    let mut files = vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("a.yml", request("a", "http://x/a")),
    ];
    if long {
        files.push((path.as_str(), request("l", "http://x/l")));
    }
    let files: Vec<_> = files.iter().map(|(p, b)| (*p, b.as_str())).collect();
    let col = committed(&sb, &files);
    write(col.path(), "a.yml", &request("a", "http://x/1"));
    if long {
        write(col.path(), &path, &request("l", "http://x/2"));
    }
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(800.), px(700.)));
    frame(cx, s.win);
    open_on(cx, &s, col.path());
    let span = |b: Bounds<Pixels>| (b.left().as_f32(), b.right().as_f32());
    let short = span(bounds(cx, &s, ("changes-file-name", 0usize)));
    let long = long.then(|| span(bounds(cx, &s, ("changes-file-name", 1usize))));
    let list = span(bounds(cx, &s, "changes-rows"));
    (short, long, list)
}

// A long file name ellipsizes inside the list once its directory has given way, instead of
// drawing past the list's edge; a short name beside it stays whole.
#[gpui_kit::test]
fn a_long_file_name_ellipsizes_inside_the_list(cx: &mut TestAppContext) {
    let (alone, _, _) = name_spans(cx, false);
    let (short, long, list) = name_spans(cx, true);
    let long = long.expect("the long row");
    assert!(
        long.1 <= list.1 + 0.5,
        "the name {long:?} draws past the list's right edge {list:?}"
    );
    // The whole name is far wider than the list, so a name that ends inside it is cut.
    assert!(
        long.1 - long.0 < list.1 - list.0,
        "the name {long:?} is not cut to the list {list:?}"
    );
    assert!(
        ((short.1 - short.0) - (alone.1 - alone.0)).abs() < 0.5,
        "the short name {short:?} is cut from {alone:?}"
    );
}

// Hovering a row's name shows its full relative path, since the row may cut it short.
#[gpui_kit::test]
fn hovering_a_file_name_shows_its_full_path(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let long = format!("{LONG_DIR}/{LONG_NAME}");
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            (&long, &request("l", "http://x/l")),
        ],
    );
    write(col.path(), &long, &request("l", "http://x/2"));
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(800.), px(700.)));
    open_on(cx, &s, col.path());
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("changes-file-name", 0usize), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(
        label(cx, &s, "changes-file-tooltip").as_deref(),
        Some(long.as_str()),
        "the hovered name's tooltip is its full path"
    );
}

/// `r.yml` renamed to `s.yml` in the index and `b.yml` modified, opened in the dialog.
fn renamed_row(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    sb.git(col.path(), &["mv", "r.yml", "s.yml"]);
    write(col.path(), "b.yml", &request("b", "http://x/2"));
    let s = git_started(cx, sb, col);
    open_on(cx, &s, col.path());
    s
}

fn renamable(sb: &Sandbox) -> tempfile::TempDir {
    committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &request("r", "http://x/r")),
            ("b.yml", &request("b", "http://x/b")),
        ],
    )
}

#[gpui_kit::test]
fn a_renamed_row_names_its_origin_on_a_second_line(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = renamable(&sb);
    let s = renamed_row(cx, &sb, &col);
    assert_eq!(
        label(cx, &s, ("changes-from", 1usize)).as_deref(),
        Some("from r.yml")
    );
    let name = bounds(cx, &s, ("changes-file-name", 1usize));
    let from = bounds(cx, &s, ("changes-from", 1usize));
    let row = bounds(cx, &s, ("changes-file", 1usize));
    assert!(from.top() >= name.bottom() - px(1.), "{name:?} {from:?}");
    assert!(from.bottom() <= row.bottom() + px(1.), "{from:?} {row:?}");
}

#[gpui_kit::test]
fn only_a_renamed_row_has_the_origin_line(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = renamable(&sb);
    let s = renamed_row(cx, &sb, &col);
    assert!(!present(cx, s.win, ("changes-from", 0usize)));
    let plain = bounds(cx, &s, ("changes-file", 0usize));
    assert!(bounds(cx, &s, ("changes-file", 1usize)).size.height > plain.size.height);
}

#[gpui_kit::test]
fn a_renamed_row_reads_out_its_origin(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = renamable(&sb);
    let s = renamed_row(cx, &sb, &col);
    assert_eq!(
        label(cx, &s, ("changes-file", 1usize)).as_deref(),
        Some("renamed s.yml from r.yml")
    );
    assert_eq!(
        label(cx, &s, ("changes-file", 0usize)).as_deref(),
        Some("modified b.yml")
    );
}

#[gpui_kit::test]
fn a_renamed_and_partly_staged_row_keeps_both_lines_clear(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = renamable(&sb);
    sb.git(col.path(), &["mv", "r.yml", "s.yml"]);
    let body = format!("{}# edited\n", request("r", "http://x/r"));
    write(col.path(), "s.yml", &body);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Both]);
    let name = bounds(cx, &s, ("changes-file-name", 0usize));
    let from = bounds(cx, &s, ("changes-from", 0usize));
    let partly = bounds(cx, &s, ("changes-partly", 0usize));
    let row = bounds(cx, &s, ("changes-file", 0usize));
    assert!(from.top() >= name.bottom() - px(1.), "{name:?} {from:?}");
    assert!(
        partly.top() >= from.bottom() - px(1.),
        "{from:?} {partly:?}"
    );
    assert!(
        partly.bottom() <= row.bottom() + px(1.),
        "{partly:?} {row:?}"
    );
}

#[gpui_kit::test]
fn a_long_origin_truncates_and_its_tooltip_is_the_whole_path(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let long = format!("{LONG_DIR}/{LONG_NAME}");
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            (&long, &request("l", "http://x/l")),
        ],
    );
    sb.git(col.path(), &["mv", &long, "s.yml"]);
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(800.), px(700.)));
    open_on(cx, &s, col.path());
    let from = bounds(cx, &s, ("changes-from", 0usize));
    let row = bounds(cx, &s, ("changes-file", 0usize));
    assert!(from.right() <= row.right(), "{from:?} {row:?}");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("changes-from", 0usize), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(
        label(cx, &s, "changes-from-tooltip").as_deref(),
        Some(long.as_str())
    );
}

// Beside a modified and an untracked file, `gone.yml` staged as new and then removed from disk.
fn with_a_staged_file_gone(sb: &Sandbox) -> tempfile::TempDir {
    let col = changed(sb);
    write(col.path(), "gone.yml", &request("g", "http://x/g"));
    sb.git(col.path(), &["add", "gone.yml"]);
    std::fs::remove_file(col.path().join("gone.yml")).unwrap();
    col
}

#[gpui_kit::test]
fn a_staged_file_gone_from_disk_says_so_on_a_tall_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_file_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Unstaged, StagedGone, Unstaged]);
    let note = ("changes-gone", 1usize);
    assert_eq!(
        label(cx, &s, note).as_deref(),
        Some("staged, gone from its path")
    );
    assert!(!present(cx, s.win, ("changes-partly", 1usize)));
    assert_eq!(
        label(cx, &s, ("changes-file", 1usize)).as_deref(),
        Some("added gone.yml, staged, gone from its path")
    );
    let (name, note) = (
        bounds(cx, &s, ("changes-file-name", 1usize)),
        bounds(cx, &s, note),
    );
    let row = bounds(cx, &s, ("changes-file", 1usize));
    assert!(note.top() >= name.bottom() - px(1.), "{name:?} {note:?}");
    assert!(note.bottom() <= row.bottom() + px(1.), "{note:?} {row:?}");
    assert!(row.size.height > bounds(cx, &s, ("changes-file", 0usize)).size.height);
    assert!(note.size.width >= px(100.), "label squeezed: {note:?}");
}

#[gpui_kit::test]
fn a_staged_file_gone_from_disk_counts_as_staged(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_file_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(view(cx).read_with(cx, |v, _| v.staged_count()), 1);
    assert_eq!(
        label(cx, &s, "changes-summary").as_deref(),
        Some("1 staged of 3")
    );
    set_message(cx, &s, "records the gone file");
    assert!(view(cx).read_with(cx, |v, cx| v.can_commit(cx)));
    view(cx).update(cx, |v, cx| v.commit(cx));
    idle(cx, &s);
    wait_for(cx, "committed", |cx| {
        !stages(cx).contains(&StagedGone) && !view(cx).read_with(cx, |v, _| v.busy())
    });
    assert_eq!(
        sb.git(col.path(), &["show", "--name-only", "--format=", "HEAD"])
            .trim(),
        "gone.yml"
    );
}

#[gpui_kit::test]
fn a_click_on_a_staged_file_gone_from_disk_drops_the_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_file_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 1usize));
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "row dropped", |cx| stages(cx) == [Unstaged, Unstaged]);
    assert_eq!(sb.git(col.path(), &["diff", "--cached", "--name-only"]), "");
}

#[gpui_kit::test]
fn a_staged_file_gone_from_disk_has_a_ticked_box(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_file_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    frame(cx, s.win);
    let ticked = |cx: &mut TestAppContext, ix: usize| {
        let box_ = ("changes-stage", ix);
        cx.update_window(s.win, |_, w, _| w.find(box_).checked())
            .unwrap()
    };
    assert_eq!(ticked(cx, 1), Some(true));
    assert_eq!(ticked(cx, 0), Some(false));
}

// `a.yml` edited and staged, then removed from disk.
fn with_a_staged_edit_gone(sb: &Sandbox) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/edited"));
    sb.git(col.path(), &["add", "a.yml"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    col
}

#[gpui_kit::test]
fn a_staged_edit_gone_from_disk_reads_as_gone_not_partly_staged(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_edit_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [StagedGone]);
    assert_eq!(
        label(cx, &s, ("changes-file", 0usize)).as_deref(),
        Some("modified a.yml, staged, gone from its path")
    );
    assert!(!present(cx, s.win, ("changes-partly", 0usize)));
    assert_eq!(
        label(cx, &s, "changes-summary").as_deref(),
        Some("1 staged of 1")
    );
}

#[gpui_kit::test]
fn unticking_a_staged_edit_gone_from_disk_unstages_it(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_edit_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 0usize));
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "unstaged", |cx| stages(cx) == [Unstaged]);
    assert_eq!(sb.git(col.path(), &["diff", "--cached", "--name-only"]), "");
}

#[gpui_kit::test]
fn a_list_of_only_staged_files_gone_from_disk_offers_unstage_all(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_edit_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "unstaged", |cx| stages(cx) == [Unstaged]);
}

#[gpui_kit::test]
fn the_box_of_a_staged_file_gone_from_disk_warns_on_hover(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = with_a_staged_file_gone(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("changes-stage", 1usize), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(
        label(cx, &s, "changes-gone-tooltip").as_deref(),
        Some("Unstaging drops the staged copy, and nothing is left at this path")
    );
    assert_eq!(
        label(cx, &s, ("changes-stage", 1usize)).as_deref(),
        Some("Stage gone.yml. Unstaging drops the staged copy, and nothing is left at this path")
    );
    assert_eq!(
        label(cx, &s, ("changes-stage", 0usize)).as_deref(),
        Some("Stage a.yml")
    );
}

// A row's name too long for the room after its status word and directory.
fn long_name_beside_a_short_one(cx: &mut TestAppContext) -> Started {
    let sb = Sandbox::new();
    let long = format!("{LONG_DIR}/{LONG_NAME}");
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            (&long, &request("l", "http://x/l")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/1"));
    write(col.path(), &long, &request("l", "http://x/2"));
    let s = git_started(cx, &sb, &col);
    // The collection must outlive the dialog it feeds.
    std::mem::forget(col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(700.)));
    open_on(cx, &s, std::path::Path::new(&s.root));
    s
}

#[gpui_kit::test]
fn a_long_name_in_a_subfolder_takes_a_readable_line_of_its_own(cx: &mut TestAppContext) {
    let s = long_name_beside_a_short_one(cx);
    let name = bounds(cx, &s, ("changes-file-name", 1usize));
    let word = bounds(cx, &s, ("changes-file-word", 1usize));
    let row = bounds(cx, &s, ("changes-file", 1usize));
    assert!(name.size.width > px(150.), "{name:?}");
    assert!(name.top() >= word.bottom() - px(1.), "{word:?} {name:?}");
    assert!(name.bottom() <= row.bottom() + px(1.), "{name:?} {row:?}");
    assert!(name.right() <= row.right(), "{name:?} {row:?}");
}

#[gpui_kit::test]
fn a_short_row_beside_a_wrapped_one_stays_one_line(cx: &mut TestAppContext) {
    let s = long_name_beside_a_short_one(cx);
    let height = cx
        .update_window(s.win, |_, w, _| {
            apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w)
        })
        .unwrap();
    let row = bounds(cx, &s, ("changes-file", 0usize));
    assert_eq!(row.size.height, height);
    let (word, name) = (
        bounds(cx, &s, ("changes-file-word", 0usize)),
        bounds(cx, &s, ("changes-file-name", 0usize)),
    );
    assert!(name.left() >= word.right(), "{word:?} {name:?}");
    assert!(bounds(cx, &s, ("changes-file", 1usize)).size.height > height);
}

#[gpui_kit::test]
fn a_wrapped_name_still_shows_its_full_path_on_hover(cx: &mut TestAppContext) {
    let s = long_name_beside_a_short_one(cx);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("changes-file-name", 1usize), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    assert_eq!(
        label(cx, &s, "changes-file-tooltip").as_deref(),
        Some(format!("{LONG_DIR}/{LONG_NAME}").as_str())
    );
}
