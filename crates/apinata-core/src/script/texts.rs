//! The texts a send would evaluate as JavaScript, listed before anything runs so each can be
//! approved by its hash.

use std::collections::BTreeSet;

use serde_json::Value;

use super::ScriptPhase;
use super::segments::segments;
use crate::assertion::{lhs, rhs};
use crate::script_trust::script_hash;
use crate::{ItemPath, RequestFile, RunPlan, VarContext, js};

/// Where a text that needs approval comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptTextKind {
    /// A script segment of `phase` at `level` (`collection`, `folder <label>` or `request`).
    Script {
        /// The phase that runs it.
        phase: ScriptPhase,
        /// The level that holds it.
        level: String,
    },
    /// The expression of the post-response variable `name`.
    Variable {
        /// The variable's name as written.
        name: String,
    },
    /// An assertion's left side.
    AssertionLeft,
    /// An assertion's right side, as written after its operator.
    AssertionRight,
}

/// One text a send needs approved, with its hash and a copy safe to show.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptText {
    /// Where the text comes from.
    pub kind: ScriptTextKind,
    /// The exact text as written, which is what approval covers.
    pub text: String,
    /// [`script_hash`] of `text`.
    pub hash: String,
    /// `text` with every sensitive value of the request's scope replaced by `<secret>`.
    pub shown: String,
}

/// Every text a send of `req` with scripts on could evaluate and that needs approval:
/// segments of the three phases in run order, then post-response variable expressions
/// that are not `res` chains, then per enabled assertion its left side (not a `res` chain)
/// and its raw right side when it would run JavaScript. Deduplicated by hash, first kept.
pub fn script_texts(req: &RequestFile, ctx: &VarContext) -> Vec<ScriptText> {
    texts_masking(req, ctx, &sensitive_strings(req, ctx))
}

/// [`script_texts`] of `req` under `ctx`, each shown with `secrets` masked.
fn texts_masking(req: &RequestFile, ctx: &VarContext, secrets: &[String]) -> Vec<ScriptText> {
    let mut found: Vec<(ScriptTextKind, String)> = Vec::new();
    for phase in [
        ScriptPhase::PreRequest,
        ScriptPhase::PostResponse,
        ScriptPhase::Tests,
    ] {
        found.extend(segments(req, ctx, phase).into_iter().map(|s| {
            let kind = ScriptTextKind::Script {
                phase,
                level: s.level,
            };
            (kind, s.code)
        }));
    }
    for var in ctx.post_response_vars_for(req) {
        let expression = js::js_trim(var.expression());
        if !lhs::supported_shape(expression) {
            let kind = ScriptTextKind::Variable {
                name: var.name().to_owned(),
            };
            found.push((kind, expression.to_owned()));
        }
    }
    for assertion in req.assertions().iter().filter(|a| a.enabled()) {
        let expression = js::js_trim(assertion.expression());
        if !lhs::supported_shape(expression) {
            found.push((ScriptTextKind::AssertionLeft, expression.to_owned()));
        }
        let (op, text) = rhs::split(assertion.bruno_value());
        if rhs::needs_script(op, &text) {
            found.push((ScriptTextKind::AssertionRight, text));
        }
    }
    let mut seen = BTreeSet::new();
    found
        .into_iter()
        .filter_map(|(kind, text)| {
            let hash = script_hash(&text);
            seen.insert(hash.clone()).then(|| ScriptText {
                kind,
                shown: mask(&text, secrets),
                text,
                hash,
            })
        })
        .collect()
}

/// [`script_texts`] over the plan's requests, deduplicated by hash, each with the first item
/// needing it.
pub fn plan_script_texts(plan: &RunPlan, ctx: &VarContext) -> Vec<(ItemPath, ScriptText)> {
    plan_script_texts_across(plan, std::slice::from_ref(ctx))
}

