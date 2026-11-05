//! An index-only copy on the Changes dialog: Stage all, Unstage all, Space, Enter and a box click
//! ask before dropping one, also past the list's cap, Cancel changes nothing, and a row words what
//! is left by its case.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::diff_doc::{DiffBody, RowKind};
use apinata_app::git_status::GitStage::{Both, Staged, StagedGone, Unstaged};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use h::*;
use support::*;

const ONE: &str = "Drop 1 staged copy?";
const PLAIN: &str = "Unstaging drops the staged copy, and nothing is left at this path";

// `a.yml` modified and `new.yml` untracked, beside `b.yml` with a staged edit gone from disk: `MD`.
fn beside_a_normal_row(sb: &Sandbox) -> tempfile::TempDir {
    let col = changed(sb);
    write(col.path(), "b.yml", &request("b", "http://x/staged"));
    sb.git(col.path(), &["add", "b.yml"]);
    std::fs::remove_file(col.path().join("b.yml")).unwrap();
    col
}

// As above with `a.yml` staged and `new.yml` gone, so every box is ticked.
fn all_ticked(sb: &Sandbox) -> tempfile::TempDir {
    let col = beside_a_normal_row(sb);
    std::fs::remove_file(col.path().join("new.yml")).unwrap();
    sb.git(col.path(), &["add", "a.yml"]);
    col
}

// What is in the index, as `git status --short` prints it.
fn short(sb: &Sandbox, col: &tempfile::TempDir) -> String {
    sb.git(col.path(), &["status", "--short"])
}

// The confirm's title, as drawn.
fn question(cx: &mut TestAppContext) -> Option<String> {
    let win = confirm_window(cx).expect("a confirm dialog is open");
    cx.update_window(win, |_, w, _| {
        w.try_find("confirm-dialog")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// The list focused with the row at `ix` selected, as the keyboard reaches it.
fn on_row(cx: &mut TestAppContext, s: &Started, ix: usize) {
    click(cx, s, ("changes-file", ix));
    frame(cx, s.win);
}

fn assert_asked_then_cancelled(
    cx: &mut TestAppContext,
    s: &Started,
    sb: &Sandbox,
    col: &tempfile::TempDir,
    want: Vec<apinata_app::git_status::GitStage>,
) {
    let before = short(sb, col);
    assert!(confirm_open(cx), "it asks first");
    assert_eq!(stages(cx), want, "nothing ran yet");
    assert_eq!(question(cx).as_deref(), Some(ONE));
    answer_confirm(cx, "confirm-cancel");
    idle(cx, s);
    assert!(!confirm_open(cx));
    assert_eq!(stages(cx), want, "Cancel leaves every row");
    assert_eq!(short(sb, col), before, "Cancel leaves the index");
}

#[gpui_kit::test]
fn stage_all_asks_first_and_cancel_leaves_every_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    assert_asked_then_cancelled(cx, &s, &sb, &col, vec![Unstaged, StagedGone, Unstaged]);
}

#[gpui_kit::test]
fn stage_all_goes_through_on_a_yes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    // Git pairs the staged deletion of `b.yml` with `new.yml` as a rename, so one row merges.
    wait_for(cx, "all staged", |cx| stages(cx) == [Staged, Staged]);
}

#[gpui_kit::test]
fn unstage_all_asks_first_and_cancel_leaves_every_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = all_ticked(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Staged, StagedGone]);
    click(cx, &s, "changes-toggle-all");
    assert_asked_then_cancelled(cx, &s, &sb, &col, vec![Staged, StagedGone]);
}

#[gpui_kit::test]
fn unstage_all_goes_through_on_a_yes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = all_ticked(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "all unstaged", |cx| stages(cx) == [Unstaged, Unstaged]);
}

