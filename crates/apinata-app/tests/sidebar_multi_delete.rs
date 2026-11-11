use crate::support;

use apinata_app::sidebar::DeleteSelected;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn press(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
}
fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}
fn select_two(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, first: &str, second: &str) {
    cx.update_window(win, |_, w, cx| w.click(row(first), cx))
        .unwrap();
    secondary_click(cx, win, row(second));
}
fn multi(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    let sb = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    sb.read_with(cx, |s, _| {
        s.multi_keys()
            .into_iter()
            .map(|(_, p)| p.as_str().to_owned())
            .collect()
    })
}

#[gpui_kit::test]
fn a_batched_command_id_is_taken_once_and_only_for_its_root(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, _| w.track_batch(&root, &[4, 5]));
    assert!(!ws.update(cx, |w, _| {
        w.take_batched(std::path::Path::new("/elsewhere"), 4)
    }));
    assert!(ws.update(cx, |w, _| w.take_batched(&root, 4)));
    assert!(
        !ws.update(cx, |w, _| w.take_batched(&root, 4)),
        "taken once"
    );
    assert!(
        !ws.update(cx, |w, _| w.take_batched(&root, 9)),
        "never tracked"
    );
}

#[gpui_kit::test]
fn delete_asks_once_and_removes_every_selected_row(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    select_two(cx, h.win, "a.yml", "c.yml");
    focus_tree(cx, &h);
    press(cx, h.win, "delete");
    assert!(confirm_open(cx), "one confirm for the whole set");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists() && !h.root.join("c.yml").exists());
    assert!(h.root.join("b.yml").exists() && h.root.join("d.yml").exists());
}

#[gpui_kit::test]
fn escape_on_the_prompt_deletes_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    select_two(cx, h.win, "a.yml", "c.yml");
    focus_tree(cx, &h);
    press(cx, h.win, "backspace");
    let win = confirm_window(cx).expect("the prompt");
    cx.simulate_keystrokes(win, "escape");
    cx.run_until_parked();
    assert!(h.root.join("a.yml").exists() && h.root.join("c.yml").exists());
}

#[gpui_kit::test]
fn a_folder_and_its_child_count_once(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap(); // expands f
    secondary_click(cx, h.win, row("f/x.yml")); // seeded with f: the set is f and f/x.yml
    focus_tree(cx, &h);
    press(cx, h.win, "delete");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(
        !h.root.join("f").exists(),
        "the folder went with its child, with no NotFound"
    );
}

// A read-only directory makes the unlink fail for one member while the other succeeds.
#[cfg(unix)]
#[gpui_kit::test]
fn a_partial_failure_leaves_one_toast_and_keeps_the_failed_row_selected(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("ro/folder.yml", &folder("Ro")),
        ("ro/r1.yml", &request("R1", "http://x/r1")),
    ]);
    let ro = col.path().join("ro");
    let _g = crate::support::ModeGuard::set(&ro, 0o555);
    if std::fs::File::create(ro.join("probe")).is_ok() {
        eprintln!("skipped: permissions are not enforced for this user");
        return;
    }
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("ro"), cx))
        .unwrap(); // expands ro
    // a.yml is picked second, by modifier click, so it has no open tab: closing a tab on
    // delete is its own toast and this test counts only the summary.
    select_two(cx, s.win, "ro/r1.yml", "a.yml");
    let before = toasts(cx, &s);
    // Call the action directly: the shell harness has no tree-focus helper.
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    cx.update_window(s.win, |_, w, cx| {
        sidebar.update(cx, |sb, cx| sb.delete_selected(&DeleteSelected, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-delete");
    frame(cx, s.win);
    assert!(!col.path().join("a.yml").exists());
    assert!(col.path().join("ro/r1.yml").exists());
    assert_eq!(toasts(cx, &s), before + 1, "one summary toast");
    assert_eq!(
        multi(cx, &s),
        ["ro/r1.yml"],
        "the failed row stays selected"
    );
}

fn prompt(cx: &mut TestAppContext) -> (String, String) {
    let win = confirm_window(cx).expect("a confirm dialog is open");
    let text = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id)
                .and_then(|e| e.label().map(str::to_owned))
                .unwrap_or_default()
        })
        .unwrap()
    };
    (text(cx, "confirm-dialog"), text(cx, "confirm-detail"))
}

// Even with a trash to undo from, a range that swept up a folder row asks before the request
// nobody selected inside it goes too.
#[gpui_kit::test]
fn a_range_over_a_folder_row_asks_first_even_with_a_trash(cx: &mut TestAppContext) {
    let bin = std::sync::Arc::new(support::fake_trash::FakeTrash::new());
    let h = open_sidebar_trash(cx, range_files(), Some(bin));
    for f in ["e", "f"] {
        cx.update_window(h.win, |_, w, cx| w.click(row(f), cx))
            .unwrap(); // expands the folder
    }
    cx.update_window(h.win, |_, w, cx| w.click(row("e/y.yml"), cx))
        .unwrap();
    shift_click(cx, h.win, row("f/x.yml"));
    focus_tree(cx, &h);
    press(cx, h.win, "delete");
    assert!(
        confirm_open(cx),
        "the folder's unselected request needs a yes"
    );
    let (title, detail) = prompt(cx);
    assert_eq!(title, "Delete 1 folder and 1 request?");
    assert_eq!(
        detail,
        "1 request in this folder was not selected and will be deleted too."
    );
    answer_confirm(cx, "confirm-cancel");
    frame(cx, h.win);
    assert!(h.root.join("e/y.yml").exists() && h.root.join("f/z.yml").exists());
    focus_tree(cx, &h);
    press(cx, h.win, "delete");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("e/y.yml").exists() && !h.root.join("f").exists());
    assert!(h.root.join("a.yml").exists() && h.root.join("e").exists());
}

// A folder whose every row was selected holds no surprise, so the trash still stands in for
// the prompt.
#[gpui_kit::test]
fn a_folder_picked_with_all_its_rows_goes_to_the_trash_without_asking(cx: &mut TestAppContext) {
    let bin = std::sync::Arc::new(support::fake_trash::FakeTrash::new());
    let h = open_sidebar_trash(cx, multi_files(), Some(bin));
    cx.update_window(h.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap(); // expands f
    secondary_click(cx, h.win, row("f/x.yml")); // seeded with f: the set is f and f/x.yml
    secondary_click(cx, h.win, row("a.yml"));
    focus_tree(cx, &h);
    press(cx, h.win, "delete");
    assert!(!confirm_open(cx));
    assert!(!h.root.join("f").exists() && !h.root.join("a.yml").exists());
}
