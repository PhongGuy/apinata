//! Request snapshots: a request's mapping carried as JSON, with literal credentials blanked.

use super::Collection;
use crate::edit::auth::{CREDENTIAL_PATHS, PARAM_ROWS};
use crate::item_path::ItemPath;
use crate::request::RequestFile;
use crate::url_password::{is_lone_placeholder, url_without_password};
use std::fmt;
use yaml_serde::{Mapping, Value};

/// A request's mapping as it stood at one moment, safe to store beside its response.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct DraftSnapshot {
    root: Mapping,
}

impl fmt::Debug for DraftSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DraftSnapshot(..)")
    }
}

/// A snapshot that could not be read back.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The text is not a JSON object.
    #[error("unreadable request snapshot: {0}")]
    Parse(String),
}

impl DraftSnapshot {
    /// The snapshot as JSON; `"null"` (which [`DraftSnapshot::from_json`] refuses) when the
    /// mapping has a key JSON cannot hold.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.root).unwrap_or_else(|_| "null".to_owned())
    }

    /// Read a snapshot written by [`DraftSnapshot::to_json`]; `Parse` unless it is a JSON object
    /// holding the `http` or `graphql` mapping its `info.type` names (absent means `http`).
    pub fn from_json(json: &str) -> Result<DraftSnapshot, SnapshotError> {
        let root: Mapping =
            serde_json::from_str(json).map_err(|e| SnapshotError::Parse(e.to_string()))?;
        let key = match root.get("info").and_then(|i| i.get("type")) {
            None => "http",
            Some(Value::String(k)) if k == "http" || k == "graphql" => k.as_str(),
            Some(_) => {
                return Err(SnapshotError::Parse(
                    "not an http or graphql request".into(),
                ));
            }
        };
        if !root.get(key).is_some_and(Value::is_mapping) {
            return Err(SnapshotError::Parse(format!("no `{key}` mapping")));
        }
        Ok(DraftSnapshot { root })
    }

    /// Rewrites every string value in place, never mapping keys and never the format tokens held
    /// under a `type` or `method` key (used for secret scrubbing).
    pub fn map_strings(&mut self, mut f: impl FnMut(&str) -> String) {
        for (_, value) in self.root.iter_mut() {
            visit_strings(value, &mut |s| *s = f(s));
        }
    }

    pub(super) fn mapping(&self) -> &Mapping {
        &self.root
    }

    pub(crate) fn root_mut(&mut self) -> &mut Mapping {
        &mut self.root
    }

    /// The key of the request mapping, `http` or `graphql`, as `info.type` names it.
    pub(crate) fn request_key(&self) -> &'static str {
        match self.root.get("info").and_then(|i| i.get("type")) {
            Some(Value::String(k)) if k == "graphql" => "graphql",
            _ => "http",
        }
    }
}

impl RequestFile {
    /// The request's current mapping, edits included, with literal credentials blanked: the auth
    /// fields, and a password in the url's userinfo. A lone `{{var}}` placeholder is kept.
    pub fn snapshot(&self) -> DraftSnapshot {
        self.snapshot_and_blanks().0
    }

    /// [`Self::snapshot`] and whether it blanked any credential, from one clone of the request.
    pub fn snapshot_and_blanks(&self) -> (DraftSnapshot, bool) {
        let mut root = self.document().mapping().clone();
        if let Some(Value::Mapping(auth)) = root
            .get_mut(self.root_key())
            .and_then(|r| r.get_mut("auth"))
        {
            blank_credentials(auth);
        }
        if let Some(Value::String(url)) =
            root.get_mut(self.root_key()).and_then(|r| r.get_mut("url"))
        {
            *url = url_without_password(url);
        }
        let blanked = root != *self.document().mapping();
        (DraftSnapshot { root }, blanked)
    }

    /// Whether [`Self::snapshot`] blanked any credential.
    pub fn blanks_credentials(&self) -> bool {
        self.snapshot_and_blanks().1
    }
}

impl Collection {
    /// A snapshot of the request at `item`, a draft included; `None` when it is not a request.
    pub fn request_snapshot(&self, item: &ItemPath) -> Option<DraftSnapshot> {
        self.request(item).map(RequestFile::snapshot)
    }
}

/// Calls `f` on each string under `value`, never mapping keys and never the format tokens
/// held under a `type` or `method` key.
pub(crate) fn visit_strings(value: &mut Value, f: &mut impl FnMut(&mut String)) {
    match value {
        Value::String(s) => f(s),
        Value::Sequence(items) => items.iter_mut().for_each(|v| visit_strings(v, f)),
        Value::Mapping(map) => {
            for (key, v) in map.iter_mut() {
                // `type` and `method` hold format tokens (request type, body mode, HTTP method),
                // not user text; rewriting one would make the snapshot unreadable.
                let token = matches!(key.as_str(), Some("type" | "method")) && v.is_string();
                if !token {
                    visit_strings(v, f);
                }
            }
        }
        Value::Tagged(tagged) => visit_strings(&mut tagged.value, f),
        _ => {}
    }
}

// Blank each credential scalar, and each OAuth 2 extra parameter row's value, unless it is a
// single `{{var}}` placeholder: a template names a secret without holding it.
fn blank_credentials(auth: &mut Mapping) {
    for path in CREDENTIAL_PATHS {
        if let Some(value) = value_at(auth, path) {
            blank(value);
        }
    }
    if let Some(Value::Mapping(stages)) = auth.get_mut(PARAM_ROWS) {
        for (_, rows) in stages.iter_mut() {
            let Value::Sequence(rows) = rows else {
                continue;
            };
            for row in rows {
                if let Some(value) = row.get_mut("value") {
                    blank(value);
                }
            }
        }
    }
}

fn value_at<'a>(map: &'a mut Mapping, path: &[&str]) -> Option<&'a mut Value> {
    let (first, rest) = path.split_first()?;
    rest.iter()
        .try_fold(map.get_mut(*first)?, |v, k| v.get_mut(*k))
}

// Only scalars are credentials; a null holds nothing, and mappings and sequences are left alone.
fn blank(value: &mut Value) {
    let keep = match value {
        Value::String(s) => is_lone_placeholder(s),
        Value::Bool(_) | Value::Number(_) => false,
        _ => true,
    };
    if !keep {
        *value = Value::String(String::new());
    }
}
