//! The branch picker model: list, fetch and switch the branches of an open collection, and
//! pull or push the current one. Core does the git work; this model orchestrates it off the UI
//! thread and checks that the collection survives the switch or pull.

use std::path::{Path, PathBuf};

use crate::blocking::{off_thread, off_thread_events};
use crate::command::Command;
use crate::sync_text::{self, Direction};
use apinata_core::{
    Branch, CancelFlag, FetchError, GitOptions, Pulled, Pushed, REPO_LOCK_WAIT, SwitchError,
    Switched, SyncError, Upstream,
};
use gpui_kit::{Context, Entity, EventEmitter, Subscription};

use crate::git_head;
use crate::workspace::Workspace;

/// How long a finished switch waits for the collection to reload before it lets go.
pub const RELOAD_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

/// What the picker is doing; only `Idle` accepts a new request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// Reading the branch list.
    Listing,
    /// Nothing is running.
    Idle,
    /// Fetching from the remotes.
    Fetching,
    /// Switching branch.
    Switching,
    /// Pulling the current branch.
    Pulling,
    /// Pushing the current branch.
    Pushing,
}

/// Why the last request did not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The working tree has changes of its own.
    Dirty,
    /// A tab of the collection has edits not yet saved to disk.
    Unsaved,
    /// Another process, such as an MCP server, holds the repository lock.
    Busy,
    /// The switch landed but unsaved work appeared meanwhile, so the collection stays open.
    KeptOpen,
    /// Git refused or failed; its text.
    Failed(String),
    /// The switch landed on a branch that tracks another branch, left as is; the full text.
    OtherUpstream(String),
    /// The branch has no upstream and the repository one remote, which a publish would use.
    Publish {
        /// The local branch.
        branch: String,
        /// The only remote.
        remote: String,
    },
}

impl Notice {
    /// The text shown to the user.
    pub fn text(&self) -> &str {
        match self {
            Notice::Dirty => "Commit or discard your changes first",
            Notice::Unsaved => "Save or discard your unsaved edits first",
            Notice::Busy => "Another tool is changing this collection; try again",
            Notice::KeptOpen => {
                "The branch switched, but the open tabs were not reloaded because of unsaved edits"
            }
            Notice::Failed(text) | Notice::OtherUpstream(text) => text,
            Notice::Publish { .. } => "This branch is not on the remote yet",
        }
    }
}

/// What the picker tells the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// The branch is checked out and the collection still opens.
    Switched {
        /// The branch name.
        branch: String,
    },
    /// The branch has no collection at its root, so the collection was closed.
    CollectionClosed {
        /// The branch name.
        branch: String,
        /// The notification text.
        message: String,
    },
    /// A pull or push finished; what it did.
    Synced {
        /// The notification text.
        message: String,
    },
}

/// The model behind the branch dialog.
pub struct BranchPicker {
    workspace: Entity<Workspace>,
    root: PathBuf,
    branches: Vec<Branch>,
    // The configured remotes as of the last listing; None until one lands.
    remotes: Option<Vec<String>>,
    current: Option<String>,
    activity: Activity,
    notice: Option<Notice>,
    cancel: CancelFlag,
    // Watches the workspace for the post-switch reload's snapshot while the switch finishes.
    reload_wait: Option<Subscription>,
    // Ends that wait when the collection never answers.
    reload_timer: Option<gpui_kit::Task<()>>,
    // The newest listing's number; an older listing's result is stale and dropped.
    listing: u64,
    // Whether the running pull's fetch is over, after which Cancel no longer stops it.
    fetched: bool,
}

// What a running pull reports from its thread.
enum PullStep {
    Busy,
    Fetched,
    Done(Result<Pulled, SyncError>),
}

// What a finished switch or pull announces once the collection has reloaded.
enum Landing {
    Switched(String),
    Synced(String),
}

impl EventEmitter<PickerEvent> for BranchPicker {}

impl BranchPicker {
    /// A picker over the repository at `root`; it starts listing at once.
    pub fn new(workspace: Entity<Workspace>, root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut picker = BranchPicker {
            workspace,
            current: git_head::branch_label(&root),
            root,
            branches: Vec::new(),
            remotes: None,
            activity: Activity::Listing,
            notice: None,
            cancel: CancelFlag::new(),
            reload_wait: None,
            reload_timer: None,
            listing: 0,
            fetched: false,
        };
        // Whatever ends the picker, a window torn down mid-switch or mid-pull included, lifts
        // the hold; a pull's HEAD is never taken as the app's own.
        cx.on_release(|this, cx| {
            let (root, activity) = (this.root.clone(), this.activity);
            let _ = this
                .workspace
                .downgrade()
                .update(cx, |w, cx| match activity {
                    Activity::Switching => w.end_switch(&root, cx),
                    Activity::Pulling => w.end_pull(&root, true, cx),
                    _ => {}
                });
        })
        .detach();
        picker.relist(cx);
        picker
    }

