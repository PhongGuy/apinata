use apinata_app::command::{Notice, NoticeKind, Target};
use apinata_app::sidebar::batch::{
    Batch, Verb, bulk_delete_ask, items_label, permanent_delete_ask,
};
use apinata_app::sidebar::multi::Tally;
use apinata_app::ui::ToastKind;
use apinata_core::ItemPath;
use std::path::PathBuf;

fn ip(p: &str) -> ItemPath {
    ItemPath::new(p).unwrap()
}
fn notice(command: u64, kind: NoticeKind) -> Notice {
    Notice { command, kind }
}
fn failed(command: u64, item: &str, why: &str) -> Notice {
    notice(
        command,
        NoticeKind::Failed {
            target: Some(Target::Request(ip(item))),
            message: why.into(),
            row: None,
        },
    )
}
/// The summary's text, without its kind.
fn text(b: &Batch) -> Option<String> {
    b.summary().map(|(_, text)| text)
}
fn kind(b: &Batch) -> Option<ToastKind> {
    b.summary().map(|(kind, _)| kind)
}
fn root() -> PathBuf {
    PathBuf::from("/c")
}
fn batch(verb: Verb) -> Batch {
    Batch::new(
        verb,
        root(),
        0,
        vec![(5, ip("a.yml")), (6, ip("b.yml")), (7, ip("f/c.yml"))],
    )
}

#[test]
fn a_clean_batch_finishes_without_a_summary() {
    let mut b = batch(Verb::Delete);
    for (id, p) in [(5, "a.yml"), (6, "b.yml"), (7, "f/c.yml")] {
        assert!(b.record(&root(), 0, &notice(id, NoticeKind::Deleted(ip(p)))));
    }
    assert!(b.is_finished());
    assert_eq!(b.summary(), None);
}

#[test]
fn partial_failure_names_each_file_and_reason_in_command_order() {
    let mut b = batch(Verb::Delete);
    b.record(&root(), 0, &notice(5, NoticeKind::Deleted(ip("a.yml"))));
    b.record(&root(), 0, &failed(6, "b.yml", "permission denied"));
    b.record(
        &root(),
        0,
        &notice(7, NoticeKind::NotFound(Target::Request(ip("f/c.yml")))),
    );
    assert!(b.is_finished());
    assert_eq!(
        text(&b).as_deref(),
        Some(
            "Deleted 1 of 3. Could not delete: b.yml (permission denied), c.yml (it no longer exists)"
        )
    );
}

#[test]
fn moves_use_their_own_verb() {
    let mut b = batch(Verb::Move);
    b.record(
        &root(),
        0,
        &notice(
            5,
            NoticeKind::Moved {
                from: ip("a.yml"),
                to: ip("f/a.yml"),
            },
        ),
    );
    b.record(&root(), 0, &failed(6, "b.yml", "exists"));
    b.record(
        &root(),
        0,
        &notice(
            7,
            NoticeKind::Moved {
                from: ip("f/c.yml"),
                to: ip("c.yml"),
            },
        ),
    );
    assert_eq!(
        text(&b).as_deref(),
        Some("Moved 2 of 3. Could not move: b.yml (exists)")
    );
}

#[test]
fn other_roots_untracked_ids_and_unrelated_kinds_are_ignored() {
    let mut b = batch(Verb::Delete);
    assert!(!b.record(
        &PathBuf::from("/other"),
        0,
        &notice(5, NoticeKind::Deleted(ip("a.yml")))
    ));
    // The same root opened again is another instance whose ids start over.
    assert!(!b.record(&root(), 1, &notice(5, NoticeKind::Deleted(ip("a.yml")))));
    assert!(!b.record(&root(), 0, &notice(99, NoticeKind::Deleted(ip("a.yml")))));
    assert!(!b.record(&root(), 0, &notice(5, NoticeKind::Created(ip("a.yml")))));
    assert!(!b.is_finished());
}

#[test]
fn settle_counts_a_silent_command_as_done_once_acknowledged() {
    let mut b = batch(Verb::Move);
    b.record(
        &root(),
        0,
        &notice(
            5,
            NoticeKind::Moved {
                from: ip("a.yml"),
                to: ip("f/a.yml"),
            },
        ),
    );
    b.record(&root(), 0, &failed(6, "b.yml", "exists"));
    b.settle(6);
    assert!(!b.is_finished(), "command 7 is not acknowledged yet");
    b.settle(7);
    assert!(b.is_finished());
    assert_eq!(
        text(&b).as_deref(),
        Some("Moved 2 of 3. Could not move: b.yml (exists)")
    );
}

