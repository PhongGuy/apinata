//! The MCP server's approval requests, read from the core inbox as a queue of prompts.
//!
//! An inbox file is only ever a request: a decision comes from [`Approvals::decide`], which
//! is the app's one writer of the trust file, or from [`Approvals::approve_scripts`].

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use apinata_core::{
    Approval, ApprovalInbox, Collection, ContextOptions, ItemPath, ScriptApproval, ScriptDecision,
    ScriptText, ScriptTrust, SecretStore, TrustStore, VarContext, normalize_host,
    plan_script_texts_across,
};
use gpui_kit::{AppContext, Context, EventEmitter, Task};
use notify_debouncer_full::DebounceEventResult;

use crate::watch::set::{Watch, WatchError, WatchFactory};

/// Requests older than this are deleted when the model starts.
pub const PRUNE_AFTER: Duration = Duration::from_secs(7 * 86_400);

/// One `(collection, host)` pair waiting for a human answer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Prompt {
    /// The collection's canonical root.
    pub collection: PathBuf,
    /// The host, normalised.
    pub host: String,
    /// The item path the agent reported; untrusted text.
    pub request: String,
    /// The proxy the refused host was reached through, from the pair's first file; untrusted.
    pub proxy: Option<String>,
    /// The host name a host override maps to `host`, from the pair's first file; untrusted.
    pub mapped_from: Option<String>,
    files: Vec<PathBuf>,
}

/// One filed script entry waiting for a human answer, with the texts it names as they are
/// on disk now.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ScriptPrompt {
    /// The collection's canonical root.
    pub collection: PathBuf,
    /// The item path the agent reported, `""` for the whole collection; untrusted text.
    pub request: String,
    /// The texts to approve, each with the first item that needs it.
    pub texts: Vec<(ItemPath, ScriptText)>,
    entry: ScriptApproval,
}

/// A lasting answer to a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Put the host on the collection's trusted list.
    Trust,
    /// Put the host on the collection's denied list.
    Deny,
}

/// What [`Approvals::decide`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The decision was written to the trust file.
    Recorded,
    /// The pair was already on this list, so nothing was written.
    AlreadyDecided(Decision),
}

/// What the approvals model reports to its views.
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalsEvent {
    /// Watching the inbox or the trust file could not start; carries the error for display.
    WatchFailed(String),
}

/// The queue of approval prompts over one inbox directory and one trust file.
pub struct Approvals {
    trust_file: PathBuf,
    script_trust: Option<PathBuf>,
    // The script trust file as last read or written by the app, so views render decisions
    // without touching disk.
    script_cache: Option<ScriptTrust>,
    secrets: Option<Arc<dyn SecretStore>>,
    inbox_dir: PathBuf,
    queue: Vec<Prompt>,
    set_aside: HashSet<(PathBuf, String)>,
    generation: u64,
    script_queue: Vec<ScriptPrompt>,
    // Entry files whose approval failed to save; not shown again this run.
    scripts_set_aside: HashSet<PathBuf>,
    script_generation: u64,
    watch: Option<Box<dyn Watch>>,
    pump: Option<Task<()>>,
    watch_failed: Option<String>,
}

impl EventEmitter<ApprovalsEvent> for Approvals {}

impl Approvals {
    /// Reads `inbox_dir` against `trust_file`, first pruning requests older than
    /// [`PRUNE_AFTER`].
    pub fn new(trust_file: PathBuf, inbox_dir: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Approvals {
            trust_file,
            script_trust: None,
            script_cache: None,
            secrets: None,
            inbox_dir,
            queue: Vec::new(),
            set_aside: HashSet::new(),
            generation: 0,
            script_queue: Vec::new(),
            scripts_set_aside: HashSet::new(),
            script_generation: 0,
            watch: None,
            pump: None,
            watch_failed: None,
        };
        this.reload(true, cx);
        this
    }

