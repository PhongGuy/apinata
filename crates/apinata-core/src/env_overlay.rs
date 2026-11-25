//! Script edits to the environment, global and collection variables, kept in memory over the files.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use serde_json::Value;

use crate::interpolate::Vars;

/// What a script did to one name in a layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OverlayEntry {
    /// Set to a value.
    Set(Value),
    /// Set to `undefined`: the name exists but removes itself and lower layers from lookups.
    Undefined,
    /// Deleted: the layer no longer has the name, so a lower layer shows through.
    Deleted,
}

/// The overlaid layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverlayLayer {
    /// Collection variables.
    Collection,
    /// The selected environment's variables.
    Environment,
    /// Global environment variables, one map for whichever is selected.
    Global,
}

/// One overlay entry as listed by [`EnvOverlay::entries`].
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverlayVar {
    layer: OverlayLayer,
    name: String,
    value: Option<String>,
    deleted: bool,
}

impl OverlayVar {
    /// The layer the script wrote.
    pub fn layer(&self) -> OverlayLayer {
        self.layer
    }

    /// The variable's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The text `{{name}}` interpolates to; `None` when set to undefined or deleted.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Whether the script deleted the name, so the file's value shows through.
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }
}

// Script-set values may be tokens, so Debug shows the layer and name only.
impl fmt::Debug for OverlayVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverlayVar")
            .field("layer", &self.layer)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

type Entries = BTreeMap<String, OverlayEntry>;

#[derive(Clone, Default)]
struct Layers {
    collection: Entries,
    /// Not keyed by environment: selecting, renaming and pinning leave it alone.
    global: Entries,
    /// Script edits per environment name; `None` is the state with no environment selected.
    environments: BTreeMap<Option<String>, Entries>,
    /// The environment unpinned handles read and write.
    selected: Option<String>,
    /// Bumped when a name's values are cleared or renamed away or onto, retiring handles pinned to it.
    generations: BTreeMap<String, u64>,
    /// Stores forked from this one, which follow its environment deletes and renames.
    forks: Vec<(Weak<Mutex<Layers>>, Weak<AtomicU64>)>,
    /// Runs after a delete or rename reached this store and its lock is free again, where
    /// another thread could fork it; never copied into a fork.
    #[cfg(test)]
    after_change: Option<AfterChange>,
}

#[cfg(test)]
type AfterChange = Arc<dyn Fn(&EnvOverlay) + Send + Sync>;

impl Layers {
    fn generation(&self, name: &str) -> u64 {
        self.generations.get(name).copied().unwrap_or(0)
    }

    fn retire(&mut self, name: &str) {
        *self.generations.entry(name.to_owned()).or_default() += 1;
    }
}

/// Environment, global and collection variables set by scripts; clones share one store. It never
/// writes a file, so the edits last as long as the context that holds it.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct EnvOverlay {
    layers: Arc<Mutex<Layers>>,
    /// Counts writes, so readers can tell a cached view is stale.
    revision: Arc<AtomicU64>,
    /// The environment a context's handle is fixed to, with that name's generation when
    /// pinned; `None` follows the shared selection.
    pinned: Option<(Option<String>, u64)>,
}

impl EnvOverlay {
    /// An empty overlay.
    pub fn new() -> EnvOverlay {
        EnvOverlay::default()
    }

    /// A store of its own holding a copy of these edits, on the same pin; writes to either
    /// never reach the other, but this store's environment deletes and renames reach the copy
    /// too, so a handle pinned on it afresh agrees with the files. A fork of the copy follows
    /// them only while the copy lives.
    pub(crate) fn fork(&self) -> EnvOverlay {
        let mut layers = self.lock();
        let mut copy = layers.clone();
        copy.forks.clear();
        #[cfg(test)]
        {
            copy.after_change = None;
        }
        let fork = EnvOverlay {
            layers: Arc::new(Mutex::new(copy)),
            revision: Arc::new(AtomicU64::new(self.revision())),
            pinned: self.pinned.clone(),
        };
        layers.forks.retain(|(store, _)| store.strong_count() > 0);
        (layers.forks).push((Arc::downgrade(&fork.layers), Arc::downgrade(&fork.revision)));
        fork
    }