#[test]
fn counts_and_the_delete_prompt_text() {
    let t = |folders, requests| Tally {
        folders,
        requests,
        others: 0,
    };
    let none = Tally::default();
    assert_eq!(items_label(1), "1 item");
    assert_eq!(items_label(5), "5 items");
    let clean = bulk_delete_ask(t(1, 2), none, false);
    assert_eq!(&*clean.title, "Delete 1 folder and 2 requests?");
    assert_eq!(
        clean.detail.as_deref(),
        Some("Folders are deleted with everything inside them.")
    );
    let dirty = bulk_delete_ask(t(1, 1), none, true);
    assert_eq!(
        dirty.detail.as_deref(),
        Some("Folders are deleted with everything inside them. Unsaved edits are lost.")
    );
    assert_eq!((dirty.default, dirty.cancel), (1, 0));
    // Without a folder in the set, the folder sentence is left out.
    assert_eq!(
        &*bulk_delete_ask(t(0, 2), none, false).title,
        "Delete 2 requests?"
    );
    assert_eq!(bulk_delete_ask(t(0, 2), none, false).detail, None);
    assert_eq!(
        bulk_delete_ask(t(0, 2), none, true).detail.as_deref(),
        Some("Unsaved edits are lost.")
    );
}

// Rows nobody selected that go with their folder are counted out in the prompt.
#[test]
fn the_delete_prompt_counts_rows_inside_folders_that_were_not_selected() {
    let t = |folders, requests, others| Tally {
        folders,
        requests,
        others,
    };
    let detail = |ask: apinata_app::confirm::Ask| ask.detail.map(|d| d.to_string());
    assert_eq!(
        detail(bulk_delete_ask(t(1, 5, 0), t(0, 1, 0), false)).as_deref(),
        Some("1 request in this folder was not selected and will be deleted too.")
    );
    assert_eq!(
        detail(bulk_delete_ask(t(2, 4, 0), t(0, 3, 0), true)).as_deref(),
        Some(
            "3 requests in these folders were not selected and will be deleted too. \
             Unsaved edits are lost."
        )
    );
    assert_eq!(
        detail(permanent_delete_ask(t(1, 0, 0), t(0, 2, 0), false)).as_deref(),
        Some(
            "2 requests in this folder were not selected and will be deleted too. \
             This is permanent."
        )
    );
    assert_eq!(
        detail(bulk_delete_ask(t(1, 1, 0), t(0, 0, 1), false)).as_deref(),
        Some("1 other file in this folder was not selected and will be deleted too.")
    );
    assert_eq!(
        detail(bulk_delete_ask(t(2, 0, 0), t(0, 1, 2), false)).as_deref(),
        Some(
            "1 request and 2 other files in these folders were not selected and will be \
             deleted too."
        )
    );
}

#[test]
fn a_repeated_renumber_warning_is_shown_once_and_does_not_fail_a_move() {
    let mut b = batch(Verb::Move);
    let warn = |id| {
        notice(
            id,
            NoticeKind::Warnings(vec![
                "Moved, but could not renumber the siblings: RO/folder.yml".into(),
            ]),
        )
    };
    let moved = |id, p: &str| {
        notice(
            id,
            NoticeKind::Moved {
                from: ip(p),
                to: ip(&format!("T/{p}")),
            },
        )
    };
    for (id, p) in [(5, "a.yml"), (6, "b.yml"), (7, "f/c.yml")] {
        assert!(b.record(&root(), 0, &warn(id)));
        b.record(&root(), 0, &moved(id, p));
    }
    assert!(b.is_finished());
    assert_eq!(
        text(&b).as_deref(),
        Some("Moved, but could not renumber the siblings: RO/folder.yml")
    );
}

#[test]
fn a_batch_where_nothing_succeeded_is_an_error() {
    for verb in [Verb::Delete, Verb::Move, Verb::Paste] {
        let mut b = batch(verb);
        for (id, p) in [(5, "a.yml"), (6, "b.yml"), (7, "f/c.yml")] {
            b.record(&root(), 0, &failed(id, p, "permission denied"));
        }
        assert!(b.is_finished());
        assert_eq!(kind(&b), Some(ToastKind::Error), "{verb:?}");
    }
}

#[test]
fn a_batch_where_some_succeeded_is_a_warning() {
    let mut b = batch(Verb::Paste);
    b.record(&root(), 0, &notice(5, NoticeKind::Created(ip("a.yml"))));
    b.record(&root(), 0, &failed(6, "b.yml", "permission denied"));
    b.record(&root(), 0, &failed(7, "f/c.yml", "permission denied"));
    assert_eq!(kind(&b), Some(ToastKind::Warning));
    assert!(text(&b).unwrap().starts_with("Pasted 1 of 3."));
}