#[gpui_kit::test]
fn the_bulk_question_names_the_count(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/staged"));
    sb.git(col.path(), &["add", "a.yml"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    assert_eq!(question(cx).as_deref(), Some("Drop 2 staged copies?"));
}

#[gpui_kit::test]
fn bulk_controls_with_no_gone_row_do_not_ask(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    assert!(!confirm_open(cx));
    wait_for(cx, "all staged", |cx| stages(cx) == [Staged, Staged]);
}

#[gpui_kit::test]
fn space_on_a_gone_row_asks_and_cancel_keeps_focus_for_another_try(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 1);
    platform_press(cx, s.win, "space");
    assert_asked_then_cancelled(cx, &s, &sb, &col, vec![Unstaged, StagedGone, Unstaged]);
    // Focus came back to the list, so the next press reaches the same row.
    frame(cx, s.win);
    platform_press(cx, s.win, "space");
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "the row unstaged", |cx| {
        !stages(cx).contains(&StagedGone) && !view(cx).read_with(cx, |v, _| v.busy())
    });
    assert_eq!(sb.git(col.path(), &["diff", "--cached", "--name-only"]), "");
}

#[gpui_kit::test]
fn enter_on_a_gone_row_asks_and_leaves_the_dialog_open(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 1);
    platform_press(cx, s.win, "enter");
    assert_asked_then_cancelled(cx, &s, &sb, &col, vec![Unstaged, StagedGone, Unstaged]);
    assert!(
        present(cx, s.win, "changes-dialog"),
        "the dialog stays open"
    );
    platform_press(cx, s.win, "enter");
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "the row unstaged", |cx| {
        !stages(cx).contains(&StagedGone)
    });
}

#[gpui_kit::test]
fn space_and_enter_on_a_normal_row_beside_it_do_not_ask(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 0);
    platform_press(cx, s.win, "space");
    idle(cx, &s);
    assert!(!confirm_open(cx));
    wait_for(cx, "the normal row staged", |cx| {
        stages(cx) == [Staged, StagedGone, Unstaged]
    });
    platform_press(cx, s.win, "enter");
    idle(cx, &s);
    assert!(!confirm_open(cx));
    wait_for(cx, "the normal row unstaged", |cx| {
        stages(cx) == [Unstaged, StagedGone, Unstaged]
    });
}

// The confirm's body, as drawn.
fn body(cx: &mut TestAppContext) -> Option<String> {
    let win = confirm_window(cx).expect("a confirm dialog is open");
    cx.update_window(win, |_, w, _| {
        w.try_find("confirm-detail")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_click_on_a_gone_rows_box_asks_with_its_hover_warning(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(
        label(cx, &s, ("changes-gone", 1usize)).as_deref(),
        Some("staged, gone from its path")
    );
    assert_eq!(hovered_warning(cx, &s, 1).as_deref(), Some(PLAIN));
    click(cx, &s, ("changes-stage", 1usize));
    assert_eq!(body(cx).as_deref(), Some(PLAIN));
    assert_asked_then_cancelled(cx, &s, &sb, &col, vec![Unstaged, StagedGone, Unstaged]);
    click(cx, &s, ("changes-stage", 1usize));
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "the row unstaged", |cx| {
        !stages(cx).contains(&StagedGone)
    });
    assert_eq!(sb.git(col.path(), &["diff", "--cached", "--name-only"]), "");
}

#[gpui_kit::test]
fn space_on_a_gone_row_asks_with_its_hover_warning(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = beside_a_normal_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 1);
    platform_press(cx, s.win, "space");
    assert_eq!(body(cx).as_deref(), Some(PLAIN));
}

// Untracked files past core's cap, as plain text so no request loads; `zz.yml` holds a staged
// edit gone from disk and sorts past them all. `listed` adds such a row at `a.yml`, inside the
// list, and `staged` stages the untracked files so every listed box is ticked.
fn past_the_cap(sb: &Sandbox, listed: bool, staged: bool) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("zz.yml", &request("z", "http://x/z")),
        ],
    );
    let mut gone = vec!["zz.yml"];
    if listed {
        gone.push("a.yml");
    }
    for name in gone {
        write(col.path(), name, &request(name, "http://x/staged"));
        sb.git(col.path(), &["add", name]);
        std::fs::remove_file(col.path().join(name)).unwrap();
    }
    for i in 0..apinata_core::GIT_FILE_CAP {
        write(col.path(), &format!("bulk/f{i:05}.txt"), "x\n");
    }
    if staged {
        sb.git(col.path(), &["add", "bulk"]);
    }
    col
}

