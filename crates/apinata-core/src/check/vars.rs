//! The variable pass: resolve every request per environment with placeholders.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use regress::Regex;

use super::{CheckOptions, relative};
use crate::collection::Folder;
use crate::document::Document;
use crate::environment::{EnvValue, resolve_environment};
use crate::error::{ContextError, HintShell, ResolveError};
use crate::{Collection, ContextOptions, RequestFile, VarContext, resolve, script_texts};

use super::report::{Finding, FindingKind, Severity};

/// Appended to a no-value warning when no secret store was given to look in.
const UNCHECKED_KEYCHAIN: &str = " (keychain not checked)";
/// Stand-in for a declared secret that has no value to check against.
const SECRET_PLACEHOLDER: &str = "<secret>";
/// Stand-in for an undefined name, valid inside a host and inside JSON text.
const NAME_PLACEHOLDER: &str = "apinata-check";
/// Upper bound on resolve rounds per request, one round per undefined name.
const MAX_ROUNDS: usize = 50;
/// Prefix of a name that reads the process environment at send time.
const PROCESS_ENV: &str = "process.env.";

/// A setter call whose first argument is a string literal; capture 3 is the name.
static LITERAL_SETTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b(setVar|setEnvVar|setGlobalEnvVar)\s*\(\s*(["'`])([^"'`]*)\2\s*[,)]"#)
        .expect("literal setter pattern compiles")
});
/// Any setter call, literal or not.
static ANY_SETTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(setVar|setEnvVar|setGlobalEnvVar)\s*\(").expect("setter pattern compiles")
});

/// Names a send could define at run time, found without running anything.
#[derive(Default)]
struct RuntimeNames {
    names: HashSet<String>,
    /// A setter whose name is computed: any undefined name might be set by it.
    dynamic: bool,
}

impl RuntimeNames {
    fn covers(&self, name: &str) -> bool {
        self.dynamic || self.names.contains(name.trim())
    }
}

/// Collect the post-response variable names and the setter names of every request.
/// Comments are not stripped, so the scan can only over-approximate.
fn runtime_names(folder: &Folder, ctx: &VarContext) -> RuntimeNames {
    let mut found = RuntimeNames::default();
    scan_folder(folder, ctx, &mut found);
    found
}

fn scan_folder(folder: &Folder, ctx: &VarContext, found: &mut RuntimeNames) {
    for req in folder.requests() {
        for var in ctx.post_response_vars_for(req) {
            found.names.insert(var.name().to_owned());
        }
        for text in script_texts(req, ctx) {
            let mut literal = 0;
            for m in LITERAL_SETTER.find_iter(&text.text) {
                literal += 1;
                if let Some(name) = m.group(3) {
                    found.names.insert(text.text[name].to_owned());
                }
            }
            if ANY_SETTER.find_iter(&text.text).count() > literal {
                found.dynamic = true;
            }
        }
    }
    for sub in folder.folders() {
        scan_folder(sub, ctx, found);
    }
}

/// Everything one request check shares across the collection for one environment.
struct Pass<'a> {
    collection: &'a Collection,
    opts: &'a CheckOptions,
    env: Option<&'a str>,
    shared: &'a HashSet<String>,
    runtime: RuntimeNames,
    /// A context with the environment's real values, to say where an unresolved name lives.
    plain: &'a VarContext,
    has_secret_placeholder: bool,
}

