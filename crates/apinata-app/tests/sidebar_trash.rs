//! Deleting from the sidebar moves to the trash with an Undo toast, or asks first and says the
//! delete is permanent when there is no trash.

use crate::support;

use crate::support::fake_trash::FakeTrash;
use apinata_app::command::{Command, TreeOp};
use apinata_app::sidebar::DeleteSelected;
use apinata_app::sidebar::menu::{BulkEntry, MenuEntry, bulk_entries, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::sync::Arc;
use std::time::Duration;
use support::*;

fn press(cx: &mut TestAppContext, h: &SidebarHarness, key: &str) {
    cx.update_window(h.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn toasts(cx: &mut TestAppContext, h: &SidebarHarness) -> usize {
    cx.update_window(h.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn with_trash(cx: &mut TestAppContext) -> (SidebarHarness, Arc<FakeTrash>) {
    let bin = Arc::new(FakeTrash::new());
    let h = open_sidebar_trash(cx, multi_files(), Some(bin.clone()));
    (h, bin)
}

// The sidebar inside a whole shell, whose own notice toasts show beside the sidebar's.
fn shell_with_trash(cx: &mut TestAppContext) -> (SidebarHarness, Arc<FakeTrash>) {
    shell_with_trash_files(cx, multi_files())
}

fn shell_with_trash_files(
    cx: &mut TestAppContext,
    files: Vec<(&'static str, String)>,
) -> (SidebarHarness, Arc<FakeTrash>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&refs);
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    let bin = Arc::new(FakeTrash::new());
    ws.update(cx, |w, _| w.set_trash(bin.clone()));
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::shell::Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    let view = shell.read_with(cx, |s, _| s.sidebar().clone());
    let h = SidebarHarness {
        col,
        state,
        root,
        win,
        ws,
        view,
    };
    (h, bin)
}

fn select_and_delete(cx: &mut TestAppContext, h: &SidebarHarness, rel: &str) {
    cx.update_window(h.win, |_, w, cx| w.click(row(rel), cx))
        .unwrap();
    focus_tree(cx, h);
    press(cx, h, "delete");
    frame(cx, h.win);
}

fn has(cx: &mut TestAppContext, h: &SidebarHarness, id: &'static str) -> bool {
    cx.update_window(h.win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

fn delete_a_and_c(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    focus_tree(cx, h);
    press(cx, h, "delete");
    frame(cx, h.win);
}

// Clicks Undo and lets its toast finish closing, then counts the toasts the undo added.
// Toasts already showing stay and are not counted.
fn undo_toasts(cx: &mut TestAppContext, h: &SidebarHarness) -> usize {
    let before = toasts(cx, h);
    click_undo(cx, h);
    cx.executor().advance_clock(Duration::from_secs(1));
    frame(cx, h.win);
    assert!(!has(cx, h, "undo-delete"), "the Undo toast closed");
    toasts(cx, h) + 1 - before
}

// Lets every toast showing time out and finish closing. A pointer over the toasts pauses
// their timers, so it moves to the tree first.
fn expire_toasts(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| w.hover("sidebar-view", cx))
        .unwrap();
    for _ in 0..20 {
        if toasts(cx, h) == 0 {
            return;
        }
        cx.executor().advance_clock(Duration::from_secs(10));
        frame(cx, h.win);
    }
    assert_eq!(toasts(cx, h), 0, "every toast timed out");
}

// Presses Ctrl+Z (Cmd+Z on macOS) and lets any toast it closes finish closing.
fn ctrl_z(cx: &mut TestAppContext, h: &SidebarHarness) {
    press(cx, h, &secondary("z"));
    cx.executor().advance_clock(Duration::from_secs(1));
    frame(cx, h.win);
}

fn click_undo(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| w.click("undo-delete", cx))
        .unwrap();
    frame(cx, h.win);
}

#[gpui_kit::test]
fn delete_with_a_trash_asks_nothing_and_offers_undo(cx: &mut TestAppContext) {
    let (h, bin) = with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    assert!(!confirm_open(cx), "the trash makes a prompt needless");
    assert!(!h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 1);
    assert_eq!(toasts(cx, &h), 1);
    let has_undo = cx
        .update_window(h.win, |_, w, _| w.try_find("undo-delete").is_some())
        .unwrap();
    assert!(has_undo);
}

#[gpui_kit::test]
fn undo_restores_the_file_and_the_row(cx: &mut TestAppContext) {
    let (h, bin) = with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    click_undo(cx, &h);
    assert!(h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 0);
    let snap = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    assert!(snap.content.requests.contains_key(&ip("a.yml")));
}

#[gpui_kit::test]
fn a_folder_undoes_with_what_is_inside(cx: &mut TestAppContext) {
    let (h, _bin) = with_trash(cx);
    select_and_delete(cx, &h, "f");
    assert!(!h.root.join("f").exists());
    click_undo(cx, &h);
    assert!(h.root.join("f/x.yml").exists() && h.root.join("f/folder.yml").exists());
}

// The toast only shows for a few seconds; the delete stays undoable from the tree after it.
#[gpui_kit::test]
fn ctrl_z_after_the_toast_expired_restores_the_delete(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    expire_toasts(cx, &h);
    assert!(!has(cx, &h, "undo-delete"), "the toast is gone");
    assert_eq!(toasts(cx, &h), 0);
    focus_tree(cx, &h);
    ctrl_z(cx, &h);
    assert!(h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 0);
    let snap = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    assert!(snap.content.requests.contains_key(&ip("a.yml")));
    assert_eq!(toasts(cx, &h), 1, "the Restored toast");
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
}

#[gpui_kit::test]
fn ctrl_z_twice_restores_the_newest_delete_first(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    select_and_delete(cx, &h, "c.yml");
    expire_toasts(cx, &h);
    focus_tree(cx, &h);
    ctrl_z(cx, &h);
    assert!(h.root.join("c.yml").exists() && !h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 1);
    ctrl_z(cx, &h);
    assert!(h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 0);
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
}

#[gpui_kit::test]
fn ctrl_z_after_the_toasts_undo_does_nothing(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    click_undo(cx, &h);
    assert!(h.root.join("a.yml").exists());
    cx.executor().advance_clock(Duration::from_secs(1));
    select_and_delete(cx, &h, "b.yml");
    click_undo(cx, &h);
    expire_toasts(cx, &h);
    focus_tree(cx, &h);
    ctrl_z(cx, &h);
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
    assert_eq!(toasts(cx, &h), 0, "nothing was left to undo");
    assert_eq!(bin.len(), 0);
}

#[gpui_kit::test]
fn ctrl_z_dismisses_the_toast_and_its_undo_then_does_nothing(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    focus_tree(cx, &h);
    press(cx, &h, &secondary("z"));
    assert!(h.root.join("a.yml").exists());
    // The closing toast's Undo can no longer be clicked, so nothing restores twice.
    let clickable = cx
        .update_window(h.win, |_, w, _| {
            w.try_find("undo-delete").map(|e| e.visible())
        })
        .unwrap();
    assert_ne!(
        clickable,
        Some(true),
        "the toast's Undo is gone with its entry"
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    frame(cx, h.win);
    assert!(!has(cx, &h, "undo-delete"), "Ctrl+Z dismissed the toast");
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
    assert_eq!(bin.len(), 0);
}

// Without a delete to undo the chord is the shell's again: the active tab's last edit.
#[gpui_kit::test]
fn ctrl_z_in_the_sidebar_with_nothing_deleted_undoes_the_active_tab(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    let file = "info:\n  name: j\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://x/1\"\n  headers:\n    - name: H1\n      value: one\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("j.yml", file)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let enabled = |cx: &mut TestAppContext| view_of(cx, &s.ws, &s.root, "j.yml").headers[0].enabled;
    assert!(!enabled(cx));
    let tree = s
        .shell
        .read_with(cx, |sh, cx| sh.sidebar().read(cx).tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.simulate_keystrokes(s.win, &secondary("z"));
    cx.run_until_parked();
    assert!(enabled(cx), "the header toggle was undone");
}

#[gpui_kit::test]
fn ctrl_z_in_the_filter_input_undoes_its_text_and_restores_nothing(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    cx.update_window(h.win, |_, w, cx| {
        w.click("sidebar-filter-input", cx);
        w.input("zz", cx);
    })
    .unwrap();
    frame(cx, h.win);
    let filter = |cx: &mut TestAppContext| {
        let input = h.view.read_with(cx, |s, _| s.filter().clone());
        input.read_with(cx, |i, _| i.value().to_string())
    };
    assert_eq!(filter(cx), "zz");
    press(cx, &h, &secondary("z"));
    assert_eq!(filter(cx), "", "the input's own undo");
    assert!(!h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn ctrl_z_in_the_rename_input_undoes_its_text_and_restores_nothing(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    cx.update_window(h.win, |_, w, cx| w.click(row("b.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    press(cx, &h, "f2");
    assert!(h.view.read_with(cx, |s, _| s.renaming()).is_some());
    cx.update_window(h.win, |_, w, cx| w.input("Q", cx))
        .unwrap();
    press(cx, &h, &secondary("z"));
    assert!(
        h.view.read_with(cx, |s, _| s.renaming()).is_some(),
        "the rename input kept the key"
    );
    assert!(!h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 1);
    press(cx, &h, "enter");
    frame(cx, h.win);
    assert!(!h.root.join("Q.yml").exists(), "the typed name was undone");
}

// Past the cap the oldest delete is no longer undoable, and its trash ticket is let go.
#[gpui_kit::test]
fn deletes_past_the_cap_forget_the_oldest(cx: &mut TestAppContext) {
    let cap = apinata_app::sidebar::UNDO_DELETE_CAP;
    let names: Vec<&'static str> = (0..=cap)
        .map(|i| &*Box::leak(format!("r{i:02}.yml").into_boxed_str()))
        .collect();
    let mut files = vec![("opencollection.yml", ROOT.to_string())];
    files.extend(names.iter().map(|n| (*n, request(n, "http://x/r"))));
    let bin = Arc::new(FakeTrash::new());
    let h = open_sidebar_trash(cx, files, Some(bin.clone()));
    for n in &names {
        select_and_delete(cx, &h, n);
    }
    expire_toasts(cx, &h);
    // Every ticket the instance could have handed out; only the evicted one is gone.
    for ticket in 1..=200 {
        h.ws.update(cx, |w, cx| {
            w.send(&h.root, Command::Tree(TreeOp::Restore { ticket }), cx)
        });
    }
    cx.run_until_parked();
    assert!(!h.root.join(names[0]).exists(), "the oldest was forgotten");
    for n in &names[1..] {
        assert!(h.root.join(n).exists(), "{n}");
    }
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn undo_onto_a_taken_name_leaves_the_new_file_and_says_why(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    let newer = request("newer", "http://x/9");
    std::fs::write(h.root.join("a.yml"), &newer).unwrap();
    let shown = undo_toasts(cx, &h);
    assert_eq!(
        std::fs::read_to_string(h.root.join("a.yml")).unwrap(),
        newer
    );
    assert_eq!(bin.len(), 1, "the trashed copy stays in the trash");
    // One error toast replaces the Undo toast; its text is checked in the batch tests.
    assert_eq!(shown, 1);
    assert!(has(cx, &h, "toast-error"));
}

#[gpui_kit::test]
fn deleting_several_undoes_them_together(cx: &mut TestAppContext) {
    let (h, bin) = with_trash(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    focus_tree(cx, &h);
    press(cx, &h, "delete");
    frame(cx, h.win);
    assert!(!confirm_open(cx));
    assert!(!h.root.join("a.yml").exists() && !h.root.join("c.yml").exists());
    assert_eq!(bin.len(), 2);
    click_undo(cx, &h);
    assert!(h.root.join("a.yml").exists() && h.root.join("c.yml").exists());
}

#[gpui_kit::test]
fn undoing_several_shows_one_restored_toast(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    delete_a_and_c(cx, &h);
    assert_eq!(
        undo_toasts(cx, &h),
        1,
        "one summary, not one toast per file"
    );
    assert!(h.root.join("a.yml").exists() && h.root.join("c.yml").exists());
    assert_eq!(bin.len(), 0);
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
}

#[gpui_kit::test]
fn undoing_one_shows_one_restored_toast(cx: &mut TestAppContext) {
    let (h, _bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    assert_eq!(undo_toasts(cx, &h), 1);
    assert!(h.root.join("a.yml").exists());
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
}

#[gpui_kit::test]
fn a_partly_failed_undo_is_one_warning(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    delete_a_and_c(cx, &h);
    let newer = request("newer", "http://x/9");
    std::fs::write(h.root.join("c.yml"), &newer).unwrap();
    assert_eq!(undo_toasts(cx, &h), 1);
    assert!(h.root.join("a.yml").exists());
    assert_eq!(
        std::fs::read_to_string(h.root.join("c.yml")).unwrap(),
        newer
    );
    assert_eq!(bin.len(), 1);
    assert!(has(cx, &h, "toast-warning"));
}

#[gpui_kit::test]
fn a_request_with_unsaved_edits_still_asks_first(cx: &mut TestAppContext) {
    let (h, bin) = with_trash(cx);
    let tab = open_tab(cx, &h.ws, &h.root, "a.yml");
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("a.yml", "http://x/edited"), cx)
    });
    cx.run_until_parked();
    let _ = tab;
    select_and_delete(cx, &h, "a.yml");
    assert!(confirm_open(cx), "unsaved edits follow the existing prompt");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn without_a_trash_the_prompt_says_the_delete_is_permanent(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    select_and_delete(cx, &h, "a.yml");
    assert!(confirm_open(cx), "no trash keeps the prompt");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists());
}

#[gpui_kit::test]
fn a_broken_trash_prompts_like_no_trash(cx: &mut TestAppContext) {
    let h = open_sidebar_trash(cx, multi_files(), Some(Arc::new(FakeTrash::broken())));
    select_and_delete(cx, &h, "a.yml");
    assert!(confirm_open(cx));
    assert!(h.root.join("a.yml").exists());
}

#[allow(dead_code)]
fn _action_exists(_: DeleteSelected) {}

#[test]
fn the_permanent_prompts_say_so_and_the_trash_prompts_do_not() {
    use apinata_app::sidebar::batch::{bulk_delete_ask, permanent_delete_ask, single_delete_ask};
    let says = |ask: apinata_app::confirm::Ask| {
        ask.detail
            .map(|d| d.to_lowercase().contains("permanent"))
            .unwrap_or(false)
    };
    assert!(says(single_delete_ask("a", false, true)));
    assert!(says(single_delete_ask("f", true, true)));
    assert!(!says(single_delete_ask("a", false, false)));
    assert!(says(permanent_delete_ask(3, true, true)));
    assert!(says(permanent_delete_ask(2, false, false)));
    assert!(!says(bulk_delete_ask(2, true, true)));
}

#[gpui_kit::test]
fn a_bulk_undo_gives_every_item_its_old_number_back(cx: &mut TestAppContext) {
    let numbered = |name: &str, seq: u32| {
        request(name, &format!("http://x/{name}")).replace("seq: 1", &format!("seq: {seq}"))
    };
    let files = vec![
        ("opencollection.yml", ROOT.to_string()),
        ("a.yml", numbered("a", 1)),
        ("b.yml", numbered("b", 2)),
        ("c.yml", numbered("c", 3)),
    ];
    let bin = Arc::new(FakeTrash::new());
    let h = open_sidebar_trash(cx, files, Some(bin));
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("b.yml"));
    focus_tree(cx, &h);
    press(cx, &h, "delete");
    frame(cx, h.win);
    click_undo(cx, &h);
    let seq = |rel: &str| {
        let text = std::fs::read_to_string(h.root.join(rel)).unwrap();
        text.lines()
            .find(|l| l.contains("seq:"))
            .unwrap()
            .trim()
            .to_owned()
    };
    assert_eq!(
        [seq("a.yml"), seq("b.yml"), seq("c.yml")],
        ["seq: 1", "seq: 2", "seq: 3"]
    );
}

#[gpui_kit::test]
fn undo_after_a_reopen_restores_nothing_and_says_nothing(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    h.ws.update(cx, |w, cx| w.close(&h.root, cx));
    cx.run_until_parked();
    h.ws.update(cx, |w, cx| w.open(h.root.clone(), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(has(cx, &h, "undo-delete"), "the toast outlives the reopen");
    click_undo(cx, &h);
    // A later notice settles whatever the undo sent; a stale ticket must not have been sent.
    select_and_delete(cx, &h, "b.yml");
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
    assert!(!h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 2);
}

#[gpui_kit::test]
fn an_old_toast_closing_after_a_reopen_keeps_the_new_delete_undoable(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    h.ws.update(cx, |w, cx| w.close(&h.root, cx));
    cx.run_until_parked();
    h.ws.update(cx, |w, cx| w.open(h.root.clone(), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    // The reopened instance numbers its commands afresh, so this delete's ticket can equal
    // the first one's; the first toast then expires while the second still shows.
    cx.executor().advance_clock(Duration::from_secs(3));
    select_and_delete(cx, &h, "b.yml");
    cx.executor().advance_clock(Duration::from_secs(3));
    frame(cx, h.win);
    assert!(has(cx, &h, "undo-delete"), "the second toast still shows");
    click_undo(cx, &h);
    assert!(h.root.join("b.yml").exists(), "the second delete undoes");
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn deleting_open_requests_shows_only_the_delete_toast(cx: &mut TestAppContext) {
    let (h, _bin) = shell_with_trash(cx);
    for rel in ["a.yml", "c.yml"] {
        h.ws.update(cx, |w, cx| w.open_item(&h.root, ip(rel), false, cx))
            .expect("a request");
    }
    frame(cx, h.win);
    delete_a_and_c(cx, &h);
    assert!(!h.root.join("a.yml").exists() && !h.root.join("c.yml").exists());
    // The delete toast already says what went; a second one about its tabs is noise.
    assert_eq!(toasts(cx, &h), 1);
    assert!(has(cx, &h, "undo-delete"));
}

// A reopened collection is a new instance, so a delete made before it closed is not undone;
// the chord goes on to the active tab's undo instead.
#[gpui_kit::test]
fn ctrl_z_after_a_reopen_restores_nothing_and_undoes_the_active_tab(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    let j = "info:\n  name: j\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://x/1\"\n  headers:\n    - name: H1\n      value: one\n";
    let mut files = multi_files();
    files.push(("j.yml", j.to_string()));
    let (h, bin) = shell_with_trash_files(cx, files);
    select_and_delete(cx, &h, "a.yml");
    expire_toasts(cx, &h);
    h.ws.update(cx, |w, cx| w.close(&h.root, cx));
    cx.run_until_parked();
    h.ws.update(cx, |w, cx| w.open(h.root.clone(), cx));
    cx.run_until_parked();
    open_tab(cx, &h.ws, &h.root, "j.yml");
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx);
    })
    .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let enabled = |cx: &mut TestAppContext| view_of(cx, &h.ws, &h.root, "j.yml").headers[0].enabled;
    assert!(!enabled(cx));
    focus_tree(cx, &h);
    ctrl_z(cx, &h);
    assert!(enabled(cx), "the header toggle was undone");
    assert!(!h.root.join("a.yml").exists());
    let snap = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    assert!(!snap.content.requests.contains_key(&ip("a.yml")));
    assert_eq!(bin.len(), 1);
}

// Clicks into the open tab's URL input, where Ctrl+Z is the input's own.
fn focus_url(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    frame(cx, h.win);
}

// Opens `rels` as tabs, the last in front, and leaves the keyboard in that tab's URL input.
fn open_tabs_with_focus_in_url(cx: &mut TestAppContext, h: &SidebarHarness, rels: &[&str]) {
    for rel in rels {
        open_tab(cx, &h.ws, &h.root, rel);
    }
    frame(cx, h.win);
    focus_url(cx, h);
}

// Right-clicks `rel` and clicks its menu's entry `ix`.
fn menu_click(cx: &mut TestAppContext, h: &SidebarHarness, rel: &str, ix: usize) {
    cx.update_window(h.win, |_, w, cx| w.right_click(row(rel), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
}

fn menu_delete(cx: &mut TestAppContext, h: &SidebarHarness, rel: &str, kind: NodeKind) {
    let entries = menu_entries(kind);
    let ix = (entries.iter().position(|e| *e == MenuEntry::Delete)).expect("Delete is offered");
    menu_click(cx, h, rel, ix);
}

fn assert_restored(cx: &mut TestAppContext, h: &SidebarHarness, rel: &str, bin: &FakeTrash) {
    assert!(h.root.join(rel).exists(), "{rel} came back");
    assert_eq!(bin.len(), 0);
    assert!(!has(cx, h, "toast-error") && !has(cx, h, "toast-warning"));
}

#[gpui_kit::test]
fn ctrl_z_right_after_delete_key_restores_the_active_tabs_request(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tab(cx, &h.ws, &h.root, "c.yml");
    open_tab(cx, &h.ws, &h.root, "a.yml");
    frame(cx, h.win);
    select_and_delete(cx, &h, "a.yml");
    assert!(!h.root.join("a.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
}

#[gpui_kit::test]
fn ctrl_z_right_after_a_menu_delete_restores_from_a_tab_with_focus(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["c.yml", "a.yml"]);
    menu_delete(cx, &h, "a.yml", NodeKind::Request);
    assert!(!h.root.join("a.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
}

// The row deleted is not the tab in front, so no tab closing moves focus at all.
#[gpui_kit::test]
fn ctrl_z_right_after_a_menu_delete_of_a_background_row_restores_it(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["a.yml"]);
    menu_delete(cx, &h, "c.yml", NodeKind::Request);
    assert!(!h.root.join("c.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "c.yml", &bin);
}

#[gpui_kit::test]
fn ctrl_z_right_after_a_folder_menu_delete_restores_what_is_inside(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["a.yml"]);
    menu_delete(cx, &h, "f", NodeKind::Folder);
    assert!(!h.root.join("f").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "f/x.yml", &bin);
}

#[gpui_kit::test]
fn ctrl_z_right_after_a_bulk_menu_delete_restores_the_set(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    open_tabs_with_focus_in_url(cx, &h, &["a.yml"]);
    let entries = bulk_entries(2, Some(2));
    let ix = (entries
        .iter()
        .position(|e| matches!(e, BulkEntry::Delete(_))))
    .expect("Delete is offered");
    menu_click(cx, &h, "a.yml", ix);
    assert!(!h.root.join("a.yml").exists() && !h.root.join("c.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
    assert!(h.root.join("c.yml").exists());
}

// Unsaved edits ask first; the dialog hands focus back to what held it, and the answer's
// delete still has to end up where Ctrl+Z undoes it.
#[gpui_kit::test]
fn ctrl_z_right_after_a_prompted_delete_restores_it(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tab(cx, &h.ws, &h.root, "a.yml");
    frame(cx, h.win);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    frame(cx, h.win);
    menu_delete(cx, &h, "a.yml", NodeKind::Request);
    assert!(confirm_open(cx), "unsaved edits ask first");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
}

#[gpui_kit::test]
fn ctrl_z_right_after_a_prompted_bulk_delete_restores_the_set(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tab(cx, &h.ws, &h.root, "a.yml");
    frame(cx, h.win);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(row("c.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("a.yml"));
    focus_url(cx, &h);
    let entries = bulk_entries(2, Some(2));
    let ix = (entries
        .iter()
        .position(|e| matches!(e, BulkEntry::Delete(_))))
    .expect("Delete is offered");
    menu_click(cx, &h, "a.yml", ix);
    assert!(confirm_open(cx), "unsaved edits ask first");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists() && !h.root.join("c.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
    assert!(h.root.join("c.yml").exists());
}

// The delete leaves focus on the tree; going back to an editor gives Ctrl+Z back to it.
#[gpui_kit::test]
fn ctrl_z_back_in_an_editor_after_a_delete_is_the_editors_own(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["c.yml", "a.yml"]);
    menu_delete(cx, &h, "a.yml", NodeKind::Request);
    expire_toasts(cx, &h);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    ctrl_z(cx, &h);
    assert!(!h.root.join("a.yml").exists(), "the delete stays");
    assert_eq!(bin.len(), 1);
}

// Each undo is one Ctrl+Z, with no click between, however the deletes were made.
#[gpui_kit::test]
fn ctrl_z_keeps_undoing_after_the_first_restore(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["a.yml"]);
    menu_delete(cx, &h, "c.yml", NodeKind::Request);
    menu_delete(cx, &h, "b.yml", NodeKind::Request);
    ctrl_z(cx, &h);
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "b.yml", &bin);
    assert!(h.root.join("c.yml").exists());
}

// Selects `rel`, then presses Delete and `undos` Ctrl+Z in one go, so every key lands before
// the collection can report the trash.
fn delete_then_ctrl_z_at_once(cx: &mut TestAppContext, h: &SidebarHarness, rel: &str, undos: u32) {
    cx.update_window(h.win, |_, w, cx| w.click(row(rel), cx))
        .unwrap();
    focus_tree(cx, h);
    cx.update_window(h.win, |_, w, cx| {
        w.press("delete", cx);
        for _ in 0..undos {
            w.press(&secondary("z"), cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    frame(cx, h.win);
}

#[gpui_kit::test]
fn ctrl_z_while_the_delete_is_landing_restores_it(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    delete_then_ctrl_z_at_once(cx, &h, "a.yml", 1);
    assert_restored(cx, &h, "a.yml", &bin);
    assert!(!has(cx, &h, "undo-delete"), "the Undo toast went with it");
}

// The delete still landing is the newest, so an older finished one stays in the trash.
#[gpui_kit::test]
fn ctrl_z_while_a_delete_is_landing_passes_over_an_older_delete(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    delete_then_ctrl_z_at_once(cx, &h, "c.yml", 1);
    assert!(h.root.join("c.yml").exists(), "the newest came back");
    assert!(!h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn each_ctrl_z_pressed_while_a_delete_lands_undoes_one_delete(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    select_and_delete(cx, &h, "a.yml");
    delete_then_ctrl_z_at_once(cx, &h, "c.yml", 2);
    assert!(h.root.join("c.yml").exists() && h.root.join("a.yml").exists());
    assert_eq!(bin.len(), 0);
}

// The key belongs to the delete: it does not fall through to the tab's undo while it lands.
#[gpui_kit::test]
fn ctrl_z_while_the_delete_is_landing_leaves_the_tabs_edit_alone(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    let j = "info:\n  name: j\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://x/1\"\n  headers:\n    - name: H1\n      value: one\n";
    let mut files = multi_files();
    files.push(("j.yml", j.to_string()));
    let (h, bin) = shell_with_trash_files(cx, files);
    open_tab(cx, &h.ws, &h.root, "j.yml");
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx);
    })
    .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    delete_then_ctrl_z_at_once(cx, &h, "a.yml", 1);
    assert!(!view_of(cx, &h.ws, &h.root, "j.yml").headers[0].enabled);
    assert_restored(cx, &h, "a.yml", &bin);
}

// A key that waited on a collection now closed is dropped, not spent on a later delete.
#[gpui_kit::test]
fn a_ctrl_z_waiting_on_a_closed_collection_never_undoes_a_later_delete(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    cx.update_window(h.win, |_, w, cx| {
        w.press("delete", cx);
        w.press(&secondary("z"), cx);
    })
    .unwrap();
    h.ws.update(cx, |w, cx| w.close(&h.root, cx));
    cx.run_until_parked();
    h.ws.update(cx, |w, cx| w.open(h.root.clone(), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    let before = bin.len();
    select_and_delete(cx, &h, "c.yml");
    assert!(!h.root.join("c.yml").exists(), "the new delete stands");
    assert_eq!(bin.len(), before + 1);
}

// How a test answers the delete prompt.
#[derive(Clone, Copy)]
enum Answer {
    Click,
    Enter,
}

fn answer_delete(cx: &mut TestAppContext, answer: Answer) {
    match answer {
        Answer::Click => answer_confirm(cx, "confirm-delete"),
        Answer::Enter => {
            let win = confirm_window(cx).expect("a confirm dialog is open");
            cx.update_window(win, |_, w, cx| w.press("enter", cx))
                .unwrap();
            cx.run_until_parked();
        }
    }
    // The dialog gives focus back only once its closing animation ends.
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

// Tabs c and a, a in front with unsaved edits, so the delete of a asks and a tab stays behind.
fn two_tabs_a_dirty(cx: &mut TestAppContext, h: &SidebarHarness) {
    open_tab(cx, &h.ws, &h.root, "c.yml");
    frame(cx, h.win);
    open_tab(cx, &h.ws, &h.root, "a.yml");
    frame(cx, h.win);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    frame(cx, h.win);
}

fn prompted_delete_with_a_tab_behind(cx: &mut TestAppContext, answer: Answer) {
    let (h, bin) = shell_with_trash(cx);
    open_tab(cx, &h.ws, &h.root, "b.yml");
    frame(cx, h.win);
    two_tabs_a_dirty(cx, &h);
    menu_delete(cx, &h, "a.yml", NodeKind::Request);
    assert!(confirm_open(cx), "unsaved edits ask first");
    answer_delete(cx, answer);
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
}

#[gpui_kit::test]
fn ctrl_z_after_a_prompted_delete_clicked_with_a_tab_behind(cx: &mut TestAppContext) {
    prompted_delete_with_a_tab_behind(cx, Answer::Click);
}

#[gpui_kit::test]
fn ctrl_z_after_a_prompted_delete_entered_with_a_tab_behind(cx: &mut TestAppContext) {
    prompted_delete_with_a_tab_behind(cx, Answer::Enter);
}

fn prompted_bulk_delete_with_a_tab_behind(cx: &mut TestAppContext, answer: Answer) {
    let (h, bin) = shell_with_trash(cx);
    two_tabs_a_dirty(cx, &h);
    cx.update_window(h.win, |_, w, cx| w.click(row("b.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("a.yml"));
    focus_url(cx, &h);
    let entries = bulk_entries(2, Some(2));
    let ix = (entries
        .iter()
        .position(|e| matches!(e, BulkEntry::Delete(_))))
    .expect("Delete is offered");
    menu_click(cx, &h, "a.yml", ix);
    assert!(confirm_open(cx), "unsaved edits ask first");
    answer_delete(cx, answer);
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists() && !h.root.join("b.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
    assert!(h.root.join("b.yml").exists());
}

#[gpui_kit::test]
fn ctrl_z_after_a_prompted_bulk_delete_clicked_with_a_tab_behind(cx: &mut TestAppContext) {
    prompted_bulk_delete_with_a_tab_behind(cx, Answer::Click);
}

#[gpui_kit::test]
fn ctrl_z_after_a_prompted_bulk_delete_entered_with_a_tab_behind(cx: &mut TestAppContext) {
    prompted_bulk_delete_with_a_tab_behind(cx, Answer::Enter);
}

// Runs the palette's "Delete selected" from wherever focus is now.
fn palette_delete(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.simulate_keystrokes(h.win, &secondary("shift-p"));
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.input("Delete selected", cx))
        .unwrap();
    frame(cx, h.win);
    cx.simulate_keystrokes(h.win, "enter");
    cx.run_until_parked();
    frame(cx, h.win);
}

#[gpui_kit::test]
fn ctrl_z_right_after_a_palette_delete_restores_from_a_tab_with_focus(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tab(cx, &h.ws, &h.root, "c.yml");
    frame(cx, h.win);
    open_tab(cx, &h.ws, &h.root, "a.yml");
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    focus_url(cx, &h);
    palette_delete(cx, &h);
    assert!(!h.root.join("a.yml").exists());
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
}

// The rule has one end: once the delete is undone the editor's undo is its own again.
#[gpui_kit::test]
fn the_editors_undo_is_its_own_once_the_delete_is_undone(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["c.yml", "a.yml"]);
    menu_delete(cx, &h, "c.yml", NodeKind::Request);
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "c.yml", &bin);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    let url = |cx: &mut TestAppContext| view_of(cx, &h.ws, &h.root, "a.yml").url.clone();
    assert!(url(cx).ends_with("/mine"));
    ctrl_z(cx, &h);
    cx.run_until_parked();
    assert!(!url(cx).ends_with("/mine"), "the typing was undone");
    assert!(h.root.join("c.yml").exists());
}

// Moving on to an editor with a delete still undoable leaves the delete in the trash.
#[gpui_kit::test]
fn the_editors_undo_is_its_own_with_a_delete_still_undoable(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs_with_focus_in_url(cx, &h, &["c.yml", "a.yml"]);
    menu_delete(cx, &h, "c.yml", NodeKind::Request);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    ctrl_z(cx, &h);
    cx.run_until_parked();
    let url = view_of(cx, &h.ws, &h.root, "a.yml").url.clone();
    assert!(!url.ends_with("/mine"), "the typing was undone");
    assert!(!h.root.join("c.yml").exists());
    assert_eq!(bin.len(), 1);
}

// Each key undoes the delete made before it, whatever comes after it. All the keys land in
// one go, so neither delete has reported when the next key arrives.
#[gpui_kit::test]
fn ctrl_z_while_landing_undoes_the_delete_before_it_not_the_one_after(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    cx.update_window(h.win, |_, w, cx| {
        w.press("delete", cx);
        w.press(&secondary("z"), cx);
        w.press("down", cx);
        w.press("delete", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    frame(cx, h.win);
    assert!(h.root.join("a.yml").exists(), "the first delete came back");
    assert!(!h.root.join("b.yml").exists(), "the later delete stands");
    assert_eq!(bin.len(), 1);
}

// A hidden tree cannot take focus, so the palette's delete leaves the keyboard in the tab and
// the toast is the way back.
#[gpui_kit::test]
fn with_the_sidebar_hidden_a_palette_delete_keeps_focus_in_the_tab(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tab(cx, &h.ws, &h.root, "c.yml");
    frame(cx, h.win);
    open_tab(cx, &h.ws, &h.root, "a.yml");
    frame(cx, h.win);
    focus_url(cx, &h);
    press(cx, &h, &secondary("b"));
    frame(cx, h.win);
    palette_delete(cx, &h);
    assert!(!h.root.join("a.yml").exists());
    let in_tab = cx
        .update_window(h.win, |_, w, _| {
            (w.context_stack().iter()).any(|c| format!("{c:?}").contains("RequestEditor"))
        })
        .unwrap();
    assert!(in_tab, "focus stays in the tab");
    click_undo(cx, &h);
    frame(cx, h.win);
    assert_restored(cx, &h, "a.yml", &bin);
}
