use crate::support;

use apinata_app::command::Target;
use apinata_app::stale::{MAX_RELOADS, Reloaded, StaleLedger, rebase};
use support::ip;

#[test]
fn an_edit_is_stale_only_before_the_snapshot_that_shows_the_reload() {
    let mut l = StaleLedger::new();
    let a = Target::Request(ip("a.yml"));
    l.record(Reloaded::Item(a.clone()), 3);
    assert!(l.is_stale(&a, 2));
    assert!(!l.is_stale(&a, 3));
    assert!(!l.is_stale(&a, 4));
    assert!(!l.is_stale(&Target::Request(ip("b.yml")), 0));
}

#[test]
fn a_directory_reload_marks_everything_under_it() {
    let mut l = StaleLedger::new();
    l.record(Reloaded::Subtree(ip("users")), 5);
    assert!(l.is_stale(&Target::Request(ip("users/a.yml")), 4));
    assert!(l.is_stale(&Target::Request(ip("users/admin/b.yml")), 4));
    assert!(l.is_stale(&Target::Folder(ip("users")), 4));
    assert!(l.is_stale(&Target::Folder(ip("users/admin")), 4));
    assert!(!l.is_stale(&Target::Request(ip("usersx/a.yml")), 4));
    assert!(!l.is_stale(&Target::Request(ip("health.yml")), 4));
    assert!(!l.is_stale(&Target::Collection, 4));
}

#[test]
fn the_latest_reload_wins() {
    let mut l = StaleLedger::new();
    let a = Target::Request(ip("users/a.yml"));
    l.record(Reloaded::Item(a.clone()), 2);
    l.record(Reloaded::Item(a.clone()), 6);
    assert_eq!(l.reloaded_at(&a), Some(6));
    l.record(Reloaded::Subtree(ip("users")), 9);
    assert_eq!(l.reloaded_at(&a), Some(9));
    assert_eq!(l.reloaded_at(&Target::Environment("dev".into())), None);
}

#[test]
fn rebase_moves_a_path_and_everything_under_it() {
    let (from, to) = (ip("users"), ip("admin/users"));
    assert_eq!(rebase(&ip("users"), &from, &to), Some(ip("admin/users")));
    assert_eq!(
        rebase(&ip("users/a.yml"), &from, &to),
        Some(ip("admin/users/a.yml"))
    );
    assert_eq!(rebase(&ip("usersx/a.yml"), &from, &to), None);
}

#[test]
fn commands_older_than_a_move_are_remapped() {
    let mut l = StaleLedger::new();
    l.record_move(
        Target::Folder(ip("users")),
        Target::Folder(ip("admin/users")),
        4,
    );
    assert_eq!(
        l.remap(&Target::Request(ip("users/a.yml")), 3),
        Target::Request(ip("admin/users/a.yml"))
    );
    assert_eq!(
        l.remap(&Target::Folder(ip("users")), 3),
        Target::Folder(ip("admin/users"))
    );
    assert_eq!(
        l.remap(&Target::Request(ip("users/a.yml")), 4),
        Target::Request(ip("users/a.yml"))
    );
}

#[test]
fn chained_moves_compose_and_prune() {
    let mut l = StaleLedger::new();
    l.record_move(
        Target::Request(ip("a.yml")),
        Target::Request(ip("b.yml")),
        2,
    );
    l.record_move(
        Target::Request(ip("b.yml")),
        Target::Request(ip("c.yml")),
        3,
    );
    assert_eq!(
        l.remap(&Target::Request(ip("a.yml")), 1),
        Target::Request(ip("c.yml"))
    );
    assert_eq!(
        l.remap(&Target::Request(ip("b.yml")), 2),
        Target::Request(ip("c.yml"))
    );
    l.record_move(
        Target::Environment("dev".into()),
        Target::Environment("prod".into()),
        5,
    );
    assert_eq!(
        l.remap(&Target::Environment("dev".into()), 4),
        Target::Environment("prod".into())
    );
    l.prune(3);
    assert_eq!(
        l.remap(&Target::Request(ip("a.yml")), 1),
        Target::Request(ip("a.yml"))
    );
    assert_eq!(
        l.remap(&Target::Environment("dev".into()), 4),
        Target::Environment("prod".into())
    );
}