    /// The trust file this model writes.
    pub fn trust_file(&self) -> &Path {
        &self.trust_file
    }

    /// Sets the script trust file the app's script decisions go to, and re-reads.
    pub fn set_script_trust(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.script_trust = Some(path);
        self.refresh(cx);
        cx.notify();
    }

    /// Sets the store environment secrets are read from, so script prompts mask them, and
    /// re-reads.
    pub fn set_secrets(&mut self, store: Arc<dyn SecretStore>, cx: &mut Context<Self>) {
        self.secrets = Some(store);
        self.refresh(cx);
    }

    /// The script trust file, once `set_script_trust` named one.
    pub fn script_trust(&self) -> Option<&Path> {
        self.script_trust.as_deref()
    }

    /// `root`'s script decision as last read or recorded; `None` while undecided or unread.
    pub fn script_decision(&self, root: &Path) -> Option<ScriptDecision> {
        self.script_cache.as_ref()?.decision(root)
    }

    /// Records a script decision the app just wrote, so every view shows it at once, and
    /// re-reads the file to pick up anything written elsewhere.
    pub fn note_script_decision(
        &mut self,
        root: &Path,
        decision: Option<ScriptDecision>,
        cx: &mut Context<Self>,
    ) {
        if let Some(cache) = &mut self.script_cache {
            cache.set_decision(root, decision);
        }
        self.refresh(cx);
        cx.notify();
    }

    /// The inbox directory this model reads.
    pub fn inbox_dir(&self) -> &Path {
        &self.inbox_dir
    }

    /// The prompts waiting for an answer, oldest file first.
    pub fn queue(&self) -> &[Prompt] {
        &self.queue
    }

    /// The script entries waiting for an answer, oldest file first; empty until
    /// `set_script_trust` named a trust file.
    pub fn script_queue(&self) -> &[ScriptPrompt] {
        &self.script_queue
    }

    /// Allow an undecided collection, approve the shown hashes, save, delete the entry. A
    /// blocked collection stays blocked. On a store error nothing is written, the entry is
    /// set aside for this run and the error text is returned.
    pub fn approve_scripts(
        &mut self,
        prompt: &ScriptPrompt,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(path) = self.script_trust.clone() else {
            return Err("no script trust file".to_owned());
        };
        // Update under the trust file's lock so decisions made elsewhere stand.
        let saved = ScriptTrust::update(&path, |trust| {
            if trust.decision(&prompt.collection).is_none() {
                trust.set_decision(&prompt.collection, Some(ScriptDecision::Allowed));
            }
            trust.approve(
                &prompt.collection,
                prompt.texts.iter().map(|(_, t)| &t.hash),
            );
        });
        match saved {
            Ok(()) => {
                self.dismiss_scripts(prompt, cx);
                Ok(())
            }
            Err(e) => {
                self.scripts_set_aside.insert(prompt.entry.file.clone());
                self.drop_script_entry(&prompt.entry.file, cx);
                Err(e.to_string())
            }
        }
    }

    /// Delete the entry; nothing is recorded. An entry that cannot be deleted is set aside
    /// for this run, so the next read does not show it again.
    pub fn dismiss_scripts(&mut self, prompt: &ScriptPrompt, cx: &mut Context<Self>) {
        if ApprovalInbox::new(self.inbox_dir.clone())
            .remove_scripts(&prompt.entry)
            .is_err()
        {
            self.scripts_set_aside.insert(prompt.entry.file.clone());
        }
        self.drop_script_entry(&prompt.entry.file, cx);
        self.refresh(cx);
    }

    fn drop_script_entry(&mut self, file: &Path, cx: &mut Context<Self>) {
        let before = self.script_queue.len();
        self.script_queue.retain(|p| p.entry.file != file);
        if self.script_queue.len() != before {
            cx.notify();
        }
    }

