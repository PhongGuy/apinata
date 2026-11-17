//! The team sync machine's transitions, one scenario per test.

use apinata_core::team::{
    Block, Role, SyncFailure, TeamAction as A, TeamEvent as E, TeamMachine, TeamState as S,
};

/// The machine after `events`, and the actions of the last one.
fn run(role: Role, events: Vec<E>) -> (TeamMachine, Vec<A>) {
    let mut m = TeamMachine::new(role);
    let mut last = Vec::new();
    for event in events {
        last = m.handle(event);
    }
    (m, last)
}

fn pulled(unpushed: usize) -> E {
    E::Pulled {
        unpushed,
        conflicted: false,
    }
}

/// A save that has been committed and is now pushing.
fn pushing() -> Vec<E> {
    vec![E::Changed, E::DebounceElapsed, E::Committed]
}

fn secret() -> SyncFailure {
    SyncFailure::Refused {
        findings: Vec::new(),
        more: 0,
    }
}

#[test]
fn the_role_picks_the_first_state() {
    assert_eq!(TeamMachine::new(Role::Write).state(), &S::Idle);
    assert_eq!(TeamMachine::new(Role::Admin).state(), &S::Idle);
    assert_eq!(
        TeamMachine::new(Role::Read).state(),
        &S::ReadOnly { paused: false }
    );
}

#[test]
fn a_change_debounces_then_commits_and_pushes() {
    let (mut m, last) = run(Role::Write, vec![E::Changed]);
    assert_eq!((m.state(), last), (&S::Idle, vec![A::StartDebounce]));
    assert_eq!(m.handle(E::DebounceElapsed), [A::Commit]);
    assert_eq!(m.state(), &S::Committing);
    assert_eq!(m.handle(E::Committed), [A::Push]);
    assert_eq!(m.handle(E::PushAccepted), Vec::<A>::new());
    assert_eq!(m.state(), &S::Idle);
}

#[test]
fn a_flush_commits_at_once() {
    let (m, last) = run(Role::Write, vec![E::Changed, E::Flush]);
    assert_eq!(
        (m.state(), last),
        (&S::Committing, vec![A::CancelDebounce, A::Commit])
    );
}

#[test]
fn changes_while_busy_wait_for_the_push_to_land() {
    let mut events = pushing();
    events.push(E::Changed);
    let (mut m, last) = run(Role::Write, events);
    assert_eq!((m.state(), last), (&S::Pushing, vec![]));
    assert_eq!(m.handle(E::PushAccepted), [A::StartDebounce]);
}

#[test]
fn a_remote_move_pulls_and_unpushed_commits_then_push() {
    let (mut m, last) = run(Role::Write, vec![E::RemoteMoved]);
    assert_eq!((m.state(), last), (&S::Pulling, vec![A::Pull]));
    assert_eq!(m.handle(pulled(1)), [A::Push]);
    let (m, _) = run(Role::Write, vec![E::RemoteMoved, pulled(0)]);
    assert_eq!(m.state(), &S::Idle);
}

#[test]
fn a_conflicted_pull_pushes_its_fallback() {
    let (m, last) = run(
        Role::Write,
        vec![
            E::RemoteMoved,
            E::Pulled {
                unpushed: 1,
                conflicted: true,
            },
        ],
    );
    assert_eq!((m.state(), last), (&S::Pushing, vec![A::Push]));
}

#[test]
fn a_remote_move_during_a_commit_pulls_once_the_push_lands() {
    let (mut m, last) = run(
        Role::Write,
        vec![E::Changed, E::DebounceElapsed, E::RemoteMoved],
    );
    assert_eq!((m.state(), last), (&S::Committing, vec![]));
    assert_eq!(m.handle(E::Committed), [A::Push]);
    assert_eq!(m.handle(E::PushAccepted), [A::Pull]);
}

