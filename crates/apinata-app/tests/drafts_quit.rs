use crate::support;

use std::cell::Cell;
use std::rc::Rc;

use apinata_app::closing::{CloseScope, confirm_close_or_keep};
use apinata_app::command::{Command, Edit, Target};
use apinata_app::draft_file::MAX_DRAFT_BYTES;
use apinata_app::draft_store::Drafts;
use apinata_app::history_key::history_key;
use apinata_core::ItemPath;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use support::*;

// Sends `edit` and lets the store fetch the root's secret list.
fn edit(cx: &mut TestAppContext, s: &Started, item: &str, edit: Edit) {
    edit_item(cx, s, item, edit);
    cx.run_until_parked();
}

fn new_draft(cx: &mut TestAppContext, s: &Started) -> (String, ItemPath) {
    new_draft_at(cx, s, ItemPath::root())
}

fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
    VisualTestContext::from_window(s.win, cx).simulate_close()
}

#[gpui_kit::test]
fn quitting_with_only_kept_drafts_asks_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/latest".into()));
    assert!(close_window(cx, &s), "no prompt for a kept draft");
    assert!(!confirm_open(cx));
    let text = std::fs::read_to_string(draft_path(&s, &hk)).expect("written at close");
    assert!(text.contains("http://127.0.0.1:9/latest"), "{text}");
}

#[gpui_kit::test]
fn quitting_with_an_unkept_draft_prompts_for_it_alone(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (small, small_key) = new_draft(cx, &s);
    edit(cx, &s, &small, Edit::Url("http://127.0.0.1:9/small".into()));
    let (big, big_key) = new_draft(cx, &s);
    edit(cx, &s, &big, Edit::Docs("x".repeat(MAX_DRAFT_BYTES)));
    assert!(!close_window(cx, &s));
    assert!(confirm_open(cx));
    let unsaved = s.ws.read_with(cx, |w, cx| w.unsaved(None, cx));
    let targets: Vec<_> = unsaved.iter().flat_map(|u| u.targets.clone()).collect();
    assert_eq!(targets, [Target::Request(ip(&big))]);
    answer_confirm(cx, "confirm-discard");
    assert!(draft_path(&s, &small_key).exists());
    assert!(!draft_path(&s, &big_key).exists());
}

#[gpui_kit::test]
fn a_draft_whose_context_cannot_be_built_prompts_at_quit(cx: &mut TestAppContext) {
    let col = two_requests();
    std::fs::create_dir(col.path().join(".env")).unwrap();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    assert!(!close_window(cx, &s));
    assert!(confirm_open(cx));
    assert!(!draft_path(&s, &hk).exists());
}

#[gpui_kit::test]
fn an_edit_made_after_the_context_broke_prompts_at_quit(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    assert!(draft_path(&s, &hk).exists());
    std::fs::create_dir(col.path().join(".env")).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    cx.run_until_parked();
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    let unsaved = s.ws.read_with(cx, |w, cx| w.unsaved(None, cx));
    let targets: Vec<_> = unsaved.iter().flat_map(|u| u.targets.clone()).collect();
    assert_eq!(
        targets,
        [Target::Request(ip(&item))],
        "the file holds an older edit"
    );
    assert!(!close_window(cx, &s));
    assert!(confirm_open(cx));
}

#[gpui_kit::test]
fn closing_a_collection_prompts_for_an_unkept_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (big, _) = new_draft(cx, &s);
    edit(cx, &s, &big, Edit::Docs("x".repeat(MAX_DRAFT_BYTES)));
    let (ws, scope) = (s.ws.clone(), CloseScope::Collection(s.root.clone()));
    cx.update_window(s.win, |_, w, cx| {
        confirm_close_or_keep(ws, scope, w, cx, |_| {})
    })
    .unwrap();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(s.ws.read_with(cx, |w, _| w.collections().len()), 1);
}

