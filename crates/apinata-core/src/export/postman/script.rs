//! Translation of the app's script API to Postman's on export.

use std::collections::HashSet;

use crate::export::ExportNote;

use scan::{
    MAX_DEPTH, find_close, ident_end, is_ident_byte, is_property_access, skip_opaque, skip_ws,
    template_end,
};

mod scan;

#[cfg(test)]
mod corpus;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tokens;

/// Longest member chain examined; the longest table key has far fewer segments.
const MAX_SEGS: usize = 16;

/// Result of translating one script.
pub(super) struct Translated {
    pub code: String,
    /// Rewrites made.
    pub rewritten: usize,
    /// Distinct `bru.`/`req.`/`res.` chains left in the output, first-seen order.
    pub leftovers: Vec<String>,
    /// The brackets did not balance, so `code` is the input unchanged.
    pub kept: bool,
}

/// One-to-one chain rewrites; the longest matching prefix of a chain wins.
pub(super) const SIMPLE: &[(&str, &str)] = &[
    ("bru.getGlobalEnvVar", "pm.globals.get"),
    ("bru.setGlobalEnvVar", "pm.globals.set"),
    ("bru.hasGlobalEnvVar", "pm.globals.has"),
    ("bru.deleteGlobalEnvVar", "pm.globals.unset"),
    ("bru.getAllGlobalEnvVars", "pm.globals.toObject"),
    ("bru.deleteAllGlobalEnvVars", "pm.globals.clear"),
    ("bru.getEnvVar", "pm.environment.get"),
    ("bru.setEnvVar", "pm.environment.set"),
    ("bru.hasEnvVar", "pm.environment.has"),
    ("bru.deleteEnvVar", "pm.environment.unset"),
    ("bru.getAllEnvVars", "pm.environment.toObject"),
    ("bru.deleteAllEnvVars", "pm.environment.clear"),
    ("bru.getVar", "pm.variables.get"),
    ("bru.setVar", "pm.variables.set"),
    ("bru.hasVar", "pm.variables.has"),
    ("bru.deleteVar", "pm.variables.unset"),
    ("bru.getAllVars", "pm.variables.toObject"),
    ("bru.getCollectionVar", "pm.collectionVariables.get"),
    ("bru.setCollectionVar", "pm.collectionVariables.set"),
    ("bru.hasCollectionVar", "pm.collectionVariables.has"),
    ("bru.deleteCollectionVar", "pm.collectionVariables.unset"),
    (
        "bru.getAllCollectionVars",
        "pm.collectionVariables.toObject",
    ),
    (
        "bru.deleteAllCollectionVars",
        "pm.collectionVariables.clear",
    ),
    ("bru.getFolderVar", "pm.variables.get"),
    ("bru.getRequestVar", "pm.variables.get"),
    ("bru.interpolate", "pm.variables.replaceIn"),
    ("bru.setNextRequest", "pm.execution.setNextRequest"),
    ("bru.runner.skipRequest", "pm.execution.skipRequest"),
    ("bru.runner.setNextRequest", "pm.execution.setNextRequest"),
    ("req.url", "pm.request.url"),
    ("req.method", "pm.request.method"),
    ("req.headers", "pm.request.headers"),
    ("req.body", "pm.request.body"),
    ("req.getHeader", "pm.request.headers.get"),
    ("req.deleteHeader", "pm.request.headers.remove"),
    ("req.headerList", "pm.request.headers"),
    ("req.headerList.get", "pm.request.headers.get"),
    ("req.headerList.has", "pm.request.headers.has"),
    ("req.headerList.one", "pm.request.headers.one"),
    ("req.headerList.all", "pm.request.headers.all"),
    ("req.headerList.count", "pm.request.headers.count"),
    ("req.headerList.indexOf", "pm.request.headers.indexOf"),
    ("req.headerList.find", "pm.request.headers.find"),
    ("req.headerList.filter", "pm.request.headers.filter"),
    ("req.headerList.each", "pm.request.headers.each"),
    ("req.headerList.map", "pm.request.headers.map"),
    ("req.headerList.reduce", "pm.request.headers.reduce"),
    ("req.headerList.toObject", "pm.request.headers.toObject"),
    ("req.headerList.toString", "pm.request.headers.toString"),
    ("req.headerList.toJSON", "pm.request.headers.toJSON"),
    ("req.headerList.add", "pm.request.headers.add"),
    ("req.headerList.upsert", "pm.request.headers.upsert"),
    ("req.headerList.remove", "pm.request.headers.remove"),
    ("req.headerList.clear", "pm.request.headers.clear"),
    ("req.headerList.populate", "pm.request.headers.populate"),
    ("req.headerList.repopulate", "pm.request.headers.repopulate"),
    ("req.headerList.assimilate", "pm.request.headers.assimilate"),
    ("req.getHost", "pm.request.url.getHost"),
    ("req.getPath", "pm.request.url.getPath"),
    ("req.getQueryString", "pm.request.url.getQueryString"),
    ("res.status", "pm.response.code"),
    ("res.statusText", "pm.response.status"),
    ("res.body", "pm.response.body"),
    ("res.url", "pm.response.url"),
    ("res.responseTime", "pm.response.responseTime"),
    ("res.headers", "pm.response.headers"),
    ("res.getBody", "pm.response.json"),
    ("res.getHeader", "pm.response.headers.get"),
    ("res.getSize", "pm.response.size"),
    ("res.headerList", "pm.response.headers"),
    ("res.headerList.get", "pm.response.headers.get"),
    ("res.headerList.has", "pm.response.headers.has"),
    ("res.headerList.one", "pm.response.headers.one"),
    ("res.headerList.all", "pm.response.headers.all"),
    ("res.headerList.count", "pm.response.headers.count"),
    ("res.headerList.indexOf", "pm.response.headers.indexOf"),
    ("res.headerList.find", "pm.response.headers.find"),
    ("res.headerList.filter", "pm.response.headers.filter"),
    ("res.headerList.each", "pm.response.headers.each"),
    ("res.headerList.map", "pm.response.headers.map"),
    ("res.headerList.reduce", "pm.response.headers.reduce"),
    ("res.headerList.toObject", "pm.response.headers.toObject"),
    ("res.headerList.toString", "pm.response.headers.toString"),
    ("res.headerList.toJSON", "pm.response.headers.toJSON"),
    ("bru.cookies.jar", "pm.cookies.jar"),
    ("bru.cookies.get", "pm.cookies.get"),
    ("bru.cookies.has", "pm.cookies.has"),
    ("bru.cookies.toObject", "pm.cookies.toObject"),
    ("bru.cookies.toString", "pm.cookies.toString"),
    ("bru.cookies.clear", "pm.cookies.clear"),
    ("bru.cookies.delete", "pm.cookies.remove"),
    ("bru.cookies.one", "pm.cookies.one"),
    ("bru.cookies.all", "pm.cookies.all"),
    ("bru.cookies.idx", "pm.cookies.idx"),
    ("bru.cookies.count", "pm.cookies.count"),
    ("bru.cookies.indexOf", "pm.cookies.indexOf"),
    ("bru.cookies.find", "pm.cookies.find"),
    ("bru.cookies.filter", "pm.cookies.filter"),
    ("bru.cookies.each", "pm.cookies.each"),
    ("bru.cookies.map", "pm.cookies.map"),
    ("bru.cookies.reduce", "pm.cookies.reduce"),
    ("bru.cookies.add", "pm.cookies.add"),
    ("bru.cookies.upsert", "pm.cookies.upsert"),
    ("bru.cookies.remove", "pm.cookies.remove"),
    ("expect.fail", "pm.expect.fail"),
];