#[test]
fn the_third_non_fast_forward_in_a_row_waits_for_a_notice() {
    let nff = || E::PushFailed(SyncFailure::NonFastForward);
    let mut events = pushing();
    events.extend([nff(), pulled(1), nff(), pulled(1)]);
    let (mut m, _) = run(Role::Write, events);
    assert_eq!(m.handle(nff()), Vec::<A>::new());
    assert_eq!(m.state(), &S::Idle);
    assert_eq!(m.handle(E::RemoteMoved), [A::Pull]);
}

#[test]
fn an_accepted_push_resets_the_non_fast_forward_count() {
    let nff = || E::PushFailed(SyncFailure::NonFastForward);
    let mut events = pushing();
    events.extend([nff(), pulled(1), nff(), pulled(1), E::PushAccepted]);
    events.extend(pushing());
    events.extend([nff(), pulled(1)]);
    let (mut m, _) = run(Role::Write, events);
    assert_eq!(m.handle(nff()), [A::Pull]);
}

#[test]
fn under_the_read_role_a_pull_skips_the_push() {
    let (m, last) = run(Role::Read, vec![E::RemoteMoved, pulled(2)]);
    assert_eq!((m.state(), last), (&S::ReadOnly { paused: false }, vec![]));
}

#[test]
fn an_outside_edit_under_the_read_role_pauses_pulls_until_discarded() {
    let (mut m, last) = run(Role::Read, vec![E::Changed]);
    assert_eq!(last, [A::CheckDirty]);
    assert_eq!(m.handle(E::Dirty(true)), Vec::<A>::new());
    assert_eq!(m.state(), &S::ReadOnly { paused: true });
    assert_eq!(m.handle(E::RemoteMoved), Vec::<A>::new());
    assert_eq!(m.handle(E::Flush), Vec::<A>::new());
    assert_eq!(m.handle(E::Discard), [A::DiscardChanges, A::Pull]);
    assert_eq!(m.state(), &S::Pulling);
}

#[test]
fn a_403_on_push_turns_read_only_and_a_raised_role_pushes_the_kept_commits() {
    let mut events = pushing();
    events.push(E::PushFailed(SyncFailure::Forbidden));
    let (mut m, _) = run(Role::Write, events);
    assert_eq!(
        (m.state(), m.role()),
        (&S::ReadOnly { paused: false }, Role::Read)
    );
    assert_eq!(m.handle(E::RemoteMoved), [A::Pull]);
    assert_eq!(m.handle(pulled(1)), Vec::<A>::new());
    assert_eq!(m.handle(E::RoleChanged(Role::Write)), [A::Pull]);
    assert_eq!(m.handle(pulled(1)), [A::Push]);
}

#[test]
fn a_lowered_role_while_idle_turns_read_only() {
    let (m, _) = run(Role::Write, vec![E::RoleChanged(Role::Read)]);
    assert_eq!(m.state(), &S::ReadOnly { paused: false });
}

#[test]
fn a_secret_refusal_blocks_commits_locally_and_retry_squashes() {
    let mut events = pushing();
    events.push(E::PushFailed(secret()));
    let (mut m, _) = run(Role::Write, events);
    let blocked = S::Blocked(Block::Refused {
        findings: Vec::new(),
        more: 0,
    });
    assert_eq!(m.state(), &blocked);
    assert_eq!(m.handle(E::Changed), [A::StartDebounce]);
    assert_eq!(m.handle(E::DebounceElapsed), [A::Commit]);
    assert_eq!(m.handle(E::Committed), Vec::<A>::new());
    assert_eq!(m.state(), &blocked);
    assert_eq!(m.handle(E::RemoteMoved), Vec::<A>::new());
    assert_eq!(m.handle(E::Retry), [A::Squash]);
    assert_eq!(m.state(), &S::Squashing);
    assert_eq!(m.handle(pulled(1)), [A::Push]);
}

