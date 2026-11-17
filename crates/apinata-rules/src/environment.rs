//! The environment rule: an environment file must be readable by Apinata and must not carry a
//! value for any variable marked secret.

use potluck_kind::{Code, Finding};
use yaml_serde::Value;

use crate::{ENV_UNPARSABLE, SECRET_VALUE};

/// The environment rule on `bytes`, reported at `path`: `env_unparsable` when
/// `apinata_format::read_mapping` refuses the bytes or a merge key cannot be folded,
/// `secret_value` when some mapping has `secret: true` and a `value` that is not null once
/// merges are folded in, else `None`.
pub(crate) fn environment_rule(path: &str, bytes: &[u8]) -> Option<Finding> {
    let rule = match apinata_format::read_mapping(bytes) {
        Err(_) => ENV_UNPARSABLE,
        Ok(root) => {
            // Apinata's reader never folds merges; the rule does, so a merge cannot carry a
            // value into a secret unseen.
            let mut doc = Value::Mapping(root);
            if !fold_merges(&mut doc) {
                ENV_UNPARSABLE
            } else if has_valued_secret(&doc) {
                SECRET_VALUE
            } else {
                return None;
            }
        }
    };
    Some(Finding::new(Code::SecretValue, rule, path))
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

/// Whether any mapping under `node`, keys included, pairs `secret: true` with a non-null
/// `value`. The reader refuses tags, so no tagged node needs looking through, and its depth
/// limit bounds the recursion.
fn has_valued_secret(node: &Value) -> bool {
    match node {
        Value::Mapping(map) => {
            let secret = map.get("secret") == Some(&Value::Bool(true));
            let valued = map.get("value").is_some_and(|v| !v.is_null());
            (secret && valued)
                || map
                    .iter()
                    .any(|(k, v)| has_valued_secret(k) || has_valued_secret(v))
        }
        Value::Sequence(items) => items.iter().any(has_valued_secret),
        _ => false,
    }
}
