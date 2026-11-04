//! The Changes dialog's diff at its edges: a file with no final newline and lines wider than
//! the pane.

use crate::git_support::{Sandbox, write};
use crate::h::*;
use crate::support::*;

use apinata_app::diff_doc::{DiffBody, RowKind};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, InputEvent as _, ScrollDelta, ScrollWheelEvent, TestAppContext, point, px,
};

fn rows(cx: &mut TestAppContext) -> Vec<apinata_app::diff_doc::DiffRow> {
    let doc = view(cx)
        .read_with(cx, |v, _| v.doc().cloned())
        .expect("the diff loaded");
    match doc.body {
        DiffBody::Lines(rows) => rows,
        other => panic!("expected lines, got {other:?}"),
    }
}

#[gpui_kit::test]
fn a_file_without_a_final_newline_shows_git_s_marker(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    let edited = request("a", "http://x/changed");
    write(col.path(), "a.yml", edited.trim_end_matches('\n'));
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 0usize));
    idle(cx, &s);
    let rows = rows(cx);
    let at = rows
        .iter()
        .position(|r| r.kind == RowKind::NoNewline)
        .expect("a marker row");
    assert_eq!(rows[at].text, "\\ No newline at end of file");
    assert_eq!(rows[at - 1].kind, RowKind::Added, "after the line it marks");
    assert_eq!((rows[at].old, rows[at].new), (None, None));
    assert!(present(cx, s.win, ("diff-line", at)));
}

#[gpui_kit::test]
fn a_line_wider_than_the_pane_scrolls_sideways_to_its_end(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    let long = format!("http://x/{}", "w".repeat(600));
    write(col.path(), "a.yml", &request("a", &long));
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 0usize));
    idle(cx, &s);
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "diff-scrollbar"),
        "a cue for the overflow"
    );
    let ix = rows(cx)
        .iter()
        .position(|r| r.kind == RowKind::Added)
        .expect("an added row");
    let at = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, _| {
            (
                w.find("changes-diff").bounds(),
                w.find(("diff-line", ix)).bounds(),
            )
        })
        .unwrap()
    };
    let (pane, row) = at(cx);
    assert!(row.right() > pane.right() + px(100.), "{row:?} in {pane:?}");
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_event(
            ScrollWheelEvent {
                position: pane.center(),
                delta: ScrollDelta::Pixels(point(px(-100000.), px(0.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    frame(cx, s.win);
    let (pane, row) = at(cx);
    assert!(
        row.right() <= pane.right() + px(2.),
        "the end of the line is in view: {row:?} in {pane:?}"
    );
}
