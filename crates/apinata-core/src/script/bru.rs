//! The `bru` object: one host dispatch function over JSON strings, wrapped by `bru.js`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use rquickjs::Function;
use serde_json::{Value, json};

use super::engine::Installer;
use super::host::HostState;
use crate::assertion::Verdict;
use crate::context::{VarContext, json_strings};
use crate::env_overlay::{OverlayEntry, OverlayLayer};
use crate::interpolate::{Vars, interpolate_lenient};
use crate::request::RequestFile;

const BRU_JS: &str = include_str!("bru.js");
const PROCESS_ENV_JS: &str = include_str!("process_env.js");

/// The environment layer's selected-name key, which scripts may read but never delete.
const ENV_NAME_KEY: &str = "__name__";

/// Installs `globalThis.bru` for one phase over `state`.
pub(crate) fn installer(state: Arc<HostState>) -> Installer {
    Box::new(move |ctx| {
        let host = move |op: String, args: String| dispatch(&state, &op, &args);
        ctx.globals()
            .set("__bru", Function::new(ctx.clone(), host)?)?;
        ctx.eval::<(), _>(BRU_JS)
    })
}

/// Wraps an expression scope's `process.env` so each value joins `state.sensitive` only when
/// an expression reads it. Runs after the scope's variables are set.
pub(crate) fn process_env_installer(state: Arc<HostState>) -> Installer {
    Box::new(move |ctx| {
        let read = move |name: String| {
            if let Some(value) = state.ctx.process_env().and_then(|env| env.get(&name)) {
                note_sensitive(&state, value);
            }
        };
        ctx.globals()
            .set("__readEnv", Function::new(ctx.clone(), read)?)?;
        ctx.eval::<(), _>(PROCESS_ENV_JS)
    })
}

/// A variable store a script names: the first argument of every scoped operation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Runtime,
    Env,
    Global,
    Collection,
    Folder,
    Request,
}

/// Runs operation `op` with the JSON array `args`. The answer is `{"v": value}`, `{}` for
/// `undefined`, `{"p": [[name, value] | [name]]}` for a whole store, or `{"e": message}`.
fn dispatch(state: &HostState, op: &str, args: &str) -> String {
    // Arguments that fail to parse are reported to the script, not treated as no arguments.
    let args: Vec<Value> = match serde_json::from_str(args) {
        Ok(args) => args,
        Err(e) => return json!({ "e": format!("invalid arguments for {op}: {e}") }).to_string(),
    };
    let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default();
    let ctx = &state.ctx;
    let answer = match op {
        "cwd" => some(ctx.root().to_string_lossy().into_owned().into()),
        "collectionName" => some(ctx.collection_name().into()),
        "envName" => defined(ctx.env_view().remove(ENV_NAME_KEY).flatten()),
        "processEnv" => {
            let value = ctx.process_env().and_then(|env| env.get(text(0))).cloned();
            if let Some(value) = &value {
                note_sensitive(state, value);
            }
            defined(value)
        }
        "oauth2Var" => {
            let found = ctx.oauth2_var(text(0));
            if let Some((value, true)) = &found {
                note_sensitive(state, value);
            }
            defined(found.map(|(value, _)| value))
        }
        "interpolate" => some(interpolate_text(state, text(0)).into()),
        "interpolateJson" => match serde_json::from_str(&interpolate_text(state, text(0))) {
            Ok(value) => some(value),
            Err(e) => json!({ "e": e.to_string() }),
        },
        "skipRequest" | "stopExecution" | "setNextRequest" => {
            let mut control = state.control.lock().unwrap_or_else(PoisonError::into_inner);
            match op {
                "skipRequest" => control.skip = true,
                "stopExecution" => control.stop = true,
                _ => control.next = Some(args.first().and_then(Value::as_str).map(str::to_owned)),
            }
            json!({})
        }
        "assertions" => some(assertion_results(state)),
        _ => match scope(text(0)) {
            Some(scope) => scoped(state, op, scope, text(1), args.get(2)),
            None => json!({ "e": format!("unknown bru operation {op}") }),
        },
    };
    answer.to_string()
}

/// The request's assertions as `getAssertionResults` lists them; empty before they have run.
fn assertion_results(state: &HostState) -> Value {
    let outcomes = state
        .assertions
        .get()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let rows = outcomes.iter().map(|outcome| {
        let a = outcome.assertion();
        let operand = a.value().unwrap_or_default();
        let mut row = json!({
            "lhsExpr": a.expression(),
            "rhsExpr": a.bruno_value(),
            "operator": a.operator(),
            "rhsOperand": operand,
        });
        match outcome.verdict() {
            Verdict::Pass => row["status"] = "pass".into(),
            Verdict::Fail { actual, detail } => {
                row["status"] = "fail".into();
                row["error"] = detail
                    .clone()
                    .unwrap_or_else(|| format!("expected {actual} to {} {operand}", a.operator()))
                    .into();
            }
            Verdict::Skipped { reason } => {
                row["status"] = "skipped".into();
                row["error"] = reason.as_str().into();
            }
        }
        row
    });
    Value::Array(rows.collect())
}