    // Runs `change` on this store, then on its live forks and theirs, one lock at a time.
    // The forks are listed under the change's own lock, so a fork taken meanwhile copies the
    // changed layers and is not changed twice. Callers serialise changes to one store.
    fn on_each_store(&self, change: &dyn Fn(&mut Layers)) {
        let forks = {
            let mut layers = self.lock();
            change(&mut layers);
            layers.forks.clone()
        };
        self.bump();
        #[cfg(test)]
        {
            let hook = self.lock().after_change.clone();
            if let Some(hook) = hook {
                hook(self);
            }
        }
        for (layers, revision) in forks {
            if let (Some(layers), Some(revision)) = (layers.upgrade(), revision.upgrade()) {
                let fork = EnvOverlay {
                    layers,
                    revision,
                    pinned: None,
                };
                fork.on_each_store(change);
            }
        }
    }

    /// A handle on `other`'s store that keeps this handle's pin, so a context built afresh can
    /// read and write edits another context made.
    pub(crate) fn on_store_of(&self, other: &EnvOverlay) -> EnvOverlay {
        EnvOverlay {
            layers: Arc::clone(&other.layers),
            revision: Arc::clone(&other.revision),
            pinned: self.pinned.clone(),
        }
    }

    /// A handle for one context, fixed to `env` whatever is selected later. Once `env` is
    /// cleared or renamed away or onto, the handle neither reads nor writes the environment layer.
    pub(crate) fn pinned(&self, env: Option<&str>) -> EnvOverlay {
        let generation = env.map_or(0, |name| self.lock().generation(name));
        EnvOverlay {
            pinned: Some((env.map(str::to_owned), generation)),
            ..self.clone()
        }
    }

    // The environment this handle's environment layer resolves to; `None` once retired.
    fn env_key(&self, layers: &Layers) -> Option<Option<String>> {
        match &self.pinned {
            None => Some(layers.selected.clone()),
            Some((Some(name), generation)) if layers.generation(name) != *generation => None,
            Some((env, _)) => Some(env.clone()),
        }
    }

