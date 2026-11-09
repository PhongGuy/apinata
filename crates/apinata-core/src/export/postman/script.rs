//! Translation of the app's script API to Postman's on export.

use std::collections::HashSet;

use crate::export::ExportNote;

use scan::{
    MAX_DEPTH, find_close, ident_end, is_ident_byte, is_property_access, parts, prev_significant,
    skip_opaque, skip_ws, template_end,
};

mod alias;
mod cookies;
mod scan;
mod send_request;

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
/// left untranslated; returns the code to write. A script with nothing to translate gets no note.
pub(super) fn for_postman(code: &str, source: &str, notes: &mut Vec<ExportNote>) -> String {
    let out = translate(code);
    let summary = match (out.kept, out.rewritten, out.leftovers.is_empty()) {
        (true, _, _) => Some(UNBALANCED.to_owned()),
        (false, 0, true) => None,
        (false, 1, _) => Some("script translated; 1 call rewritten for Postman".to_owned()),
        (false, n, _) => Some(format!(
            "script translated; {n} calls rewritten for Postman"
        )),
    };
    if let Some(summary) = summary {
        notes.push(ExportNote::new(source, summary));
    }
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
    let code = alias::resolve(code);
    let (code, mut rewritten) = send_request::rewrite(&code);
    let (code, renamed) = cookies::rename_jar_methods(&code);
    rewritten += renamed;
    let code = translate_code(&code, 0, &mut rewritten);
    Translated {
        leftovers: leftovers(&code),
        code,
        rewritten,
        kept: false,
    }
}

/// Whether every bracket outside literals and comments closes in order, and every template
/// and `${}` hole closes within the nesting the translator descends into.
fn balanced(src: &str) -> bool {
    brackets_close(src, 0, Vec::new(), 0) == Some(src.len())
}