// Opens the dialog over a list cut at the cap and presses the bulk button.
fn bulk_past_the_cap(cx: &mut TestAppContext, s: &Started, col: &tempfile::TempDir) {
    open_on(cx, s, col.path());
    assert!(
        view(cx).read_with(cx, |v, _| v.truncated()),
        "the list stops at the cap"
    );
    assert!(!stages(cx).is_empty());
    click(cx, s, "changes-toggle-all");
}

#[gpui_kit::test]
fn stage_all_past_the_cap_asks_for_the_listed_count_or_more(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = past_the_cap(&sb, true, false);
    let s = git_started(cx, &sb, &col);
    bulk_past_the_cap(cx, &s, &col);
    assert_eq!(
        question(cx).as_deref(),
        Some("Drop 1 or more staged copies?")
    );
    let before = short(&sb, &col);
    answer_confirm(cx, "confirm-cancel");
    idle(cx, &s);
    assert_eq!(short(&sb, &col), before, "Cancel leaves the index");
}

#[gpui_kit::test]
fn unstage_all_past_the_cap_asks_for_the_listed_count_or_more(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = past_the_cap(&sb, true, true);
    let s = git_started(cx, &sb, &col);
    bulk_past_the_cap(cx, &s, &col);
    assert!(stages(cx).iter().all(|s| s.is_full()), "Unstage all");
    assert_eq!(
        question(cx).as_deref(),
        Some("Drop 1 or more staged copies?")
    );
}

#[gpui_kit::test]
fn unstage_all_past_the_cap_with_no_listed_gone_row_still_asks(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = past_the_cap(&sb, false, true);
    let s = git_started(cx, &sb, &col);
    bulk_past_the_cap(cx, &s, &col);
    assert!(!stages(cx).contains(&StagedGone), "zz.yml is past the cap");
    assert_eq!(
        question(cx).as_deref(),
        Some("Drop any staged copies past the list?")
    );
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "the index emptied", |cx| {
        !view(cx).read_with(cx, |v, _| v.busy())
            && sb
                .git(col.path(), &["diff", "--cached", "--name-only"])
                .is_empty()
    });
}

#[gpui_kit::test]
fn stage_all_past_the_cap_with_no_listed_gone_row_still_asks(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = past_the_cap(&sb, false, false);
    let s = git_started(cx, &sb, &col);
    bulk_past_the_cap(cx, &s, &col);
    assert_eq!(
        question(cx).as_deref(),
        Some("Drop any staged copies past the list?")
    );
}

// The tooltip the box at `ix` shows after a hover.
fn hovered_warning(cx: &mut TestAppContext, s: &Started, ix: usize) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("changes-stage", ix), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    label(cx, s, "changes-gone-tooltip")
}

// Asserts the row at `ix` says where its file went, on the note, the row's name and the box.
fn assert_kept_wording(cx: &mut TestAppContext, s: &Started, ix: usize, path: &str, kept: &str) {
    let note = format!("staged, file now at {kept}");
    let warning = format!("Unstaging drops the staged copy; the file at {kept} stays");
    assert_eq!(stages(cx)[ix], StagedGone);
    assert_eq!(
        label(cx, s, ("changes-gone", ix)).as_deref(),
        Some(note.as_str())
    );
    assert_eq!(
        label(cx, s, ("changes-file", ix)).as_deref(),
        Some(format!("modified {path}, {note}").as_str())
    );
    assert_eq!(
        label(cx, s, ("changes-stage", ix)).as_deref(),
        Some(format!("Stage {path}. {warning}").as_str())
    );
    assert_eq!(hovered_warning(cx, s, ix), Some(warning));
}