    // This handle's environment edits, if it has a live environment and any edits there.
    fn env_entries<'a>(&self, layers: &'a Layers) -> Option<&'a Entries> {
        layers.environments.get(&self.env_key(layers)?)
    }

    /// Choose the environment that unpinned handles read and write.
    pub fn select_environment(&self, name: Option<&str>) {
        self.lock().selected = name.map(str::to_owned);
        self.bump();
    }

    /// Forget the script values set under environment `name`, here and in every live fork.
    pub fn clear_environment(&self, name: &str) {
        self.on_each_store(&|layers| {
            layers.environments.remove(&Some(name.to_owned()));
            layers.retire(name);
        });
    }

    /// Move environment `from`'s values to `to`, replacing any there, and the selection with
    /// them, here and in every live fork.
    pub fn rename_environment(&self, from: &str, to: &str) {
        self.on_each_store(&|layers| {
            match layers.environments.remove(&Some(from.to_owned())) {
                Some(values) => layers.environments.insert(Some(to.to_owned()), values),
                None => layers.environments.remove(&Some(to.to_owned())),
            };
            layers.retire(from);
            layers.retire(to);
            if layers.selected.as_deref() == Some(from) {
                layers.selected = Some(to.to_owned());
            }
        });
    }

    /// Whether no script has touched the collection layer, the global layer or this handle's environment.
    pub fn is_empty(&self) -> bool {
        let layers = self.lock();
        layers.collection.is_empty()
            && layers.global.is_empty()
            && self.env_entries(&layers).is_none_or(BTreeMap::is_empty)
    }

    /// Forget every edit in every environment, the globals and the collection, so the files'
    /// values show again.
    pub fn clear(&self) {
        let mut layers = self.lock();
        layers.collection.clear();
        layers.global.clear();
        layers.environments.clear();
        drop(layers);
        self.bump();
    }

    /// Forget every edit in `layer` (for the environment layer, this handle's environment),
    /// leaving the other layers' edits in place.
    pub fn clear_layer(&self, layer: OverlayLayer) {
        let mut layers = self.lock();
        match layer {
            OverlayLayer::Collection => layers.collection.clear(),
            OverlayLayer::Global => layers.global.clear(),
            OverlayLayer::Environment => {
                if let Some(key) = self.env_key(&layers) {
                    layers.environments.remove(&key);
                }
            }
        }
        drop(layers);
        self.bump();
    }

    /// The environment-level names a script set, set to `undefined` or deleted.
    pub fn env_names(&self) -> Vec<String> {
        let layers = self.lock();
        (self.env_entries(&layers).into_iter())
            .flat_map(|map| map.keys().cloned())
            .collect()
    }

    /// The collection-level names a script set, set to `undefined` or deleted.
    pub fn collection_names(&self) -> Vec<String> {
        self.lock().collection.keys().cloned().collect()
    }

    /// Environment entries by name, then global entries, then collection entries.
    pub fn entries(&self) -> Vec<OverlayVar> {
        let layers = self.lock();
        let listed = |layer: OverlayLayer, map: &Entries| {
            (map.iter())
                .map(|(name, entry)| OverlayVar {
                    layer,
                    name: name.clone(),
                    value: match entry {
                        OverlayEntry::Set(value) => Some(crate::js::to_text(value)),
                        OverlayEntry::Undefined | OverlayEntry::Deleted => None,
                    },
                    deleted: matches!(entry, OverlayEntry::Deleted),
                })
                .collect::<Vec<_>>()
        };
        let mut out = (self.env_entries(&layers))
            .map(|map| listed(OverlayLayer::Environment, map))
            .unwrap_or_default();
        out.extend(listed(OverlayLayer::Global, &layers.global));
        out.extend(listed(OverlayLayer::Collection, &layers.collection));
        out
    }

    /// Set `name` in `layer` to a string, as a script setting a string would.
    pub fn set_text(&self, layer: OverlayLayer, name: impl Into<String>, value: impl Into<String>) {
        self.put(
            layer,
            name.into(),
            OverlayEntry::Set(Value::String(value.into())),
        );
    }

    /// Remove the entry for `name` from `layer` (for the environment layer, this handle's
    /// environment), so the file's value shows through again; returns whether it was there.
    pub fn remove(&self, layer: OverlayLayer, name: &str) -> bool {
        let mut layers = self.lock();
        let removed = match layer {
            OverlayLayer::Collection => layers.collection.remove(name).is_some(),
            OverlayLayer::Global => layers.global.remove(name).is_some(),
            OverlayLayer::Environment => match self.env_key(&layers) {
                Some(key) => {
                    (layers.environments.get_mut(&key)).is_some_and(|m| m.remove(name).is_some())
                }
                None => false,
            },
        };
        drop(layers);
        if removed {
            self.bump();
        }
        removed
    }

    pub(crate) fn put(&self, layer: OverlayLayer, name: String, entry: OverlayEntry) {
        let mut layers = self.lock();
        let map = match layer {
            OverlayLayer::Collection => &mut layers.collection,
            OverlayLayer::Global => &mut layers.global,
            OverlayLayer::Environment => match self.env_key(&layers) {
                Some(key) => layers.environments.entry(key).or_default(),
                None => return,
            },
        };
        map.insert(name, entry);
        drop(layers);
        self.bump();
    }

    /// A number that changes on every write.
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    /// `base` with the layer's edits applied: a deleted name is removed, an `undefined` one
    /// maps to `None`, every other name to its value.
    pub(crate) fn view(&self, layer: OverlayLayer, base: &Vars) -> BTreeMap<String, Option<Value>> {
        let mut view: BTreeMap<String, Option<Value>> = base
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect();
        let layers = self.lock();
        let edits = match layer {
            OverlayLayer::Collection => Some(&layers.collection),
            OverlayLayer::Global => Some(&layers.global),
            OverlayLayer::Environment => self.env_entries(&layers),
        };
        for (name, entry) in edits.into_iter().flatten() {
            match entry {
                OverlayEntry::Set(value) => view.insert(name.clone(), Some(value.clone())),
                OverlayEntry::Undefined => view.insert(name.clone(), None),
                OverlayEntry::Deleted => view.remove(name),
            };
        }
        view
    }

    // A panic while holding the lock cannot leave a map half-written, so poison is ignored.
    fn lock(&self) -> MutexGuard<'_, Layers> {
        self.layers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// Script-set values may be tokens, so Debug shows names only.