/// Resolve every request once per environment (once with none selected when the
/// collection has no environments) and record what the resolution reports.
/// Returns the items that could not be fully judged, sorted and without repeats.
pub(super) fn check_variables(
    collection: &Collection,
    opts: &CheckOptions,
    environments: &[String],
    out: &mut Vec<Finding>,
) -> Vec<String> {
    let mut unresolved = Vec::new();
    let shared = shared_placeholders(collection);
    let passes: Vec<Option<&str>> = if environments.is_empty() {
        vec![None]
    } else {
        environments.iter().map(|e| Some(e.as_str())).collect()
    };
    for env in passes {
        // A context that cannot be built is reported once, not once per request.
        let base = match VarContext::new(collection, options(collection, opts, env, &[])) {
            Ok(ctx) => ctx,
            Err(e) => {
                out.push(context_finding(&e, env));
                continue;
            }
        };
        if let Some(name) = env {
            for _ in base.warnings() {
                out.push(Finding::new(
                    format!("environments/{name}.yml"),
                    env,
                    Severity::Warning,
                    FindingKind::InvalidRequest,
                    format!("environment `{name}` has a configuration warning"),
                ));
            }
        }
        // The same options as a real run, without the placeholders, so a secret that a
        // run would find empty shows up in `env_unstored`.
        let plain = VarContext::new(collection, plain_options(opts, env));
        if let (Some(name), Ok(plain)) = (env, &plain) {
            no_value_warnings(
                plain,
                name,
                opts.secrets.is_some() || opts.keychain_not_used,
                opts.hint_shell,
                out,
            );
        }
        let pass = Pass {
            collection,
            opts,
            env,
            shared: &shared,
            runtime: runtime_names(collection.tree(), &base),
            plain: plain.as_ref().unwrap_or(&base),
            has_secret_placeholder: env.is_some_and(|n| has_secret(collection, n)),
        };
        check_folder(&pass, collection.tree(), (out, &mut unresolved));
    }
    unresolved.sort();
    unresolved.dedup();
    unresolved
}

fn check_folder(pass: &Pass<'_>, folder: &Folder, sinks: (&mut Vec<Finding>, &mut Vec<String>)) {
    let (out, unresolved) = sinks;
    for req in folder.requests() {
        if !check_request(pass, req, out) {
            unresolved.push(relative(pass.collection, req.path()));
        }
    }
    for sub in folder.folders() {
        check_folder(pass, sub, (out, unresolved));
    }
}

/// Resolve `req` until it succeeds or fails for a reason other than a missing name,
/// supplying a placeholder for each missing name so that all of them are reported.
/// Returns false when the request could not be fully judged.
fn check_request(pass: &Pass<'_>, req: &RequestFile, out: &mut Vec<Finding>) -> bool {
    let (collection, env) = (pass.collection, pass.env);
    let item = relative(collection, req.path());
    let mut known = pass.shared.clone();
    known.extend(placeholders(req.document()));
    let mut undefined: Vec<String> = Vec::new();
    let mut prompts: Vec<String> = Vec::new();
    for _ in 0..MAX_ROUNDS {
        let ctx = match VarContext::new(collection, options(collection, pass.opts, env, &undefined))
        {
            Ok(ctx) => ctx,
            // Reported once by the caller.
            Err(_) => return true,
        };
        let ctx = if prompts.is_empty() {
            ctx
        } else {
            ctx.with_prompt_values(prompts.iter().map(|l| (l.as_str(), NAME_PLACEHOLDER)))
        };
        match resolve(req, &ctx) {
            Ok(resolved) => {
                // The message names the header as written and never quotes its value.
                if let Some(problem) = resolved.header_problem() {
                    out.push(Finding::new(
                        item.clone(),
                        env,
                        Severity::Error,
                        FindingKind::InvalidRequest,
                        problem.to_owned(),
                    ));
                }
                super::files::check_files(&item, env, &resolved, out);
                return true;
            }
            Err(ResolveError::PromptRequired { labels }) => {
                // A label that is still unanswered after being answered cannot be judged.
                if labels.iter().all(|l| prompts.contains(l)) {
                    return false;
                }
                prompts.extend(labels);
            }
            // A token only a run can fetch; nothing here to judge.
            Err(ResolveError::OAuth2NoToken { .. }) => return false,
            Err(ResolveError::UndefinedVariable { name, .. }) => {
                // A name the files never write came out of a variable's value, so
                // it is not a defect of this collection and the request cannot be judged.
                if undefined.contains(&name) || !known.contains(name.trim()) {
                    return false;
                }
                out.push(undefined_finding(pass, &item, &name));
                undefined.push(name);
            }
            Err(e) => {
                // With a placeholder in play the failure may be its doing, so the
                // request is reported as unjudged instead of guessed at. A URL written
                // without any variable fails by its own text, whatever else is stubbed.
                let own_fault = is_url_error(&e) && url_is_literal(req);
                let stubbed =
                    !undefined.is_empty() || !prompts.is_empty() || pass.has_secret_placeholder;
                if stubbed && !own_fault {
                    return false;
                }
                out.push(Finding::new(
                    item,
                    env,
                    Severity::Error,
                    FindingKind::InvalidRequest,
                    invalid_text(&e),
                ));
                return true;
            }
        }
    }
    // The round cap was hit: more names are missing than are worth listing.
    false
}