#[test]
fn a_401_signs_out_and_signing_in_pulls() {
    let mut events = pushing();
    events.push(E::PushFailed(SyncFailure::Unauthenticated));
    let (mut m, _) = run(Role::Write, events);
    assert_eq!(m.state(), &S::Blocked(Block::SignedOut));
    assert_eq!(m.handle(E::SignedIn), [A::Pull]);
    let (m, _) = run(Role::Write, vec![E::SignedOut]);
    assert_eq!(m.state(), &S::Blocked(Block::SignedOut));
}

#[test]
fn a_404_is_gone_and_retry_does_nothing() {
    let (mut m, _) = run(
        Role::Write,
        vec![E::RemoteMoved, E::PullFailed(SyncFailure::NotFound)],
    );
    assert_eq!(m.state(), &S::Blocked(Block::Gone));
    assert_eq!(m.handle(E::Retry), Vec::<A>::new());
    let (m, _) = run(
        Role::Write,
        vec![E::RemoteMoved, E::PullFailed(SyncFailure::Forbidden)],
    );
    assert_eq!(
        m.state(),
        &S::Blocked(Block::Gone),
        "a read grant always allows a fetch"
    );
}

#[test]
fn a_hook_refusing_the_flush_inside_a_pull_or_squash_blocks_until_retry() {
    let refused = || E::CommitRefused {
        output: "nope".into(),
    };
    let (mut m, _) = run(Role::Write, vec![E::RemoteMoved, refused()]);
    assert_eq!(
        m.state(),
        &S::Blocked(Block::Hook {
            output: "nope".into()
        })
    );
    assert_eq!(m.handle(E::Retry), [A::Commit]);
    assert_eq!(m.state(), &S::Committing);
    let mut events = pushing();
    events.extend([E::PushFailed(secret()), E::Retry, refused()]);
    let (m, _) = run(Role::Write, events);
    assert_eq!(
        m.state(),
        &S::Blocked(Block::Hook {
            output: "nope".into()
        })
    );
}

#[test]
fn a_refused_commit_hook_blocks_until_a_later_commit_passes() {
    let refused = E::CommitRefused {
        output: "nope".into(),
    };
    let (mut m, _) = run(Role::Write, vec![E::Changed, E::DebounceElapsed, refused]);
    assert_eq!(
        m.state(),
        &S::Blocked(Block::Hook {
            output: "nope".into()
        })
    );
    assert_eq!(m.handle(E::Changed), [A::StartDebounce]);
    assert_eq!(m.handle(E::DebounceElapsed), [A::Commit]);
    assert_eq!(m.state(), &S::Committing);
    assert_eq!(m.handle(E::Committed), [A::Push]);
}

#[test]
fn offline_commits_pile_up_and_reaching_the_server_pulls() {
    let down = E::PullFailed(SyncFailure::Offline {
        message: "down".into(),
    });
    let (mut m, _) = run(Role::Write, vec![E::RemoteMoved, down]);
    assert_eq!(m.state(), &S::Offline);
    assert_eq!(m.handle(E::Changed), [A::StartDebounce]);
    assert_eq!(m.handle(E::DebounceElapsed), [A::Commit]);
    assert_eq!(m.handle(E::Committed), Vec::<A>::new());
    assert_eq!(m.state(), &S::Offline);
    assert_eq!(m.handle(E::Reachable), [A::Pull]);
}

#[test]
fn a_flush_while_offline_or_blocked_commits_locally() {
    let down = E::PullFailed(SyncFailure::Offline {
        message: "down".into(),
    });
    let (mut m, _) = run(Role::Write, vec![E::RemoteMoved, down]);
    assert_eq!(m.handle(E::Flush), [A::CancelDebounce, A::Commit]);
    assert_eq!(m.state(), &S::Offline);
    let mut events = pushing();
    events.push(E::PushFailed(secret()));
    let (mut m, _) = run(Role::Write, events);
    assert_eq!(m.handle(E::Flush), [A::CancelDebounce, A::Commit]);
    assert!(matches!(m.state(), S::Blocked(Block::Refused { .. })));
    let (mut m, _) = run(Role::Read, vec![]);
    assert_eq!(m.handle(E::Flush), Vec::<A>::new());
}

