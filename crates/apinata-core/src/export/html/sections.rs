//! The opt-in sections: a request's assertions and scripts, and the environment listing.
//! Everything is shown as written and escaped; nothing is run or resolved.

use super::text::escape;
use crate::environment::{EnvValue, own_names, resolve_environment};
use crate::export::ExportNote;
use crate::js::{self, js_trim};
use crate::request::RequestFile;
use crate::script::ScriptPhase;

/// The request's assertions as a list, then each non-blank script as a code block.
pub(super) fn scripts(req: &RequestFile) -> String {
    let mut out = String::new();
    let items: String = req
        .assertions()
        .iter()
        .map(|a| {
            let mut line = format!("{} {}", a.expression(), a.operator());
            if let Some(value) = a.value() {
                line.push(' ');
                line.push_str(value);
            }
            match a.enabled() {
                true => format!("<li><code>{}</code></li>\n", escape(&line)),
                false => format!(
                    "<li><code>{}</code> <span class=\"tag\">disabled</span></li>\n",
                    escape(&line)
                ),
            }
        })
        .collect();
    if !items.is_empty() {
        out.push_str(&format!(
            "<p class=\"part\">Assertions</p>\n<ul>\n{items}</ul>\n"
        ));
    }
    for phase in [
        ScriptPhase::PreRequest,
        ScriptPhase::PostResponse,
        ScriptPhase::Tests,
    ] {
        let code = req.script(phase);
        if !js_trim(&code).is_empty() {
            out.push_str(&format!(
                "<p class=\"part\">Script ({})</p>\n<pre>{}</pre>\n",
                phase.as_str(),
                escape(js::script_code(&code))
            ));
        }
    }
    out
}

/// One table per environment, flattened through `extends`; empty when there are none. A secret
/// row shows a mask, and its value is never read, so it cannot reach the page.
pub(super) fn environments(collection: &crate::Collection, notes: &mut Vec<ExportNote>) -> String {
    let envs = collection.environments();
    let mut out = String::new();
    let mut warned = std::collections::HashSet::new();
    for env in envs {
        let Some(resolved) = resolve_environment(envs, env.name()) else {
            continue;
        };
        // The file's own rows come first in file order, then the inherited ones it does not
        // override.
        let mut order = own_names(env);
        order.retain(|n| resolved.vars.contains_key(n));
        let rest: Vec<String> = resolved
            .vars
            .keys()
            .filter(|k| !order.contains(k))
            .cloned()
            .collect();
        order.extend(rest);
        let rows: String = order
            .iter()
            .filter_map(|name| resolved.vars.get(name).map(|value| (name, value)))
            .map(|(name, value)| {
                let cell = match value {
                    EnvValue::Plain(v) => escape(&js::to_text(v)),
                    EnvValue::Secret => "<span class=\"tag\">secret</span>".to_owned(),
                };
                format!("<tr><td>{}</td><td>{cell}</td></tr>\n", escape(name))
            })
            .collect();
        out.push_str(&format!(
            "<table>\n<caption>{}</caption>\n<thead><tr><th>Name</th><th>Value</th></tr></thead>\n<tbody>\n{rows}</tbody>\n</table>\n",
            escape(env.name())
        ));
        // A broken chain is reported by every environment that walks it, but only once here.
        for warning in resolved
            .warnings
            .into_iter()
            .filter(|w| warned.insert(w.clone()))
        {
            notes.push(ExportNote::new(
                format!("environments / {}", env.name()),
                warning,
            ));
        }
    }
    if out.is_empty() {
        return out;
    }
    format!("<section id=\"environments\">\n<h2>Environments</h2>\n{out}</section>\n")
}
