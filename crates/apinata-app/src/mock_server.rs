//! The mock servers the status bar toggles: core's mock, one per collection, bound to
//! 127.0.0.1 on the collection's remembered port and started on the app's tokio runtime so
//! the UI never waits. A running server reloads its routes when the saved files change.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::mock::{self, MockHandle, MockOptions};
use gpui_kit::{App, AppContext as _, Task};

use crate::command::MockSource;
use crate::snapshot::{CollectionSnapshot, RequestView};
use crate::tokio_bridge;

/// The toggle's tooltip while the server is off.
pub const MOCK_TIP: &str = "Answer calls on 127.0.0.1 with this collection's saved examples";

/// The toggle's tooltip while the server answers on `port`: how to add examples and stop it.
pub fn running_tip(port: u16) -> String {
    format!(
        "Answering on port {port} with saved examples. Save a response as an example to add \
         one. Click to stop the mock server."
    )
}

/// The status bar's short line for a server answering on `port` from `examples` examples.
pub fn status(examples: usize, port: u16) -> String {
    match examples {
        0 => format!("Mock on port {port}: no saved examples"),
        1 => format!("Mock on port {port}: 1 example"),
        n => format!("Mock on port {port}: {n} examples"),
    }
}

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
    Starting {
        id: u64,
        _task: Task<()>,
    },
    // `reload` is the rebuild in flight; `again` asks for one more once it lands.
    On {
        handle: MockHandle,
        reload: Option<Task<()>>,
        again: bool,
    },
}

/// Every collection's server, by absolute root. Dropping a server's handle stops it, so a
/// closed collection or a quitting app takes its servers with it.
#[derive(Default)]
pub struct MockServers {
    slots: HashMap<PathBuf, Slot>,
    // The snapshot each server's routes were last asked for, to tell when the files moved on.
    seen: HashMap<PathBuf, Arc<CollectionSnapshot>>,
    // A stop still closing its listener, kept here so an abandoned start cannot cancel it; the
    // receiver closes when it is done, and every later start waits for that to reuse the port.
    stopping: HashMap<PathBuf, (Task<()>, async_channel::Receiver<()>)>,
    next: u64,
    port: Option<u16>,
    rebuilds: u64,
}

impl MockServers {
    /// The state of `root`'s server.
    pub fn state(&self, root: &Path) -> MockState {
        match self.slots.get(root) {
            None => MockState::Off,
            Some(Slot::Starting { .. }) => MockState::Starting,
            Some(Slot::On { handle, .. }) => MockState::On(handle.addr()),
        }
    }

    /// How many saved examples `root`'s running server answers from.
    pub fn examples(&self, root: &Path) -> Option<usize> {
        match self.slots.get(root) {
            Some(Slot::On { handle, .. }) => Some(handle.routes().iter().map(|r| r.examples).sum()),
            _ => None,
        }
    }

    // Test-only, through `Workspace::set_mock_port`: the port later starts bind.
    pub(crate) fn set_port(&mut self, port: u16) {
        self.port = Some(port);
    }

    /// Records `snapshot` as what `root`'s routes are built from. Returns whether `root`'s
    /// running server should rebuild them: the saved state differs from the last one seen and
    /// no rebuild is in flight, else one in flight is asked to run again once it lands.
    /// A server still starting keeps the snapshot its start saw, so nothing saved meanwhile
    /// is missed once it runs.
    pub fn observe(&mut self, root: &Path, snapshot: Arc<CollectionSnapshot>) -> bool {
        let Some(Slot::On { reload, again, .. }) = self.slots.get_mut(root) else {
            return false;
        };
        let moved = (self.seen.get(root)).is_none_or(|old| routes_differ(old, &snapshot));
        self.seen.insert(root.to_path_buf(), snapshot);
        if moved && reload.is_some() {
            *again = true;
        }
        moved && reload.is_none()
    }

    /// How many rebuilds have started, across every collection.
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// Keeps `task` as `root`'s rebuild in flight.
    pub fn reloading(&mut self, root: &Path, task: Task<()>) {
        if let Some(Slot::On { reload, again, .. }) = self.slots.get_mut(root) {
            self.rebuilds += 1;
            *reload = Some(task);
            *again = false;
        }
    }

