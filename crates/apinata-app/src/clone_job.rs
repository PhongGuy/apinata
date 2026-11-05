//! A clone running off the UI thread: progress events out, a cancel flag in.

use std::path::PathBuf;

use apinata_core::{CancelFlag, CloneError, CloneProgress, CloneRequest, GitOptions, clone_repo};
use gpui_kit::App;

use crate::blocking::off_thread_events;

/// What a running clone reports; `Finished` is always the last event.
pub enum CloneEvent {
    /// One progress line from git.
    Progress(CloneProgress),
    /// The clone ended: the new folder, or why not.
    Finished(Result<PathBuf, CloneError>),
}

/// A clone in flight; dropping it cancels the clone.
pub struct CloneJob {
    events: async_channel::Receiver<CloneEvent>,
    cancel: CancelFlag,
}

impl CloneJob {
    /// Starts `clone_repo` on its own thread, so a slow git cannot stall the cancel path.
    pub fn start(req: CloneRequest, opts: GitOptions, _cx: &App) -> CloneJob {
        let cancel = CancelFlag::new();
        let flag = cancel.clone();
        let events = off_thread_events(move |tx| {
            let result = clone_repo(&req, &flag, &opts, &mut |p| {
                let _ = tx.try_send(CloneEvent::Progress(p));
            });
            let _ = tx.try_send(CloneEvent::Finished(result));
        });
        CloneJob { events, cancel }
    }

    /// A job fed from `events` by the caller instead of git; the seam for tests that script
    /// the event stream, including a channel that closes without `Finished`.
    pub fn from_events(events: async_channel::Receiver<CloneEvent>) -> CloneJob {
        CloneJob {
            events,
            cancel: CancelFlag::new(),
        }
    }

    /// The event stream, ending with `Finished`.
    pub fn events(&self) -> &async_channel::Receiver<CloneEvent> {
        &self.events
    }

    /// Sets the cancel flag; `Finished(Err(Cancelled))` follows once core has cleaned up.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

// The thread outlives the job, so a closed window must stop git through the flag.
impl Drop for CloneJob {
    fn drop(&mut self) {
        self.cancel();
    }
}
