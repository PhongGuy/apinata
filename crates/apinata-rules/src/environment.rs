//! The environment rule: an environment file must be readable by Apinata and must not carry a
//! value for any variable marked secret.

use potluck_kind::{Code, Finding};
use yaml_serde::Value;

use apinata_format::{ReadError, ReadErrorKind};

use crate::{ENV_UNPARSABLE, SECRET_VALUE};

/// The environment rule on `bytes`, reported at `path`: `env_unparsable` when
/// `apinata_format::read_mapping` refuses the bytes or a merge key cannot be folded,
/// `secret_value` when some mapping has `secret: true` and a `value` that is not null once
/// merges are folded in, else `None`. The detail names the secrets or the refusal, never a
/// value and never the reader's own message.
pub(crate) fn environment_rule(path: &str, bytes: &[u8]) -> Option<Finding> {
    let root = match apinata_format::read_mapping(bytes) {
        Err(e) => return Some(unparsable(path, &refusal_detail(&e))),
        Ok(root) => root,
    };
    // Apinata's reader never folds merges; the rule does, so a merge cannot carry a value into
    // a secret unseen.
    let mut doc = Value::Mapping(root);
    if !fold_merges(&mut doc) {
        return Some(unparsable(path, "a merge key that cannot be applied"));
    }
    let mut names = Names::default();
    if !valued_secrets(&doc, &mut names) {
        return None;
    }
    let finding = Finding::new(Code::SecretValue, SECRET_VALUE, path);
    Some(if names.list.is_empty() {
        finding
    } else {
        finding.with_detail(&names.list.join(", "))
    })
}

/// An `env_unparsable` finding at `path` carrying `detail`.
pub(crate) fn unparsable(path: &str, detail: &str) -> Finding {
    Finding::new(Code::SecretValue, ENV_UNPARSABLE, path).with_detail(detail)
}

// A fixed phrase per refusal, or the parser's position: the reader's message may quote the
// file, so it never reaches a detail.
fn refusal_detail(e: &ReadError) -> String {
    match (e.kind(), e.position()) {
        (ReadErrorKind::NotUtf8, _) => "not UTF-8".to_owned(),
        (ReadErrorKind::TooManyValues, _) => "too many values once aliases expand".to_owned(),
        (ReadErrorKind::UnfollowableAliases, _) => {
            "aliases in a syntax the check cannot follow".to_owned()
        }
        (ReadErrorKind::Syntax, Some((line, column))) => format!("line {line}, column {column}"),
        (ReadErrorKind::Tag, _) => "YAML tags are not supported".to_owned(),
        (ReadErrorKind::NotMapping, _) => "the root is not a mapping".to_owned(),
        _ => "not valid YAML".to_owned(),
    }
}

// Rounds of `apply_merge` allowed before a leftover `<<` counts as unparsable; the
// parser's nesting limit, since each round folds at least one level.
const MERGE_ROUNDS: usize = 128;

// One `apply_merge` folds a single level, so a `<<` carried in by a merge survives it;
// repeat until none is left. `false` on a merge error, on the round bound, and at once for a
// `<<` inside a key, which `apply_merge` never folds since it walks values only.
fn fold_merges(doc: &mut Value) -> bool {
    if merge_in_a_key(doc) {
        return false;
    }
    for _ in 0..MERGE_ROUNDS {
        if !has_merge_key(doc) {
            return true;
        }
        if doc.apply_merge().is_err() {
            return false;
        }
    }
    !has_merge_key(doc)
}

fn has_merge_key(node: &Value) -> bool {
    match node {
        Value::Mapping(map) => {
            map.contains_key("<<")
                || map
                    .iter()
                    .any(|(k, v)| has_merge_key(k) || has_merge_key(v))
        }
        Value::Sequence(items) => items.iter().any(has_merge_key),
        _ => false,
    }
}

// Hands each mapping key to `has_merge_key` once and recurses only through values and items,
// so every node is visited a bounded number of times.
fn merge_in_a_key(node: &Value) -> bool {
    match node {
        Value::Mapping(map) => map
            .iter()
            .any(|(k, v)| has_merge_key(k) || merge_in_a_key(v)),
        Value::Sequence(items) => items.iter().any(merge_in_a_key),
        _ => false,
    }
}

// The most bytes a detail's joined names may take; Potluck keeps no more than this of a detail.
const DETAIL_BYTES: usize = 200;

/// The valued secrets' names for the detail, each once, in walk order, joined no longer than
/// `DETAIL_BYTES`. The first name that does not fit closes the list, which keeps it short and
/// so keeps the walk linear however many secrets a file holds.
#[derive(Default)]
struct Names {
    list: Vec<String>,
    bytes: usize,
    full: bool,
}

impl Names {
    fn add(&mut self, name: &str) {
        if self.full || self.list.iter().any(|n| n == name) {
            return;
        }
        let bytes = self.bytes + if self.list.is_empty() { 0 } else { 2 } + name.len();
        if bytes > DETAIL_BYTES {
            self.full = true;
            return;
        }
        self.bytes = bytes;
        self.list.push(name.to_owned());
    }
}

/// Whether any mapping under `node`, keys included, pairs `secret: true` with a non-null
/// `value`, adding each such mapping's string `name` to `names`. The
/// reader refuses tags, so no tagged node needs looking through, and its depth limit bounds
/// the recursion.
fn valued_secrets(node: &Value, names: &mut Names) -> bool {
    match node {
        Value::Mapping(map) => {
            let secret = map.get("secret") == Some(&Value::Bool(true));
            let valued = map.get("value").is_some_and(|v| !v.is_null());
            let here = secret && valued;
            if here && let Some(Value::String(name)) = map.get("name") {
                names.add(name);
            }
            // Visit every entry, not just up to the first hit, so every name is collected.
            let mut found = here;
            for (k, v) in map {
                found |= valued_secrets(k, names);
                found |= valued_secrets(v, names);
            }
            found
        }
        Value::Sequence(items) => {
            let mut found = false;
            for item in items {
                found |= valued_secrets(item, names);
            }
            found
        }
        _ => false,
    }
}
