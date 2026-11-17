//! The team sync loop as a sans-IO state machine: events in, the git work to run out.

use serde::{Deserialize, Deserializer, Serialize};

use super::{Finding, SyncFailure};

/// What the signed-in user may do on the team server, ordered from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Role {
    /// Fetch only.
    Read,
    /// Fetch and push.
    Write,
    /// Fetch, push and manage the team.
    Admin,
}

impl Role {
    /// The server's role text; anything unknown reads as `Read`, the least privilege.
    pub fn parse(text: &str) -> Role {
        match text {
            "write" => Role::Write,
            "admin" => Role::Admin,
            _ => Role::Read,
        }
    }

    /// The role's name for people.
    pub fn label(self) -> &'static str {
        match self {
            Role::Read => "Read",
            Role::Write => "Write",
            Role::Admin => "Admin",
        }
    }

    /// Whether the role may push.
    pub fn can_write(self) -> bool {
        self >= Role::Write
    }
}

// By hand, since `#[serde(other)]` cannot make an unknown role fall back to the first variant.
impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Role, D::Error> {
        Ok(Role::parse(&String::deserialize(deserializer)?))
    }
}

/// Why syncing stopped until the user acts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Block {
    /// The server's hook refused the pushed commits, for secrets or other reasons.
    Refused {
        /// The findings the hook listed.
        findings: Vec<Finding>,
        /// Findings beyond the listed ones.
        more: usize,
    },
    /// The token was refused; signing in again resumes.
    SignedOut,
    /// The repository no longer exists on the server.
    Gone,
    /// A local commit hook refused the commit.
    Hook {
        /// The hook's output.
        output: String,
    },
    /// Any other failure; Retry pulls.
    Failed {
        /// Git's text.
        message: String,
    },
}

/// Where the sync loop stands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TeamState {
    /// Nothing to do.
    Idle,
    /// The read role: pulls only, paused while the tree has outside edits.
    ReadOnly {
        /// Pulls wait until the outside edits are discarded.
        paused: bool,
    },
    /// Committing the tree.
    Committing,
    /// Pushing the local commits.
    Pushing,
    /// Pulling the remote with a rebase.
    Pulling,
    /// Squashing the unpushed commits after a secret refusal.
    Squashing,
    /// Stopped until the user acts.
    Blocked(Block),
    /// The server is unreachable; commits stay local.
    Offline,
}

/// Something the driver or the app saw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TeamEvent {
    /// Files in the collection changed.
    Changed,
    /// The debounce after a change ran out.
    DebounceElapsed,
    /// Commit now, such as when the window closes.
    Flush,
    /// The server announced a new remote tip.
    RemoteMoved,
    /// The commit landed.
    Committed,
    /// The tree held nothing to commit and nothing waits to be pushed.
    NothingCommitted,
    /// A local hook refused the commit.
    CommitRefused {
        /// The hook's output.
        output: String,
    },
    /// The server accepted the push.
    PushAccepted,
    /// The push failed.
    PushFailed(SyncFailure),
    /// The pull or squash finished, leaving `unpushed` local commits; `conflicted` when the
    /// rebase fell back to a conflict commit.
    Pulled {
        /// Local commits the remote does not have.
        unpushed: usize,
        /// The rebase conflicted and its fallback was committed.
        conflicted: bool,
    },
    /// The pull failed.
    PullFailed(SyncFailure),
    /// Whether the tree has uncommitted edits.
    Dirty(bool),
    /// The user discarded the outside edits.
    Discard,
    /// The user asked to try again.
    Retry,
    /// The user signed in again.
    SignedIn,
    /// The user signed out.
    SignedOut,
    /// The server said the repository is gone.
    Gone,
    /// The server can be reached again.
    Reachable,
    /// The user's role on the team changed.
    RoleChanged(Role),
}

/// Work the driver runs for the machine, in the order given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TeamAction {
    /// Start, or restart, the debounce timer.
    StartDebounce,
    /// Stop the debounce timer.
    CancelDebounce,
    /// Commit the whole tree.
    Commit,
    /// Push the local commits.
    Push,
    /// Pull with a rebase.
    Pull,
    /// Squash the unpushed commits into one.
    Squash,
    /// Report whether the tree is dirty.
    CheckDirty,
    /// Throw away the tree's uncommitted edits.
    DiscardChanges,
}

/// After this many non-fast-forward pushes in a row the loop waits for a notice.
const MAX_REJECTIONS: u8 = 3;