/// A called key becomes the text, its arguments and closing paren included.
pub(super) const CALLS: &[(&str, &str)] = &[
    ("bru.getEnvName", "pm.environment.name"),
    ("req.getUrl", "pm.request.url"),
    ("req.getMethod", "pm.request.method"),
    ("req.getHeaders", "pm.request.headers"),
    ("req.getBody", "pm.request.body"),
    ("req.getName", "pm.info.requestName"),
    ("req.getAuthMode", "pm.request.auth.type"),
    ("req.getPathParams", "pm.request.url.variables"),
    ("res.getStatus", "pm.response.code"),
    ("res.getStatusText", "pm.response.status"),
    ("res.getResponseTime", "pm.response.responseTime"),
    ("res.getHeaders", "pm.response.headers"),
    ("res.getUrl", "pm.response.url"),
    (
        "bru.runner.stopExecution",
        "pm.execution.setNextRequest(null)",
    ),
];

const UNBALANCED: &str =
    "script kept as written; its brackets do not balance, so no calls were rewritten";

/// Translate `code` for Postman, pushing a summary note at `source` and one note per call
/// left untranslated; returns the code to write.
pub(super) fn for_postman(code: &str, source: &str, notes: &mut Vec<ExportNote>) -> String {
    let out = translate(code);
    let summary = match (out.kept, out.rewritten) {
        (true, _) => UNBALANCED.to_owned(),
        (false, 1) => "script translated; 1 call rewritten for Postman".to_owned(),
        (false, n) => format!("script translated; {n} calls rewritten for Postman"),
    };
    notes.push(ExportNote::new(source, summary));
    for chain in &out.leftovers {
        let detail = format!(
            "uses `{chain}`, which was not translated for Postman; the line was kept as written"
        );
        notes.push(ExportNote::new(source, detail));
    }
    out.code
}

