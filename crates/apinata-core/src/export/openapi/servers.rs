//! Server lists for a request origin: the origin as an OpenAPI server template, then one
//! concrete server per environment that fills in every variable.

use crate::Collection;
use crate::edit::VarValue;
use crate::environment::{EnvValue, resolve_environment};
use crate::js;
use serde_json::{Map, Value, json};

/// The servers for one origin as written (`{{x}}` kept): the literal origin alone, or the
/// templated origin with its variables' defaults, then one server per environment that
/// resolves every variable to a plain value.
pub(super) fn servers(collection: &Collection, origin: &str) -> Vec<Value> {
    let names = variable_names(origin);
    if names.is_empty() {
        return vec![json!({"url": origin})];
    }
    let mut variables = Map::new();
    for name in &names {
        variables.insert(
            (*name).into(),
            json!({"default": default_value(collection, name)}),
        );
    }
    let mut out =
        vec![json!({"url": origin.replace("{{", "{").replace("}}", "}"), "variables": variables})];

    // An environment yields a server only when every name has a plain value and the
    // substituted URL is free of further templates; secrets never reach the document.
    'envs: for env in collection.environments() {
        let Some(resolved) = resolve_environment(collection.environments(), env.name()) else {
            continue;
        };
        let mut url = origin.to_owned();
        for name in &names {
            let Some(EnvValue::Plain(value)) = resolved.vars.get(*name) else {
                continue 'envs;
            };
            url = url.replace(&format!("{{{{{name}}}}}"), &js::to_text(value));
        }
        if !url.contains("{{") {
            let url = super::url::strip_userinfo(&url).unwrap_or(url);
            out.push(json!({"url": url, "description": env.name()}));
        }
    }
    out
}

/// The `{{x}}` names in `origin`, each once, in first-seen order.
fn variable_names(origin: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = origin;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = &after[..end];
        if !names.contains(&name) {
            names.push(name);
        }
        rest = &after[end + 2..];
    }
    names
}

/// The first enabled collection variable named `name` as text without any URL user name and
/// password, or `""` when there is none, it is marked secret, or its value is a template.
fn default_value(collection: &Collection, name: &str) -> String {
    // `vars()` maps the YAML list one to one, so the same index finds the row's secret flag.
    let rows = collection.root_document().get(&["request", "variables"]);
    let vars = collection.file().vars();
    let Some((index, var)) = vars
        .iter()
        .enumerate()
        .find(|(_, v)| v.enabled && v.name == name)
    else {
        return String::new();
    };
    let secret = rows
        .and_then(|r| r.get(index))
        .and_then(|row| row.get("secret"))
        .and_then(|s| s.as_bool())
        == Some(true);
    let text = match &var.value {
        VarValue::Text(text) => text,
        VarValue::Typed { data, .. } => data,
    };
    if secret || text.contains("{{") {
        String::new()
    } else {
        super::url::strip_userinfo(text).unwrap_or_else(|| text.clone())
    }
}