#[test]
fn a_read_role_pull_that_finds_a_dirty_tree_pauses() {
    let (m, last) = run(Role::Read, vec![E::RemoteMoved, E::Dirty(true)]);
    assert_eq!((m.state(), last), (&S::ReadOnly { paused: true }, vec![]));
}

#[test]
fn the_gone_notice_blocks_from_any_state() {
    let (m, _) = run(Role::Write, vec![E::Gone]);
    assert_eq!(m.state(), &S::Blocked(Block::Gone));
    let (m, _) = run(Role::Read, vec![E::Gone]);
    assert_eq!(m.state(), &S::Blocked(Block::Gone));
}

#[test]
fn reachable_outside_offline_changes_nothing() {
    let (mut m, _) = run(Role::Write, vec![]);
    assert_eq!(m.handle(E::Reachable), Vec::<A>::new());
    assert_eq!(m.state(), &S::Idle);
}

#[test]
fn any_other_failure_blocks_and_retry_pulls() {
    let other = E::PushFailed(SyncFailure::Other {
        message: "boom".into(),
    });
    let mut events = pushing();
    events.push(other);
    let (mut m, _) = run(Role::Write, events);
    assert_eq!(
        m.state(),
        &S::Blocked(Block::Failed {
            message: "boom".into()
        })
    );
    assert_eq!(m.handle(E::Retry), [A::Pull]);
}

#[test]
fn a_role_lowered_mid_commit_settles_read_only_and_ignores_the_late_result() {
    let lowered = E::RoleChanged(Role::Read);
    let (mut m, _) = run(Role::Write, vec![E::Changed, E::DebounceElapsed, lowered]);
    assert_eq!(m.state(), &S::ReadOnly { paused: false });
    assert_eq!(m.handle(E::Committed), Vec::<A>::new());
    assert_eq!(m.state(), &S::ReadOnly { paused: false });
    assert_eq!(
        m.handle(E::CommitRefused {
            output: "nope".into()
        }),
        Vec::<A>::new()
    );
    assert_eq!(m.state(), &S::ReadOnly { paused: false });
}

#[test]
fn a_role_lowered_while_a_hook_blocks_settles_read_only() {
    let refused = E::CommitRefused {
        output: "nope".into(),
    };
    let events = vec![
        E::Changed,
        E::DebounceElapsed,
        refused,
        E::RoleChanged(Role::Read),
    ];
    let (m, _) = run(Role::Write, events);
    assert_eq!(m.state(), &S::ReadOnly { paused: false });
}

#[test]
fn a_role_lowered_while_a_secret_blocks_settles_read_only() {
    let mut events = pushing();
    events.extend([E::PushFailed(secret()), E::RoleChanged(Role::Read)]);
    let (m, _) = run(Role::Write, events);
    assert_eq!(m.state(), &S::ReadOnly { paused: false });
}

#[test]
fn a_hook_refusing_the_flush_inside_a_pull_pulls_once_the_commit_lands() {
    let (mut m, _) = run(
        Role::Write,
        vec![
            E::RemoteMoved,
            E::CommitRefused {
                output: "nope".into(),
            },
        ],
    );
    assert_eq!(m.handle(E::Retry), [A::Commit]);
    assert_eq!(m.handle(E::Committed), [A::Push]);
    assert_eq!(m.handle(E::PushAccepted), [A::Pull]);
}

#[test]
fn a_flush_while_busy_commits_as_soon_as_the_step_settles() {
    let mut events = pushing();
    events.push(E::Flush);
    let (mut m, last) = run(Role::Write, events);
    assert_eq!((m.state(), last), (&S::Pushing, vec![]));
    assert_eq!(m.handle(E::PushAccepted), [A::CancelDebounce, A::Commit]);
    assert_eq!(m.state(), &S::Committing);
    let (mut m, _) = run(Role::Write, vec![E::RemoteMoved, E::Flush]);
    assert_eq!(m.handle(pulled(0)), [A::CancelDebounce, A::Commit]);
}
