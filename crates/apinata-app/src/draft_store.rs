//! Keeps the workspace's dirty untitled drafts in `<data dir>/drafts`, blanked and scrubbed of
//! the collection's secret values, so they survive a restart.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::{DraftSnapshot, ItemPath};
use gpui_kit::{
    App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task, WeakEntity,
};

use crate::command::Command;
use crate::draft_file::{self, BuildError, DraftFile};
use crate::persisted::{Persisted, SaveFailed};
use crate::workspace::Workspace;

/// The toast for reopening the drafts `names` at start or on reopening their collection: one
/// by name, more by count.
pub fn reopened_message(names: &[String]) -> String {
    match names {
        [one] => format!("Reopened \"{one}\"."),
        many => format!("Reopened {} unsaved drafts.", many.len()),
    }
}

/// `reopened_message`, adding that credentials were cleared from what was reopened.
pub fn reopened_blanked_message(names: &[String]) -> String {
    let whose = if names.len() == 1 { "Its" } else { "Some" };
    format!(
        "{} {whose} credentials were cleared; enter them again before sending.",
        reopened_message(names)
    )
}

/// The app's draft store, set by `app::start`.
pub struct Drafts(pub Entity<DraftStore>);

impl Global for Drafts {}

/// What the store reports for a toast.
#[derive(Debug, Clone, PartialEq)]
pub enum DraftStoreEvent {
    /// Draft `name` is not kept after restart, for `reason`.
    NotKept {
        /// The draft's display name.
        name: String,
        /// Why it is not kept.
        reason: NotKeptReason,
    },
    /// A draft file could not be written; the I/O error text.
    SaveFailed(String),
    /// Kept drafts were reopened, by name; `credentials_blanked` when any had its credentials
    /// blanked.
    Reopened {
        /// The reopened drafts' names, oldest first.
        names: Vec<String>,
        /// Whether any of them had its credentials blanked.
        credentials_blanked: bool,
    },
}

/// Why a draft is not kept after restart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotKeptReason {
    /// Its file would be over `MAX_DRAFT_BYTES`.
    TooLarge,
    /// `MAX_DRAFTS` drafts are kept already.
    TooMany,
    /// Its request cannot be written as JSON.
    Unserializable,
}

// What became of a tracked draft's latest build.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome {
    Written,
    // The root's secret list could not be had, so nothing was written.
    NoSecrets,
    // Not written and not retried until the draft changes: over the size cap or unserializable.
    TooLarge,
    // Not written: MAX_DRAFTS files are kept already.
    TooMany,
}

// A draft file's identity: its history key and which of the tabs sharing that key it is.
type Id = (ItemPath, u32);

struct Tracked {
    built_from: Option<Arc<DraftSnapshot>>,
    // Whether that build held credentials to blank; the blanked snapshot hides a literal change.
    built_blanks: bool,
    // The context generation of the secret list that build used; a later generation may
    // have changed which values are secret.
    built_gen: u64,
    // That list itself: a later generation with an equal list leaves the file as it is.
    built_list: Vec<String>,
    outcome: Outcome,
    file: Option<Entity<Persisted<DraftFile>>>,
    _failed: Option<Subscription>,
}

#[derive(Default)]
struct RootState {
    // The last list and the context generation it was asked for under.
    list: Option<(u64, Vec<String>)>,
    // Dropping this task cancels the fetch, so it lives here until the reply is handled.
    fetch: Option<Task<()>>,
    failed_gen: Option<u64>,
}

// A dirty, keyed draft whose file is out of date.
struct Stale {
    item: ItemPath,
    key: ItemPath,
    slot: u32,
    name: String,
    snapshot: Arc<DraftSnapshot>,
    blanks: bool,
}

/// Writes one draft file per dirty untitled draft, a save delay after it changes.
pub struct DraftStore {
    dir: PathBuf,
    lock: Option<File>,
    live: BTreeMap<PathBuf, Vec<DraftFile>>,
    workspace: Option<WeakEntity<Workspace>>,
    tracked: HashMap<Id, Tracked>,
    // The file each open dirty draft tab writes, as (root, item) to its id.
    bound: HashMap<(PathBuf, ItemPath), Id>,
    // Restored files whose tab could not be made: no tab shows them, so nothing may rebind,
    // rewrite or discard them. They stay on disk to restore again on the next launch.
    reserved: HashSet<Id>,
    roots: HashMap<PathBuf, RootState>,
    // Drafts already reported as not kept, so each is reported once.
    warned: HashSet<ItemPath>,
    fetches: u64,
    writes: u64,
    _observe: Option<Subscription>,
}

impl EventEmitter<DraftStoreEvent> for DraftStore {}