fn is_url_error(e: &ResolveError) -> bool {
    matches!(e, ResolveError::EmptyUrl | ResolveError::InvalidUrl { .. })
}

/// Whether the URL and its path params are plain text, so no variable can shape the URL.
fn url_is_literal(req: &RequestFile) -> bool {
    let params = req.document().get(&[req.root_key(), "params"]);
    let params = params
        .and_then(|v| yaml_serde::to_string(v).ok())
        .unwrap_or_default();
    !req.url().contains("{{") && !params.contains("{{")
}

/// The finding for an undefined name: an error unless the name may legitimately be
/// supplied by the process environment or defined while the collection runs.
fn undefined_finding(pass: &Pass<'_>, item: &str, name: &str) -> Finding {
    let (severity, message) = if name.trim().starts_with(PROCESS_ENV) {
        (
            Severity::Warning,
            format!("`{name}` depends on the process environment"),
        )
    } else if pass.runtime.covers(name) {
        (
            Severity::Warning,
            format!("variable `{name}` is not defined here; it may be set at run time"),
        )
    } else {
        (Severity::Error, format!("variable `{name}` is not defined"))
    };
    let message = message + &defined_elsewhere(pass.plain, name);
    Finding::new(
        item,
        pass.env,
        severity,
        FindingKind::UndefinedVariable,
        message,
    )
}

/// The `; defined in environment X` suffix, empty when no other environment defines `name`.
/// It lists every other defining environment, so the text is the same in each checked one.
fn defined_elsewhere(ctx: &VarContext, name: &str) -> String {
    match ctx.other_envs_defining(name.trim()).as_slice() {
        [] => String::new(),
        [one] => format!("; defined in environment {one}"),
        many => format!("; defined in environments {}", many.join(", ")),
    }
}

/// One warning per secret of `env` that a real run would find without a value. Without a store
/// the keychain was not consulted, and a constant suffix says so, which keeps `collapse` merging.
fn no_value_warnings(
    plain: &VarContext,
    env: &str,
    store_checked: bool,
    shell: HintShell,
    out: &mut Vec<Finding>,
) {
    let note = if store_checked {
        ""
    } else {
        UNCHECKED_KEYCHAIN
    };
    for name in plain.unstored_env_secrets() {
        let text = plain.secret_hint(env, name).text(name, shell);
        out.push(Finding::new(
            env,
            Some(env),
            Severity::Warning,
            FindingKind::UndefinedVariable,
            format!("{text}{note}"),
        ));
    }
}

/// The caller's options with no placeholders added.
fn plain_options(opts: &CheckOptions, env: Option<&str>) -> ContextOptions {
    let mut o = ContextOptions::new();
    if let Some(name) = env {
        o = o.env(name);
    }
    for (k, v) in &opts.env_vars {
        o = o.env_var(k, v);
    }
    for (k, v) in &opts.global_env_vars {
        o = o.global_env_var(k, v);
    }
    if let Some(store) = &opts.secrets {
        o = o.secrets(Arc::clone(store));
    }
    o
}

/// Whether the environment declares a secret that gets a placeholder value.
fn has_secret(collection: &Collection, env: &str) -> bool {
    resolve_environment(collection.environments(), env)
        .is_some_and(|r| r.vars.values().any(|v| *v == EnvValue::Secret))
}

