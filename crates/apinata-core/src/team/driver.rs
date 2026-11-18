//! The team sync loop's driver: one thread owns the machine, reads inputs in order and runs each
//! action's git calls to completion before reading the next, so git never runs twice at once.

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::git::pre_sync_tip;
use crate::{
    CancelFlag, CommitError, ConflictPath, GitOptions, PullRange, RebasePulled, SyncError,
    commit_all, discard_changes, is_dirty, push, rebase_pull, redact_url, remote_tip,
    squash_unpushed, tracking_tip, unpushed,
};

use super::{
    Identity, Role, SyncFailure, TeamAction, TeamAuth, TeamEvent, TeamMachine, TeamState, classify,
    redact_token, team_options,
};

const COMMIT_MESSAGE: &str = "Save changes";
const DEBOUNCE_MIN: Duration = Duration::from_secs(3);
const DEBOUNCE_MAX: Duration = Duration::from_secs(5);
const DEBOUNCE_DEFAULT: Duration = Duration::from_secs(4);
/// How often a pull commits a save that raced it and tries again.
const DIRTY_RETRIES: u32 = 3;

/// Where a team collection lives, who syncs it, and how long a change settles before it commits.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TeamConfig {
    root: PathBuf,
    copies: PathBuf,
    auth: TeamAuth,
    identity: Identity,
    role: Role,
    base: GitOptions,
    debounce: Duration,
}

impl TeamConfig {
    /// The collection at `root` (the repository's top), conflict copies under `copies`, and a
    /// 4 s debounce.
    pub fn new(
        root: PathBuf,
        copies: PathBuf,
        auth: TeamAuth,
        identity: Identity,
        role: Role,
    ) -> TeamConfig {
        TeamConfig {
            root,
            copies,
            auth,
            identity,
            role,
            base: GitOptions::new(),
            debounce: DEBOUNCE_DEFAULT,
        }
    }

    /// The options the team env is laid over.
    pub fn base_options(mut self, base: GitOptions) -> TeamConfig {
        self.base = base;
        self
    }

    /// The quiet period after a change, clamped to 3 to 5 seconds.
    pub fn debounce(mut self, wait: Duration) -> TeamConfig {
        self.debounce = wait.clamp(DEBOUNCE_MIN, DEBOUNCE_MAX);
        self
    }

    /// For tests only: the quiet period without the clamp.
    #[doc(hidden)]
    pub fn debounce_unclamped(mut self, wait: Duration) -> TeamConfig {
        self.debounce = wait;
        self
    }

    /// The quiet period after a change.
    pub fn debounce_time(&self) -> Duration {
        self.debounce
    }

    /// The collection root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder conflict copies are written under.
    pub fn copies(&self) -> &Path {
        &self.copies
    }

    /// The user's role on the workspace.
    pub fn role(&self) -> Role {
        self.role
    }
}

/// What the app shows of the sync loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TeamStatus {
    /// Where the loop stands.
    pub state: TeamState,
    /// Local commits the remote does not have.
    pub unpushed: usize,
    /// The paths the latest pull conflicted on.
    pub conflicts: Vec<ConflictPath>,
}

impl TeamStatus {
    /// A status of `state` with `unpushed` local commits and the latest pull's `conflicts`.
    pub fn new(state: TeamState, unpushed: usize, conflicts: Vec<ConflictPath>) -> TeamStatus {
        TeamStatus {
            state,
            unpushed,
            conflicts,
        }
    }
}

enum Input {
    Event(TeamEvent),
    RemoteMoved(Option<String>),
    Poll,
    SignedIn(TeamAuth, Identity),
    Stop,
}

/// The status plus what `flush` waits on: inputs sent but not yet read, and one being handled.
struct Shared {
    status: TeamStatus,
    queued: usize,
    in_flight: bool,
}

type SharedState = Arc<(Mutex<Shared>, Condvar)>;

fn lock(shared: &SharedState) -> MutexGuard<'_, Shared> {
    shared.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// The pulls whose renamed secrets nobody has answered yet, kept as `<copies>.renames` so they