#[test]
fn reload_records_follow_a_move() {
    let mut l = StaleLedger::new();
    l.record(Reloaded::Item(Target::Request(ip("users/a.yml"))), 3);
    l.record_move(
        Target::Folder(ip("users")),
        Target::Folder(ip("admin/users")),
        4,
    );
    let moved = l.remap(&Target::Request(ip("users/a.yml")), 2);
    assert!(l.is_stale(&moved, 2));
}

#[test]
fn pruning_keeps_reload_records_for_edits_built_from_older_snapshots() {
    let mut l = StaleLedger::new();
    let a = Target::Request(ip("a.yml"));
    l.record(Reloaded::Item(a.clone()), 3);
    l.prune(3);
    assert_eq!(l.reloaded_at(&a), Some(3));
    assert!(
        l.is_stale(&a, 2),
        "an edit built from snapshot 2 is still held"
    );
    assert!(!l.is_stale(&a, 3));
}

// Reload records are capped so a long session cannot grow the ledger without bound; an evicted
// record lifts a ledger-wide floor, so every edit it would have held is still held.
fn bounded_ledger() -> StaleLedger {
    let mut l = StaleLedger::new();
    for n in 1..=1000u64 {
        l.record(Reloaded::Item(Target::Request(ip(&format!("r{n}.yml")))), n);
    }
    l
}

#[test]
fn recording_many_targets_keeps_the_ledger_bounded() {
    let l = bounded_ledger();
    assert!(l.len() <= 256, "the ledger holds {} records", l.len());
    assert!(!l.is_empty());
}

#[test]
fn an_evicted_target_is_still_stale_below_its_reload() {
    let l = bounded_ledger();
    let first = Target::Request(ip("r1.yml"));
    assert!(l.is_stale(&first, 0));
    let late = Target::Request(ip("r1000.yml"));
    assert!(l.is_stale(&late, 999));
    assert!(!l.is_stale(&late, 1000));
}

#[test]
fn a_basis_below_the_floor_is_stale_for_any_target() {
    let l = bounded_ledger();
    let never = Target::Request(ip("never-reloaded.yml"));
    assert!(l.is_stale(&never, 1));
    assert!(l.is_stale(&Target::Collection, 1));
    assert!(l.is_stale(&Target::Environment("dev".into()), 1));
}

#[test]
fn a_basis_at_the_floor_is_not_stale() {
    let l = bounded_ledger();
    let never = Target::Request(ip("never-reloaded.yml"));
    // Records 1..=1000 overflow the cap by 1000 - MAX_RELOADS; the last evicted sets the floor.
    let floor = 1000 - MAX_RELOADS as u64;
    assert!(l.is_stale(&never, floor - 1));
    assert!(!l.is_stale(&never, floor));
    assert!(!l.is_stale(&Target::Collection, 1000));
}

#[test]
fn a_move_after_eviction_keeps_the_floor_and_the_records() {
    let mut l = bounded_ledger();
    l.record_move(
        Target::Request(ip("r1000.yml")),
        Target::Request(ip("moved.yml")),
        1001,
    );
    let moved = l.remap(&Target::Request(ip("r1000.yml")), 999);
    assert_eq!(moved, Target::Request(ip("moved.yml")));
    assert!(l.is_stale(&moved, 999));
    assert!(l.is_stale(&Target::Request(ip("r1.yml")), 0));
    assert!(l.is_stale(&Target::Request(ip("never-reloaded.yml")), 1));
    assert!(l.len() <= 256);
}
