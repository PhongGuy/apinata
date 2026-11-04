//! `check_collection`: the core's static check, returned as the document `apinata check --json` prints.

use std::sync::Arc;

use apinata_core::{CheckOptions, Collection, HintShell, SecretStore, check_collection};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

/// Arguments of `check_collection`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckArgs {
    /// Check only this environment. Default: every environment, and none.
    #[serde(default)]
    pub env: Option<String>,
}

/// The versioned report for the collection on disk; an unknown environment is the core's message.
/// A secret that has a value in `store` is not reported; the values themselves are never returned.
pub(crate) fn check(
    c: &Collection,
    args: &CheckArgs,
    store: Arc<dyn SecretStore>,
) -> Result<Map<String, Value>, String> {
    let mut opts = CheckOptions::default()
        .hint_shell(HintShell::Mcp)
        .secrets(store);
    if let Some(name) = &args.env {
        opts = opts.env(name);
    }
    let report = check_collection(c.root(), &opts).map_err(|e| e.to_string())?;
    match report.to_json_versioned() {
        Value::Object(map) => Ok(map),
        other => Err(format!("unexpected report shape: {other}")),
    }
}
