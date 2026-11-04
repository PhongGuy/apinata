//! Re-sending a request to save its full response body to a file: the request is planned and
//! run again through core with a body file, apart from the send model, so the shown
//! response and history are untouched. It runs on the unlogged executor, so the session log
//! gains no entry either.

use std::path::{Path, PathBuf};

use apinata_core::{
    CancelHandle, ExecutionMode, ItemPath, Outcome, RunEvent, RunOptions, StreamLimits,
};
use gpui_kit::{Context, Entity, Task};

use crate::command::Command;
use crate::executor;
use crate::prompt_vars::prompt_answers;
use crate::script_gate::{Prepared, prepare};
use crate::send_model::await_plan;
use crate::settings;
use crate::tokio_bridge;
use crate::workspace::Workspace;

/// Where a download stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadState {
    /// Nothing started.
    Idle,
    /// Planning or in flight; `received` counts bytes so far, `total` is the declared size.
    Running {
        /// Bytes received so far.
        received: u64,
        /// The expected size when the server declared one.
        total: Option<u64>,
    },
    /// The file was saved.
    Done {
        /// The final response status.
        status: u16,
        /// The size of the saved file.
        bytes: u64,
    },
    /// No file was saved; the message says why.
    Failed(String),
    /// The user cancelled; no file remains.
    Cancelled,
}

/// What the UI does before downloading a response sent with a given method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPolicy {
    /// A HEAD response has no body to fetch, so no download is offered.
    Hidden,
    /// A safe method: the download starts at the save prompt.
    Direct,
    /// Any other method: the user confirms sending the request again first.
    Confirm,
}

/// The policy for a request method, compared without regard to case.
pub fn download_policy(method: &str) -> DownloadPolicy {
    match method.to_ascii_uppercase().as_str() {
        "HEAD" => DownloadPolicy::Hidden,
        "GET" | "OPTIONS" => DownloadPolicy::Direct,
        _ => DownloadPolicy::Confirm,
    }
}

/// One request's full-body download: at most one at a time, cancellable.
pub struct DownloadModel {
    workspace: Entity<Workspace>,
    state: DownloadState,
    cancel: Option<CancelHandle>,
    task: Option<Task<()>>,
    // The run has started, so it reports its own end.
    in_flight: bool,
    // The save target and whether a file was already there when the download started.
    target: Option<(PathBuf, bool)>,
}

impl DownloadModel {
    /// An idle model planning through the collections open in `workspace`.
    pub fn new(workspace: Entity<Workspace>) -> Self {
        Self {
            workspace,
            state: DownloadState::Idle,
            cancel: None,
            task: None,
            in_flight: false,
            target: None,
        }
    }

    /// The current state.
    pub fn state(&self) -> &DownloadState {
        &self.state
    }

    /// Whether a download is under way.
    pub fn is_running(&self) -> bool {
        matches!(self.state, DownloadState::Running { .. })
    }

    /// A short text of the state, for assertion messages.
    pub fn describe(&self) -> String {
        format!("{:?}", self.state)
    }

