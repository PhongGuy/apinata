use crate::support;

use apinata_app::closing::{CloseScope, close_detail, confirm_close_or_keep, dirty_targets};
use apinata_app::command::{Command, Edit, Target};
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use std::cell::Cell;
use std::rc::Rc;
use support::*;

fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
    VisualTestContext::from_window(s.win, cx).simulate_close()
}

fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

fn file(s: &Started, rel: &str) -> String {
    std::fs::read_to_string(s.root.join(rel)).unwrap()
}

fn dirty_a(cx: &mut TestAppContext, s: &Started) {
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn dirty_targets_cover_every_document_kind(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/folder.yml", &folder("Users")),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Folder(ip("users")),
            edit: Edit::Docs("d".into()),
        },
    );
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Collection,
            edit: Edit::Docs("c".into()),
        },
    );
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Environment("dev".into()),
            edit: Edit::Color(Some("#112233".into())),
        },
    );
    cx.run_until_parked();
    assert_eq!(
        dirty_targets(&snap(cx, &model).content),
        [
            Target::Request(ip("a.yml")),
            Target::Folder(ip("users")),
            Target::Collection,
            Target::Environment("dev".into())
        ]
    );
}

#[gpui_kit::test]
fn a_command_not_yet_applied_counts_as_unsaved(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Reload(Vec::new()), cx));
    let unsaved = s.ws.read_with(cx, |w, cx| w.unsaved(None, cx));
    let [u] = unsaved.as_slice() else {
        panic!("{unsaved:?}")
    };
    assert_eq!((&u.root, u.targets.len(), u.pending), (&s.root, 0, 1));
    cx.run_until_parked();
    assert!(s.ws.read_with(cx, |w, cx| w.unsaved(None, cx)).is_empty());
}

#[gpui_kit::test]
fn a_clean_window_closes_without_a_prompt(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert!(close_window(cx, &s));
    assert!(!confirm_open(cx));
}

#[gpui_kit::test]
fn cancel_keeps_the_window_and_the_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    assert!(!close_window(cx, &s));
    assert!(!close_window(cx, &s), "a second request while prompting");
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(!confirm_open(cx), "only one prompt was opened");
    assert_eq!(windows(cx), 1);
    assert!(view_of(cx, &s.ws, &s.root, "a.yml").dirty);
}

#[gpui_kit::test]
fn save_all_saves_then_closes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert!(file(&s, "a.yml").contains("http://x/9"));
    assert_eq!(windows(cx), 0);
}

#[gpui_kit::test]
fn discard_closes_without_saving(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert_eq!(windows(cx), 0);
    assert!(file(&s, "a.yml").contains("http://x/0"));
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_window_open(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    // Changed on disk under the draft and not yet reloaded, so the save conflicts.
    std::fs::write(s.root.join("a.yml"), request("a", "http://x/disk")).unwrap();
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert_eq!(windows(cx), 1);
    assert!(
        file(&s, "a.yml").contains("http://x/disk"),
        "the conflict is not overwritten"
    );
}

#[gpui_kit::test]
fn closing_a_dirty_collection_from_the_sidebar_prompts(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    choose_menu(cx, s.win, "", NodeKind::Collection, MenuEntry::Close);
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert!(s.ws.read_with(cx, |w, _| w.collections().is_empty()));
    assert_eq!(windows(cx), 1);
    assert!(file(&s, "a.yml").contains("http://x/0"));
}

// Runs the close flow for the collection at `s.root`; the returned flag turns true if the
// flow reports the collection as kept open.
fn close_collection(cx: &mut TestAppContext, s: &Started) -> Rc<Cell<bool>> {
    let kept = Rc::new(Cell::new(false));
    let (ws, scope, flag) = (
        s.ws.clone(),
        CloseScope::Collection(s.root.clone()),
        kept.clone(),
    );
    cx.update_window(s.win, |_, w, cx| {
        confirm_close_or_keep(ws, scope, w, cx, move |_| flag.set(true))
    })
    .unwrap();
    cx.run_until_parked();
    kept
}

#[gpui_kit::test]
fn a_collection_saved_and_closed_is_not_reported_kept(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    let kept = close_collection(cx, &s);
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert!(s.ws.read_with(cx, |w, _| w.collections().is_empty()));
    assert!(!kept.get());
}

#[gpui_kit::test]
fn a_collection_whose_save_fails_is_reported_kept(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    std::fs::write(s.root.join("a.yml"), request("a", "http://x/disk")).unwrap();
    let kept = close_collection(cx, &s);
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert_eq!(s.ws.read_with(cx, |w, _| w.collections().len()), 1);
    assert!(kept.get());
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| (*n).to_owned()).collect()
}