impl fmt::Debug for EnvOverlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let layers = self.lock();
        let environments: BTreeMap<_, Vec<_>> = (layers.environments.iter())
            .map(|(env, map)| (env, map.keys().collect()))
            .collect();
        f.debug_struct("EnvOverlay")
            .field("collection", &layers.collection.keys().collect::<Vec<_>>())
            .field("global", &layers.global.keys().collect::<Vec<_>>())
            .field("environments", &environments)
            .field("selected", &layers.selected)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_render_values_as_text_and_mark_undefined_and_deleted() {
        let o = EnvOverlay::new();
        o.put(
            OverlayLayer::Environment,
            "n".into(),
            OverlayEntry::Set(serde_json::json!(1.5)),
        );
        o.put(
            OverlayLayer::Environment,
            "u".into(),
            OverlayEntry::Undefined,
        );
        o.put(OverlayLayer::Collection, "d".into(), OverlayEntry::Deleted);
        let got: Vec<_> = (o.entries().iter())
            .map(|v| {
                (
                    v.layer(),
                    v.name().to_owned(),
                    v.value().map(str::to_owned),
                    v.is_deleted(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    OverlayLayer::Environment,
                    "n".to_owned(),
                    Some("1.5".to_owned()),
                    false
                ),
                (OverlayLayer::Environment, "u".to_owned(), None, false),
                (OverlayLayer::Collection, "d".to_owned(), None, true),
            ]
        );
    }

    #[test]
    fn clearing_one_layer_keeps_the_other_and_bumps_the_revision() {
        let o = EnvOverlay::new();
        o.set_text(OverlayLayer::Environment, "e", "1");
        o.set_text(OverlayLayer::Collection, "c", "2");
        let before = o.revision();
        o.clear_layer(OverlayLayer::Environment);
        assert!(o.env_names().is_empty());
        assert_eq!(o.collection_names(), ["c"]);
        assert!(o.revision() > before);
    }

    #[test]
    fn a_pinned_handle_keeps_its_environment_across_a_switch() {
        let live = EnvOverlay::new();
        live.select_environment(Some("staging"));
        let staging = live.pinned(Some("staging"));
        staging.set_text(OverlayLayer::Environment, "region", "eu");
        live.select_environment(Some("prod"));
        staging.set_text(OverlayLayer::Environment, "zone", "a");
        let base = Vars::new();
        assert_eq!(staging.view(OverlayLayer::Environment, &base).len(), 2);
        assert!(live.env_names().is_empty());
        assert!(
            live.pinned(Some("prod"))
                .view(OverlayLayer::Environment, &base)
                .is_empty()
        );
        live.select_environment(Some("staging"));
        assert_eq!(live.env_names(), ["region", "zone"]);
    }

    #[test]
    fn deleting_or_renaming_an_environment_moves_only_its_values() {
        let live = EnvOverlay::new();
        live.pinned(Some("a"))
            .set_text(OverlayLayer::Environment, "x", "1");
        live.pinned(Some("b"))
            .set_text(OverlayLayer::Environment, "y", "2");
        live.select_environment(Some("a"));
        live.rename_environment("a", "c");
        assert_eq!(live.env_names(), ["x"]);
        live.clear_environment("b");
        live.select_environment(Some("b"));
        assert!(live.env_names().is_empty());
        live.select_environment(Some("a"));
        assert!(live.env_names().is_empty());
    }

    // A handle pinned before its environment is deleted can no longer write it, so a
    // recreated environment of that name starts empty; a handle pinned after works again.
    #[test]
    fn deleting_an_environment_retires_handles_pinned_to_it() {
        let live = EnvOverlay::new();
        let old = live.pinned(Some("a"));
        old.set_text(OverlayLayer::Environment, "x", "1");
        live.clear_environment("a");
        old.set_text(OverlayLayer::Environment, "y", "2");
        old.set_text(OverlayLayer::Collection, "kept", "yes");
        let base = Vars::new();
        assert!(old.view(OverlayLayer::Environment, &base).is_empty());
        assert!(old.env_names().is_empty());
        live.select_environment(Some("a"));
        assert!(live.env_names().is_empty());
        assert_eq!(live.collection_names(), ["kept"]);
        let fresh = live.pinned(Some("a"));
        fresh.set_text(OverlayLayer::Environment, "z", "3");
        assert_eq!(live.env_names(), ["z"]);
        assert!(old.view(OverlayLayer::Environment, &base).is_empty());
    }

    // A handle pinned to a renamed environment strands nothing under the old name and
    // reads nothing from it, even when a new environment takes that name.
    #[test]
    fn renaming_an_environment_retires_handles_pinned_to_its_old_name() {
        let live = EnvOverlay::new();
        let old = live.pinned(Some("a"));
        old.set_text(OverlayLayer::Environment, "x", "1");
        live.rename_environment("a", "c");
        old.set_text(OverlayLayer::Environment, "y", "2");
        live.select_environment(Some("a"));
        assert!(live.env_names().is_empty());
        live.pinned(Some("a"))
            .set_text(OverlayLayer::Environment, "new", "1");
        assert!(old.env_names().is_empty());
        live.select_environment(Some("c"));
        assert_eq!(live.env_names(), ["x"]);
    }

    // A handle pinned to the target name before a rename belongs to the environment that
    // name used to mean, so it must not read or write the values moved in from `from`.
    #[test]
    fn renaming_an_environment_retires_handles_pinned_to_its_new_name() {
        let live = EnvOverlay::new();
        let stale = live.pinned(Some("c"));
        live.pinned(Some("a"))
            .set_text(OverlayLayer::Environment, "x", "1");
        live.rename_environment("a", "c");
        let base = Vars::new();
        assert!(stale.view(OverlayLayer::Environment, &base).is_empty());
        assert!(stale.env_names().is_empty());
        stale.set_text(OverlayLayer::Environment, "y", "2");
        live.select_environment(Some("c"));
        assert_eq!(live.env_names(), ["x"]);
    }

    // A fork follows a rename but its own writes stay its own; a dropped fork is skipped.
    #[test]
    fn a_fork_follows_renames_and_a_dropped_fork_is_skipped() {
        let live = EnvOverlay::new();
        let fork = live.pinned(Some("a")).fork();
        fork.set_text(OverlayLayer::Environment, "x", "1");
        drop(live.fork());
        live.rename_environment("a", "c");
        assert_eq!(fork.pinned(Some("c")).env_names(), ["x"]);
        assert!(live.pinned(Some("c")).env_names().is_empty());
        assert_eq!(live.lock().forks.len(), 2);
        drop(live.fork());
        assert_eq!(live.lock().forks.len(), 2, "the dead fork was pruned");
    }

    // A fork taken just after a rename reached the store already holds the renamed values,
    // so the rename must not reach it a second time.
    #[test]
    fn a_fork_taken_during_a_rename_gets_it_once() {
        let live = EnvOverlay::new();
        live.pinned(Some("a"))
            .set_text(OverlayLayer::Environment, "x", "1");
        let late: Arc<Mutex<Option<EnvOverlay>>> = Arc::default();
        let slot = Arc::clone(&late);
        live.lock().after_change = Some(Arc::new(move |store: &EnvOverlay| {
            let mut slot = slot.lock().unwrap();
            if slot.is_none() {
                *slot = Some(store.fork());
            }
        }));
        live.rename_environment("a", "c");
        let late = late.lock().unwrap().take().expect("the hook forked");
        assert_eq!(late.pinned(Some("c")).env_names(), ["x"]);
        let generation = |o: &EnvOverlay| o.lock().generation("c");
        assert_eq!(generation(&late), generation(&live));
    }

    #[test]
    fn debug_shows_no_values() {
        let o = EnvOverlay::new();
        o.set_text(OverlayLayer::Environment, "token", "s3cr3t-value");
        let shown = format!("{:?} {:?}", o, o.entries());
        assert!(
            shown.contains("token") && !shown.contains("s3cr3t-value"),
            "{shown}"
        );
    }
}