// Match brackets from `start` with `open` already pending; a template recurses one level.
// Returns the index just past the bracket that empties `open`, or the end when `open` started
// empty and every bracket closed.
fn brackets_close(src: &str, start: usize, mut open: Vec<u8>, level: usize) -> Option<usize> {
    let b = src.as_bytes();
    let waits_for_close = !open.is_empty();
    let mut i = start;
    while i < b.len() {
        if b[i] == b'`' {
            i = template_closes(src, i, level + 1)?;
            continue;
        }
        if let Some(end) = skip_opaque(src, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' => open.push(b')'),
            b'[' => open.push(b']'),
            b'{' => open.push(b'}'),
            c @ (b')' | b']' | b'}') => {
                if open.pop() != Some(c) {
                    return None;
                }
                if waits_for_close && open.is_empty() {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (!waits_for_close && open.is_empty()).then_some(b.len())
}

/// End of the template at `start` when it and all its holes close and `level` is one the
/// translator still descends into.
fn template_closes(src: &str, start: usize, level: usize) -> Option<usize> {
    if level >= MAX_DEPTH {
        return None;
    }
    let b = src.as_bytes();
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'`' => return Some(i + 1),
            b'$' if b.get(i + 1) == Some(&b'{') => {
                i = brackets_close(src, i + 2, vec![b'}'], level)?;
            }
            _ => i += 1,
        }
    }
    None
}

/// The last significant token `translate_code` passed, as far as statement boundaries need it.
#[derive(Clone, Copy)]
enum Last<'a> {
    None,
    /// A punctuator byte, at its index.
    Punct(usize),
    Word(&'a str),
    /// A number, string, template or regex literal.
    Literal,
    /// A `)`; true when it closes an `if`, `for`, `while` or `with` header.
    Close(bool),
}

/// Words that keep an operator's place before an expression.
const OPERATOR_WORDS: &[&str] = &[
    "typeof",
    "void",
    "delete",
    "await",
    "new",
    "in",
    "instanceof",
];

// Copy literals and comments as they are, descend into template holes, and try a rewrite at
// each identifier that starts a chain rather than continuing one. The last token and the open
// parens are tracked so reshaped calls can respect the statement around them.
fn translate_code(src: &str, depth: usize, rewritten: &mut usize) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut last = Last::None;
    let mut last_end = 0;
    let mut parens: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            i = copy_template(src, i, depth, &mut out, rewritten);
            (last, last_end) = (Last::Literal, i);
        } else if let Some(end) = skip_opaque(src, i) {
            out.push_str(&src[i..end]);
            if !(b[i] == b'/' && matches!(b.get(i + 1), Some(b'/' | b'*'))) {
                (last, last_end) = (Last::Literal, end);
            }
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if !is_property_access(b, i)
                && let Some(found) = rewrite_chain(src, i, end, depth, rewritten)
            {
                let semicolon = needs_semicolon(src, last, last_end, i);
                match found.shape {
                    Shape::Assign if needs_parens(src, last, found.next) => {
                        if semicolon {
                            out.push(';');
                        }
                        out.push('(');
                        out.push_str(&found.text);
                        out.push(')');
                    }
                    Shape::Loop if semicolon => {
                        out.push(';');
                        out.push_str(&found.text);
                    }
                    _ => out.push_str(&found.text),
                }
                *rewritten += 1;
                last = if b[found.next - 1] == b')' {
                    Last::Close(false)
                } else {
                    Last::Word("")
                };
                (i, last_end) = (found.next, found.next);
                continue;
            }
            out.push_str(&src[i..end]);
            last = if b[i].is_ascii_digit() {
                Last::Literal
            } else {
                Last::Word(&src[i..end])
            };
            (i, last_end) = (end, end);
        } else {
            if !b[i].is_ascii_whitespace() {
                last = match b[i] {
                    b'(' => {
                        let header = matches!(last, Last::Word("if" | "for" | "while" | "with"));
                        parens.push(header);
                        Last::Punct(i)
                    }
                    b')' => Last::Close(parens.pop().unwrap_or(false)),
                    _ => Last::Punct(i),
                };
                last_end = i + 1;
            }
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

// An assignment can stand bare where the token before it starts an expression slot and the
// token after it ends that expression; anywhere else a neighbouring operator would bind it.
fn needs_parens(src: &str, last: Last, end: usize) -> bool {
    let b = src.as_bytes();
    let before = |k: usize, n: usize| k.checked_sub(n).map(|j| b[j]);
    let bare_before = match last {
        Last::None | Last::Close(_) => true,
        Last::Word(w) => !OPERATOR_WORDS.contains(&w),
        Last::Literal => false,
        Last::Punct(k) => match b[k] {
            b';' | b'{' | b'}' | b'(' | b'[' | b',' | b':' => true,
            b'?' => before(k, 1) != Some(b'?'),
            b'>' => before(k, 1) == Some(b'='),
            b'=' => match before(k, 1) {
                Some(b'=' | b'!') => false,
                Some(c @ (b'<' | b'>')) => before(k, 2) == Some(c),
                _ => true,
            },
            _ => false,
        },
    };
    let (next, newline) = skip_trivia(src, end);
    let bare_after = match b.get(next) {
        None | Some(b';' | b',' | b')' | b']' | b'}' | b':') => true,
        Some(&c) => newline && (is_ident_byte(c) || matches!(c, b'\'' | b'"' | b'`' | b'{')),
    };
    !(bare_before && bare_after)
}

// New text starting a line continues the previous statement unless that one ended where ASI
// would end it anyway; after a value-like token, a leading `;` keeps the statements apart.
fn needs_semicolon(src: &str, last: Last, last_end: usize, start: usize) -> bool {
    let newline = src[last_end..start].contains('\n');
    newline
        && match last {
            Last::None => false,
            Last::Punct(k) => matches!(src.as_bytes()[k], b']' | b'}'),
            Last::Literal => true,
            Last::Word(w) => !matches!(w, "else" | "do" | "return") && !OPERATOR_WORDS.contains(&w),
            Last::Close(header) => !header,
        }
}

/// The first byte at or after `i` outside whitespace and comments, and whether a line break
/// was passed on the way.
fn skip_trivia(src: &str, mut i: usize) -> (usize, bool) {
    let b = src.as_bytes();
    let mut newline = false;
    loop {
        let k = skip_ws(b, i);
        newline |= src[i..k].contains('\n');
        i = k;
        match (b.get(i), b.get(i + 1)) {
            (Some(b'/'), Some(b'/' | b'*')) => {
                let end = skip_opaque(src, i).unwrap_or(b.len());
                newline |= src[i..end].contains('\n');
                i = end;
            }
            _ => return (i, newline),
        }
    }
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

/// Calls whose arguments move into new syntax; rewritten only when called.
const RESHAPED: &[&str] = &[
    "req.setUrl",
    "req.setMethod",
    "req.setHeader",
    "req.setBody",
    "req.setHeaders",
    "JSON.stringify",
];

/// The statement shape a rewrite produces.
enum Shape {
    Expr,
    /// `pm.request.url = x`, which an operator next to it would bind.
    Assign,
    /// A call of a function expression, which a previous unterminated line would call.
    Loop,
}

struct Rewrite {
    text: String,
    next: usize,
    shape: Shape,
}

// Gather the `.name` segments that follow the head with no space between, then try the
// longest key first; at each length a call row needs a `(` and wins over a plain row.
fn rewrite_chain(
    src: &str,
    start: usize,
    head_end: usize,
    depth: usize,
    rewritten: &mut usize,
) -> Option<Rewrite> {
    if !matches!(
        &src[start..head_end],
        "bru" | "req" | "res" | "expect" | "JSON"
    ) {
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
        let open = skip_ws(b, end);
        let close = (b.get(open) == Some(&b'('))
            .then(|| find_close(src, open))
            .flatten();
        if RESHAPED.contains(&key) {
            let close = close?;
            let (text, shape) = reshape(key, src, open, close, depth, rewritten)?;
            return Some(Rewrite {
                text,
                next: close + 1,
                shape,
            });
        }
        if let Some((_, text)) = CALLS.iter().find(|(k, _)| *k == key)
            && let Some(close) = close
        {
            return Some(Rewrite {
                text: (*text).to_owned(),
                next: close + 1,
                shape: Shape::Expr,
            });
        }
        if let Some((_, text)) = SIMPLE.iter().find(|(k, _)| *k == key) {
            return Some(Rewrite {
                text: (*text).to_owned(),
                next: end,
                shape: Shape::Expr,
            });
        }
    }
    None
}

// Move a reshaped call's arguments into Postman's syntax, later arguments dropped. A spread
// where an argument would land inside new syntax keeps the call as written.
fn reshape(
    key: &str,
    src: &str,
    open: usize,
    close: usize,
    depth: usize,
    rewritten: &mut usize,
) -> Option<(String, Shape)> {
    let args = parts(src, open + 1, close);
    let spread = |n: usize| {
        args.get(n)
            .is_some_and(|&(s, e)| src[s..e].starts_with("..."))
    };
    let mut arg = |n: usize| {
        let (s, e) = args[n];
        if depth + 1 >= MAX_DEPTH {
            src[s..e].to_owned()
        } else {
            translate_code(&src[s..e], depth + 1, rewritten)
        }
    };
    Some(match key {
        "req.setUrl" | "req.setMethod" => {
            let prop = if key == "req.setUrl" {
                "pm.request.url"
            } else {
                "pm.request.method"
            };
            if args.is_empty() {
                (prop.to_owned(), Shape::Expr)
            } else if spread(0) {
                return None;
            } else {
                (format!("{prop} = {}", arg(0)), Shape::Assign)
            }
        }
        "req.setHeader" if args.len() >= 2 => {
            if spread(0) || spread(1) {
                return None;
            }
            let (k, v) = (arg(0), arg(1));
            (
                format!("pm.request.headers.upsert({{ key: {k}, value: {v} }})"),
                Shape::Expr,
            )
        }
        "req.setHeader" => {
            let passed: Vec<String> = (0..args.len()).map(&mut arg).collect();
            (
                format!("pm.request.headers.upsert({})", passed.join(", ")),
                Shape::Expr,
            )
        }
        "req.setBody" if args.is_empty() => ("pm.request.body".to_owned(), Shape::Expr),
        "req.setBody" => {
            if spread(0) {
                return None;
            }
            (
                format!(
                    "pm.request.body.update({{ mode: \"raw\", raw: JSON.stringify({}) }})",
                    arg(0)
                ),
                Shape::Expr,
            )
        }
        "req.setHeaders" if args.is_empty() => ("pm.request.headers".to_owned(), Shape::Expr),
        "req.setHeaders" => {
            if spread(0) {
                return None;
            }
            (
                format!(
                    "(function() {{ const _headers = {}; for (const key in _headers) {{ pm.request.headers.upsert({{ key: key, value: _headers[key] }}); }} }})()",
                    arg(0)
                ),
                Shape::Loop,
            )
        }
        _ => {
            let [(s, e)] = args.as_slice() else {
                return None;
            };
            response_body_call(src, *s, *e)
                .then(|| ("pm.response.text()".to_owned(), Shape::Expr))?
        }
    })
}

/// Whether `src[s..e]` is wholly a `res.getBody(...)` call.
fn response_body_call(src: &str, s: usize, e: usize) -> bool {
    let b = src.as_bytes();
    let name_end = s + "res.getBody".len();
    if !src[s..e].starts_with("res.getBody") || ident_end(b, name_end) != name_end {
        return false;
    }
    let open = skip_ws(b, name_end);
    b.get(open) == Some(&b'(') && find_close(src, open) == Some(e - 1)
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
            if b[i] == b'`' {
                // A template the translator could not descend into is scanned as plain text,
                // so calls swallowed by an unclosed hole or deep nesting are still reported.
                if template_closes(code, i, depth + 1).is_some() {
                    template_holes(code, i, end, depth + 1, each);
                } else {
                    flat_chains(&code[i..end], each);
                }
            }
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            i = report_chain(code, i, end, each);
        } else {
            i += 1;
        }
    }
}

