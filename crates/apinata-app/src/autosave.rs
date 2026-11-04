//! Opt-in autosave: saves the workspace's dirty files a fixed delay after the last edit.
//!
//! A pass only sends `Command::Save`, so core's hash guard turns any change on disk into a
//! conflict and the changed-on-disk banner; a file already marked changed, removed or broken
//! is skipped, and nothing is retried until the next edit.

use std::time::Duration;

use gpui_kit::{Context, Entity, Subscription, Task};

use crate::command::{Command, Target};
use crate::snapshot::{Content, DiskState};
use crate::tab_area::TabArea;
use crate::workspace::Workspace;

/// How long after the last edit an autosave runs.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(1000);

/// Saves the workspace's dirty files AUTOSAVE_DELAY after the last edit while enabled.
pub struct Autosave {
    workspace: Entity<Workspace>,
    area: Option<Entity<TabArea>>,
    enabled: bool,
    // The edit generation last seen, so only a new edit restarts the countdown.
    seen: u64,
    pending: Option<Task<()>>,
    _observe: Subscription,
}

impl Autosave {
    /// Off until `set_enabled`; `area` supplies the typed scripts a confirmed save approves.
    pub fn new(
        workspace: Entity<Workspace>,
        area: Option<Entity<TabArea>>,
        cx: &mut Context<Self>,
    ) -> Autosave {
        let seen = workspace.read(cx).edit_generation();
        // Saves, reloads and conflicts notify too, but leave the generation alone, so they
        // never re-arm the timer.
        let observe = cx.observe(&workspace, |this, workspace, cx| {
            let generation = workspace.read(cx).edit_generation();
            if generation != this.seen {
                this.seen = generation;
                if this.enabled {
                    this.schedule(cx);
                }
            }
        });
        Autosave {
            workspace,
            area,
            enabled: false,
            seen,
            pending: None,
            _observe: observe,
        }
    }

    /// On schedules a pass when anything is dirty; off drops the pending pass; the current
    /// value again does nothing.
    pub fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        if !on {
            self.pending = None;
        } else if !self.workspace.read(cx).unsaved(None, cx).is_empty() {
            self.schedule(cx);
        }
    }

    /// Whether autosave is on.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    // Replacing the task cancels the previous countdown, so a burst of edits costs one pass.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(AUTOSAVE_DELAY);
        self.pending = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.pass(cx)).ok();
        }));
    }

    // Saves every dirty target that still matches its file, as a manual save would; a draft
    // has no file, and a target whose disk differs waits for the user to resolve it.
    fn pass(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        let mut saves = Vec::new();
        let workspace = self.workspace.read(cx);
        for u in workspace.unsaved(None, cx) {
            let Some(snapshot) = workspace.snapshot(&u.root, cx) else {
                continue;
            };
            for target in u.targets {
                let draft = matches!(&target, Target::Request(item) if item.is_draft());
                if !draft && disk_of(&snapshot.content, &target) == Some(DiskState::Present) {
                    saves.push((u.root.clone(), target));
                }
            }
        }
        // Each save carries the text typed in its own editor now; it is approved only once
        // the write is confirmed.
        for (root, target) in saves {
            let texts = (self.area.as_ref())
                .map(|a| a.read(cx).typed_for(&root, &target, cx))
                .unwrap_or_default();
            self.workspace.update(cx, |w, cx| {
                let id = w.send(&root, Command::Save(target.clone()), cx);
                w.approve_when_saved(&root, id, Some(target), texts);
            });
        }
    }
}

pub(crate) fn disk_of(content: &Content, target: &Target) -> Option<DiskState> {
    match target {
        Target::Request(p) => content.requests.get(p).map(|v| v.disk),
        Target::Folder(p) => content.folders.get(p).map(|v| v.disk),
        Target::Collection => Some(content.collection.disk),
        Target::Environment(name) => (content.environments.iter())
            .find(|e| &e.name == name)
            .map(|e| e.disk),
    }
}
