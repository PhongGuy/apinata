//! The branch picker model: list, fetch and switch the branches of an open collection.
//! Core does the git work; this model orchestrates it off the UI thread and checks that the
//! collection survives the switch.

use std::path::{Path, PathBuf};

use crate::blocking::off_thread;
use crate::command::Command;
use apinata_core::{Branch, CancelFlag, FetchError, GitOptions, SwitchError, Switched, Upstream};
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
}

/// Why the last request did not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The working tree has changes of its own.
    Dirty,
    /// A tab of the collection has edits not yet saved to disk.
    Unsaved,
    /// The switch landed but unsaved work appeared meanwhile, so the collection stays open.
    KeptOpen,
    /// Git refused or failed; its text.
    Failed(String),
    /// The switch landed on a branch that tracks another branch, left as is; the full text.
    OtherUpstream(String),
}

impl Notice {
    /// The text shown to the user.
    pub fn text(&self) -> &str {
        match self {
            Notice::Dirty => "Commit or discard your changes first",
            Notice::Unsaved => "Save or discard your unsaved edits first",
            Notice::KeptOpen => {
                "The branch switched, but the open tabs were not reloaded because of unsaved edits"
            }
            Notice::Failed(text) | Notice::OtherUpstream(text) => text,
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
}

/// The model behind the branch dialog.
pub struct BranchPicker {
    workspace: Entity<Workspace>,
    root: PathBuf,
    branches: Vec<Branch>,
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
            activity: Activity::Listing,
            notice: None,
            cancel: CancelFlag::new(),
            reload_wait: None,
            reload_timer: None,
            listing: 0,
        };
        // Whatever ends the picker, a window torn down mid-switch included, lifts the hold.
        cx.on_release(|this, cx| {
            let root = this.root.clone();
            let _ = this
                .workspace
                .downgrade()
                .update(cx, |w, cx| w.end_switch(&root, cx));
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

    // Lists the branches in the background; an unreadable repository leaves the list empty.
    // Only the newest listing lands, and it ends the listing or fetch that started it, never a
    // switch or a fetch begun after it.
    fn relist(&mut self, cx: &mut Context<Self>) {
        self.listing += 1;
        let id = self.listing;
        let ends = matches!(self.activity, Activity::Listing | Activity::Fetching)
            .then_some(self.activity);
        let (root, options) = (self.root.clone(), self.options(cx));
        let rx = off_thread(move || apinata_core::branches(&root, &options));
        cx.spawn(async move |this, cx| {
            let Ok(listed) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.listing != id {
                    return;
                }
                this.branches = listed.unwrap_or_default();
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
                this.relist(cx);
            });
        })
        .detach();
    }

    /// Stops a running fetch; a cancelled fetch leaves no notice.
    pub fn cancel_fetch(&mut self) {
        self.cancel.cancel();
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
        let rx = off_thread(move || apinata_core::switch_branch(&root, &branch, &options));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| this.switched(name, &picked, result, cx));
        })
        .detach();
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
                    let reload = self
                        .workspace
                        .update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
                    if let Some(id) = reload
                        && !self.reload_acked(id, cx)
                    {
                        // The reload's snapshot has not arrived; finish when it does.
                        let sub = cx.observe(&self.workspace, move |this, _, cx| {
                            if this.reload_acked(id, cx) {
                                this.land(name.clone(), cx);
                            }
                        });
                        self.reload_wait = Some(sub);
                        let timer = cx.background_executor().timer(RELOAD_BOUND);
                        self.reload_timer = Some(cx.spawn(async move |this, cx| {
                            timer.await;
                            let _ = this.update(cx, |this, cx| this.reload_timed_out(cx));
                        }));
                        cx.notify();
                        return;
                    }
                    self.land(name, cx);
                    return;
                } else {
                    self.finish_closed(name, cx);
                    return;
                }
            }
            Err(SwitchError::Dirty) => self.notice = Some(Notice::Dirty),
            Err(e) => self.notice = Some(Notice::Failed(e.to_string())),
        }
        self.release(cx);
    }

    // Whether the collection has applied the reload command `id`, or is no longer there to.
    fn reload_acked(&self, id: u64, cx: &Context<Self>) -> bool {
        let ws = self.workspace.read(cx);
        ws.snapshot(&self.root, cx).is_none_or(|s| s.acked >= id)
    }

    // The reload has landed: end the hold and tell the shell the switch is done.
    fn land(&mut self, name: String, cx: &mut Context<Self>) {
        self.reload_wait = None;
        self.reload_timer = None;
        self.release(cx);
        cx.emit(PickerEvent::Switched { branch: name });
    }

    // The collection never answered the reload: let go rather than hold it read-only forever.
    fn reload_timed_out(&mut self, cx: &mut Context<Self>) {
        if self.reload_wait.take().is_some() {
            self.notice = Some(Notice::Failed(
                "The branch switched, but the open tabs were not reloaded".to_owned(),
            ));
            self.release(cx);
        }
    }

    fn finish_closed(&mut self, name: String, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.workspace.update(cx, |w, cx| w.close(&root, cx));
        self.release(cx);
        let message = closed_message(&name);
        cx.emit(PickerEvent::CollectionClosed {
            branch: name,
            message,
        });
    }

    // Ends the switch: the picker is idle again and the collection takes edits.
    fn release(&mut self, cx: &mut Context<Self>) {
        self.activity = Activity::Idle;
        let root = self.root.clone();
        self.workspace.update(cx, |w, cx| w.end_switch(&root, cx));
        cx.notify();
    }
}

/// Whether the branch just checked out still has a collection at `root`.
pub fn collection_survives(root: &Path) -> bool {
    root.join("opencollection.yml").is_file()
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
