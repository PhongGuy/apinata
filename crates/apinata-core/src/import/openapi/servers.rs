//! Servers: the document's become environments, an operation's become request variables.

use super::Doc;
use super::bounds::approx_size;
use super::operations::{Operation, normalize_name};
use super::version::Version;
use crate::edit::{VarValue, Variable};
use crate::env_file::EnvVariable;
use crate::import::{DocumentError, ImportNote, ImportedEnvironment, NoteKind};
use crate::js;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// The bytes of a resolved URL needed to tell whether it starts with `http://` or `https://`.
const SCHEME_PREFIX: usize = "https://".len();

/// One server read as variables: `baseUrl` first, then each declared variable, every one with
/// its document path. `resolved` is the start of the URL with the defaults substituted.
struct Server {
    vars: Vec<(String, String, String)>,
    resolved: String,
}

/// The document's servers as environments. When none of them gives an absolute URL, a
/// `NeedsInput` note asking for `baseUrl` goes to `notes`.
pub(crate) fn environments(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
) -> Result<Vec<ImportedEnvironment>, DocumentError> {
    let (named, source) = if doc.version == Version::Swagger2 {
        (swagger_servers(doc)?, "host")
    } else {
        (openapi_servers(doc)?, "servers")
    };
    let mut absolute = false;
    let mut out = Vec::with_capacity(named.len());
    for (name, server) in named {
        let scheme = |s: &str| {
            server
                .resolved
                .get(..s.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(s))
        };
        absolute |= scheme("http://") || scheme("https://");
        doc.budget.charge(1 + server.vars.len() as u64)?;
        let (variables, sources) = server
            .vars
            .into_iter()
            .map(|(n, v, s)| (EnvVariable::new(n, VarValue::Text(v)), s))
            .unzip();
        out.push(ImportedEnvironment {
            name,
            variables,
            notes: Vec::new(),
            sources,
        });
    }
    if !absolute {
        let note = ImportNote::new(
            source,
            NoteKind::NeedsInput,
            "no server gives an absolute URL; set `baseUrl` in an environment before sending",
        );
        doc.budget.charge_note(&note)?;
        notes.push(note);
    }
    Ok(out)
}

/// The variables of the first server of `op`, else of its path item; empty when neither
/// declares servers. More than one server adds a `Converted` note to `notes`.
pub(crate) fn request_vars(
    doc: &mut Doc<'_>,
    op: &Operation<'_>,
    notes: &mut Vec<ImportNote>,
) -> Result<Vec<Variable>, DocumentError> {
    fn declared(value: &Value) -> Option<&Vec<Value>> {
        value
            .get("servers")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
    }
    // Swagger 2.0 has no `servers`; its operations all use the document's host.
    if doc.version == Version::Swagger2 {
        return Ok(Vec::new());
    }
    let (servers, source) = match (declared(op.op()), declared(&op.item)) {
        (Some(s), _) => (s, format!("{}.servers", op.source)),
        (None, Some(s)) => (s, format!("paths.{}.servers", op.path)),
        (None, None) => return Ok(Vec::new()),
    };
    if servers.len() > 1 {
        let note = ImportNote::new(
            source.as_str(),
            NoteKind::Converted,
            "only the first of these servers is used",
        );
        doc.budget.charge_note(&note)?;
        notes.push(note);
    }
    let Some(server) = server(doc, &servers[0], &format!("{source}[0]"))? else {
        return Ok(Vec::new());
    };
    doc.budget.charge(server.vars.len() as u64)?;
    Ok(server
        .vars
        .into_iter()
        .map(|(n, v, _)| Variable::new(n, VarValue::Text(v)))
        .collect())
}