    /// Creates and watches the inbox directory and the trust file's directory, refreshing
    /// on every batch; `None` stops.
    pub fn set_watch_factory(&mut self, factory: Option<WatchFactory>, cx: &mut Context<Self>) {
        self.watch = None;
        self.pump = None;
        self.watch_failed = None;
        let Some(factory) = factory else {
            return;
        };
        match self.start_watch(&factory) {
            Ok((watch, batches)) => {
                self.watch = Some(watch);
                self.pump = Some(cx.spawn(async move |this, cx| {
                    while let Ok(trust_changed) = batches.recv().await {
                        let pumped = this.update(cx, |a, cx| {
                            if trust_changed {
                                a.set_aside.clear();
                            }
                            a.refresh(cx);
                        });
                        if pumped.is_err() {
                            break;
                        }
                    }
                }));
                // A request filed before the watch was placed has no event of its own.
                self.refresh(cx);
            }
            Err(message) => {
                self.watch_failed = Some(message.clone());
                cx.emit(ApprovalsEvent::WatchFailed(message));
            }
        }
    }

    /// Why watching could not start, or `None` while watching or not asked to.
    pub fn watch_failed(&self) -> Option<&str> {
        self.watch_failed.as_deref()
    }

    /// The trust file is replaced by rename, so its directory is watched rather than the file;
    /// each batch sends whether it named the trust file, which retries pairs set aside.
    fn start_watch(
        &self,
        factory: &WatchFactory,
    ) -> Result<(Box<dyn Watch>, async_channel::Receiver<bool>), String> {
        let trust_dir = self
            .trust_file
            .parent()
            .ok_or_else(|| format!("{} has no directory", self.trust_file.display()))?
            .to_path_buf();
        for dir in [&self.inbox_dir, &trust_dir] {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let real_trust_file = fs::canonicalize(&trust_dir)
            .map(|d| d.join(self.trust_file.file_name().unwrap_or_default()))
            .unwrap_or_else(|_| self.trust_file.clone());
        let trust_file = self.trust_file.clone();
        let (tx, rx) = async_channel::unbounded();
        let mut watch = factory(Box::new(move |batch: DebounceEventResult| {
            let named = |p: &Path| p == trust_file || p == real_trust_file;
            // A failed or rescanned batch may hide a trust file change, so it counts as one.
            let trust_changed = batch.as_ref().map_or(true, |events| {
                events
                    .iter()
                    .any(|e| e.need_rescan() || e.paths.iter().any(|p| named(p)))
            });
            let _ = tx.try_send(trust_changed);
        }))?;
        for dir in [&self.inbox_dir, &trust_dir] {
            watch.watch(dir).map_err(|e| match e {
                WatchError::Unwatchable => format!("{} cannot be watched", dir.display()),
                WatchError::Failed(message) => message,
            })?;
        }
        Ok((watch, rx))
    }

    /// Re-reads the inbox and the trust file off the UI thread and rebuilds the queue.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload(false, cx);
    }

    /// Records `decision` for the prompt's pair, unless the trust file already lists the pair,
    /// then removes its requests and drops it from the queue. On a store error nothing is
    /// written, the pair is set aside for this run and the error text is returned.
    pub fn decide(
        &mut self,
        prompt: &Prompt,
        decision: Decision,
        cx: &mut Context<Self>,
    ) -> Result<Outcome, String> {
        let (collection, host) = (prompt.collection.as_path(), prompt.host.as_str());
        // Reload right before writing so a decision made elsewhere stands and the window
        // in which a concurrent writer's entry can be lost stays small.
        let recorded = TrustStore::load(&self.trust_file).and_then(|mut store| {
            if store.is_trusted(collection, host) {
                return Ok(Outcome::AlreadyDecided(Decision::Trust));
            }
            if store.is_denied(collection, host) {
                return Ok(Outcome::AlreadyDecided(Decision::Deny));
            }
            match decision {
                Decision::Trust => store.trust(collection, host)?,
                Decision::Deny => store.deny(collection, host)?,
            }
            store.save().map(|()| Outcome::Recorded)
        });
        match recorded {
            Ok(outcome) => {
                self.clear(prompt, cx);
                Ok(outcome)
            }
            Err(e) => {
                self.set_aside
                    .insert((prompt.collection.clone(), prompt.host.clone()));
                self.drop_pair(&prompt.collection, &prompt.host, cx);
                Err(e.to_string())
            }
        }
    }