#[test]
fn the_detail_names_the_changes_and_agrees_in_number() {
    assert_eq!(
        close_detail(&names(&["a"]), 1, false),
        "\"a\" has unsaved changes."
    );
    assert_eq!(
        close_detail(&names(&["a", "b"]), 1, false),
        "\"a\" and \"b\" have unsaved changes."
    );
    assert_eq!(
        close_detail(&names(&["Get post", "b", "c"]), 2, false),
        "\"Get post\" and 2 others have unsaved changes in 2 collections."
    );
    assert_eq!(
        close_detail(&[], 2, false),
        "Changes to 2 collections are still being applied."
    );
    assert_eq!(
        close_detail(&[], 1, false),
        "Changes to 1 collection are still being applied."
    );
}

#[test]
fn the_detail_names_the_scratch_pad_as_itself() {
    assert_eq!(
        close_detail(&names(&["x"]), 0, true),
        "\"x\" has unsaved changes in the scratch pad."
    );
    assert_eq!(
        close_detail(&names(&["x", "y", "z"]), 2, true),
        "\"x\" and 2 others have unsaved changes in 2 collections and the scratch pad."
    );
    assert_eq!(
        close_detail(&[], 0, true),
        "Changes to the scratch pad are still being applied."
    );
}

// The prompt counts the scratch pad apart from the collections the user opened.
#[gpui_kit::test]
fn a_scratch_draft_is_named_as_the_scratch_pad(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &s.ws);
    let unsaved = [apinata_app::closing::Unsaved::new(
        root,
        vec![apinata_app::command::Target::Request(item)],
        0,
    )];
    let detail = s.ws.read_with(cx, |w, cx| {
        apinata_app::closing::unsaved_detail(w, &unsaved, &CloseScope::Window, cx)
    });
    assert_eq!(
        detail,
        "\"Untitled request\" has unsaved changes in the scratch pad."
    );
}

// Types into a saved request and closes its tab in the same breath, before the actor has
// applied the keystroke.
fn type_then_close_tab(
    cx: &mut TestAppContext,
    s: &Started,
    item: &str,
) -> apinata_app::tabs::TabId {
    let tab = open_tab(cx, &s.ws, &s.root, item);
    cx.run_until_parked();
    edit_item(
        cx,
        s,
        item,
        apinata_app::command::Edit::Url("http://127.0.0.1:9/typed".into()),
    );
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    tab
}

#[gpui_kit::test]
fn closing_a_tab_right_after_typing_prompts_for_the_edit(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = type_then_close_tab(cx, &s, "a.yml");
    cx.run_until_parked();
    assert!(confirm_open(cx), "the unacknowledged edit prompts");
    assert!(s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()));
    answer_confirm(cx, "confirm-cancel");
    assert!(s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()));
}

#[gpui_kit::test]
fn a_tab_close_waits_no_longer_than_the_settle_for_a_stalled_actor(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::SeqCst;
    let stall = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let col = two_requests();
    let s = started(cx, &col);
    let faults = apinata_app::actor::Faults {
        stall: Some(stall.clone()),
        ..Default::default()
    };
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.close(&root, cx);
        w.set_open_faults(faults);
        w.open(root.clone(), cx);
    });
    cx.run_until_parked();
    stall.store(true, SeqCst);
    let tab = type_then_close_tab(cx, &s, "a.yml");
    cx.run_until_parked();
    assert!(!confirm_open(cx), "still waiting for the actor");
    assert!(s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()));
    cx.executor()
        .advance_clock(apinata_app::closing::SETTLE * 2);
    cx.run_until_parked();
    stall.store(false, SeqCst);
    assert!(confirm_open(cx), "the prompt after the wait");
    assert!(s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()));
}

#[gpui_kit::test]
fn closing_a_tab_twice_while_waiting_opens_one_prompt(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = type_then_close_tab(cx, &s, "a.yml");
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    assert!(!confirm_open(cx), "a second prompt would still be open");
}

// Reopens the collection under an actor that stops applying commands while its flag is set.
fn stallable(
    cx: &mut TestAppContext,
    s: &Started,
    stall: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let faults = apinata_app::actor::Faults {
        stall: Some(stall),
        ..Default::default()
    };
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.close(&root, cx);
        w.set_open_faults(faults);
        w.open(root.clone(), cx);
    });
    cx.run_until_parked();
}