#[gpui_kit::test]
fn closing_and_reopening_the_collection_restores_its_drafts(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/back".into()));
    let kept = Rc::new(Cell::new(false));
    let flag = kept.clone();
    let (ws, scope) = (s.ws.clone(), CloseScope::Collection(s.root.clone()));
    cx.update_window(s.win, |_, w, cx| {
        confirm_close_or_keep(ws, scope, w, cx, move |_| flag.set(true))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!confirm_open(cx) && !kept.get());
    assert!(s.ws.read_with(cx, |w, _| w.collections().is_empty()));
    s.ws.update(cx, |w, cx| w.open(s.root.clone(), cx));
    cx.run_until_parked();
    let back = s.ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().find_map(|e| {
            history_key(w, e.id)
                .filter(|(_, k)| *k == hk)
                .map(|_| e.item().cloned())
        })
    });
    let item = back.flatten().expect("restored with its key");
    assert_eq!(
        ws_snap(cx, &s.ws, &s.root).content.requests[&item].url,
        "http://127.0.0.1:9/back"
    );
}

#[gpui_kit::test]
fn discard_removes_a_listed_draft_written_while_the_prompt_was_open(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let drafts = cx.update(|cx| cx.global::<Drafts>().0.clone());
    let (item, hk) = new_draft(cx, &s);
    // The prompt listed the draft as unkept; its file lands before Discard is clicked, and
    // Discard must still remove it.
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    assert!(
        draft_path(&s, &hk).exists(),
        "written while the prompt was open"
    );
    let listed = vec![(s.root.clone(), ip(&item), hk.clone())];
    drafts.update(cx, |d, cx| d.discard_listed(&listed, cx));
    assert!(!draft_path(&s, &hk).exists(), "a listed draft is discarded");
}

#[gpui_kit::test]
fn a_close_right_after_a_keystroke_keeps_that_keystroke(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    // The last edit is sent but the window closes before the actor has applied it.
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    // The close waits for the actor, flushes what it published, then goes without a prompt.
    close_window(cx, &s);
    cx.run_until_parked();
    assert!(!confirm_open(cx));
    assert!(
        cx.update_window(s.win, |_, _, _| ()).is_err(),
        "the window closed"
    );
    let text = std::fs::read_to_string(draft_path(&s, &hk)).expect("written at close");
    assert!(text.contains("http://127.0.0.1:9/second"), "{text}");
}

#[gpui_kit::test]
fn the_title_bar_close_right_after_a_keystroke_keeps_that_keystroke(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.update(cx, |sh, cx| sh.request_window_close(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!confirm_open(cx));
    assert!(cx.update_window(s.win, |_, _, _| ()).is_err(), "closed");
    let text = std::fs::read_to_string(draft_path(&s, &hk)).expect("written at close");
    assert!(text.contains("http://127.0.0.1:9/second"), "{text}");
}

#[gpui_kit::test]
fn discard_removes_a_listed_draft_that_lands_while_the_prompt_is_open(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    // A broken context keeps the second edit off disk, so the close prompt lists the draft.
    std::fs::create_dir(col.path().join(".env")).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    cx.run_until_parked();
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    assert!(!close_window(cx, &s));
    cx.run_until_parked();
    assert!(confirm_open(cx));
    // The context recovers while the prompt is open, so the draft is written under it.
    std::fs::remove_dir(col.path().join(".env")).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    wait_save(cx);
    let text = std::fs::read_to_string(draft_path(&s, &hk)).unwrap();
    assert!(
        text.contains("/second"),
        "written while the prompt was open: {text}"
    );
    answer_confirm(cx, "confirm-discard");
    assert!(!draft_path(&s, &hk).exists(), "a listed draft is discarded");
}

#[gpui_kit::test]
fn an_actor_that_never_applies_gets_the_prompt_after_the_wait(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::SeqCst;
    let stall = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let col = two_requests();
    let s = started(cx, &col);
    // Reopen the collection under an actor that can be told to stop applying commands.
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
    let (item, hk) = new_draft(cx, &s);
    edit(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    stall.store(true, SeqCst);
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    assert!(!close_window(cx, &s));
    cx.run_until_parked();
    assert!(!confirm_open(cx), "still waiting for the actor");
    cx.executor()
        .advance_clock(apinata_app::closing::SETTLE * 2);
    cx.run_until_parked();
    stall.store(false, SeqCst);
    assert!(confirm_open(cx), "the normal prompt after the wait");
    // The draft file holds the last edit the actor did apply.
    let text = std::fs::read_to_string(draft_path(&s, &hk)).unwrap();
    assert!(text.contains("/first"), "{text}");
}