/// Every chain in `code`, ignoring literals and comments: a fallback for text whose structure
/// could not be followed.
fn flat_chains(code: &str, each: &mut dyn FnMut(String)) {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            i = report_chain(code, i, end, each);
        } else {
            i += 1;
        }
    }
}

// Report the chain headed by the identifier at `start..head_end` when it is a free-standing
// `bru`, `req` or `res`; segments may be spaced or optional (`bru ?. getVar`). Returns where
// scanning resumes.
fn report_chain(code: &str, start: usize, head_end: usize, each: &mut dyn FnMut(String)) -> usize {
    let b = code.as_bytes();
    let head = &code[start..head_end];
    if !matches!(head, "bru" | "req" | "res") || is_property_access(b, start) {
        return head_end;
    }
    let mut chain = head.to_owned();
    let mut next = head_end;
    loop {
        let dot = skip_ws(b, next);
        let after = match (b.get(dot), b.get(dot + 1)) {
            (Some(b'.'), _) => dot + 1,
            (Some(b'?'), Some(b'.')) => dot + 2,
            _ => break,
        };
        let name = skip_ws(b, after);
        if !b
            .get(name)
            .is_some_and(|c| is_ident_byte(*c) && !c.is_ascii_digit())
        {
            break;
        }
        next = ident_end(b, name);
        chain.push('.');
        chain.push_str(&code[name..next]);
    }
    if next > head_end || bare_alias(b, start, head_end) {
        each(chain);
    }
    next
}

// A head with no segment that is the whole right-hand side of a plain `=` aliases the API;
// one the resolver left in place would reach Postman untranslated.
fn bare_alias(b: &[u8], start: usize, end: usize) -> bool {
    let assigned = match prev_significant(b, start) {
        Some((k, b'=')) => !matches!(
            k.checked_sub(1).map(|j| b[j]),
            Some(b'=' | b'!' | b'<' | b'>')
        ),
        _ => false,
    };
    let mut k = end;
    while k < b.len() && matches!(b[k], b' ' | b'\t' | b'\r') {
        k += 1;
    }
    assigned
        && b.get(k)
            .is_none_or(|c| matches!(c, b';' | b',' | b'}' | b'\n'))
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