/// The team sync loop's state, fed one event at a time.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TeamMachine {
    state: TeamState,
    role: Role,
    pending_change: bool,
    pending_flush: bool,
    pending_pull: bool,
    rejections: u8,
}

impl TeamMachine {
    /// A machine at rest for `role`: Idle, or ReadOnly under the read role.
    pub fn new(role: Role) -> TeamMachine {
        let state = if role == Role::Read {
            TeamState::ReadOnly { paused: false }
        } else {
            TeamState::Idle
        };
        TeamMachine {
            state,
            role,
            pending_change: false,
            pending_flush: false,
            pending_pull: false,
            rejections: 0,
        }
    }

    /// Where the loop stands.
    pub fn state(&self) -> &TeamState {
        &self.state
    }

    /// The role the machine acts under.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Applies `event` and returns the actions the driver must run, in order.
    pub fn handle(&mut self, event: TeamEvent) -> Vec<TeamAction> {
        use TeamAction as A;
        use TeamEvent as E;
        use TeamState as S;
        let writes = self.role != Role::Read;
        match event {
            E::Gone => self.stop(S::Blocked(Block::Gone)),
            E::SignedOut => self.stop(S::Blocked(Block::SignedOut)),
            // The read role never commits, so a refusal arriving after the role dropped is stale.
            E::CommitRefused { .. } if !writes => Vec::new(),
            // A refused flush inside a pull or squash keeps that pull owed until a commit lands.
            E::CommitRefused { output } => {
                let pull_owed =
                    self.pending_pull || matches!(self.state, S::Pulling | S::Squashing);
                let actions = self.stop(S::Blocked(Block::Hook { output }));
                self.pending_pull = pull_owed;
                actions
            }
            E::RoleChanged(role) => self.role_changed(role),
            // A busy step owns the tree, so work arriving meanwhile waits for it to settle.
            E::Flush if self.busy() => {
                self.pending_flush = writes;
                Vec::new()
            }
            E::Changed | E::DebounceElapsed if self.busy() => {
                self.pending_change = true;
                Vec::new()
            }
            E::RemoteMoved if self.busy() => {
                self.pending_pull = true;
                Vec::new()
            }
            E::Changed => match self.state {
                S::ReadOnly { paused: false } => vec![A::CheckDirty],
                _ if writes => vec![A::StartDebounce],
                _ => Vec::new(),
            },
            E::Flush if writes => {
                if self.state == S::Idle {
                    self.state = S::Committing;
                }
                vec![A::CancelDebounce, A::Commit]
            }
            // Offline and blocked states commit locally and stay put; a hook block retries
            // through Committing so a passing commit clears it.
            E::DebounceElapsed if writes => {
                if matches!(self.state, S::Idle | S::Blocked(Block::Hook { .. })) {
                    self.state = S::Committing;
                }
                vec![A::Commit]
            }
            E::RemoteMoved if matches!(self.state, S::Idle | S::ReadOnly { paused: false }) => {
                self.pull()
            }
            E::Committed
                if writes
                    && matches!(self.state, S::Committing | S::Blocked(Block::Hook { .. })) =>
            {
                self.state = S::Pushing;
                vec![A::Push]
            }
            E::NothingCommitted
                if writes
                    && matches!(self.state, S::Committing | S::Blocked(Block::Hook { .. })) =>
            {
                self.settle()
            }
            E::PushAccepted if self.state == S::Pushing => {
                self.rejections = 0;
                self.settle()
            }
            E::PushFailed(failure) if self.state == S::Pushing => self.failed(failure, true),
            E::Pulled {
                unpushed,
                conflicted,
            } if matches!(self.state, S::Pulling | S::Squashing) => {
                if writes && (unpushed > 0 || conflicted) {
                    self.state = S::Pushing;
                    vec![A::Push]
                } else {
                    self.settle()
                }
            }
            E::PullFailed(failure) if matches!(self.state, S::Pulling | S::Squashing) => {
                self.failed(failure, false)
            }
            E::Dirty(true) if !writes && matches!(self.state, S::Pulling | S::ReadOnly { .. }) => {
                self.state = S::ReadOnly { paused: true };
                Vec::new()
            }
            E::Discard if matches!(self.state, S::ReadOnly { .. }) => {
                let mut actions = vec![A::DiscardChanges];
                actions.extend(self.pull());
                actions
            }
            E::Retry => match self.state {
                S::Blocked(Block::Refused { .. }) if writes => {
                    self.state = S::Squashing;
                    vec![A::Squash]
                }
                S::Blocked(Block::Hook { .. }) if writes => {
                    self.state = S::Committing;
                    vec![A::Commit]
                }
                S::Blocked(Block::Failed { .. }) => self.pull(),
                _ => Vec::new(),
            },
            E::SignedIn if self.state == S::Blocked(Block::SignedOut) => self.pull(),
            E::Reachable if self.state == S::Offline => self.pull(),
            _ => Vec::new(),
        }
    }

