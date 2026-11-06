//! The app-wide GraphQL schema store: one schema per raw request URL, shared by every tab that
//! targets it, with generation numbers so an older load never overwrites a newer one. With a
//! disk cache, a URL's first use reads its kept schema. Each loaded schema also gets one
//! compiled form for query validation, built off the UI thread and shared by every tab.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use apinata_core::{CompiledSchema, Schema};
use gpui_kit::{App, AppContext as _, Global};

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

/// What a query check can use for one URL.
#[derive(Debug, Clone)]
pub enum Lookup {
    /// No entry, or the last load failed.
    NoSchema,
    /// The schema is loading, or its compiled form is building.
    Pending,
    /// The compiled schema of the load numbered `generation`.
    Ready {
        /// The load the schema came from.
        generation: u64,
        /// The shared compiled schema.
        schema: Arc<CompiledSchema>,
    },
    /// The load numbered `generation` produced a schema that could not be compiled.
    Unusable {
        /// The load the schema came from.
        generation: u64,
        /// Why it could not be compiled.
        reason: String,
    },
}

/// Identifies the schema a check ran against, so a changed schema can trigger a new check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKey {
    /// Syntax only: no schema, or its load failed.
    NoSchema,
    /// The compiled schema of this generation.
    Ready(u64),
    /// The uncompilable schema of this generation.
    Unusable(u64),
}

impl Lookup {
    /// The key of this lookup, or `None` while it is pending.
    pub fn key(&self) -> Option<CheckKey> {
        match self {
            Lookup::NoSchema => Some(CheckKey::NoSchema),
            Lookup::Pending => None,
            Lookup::Ready { generation, .. } => Some(CheckKey::Ready(*generation)),
            Lookup::Unusable { generation, .. } => Some(CheckKey::Unusable(*generation)),
        }
    }
}

// The compiled form of one load, tagged in the store with that load's generation.
enum Slot {
    Building,
    Ready(Arc<CompiledSchema>),
    Unusable(String),
}

/// Schemas keyed by raw request URL. One counter numbers every load across all URLs.
#[derive(Default)]
pub struct SchemaStore {
    entries: HashMap<String, (u64, SchemaState)>,
    compiled: HashMap<String, (u64, Slot)>,
    sources: HashMap<String, String>,
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
        self.forget_derived(url);
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
        self.forget_derived(url);
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

    /// Starts a load of `url` from the schema file `file`, which `source` then names.
    pub fn begin_file(&mut self, url: &str, file: &Path) -> u64 {
        let generation = self.begin(url);
        self.sources
            .insert(url.to_owned(), file.display().to_string());
        generation
    }

    /// The schema file the current load of `url` came from, if it came from one.
    pub fn source(&self, url: &str) -> Option<&str> {
        self.sources.get(url).map(String::as_str)
    }

    /// What a query check of `url` can use right now.
    pub fn lookup(&self, url: &str) -> Lookup {
        let Some((generation, state)) = self.entries.get(url) else {
            return Lookup::NoSchema;
        };
        match state {
            SchemaState::Loading => Lookup::Pending,
            SchemaState::Failed(_) => Lookup::NoSchema,
            SchemaState::Loaded(_) => match self.compiled.get(url) {
                Some((built, Slot::Ready(schema))) if built == generation => Lookup::Ready {
                    generation: *generation,
                    schema: schema.clone(),
                },
                Some((built, Slot::Unusable(reason))) if built == generation => Lookup::Unusable {
                    generation: *generation,
                    reason: reason.clone(),
                },
                _ => Lookup::Pending,
            },
        }
    }

    /// Claims the one compile of `url`'s loaded schema: its generation and schema, or `None`
    /// when nothing is loaded or this generation was already claimed.
    pub fn claim_build(&mut self, url: &str) -> Option<(u64, Arc<Schema>)> {
        if !self.needs_build(url) {
            return None;
        }
        let (generation, SchemaState::Loaded(schema)) = self.entries.get(url)? else {
            return None;
        };
        self.compiled
            .insert(url.to_owned(), (*generation, Slot::Building));
        Some((*generation, schema.clone()))
    }

    /// Stores a compile `result` for `url` only when `generation` is still its newest load;
    /// true if stored.
    pub fn store_compiled(
        &mut self,
        url: &str,
        generation: u64,
        result: Result<CompiledSchema, String>,
    ) -> bool {
        match self.entries.get(url) {
            Some((current, SchemaState::Loaded(_))) if *current == generation => {
                let slot = match result {
                    Ok(schema) => Slot::Ready(Arc::new(schema)),
                    Err(reason) => Slot::Unusable(reason),
                };
                self.compiled.insert(url.to_owned(), (generation, slot));
                true
            }
            _ => false,
        }
    }

    // A loaded schema whose generation has no compiled slot yet.
    fn needs_build(&self, url: &str) -> bool {
        match self.entries.get(url) {
            Some((generation, SchemaState::Loaded(_))) => {
                !matches!(self.compiled.get(url), Some((built, _)) if built == generation)
            }
            _ => false,
        }
    }

    // A new load makes the old compiled form and file name stale; dropping them frees the
    // compiled schema once no running check holds it.
    fn forget_derived(&mut self, url: &str) {
        self.compiled.remove(url);
        self.sources.remove(url);
    }
}

/// The state of `url` in the app's store; `None` without the store or an entry.
pub fn state(cx: &App, url: &str) -> Option<SchemaState> {
    cx.try_global::<SchemaStore>()?.state(url).cloned()
}

/// The compiled schema for `url`, claiming and starting its background build on first use.
pub fn compiled(url: &str, cx: &mut App) -> Lookup {
    // Reads go through `try_global`: `default_global` notifies observers, so only a claim,
    // at most once per generation, may write.
    let Some(store) = cx.try_global::<SchemaStore>() else {
        return Lookup::NoSchema;
    };
    if !store.needs_build(url) {
        return store.lookup(url);
    }
    let Some((generation, schema)) = cx.default_global::<SchemaStore>().claim_build(url) else {
        return Lookup::Pending;
    };
    let build = cx.background_spawn(async move {
        // A panic must still end the build, or the slot would stay pending forever.
        catch_unwind(AssertUnwindSafe(|| {
            CompiledSchema::new(&schema).map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|_| Err("it could not be prepared".to_owned()))
    });
    let url = url.to_owned();
    cx.spawn(async move |cx| {
        let result = build.await;
        cx.update(|cx| {
            cx.default_global::<SchemaStore>()
                .store_compiled(&url, generation, result)
        });
    })
    .detach();
    Lookup::Pending
}

/// `SchemaStore::source` in the app's store.
pub fn source(cx: &App, url: &str) -> Option<String> {
    cx.try_global::<SchemaStore>()?
        .source(url)
        .map(str::to_owned)
}