impl DraftStore {
    /// Takes `dir/.lock` and loads `dir`; without the lock the store is disabled and does nothing.
    pub fn open(dir: PathBuf, _cx: &mut Context<Self>) -> DraftStore {
        let lock = draft_file::lock(&dir);
        let mut live: BTreeMap<PathBuf, Vec<DraftFile>> = BTreeMap::new();
        if lock.is_some() {
            let loaded = draft_file::load_dir(&dir);
            for line in &loaded.skipped {
                eprintln!("apinata: drafts: skipped {line}");
            }
            if loaded.unopened > 0 {
                eprintln!(
                    "apinata: drafts: {} older drafts left closed",
                    loaded.unopened
                );
            }
            for file in loaded.drafts {
                live.entry(file.root.clone()).or_default().push(file);
            }
        } else {
            eprintln!("apinata: drafts: another instance keeps the drafts; none are kept here");
        }
        DraftStore {
            dir,
            lock,
            live,
            workspace: None,
            tracked: HashMap::new(),
            bound: HashMap::new(),
            reserved: HashSet::new(),
            roots: HashMap::new(),
            warned: HashSet::new(),
            fetches: 0,
            writes: 0,
            _observe: None,
        }
    }

    /// Whether the store holds kept drafts for the collection at `root`.
    pub fn holds(&self, root: &Path) -> bool {
        self.live.contains_key(root)
    }

    /// Whether this store holds the drafts lock.
    pub fn enabled(&self) -> bool {
        self.lock.is_some()
    }

    /// Watches `workspace` (held weakly) and writes its dirty drafts.
    pub fn attach(&mut self, workspace: &Entity<Workspace>, cx: &mut Context<Self>) {
        if !self.enabled() {
            return;
        }
        self.workspace = Some(workspace.downgrade());
        self._observe = Some(cx.observe(workspace, |this, _, cx| this.scan(cx)));
        self.scan(cx);
    }

