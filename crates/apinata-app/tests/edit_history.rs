//! The undo and redo stack of structured request edits.

use apinata_app::command::Edit;
use apinata_app::edit_history::{EditHistory, Typing};
use apinata_core::{Param, ParamKind};

/// A params edit that lists `names`, so a test reads an edit back as its names.
fn params(names: &[&str]) -> Edit {
    Edit::Params(
        names
            .iter()
            .map(|n| Param::new(*n, "", ParamKind::Query))
            .collect(),
    )
}

fn names(edit: &Edit) -> Vec<String> {
    match edit {
        Edit::Params(p) => p.iter().map(|p| p.name.clone()).collect(),
        other => panic!("not params: {other:?}"),
    }
}

/// A validity check that accepts whatever is asked, as when the view still agrees.
fn agrees(_: &Edit) -> bool {
    true
}

fn sent_names(h: &EditHistory) -> Vec<String> {
    names(h.last_sent(apinata_app::command::Field::UrlParams).unwrap())
}

fn typing(row: usize, col: u8, rows: usize) -> Option<Typing> {
    Some(Typing {
        key: (row, col),
        rows,
    })
}

#[test]
fn undo_returns_the_before_value_and_redo_the_after() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.record(params(&["a"]), params(&["a", "b"]), None);
    assert!(h.can_undo() && !h.can_redo());
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["a"]);
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), Vec::<String>::new());
    assert!(!h.can_undo() && h.can_redo());
    assert!(h.undo(Some(&agrees)).is_none());
    assert_eq!(names(&h.redo(Some(&agrees)).unwrap()), ["a"]);
    assert_eq!(names(&h.redo(Some(&agrees)).unwrap()), ["a", "b"]);
    assert!(h.redo(Some(&agrees)).is_none());
}

#[test]
fn a_new_edit_clears_redo() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.undo(Some(&agrees)).unwrap();
    assert!(h.can_redo());
    h.record(params(&[]), params(&["z"]), None);
    assert!(!h.can_redo());
}

#[test]
fn typing_in_one_cell_is_one_step() {
    let mut h = EditHistory::default();
    h.record(params(&["a"]), params(&["ab"]), typing(0, 0, 1));
    h.record(params(&["ab"]), params(&["abc"]), typing(0, 0, 1));
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["a"]);
    assert!(!h.can_undo());
    assert_eq!(names(&h.redo(Some(&agrees)).unwrap()), ["abc"]);
}

#[test]
fn a_different_cell_or_row_count_starts_a_new_step() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), typing(0, 0, 1));
    h.record(params(&["a"]), params(&["ab"]), typing(0, 1, 1));
    h.record(params(&["ab"]), params(&["ab", "c"]), typing(0, 1, 2));
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["ab"]);
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["a"]);
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), Vec::<String>::new());
}

#[test]
fn an_untyped_edit_never_merges() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.record(params(&["a"]), params(&["b"]), None);
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["a"]);
}

#[test]
fn a_redone_step_does_not_absorb_the_next_keystroke() {
    let mut h = EditHistory::default();
    h.record(params(&["a"]), params(&["ab"]), typing(0, 0, 1));
    h.undo(Some(&agrees)).unwrap();
    h.redo(Some(&agrees)).unwrap();
    h.record(params(&["ab"]), params(&["abc"]), typing(0, 0, 1));
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["ab"]);
}

#[test]
fn a_view_that_moved_on_clears_the_history() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    assert!(h.undo(Some(&|_: &Edit| false)).is_none());
    assert!(!h.can_undo() && !h.can_redo());
}

#[test]
fn a_stale_step_retires_only_its_own_field() {
    let mut h = EditHistory::default();
    let headers = |n: &str| Edit::Headers(vec![apinata_core::KeyValue::new(n, "")]);
    h.record(headers("a"), headers("b"), None);
    h.record(params(&[]), params(&["a"]), None);
    assert!(
        h.undo(Some(&|e: &Edit| matches!(e, Edit::Headers(_))))
            .is_none()
    );
    assert!(h.can_undo(), "the headers step stays");
    assert!(h.undo(Some(&|_: &Edit| true)).is_some());
    assert!(!h.can_undo());
}