    /// Runs `item` of the collection at `root` again, saving its full body to `path`. Returns
    /// `false` while a download is already under way (nothing changes) or when it fails before
    /// starting (the state is then `Failed`).
    pub fn start(
        &mut self,
        root: &Path,
        item: ItemPath,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_running() {
            return false;
        }
        let Some(exec) = executor::unlogged(cx) else {
            self.end(
                DownloadState::Failed("the HTTP client is not ready".into()),
                cx,
            );
            return false;
        };
        let (reply, answer) = async_channel::bounded(1);
        let sent = self
            .workspace
            .update(cx, |w, cx| w.send(root, Command::Plan { item, reply }, cx));
        if sent.is_none() {
            self.end(
                DownloadState::Failed("the collection is not open".into()),
                cx,
            );
            return false;
        }
        let cancel = CancelHandle::new();
        self.cancel = Some(cancel.clone());
        self.in_flight = false;
        self.target = Some((path.clone(), path.exists()));
        self.state = DownloadState::Running {
            received: 0,
            total: None,
        };
        cx.notify();
        let root = root.to_path_buf();
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut planned = match await_plan(answer).await {
                Ok(planned) => planned,
                Err(msg) => {
                    return drop(this.update(cx, |m, cx| m.end(DownloadState::Failed(msg), cx)));
                }
            };
            // A prompt in the request's scripts is asked again; Cancel sends nothing.
            let labels = planned
                .plan
                .requests()
                .next()
                .map(|file| apinata_core::prompt_labels(file, &planned.context))
                .unwrap_or_default();
            if !labels.is_empty() {
                let memory = this
                    .update(cx, |m, cx| m.workspace.read(cx).prompt_memory(&root, cx))
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let Some(answers) = prompt_answers(labels, memory, cx).await else {
                    return drop(this.update(cx, |m, cx| m.end(DownloadState::Cancelled, cx)));
                };
                planned.context = planned.context.with_prompt_values(answers);
            }
            planned.context = cx.update(|cx| settings::with_request_timeout(&planned.context, cx));
            let scripts = match prepare(root, &planned, cx) {
                Prepared::Ready(scripts) => scripts,
                Prepared::Ask(question) => question.ask(cx).await,
            };
            let (tx, rx) = async_channel::unbounded::<RunEvent>();
            let on_event = Box::new(move |event: RunEvent| drop(tx.try_send(event)));
            let _ = this.update(cx, |m, _| m.in_flight = true);
            let run = cx.update(|cx| {
                let opts = RunOptions::new()
                    .authorize(crate::oauth2_signin::authorize_options(cx))
                    .execution_mode(ExecutionMode::Standalone)
                    .scripts(scripts)
                    .stream(StreamLimits::new())
                    .body_files(move |_, _| Some(path.clone()));
                let run =
                    apinata_core::run(planned.plan, planned.context, exec, opts, cancel, on_event);
                tokio_bridge::spawn(cx, run)
            });
            // Progress arrives per chunk; a queued batch is applied in one model update.
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < 256 {
                    match rx.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                let _ = this.update(cx, |m, cx| {
                    for event in batch {
                        m.apply(event, cx);
                    }
                });
            }
            // A run that dies without a result must not leave the model Running.
            let _ = this.update(cx, |m, cx| {
                if m.is_running() {
                    let state = DownloadState::Failed("the download ended unexpectedly".into());
                    m.end(state, cx);
                }
            });
            drop(run);
        }));
        true
    }

    /// Cancels the download; the run ends as `Cancelled` and leaves no file.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.is_running() {
            return;
        }
        if self.in_flight {
            if let Some(cancel) = &self.cancel {
                cancel.cancel();
            }
        } else {
            // Still planning or asking: there is no run to report, so end it here.
            self.task = None;
            self.end(DownloadState::Cancelled, cx);
        }
    }

    fn apply(&mut self, event: RunEvent, cx: &mut Context<Self>) {
        match event {
            RunEvent::ResponseProgress {
                received, total, ..
            } if self.is_running() => {
                self.state = DownloadState::Running { received, total };
                cx.notify();
            }
            RunEvent::RequestFinished { result, .. } => {
                if matches!(result.outcome, Outcome::Cancelled | Outcome::Skipped) {
                    self.discard_after_cancel(result.response.as_ref().and_then(|r| r.body_file()));
                }
                let state = match (result.outcome, &result.response, &result.error) {
                    (Outcome::Cancelled | Outcome::Skipped, _, _) => DownloadState::Cancelled,
                    (_, Some(res), None) if res.body_file().is_some() => DownloadState::Done {
                        status: res.status(),
                        bytes: res.size(),
                    },
                    (_, _, Some(e)) => DownloadState::Failed(e.to_string()),
                    _ => DownloadState::Failed("the response was not saved".into()),
                };
                self.end(state, cx);
            }
            _ => {}
        }
    }

    // Cancel always wins: a file the run committed before the cancel took effect is removed.
    fn discard_after_cancel(&self, committed: Option<&Path>) {
        let Some((path, existed)) = &self.target else {
            return;
        };
        if should_discard(committed.is_some(), *existed, path.exists()) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn end(&mut self, state: DownloadState, cx: &mut Context<Self>) {
        self.state = state;
        self.cancel = None;
        cx.notify();
    }
}

// A cancelled run leaves no file of its own: one it committed goes, and so does one that was
// not there before the download and is now. A file that predates a transfer that never
// committed is the user's and stays.
fn should_discard(committed: bool, existed_before: bool, exists_now: bool) -> bool {
    exists_now && (committed || !existed_before)
}

#[cfg(test)]
mod tests {
    use super::should_discard;

    #[test]
    fn a_committed_file_goes_even_over_an_older_one() {
        assert!(should_discard(true, true, true));
        assert!(should_discard(true, false, true));
    }

    #[test]
    fn a_new_file_goes_and_an_older_untouched_one_stays() {
        assert!(should_discard(false, false, true));
        assert!(!should_discard(false, true, true));
        assert!(!should_discard(false, false, false));
    }
}