    /// Forgets draft `key` and removes its file.
    pub fn discard(&mut self, key: &ItemPath, cx: &mut Context<Self>) {
        if !self.enabled() {
            return;
        }
        // Loaded files not taken yet sit on disk untracked, so each slot is removed by name.
        let mut untaken: HashSet<u32> = HashSet::from([0]);
        for files in self.live.values_mut() {
            untaken.extend(
                files
                    .iter()
                    .filter(|f| f.key == key.as_str())
                    .map(|f| f.slot),
            );
            files.retain(|f| f.key != key.as_str());
        }
        self.bound.retain(|_, (k, _)| k != key);
        let ids: Vec<Id> = (self.tracked.keys())
            .filter(|id| id.0 == *key && !self.reserved.contains(id))
            .cloned()
            .collect();
        let held =
            |s: &u32| ids.iter().any(|id| id.1 == *s) || self.reserved.contains(&(key.clone(), *s));
        for slot in untaken.into_iter().filter(|s| !held(s)) {
            let path = self.dir.join(draft_file::file_name_slot(key, slot));
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("apinata: drafts: could not remove {}: {e}", path.display());
            }
        }
        for id in ids {
            self.drop_file(&id, cx);
        }
    }

    // Forgets one draft file and removes it from disk.
    fn drop_file(&mut self, id: &Id, cx: &mut Context<Self>) {
        if let Some(file) = self.tracked.remove(id).and_then(|t| t.file) {
            file.update(cx, |p, cx| p.discard(cx)).detach();
        }
    }

    /// Moves `root`'s loaded drafts, oldest first, into the tracked set, each holding its file
    /// as already written, and returns them to be opened. Emits `Reopened` once when
    /// there are any. A disabled store returns nothing.
    pub fn take_root(&mut self, root: &Path, cx: &mut Context<Self>) -> Vec<DraftFile> {
        if !self.enabled() {
            return Vec::new();
        }
        let mut files = self.live.remove(root).unwrap_or_default();
        files.retain(|f| f.item_key().is_some());
        files.sort_by(|a, b| (a.created_ms, &a.key).cmp(&(b.created_ms, &b.key)));
        for file in &files {
            let Some(key) = file.item_key() else {
                continue;
            };
            let path = self.dir.join(draft_file::file_name_slot(&key, file.slot));
            let (p, failed) = persisted(file.clone(), path, cx);
            // The toast below informs the user once per draft, so the file stops asking for it;
            // a literal credential typed later sets the flag again through the next build.
            if file.credentials_blanked {
                p.update(cx, |p, cx| p.update(cx, |v| v.credentials_blanked = false));
            }
            // The tab is made from this very file, so it counts as kept until it changes. Its
            // request is already blanked, so the view will hold no credentials to blank.
            self.tracked.insert(
                (key, file.slot),
                Tracked {
                    built_from: file.snapshot().map(Arc::new),
                    built_blanks: false,
                    built_gen: 0,
                    built_list: Vec::new(),
                    outcome: Outcome::Written,
                    file: Some(p),
                    _failed: Some(failed),
                },
            );
        }
        if !files.is_empty() {
            cx.emit(DraftStoreEvent::Reopened {
                names: files.iter().map(DraftFile::name).collect(),
                credentials_blanked: files.iter().any(|f| f.credentials_blanked),
            });
        }
        files
    }

    /// Binds the restored tab `item` of `root` to the file slot `slot` of `key` it was made from,
    /// so a rewrite never lands on another tab's file. Does nothing for a file not tracked.
    pub fn bind_restored(&mut self, root: &Path, item: &ItemPath, key: &ItemPath, slot: u32) {
        let id = (key.clone(), slot);
        if self.tracked.contains_key(&id) {
            self.bound.insert((root.to_path_buf(), item.clone()), id);
        }
    }

    /// Keeps the restored file `slot` of `key` out of reach of every tab: its tab could not be
    /// made, so the user never saw its content and no later tab may take the slot over.
    pub fn reserve_failed(&mut self, key: &ItemPath, slot: u32) {
        let id = (key.clone(), slot);
        if self.tracked.contains_key(&id) {
            self.reserved.insert(id);
        }
    }

    /// How many secret lists the store has asked for; for tests and logs.
    pub fn fetches(&self) -> u64 {
        self.fetches
    }

    /// How many draft files the store has built and handed to be written; for tests and logs.
    pub fn writes(&self) -> u64 {
        self.writes
    }

    // Asks each open root with stale drafts for its secret list, one fetch per root at a time.
    fn scan(&mut self, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        // Every open collection, the scratch included: its drafts must be kept and prompted.
        let roots: Vec<PathBuf> = (ws.read(cx).collections().iter())
            .map(|c| c.root().to_path_buf())
            .collect();
        for root in roots {
            for key in self.cleaned(&ws, &root, cx) {
                self.discard(&key, cx);
            }
            self.rebind(&ws, &root, cx);
            let busy = self.roots.get(&root).is_some_and(|r| r.fetch.is_some());
            if busy || self.stale(&ws, &root, cx).is_empty() {
                continue;
            }
            let asked = generation(&ws, &root, cx);
            let (reply, rx) = async_channel::bounded(1);
            ws.update(cx, |w, cx| {
                w.send(&root, Command::SecretValues { reply }, cx)
            });
            self.fetches += 1;
            let at = root.clone();
            let fetch = cx.spawn(async move |this, cx| {
                let result = (rx.recv().await).unwrap_or_else(|e| Err(e.to_string()));
                this.update(cx, |s, cx| s.replied(&at, asked, result, cx))
                    .ok();
            });
            self.roots.entry(root).or_default().fetch = Some(fetch);
        }
    }

    // Tracked drafts of `root` whose tab is open but no longer dirty: an edit undone back to
    // clean, whose file would otherwise bring the undone content back. A key a dirty tab
    // still holds keeps its file.
    fn cleaned(&self, ws: &Entity<Workspace>, root: &Path, cx: &App) -> Vec<ItemPath> {
        let w = ws.read(cx);
        let Some(snap) = w.snapshot(root, cx) else {
            return Vec::new();
        };
        (snap.content.requests.iter())
            .filter(|(item, view)| item.is_draft() && !view.dirty)
            .filter_map(|(item, _)| w.draft_key(root, item))
            .filter(|key| {
                self.tracked.keys().any(|(k, _)| k == key) && !w.draft_key_held_dirty(key, cx)
            })
            .collect()
    }

    // The dirty, keyed drafts of `root` built from an older view, or left unwritten for want of
    // a secret list while the root's variable context has since changed.
    fn stale(&self, ws: &Entity<Workspace>, root: &Path, cx: &App) -> Vec<Stale> {
        let w = ws.read(cx);
        let Some(snap) = w.snapshot(root, cx) else {
            return Vec::new();
        };
        let failed_gen = self.roots.get(root).and_then(|r| r.failed_gen);
        let mut out = Vec::new();
        for (item, view) in snap.content.requests.iter() {
            let (true, true) = (item.is_draft(), view.dirty) else {
                continue;
            };
            let Some(snapshot) = view.snapshot.as_ref() else {
                continue;
            };
            let Some(id) = self.bound.get(&(root.to_path_buf(), item.clone())) else {
                continue;
            };
            let is_stale = match self.tracked.get(id) {
                None => true,
                Some(t) => {
                    t.built_from.as_ref() != Some(snapshot)
                        || t.built_blanks != view.blanks_credentials
                        || (t.outcome == Outcome::Written && t.built_gen < snap.context_generation)
                        || (t.outcome == Outcome::NoSecrets
                            && failed_gen != Some(snap.context_generation))
                }
            };
            if is_stale {
                out.push(Stale {
                    item: item.clone(),
                    key: id.0.clone(),
                    slot: id.1,
                    name: view.name.clone(),
                    snapshot: Arc::clone(snapshot),
                    blanks: view.blanks_credentials,
                });
            }
        }
        out
    }

    // Gives each dirty, keyed draft tab of `root` its own file slot, and drops the file of a
    // tab that closed, went clean or was rekeyed. A tab takes a restored file no tab holds yet
    // before it takes a new slot, so tabs sharing a key never share a file.
    fn rebind(&mut self, ws: &Entity<Workspace>, root: &Path, cx: &mut Context<Self>) {
        let w = ws.read(cx);
        let Some(snap) = w.snapshot(root, cx) else {
            return;
        };
        let mut dirty: Vec<(ItemPath, ItemPath)> = (snap.content.requests.iter())
            .filter(|(item, view)| item.is_draft() && view.dirty)
            .filter_map(|(item, _)| Some((item.clone(), w.draft_key(root, item)?)))
            .collect();
        dirty.sort();
        let gone: Vec<(PathBuf, ItemPath)> = (self.bound.iter())
            .filter(|((r, item), (key, _))| {
                r == root && !dirty.iter().any(|(i, k)| i == item && k == key)
            })
            .map(|(at, _)| at.clone())
            .collect();
        for at in gone {
            if let Some(id) = self.bound.remove(&at) {
                self.drop_file(&id, cx);
            }
        }
        for (item, key) in dirty {
            let at = (root.to_path_buf(), item);
            if self.bound.contains_key(&at) {
                continue;
            }
            let mut taken: HashSet<u32> = (self.bound.values())
                .filter(|(k, _)| *k == key)
                .map(|(_, slot)| *slot)
                .collect();
            taken.extend(
                (self.reserved.iter())
                    .filter(|(k, _)| *k == key)
                    .map(|id| id.1),
            );
            let slot = (self.tracked.keys())
                .filter(|(k, slot)| *k == key && !taken.contains(slot))
                .map(|(_, slot)| *slot)
                .min()
                .or_else(|| (0..).find(|slot| !taken.contains(slot)))
                .unwrap_or(0);
            self.bound.insert(at, (key, slot));
        }
    }

    // Builds and writes each stale draft of `root` from its current view with the list asked
    // for under context generation `asked`, or marks it unwritten when the list could not be
    // had; either way it is not retried until something changes. A list asked for under an
    // older context is dropped and asked for again.
    fn replied(
        &mut self,
        root: &Path,
        asked: u64,
        result: Result<Vec<String>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.workspace.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let current = asked >= generation(&ws, root, cx);
        let state = self.roots.entry(root.to_path_buf()).or_default();
        state.fetch = None;
        if current {
            match result {
                Ok(list) => {
                    state.list = Some((asked, list));
                    state.failed_gen = None;
                }
                Err(e) => {
                    eprintln!(
                        "apinata: drafts: no secret list for {}: {e}",
                        root.display()
                    );
                    state.list = None;
                    state.failed_gen = Some(asked);
                }
            }
            let list = state.list.as_ref().map(|(_, l)| l.clone());
            self.build_stale(&ws, root, list.as_deref(), cx);
        }
        self.scan(cx);
    }

    // Builds and writes each stale draft of `root` with `list`, or marks it unwritten without one.
    fn build_stale(
        &mut self,
        ws: &Entity<Workspace>,
        root: &Path,
        list: Option<&[String]>,
        cx: &mut Context<Self>,
    ) {
        self.rebind(ws, root, cx);
        let built_gen = self
            .roots
            .get(root)
            .and_then(|r| r.list.as_ref())
            .map_or(0, |(g, _)| *g);
        for stale in self.stale(ws, root, cx) {
            // Only the context generation moved and the secret list came out the same: the
            // file on hand is already right, so record the new generation without a write.
            if let Some(list) = list
                && let Some(t) = self.tracked.get_mut(&(stale.key.clone(), stale.slot))
                && t.outcome == Outcome::Written
                && t.built_from.as_ref() == Some(&stale.snapshot)
                && t.built_blanks == stale.blanks
                && t.built_list == list
            {
                t.built_gen = built_gen;
                continue;
            }
            let outcome = match list {
                None => Outcome::NoSecrets,
                Some(list) => self.write(root, &stale, list, cx),
            };
            let t = self
                .tracked
                .entry((stale.key, stale.slot))
                .or_insert(Tracked {
                    built_from: None,
                    built_blanks: false,
                    built_gen: 0,
                    built_list: Vec::new(),
                    outcome,
                    file: None,
                    _failed: None,
                });
            t.built_list = list.map(<[String]>::to_vec).unwrap_or_default();
            t.built_from = Some(stale.snapshot);
            t.built_blanks = stale.blanks;
            t.built_gen = built_gen;
            t.outcome = outcome;
        }
    }

    /// Builds every stale dirty draft of the open roots, or only of `only`, with its root's
    /// last secret list and writes each tracked file of those roots on this thread. A root with
    /// no list, or one asked for under an older context, builds nothing; a draft over a cap gets
    /// nothing written; a write error emits `SaveFailed`.
    pub fn flush_now(&mut self, only: Option<&Path>, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let roots = self.roots_in(&ws, only, cx);
        for root in &roots {
            let current = generation(&ws, root, cx);
            let list = (self.roots.get(root).and_then(|r| r.list.clone()))
                .filter(|(asked, _)| *asked >= current)
                .map(|(_, list)| list);
            if let Some(list) = list {
                self.build_stale(&ws, root, Some(&list), cx);
            }
        }
        let files: Vec<_> = (self.tracked.values())
            .filter_map(|t| t.file.clone())
            .filter(|p| roots.contains(&p.read(cx).get().root))
            .collect();
        for file in files {
            if let Err(e) = file.update(cx, |p, _| p.flush_now()) {
                eprintln!("apinata: drafts: could not write a draft: {e}");
                cx.emit(DraftStoreEvent::SaveFailed(e.to_string()));
            }
        }
    }

    /// Whether draft `key` is kept as `current`: tracked, last built from it with the same
    /// `blanks` (whether it holds credentials to blank), and that build on disk.
    pub fn is_kept(&self, key: &ItemPath, current: &DraftSnapshot, blanks: bool, cx: &App) -> bool {
        (self.tracked.iter()).any(|((k, _), t)| {
            k == key
                && t.built_from.as_deref() == Some(current)
                && t.built_blanks == blanks
                && latest_on_disk(t, cx)
        })
    }

    /// Discards the file of each draft in `listed`, as `(root, item, key)`, so an older file
    /// cannot bring it back. A key that a dirty, kept draft not in `listed` holds is left alone.
    pub fn discard_listed(
        &mut self,
        listed: &[(PathBuf, ItemPath, ItemPath)],
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.workspace.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let mut kept = HashSet::new();
        for root in self.roots_in(&ws, None, cx) {
            let w = ws.read(cx);
            let Some(snap) = w.snapshot(&root, cx) else {
                continue;
            };
            for (item, view) in snap.content.requests.iter() {
                let unlisted = !listed.iter().any(|(r, i, _)| *r == root && i == item);
                let Some(key) = (item.is_draft() && view.dirty && unlisted)
                    .then(|| w.draft_key(&root, item))
                    .flatten()
                else {
                    continue;
                };
                if (view.snapshot.as_ref())
                    .is_some_and(|s| self.is_kept(&key, s, view.blanks_credentials, cx))
                {
                    kept.insert(key);
                }
            }
        }
        for (_, _, key) in listed {
            if !kept.contains(key) && self.tracked.keys().any(|(k, _)| k == key) {
                self.discard(key, cx);
            }
        }
    }

    /// Hands `root`'s tracked drafts whose latest build is on disk back to the loaded set, as their current
    /// files, so reopening the root restores them; forgets the rest of the root's state.
    pub fn return_root(&mut self, root: &Path, cx: &App) {
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let of_root: Vec<Id> = (self.tracked.iter())
            .filter(|(_, t)| {
                t.file
                    .as_ref()
                    .is_some_and(|p| p.read(cx).get().root == root)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in of_root {
            let Some(t) = self.tracked.remove(&key) else {
                continue;
            };
            self.reserved.remove(&key);
            if let Some(file) = t.file.as_ref().filter(|_| latest_on_disk(&t, cx)) {
                self.live
                    .entry(root.clone())
                    .or_default()
                    .push(file.read(cx).get().clone());
            }
        }
        self.bound.retain(|(r, _), _| *r != root);
        self.roots.remove(&root);
    }

    // The open roots, or only `only` when it is open.
    fn roots_in(&self, ws: &Entity<Workspace>, only: Option<&Path>, cx: &App) -> Vec<PathBuf> {
        // The scratch is included on purpose, so its drafts are flushed and prompted too.
        let only = only.map(|o| std::path::absolute(o).unwrap_or_else(|_| o.to_path_buf()));
        (ws.read(cx).collections().iter())
            .map(|c| c.root().to_path_buf())
            .filter(|r| only.as_ref().is_none_or(|o| o == r))
            .collect()
    }

    // Builds the file for `stale` and hands it to its `Persisted`, created on the first write.
    fn write(
        &mut self,
        root: &Path,
        stale: &Stale,
        list: &[String],
        cx: &mut Context<Self>,
    ) -> Outcome {
        let parent = stale.item.parent().unwrap_or_else(ItemPath::root);
        let id = (stale.key.clone(), stale.slot);
        let built = draft_file::build_slot(
            &stale.key,
            stale.slot,
            root,
            &parent,
            &stale.snapshot,
            stale.blanks,
            list,
        );
        let file = match built {
            Ok(file) => file,
            Err(err) => {
                let reason = match &err {
                    BuildError::TooLarge => {
                        eprintln!(
                            "apinata: drafts: {} is over 1 MiB and is not kept",
                            stale.key.as_str()
                        );
                        NotKeptReason::TooLarge
                    }
                    BuildError::Unserializable(why) => {
                        eprintln!(
                            "apinata: drafts: {} cannot be written and is not kept: {why}",
                            stale.key.as_str()
                        );
                        NotKeptReason::Unserializable
                    }
                };
                // An earlier file would bring back content older than the draft, so it goes too.
                if let Some(t) = self.tracked.get_mut(&id)
                    && let Some(p) = t.file.take()
                {
                    t._failed = None;
                    p.update(cx, |p, cx| p.discard(cx)).detach();
                }
                self.not_kept(stale, reason, cx);
                return Outcome::TooLarge;
            }
        };
        let has_file = self.tracked.get(&id).is_some_and(|t| t.file.is_some());
        if !has_file && self.kept_count() >= draft_file::MAX_DRAFTS {
            eprintln!(
                "apinata: drafts: {} is not kept: {} drafts are kept already",
                stale.key.as_str(),
                draft_file::MAX_DRAFTS
            );
            self.not_kept(stale, NotKeptReason::TooMany, cx);
            return Outcome::TooMany;
        }
        let path = (self.dir).join(draft_file::file_name_slot(&stale.key, stale.slot));
        let t = self.tracked.entry(id).or_insert(Tracked {
            built_from: None,
            built_blanks: false,
            built_gen: 0,
            built_list: Vec::new(),
            outcome: Outcome::Written,
            file: None,
            _failed: None,
        });
        let persisted = match &t.file {
            Some(p) => p.clone(),
            None => {
                let (p, failed) = persisted(file.clone(), path, cx);
                t._failed = Some(failed);
                t.file = Some(p.clone());
                p
            }
        };
        persisted.update(cx, |p, cx| p.update(cx, |v| *v = file));
        self.writes += 1;
        Outcome::Written
    }

    // Files the cap counts: tracked drafts holding one plus loaded drafts not yet tracked.
    fn kept_count(&self) -> usize {
        let tracked = self.tracked.values().filter(|t| t.file.is_some()).count();
        let live = (self.live.values().flatten())
            .filter(|f| {
                ItemPath::new(&f.key).is_none_or(|k| !self.tracked.contains_key(&(k, f.slot)))
            })
            .count();
        tracked + live
    }

    fn not_kept(&mut self, stale: &Stale, reason: NotKeptReason, cx: &mut Context<Self>) {
        if self.warned.insert(stale.key.clone()) {
            cx.emit(DraftStoreEvent::NotKept {
                name: stale.name.clone(),
                reason,
            });
        }
    }
}

// The context generation of `root`'s latest snapshot; 0 before the first.
fn generation(ws: &Entity<Workspace>, root: &Path, cx: &App) -> u64 {
    (ws.read(cx).snapshot(root, cx)).map_or(0, |s| s.context_generation)
}

// A `Persisted` for a draft file that is already on disk, forwarding its write failures.
fn persisted(
    file: DraftFile,
    path: PathBuf,
    cx: &mut Context<DraftStore>,
) -> (Entity<Persisted<DraftFile>>, Subscription) {
    let p = cx.new(|_| Persisted::new(file, path));
    let failed = cx.subscribe(&p, |_, _, e: &SaveFailed, cx| {
        cx.emit(DraftStoreEvent::SaveFailed(e.0.clone()));
    });
    (p, failed)
}

// Whether the draft's file holds its latest build: an older file left by a failed or skipped
// build does not count.
fn latest_on_disk(t: &Tracked, cx: &App) -> bool {
    t.outcome == Outcome::Written && t.file.as_ref().is_some_and(|p| p.read(cx).is_written())
}

/// `DraftStore::discard` through the global; nothing without one.
pub fn discard(key: &ItemPath, cx: &mut App) {
    if let Some(store) = store(cx) {
        store.update(cx, |s, cx| s.discard(key, cx));
    }
}

// The store behind the global, if any.
fn store(cx: &App) -> Option<Entity<DraftStore>> {
    cx.try_global::<Drafts>().map(|d| d.0.clone())
}

/// `DraftStore::flush_now` through the global; nothing without one.
pub fn flush_now(only: Option<&Path>, cx: &mut App) {
    if let Some(store) = store(cx) {
        store.update(cx, |s, cx| s.flush_now(only, cx));
    }
}

/// `DraftStore::bind_restored` through the global; nothing without one.
pub fn bind_restored(root: &Path, item: &ItemPath, key: &ItemPath, slot: u32, cx: &mut App) {
    if let Some(store) = store(cx) {
        store.update(cx, |s, _| s.bind_restored(root, item, key, slot));
    }
}

/// `DraftStore::reserve_failed` through the global; nothing without one.
pub fn reserve_failed(key: &ItemPath, slot: u32, cx: &mut App) {
    if let Some(store) = store(cx) {
        store.update(cx, |s, _| s.reserve_failed(key, slot));
    }
}

/// `DraftStore::is_kept` through the global; false without one.
pub fn is_kept(key: &ItemPath, current: &DraftSnapshot, blanks: bool, cx: &App) -> bool {
    store(cx).is_some_and(|s| s.read(cx).is_kept(key, current, blanks, cx))
}

/// `DraftStore::discard_listed` through the global; nothing without one.
pub fn discard_listed(listed: &[(PathBuf, ItemPath, ItemPath)], cx: &mut App) {
    if let Some(store) = store(cx) {
        store.update(cx, |s, cx| s.discard_listed(listed, cx));
    }
}

/// `DraftStore::return_root` through the global; nothing without one.
pub fn return_root(root: &Path, cx: &mut App) {
    if let Some(store) = store(cx) {
        store.update(cx, |s, cx| s.return_root(root, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::DraftStore;
    use crate::command::{Command, Edit, Target};
    use crate::persist::{Paths, save_json};
    use crate::session::Session;
    use crate::settings::{ProxyMode, Settings};
    use apinata_core::{Collection, ItemPath, MemoryStore, SecretStore};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use std::sync::Arc;

    const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: token\n";

    // The reply goes through the actor and cannot be delayed from outside, so this calls
    // `replied` directly on a store that is not observing: a list asked for before the
    // environment switch must not build a file, and the store must ask again.
    #[gpui_kit::test]
    fn a_secret_list_asked_under_an_older_context_is_dropped_and_asked_again(
        cx: &mut TestAppContext,
    ) {
        let col = tempfile::tempdir().unwrap();
        std::fs::write(
            col.path().join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .unwrap();
        std::fs::create_dir(col.path().join("environments")).unwrap();
        std::fs::write(col.path().join("environments/dev.yml"), DEV).unwrap();
        let secrets = Arc::new(MemoryStore::new());
        let c = Collection::open(col.path()).unwrap();
        secrets
            .set(&c.secret_key("dev", "token"), "env-token-value-1")
            .unwrap();

        // The app's own store is disabled by this lock, so only the store under test writes.
        let files = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(files.path());
        let mut settings = Settings::default();
        settings.proxy.mode = ProxyMode::Off;
        save_json(&paths.settings, &settings).unwrap();
        let session = Session {
            open: vec![col.path().to_path_buf()],
            ..Default::default()
        };
        save_json(&paths.session, &session).unwrap();
        let _lock = crate::draft_file::lock(&paths.drafts).expect("the drafts lock is free");
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            cx.set_reduce_motion(true);
        });
        let (win, shell) = cx
            .update(|cx| crate::app::start(&paths, secrets, Vec::new(), cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let ws = shell.read_with(cx, |s, _| s.workspace().clone());
        let root = col.path().to_path_buf();

        ws.update(cx, |w, cx| w.new_draft(&root, ItemPath::root(), cx))
            .expect("open");
        cx.run_until_parked();
        let (item, key) = ws.read_with(cx, |w, _| {
            let e = w.tabs().active().expect("the draft tab");
            let key = w.tabs().history_key(e.id).cloned().expect("keyed");
            (e.item().expect("a request").clone(), key)
        });
        let edit = Command::Edit {
            target: Target::Request(item),
            edit: Edit::Url("http://127.0.0.1:9/env-token-value-1".into()),
        };
        ws.update(cx, |w, cx| w.send(&root, edit, cx));
        cx.run_until_parked();
        let older = cx.update(|cx| super::generation(&ws, &root, cx));
        ws.update(cx, |w, cx| {
            w.select_environment(&root, Some("dev".into()), cx)
        });
        cx.run_until_parked();
        assert!(cx.update(|cx| super::generation(&ws, &root, cx)) > older);

        let dir = tempfile::tempdir().unwrap();
        let store = cx.new(|cx| DraftStore::open(dir.path().to_path_buf(), cx));
        // An empty list, asked for before the switch, would leave the token in the file.
        store.update(cx, |s, cx| {
            s.workspace = Some(ws.downgrade());
            s.replied(&root, older, Ok(Vec::new()), cx);
            s.flush_now(None, cx);
        });
        let path = dir.path().join(crate::draft_file::file_name(&key));
        assert!(
            !path.exists(),
            "built from the older list: {}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert_eq!(store.read_with(cx, |s, _| s.fetches()), 1, "asked again");
    }

    // A value that becomes secret after the draft was written must leave the file without the
    // draft being edited again.
    #[gpui_kit::test]
    fn a_draft_file_is_rewritten_when_a_value_becomes_secret(cx: &mut TestAppContext) {
        let col = tempfile::tempdir().unwrap();
        std::fs::write(
            col.path().join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .unwrap();
        std::fs::create_dir(col.path().join("environments")).unwrap();
        std::fs::write(col.path().join("environments/dev.yml"), DEV).unwrap();
        let secrets = Arc::new(MemoryStore::new());
        let c = Collection::open(col.path()).unwrap();
        secrets
            .set(&c.secret_key("dev", "token"), "env-token-value-1")
            .unwrap();

        let files = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(files.path());
        let mut settings = Settings::default();
        settings.proxy.mode = ProxyMode::Off;
        save_json(&paths.settings, &settings).unwrap();
        let session = Session {
            open: vec![col.path().to_path_buf()],
            ..Default::default()
        };
        save_json(&paths.session, &session).unwrap();
        let _lock = crate::draft_file::lock(&paths.drafts).expect("the drafts lock is free");
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            cx.set_reduce_motion(true);
        });
        let (win, shell) = cx
            .update(|cx| crate::app::start(&paths, secrets, Vec::new(), cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let ws = shell.read_with(cx, |s, _| s.workspace().clone());
        let root = col.path().to_path_buf();

        let dir = tempfile::tempdir().unwrap();
        let store = cx.new(|cx| DraftStore::open(dir.path().to_path_buf(), cx));
        store.update(cx, |s, cx| s.attach(&ws, cx));

        ws.update(cx, |w, cx| w.new_draft(&root, ItemPath::root(), cx))
            .expect("open");
        cx.run_until_parked();
        let (item, key) = ws.read_with(cx, |w, _| {
            let e = w.tabs().active().expect("the draft tab");
            let key = w.tabs().history_key(e.id).cloned().expect("keyed");
            (e.item().expect("a request").clone(), key)
        });
        let edit = Command::Edit {
            target: Target::Request(item),
            edit: Edit::Url("http://127.0.0.1:9/env-token-value-1".into()),
        };
        ws.update(cx, |w, cx| w.send(&root, edit, cx));
        cx.run_until_parked();
        store.update(cx, |s, cx| s.flush_now(None, cx));
        let path = dir.path().join(crate::draft_file::file_name(&key));
        let before = std::fs::read_to_string(&path).expect("the draft is written");
        assert!(before.contains("env-token-value-1"), "not secret yet");

        // The draft is untouched from here on; only the secret values change.
        ws.update(cx, |w, cx| {
            w.select_environment(&root, Some("dev".into()), cx)
        });
        cx.run_until_parked();
        store.update(cx, |s, cx| s.flush_now(None, cx));
        let after = std::fs::read_to_string(&path).expect("the draft is still there");
        assert!(!after.contains("env-token-value-1"), "leaked: {after}");

        // The draft is parked now: nothing may ask for another list.
        let settled = store.read_with(cx, |s, _| s.fetches());
        cx.run_until_parked();
        store.update(cx, |s, cx| s.flush_now(None, cx));
        cx.run_until_parked();
        assert_eq!(
            store.read_with(cx, |s, _| s.fetches()),
            settled,
            "the refetch loops"
        );
    }

    // A context change that leaves the secret list as it was must not rewrite the file; one that
    // changes the list must.
    #[gpui_kit::test]
    fn a_context_change_with_the_same_secret_list_writes_nothing(cx: &mut TestAppContext) {
        let col = tempfile::tempdir().unwrap();
        std::fs::write(
            col.path().join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .unwrap();
        std::fs::create_dir(col.path().join("environments")).unwrap();
        std::fs::write(col.path().join("environments/dev.yml"), DEV).unwrap();
        std::fs::write(
            col.path().join("environments/dev2.yml"),
            DEV.replace("dev", "dev2"),
        )
        .unwrap();
        let secrets = Arc::new(MemoryStore::new());
        let c = Collection::open(col.path()).unwrap();
        for env in ["dev", "dev2"] {
            secrets
                .set(&c.secret_key(env, "token"), "env-token-value-1")
                .unwrap();
        }

        let files = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(files.path());
        let mut settings = Settings::default();
        settings.proxy.mode = ProxyMode::Off;
        save_json(&paths.settings, &settings).unwrap();
        let session = Session {
            open: vec![col.path().to_path_buf()],
            ..Default::default()
        };
        save_json(&paths.session, &session).unwrap();
        let _lock = crate::draft_file::lock(&paths.drafts).expect("the drafts lock is free");
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            cx.set_reduce_motion(true);
        });
        let (win, shell) = cx
            .update(|cx| crate::app::start(&paths, secrets, Vec::new(), cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let ws = shell.read_with(cx, |s, _| s.workspace().clone());
        let root = col.path().to_path_buf();

        let dir = tempfile::tempdir().unwrap();
        let store = cx.new(|cx| DraftStore::open(dir.path().to_path_buf(), cx));
        store.update(cx, |s, cx| s.attach(&ws, cx));
        ws.update(cx, |w, cx| w.new_draft(&root, ItemPath::root(), cx))
            .expect("open");
        cx.run_until_parked();
        let item = ws.read_with(cx, |w, _| {
            w.tabs()
                .active()
                .expect("the draft tab")
                .item()
                .expect("a request")
                .clone()
        });
        let edit = Command::Edit {
            target: Target::Request(item),
            edit: Edit::Url("http://127.0.0.1:9/env-token-value-1".into()),
        };
        ws.update(cx, |w, cx| w.send(&root, edit, cx));
        let select = |name: Option<&str>, cx: &mut TestAppContext| {
            ws.update(cx, |w, cx| {
                w.select_environment(&root, name.map(Into::into), cx)
            });
            cx.run_until_parked();
            store.update(cx, |s, cx| s.flush_now(None, cx));
            cx.run_until_parked();
            store.read_with(cx, |s, _| s.writes())
        };
        select(Some("dev"), cx);
        let written = store.read_with(cx, |s, _| s.writes());
        assert_eq!(select(Some("dev2"), cx), written, "same list, rewritten");
        assert!(select(None, cx) > written, "a changed list was not written");
    }
}