/// outlive a restart. One lock, shared by the driver and the app side, covers every
/// read-modify-write, so a pull landing during a clear is never lost.
#[derive(Clone)]
struct RangeFile {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl RangeFile {
    fn new(copies: &Path) -> RangeFile {
        let mut name = copies.as_os_str().to_owned();
        name.push(".renames");
        RangeFile {
            path: PathBuf::from(name),
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn hold(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The stored range; a missing or malformed file is none.
    fn read(&self) -> Option<PullRange> {
        let text = fs::read_to_string(&self.path).ok()?;
        let mut lines = text.lines();
        let (from, to) = (lines.next()?.trim(), lines.next()?.trim());
        (!from.is_empty() && !to.is_empty() && lines.next().is_none())
            .then(|| PullRange::new(from, to))
    }

    /// Extends the range to `to`, keeping its start, or starts it at `pre` when there is none.
    /// A failed write is ignored: the orphaned values stay listed elsewhere.
    fn record(&self, pre: Option<String>, to: &str) {
        let _held = self.hold();
        let Some(from) = self.read().map(|r| r.from).or(pre) else {
            return;
        };
        let _ = self.write(&PullRange::new(from, to));
    }

    /// Written to a temporary file and renamed into place, so a crash never leaves one line.
    fn write(&self, range: &PullRange) -> std::io::Result<()> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| std::io::Error::other("the range file needs a parent folder"))?;
        fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        write!(tmp, "{}\n{}\n", range.from, range.to)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.path).map_err(|e| e.error)?;
        Ok(())
    }

    fn pending(&self) -> Option<PullRange> {
        let _held = self.hold();
        self.read()
    }

    fn settle(&self, range: &PullRange) -> bool {
        let _held = self.hold();
        self.read().as_ref() == Some(range) && fs::remove_file(&self.path).is_ok()
    }
}

/// A running team sync loop. Dropping it stops the thread and cancels its git call in flight
/// without waiting for it.
#[non_exhaustive]
pub struct TeamSync {
    tx: Sender<Input>,
    shared: SharedState,
    cancel: CancelFlag,
    ranges: RangeFile,
}

impl TeamSync {
    /// Sets the status to Pulling (read roles too) and only then spawns the driver thread,
    /// whose first input is the start pull; `on_status` runs on the thread after each change.
    pub fn start(config: TeamConfig, on_status: Box<dyn Fn(&TeamStatus) + Send>) -> TeamSync {
        let status = TeamStatus {
            state: TeamState::Pulling,
            unpushed: 0,
            conflicts: Vec::new(),
        };
        let shared: SharedState = Arc::new((
            Mutex::new(Shared {
                status,
                queued: 0,
                in_flight: false,
            }),
            Condvar::new(),
        ));
        let (tx, rx) = mpsc::channel();
        let cancel = CancelFlag::new();
        let ranges = RangeFile::new(&config.copies);
        let sync = TeamSync {
            tx,
            shared: Arc::clone(&shared),
            cancel: cancel.clone(),
            ranges: ranges.clone(),
        };
        // A machine at rest pulls on a remote move, which is how the start pull begins.
        sync.send(Input::Event(TeamEvent::RemoteMoved));
        let opts = team_options(config.base.clone(), &config.auth, &config.identity);
        let driver = Driver {
            machine: TeamMachine::new(config.role),
            opts,
            config,
            shared,
            cancel,
            on_status,
            deadline: None,
            ranges,
        };
        std::thread::spawn(move || driver.run(rx));
        sync
    }

    /// Files in the collection changed.
    pub fn changed(&self) {
        self.event(TeamEvent::Changed);
    }