/// The variable names written as `{{name}}` in the collection root, the folders and
/// the environments, the files a request inherits from.
fn shared_placeholders(collection: &Collection) -> HashSet<String> {
    fn folders(f: &Folder, out: &mut HashSet<String>) {
        if let Some(doc) = f.document() {
            out.extend(placeholders(doc));
        }
        for sub in f.folders() {
            folders(sub, out);
        }
    }
    let mut out = placeholders(collection.root_document());
    folders(collection.tree(), &mut out);
    for env in collection.environments() {
        out.extend(placeholders(env.document()));
    }
    out
}

/// Every name between `{{` and `}}` in a document's text.
fn placeholders(doc: &Document) -> HashSet<String> {
    let text = yaml_serde::to_string(doc.mapping()).unwrap_or_default();
    let mut out = HashSet::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else { break };
        out.insert(rest[..end].trim().to_owned());
        rest = &rest[end..];
    }
    out
}

/// The context options for one environment: placeholders for declared secrets and
/// undefined names, then the caller's values, which win.
fn options(
    collection: &Collection,
    opts: &CheckOptions,
    env: Option<&str>,
    undefined: &[String],
) -> ContextOptions {
    let mut o = ContextOptions::new();
    if let Some(name) = env {
        o = o.env(name);
        if let Some(resolved) = resolve_environment(collection.environments(), name) {
            for (var, value) in &resolved.vars {
                if *value == EnvValue::Secret {
                    o = o.sensitive_env_var(var, SECRET_PLACEHOLDER);
                }
            }
        }
    }
    for name in undefined {
        o = o.env_var(name, NAME_PLACEHOLDER);
    }
    for (k, v) in &opts.env_vars {
        o = o.env_var(k, v);
    }
    for (k, v) in &opts.global_env_vars {
        o = o.global_env_var(k, v);
    }
    o
}

fn context_finding(e: &ContextError, env: Option<&str>) -> Finding {
    let environment_file = || env.map_or_else(String::new, |n| format!("environments/{n}.yml"));
    let (item, text) = match e {
        ContextError::DotEnv { .. } => (".env".to_owned(), "the .env file cannot be read"),
        ContextError::EnvironmentFailedToLoad { .. } => {
            (environment_file(), "the environment file cannot be loaded")
        }
        _ => (
            environment_file(),
            "the environment cannot be loaded or read",
        ),
    };
    Finding::new(
        item,
        env,
        Severity::Error,
        FindingKind::UnparsableItem,
        text,
    )
}

/// Fixed text for a resolve failure; never the error's own text, which can carry values.
fn invalid_text(e: &ResolveError) -> &'static str {
    match e {
        ResolveError::CyclicVariable { .. } => "a variable refers to itself",
        ResolveError::UnsupportedBody { .. } => "the body type is not supported",
        ResolveError::UnsupportedAuth { .. } => "the auth type is not supported",
        ResolveError::GraphqlVariables { .. } => "the GraphQL variables are not valid JSON",
        ResolveError::InvalidUrl { .. } => "the URL is not valid",
        ResolveError::EmptyUrl => "the URL is empty",
        ResolveError::InvalidProxy { .. } => "the proxy settings are not valid",
        _ => "the request cannot be resolved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(e: &ContextError, env: Option<&str>) -> (String, String) {
        let f = context_finding(e, env);
        (f.item().to_owned(), f.message().to_owned())
    }

    #[test]
    fn a_failed_environment_load_has_fixed_text_not_the_error_text() {
        let e = ContextError::EnvironmentFailedToLoad {
            name: "dev".into(),
            message: "canary-load-text".into(),
        };
        assert_eq!(
            text_of(&e, Some("dev")),
            (
                "environments/dev.yml".to_owned(),
                "the environment file cannot be loaded".to_owned()
            )
        );
    }

    #[test]
    fn an_unknown_context_error_gets_the_generic_text() {
        let e = ContextError::EnvironmentNotFound {
            name: "canary-name".into(),
            available: Vec::new(),
        };
        assert_eq!(
            text_of(&e, Some("dev")),
            (
                "environments/dev.yml".to_owned(),
                "the environment cannot be loaded or read".to_owned()
            )
        );
    }
}
