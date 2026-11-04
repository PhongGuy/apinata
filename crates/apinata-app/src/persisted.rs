//! A value owned by a GPUI entity and written to a JSON file shortly after it changes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::{AppContext, Context, EventEmitter, Task};
use serde::Serialize;

use crate::persist::save_json;

/// How long a change waits before it is written, so a burst of edits costs one write.
pub const SAVE_DELAY: Duration = Duration::from_millis(500);

/// Emitted when a write fails; holds the I/O error text for a notification.
#[derive(Debug, Clone)]
pub struct SaveFailed(pub String);

// Set by `discard`: every write compares its generation against it and is skipped.
const DISCARDED: u64 = u64::MAX;

/// A value saved to `path` as JSON, debounced by `SAVE_DELAY` and written off the UI thread.
pub struct Persisted<T> {
    value: T,
    path: PathBuf,
    generation: u64,
    written: Arc<Mutex<u64>>,
    pending: Option<Task<()>>,
    discarded: bool,
}

impl<T: 'static> EventEmitter<SaveFailed> for Persisted<T> {}

impl<T: Serialize + Clone + Send + 'static> Persisted<T> {
    /// Wraps `value`, which is saved to `path` only after its first change.
    pub fn new(value: T, path: PathBuf) -> Self {
        Persisted {
            value,
            path,
            generation: 0,
            written: Arc::new(Mutex::new(0)),
            pending: None,
            discarded: false,
        }
    }

    /// The current value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// The file the value is saved to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Changes the value, notifies observers and schedules a save `SAVE_DELAY` later,
    /// replacing any save still waiting.
    pub fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut T)) {
        f(&mut self.value);
        self.generation += 1;
        if self.discarded {
            cx.notify();
            return;
        }
        let timer = cx.background_executor().timer(SAVE_DELAY);
        // Replacing the task cancels the previous countdown, so a burst costs one write.
        self.pending = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let Ok(write) = this.update(cx, |p, cx| p.write(cx)) else {
                return;
            };
            write.await;
        }));
        cx.notify();
    }

    /// Cancels the waiting save and writes the current value now.
    pub fn flush(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.pending = None;
        self.write(cx)
    }

    /// Cancels a waiting save and removes the file before returning; no later write can
    /// recreate it. The returned task is already complete.
    pub fn discard(&mut self, _cx: &mut Context<Self>) -> Task<()> {
        self.pending = None;
        self.discarded = true;
        // Taking the write lock waits out a running write; the marker then stops a queued one.
        let mut last = self.written.lock().unwrap_or_else(|e| e.into_inner());
        *last = DISCARDED;
        if let Err(e) = std::fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("apinata: could not remove {}: {e}", self.path.display());
        }
        Task::ready(())
    }

    /// Whether the current value is on disk. A fresh value counts as written; a discarded one does not.
    pub fn is_written(&self) -> bool {
        if self.discarded {
            return false;
        }
        let last = *self.written.lock().unwrap_or_else(|e| e.into_inner());
        last != DISCARDED && last >= self.generation
    }

    /// Cancels the waiting save and writes the current value on the calling thread,
    /// unless it is already written or discarded.
    pub fn flush_now(&mut self) -> std::io::Result<()> {
        self.pending = None;
        let mut last = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if self.discarded || *last >= self.generation {
            return Ok(());
        }
        save_json(&self.path, &self.value)?;
        *last = self.generation;
        Ok(())
    }

    fn write(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.discarded {
            return Task::ready(());
        }
        let (value, path, generation, written) = (
            self.value.clone(),
            self.path.clone(),
            self.generation,
            self.written.clone(),
        );
        // Writes serialise on the lock, and one older than the last landed write is skipped.
        let job = cx.background_spawn(async move {
            let mut last = written.lock().unwrap_or_else(|e| e.into_inner());
            if *last >= generation {
                return Ok(());
            }
            save_json(&path, &value).map(|()| *last = generation)
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = job.await {
                let _ = this.update(cx, |_, cx| cx.emit(SaveFailed(e.to_string())));
            }
        })
    }
}