    /// Commit now and push; true once the machine settled within `wait`.
    pub fn flush(&self, wait: Duration) -> bool {
        self.event(TeamEvent::Flush);
        let until = Instant::now() + wait;
        let mut guard = lock(&self.shared);
        loop {
            if guard.queued == 0 && !guard.in_flight && !busy(&guard.status.state) {
                return true;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            guard = match self.shared.1.wait_timeout(guard, left) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
    }

    /// A notice's sha (`None`: unknown); equal to the tracking tip, it changes nothing.
    pub fn remote_moved(&self, sha: Option<String>) {
        self.send(Input::RemoteMoved(sha));
    }

    /// `remote_tip`: a new tip is `RemoteMoved`; success while Offline is `Reachable`.
    pub fn poll(&self) {
        self.send(Input::Poll);
    }

    /// The server can be reached again.
    pub fn reachable(&self) {
        self.event(TeamEvent::Reachable);
    }

    /// The user asked to try again.
    pub fn retry(&self) {
        self.event(TeamEvent::Retry);
    }

    /// The user discarded the outside edits.
    pub fn discard(&self) {
        self.event(TeamEvent::Discard);
    }

    /// The user signed in again: later git calls carry `auth`, and later commits `identity`.
    pub fn signed_in(&self, auth: TeamAuth, identity: Identity) {
        self.send(Input::SignedIn(auth, identity));
    }

    /// The user signed out.
    pub fn signed_out(&self) {
        self.event(TeamEvent::SignedOut);
    }

    /// The server said the repository is gone.
    pub fn gone(&self) {
        self.event(TeamEvent::Gone);
    }

    /// The user's role on the team changed.
    pub fn set_role(&self, role: Role) {
        self.event(TeamEvent::RoleChanged(role));
    }

    /// The loop's current status.
    pub fn status(&self) -> TeamStatus {
        lock(&self.shared).status.clone()
    }

    /// The range of pulls not yet answered: `<copies>.renames`, two lines `from`, `to`.
    pub fn pending_renames(&self) -> Option<PullRange> {
        self.ranges.pending()
    }

    /// Clears the pending range if it still equals `range`; whether it did.
    pub fn renames_settled(&self, range: &PullRange) -> bool {
        self.ranges.settle(range)
    }

    fn event(&self, event: TeamEvent) {
        self.send(Input::Event(event));
    }

    /// Counts the input before sending it, so `flush` never sees an empty queue too early.
    fn send(&self, input: Input) {
        let mut guard = lock(&self.shared);
        guard.queued += 1;
        if self.tx.send(input).is_err() {
            guard.queued -= 1;
        }
    }
}

impl Drop for TeamSync {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.tx.send(Input::Stop);
    }
}

fn busy(state: &TeamState) -> bool {
    matches!(
        state,
        TeamState::Committing | TeamState::Pushing | TeamState::Pulling | TeamState::Squashing
    )
}

struct Driver {
    machine: TeamMachine,
    config: TeamConfig,
    opts: GitOptions,
    shared: SharedState,
    cancel: CancelFlag,
    on_status: Box<dyn Fn(&TeamStatus) + Send>,
    deadline: Option<Instant>,
    ranges: RangeFile,
}

impl Driver {
    /// Reads inputs until stopped; a quiet period that reaches the debounce deadline becomes
    /// `DebounceElapsed`.
    fn run(mut self, rx: Receiver<Input>) {
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            let input = match self.deadline {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(input) => Some(input),
                    Err(_) => return,
                },
            };
            {
                let mut guard = lock(&self.shared);
                if input.is_some() {
                    guard.queued = guard.queued.saturating_sub(1);
                }
                guard.in_flight = true;
            }
            let event = match input {
                Some(Input::Stop) => return,
                Some(input) => self.input(input),
                None => {
                    self.deadline = None;
                    Some(TeamEvent::DebounceElapsed)
                }
            };
            if let Some(event) = event {
                self.handle(event);
            }
            lock(&self.shared).in_flight = false;
            self.shared.1.notify_all();
        }
    }

    /// Turns an input into the machine event it stands for, if any.
    fn input(&mut self, input: Input) -> Option<TeamEvent> {
        match input {
            Input::Event(event) => Some(event),
            Input::RemoteMoved(Some(sha)) => {
                let known = tracking_tip(&self.config.root, &self.opts).ok().flatten();
                (known.as_deref() != Some(sha.as_str())).then_some(TeamEvent::RemoteMoved)
            }
            Input::RemoteMoved(None) => Some(TeamEvent::RemoteMoved),
            Input::Poll => {
                let tip = remote_tip(&self.config.root, &self.cancel, &self.opts).ok()?;
                if *self.machine.state() == TeamState::Offline {
                    return Some(TeamEvent::Reachable);
                }
                let known = tracking_tip(&self.config.root, &self.opts).ok().flatten();
                (tip != known).then_some(TeamEvent::RemoteMoved)
            }
            Input::SignedIn(auth, identity) => {
                self.opts = team_options(self.config.base.clone(), &auth, &identity);
                self.config.auth = auth;
                self.config.identity = identity;
                Some(TeamEvent::SignedIn)
            }
            Input::Stop => None,
        }
    }

    /// Feeds `event` to the machine and runs the actions it returns, and those their results
    /// return, in order and one at a time.
    fn handle(&mut self, event: TeamEvent) {
        let mut actions: VecDeque<TeamAction> = self.machine.handle(event).into();
        self.publish(false);
        while let Some(action) = actions.pop_front() {
            if self.cancel.is_cancelled() {
                return;
            }
            if let Some(back) = self.act(action) {
                actions.extend(self.machine.handle(back));
            }
            self.publish(true);
        }
    }