#[gpui_kit::test]
fn a_staged_edit_moved_and_marked_add_n_names_the_new_path(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    write(col.path(), "b.yml", &request("b", "http://x/staged"));
    sb.git(col.path(), &["add", "b.yml"]);
    std::fs::rename(col.path().join("b.yml"), col.path().join("c.yml")).unwrap();
    sb.git(col.path(), &["add", "-N", "c.yml"]);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_kept_wording(cx, &s, 1, "b.yml", "c.yml");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_staged_edit_under_a_link_names_the_links_target(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("p/a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "p/a.yml", &request("a", "http://x/staged"));
    sb.git(col.path(), &["add", "p/a.yml"]);
    std::fs::rename(col.path().join("p"), col.path().join("q")).unwrap();
    std::os::unix::fs::symlink("q", col.path().join("p")).unwrap();
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx)[1], StagedGone, "p, then p/a.yml, then q/a.yml");
    assert_kept_wording(cx, &s, 1, "p/a.yml", "q/a.yml");
}

// Whether the loaded diff shows only removed lines, which is what a deleted row means.
fn removes_only(cx: &mut TestAppContext) -> bool {
    match view(cx).read_with(cx, |v, _| v.doc().cloned()) {
        Some(doc) => match doc.body {
            DiffBody::Lines(rows) => rows
                .iter()
                .all(|r| matches!(r.kind, RowKind::Header | RowKind::Removed)),
            _ => false,
        },
        None => false,
    }
}

#[cfg(unix)]
#[gpui_kit::test]
fn the_diff_follows_a_row_unstaged_under_a_link(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("p/a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "p/a.yml", &request("a", "http://x/staged"));
    sb.git(col.path(), &["add", "p/a.yml"]);
    std::fs::rename(col.path().join("p"), col.path().join("q")).unwrap();
    std::os::unix::fs::symlink("q", col.path().join("p")).unwrap();
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 1usize));
    idle(cx, &s);
    wait_for(cx, "the staged diff", |cx| {
        view(cx).read_with(cx, |v, _| v.doc().is_some()) && !removes_only(cx)
    });
    click(cx, &s, ("changes-stage", 1usize));
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "the row unstaged", |cx| stages(cx)[1] == Unstaged);
    wait_for(cx, "the pane matches the row", removes_only);
    // Leaving the row and coming back reads the same.
    click(cx, &s, ("changes-file", 2usize));
    idle(cx, &s);
    click(cx, &s, ("changes-file", 1usize));
    idle(cx, &s);
    wait_for(cx, "the pane still matches", removes_only);
}

// `p/a.yml` committed, an edit of it staged, and `p` replaced by `replace`.
fn staged_under_p(
    sb: &Sandbox,
    names: &[&str],
    replace: impl FnOnce(&std::path::Path),
) -> tempfile::TempDir {
    let mut tree = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    tree.extend(
        names
            .iter()
            .map(|n| (format!("p/{n}"), request(n, "http://x/a"))),
    );
    let borrowed: Vec<(&str, &str)> = tree.iter().map(|(p, t)| (&**p, &**t)).collect();
    let col = committed(sb, &borrowed);
    for name in names {
        let at = format!("p/{name}");
        write(col.path(), &at, &request(name, "http://x/staged"));
        sb.git(col.path(), &["add", &at]);
    }
    replace(col.path());
    col
}

#[cfg(unix)]
fn p_becomes_a_link(col: &std::path::Path) {
    std::fs::rename(col.join("p"), col.join("q")).unwrap();
    std::os::unix::fs::symlink("q", col.join("p")).unwrap();
}

fn p_becomes_a_file(col: &std::path::Path) {
    std::fs::remove_dir_all(col.join("p")).unwrap();
    std::fs::write(col.join("p"), "x").unwrap();
}