    /// Swaps in the routes of `source` when it arrived, and ends the rebuild. Returns whether
    /// a newer change asked for another one meanwhile.
    pub fn reloaded(&mut self, root: &Path, source: Option<&MockSource>) -> bool {
        let Some(Slot::On {
            handle,
            reload,
            again,
        }) = self.slots.get_mut(root)
        else {
            return false;
        };
        match source {
            Some(source) => handle.reload(&source.collection, &source.context),
            // The routes are still the old ones, so the next snapshot must rebuild them.
            None => {
                self.seen.remove(root);
            }
        }
        *reload = None;
        std::mem::take(again)
    }

    /// Closes once `root`'s last stopped server has released its listener.
    pub fn stopped(&self, root: &Path) -> Option<async_channel::Receiver<()>> {
        self.stopping.get(root).map(|(_, done)| done.clone())
    }

    /// Records a start for `root` running as `task` and returns its id.
    pub fn begin(
        &mut self,
        root: PathBuf,
        seen: Option<Arc<CollectionSnapshot>>,
        task: impl FnOnce(u64) -> Task<()>,
    ) -> u64 {
        match seen {
            Some(seen) => self.seen.insert(root.clone(), seen),
            None => self.seen.remove(&root),
        };
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
                let on = Slot::On {
                    handle,
                    reload: None,
                    again: false,
                };
                self.slots.insert(root.to_path_buf(), on);
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

    /// Stops `root`'s server, or abandons its start. The listener closes in the background;
    /// the next start of `root` waits for that through [`MockServers::stopped`].
    pub fn stop(&mut self, root: &Path, cx: &App) {
        self.seen.remove(root);
        match self.slots.remove(root) {
            Some(Slot::On { handle, .. }) if tokio_bridge::is_running(cx) => {
                let stop = tokio_bridge::spawn(cx, handle.stop());
                let (closed, done) = async_channel::bounded(1);
                let task = cx.background_spawn(async move {
                    let _ = stop.await;
                    drop(closed);
                });
                self.stopping.insert(root.to_path_buf(), (task, done));
            }
            _ => {}
        }
    }

    /// The port a test set for every start, if any.
    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

/// Whether the saved state the routes are built from may differ between two snapshots. A
/// clean request's view is rebuilt only when its file or examples change; once it holds a
/// draft, only example edits and outside changes to the file count. Any save counts, since a
/// later keystroke in the same batch can leave a saved request looking untouched.
pub fn routes_differ(old: &CollectionSnapshot, new: &CollectionSnapshot) -> bool {
    if old.selected_environment != new.selected_environment
        || old.context_generation != new.context_generation
        || old.save_seq != new.save_seq
    {
        return true;
    }
    let (old, new) = (&old.content.requests, &new.content.requests);
    if Arc::ptr_eq(old, new) {
        return false;
    }
    let saved = |r: &&Arc<RequestView>| !r.draft;
    if old.values().filter(saved).count() != new.values().filter(saved).count() {
        return true;
    }
    new.iter()
        .filter(|(_, r)| !r.draft)
        .any(|(path, r)| match old.get(path) {
            Some(was) if Arc::ptr_eq(was, r) => false,
            Some(was) if r.dirty => saved_part(was) != saved_part(r),
            _ => true,
        })
}

// What of a drafted request's view comes from its file: the examples, written straight to
// disk, and the mark of the last outside change.
fn saved_part(r: &RequestView) -> (Vec<&str>, u64) {
    let names = r.examples.iter().map(|e| e.name.as_str()).collect();
    (names, r.reloaded)
}

/// Starts core's mock over `source` on 127.0.0.1:`port`, on the tokio runtime. With
/// `fallback`, a port that cannot be bound gives way to a free one.
pub fn start(
    source: MockSource,
    port: u16,
    fallback: bool,
    cx: &App,
) -> Task<Result<MockHandle, String>> {
    if !tokio_bridge::is_running(cx) {
        return Task::ready(Err("the background runtime is not running".into()));
    }
    let options = |port| {
        MockOptions::new()
            .bind(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .port(port)
    };
    let job = tokio_bridge::spawn(cx, async move {
        let (collection, context) = (&source.collection, &source.context);
        match mock::start(collection, context, options(port)).await {
            Err(mock::MockError::Bind(_)) if fallback && port != 0 => {
                mock::start(collection, context, options(0)).await
            }
            started => started,
        }
        .map_err(|e| e.to_string())
    });
    cx.background_spawn(async move { job.await.map_err(|e| e.to_string()).and_then(|r| r) })
}
