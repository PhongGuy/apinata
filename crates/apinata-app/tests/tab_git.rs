//! Git letters on request tabs, beside the dirty dot, and the tab menu's Show changes item.

use crate::git_support;
use crate::support;

use apinata_app::changes_dialog::current;
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext};
use support::*;

fn committed(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

/// The real app with `a.yml` changed on disk, `b.yml` unchanged, both open as tabs 0 and 1.
fn two_tabs(cx: &mut TestAppContext, sb: &Sandbox) -> (tempfile::TempDir, Started) {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("b.yml", &request("b", "http://x/b")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    (col, s)
}

fn label(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_changed_tab_carries_the_letter_beside_its_dirty_dot(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = two_tabs(cx, &sb);
    assert_eq!(
        label(cx, s.win, ("tab-git", 0usize)).as_deref(),
        Some("Modified since last commit")
    );
    assert!(!present(cx, s.win, ("tab-git", 1usize)));
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/draft"), cx)
    });
    frame(cx, s.win);
    assert!(present(cx, s.win, ("dirty", 0usize)));
    assert!(present(cx, s.win, ("tab-git", 0usize)));
}

#[gpui_kit::test]
fn the_menu_of_a_changed_tab_opens_its_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = two_tabs(cx, &sb);
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    click_menu(cx, s.win, "Show changes");
    frame(cx, s.win);
    let selected = cx.update(|cx| {
        current(cx).and_then(|v| {
            v.read(cx)
                .selected()
                .map(|p| p.to_string_lossy().into_owned())
        })
    });
    assert_eq!(selected.as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn the_menu_of_an_unchanged_tab_has_no_show_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = two_tabs(cx, &sb);
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", 1usize), cx))
        .unwrap();
    frame(cx, s.win);
    let (last, extra) = cx
        .update_window(s.win, |_, w, _| {
            (
                try_menu_index(w, "Show changes").is_none(),
                menu_labels(w).into_iter().flatten().last(),
            )
        })
        .unwrap();
    assert!(last, "Show changes is absent");
    assert_eq!(extra.as_deref(), Some("Pin tab"), "the menu is open");
}

// The letter comes before the dirty dot whether the tab is active or not, since an inactive
// tab's dot sits in its close slot at the end.
#[gpui_kit::test]
fn the_letter_precedes_the_dirty_dot_on_every_tab(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = two_tabs(cx, &sb);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/draft"), cx)
    });
    for active in [1usize, 0] {
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(active)));
        frame(cx, s.win);
        let (letter, dot) = cx
            .update_window(s.win, |_, w, _| {
                (
                    w.find(("tab-git", 0usize)).bounds(),
                    w.find(("dirty", 0usize)).bounds(),
                )
            })
            .unwrap();
        assert!(
            letter.right() <= dot.left(),
            "active tab {active}: letter {letter:?}, dot {dot:?}"
        );
    }
}

// On the underline strip the letter and the dot share the label's centre line, active or not.
#[gpui_kit::test]
fn the_letter_and_the_dirty_dot_share_a_centre_line(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = two_tabs(cx, &sb);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/draft"), cx)
    });
    for active in [1usize, 0] {
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(active)));
        frame(cx, s.win);
        let (letter, dot) = cx
            .update_window(s.win, |_, w, _| {
                (
                    w.find(("tab-git", 0usize)).bounds().center().y,
                    w.find(("dirty", 0usize)).bounds().center().y,
                )
            })
            .unwrap();
        assert!(
            (letter - dot).abs() <= gpui_kit::px(1.),
            "active tab {active}: letter centre {letter:?}, dot centre {dot:?}"
        );
    }
}