#[test]
fn a_batch_with_only_side_effect_warnings_is_a_warning() {
    let mut b = batch(Verb::Move);
    b.record(
        &root(),
        0,
        &notice(5, NoticeKind::Warnings(vec!["could not renumber".into()])),
    );
    b.settle(7);
    assert_eq!(kind(&b), Some(ToastKind::Warning));
}

#[test]
fn the_unsaved_notice_counts_only_the_dirty_pastes_that_were_written() {
    let mut b = Batch::new(
        Verb::Paste,
        root(),
        0,
        vec![(5, ip("a.yml")), (6, ip("b.yml"))],
    );
    b.note_unsaved(vec![5, 6], false);
    b.record(&root(), 0, &notice(5, NoticeKind::Created(ip("a.yml"))));
    b.record(&root(), 0, &failed(6, "b.yml", "hidden by the ignore rule"));
    assert!(b.is_finished());
    assert_eq!(
        b.unsaved_notice().as_deref(),
        Some("Pasted the saved version of 1 item; unsaved edits were not copied.")
    );
    assert_eq!(kind(&b), Some(ToastKind::Warning));
}

fn restores(items: &[&str]) -> Batch {
    let sent = (items.iter().enumerate())
        .map(|(i, p)| (10 + i as u64, ip(p)))
        .collect();
    Batch::new(Verb::Restore, root(), 0, sent)
}

#[test]
fn a_clean_restore_names_one_item_or_counts_several() {
    let mut one = restores(&["a.yml"]);
    one.record(&root(), 0, &notice(10, NoticeKind::Restored(ip("a.yml"))));
    assert!(one.is_finished() && one.summary().is_none());
    assert_eq!(one.restored_message().as_deref(), Some("Restored \"a\""));
    let mut two = restores(&["a.yml", "f/c.yml"]);
    two.record(&root(), 0, &notice(10, NoticeKind::Restored(ip("a.yml"))));
    two.record(&root(), 0, &notice(11, NoticeKind::Restored(ip("f/c.yml"))));
    assert!(two.is_finished());
    assert_eq!(two.restored_message().as_deref(), Some("Restored 2 items"));
}

#[test]
fn a_failed_single_restore_reads_like_a_failed_delete() {
    let mut b = restores(&["a.yml"]);
    b.record(&root(), 0, &failed(10, "a.yml", "a.yml already exists"));
    assert!(b.is_finished() && b.restored_message().is_none());
    assert_eq!(kind(&b), Some(ToastKind::Error));
    assert_eq!(
        text(&b).as_deref(),
        Some("Restored 0 of 1. Could not restore: a.yml (a.yml already exists)")
    );
}

#[test]
fn a_partial_restore_is_a_warning_naming_the_failure() {
    let mut b = restores(&["a.yml", "c.yml"]);
    b.record(&root(), 0, &failed(10, "a.yml", "exists"));
    b.record(&root(), 0, &notice(11, NoticeKind::Restored(ip("c.yml"))));
    assert!(b.is_finished() && b.restored_message().is_none());
    assert_eq!(kind(&b), Some(ToastKind::Warning));
    assert_eq!(
        text(&b).as_deref(),
        Some("Restored 1 of 2. Could not restore: a.yml (exists)")
    );
}

#[test]
fn only_a_restore_batch_counts_a_restored_notice() {
    let mut b = batch(Verb::Delete);
    assert!(!b.record(&root(), 0, &notice(5, NoticeKind::Restored(ip("a.yml")))));
    assert!(b.restored_message().is_none());
}

#[test]
fn an_empty_restore_says_nothing() {
    let b = restores(&[]);
    assert!(b.is_finished());
    assert!(b.restored_message().is_none() && b.summary().is_none());
}

#[test]
fn an_unanswered_restore_is_no_longer_in_the_trash() {
    let mut b = restores(&["a.yml", "c.yml"]);
    b.record(&root(), 0, &notice(11, NoticeKind::Restored(ip("c.yml"))));
    b.settle(11);
    assert!(b.is_finished() && b.restored_message().is_none());
    assert_eq!(kind(&b), Some(ToastKind::Warning));
    assert_eq!(
        text(&b).as_deref(),
        Some("Restored 1 of 2. Could not restore: a.yml (it is no longer in the trash)")
    );
}