/// [`plan_script_texts`] under the first of `contexts`, each text shown with the sensitive
/// values of every one of `contexts` masked, for a prompt that names no environment.
/// Empty when `contexts` is.
pub fn plan_script_texts_across(
    plan: &RunPlan,
    contexts: &[VarContext],
) -> Vec<(ItemPath, ScriptText)> {
    let Some(first) = contexts.first() else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (item, req) in plan.items().zip(plan.requests()) {
        let mut secrets: Vec<String> = contexts
            .iter()
            .flat_map(|ctx| sensitive_strings(req, ctx))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for text in texts_masking(req, first, &secrets) {
            if seen.insert(text.hash.clone()) {
                out.push((item.clone(), text));
            }
        }
    }
    out
}

/// Every non-empty string inside the values `req`'s scope marks sensitive, and every value
/// scripts tainted, longest first so a value containing another is masked whole. Of
/// `process`, only `.env` values count: inherited shell values are sensitive once read.
fn sensitive_strings(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let (vars, names) = ctx.lookup_for(req);
    let mut out: BTreeSet<String> = ctx.runtime_vars().tainted().into_iter().collect();
    let scoped = names.iter().filter(|name| *name != "process");
    for value in scoped.filter_map(|name| vars.get(name)) {
        collect_strings(value, &mut out);
    }
    // Global and environment secrets count even when a closer layer shadows their name.
    out.extend(
        ctx.layer_secrets()
            .into_iter()
            .map(|(_, v)| v)
            .filter(|v| !v.is_empty()),
    );
    if let Some(env) = ctx.process_env() {
        for value in ctx.dotenv_keys().iter().filter_map(|key| env.get(key)) {
            collect_strings(value, &mut out);
        }
    }
    let mut out: Vec<String> = out.into_iter().collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.len()));
    out
}

/// Walks `value` to its string leaves, so an object such as `process` contributes each one.
fn collect_strings(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) if !s.is_empty() => {
            out.insert(s.clone());
        }
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

/// `text` with each of `secrets`, taken in order, replaced by `<secret>`.
pub(crate) fn mask(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_owned(), |shown, secret| {
        shown.replace(secret, "<secret>")
    })
}

#[cfg(test)]
mod shadowed_secret_tests {
    use super::*;
    use crate::collection::Collection;
    use crate::context::ContextOptions;
    use crate::secrets::{MemoryStore, SecretStore};
    use std::sync::Arc;

    const SECRET: &str = "env-token-value-1";

    // The environment secret `token` is shadowed by a folder variable; the request's script
    // holds the secret's text, which the approval prompt must not show.
    fn shown_with(shadow: bool) -> Vec<String> {
        let dir = tempfile::TempDir::new().unwrap();
        let script = format!(
            "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        const k = '{SECRET}';\n"
        );
        let folder_vars = if shadow {
            "  variables:\n    - name: token\n      value: plain\n"
        } else {
            ""
        };
        for (rel, text) in [
            (
                "opencollection.yml",
                "opencollection: 1.0.0\ninfo:\n  name: c\n".to_owned(),
            ),
            (
                "environments/dev.yml",
                "name: dev\nvariables:\n  - secret: true\n    name: token\n".to_owned(),
            ),
            (
                "f/folder.yml",
                format!("info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n{folder_vars}"),
            ),
            (
                "f/r.yml",
                format!(
                    "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\n{script}"
                ),
            ),
        ] {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let c = Collection::open(dir.path()).unwrap();
        let store = Arc::new(MemoryStore::new());
        store.set(&c.secret_key("dev", "token"), SECRET).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new().env("dev").secrets(store)).unwrap();
        let req = RequestFile::load(dir.path().join("f/r.yml")).unwrap();
        script_texts(&req, &ctx)
            .into_iter()
            .map(|t| t.shown)
            .collect()
    }

    #[test]
    fn an_unshadowed_secret_is_masked_in_shown_text() {
        let shown = shown_with(false);
        assert!(shown.iter().all(|s| !s.contains(SECRET)), "{shown:?}");
    }

    #[test]
    fn a_shadowed_secret_is_masked_in_shown_text() {
        let shown = shown_with(true);
        assert!(!shown.is_empty());
        assert!(shown.iter().all(|s| !s.contains(SECRET)), "{shown:?}");
    }
}