/// Each root server with a string `url`, named by its `name` or `description`, else by its
/// position, with repeated names counted up.
fn openapi_servers(doc: &mut Doc<'_>) -> Result<Vec<(String, Server)>, DocumentError> {
    let root = doc.root;
    let mut out = Vec::new();
    let mut taken = HashSet::new();
    let mut seen: HashMap<String, u64> = HashMap::new();
    let list = root.get("servers").and_then(Value::as_array);
    for (i, value) in list.into_iter().flatten().enumerate() {
        let Some(server) = server(doc, value, &format!("servers[{i}]"))? else {
            continue;
        };
        let label = ["name", "description"]
            .into_iter()
            .filter_map(|k| value.get(k).and_then(Value::as_str))
            .find(|s| !s.is_empty());
        doc.budget.charge_text(label.unwrap_or_default())?;
        let base = label
            .map(normalize_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Environment {}", i + 1));
        // The first use keeps the name; each repeat takes the next free count from 2.
        let mut name = base.clone();
        if !taken.insert(name.clone()) {
            let count = seen.entry(base.clone()).or_insert(1);
            loop {
                *count += 1;
                name = format!("{base} ({count})");
                if taken.insert(name.clone()) {
                    break;
                }
            }
        }
        out.push((name, server));
    }
    Ok(out)
}

/// One server per scheme from Swagger 2.0's `host` and `basePath`; `basePath` alone gives a
/// relative one and neither gives none.
fn swagger_servers(doc: &mut Doc<'_>) -> Result<Vec<(String, Server)>, DocumentError> {
    let root = doc.root;
    let text = |key: &str| root.get(key).and_then(Value::as_str);
    let base_path = text("basePath").unwrap_or_default();
    let (urls, source): (Vec<String>, &str) = match text("host") {
        Some(host) => {
            let mut schemes: Vec<&str> = root
                .get("schemes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if schemes.is_empty() {
                schemes.push("https");
            }
            let each = host.len() + base_path.len() + 3;
            let copied: usize = schemes.iter().map(|s| s.len() + each).sum();
            doc.budget.charge_bytes(copied as u64)?;
            let urls = schemes
                .iter()
                .map(|s| format!("{s}://{host}{base_path}"))
                .collect();
            (urls, "host")
        }
        None if !base_path.is_empty() => {
            doc.budget.charge_bytes(base_path.len() as u64)?;
            (vec![base_path.to_owned()], "basePath")
        }
        None => return Ok(Vec::new()),
    };
    let single = urls.len() == 1;
    Ok(urls
        .into_iter()
        .enumerate()
        .map(|(i, url)| {
            let url = trim_slash(url);
            let name = if single {
                "Environment".to_owned()
            } else {
                format!("Environment {}", i + 1)
            };
            let mut resolved = String::new();
            push_prefix(&mut resolved, &url);
            let server = Server {
                vars: vec![("baseUrl".to_owned(), url, source.to_owned())],
                resolved,
            };
            (name, server)
        })
        .collect())
}

/// A 3.x Server Object read as variables, or `None` when its `url` is not a string. Each
/// declared `{v}` in the URL becomes `{{v}}`, valued by its `default`, else its first `enum`.
fn server(doc: &mut Doc<'_>, value: &Value, source: &str) -> Result<Option<Server>, DocumentError> {
    let Some(url) = value.get("url").and_then(Value::as_str) else {
        return Ok(None);
    };
    let declared = value.get("variables").and_then(Value::as_object);
    let mut values: HashMap<&str, String> = HashMap::new();
    let mut inlined = None;
    let mut vars = vec![(String::new(), String::new(), format!("{source}.url"))];
    for (name, var) in declared.into_iter().flatten() {
        let chosen = var
            .get("default")
            .or_else(|| var.get("enum").and_then(|e| e.get(0)));
        // A value is copied into its variable and into the lookup the URL is filled from.
        let size = chosen.map_or(0, approx_size);
        doc.budget
            .charge_bytes(2 * size + 2 * name.len() as u64 + source.len() as u64 + 16)?;
        let text = chosen.map(js::to_text).unwrap_or_default();
        // A variable named `baseUrl` would collide with the one built from the URL, so its
        // value is written into the URL instead and no variable of its own is kept.
        if name == "baseUrl" {
            inlined = Some(text);
            continue;
        }
        values.insert(name, text.clone());
        vars.push((name.clone(), text, format!("{source}.variables.{name}")));
    }
    let url = match &inlined {
        Some(text) => {
            doc.budget
                .charge_bytes(text.len() as u64 * url.matches("{baseUrl}").count() as u64)?;
            std::borrow::Cow::Owned(url.replace("{baseUrl}", text))
        }
        None => std::borrow::Cow::Borrowed(url),
    };
    // `baseUrl` copies the URL, each brace pair growing by two.
    doc.budget.charge_bytes(2 * url.len() as u64)?;
    let (base, resolved) = substitute(&url, &values);
    vars[0].0 = "baseUrl".to_owned();
    vars[0].1 = trim_slash(base);
    Ok(Some(Server { vars, resolved }))
}

/// `url` with each declared `{v}` written as `{{v}}`, and the first bytes of `url` with each
/// replaced by its value, enough to read the scheme; spelling it out in full could multiply a
/// large default by every use.
fn substitute(url: &str, values: &HashMap<&str, String>) -> (String, String) {
    let mut base = String::with_capacity(url.len());
    let mut resolved = String::new();
    let mut rest = url;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let name = &rest[open + 1..open + close];
        let literal = &rest[open..=open + close];
        base.push_str(&rest[..open]);
        push_prefix(&mut resolved, &rest[..open]);
        match values.get(name) {
            Some(v) => {
                base.push_str(&format!("{{{{{name}}}}}"));
                push_prefix(&mut resolved, v);
            }
            None => {
                base.push_str(literal);
                push_prefix(&mut resolved, literal);
            }
        }
        rest = &rest[open + close + 1..];
    }
    base.push_str(rest);
    push_prefix(&mut resolved, rest);
    (base, resolved)
}

/// Append `text` to `prefix` until it holds `SCHEME_PREFIX` bytes.
fn push_prefix(prefix: &mut String, text: &str) {
    for c in text.chars() {
        if prefix.len() >= SCHEME_PREFIX {
            return;
        }
        prefix.push(c);
    }
}

/// `url` without one trailing `/`.
fn trim_slash(mut url: String) -> String {
    if url.ends_with('/') {
        url.pop();
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_reads_only_the_scheme_of_a_repeated_large_default() {
        // Spelled out in full, this URL would be 10 MB from a 3 KB template.
        let url = "{a}".repeat(1000);
        let values = HashMap::from([("a", "x".repeat(10_000))]);
        let (base, resolved) = substitute(&url, &values);
        assert_eq!(base, "{{a}}".repeat(1000));
        assert!(resolved.len() <= SCHEME_PREFIX, "{} bytes", resolved.len());
        let values = HashMap::from([("s", "https".to_owned()), ("h", "x.test".to_owned())]);
        assert_eq!(substitute("{s}://{h}/v1", &values).1, "https://");
        assert_eq!(substitute("{s}:/", &values).1, "https:/");
        assert_eq!(substitute("ht{u}", &values).1, "ht{u}");
    }
}