/// The operations on one store: `has`, `get`, `set`, `delete`, `deleteAll` and `all`.
fn scoped(state: &HostState, op: &str, scope: Scope, name: &str, value: Option<&Value>) -> Value {
    let ctx = &state.ctx;
    match op {
        "has" => some(view(ctx, &state.request, scope).contains_key(name).into()),
        "get" => {
            let stored = view(ctx, &state.request, scope).remove(name).flatten();
            let value = stored.map(|v| interpolate_value(state, v));
            if let Some(value) = &value
                && sensitive(ctx, scope, name)
            {
                note_sensitive(state, value);
            }
            defined(value)
        }
        "all" => {
            let mut pairs = Vec::new();
            for (key, value) in view(ctx, &state.request, scope) {
                if scope == Scope::Env && key == ENV_NAME_KEY {
                    continue;
                }
                if let Some(value) = &value
                    && sensitive(ctx, scope, &key)
                {
                    note_sensitive(state, value);
                }
                pairs.push(match value {
                    Some(value) => json!([key, value]),
                    None => json!([key]),
                });
            }
            json!({ "p": pairs })
        }
        "set" | "delete" | "deleteAll" => {
            write(state, op, scope, name, value);
            json!({})
        }
        _ => json!({ "e": format!("unknown bru operation {op}") }),
    }
}

/// Whether a value read from `scope` under `name` came from a secret or a sensitive override.
fn sensitive(ctx: &VarContext, scope: Scope, name: &str) -> bool {
    match scope {
        Scope::Env => ctx.env_name_sensitive(name),
        Scope::Global => ctx.global_name_sensitive(name),
        _ => false,
    }
}

/// Applies a write. Runtime writes go to the shared `RuntimeVars`, environment, global and
/// collection writes to the overlay; the folder and request stores are read only.
fn write(state: &HostState, op: &str, scope: Scope, name: &str, value: Option<&Value>) {
    let ctx = &state.ctx;
    let entry = || {
        value
            .cloned()
            .map_or(OverlayEntry::Undefined, OverlayEntry::Set)
    };
    let layer = match scope {
        Scope::Runtime => {
            let vars = ctx.runtime_vars();
            match op {
                "set" => vars.set(name.to_owned(), value.cloned()),
                "delete" => vars.remove(name),
                _ => vars.clear(),
            }
            return;
        }
        Scope::Env => OverlayLayer::Environment,
        Scope::Global => OverlayLayer::Global,
        Scope::Collection => OverlayLayer::Collection,
        Scope::Folder | Scope::Request => return,
    };
    let overlay = ctx.env_overlay();
    // Deleting the selected environment's name is ignored, as the reference does.
    let protected = |key: &str| scope == Scope::Env && key == ENV_NAME_KEY;
    match op {
        "set" => overlay.put(layer, name.to_owned(), entry()),
        "delete" if !protected(name) && view(ctx, &state.request, scope).contains_key(name) => {
            overlay.put(layer, name.to_owned(), OverlayEntry::Deleted);
        }
        "deleteAll" => {
            for key in view(ctx, &state.request, scope).into_keys() {
                if !protected(&key) {
                    overlay.put(layer, key, OverlayEntry::Deleted);
                }
            }
        }
        _ => {}
    }
}

fn scope(name: &str) -> Option<Scope> {
    Some(match name {
        "runtime" => Scope::Runtime,
        "env" => Scope::Env,
        "global" => Scope::Global,
        "collection" => Scope::Collection,
        "folder" => Scope::Folder,
        "request" => Scope::Request,
        _ => return None,
    })
}

/// One store as the script sees it now; `None` is a name set to `undefined`.
fn view(ctx: &VarContext, req: &RequestFile, scope: Scope) -> BTreeMap<String, Option<Value>> {
    let plain = |vars: Vars| vars.into_iter().map(|(k, v)| (k, Some(v)));
    match scope {
        Scope::Runtime => ctx.runtime_vars().snapshot(),
        Scope::Env => ctx.script_env_view(),
        Scope::Global => ctx.global_view(),
        Scope::Collection => ctx.collection_view(),
        Scope::Folder => plain(ctx.folder_vars(req)).collect(),
        Scope::Request => plain(VarContext::request_vars(req)).collect(),
    }
}

/// A getter's value with its placeholders expanded, as the reference's `interpolate` does:
/// strings directly, objects and arrays through their JSON text, anything else as is.
fn interpolate_value(state: &HostState, value: Value) -> Value {
    match &value {
        Value::String(text) => Value::String(interpolate_text(state, text)),
        Value::Object(_) | Value::Array(_) => {
            serde_json::from_str(&interpolate_text(state, &value.to_string())).unwrap_or(value)
        }
        _ => value,
    }
}

/// Expands `text` over the request's current lookup map, leaving undefined names as written
/// and recording every sensitive substitution.
pub(crate) fn interpolate_text(state: &HostState, text: &str) -> String {
    let ctx = &state.ctx;
    let (vars, names) = ctx.lookup_with(&state.request, &ctx.runtime_vars().snapshot());
    let tracker = ctx.tracker(names);
    let out = interpolate_lenient(text, &vars, &tracker, &RefCell::new(ctx.generator()));
    note_read(state, tracker.into_values());
    out
}

/// Records every non-empty string inside `value` as a sensitive value the script read.
pub(super) fn note_sensitive(state: &HostState, value: &Value) {
    let mut found = Vec::new();
    json_strings(value, &mut found);
    note_read(state, found.into_iter().map(str::to_owned));
}

// A read value is tainted on the shared runtime store at once, not when the phase ends, so a
// concurrent send substituting a copy of it already treats the copy as sensitive.
pub(super) fn note_read(state: &HostState, values: impl IntoIterator<Item = String>) {
    let fresh: Vec<String> = {
        let mut sensitive = state
            .sensitive
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        values
            .into_iter()
            .filter(|v| !v.is_empty())
            .filter(|v| {
                let new = !sensitive.contains(v);
                if new {
                    sensitive.push(v.clone());
                }
                new
            })
            .collect()
    };
    state.ctx.runtime_vars().taint(&fresh);
}

fn some(value: Value) -> Value {
    json!({ "v": value })
}

fn defined(value: Option<Value>) -> Value {
    value.map_or_else(|| json!({}), some)
}