    /// Removes the pair's inbox files and drops it from the queue; writes no trust.
    pub fn dismiss(&mut self, prompt: &Prompt, cx: &mut Context<Self>) {
        self.clear(prompt, cx);
    }

    /// Deletes the prompt's files by path as well as through core's `resolve`, which matches
    /// raw file text and so misses a hand-written non-canonical host.
    fn clear(&mut self, prompt: &Prompt, cx: &mut Context<Self>) {
        remove_files(&prompt.files);
        let _ =
            ApprovalInbox::new(self.inbox_dir.clone()).resolve(&prompt.collection, &prompt.host);
        self.drop_pair(&prompt.collection, &prompt.host, cx);
        self.refresh(cx);
    }

    fn drop_pair(&mut self, collection: &Path, host: &str, cx: &mut Context<Self>) {
        let before = self.queue.len();
        self.queue
            .retain(|p| !(p.collection == collection && p.host == host));
        if self.queue.len() != before {
            cx.notify();
        }
    }

    /// Reads on the background executor; a result older than the newest reload is dropped,
    /// so a slow read never restores a pair decided since it started.
    fn reload(&mut self, prune: bool, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (inbox_dir, trust_file) = (self.inbox_dir.clone(), self.trust_file.clone());
        let read = cx.background_spawn(async move {
            let inbox = ApprovalInbox::new(inbox_dir);
            if prune {
                let _ = inbox.prune(PRUNE_AFTER);
            }
            let pending = inbox.pending().unwrap_or_default();
            let store =
                TrustStore::load(&trust_file).unwrap_or_else(|_| TrustStore::empty(trust_file));
            (pending, store)
        });
        cx.spawn(async move |this, cx| {
            let (pending, store) = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.rebuild(pending, &store, cx);
                }
            });
        })
        .detach();
        self.reload_scripts(cx);
    }

    /// Reads script entries on the background executor under their own generation, deleting
    /// the ones nothing is left to show for. A trust file that cannot be read shows nothing
    /// and deletes nothing.
    fn reload_scripts(&mut self, cx: &mut Context<Self>) {
        let Some(trust_file) = self.script_trust.clone() else {
            return;
        };
        self.script_generation += 1;
        let generation = self.script_generation;
        let inbox_dir = self.inbox_dir.clone();
        let secrets = self.secrets.clone();
        let read = cx.background_spawn(async move {
            let trust = ScriptTrust::load(trust_file).ok()?;
            let cache = trust.clone();
            let inbox = ApprovalInbox::new(inbox_dir);
            let mut prompts = Vec::new();
            for entry in inbox.pending_scripts().unwrap_or_default() {
                match script_prompt(&trust, secrets.as_ref(), entry.clone()) {
                    Some(prompt) => prompts.push(prompt),
                    None => {
                        let _ = inbox.remove_scripts(&entry);
                    }
                }
            }
            Some((cache, prompts))
        });
        cx.spawn(async move |this, cx| {
            let (cache, queue) = match read.await {
                Some((cache, queue)) => (Some(cache), queue),
                None => (None, Vec::new()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.script_generation != generation {
                    return;
                }
                // A file that cannot be read shows no decision rather than a stale one.
                if this.script_cache != cache {
                    this.script_cache = cache;
                    cx.notify();
                }
                let queue: Vec<ScriptPrompt> = queue
                    .into_iter()
                    .filter(|p| !this.scripts_set_aside.contains(&p.entry.file))
                    .collect();
                if queue != this.script_queue {
                    this.script_queue = queue;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Turns pending files into prompts: invalid hosts, non-collections and pairs the store
    /// already decides are deleted unprompted, and one prompt per pair keeps every file.
    /// Files come in file-name order; a prompt's proxy, and the name mapped to its host, is the first
    /// one any of its files names.
    fn rebuild(&mut self, pending: Vec<Approval>, store: &TrustStore, cx: &mut Context<Self>) {
        let mut queue: Vec<Prompt> = Vec::new();
        let mut doomed = Vec::new();
        for approval in pending {
            let Some(host) = normalize_host(&approval.host) else {
                doomed.push(approval.file);
                continue;
            };
            if !approval.collection.join("opencollection.yml").is_file() {
                doomed.push(approval.file);
                continue;
            }
            let collection = fs::canonicalize(&approval.collection).unwrap_or(approval.collection);
            if store.is_trusted(&collection, &host) || store.is_denied(&collection, &host) {
                doomed.push(approval.file);
                continue;
            }
            if self.set_aside.contains(&(collection.clone(), host.clone())) {
                continue;
            }
            match queue
                .iter_mut()
                .find(|p| p.collection == collection && p.host == host)
            {
                Some(prompt) => {
                    prompt.proxy = prompt.proxy.take().or(approval.proxy);
                    prompt.mapped_from = prompt.mapped_from.take().or(approval.mapped_from);
                    prompt.files.push(approval.file);
                }
                None => queue.push(Prompt {
                    collection,
                    host,
                    request: approval.request,
                    proxy: approval.proxy,
                    mapped_from: approval.mapped_from,
                    files: vec![approval.file],
                }),
            }
        }
        remove_files(&doomed);
        if queue != self.queue {
            self.queue = queue;
            cx.notify();
        }
    }
}

/// The prompt for `entry` from the collection as it is on disk: the listed texts not yet
/// approved, or every listed one while the collection is undecided. `None` (delete the
/// entry) for a collection that is blocked, cannot be opened or planned, or has no such text.
/// The entry names no environment, so texts mask the secrets of every one, read from `secrets`.
fn script_prompt(
    trust: &ScriptTrust,
    secrets: Option<&Arc<dyn SecretStore>>,
    entry: ScriptApproval,
) -> Option<ScriptPrompt> {
    let undecided = match trust.decision(&entry.collection) {
        None => true,
        Some(ScriptDecision::Allowed) => false,
        // Blocked, or a decision this build does not know.
        Some(_) => return None,
    };
    let collection = Collection::open(&entry.collection).ok()?;
    // A context that cannot be built drops the entry rather than show a value unmasked.
    let environments = collection.environments().iter().map(|e| Some(e.name()));
    let contexts = std::iter::once(None)
        .chain(environments)
        .map(|env| {
            let mut options = ContextOptions::new();
            if let Some(store) = secrets {
                options = options.secrets(store.clone());
            }
            if let Some(name) = env {
                options = options.env(name);
            }
            VarContext::new(&collection, options).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let target = match entry.request.as_str() {
        "" => collection.root().to_path_buf(),
        request => collection.root().join(request),
    };
    let plan = collection.plan(&target).ok()?;
    let texts: Vec<(ItemPath, ScriptText)> = plan_script_texts_across(&plan, &contexts)
        .into_iter()
        .filter(|(_, t)| entry.hashes.contains(&t.hash))
        .filter(|(_, t)| undecided || !trust.is_approved(&entry.collection, &t.hash))
        .collect();
    (!texts.is_empty()).then(|| ScriptPrompt {
        collection: entry.collection.clone(),
        request: entry.request.clone(),
        texts,
        entry,
    })
}

/// Deletes inbox files; one already gone was removed by another process.
fn remove_files(files: &[PathBuf]) {
    for file in files {
        let _ = fs::remove_file(file);
    }
}