    fn act(&mut self, action: TeamAction) -> Option<TeamEvent> {
        let root = self.config.root.clone();
        match action {
            TeamAction::StartDebounce => {
                self.deadline = Some(Instant::now() + self.config.debounce);
                None
            }
            TeamAction::CancelDebounce => {
                self.deadline = None;
                None
            }
            // A commit that found nothing, with nothing unpushed either, has nothing to push.
            TeamAction::Commit => Some(match self.commit() {
                Err(refused) => refused,
                Ok(false) if unpushed(&root, &self.opts).is_ok_and(|n| n == 0) => {
                    TeamEvent::NothingCommitted
                }
                Ok(_) => TeamEvent::Committed,
            }),
            TeamAction::Push => Some(match push(&root, None, &self.cancel, &self.opts) {
                Ok(_) => TeamEvent::PushAccepted,
                Err(e) => TeamEvent::PushFailed(classify(&e)),
            }),
            TeamAction::Pull | TeamAction::Squash => Some(self.pull(action == TeamAction::Pull)),
            TeamAction::CheckDirty => Some(TeamEvent::Dirty(
                is_dirty(&root, &self.opts).unwrap_or(false),
            )),
            TeamAction::DiscardChanges => discard_changes(&root, &self.opts).err().map(|e| {
                TeamEvent::PullFailed(SyncFailure::Other {
                    message: redact_token(&redact_url(&e.to_string())),
                })
            }),
        }
    }

    /// Commits the whole tree: whether a commit was made, or `CommitRefused` when git refused.
    fn commit(&self) -> Result<bool, TeamEvent> {
        match commit_all(&self.config.root, COMMIT_MESSAGE, &self.opts) {
            Ok(made) => Ok(made.is_some()),
            Err(CommitError::Failed { output }) => Err(TeamEvent::CommitRefused { output }),
            Err(e) => Err(TeamEvent::CommitRefused {
                output: e.to_string(),
            }),
        }
    }

    /// A pull (`rebase`) or a squash. A write role flushes first, so a pull never meets edits
    /// still in their debounce; a save landing between that commit and the pull's clean-tree
    /// check is committed and the pull tried again, a few times before it counts as a failure.
    fn pull(&mut self, rebase: bool) -> TeamEvent {
        let writes = self.machine.role() != Role::Read;
        let (root, copies) = (&self.config.root, &self.config.copies);
        let mut tries = 0;
        loop {
            if writes && let Err(refused) = self.commit() {
                return refused;
            }
            let result = if rebase {
                rebase_pull(root, copies, &self.cancel, &self.opts)
            } else {
                squash_unpushed(root, COMMIT_MESSAGE, copies, &self.cancel, &self.opts)
            };
            match result {
                Err(SyncError::Dirty) if writes && tries < DIRTY_RETRIES => tries += 1,
                Err(SyncError::Dirty) if writes => {
                    return TeamEvent::PullFailed(SyncFailure::Other {
                        message: "Files kept changing during the sync.".into(),
                    });
                }
                result => return self.pulled(result),
            }
        }
    }

    /// The event a pull or squash result stands for; it also records the pull's conflicts, and
    /// widens the pending rename range when HEAD moved.
    fn pulled(&mut self, result: Result<RebasePulled, SyncError>) -> TeamEvent {
        if let Ok(
            RebasePulled::FastForwarded { to }
            | RebasePulled::Rebased { to, .. }
            | RebasePulled::Conflicted { to, .. },
        ) = &result
        {
            // Git runs before the range lock is taken, never under it.
            let pre = pre_sync_tip(&self.config.root, &self.opts).ok().flatten();
            self.ranges.record(pre, to);
        }
        let (paths, conflicted) = match result {
            Ok(RebasePulled::Conflicted { paths, .. }) => (paths, true),
            Ok(_) => (Vec::new(), false),
            Err(SyncError::Dirty) => return TeamEvent::Dirty(true),
            Err(e) => return TeamEvent::PullFailed(classify(&e)),
        };
        lock(&self.shared).status.conflicts = paths;
        TeamEvent::Pulled {
            unpushed: unpushed(&self.config.root, &self.opts).unwrap_or(0),
            conflicted,
        }
    }

    /// Refreshes the status from the machine, and after an action the unpushed count, and
    /// reports it when it changed. The count is read before locking, so `status` never waits.
    fn publish(&self, after_action: bool) {
        let count = after_action
            .then(|| unpushed(&self.config.root, &self.opts).ok())
            .flatten();
        let fresh = {
            let mut guard = lock(&self.shared);
            let before = guard.status.clone();
            guard.status.state = self.machine.state().clone();
            if let Some(count) = count {
                guard.status.unpushed = count;
            }
            (guard.status != before).then(|| guard.status.clone())
        };
        if let Some(status) = fresh {
            (self.on_status)(&status);
        }
        self.shared.1.notify_all();
    }
}
