//! The app-wide GraphQL schema store: one schema per raw request URL, shared by every tab that
//! targets it, with generation numbers so an older load never overwrites a newer one. With a
//! disk cache, a URL's first use reads its kept schema.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use apinata_core::Schema;
use gpui_kit::{App, Global};

use crate::schema_cache::SchemaCache;

/// Where the schema of one URL stands.
#[derive(Debug, Clone)]
pub enum SchemaState {
    /// A load is in flight.
    Loading,
    /// The schema the last load produced.
    Loaded(Arc<Schema>),
    /// Why the last load failed.
    Failed(String),
}

/// Schemas keyed by raw request URL. One counter numbers every load across all URLs.
#[derive(Default)]
pub struct SchemaStore {
    entries: HashMap<String, (u64, SchemaState)>,
    next: u64,
    cache: Option<SchemaCache>,
    probed: HashSet<String>,
}

impl Global for SchemaStore {}

impl SchemaStore {
    /// An empty store whose schemas are kept in, and first read from, `cache`.
    pub fn with_cache(cache: SchemaCache) -> SchemaStore {
        SchemaStore {
            cache: Some(cache),
            ..SchemaStore::default()
        }
    }

    /// The disk cache, when the store has one.
    pub fn cache(&self) -> Option<&SchemaCache> {
        self.cache.as_ref()
    }

    /// Claims the one disk read of `url`: the cache when there is one, `url` has no entry and
    /// was never probed.
    pub fn probe(&mut self, url: &str) -> Option<SchemaCache> {
        let cache = self.cache.as_ref()?;
        if self.entries.contains_key(url) || !self.probed.insert(url.to_owned()) {
            return None;
        }
        Some(cache.clone())
    }

    /// Stores `schema`, read from disk, for `url` unless a load has started for it; true if stored.
    pub fn restore(&mut self, url: &str, schema: Schema) -> bool {
        if self.entries.contains_key(url) {
            return false;
        }
        self.next += 1;
        let state = SchemaState::Loaded(Arc::new(schema));
        self.entries.insert(url.to_owned(), (self.next, state));
        true
    }

    /// The state of `url`, or `None` when it was never loaded.
    pub fn state(&self, url: &str) -> Option<&SchemaState> {
        self.entries.get(url).map(|(_, state)| state)
    }

    /// Starts a load of `url`: its state becomes `Loading` and the new generation is returned.
    pub fn begin(&mut self, url: &str) -> u64 {
        self.next += 1;
        self.entries
            .insert(url.to_owned(), (self.next, SchemaState::Loading));
        self.next
    }

    /// Stores `result` for `url` only when `generation` is still its newest load; true if stored.
    pub fn finish(&mut self, url: &str, generation: u64, result: Result<Schema, String>) -> bool {
        match self.entries.get_mut(url) {
            Some((current, state)) if *current == generation => {
                *state = match result {
                    Ok(schema) => SchemaState::Loaded(Arc::new(schema)),
                    Err(message) => SchemaState::Failed(message),
                };
                true
            }
            _ => false,
        }
    }
}

/// The state of `url` in the app's store; `None` without the store or an entry.
pub fn state(cx: &App, url: &str) -> Option<SchemaState> {
    cx.try_global::<SchemaStore>()?.state(url).cloned()
}