    fn busy(&self) -> bool {
        matches!(
            self.state,
            TeamState::Committing | TeamState::Pushing | TeamState::Pulling | TeamState::Squashing
        )
    }

    /// Starts a pull, which under a write role commits the tree first and so covers a flush.
    fn pull(&mut self) -> Vec<TeamAction> {
        self.state = TeamState::Pulling;
        self.pending_pull = false;
        self.pending_flush = false;
        vec![TeamAction::Pull]
    }

    /// Back at rest: a pull that waited runs first, then a flush that waited commits at once,
    /// else a change that waited is picked up.
    fn settle(&mut self) -> Vec<TeamAction> {
        let writes = self.role != Role::Read;
        self.state = if writes {
            TeamState::Idle
        } else {
            TeamState::ReadOnly { paused: false }
        };
        if self.pending_pull {
            return self.pull();
        }
        if std::mem::take(&mut self.pending_flush) && writes {
            self.pending_change = false;
            self.state = TeamState::Committing;
            return vec![TeamAction::CancelDebounce, TeamAction::Commit];
        }
        if std::mem::take(&mut self.pending_change) {
            return vec![if writes {
                TeamAction::StartDebounce
            } else {
                TeamAction::CheckDirty
            }];
        }
        Vec::new()
    }

    /// Enters a state that waits on the user or the network. A pending pull is moot there, but a
    /// pending flush still commits locally at once and a pending change still debounces.
    fn stop(&mut self, state: TeamState) -> Vec<TeamAction> {
        self.state = state;
        self.pending_pull = false;
        let writes = self.role != Role::Read;
        if std::mem::take(&mut self.pending_flush) && writes {
            self.pending_change = false;
            return vec![TeamAction::CancelDebounce, TeamAction::Commit];
        }
        if std::mem::take(&mut self.pending_change) && writes {
            return vec![TeamAction::StartDebounce];
        }
        Vec::new()
    }

    fn failed(&mut self, failure: SyncFailure, push: bool) -> Vec<TeamAction> {
        match failure {
            // The remote moved on: pull and push again, until enough rejections in a row
            // suggest a busy remote, then wait for its next notice.
            SyncFailure::NonFastForward if push => {
                self.rejections += 1;
                if self.rejections >= MAX_REJECTIONS {
                    self.rejections = 0;
                    self.settle()
                } else {
                    self.pull()
                }
            }
            SyncFailure::NonFastForward => self.stop(TeamState::Blocked(Block::Failed {
                message: "The remote moved during the pull.".into(),
            })),
            SyncFailure::Unauthenticated => self.stop(TeamState::Blocked(Block::SignedOut)),
            SyncFailure::Forbidden if push => {
                self.role = Role::Read;
                self.settle()
            }
            // A read grant always allows a fetch, so a refused pull means the repository is gone.
            SyncFailure::Forbidden | SyncFailure::NotFound => {
                self.stop(TeamState::Blocked(Block::Gone))
            }
            SyncFailure::Offline { .. } => self.stop(TeamState::Offline),
            SyncFailure::Refused { findings, more } => {
                self.stop(TeamState::Blocked(Block::Refused { findings, more }))
            }
            SyncFailure::Other { message } => {
                self.stop(TeamState::Blocked(Block::Failed { message }))
            }
        }
    }

    /// A lowered role turns read-only from rest, a commit or a block only a write could clear;
    /// a raised one leaves read-only by pulling, so the commits kept while read-only push.
    fn role_changed(&mut self, role: Role) -> Vec<TeamAction> {
        self.role = role;
        match self.state {
            TeamState::Idle
            | TeamState::Committing
            | TeamState::Blocked(Block::Hook { .. } | Block::Refused { .. })
                if role == Role::Read =>
            {
                self.settle()
            }
            TeamState::ReadOnly { .. } if role != Role::Read => self.pull(),
            _ => Vec::new(),
        }
    }
}