// A tab close is settling on a stalled actor and the window is asked to close meanwhile.
fn window_close_during_a_tab_settle(
    cx: &mut TestAppContext,
) -> (Started, apinata_app::tabs::TabId) {
    use std::sync::atomic::Ordering::SeqCst;
    let col = two_requests();
    let s = started(cx, &col);
    let stall = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    stallable(cx, &s, stall.clone());
    stall.store(true, SeqCst);
    let tab = type_then_close_tab(cx, &s, "a.yml");
    cx.run_until_parked();
    assert!(!close_window(cx, &s), "held while the close settles");
    assert!(!confirm_open(cx), "still waiting for the actor");
    cx.executor()
        .advance_clock(apinata_app::closing::SETTLE * 2);
    cx.run_until_parked();
    stall.store(false, SeqCst);
    (s, tab)
}

#[gpui_kit::test]
fn a_window_close_during_a_settle_asks_after_the_tab_prompt(cx: &mut TestAppContext) {
    let (s, tab) = window_close_during_a_tab_settle(cx);
    assert!(confirm_open(cx), "the tab's prompt comes first");
    answer_confirm(cx, "confirm-cancel");
    assert!(s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()));
    // The stalled command is still unapplied, so the window close settles in its turn.
    cx.executor()
        .advance_clock(apinata_app::closing::SETTLE * 2);
    cx.run_until_parked();
    assert!(confirm_open(cx), "then the queued window close asks");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(windows(cx), 1);
}

#[gpui_kit::test]
fn a_window_close_during_a_settle_runs_after_the_tab_is_discarded(cx: &mut TestAppContext) {
    let (_s, _tab) = window_close_during_a_tab_settle(cx);
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-discard");
    cx.executor()
        .advance_clock(apinata_app::closing::SETTLE * 2);
    cx.run_until_parked();
    assert!(confirm_open(cx), "the queued close runs and asks");
    answer_confirm(cx, "confirm-discard");
    assert_eq!(windows(cx), 0, "and closes on Discard");
}

// The window close is asked while the tab's own prompt shows, with nothing left to settle, so
// the tab prompt's answer releases it straight away.
#[gpui_kit::test]
fn a_window_close_during_the_tab_prompt_asks_after_cancel(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    dirty_a(cx, &s);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx), "the tab's prompt");
    assert!(!close_window(cx, &s), "held while the tab prompt shows");
    answer_confirm(cx, "confirm-cancel");
    assert!(s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()));
    assert!(confirm_open(cx), "then the queued window close asks");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(windows(cx), 1);
}

// A window close queued behind the tab prompt goes with the window when it closes another way,
// rather than holding its workspace for good.
#[gpui_kit::test]
fn a_queued_window_close_goes_with_a_window_closed_another_way(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    dirty_a(cx, &s);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!close_window(cx, &s), "held while the tab prompt shows");
    let id = s.win.window_id();
    assert!(cx.update(|cx| apinata_app::closing::close_queued(id, cx)));
    cx.update_window(s.win, |_, w, _| w.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(windows(cx), 0);
    assert!(!cx.update(|cx| apinata_app::closing::close_queued(id, cx)));
}

fn prompt_detail(cx: &mut TestAppContext) -> Option<String> {
    let win = confirm_window(cx).expect("the close prompt");
    cx.update_window(win, |_, w, _| {
        w.find("confirm-detail").label().map(str::to_owned)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_window_prompt_names_the_changed_requests(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("b.yml", "http://x/8"), cx)
    });
    cx.run_until_parked();
    assert!(!close_window(cx, &s));
    assert_eq!(
        prompt_detail(cx).as_deref(),
        Some("\"a\" and \"b\" have unsaved changes.")
    );
    answer_confirm(cx, "confirm-cancel");
}

// Closing one collection names it as this collection rather than counting one collection.
#[gpui_kit::test]
fn closing_one_collection_says_this_collection(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    dirty_a(cx, &s);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("b.yml", "http://x/8"), cx)
    });
    cx.run_until_parked();
    close_collection(cx, &s);
    let win = confirm_window(cx).expect("the close prompt");
    let detail = cx
        .update_window(win, |_, w, _| {
            w.find("confirm-detail").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        detail.as_deref(),
        Some("\"a\" and \"b\" have unsaved changes in this collection.")
    );
    answer_confirm(cx, "confirm-cancel");
}