#[cfg(unix)]
#[gpui_kit::test]
fn space_on_a_link_row_over_a_gone_row_asks_and_cancel_keeps_the_copy(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = staged_under_p(&sb, &["a.yml"], p_becomes_a_link);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 0);
    let want = stages(cx);
    assert_eq!(want[1], StagedGone);
    platform_press(cx, s.win, "space");
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
    platform_press(cx, s.win, "space");
    answer_confirm(cx, "confirm-drop");
    idle(cx, &s);
    wait_for(cx, "the copy dropped", |cx| {
        !stages(cx).contains(&StagedGone)
    });
}

#[gpui_kit::test]
fn enter_on_a_file_over_a_gone_row_asks(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = staged_under_p(&sb, &["a.yml"], p_becomes_a_file);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 0);
    let want = stages(cx);
    platform_press(cx, s.win, "enter");
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

#[gpui_kit::test]
fn the_box_of_a_file_over_a_gone_row_asks_with_the_count_it_drops(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = staged_under_p(&sb, &["a.yml", "b.yml"], p_becomes_a_file);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let want = stages(cx);
    assert_eq!(want, [Unstaged, StagedGone, StagedGone]);
    click(cx, &s, ("changes-stage", 0usize));
    assert_eq!(question(cx).as_deref(), Some("Drop 2 staged copies?"));
    answer_confirm(cx, "confirm-cancel");
    idle(cx, &s);
    assert_eq!(stages(cx), want);
}

#[gpui_kit::test]
fn the_box_of_a_file_under_a_gone_path_asks(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/staged"));
    sb.git(col.path(), &["add", "a.yml"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    write(col.path(), "a.yml/k.yml", &request("k", "http://x/k"));
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let want = stages(cx);
    assert_eq!(want, [StagedGone, Unstaged]);
    click(cx, &s, ("changes-stage", 1usize));
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

#[gpui_kit::test]
fn a_row_that_only_shares_a_prefix_with_a_gone_row_does_not_ask(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/staged"));
    sb.git(col.path(), &["add", "a.yml"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    write(col.path(), "a.yml2", "not under a.yml\n");
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [StagedGone, Unstaged]);
    click(cx, &s, ("changes-stage", 1usize));
    idle(cx, &s);
    assert!(!confirm_open(cx), "a.yml2 is not under a.yml");
    wait_for(cx, "a.yml2 staged", |cx| stages(cx)[1] == Staged);
}

// `n.yml` a staged rename of `o.yml`, with a staged `o.yml/x.yml` since gone from disk.
fn gone_under_a_renames_origin(sb: &Sandbox) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("o.yml", &request("o", "http://x/o")),
        ],
    );
    sb.git(col.path(), &["mv", "o.yml", "n.yml"]);
    write(col.path(), "o.yml/x.yml", &request("x", "http://x/x"));
    sb.git(col.path(), &["add", "o.yml/x.yml"]);
    std::fs::remove_dir_all(col.path().join("o.yml")).unwrap();
    col
}

#[gpui_kit::test]
fn the_box_of_a_rename_asks_for_a_gone_row_under_its_origin(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = gone_under_a_renames_origin(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let want = stages(cx);
    assert_eq!(want, [Staged, StagedGone]);
    click(cx, &s, ("changes-stage", 0usize));
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

#[gpui_kit::test]
fn space_on_a_rename_asks_for_a_gone_row_under_its_origin(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = gone_under_a_renames_origin(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    on_row(cx, &s, 0);
    let want = stages(cx);
    platform_press(cx, s.win, "space");
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

#[gpui_kit::test]
fn a_rename_asks_for_its_origin_only_when_it_unstages(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = gone_under_a_renames_origin(&sb);
    let edited = std::fs::read_to_string(col.path().join("n.yml")).unwrap() + "# edited\n";
    write(col.path(), "n.yml", &edited);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Both, StagedGone]);
    // Staging leaves the origin alone, so the gone row keeps its copy with no question.
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    assert!(!confirm_open(cx), "staging n.yml drops nothing");
    let want = vec![Staged, StagedGone];
    wait_for(cx, "n.yml staged", |cx| stages(cx) == want);
    // Unstaging resets the origin too, which does drop it.
    click(cx, &s, ("changes-stage", 0usize));
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

#[gpui_kit::test]
fn the_box_of_a_rename_asks_for_a_gone_row_over_its_origin(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("d/o.yml", &request("o", "http://x/o")),
        ],
    );
    sb.git(col.path(), &["mv", "d/o.yml", "n.yml"]);
    std::fs::remove_dir(col.path().join("d")).unwrap();
    write(col.path(), "d", "a file where the directory was\n");
    sb.git(col.path(), &["add", "d"]);
    std::fs::remove_file(col.path().join("d")).unwrap();
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let want = stages(cx);
    assert_eq!(want, [StagedGone, Staged]);
    click(cx, &s, ("changes-stage", 1usize));
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

// The deletion of a file `d`, with `d/n` a staged rename of `o.yml` and a staged `o.yml/x.yml`
// since gone from disk.
fn gone_under_the_origin_of_a_rename_under_a_row(sb: &Sandbox) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("d", "a file where the directory comes\n"),
            ("o.yml", &request("o", "http://x/o")),
        ],
    );
    sb.git(col.path(), &["rm", "-q", "d"]);
    std::fs::create_dir(col.path().join("d")).unwrap();
    sb.git(col.path(), &["mv", "o.yml", "d/n"]);
    write(col.path(), "o.yml/x.yml", &request("x", "http://x/x"));
    sb.git(col.path(), &["add", "o.yml/x.yml"]);
    std::fs::remove_dir_all(col.path().join("o.yml")).unwrap();
    col
}