    /// The repository root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The local branches, then the remote ones no local branch tracks, as of the last listing.
    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    /// Whether the last listing found no remote, so there is nothing to fetch, pull or push.
    /// False until a listing lands, and when the remotes could not be read.
    pub fn lacks_remote(&self) -> bool {
        self.remotes.as_ref().is_some_and(Vec::is_empty)
    }

    /// The checked-out branch, if HEAD is on one.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// What the picker is doing now.
    pub fn activity(&self) -> Activity {
        self.activity
    }

    /// Why the last request failed, if it did.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    fn options(&self, cx: &Context<Self>) -> GitOptions {
        self.workspace
            .read(cx)
            .git_options()
            .cloned()
            .unwrap_or_default()
    }

    // Lists the branches and remotes and rereads HEAD in the background; an unreadable
    // repository leaves the list empty and the remotes unknown. Only the newest listing lands, and it ends the listing or fetch that started
    // it, never a switch or a fetch begun after it.
    fn relist(&mut self, cx: &mut Context<Self>) {
        self.listing += 1;
        let id = self.listing;
        let ends = matches!(self.activity, Activity::Listing | Activity::Fetching)
            .then_some(self.activity);
        let (root, options) = (self.root.clone(), self.options(cx));
        let rx = off_thread(move || {
            let listed = apinata_core::branches(&root, &options);
            let remotes = apinata_core::remotes(&root, &options).ok();
            (listed, remotes, git_head::branch_label(&root))
        });
        cx.spawn(async move |this, cx| {
            let Ok((listed, remotes, current)) = rx.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.listing != id {
                    return;
                }
                this.branches = listed.unwrap_or_default();
                this.remotes = remotes;
                this.current = current;
                if ends == Some(this.activity) {
                    this.activity = Activity::Idle;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetches every remote, then relists. A repository without a remote says so.
    pub fn fetch(&mut self, cx: &mut Context<Self>) {
        if self.activity != Activity::Idle {
            return;
        }
        self.notice = None;
        self.activity = Activity::Fetching;
        // A listing still running from before is superseded by the one this fetch ends with.
        self.listing += 1;
        self.cancel = CancelFlag::new();
        let (root, options, cancel) = (self.root.clone(), self.options(cx), self.cancel.clone());
        let rx = off_thread(move || {
            let remotes = apinata_core::remotes(&root, &options).unwrap_or_default();
            if remotes.is_empty() {
                return Some(Notice::Failed(
                    "This repository has no remote to fetch from".to_owned(),
                ));
            }
            for remote in remotes {
                match apinata_core::fetch(&root, &remote, &cancel, &options) {
                    Ok(()) => {}
                    Err(FetchError::Cancelled) => return None,
                    Err(e) => return Some(Notice::Failed(e.to_string())),
                }
            }
            None
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let notice = rx.recv().await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                this.notice = notice;
                this.sync_ended(cx);
            });
        })
        .detach();
    }

    /// Whether the running fetch, pull or push can still be stopped: a pull only until its
    /// fetch is over, since the fast-forward is never stopped halfway.
    pub fn can_cancel(&self) -> bool {
        match self.activity {
            Activity::Fetching | Activity::Pushing => true,
            Activity::Pulling => !self.fetched,
            _ => false,
        }
    }

    /// Stops a running fetch, pull or push; a cancelled fetch or pull leaves no notice.
    pub fn cancel_fetch(&mut self) {
        self.cancel.cancel();
    }

    // Every fetch, pull or push that ends, however it ends, relists: git may have moved HEAD
    // or the branches meanwhile, from the app or from outside it.
    fn sync_ended(&mut self, cx: &mut Context<Self>) {
        self.relist(cx);
    }

    // A pull or push may start while the list is still loading, since neither reads it.
    fn may_sync(&self) -> bool {
        matches!(self.activity, Activity::Idle | Activity::Listing)
    }

    /// Fetches the upstream of the current branch and fast-forwards to it, holding the
    /// collection read-only meanwhile. Unsaved in-app edits refuse it up front, as for a switch.
    pub fn pull(&mut self, cx: &mut Context<Self>) {
        if !self.may_sync() {
            return;
        }
        self.notice = None;
        let root = self.root.clone();
        if !self.workspace.read(cx).unsaved(Some(&root), cx).is_empty() {
            self.notice = Some(Notice::Unsaved);
            cx.notify();
            return;
        }
        self.activity = Activity::Pulling;
        self.fetched = false;
        self.cancel = CancelFlag::new();
        self.workspace.update(cx, |w, cx| w.begin_pull(&root, cx));
        let (options, cancel) = (self.options(cx), self.cancel.clone());
        let rx = off_thread_events(move |tx| {
            // Held for the whole pull, so another tool's write never lands mid-checkout.
            let Ok(_repo) = apinata_core::repo_lock(&root, REPO_LOCK_WAIT, &options) else {
                let _ = tx.try_send(PullStep::Busy);
                return;
            };
            let done = apinata_core::pull_reporting(&root, &cancel, &options, &mut || {
                let _ = tx.try_send(PullStep::Fetched);
            });
            let _ = tx.try_send(PullStep::Done(done));
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            while let Ok(step) = rx.recv().await {
                let _ = this.update(cx, |this, cx| match step {
                    PullStep::Busy => this.busy(cx),
                    PullStep::Fetched => {
                        this.fetched = true;
                        cx.notify();
                    }
                    PullStep::Done(result) => this.pulled(result, cx),
                });
            }
        })
        .detach();
    }

    // Lands a pull the way a switch lands: a branch that moved reloads the collection before
    // the pull is announced, or closes it when the collection file is gone.
    fn pulled(&mut self, result: Result<Pulled, SyncError>, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.sync_ended(cx);
        match result {
            Ok(done) => {
                let message = sync_text::pulled(&done);
                if done.from == done.to
                    || !self.workspace.read(cx).unsaved(Some(&root), cx).is_empty()
                {
                    self.release(true, cx);
                    cx.emit(PickerEvent::Synced { message });
                } else if collection_survives(&root) {
                    self.reload_then_land(Landing::Synced(message), cx);
                } else {
                    let branch = done.branch;
                    self.close_collection(
                        branch.clone(),
                        format!("Closed the collection: {branch} no longer has a collection at its root."),
                        cx,
                    );
                }
                return;
            }
            Err(SyncError::Cancelled) => {}
            Err(SyncError::Dirty) => self.notice = Some(Notice::Dirty),
            Err(e) => self.notice = Some(Notice::Failed(sync_text::failure(&e, Direction::Pull))),
        }
        self.release(true, cx);
    }

    /// Pushes the current branch to its upstream. A branch without one, in a repository with
    /// exactly one remote, gets a notice that offers to publish it there.
    pub fn push(&mut self, cx: &mut Context<Self>) {
        self.start_push(None, cx);
    }

    /// Publishes the branch the notice names to its only remote and makes that its upstream.
    pub fn publish(&mut self, cx: &mut Context<Self>) {
        if let Some(Notice::Publish { remote, .. }) = &self.notice {
            let remote = remote.clone();
            self.start_push(Some(remote), cx);
        }
    }

    fn start_push(&mut self, set_upstream: Option<String>, cx: &mut Context<Self>) {
        if !self.may_sync() {
            return;
        }
        self.notice = None;
        self.activity = Activity::Pushing;
        self.cancel = CancelFlag::new();
        let (root, options, cancel) = (self.root.clone(), self.options(cx), self.cancel.clone());
        let rx = off_thread(move || {
            let done = apinata_core::push(&root, set_upstream.as_deref(), &cancel, &options);
            // A branch without an upstream is offered a publish, which needs the remotes.
            let remotes = match &done {
                Err(SyncError::NoUpstream { .. }) => {
                    apinata_core::remotes(&root, &options).unwrap_or_default()
                }
                _ => Vec::new(),
            };
            (done, remotes)
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok((done, remotes)) = rx.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.pushed(done, remotes, cx));
        })
        .detach();
    }

    fn pushed(
        &mut self,
        result: Result<Pushed, SyncError>,
        remotes: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.activity = Activity::Idle;
        self.sync_ended(cx);
        match result {
            Ok(done) => {
                cx.notify();
                cx.emit(PickerEvent::Synced {
                    message: sync_text::pushed(&done),
                });
                return;
            }
            Err(SyncError::NoUpstream { branch, .. }) => {
                self.notice = Some(match <[String; 1]>::try_from(remotes) {
                    Ok([remote]) => Notice::Publish { branch, remote },
                    Err(remotes) => Notice::Failed(sync_text::no_upstream(&branch, &remotes)),
                });
            }
            Err(e) => self.notice = Some(Notice::Failed(sync_text::failure(&e, Direction::Push))),
        }
        cx.notify();
    }

    /// Switches to `branch`, then reports whether the collection still opens there.
    pub fn switch(&mut self, branch: Branch, cx: &mut Context<Self>) {
        if self.activity != Activity::Idle {
            return;
        }
        self.notice = None;
        // The switch closes the collection when the branch lacks it, which would drop unsaved
        // in-app edits, so any such edit refuses the switch up front.
        let root = self.root.clone();
        if !self.workspace.read(cx).unsaved(Some(&root), cx).is_empty() {
            self.notice = Some(Notice::Unsaved);
            cx.notify();
            return;
        }
        self.activity = Activity::Switching;
        self.workspace.update(cx, |w, cx| w.begin_switch(&root, cx));
        let (root, options) = (self.root.clone(), self.options(cx));
        // A local row is named by its branch; a remote-only row by the remote branch it shows.
        let remote = branch.upstream.as_ref();
        let name = (branch.local.clone())
            .or_else(|| remote.map(|u| u.name.clone()))
            .unwrap_or_default();
        let picked = remote.map_or_else(String::new, |u| format!("{}/{}", u.remote, u.name));
        // The lock is held for the whole switch, as for a pull.
        let rx = off_thread(move || {
            apinata_core::repo_lock(&root, REPO_LOCK_WAIT, &options)
                .map(|_repo| apinata_core::switch_branch(&root, &branch, &options))
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| match result {
                Ok(result) => this.switched(name, &picked, result, cx),
                Err(_) => this.busy(cx),
            });
        })
        .detach();
    }

    /// Makes branch `name` at HEAD and switches to it. The files stay as they are, uncommitted
    /// changes and unsaved edits included, so the collection needs no reload.
    pub fn create(&mut self, name: String, cx: &mut Context<Self>) {
        if self.activity != Activity::Idle {
            return;
        }
        self.notice = None;
        self.activity = Activity::Switching;
        let (root, options) = (self.root.clone(), self.options(cx));
        self.workspace.update(cx, |w, cx| w.begin_switch(&root, cx));
        let rx = off_thread(move || {
            apinata_core::repo_lock(&root, REPO_LOCK_WAIT, &options)
                .map(|_repo| apinata_core::create_branch(&root, &name, &options))
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| match result {
                Ok(result) => this.created(result, cx),
                Err(_) => this.busy(cx),
            });
        })
        .detach();
    }

    fn created(&mut self, result: Result<Switched, SwitchError>, cx: &mut Context<Self>) {
        let root = self.root.clone();
        match result {
            Ok(done) => {
                self.current = git_head::branch_label(&root);
                self.relist(cx);
                self.workspace.update(cx, |w, cx| w.refresh_git(&root, cx));
                self.release(true, cx);
                cx.emit(PickerEvent::Switched {
                    branch: done.branch,
                });
                return;
            }
            Err(e) => self.notice = Some(Notice::Failed(e.to_string())),
        }
        self.release(true, cx);
    }

    // Lands a switch. Unsaved work that appeared anyway keeps the collection open; a collection
    // that survives reloads its open tabs before the switch ends, so no paint shows the old
    // branch; one that does not is closed.
    fn switched(
        &mut self,
        name: String,
        picked: &str,
        result: Result<Switched, SwitchError>,
        cx: &mut Context<Self>,
    ) {
        let root = self.root.clone();
        match result {
            Ok(done) => {
                // Nothing on the landing path clears it; a weightier notice below replaces it.
                if let Upstream::Other { upstream } = &done.upstream {
                    self.notice = Some(Notice::OtherUpstream(format!(
                        "{} tracks {upstream}, not {picked}; its upstream was left as is.",
                        done.branch
                    )));
                }
                self.current = git_head::branch_label(&root);
                // A dialog that stays open must show the new HEAD and any branch just made.
                self.relist(cx);
                self.workspace.update(cx, |w, cx| w.refresh_git(&root, cx));
                if !self.workspace.read(cx).unsaved(Some(&root), cx).is_empty() {
                    self.notice = Some(Notice::KeptOpen);
                } else if collection_survives(&root) {
                    self.reload_then_land(Landing::Switched(name), cx);
                    return;
                } else {
                    let message = closed_message(&name);
                    self.close_collection(name, message, cx);
                    return;
                }
            }
            Err(SwitchError::Dirty) => self.notice = Some(Notice::Dirty),
            Err(e) => self.notice = Some(Notice::Failed(e.to_string())),
        }
        self.release(true, cx);
    }

    // Ends a pull or switch that never started because another tool held the repository.
    fn busy(&mut self, cx: &mut Context<Self>) {
        self.notice = Some(Notice::Busy);
        self.release(true, cx);
    }

    // Reloads the whole collection, then lands once its snapshot has the reload, so no paint
    // shows the old files.
    fn reload_then_land(&mut self, landing: Landing, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let reload = self
            .workspace
            .update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
        let Some(id) = reload.filter(|id| !self.reload_acked(*id, cx)) else {
            self.land(landing, cx);
            return;
        };
        // The reload's snapshot has not arrived; finish when it does.
        let mut landing = Some(landing);
        let sub = cx.observe(&self.workspace, move |this, _, cx| {
            if this.reload_acked(id, cx)
                && let Some(landing) = landing.take()
            {
                this.land(landing, cx);
            }
        });
        self.reload_wait = Some(sub);
        let timer = cx.background_executor().timer(RELOAD_BOUND);
        self.reload_timer = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| this.reload_timed_out(cx));
        }));
        cx.notify();
    }

    // Whether the collection has applied the reload command `id`, or is no longer there to.
    fn reload_acked(&self, id: u64, cx: &Context<Self>) -> bool {
        let ws = self.workspace.read(cx);
        ws.snapshot(&self.root, cx).is_none_or(|s| s.acked >= id)
    }

    // The reload has landed: end the hold, its watcher reloads already covered, and tell the
    // shell the switch or pull is done.
    fn land(&mut self, landing: Landing, cx: &mut Context<Self>) {
        self.reload_wait = None;
        self.reload_timer = None;
        self.release(false, cx);
        cx.emit(match landing {
            Landing::Switched(branch) => PickerEvent::Switched { branch },
            Landing::Synced(message) => PickerEvent::Synced { message },
        });
    }

    // The collection never answered the reload: let go rather than hold it read-only forever.
    fn reload_timed_out(&mut self, cx: &mut Context<Self>) {
        if self.reload_wait.take().is_some() {
            let done = match self.activity {
                Activity::Pulling => "The pull finished",
                _ => "The branch switched",
            };
            self.notice = Some(Notice::Failed(format!(
                "{done}, but the open tabs were not reloaded"
            )));
            self.release(true, cx);
        }
    }

    fn close_collection(&mut self, branch: String, message: String, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.workspace.update(cx, |w, cx| w.close(&root, cx));
        self.release(true, cx);
        cx.emit(PickerEvent::CollectionClosed { branch, message });
    }

    // Ends the switch or pull: the picker is idle again and the collection takes edits. With
    // `replay` a pull's held watcher reloads run; without it a full reload has covered them.
    fn release(&mut self, replay: bool, cx: &mut Context<Self>) {
        let ended = std::mem::replace(&mut self.activity, Activity::Idle);
        let root = self.root.clone();
        self.workspace.update(cx, |w, cx| match ended {
            Activity::Pulling => w.end_pull(&root, replay, cx),
            _ => w.end_switch(&root, cx),
        });
        cx.notify();
    }
}

/// Whether the branch just checked out still has a collection at `root`.
pub fn collection_survives(root: &Path) -> bool {
    root.join("opencollection.yml").is_file()
}

/// Whether `branch` shows under the filter `query`: its local name or `remote/name` holds the
/// query, ignoring case. An empty query shows every branch.
pub fn matches_filter(branch: &Branch, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let remote = branch
        .upstream
        .as_ref()
        .map(|u| format!("{}/{}", u.remote, u.name));
    branch
        .local
        .iter()
        .chain(remote.iter())
        .any(|name| name.to_lowercase().contains(&query))
}

/// The notification shown when a switch closed the collection.
pub fn closed_message(branch: &str) -> String {
    format!("Closed the collection: branch {branch} has no collection at its root.")
}

// Closing the dialog drops the model; a fetch still running stops with it.
impl Drop for BranchPicker {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
