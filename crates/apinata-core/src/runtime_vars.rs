//! The runtime variable scope: values captured after a response, shared by every clone.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::Value;

use crate::js;

/// Variables set by post-response variables; clones share one store.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct RuntimeVars {
    /// `None` is a variable set to `undefined`, which still shadows lower layers.
    store: Arc<Mutex<BTreeMap<String, Option<Value>>>>,
    /// Counts writes, so readers can tell a cached view is stale.
    revision: Arc<AtomicU64>,
    /// Values scripts read from sensitive sources; they outlive `clear`, since a copy of one may
    /// still sit in a variable of another scope.
    tainted: Arc<Mutex<Vec<String>>>,
    /// Names whose values a person marked sensitive; forgetting the variable forgets the mark.
    sensitive: Arc<Mutex<BTreeSet<String>>>,
}

impl RuntimeVars {
    /// An empty store.
    pub fn new() -> RuntimeVars {
        RuntimeVars::default()
    }

    /// Set `name` to a string, as setting it from a script with a string would.
    pub fn set_text(&self, name: impl Into<String>, value: impl Into<String>) {
        self.set(name.into(), Some(Value::String(value.into())));
    }

    /// Every variable by name; `value()` is the interpolation text, `None` for undefined.
    pub fn entries(&self) -> Vec<RuntimeVar> {
        self.lock()
            .iter()
            .map(|(name, value)| RuntimeVar {
                name: name.clone(),
                value: value.as_ref().map(js::to_text),
            })
            .collect()
    }

    /// How many variables are set, `undefined` ones included.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no variable is set.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Forget every variable.
    pub fn clear(&self) {
        self.lock().clear();
        self.sensitive_set().clear();
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    /// Marks the variable `name` as holding a sensitive value, so a view masks it whatever its
    /// text. The mark lasts until `unset(name)` or `clear`.
    pub fn mark_sensitive(&self, name: &str) {
        self.sensitive_set().insert(name.to_owned());
    }

    /// Whether `name` was marked by `mark_sensitive` and not forgotten since.
    pub fn is_sensitive(&self, name: &str) -> bool {
        self.sensitive_set().contains(name)
    }

    fn sensitive_set(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.sensitive
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Forget `name` alone; returns whether it was set.
    pub fn unset(&self, name: &str) -> bool {
        let was_set = self.lock().contains_key(name);
        self.remove(name);
        was_set
    }

    pub(crate) fn set(&self, name: String, value: Option<Value>) {
        self.lock().insert(name, value);
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    /// Forget `name`, so lower layers show through again.
    pub(crate) fn remove(&self, name: &str) {
        self.sensitive_set().remove(name);
        if self.lock().remove(name).is_some() {
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A number that changes on every `set`, `remove` and `clear`.
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// Remembers the non-empty `values` as sensitive for the store's lifetime, without repeats.
    /// A value equal to a tainted one is then secret whether copied or coincidental (fails closed).
    pub(crate) fn taint(&self, values: &[String]) {
        let mut tainted = self.tainted.lock().unwrap_or_else(PoisonError::into_inner);
        for value in values {
            if !value.is_empty() && !tainted.contains(value) {
                tainted.push(value.clone());
            }
        }
    }

    /// Whether `text` contains a value a script read as sensitive, without copying the list.
    pub(crate) fn taints(&self, text: &str) -> bool {
        let tainted = self.tainted.lock().unwrap_or_else(PoisonError::into_inner);
        tainted.iter().any(|t| text.contains(t.as_str()))
    }

    /// Every value a script has read as sensitive through this store, in first-read order.
    pub fn tainted(&self) -> Vec<String> {
        let tainted = self.tainted.lock().unwrap_or_else(PoisonError::into_inner);
        tainted.clone()
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<String, Option<Value>> {
        self.lock().clone()
    }

    // A panic while holding the lock cannot leave the map half-written, so poison is ignored.
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Option<Value>>> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// Captured values may be tokens, so Debug shows names only.
impl fmt::Debug for RuntimeVars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.lock().keys()).finish()
    }
}

/// One runtime variable as listed by [`RuntimeVars::entries`].
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuntimeVar {
    name: String,
    value: Option<String>,
}

impl RuntimeVar {
    /// The variable's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The text `{{name}}` interpolates to, or `None` when the variable is `undefined`.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

impl fmt::Debug for RuntimeVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeVar")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
