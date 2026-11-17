//! The environment rule: an environment file must be readable by Apinata and must not carry a
//! value for any variable marked secret.

use potluck_kind::{Code, Finding};
use yaml_serde::Value;

use crate::{ENV_UNPARSABLE, SECRET_VALUE};

/// The environment rule on `bytes`, reported at `path`: `env_unparsable` when
/// `apinata_format::read_mapping` refuses the bytes, `secret_value` when some mapping has
/// `secret: true` and a `value` that is not null, else `None`.
pub(crate) fn environment_rule(path: &str, bytes: &[u8]) -> Option<Finding> {
    let rule = match apinata_format::read_mapping(bytes) {
        Err(_) => ENV_UNPARSABLE,
        Ok(root) => {
            if !has_valued_secret(&Value::Mapping(root)) {
                return None;
            }
            SECRET_VALUE
        }
    };
    Some(Finding::new(Code::SecretValue, rule, path))
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