#[test]
fn the_view_is_asked_about_the_value_the_step_leaves() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    let leaves_a = |e: &Edit| names(e) == ["a"];
    let before = h.undo(Some(&leaves_a)).unwrap();
    assert!(names(&before).is_empty());
    // A redo leaves the empty list, so a view still showing "a" is stale.
    assert!(h.redo(Some(&leaves_a)).is_none());
}

#[test]
fn while_commands_are_in_flight_the_last_sent_value_decides() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    assert_eq!(names(&h.undo(None).unwrap()), Vec::<String>::new());
    assert_eq!(names(&h.redo(None).unwrap()), ["a"]);
    // Another edit of the same field slips in, unrecorded.
    h.note_sent(&params(&["other"]));
    assert!(h.undo(None).is_none());
    assert!(!h.can_undo());
}

#[test]
fn the_last_sent_value_is_kept_per_field() {
    let mut h = EditHistory::default();
    assert!(
        h.last_sent(apinata_app::command::Field::UrlParams)
            .is_none()
    );
    h.record(params(&[]), params(&["a"]), None);
    h.note_sent(&Edit::Docs("d".into()));
    let sent = h.last_sent(apinata_app::command::Field::UrlParams).unwrap();
    assert_eq!(names(sent), ["a"]);
    assert!(h.last_sent(apinata_app::command::Field::Docs).is_some());
    h.clear();
    assert!(
        h.last_sent(apinata_app::command::Field::UrlParams)
            .is_none()
    );
}

#[test]
fn a_refused_new_step_is_removed_and_redo_comes_back() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.undo(Some(&agrees)).unwrap();
    h.record(params(&[]), params(&["z"]), None);
    assert!(!h.can_redo());
    h.bind(Some(7));
    h.refused(7);
    assert!(!h.can_undo());
    assert!(h.can_redo());
}

#[test]
fn a_refused_keystroke_restores_the_step_it_merged_into() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), typing(0, 0, 1));
    h.bind(Some(1));
    h.record(params(&["a"]), params(&["ab"]), typing(0, 0, 1));
    h.bind(Some(2));
    h.refused(2);
    assert_eq!(sent_names(&h), ["a"]);
    assert_eq!(names(&h.undo(None).unwrap()), Vec::<String>::new());
}

#[test]
fn a_refused_undo_puts_the_step_back() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.bind(Some(1));
    h.undo(Some(&agrees)).unwrap();
    h.bind(Some(2));
    assert!(h.can_redo() && !h.can_undo());
    h.refused(2);
    assert!(h.can_undo() && !h.can_redo());
    assert_eq!(names(&h.undo(None).unwrap()), Vec::<String>::new());
}

#[test]
fn a_refused_redo_puts_the_step_back() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.undo(Some(&agrees)).unwrap();
    h.bind(Some(1));
    h.redo(Some(&agrees)).unwrap();
    h.bind(Some(2));
    h.refused(2);
    assert!(h.can_redo() && !h.can_undo());
    assert_eq!(names(&h.redo(None).unwrap()), ["a"]);
}

#[test]
fn a_refused_unrecorded_edit_leaves_the_steps_alone() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.bind(Some(1));
    h.note_sent(&Edit::Url("http://x".into()));
    h.bind(Some(2));
    h.refused(2);
    assert!(h.can_undo());
    assert_eq!(sent_names(&h), ["a"]);
}

#[test]
fn a_command_never_sent_is_reversed_at_once() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.bind(None);
    assert!(!h.can_undo());
}

#[test]
fn a_settled_command_is_no_longer_reversible() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.bind(Some(3));
    h.settle(3);
    h.refused(3);
    assert!(h.can_undo());
}

#[test]
fn a_staged_step_can_be_taken_back_before_it_is_sent() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), None);
    h.bind(Some(1));
    h.undo(None).unwrap();
    h.revert_staged();
    assert!(h.can_undo() && !h.can_redo());
}

#[test]
fn ending_the_typing_run_starts_a_new_step_at_the_next_keystroke() {
    let mut h = EditHistory::default();
    h.record(params(&[]), params(&["a"]), typing(0, 0, 1));
    h.record(params(&["a"]), params(&["ab"]), typing(0, 0, 1));
    h.end_typing();
    h.record(params(&["ab"]), params(&["abc"]), typing(0, 0, 1));
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), ["ab"]);
    assert_eq!(names(&h.undo(Some(&agrees)).unwrap()), Vec::<String>::new());
}