#[gpui_kit::test]
fn a_row_over_a_rename_target_asks_for_a_gone_row_under_the_origin(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = gone_under_the_origin_of_a_rename_under_a_row(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let want = stages(cx);
    assert_eq!(want, [Staged, Staged, StagedGone]);
    click(cx, &s, ("changes-stage", 0usize));
    assert_asked_then_cancelled(cx, &s, &sb, &col, want);
}

#[gpui_kit::test]
fn a_row_staging_over_a_rename_target_leaves_the_origin_alone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = gone_under_the_origin_of_a_rename_under_a_row(&sb);
    let edited = std::fs::read_to_string(col.path().join("d/n")).unwrap() + "# edited\n";
    write(col.path(), "d/n", &edited);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Staged, Both, StagedGone]);
    click(cx, &s, ("changes-stage", 1usize));
    idle(cx, &s);
    assert!(!confirm_open(cx), "staging d/n drops nothing");
    wait_for(cx, "d/n staged", |cx| {
        stages(cx) == [Staged, Staged, StagedGone]
    });
}

#[gpui_kit::test]
fn a_moved_file_with_renames_off_speaks_only_of_the_path(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    sb.git(col.path(), &["config", "status.renames", "false"]);
    write(col.path(), "b.yml", &request("b", "http://x/staged"));
    sb.git(col.path(), &["add", "b.yml"]);
    std::fs::rename(col.path().join("b.yml"), col.path().join("c.yml")).unwrap();
    sb.git(col.path(), &["add", "-N", "c.yml"]);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(
        label(cx, &s, ("changes-gone", 1usize)).as_deref(),
        Some("staged, gone from its path")
    );
    assert_eq!(hovered_warning(cx, &s, 1).as_deref(), Some(PLAIN));
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_link_to_other_text_says_the_path_opens_another_file(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = staged_under_p(&sb, &["a.yml"], p_becomes_a_link);
    write(col.path(), "q/a.yml", &request("a", "http://x/later"));
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let warning = "Unstaging drops the staged copy; the path now opens another file (q/a.yml)";
    assert_eq!(
        label(cx, &s, ("changes-gone", 1usize)).as_deref(),
        Some("staged, path now opens q/a.yml")
    );
    assert_eq!(hovered_warning(cx, &s, 1).as_deref(), Some(warning));
    click(cx, &s, ("changes-stage", 1usize));
    assert_eq!(body(cx).as_deref(), Some(warning));
}