/// Translate a script's code. Never fails: anything not in the tables is kept as written.
pub(super) fn translate(code: &str) -> Translated {
    // A rewrite inside unbalanced brackets could land anywhere, so such a script is kept whole.
    if !balanced(code) {
        return Translated {
            code: code.to_owned(),
            rewritten: 0,
            leftovers: leftovers(code),
            kept: true,
        };
    }
    let mut rewritten = 0;
    let code = translate_code(code, 0, &mut rewritten);
    Translated {
        leftovers: leftovers(&code),
        code,
        rewritten,
        kept: false,
    }
}

/// Whether every bracket outside literals and comments closes in order.
fn balanced(src: &str) -> bool {
    let b = src.as_bytes();
    let mut open = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(src, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' => open.push(b')'),
            b'[' => open.push(b']'),
            b'{' => open.push(b'}'),
            c @ (b')' | b']' | b'}') if open.pop() != Some(c) => return false,
            _ => {}
        }
        i += 1;
    }
    open.is_empty()
}

// Copy literals and comments as they are, descend into template holes, and try a rewrite at
// each identifier that starts a chain rather than continuing one.
fn translate_code(src: &str, depth: usize, rewritten: &mut usize) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            i = copy_template(src, i, depth, &mut out, rewritten);
        } else if let Some(end) = skip_opaque(src, i) {
            out.push_str(&src[i..end]);
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if !is_property_access(b, i)
                && let Some((text, next)) = rewrite_chain(src, i, end)
            {
                out.push_str(text);
                *rewritten += 1;
                i = next;
                continue;
            }
            out.push_str(&src[i..end]);
            i = end;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// Copy a template literal, translating only its `${}` holes; past the depth cap, verbatim.
fn copy_template(
    src: &str,
    start: usize,
    depth: usize,
    out: &mut String,
    rewritten: &mut usize,
) -> usize {
    let b = src.as_bytes();
    let end = template_end(src, start);
    let mut i = start + 1;
    let mut text_from = start;
    while i < end && depth + 1 < MAX_DEPTH {
        match b[i] {
            b'\\' => i += 2,
            b'$' if b.get(i + 1) == Some(&b'{') => {
                let Some(close) = find_close(src, i + 1) else {
                    break;
                };
                out.push_str(&src[text_from..i + 2]);
                out.push_str(&translate_code(&src[i + 2..close], depth + 1, rewritten));
                out.push('}');
                i = close + 1;
                text_from = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&src[text_from.min(end)..end]);
    end
}

// Gather the `.name` segments that follow the head with no space between, then try the
// longest key first; at each length a call row needs a `(` and wins over a plain row.
fn rewrite_chain(src: &str, start: usize, head_end: usize) -> Option<(&'static str, usize)> {
    if !matches!(&src[start..head_end], "bru" | "req" | "res" | "expect") {
        return None;
    }
    let b = src.as_bytes();
    let mut ends = Vec::new();
    let mut pos = head_end;
    while ends.len() < MAX_SEGS
        && b.get(pos) == Some(&b'.')
        && b.get(pos + 1).is_some_and(|c| is_ident_byte(*c))
    {
        pos = ident_end(b, pos + 1);
        ends.push(pos);
    }
    for &end in ends.iter().rev() {
        let key = &src[start..end];
        if let Some((_, text)) = CALLS.iter().find(|(k, _)| *k == key) {
            let open = skip_ws(b, end);
            if b.get(open) == Some(&b'(')
                && let Some(close) = find_close(src, open)
            {
                return Some((text, close + 1));
            }
        }
        if let Some((_, text)) = SIMPLE.iter().find(|(k, _)| *k == key) {
            return Some((text, end));
        }
    }
    None
}

/// The distinct `bru.`/`req.`/`res.` member chains in `code`, template holes included.
fn leftovers(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    each_chain(code, 0, &mut |chain| {
        if seen.insert(chain.clone()) {
            found.push(chain);
        }
    });
    found
}

fn each_chain(code: &str, depth: usize, each: &mut dyn FnMut(String)) {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(code, i) {
            if b[i] == b'`' && depth + 1 < MAX_DEPTH {
                template_holes(code, i, end, depth + 1, each);
            }
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            let mut next = end;
            if matches!(&code[i..end], "bru" | "req" | "res") && !is_property_access(b, i) {
                while b.get(next) == Some(&b'.')
                    && b.get(next + 1).is_some_and(|c| is_ident_byte(*c))
                {
                    next = ident_end(b, next + 1);
                }
                if next > end {
                    each(code[i..next].to_owned());
                }
            }
            i = next;
        } else {
            i += 1;
        }
    }
}

fn template_holes(
    code: &str,
    start: usize,
    end: usize,
    depth: usize,
    each: &mut dyn FnMut(String),
) {
    let b = code.as_bytes();
    let mut i = start + 1;
    while i < end {
        match b[i] {
            b'\\' => i += 2,
            b'$' if b.get(i + 1) == Some(&b'{') => {
                let Some(close) = find_close(code, i + 1) else {
                    return;
                };
                each_chain(&code[i + 2..close], depth, each);
                i = close + 1;
            }
            _ => i += 1,
        }
    }
}
