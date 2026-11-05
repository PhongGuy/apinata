//! The mock servers the status bar toggles: core's mock, one per collection, bound to
//! 127.0.0.1 on a free port and started on the app's tokio runtime so the UI never waits.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use apinata_core::mock::{self, MockHandle, MockOptions};
use gpui_kit::{App, AppContext as _, Task};

use crate::command::MockSource;
use crate::tokio_bridge;

/// The toggle's tooltip. Core builds the routes once, so later edits need a restart.
pub const MOCK_TIP: &str = "Serve this collection's saved examples on 127.0.0.1. \
    Example edits and environment changes take effect after turning it off and on.";

/// Where one collection's mock server stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockState {
    /// Not running.
    Off,
    /// Turned on and still binding.
    Starting,
    /// Answering on this address.
    On(SocketAddr),
}

enum Slot {
    // The id tells a finished start apart from one a later toggle replaced.
    Starting { id: u64, _task: Task<()> },
    On(MockHandle),
}

/// Every collection's server, by absolute root. Dropping a server's handle stops it, so a
/// closed collection or a quitting app takes its servers with it.
#[derive(Default)]
pub struct MockServers {
    slots: HashMap<PathBuf, Slot>,
    next: u64,
    port: u16,
}

impl MockServers {
    /// The state of `root`'s server.
    pub fn state(&self, root: &Path) -> MockState {
        match self.slots.get(root) {
            None => MockState::Off,
            Some(Slot::Starting { .. }) => MockState::Starting,
            Some(Slot::On(handle)) => MockState::On(handle.addr()),
        }
    }

    // Test-only, through `Workspace::set_mock_port`: the port later starts bind.
    pub(crate) fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    /// Records a start for `root` running as `task` and returns its id.
    pub fn begin(&mut self, root: PathBuf, task: impl FnOnce(u64) -> Task<()>) -> u64 {
        self.next += 1;
        let id = self.next;
        self.slots.insert(
            root,
            Slot::Starting {
                id,
                _task: task(id),
            },
        );
        id
    }

    /// Keeps `handle` as `root`'s server when start `id` is still the one waited for; a
    /// replaced start's server is stopped. Returns whether it was kept.
    pub fn finish(&mut self, root: &Path, id: u64, handle: MockHandle) -> bool {
        match self.slots.get(root) {
            Some(Slot::Starting { id: current, .. }) if *current == id => {
                self.slots.insert(root.to_path_buf(), Slot::On(handle));
                true
            }
            _ => false,
        }
    }

    /// Forgets start `id` of `root` that failed; returns whether it was still current.
    pub fn fail(&mut self, root: &Path, id: u64) -> bool {
        let current = matches!(
            self.slots.get(root),
            Some(Slot::Starting { id: current, .. }) if *current == id
        );
        if current {
            self.slots.remove(root);
        }
        current
    }

    /// Stops `root`'s server, or abandons its start. The listener closes once the returned
    /// task finishes; dropping the task still stops the server, just without waiting.
    pub fn stop(&mut self, root: &Path, cx: &App) -> Option<Task<()>> {
        match self.slots.remove(root)? {
            Slot::Starting { .. } => None,
            Slot::On(handle) if tokio_bridge::is_running(cx) => {
                let stop = tokio_bridge::spawn(cx, handle.stop());
                Some(cx.background_spawn(async move {
                    let _ = stop.await;
                }))
            }
            Slot::On(_) => None,
        }
    }

    /// The port a start binds: 0 for a free one unless a test set another.
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Starts core's mock over `source` on 127.0.0.1:`port`, on the tokio runtime.
pub fn start(source: MockSource, port: u16, cx: &App) -> Task<Result<MockHandle, String>> {
    if !tokio_bridge::is_running(cx) {
        return Task::ready(Err("the background runtime is not running".into()));
    }
    let options = MockOptions::new()
        .bind(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .port(port);
    let job = tokio_bridge::spawn(cx, async move {
        mock::start(&source.collection, &source.context, options)
            .await
            .map_err(|e| e.to_string())
    });
    cx.background_spawn(async move { job.await.map_err(|e| e.to_string()).and_then(|r| r) })
}
