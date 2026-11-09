//! Translation of Postman `pm.*` script calls to the runtime's `bru`/`req`/`res`
//! vocabulary. Follows `packages/bruno-converters/src/utils/postman-to-bruno-translator.js`
//! (usebruno/bruno @1301ea17) without an AST: a token-aware scan leaves strings,
//! comments and template text alone and rewrites member chains at identifier
//! boundaries. Code the table cannot express passes through unchanged. Aliases
//! (`const r = pm.response`) are resolved first, in `alias`; `pm.sendRequest` is rewritten
//! in `send_request`, with `scope` telling it where `await` is legal. The `pm.response.to.*`
//! status helpers are rewritten only as whole statements. Not ported: `pm.require`, the
//! content-type helpers (`to.be.json`, `to.be.html`) and anything else reported as unsupported.

use std::collections::HashSet;

use crate::js_scan::punct_regex::{find_close, skip_opaque, template_end};
use crate::js_scan::{
    MAX_DEPTH, ident_end, is_ident_byte, is_property_access, prev_significant, skip_trivia, skip_ws,
};
#[cfg(test)]
use crate::js_scan::{NESTING, NestingGuard};

mod alias;
mod scope;
mod send_request;

/// Result of translating one script. `unsupported` lists the distinct `pm.*` chains
/// still present afterwards, `replaced` counts the rewrites made.
pub(super) struct Translated {
    pub code: String,
    pub unsupported: Vec<String>,
    pub replaced: usize,
}

/// Translate a script's code. Never fails: anything not recognised is kept as written.
pub(super) fn translate(exec: &str) -> Translated {
    let mut cx = Ctx::new(exec);
    let resolved = alias::resolve(exec, &mut cx);
    cx.tab = guess_tab_width(&resolved);
    let resolved = send_request::hoist_declared_options(&resolved, &mut cx);
    let code = translate_code(&resolved, &mut cx);
    let mut unsupported = Vec::new();
    collect_unsupported(&code, &mut unsupported, &mut HashSet::new());
    Translated {
        code,
        unsupported,
        replaced: cx.replaced,
    }
}

/// Longest member chain examined; the longest table key has far fewer segments.
const MAX_SEGS: usize = 16;
/// State shared across one translation: rewrite count, nesting depth, the legacy
/// names the whole script declares (computed once) and a scan budget that keeps
/// repeated unclosed calls from rescanning the rest of the script each time.
struct Ctx {
    replaced: usize,
    depth: usize,
    declared: [bool; 3],
    budget: usize,
    /// Whether `await` is legal where the scan currently is.
    await_ok: bool,
    /// Whether a statement may begin at the identifier being examined.
    at_stmt: bool,
    /// Indentation step the reference's printer would use for this script.
    tab: usize,
    /// The function the scan is in, the last id handed out, and the functions that got
    /// an `await` inserted (a promise-chain handler holding one must become `async`).
    fid: u32,
    next_fid: u32,
    awaited: HashSet<u32>,
}

/// Width in columns of the leading blanks of `line`; a tab reaches the next multiple of 4.
fn columns(line: &[u8]) -> usize {
    let mut col = 0;
    for c in line {
        match c {
            b' ' => col += 1,
            b'\t' => col = (col / 4 + 1) * 4,
            _ => break,
        }
    }
    col
}

/// The printer's indentation step: the commonest change of indent between consecutive
/// lines, smallest first, 2 when the script has none.
fn guess_tab_width(src: &str) -> usize {
    let mut counts = [0usize; 65];
    let mut last = 0usize;
    for line in src.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let indent = columns(line.as_bytes());
        let diff = indent.abs_diff(last);
        if diff < counts.len() {
            counts[diff] += 1;
        }
        last = indent;
    }
    let mut best = (0, 2);
    for (width, n) in counts.iter().enumerate().skip(1) {
        if *n > best.0 {
            best = (*n, width);
        }
    }
    best.1
}

const LEGACY: [&str; 3] = ["responseBody", "responseHeaders", "responseTime"];

impl Ctx {
    fn new(script: &str) -> Self {
        Ctx {
            replaced: 0,
            depth: 0,
            declared: LEGACY.map(|n| declares(script, n)),
            budget: script.len().saturating_mul(8).saturating_add(1 << 16),
            await_ok: true,
            at_stmt: false,
            tab: 2,
            fid: 0,
            next_fid: 0,
            awaited: HashSet::new(),
        }
    }

    fn declares(&self, name: &str) -> bool {
        LEGACY
            .iter()
            .position(|n| *n == name)
            .is_some_and(|i| self.declared[i])
    }

    /// `find_close` that charges the scanned length against the budget.
    fn find_close(&mut self, src: &str, open: usize) -> Option<usize> {
        if self.budget == 0 {
            return None;
        }
        let close = find_close(src, open);
        let spent = close.unwrap_or(src.len()) - open;
        self.budget = self.budget.saturating_sub(spent);
        close
    }
}

/// One-to-one chain rewrites, keyed by the `pm.`-normalised chain (the reference's
/// `simpleTranslations`). The longest matching prefix of a chain wins.
const SIMPLE: &[(&str, &str)] = &[
    ("pm.globals.get", "bru.getGlobalEnvVar"),
    ("pm.globals.set", "bru.setGlobalEnvVar"),
    ("pm.globals.has", "bru.hasGlobalEnvVar"),
    ("pm.globals.replaceIn", "bru.interpolate"),
    ("pm.globals.unset", "bru.deleteGlobalEnvVar"),
    ("pm.globals.toObject", "bru.getAllGlobalEnvVars"),
    ("pm.globals.clear", "bru.deleteAllGlobalEnvVars"),
    ("pm.environment.get", "bru.getEnvVar"),
    ("pm.environment.set", "bru.setEnvVar"),
    ("pm.environment.name", "bru.getEnvName()"),
    ("pm.environment.unset", "bru.deleteEnvVar"),
    ("pm.environment.replaceIn", "bru.interpolate"),
    ("pm.environment.toObject", "bru.getAllEnvVars"),
    ("pm.environment.clear", "bru.deleteAllEnvVars"),
    ("pm.variables.get", "bru.getVar"),
    ("pm.variables.set", "bru.setVar"),
    ("pm.variables.has", "bru.hasVar"),
    ("pm.variables.toObject", "bru.getAllVars"),
    ("pm.variables.replaceIn", "bru.interpolate"),
    ("pm.collectionVariables.get", "bru.getCollectionVar"),
    ("pm.collectionVariables.set", "bru.setCollectionVar"),
    ("pm.collectionVariables.has", "bru.hasCollectionVar"),
    ("pm.collectionVariables.unset", "bru.deleteCollectionVar"),
    ("pm.collectionVariables.replaceIn", "bru.interpolate"),
    (
        "pm.collectionVariables.clear",
        "bru.deleteAllCollectionVars",
    ),
    (
        "pm.collectionVariables.toObject",
        "bru.getAllCollectionVars",
    ),
    ("pm.test", "test"),
    ("pm.expect", "expect"),
    ("pm.expect.fail", "expect.fail"),
    ("pm.info.requestName", "req.getName()"),
    ("pm.request.headers.remove", "req.deleteHeader"),
    ("pm.request.headers.get", "req.headerList.get"),
    ("pm.request.headers.has", "req.headerList.has"),
    ("pm.request.headers.one", "req.headerList.one"),
    ("pm.request.headers.all", "req.headerList.all"),
    ("pm.request.headers.count", "req.headerList.count"),
    ("pm.request.headers.indexOf", "req.headerList.indexOf"),
    ("pm.request.headers.find", "req.headerList.find"),
    ("pm.request.headers.filter", "req.headerList.filter"),
    ("pm.request.headers.each", "req.headerList.each"),
    ("pm.request.headers.map", "req.headerList.map"),
    ("pm.request.headers.reduce", "req.headerList.reduce"),
    ("pm.request.headers.toObject", "req.headerList.toObject"),
    ("pm.request.headers.toString", "req.headerList.toString"),
    ("pm.request.headers.toJSON", "req.headerList.toJSON"),
    ("pm.request.headers.clear", "req.headerList.clear"),
    ("pm.response.headers.has", "res.headerList.has"),
    ("pm.response.headers.one", "res.headerList.one"),
    ("pm.response.headers.all", "res.headerList.all"),
    ("pm.response.headers.count", "res.headerList.count"),
    ("pm.response.headers.indexOf", "res.headerList.indexOf"),
    ("pm.response.headers.find", "res.headerList.find"),
    ("pm.response.headers.filter", "res.headerList.filter"),
    ("pm.response.headers.each", "res.headerList.each"),
    ("pm.response.headers.map", "res.headerList.map"),
    ("pm.response.headers.reduce", "res.headerList.reduce"),
    ("pm.response.headers.toObject", "res.headerList.toObject"),
    ("pm.response.headers.toString", "res.headerList.toString"),
    ("pm.response.headers.toJSON", "res.headerList.toJSON"),
    ("pm.request.url.getHost", "req.getHost"),
    ("pm.request.url.getPath", "req.getPath"),
    ("pm.request.url.getQueryString", "req.getQueryString"),
    ("pm.request.url.variables", "req.getPathParams()"),
    ("pm.request.url", "req.getUrl()"),
    ("pm.request.method", "req.getMethod()"),
    ("pm.request.headers", "req.getHeaders()"),
    ("pm.request.body", "req.getBody()"),
    ("request.url", "req.getUrl()"),
    ("request.method", "req.getMethod()"),
    ("request.headers", "req.getHeaders()"),
    ("request.body", "req.getBody()"),
    ("request.name", "req.getName()"),
    ("pm.response.json", "res.getBody"),
    ("pm.response.code", "res.getStatus()"),
    ("pm.response.status", "res.statusText"),
    ("pm.response.responseTime", "res.getResponseTime()"),
    ("pm.response.statusText", "res.statusText"),
    ("pm.response.headers", "res.getHeaders()"),
    ("pm.response.size", "res.getSize"),
    ("pm.response.responseSize", "res.getSize().body"),
    ("pm.response.size().body", "res.getSize().body"),
    ("pm.response.size().header", "res.getSize().header"),
    ("pm.response.size().total", "res.getSize().total"),
    ("pm.cookies.jar", "bru.cookies.jar"),
    ("pm.cookies.jar().get", "bru.cookies.jar().getCookie"),
    ("pm.cookies.jar().getAll", "bru.cookies.jar().getCookies"),
    ("pm.cookies.jar().set", "bru.cookies.jar().setCookie"),
    ("pm.cookies.jar().unset", "bru.cookies.jar().deleteCookie"),
    ("pm.cookies.jar().clear", "bru.cookies.jar().deleteCookies"),
    ("pm.cookies.get", "bru.cookies.get"),
    ("pm.cookies.has", "bru.cookies.has"),
    ("pm.cookies.toObject", "bru.cookies.toObject"),
    ("pm.cookies.toString", "bru.cookies.toString"),
    ("pm.cookies.clear", "bru.cookies.clear"),
    ("pm.cookies.remove", "bru.cookies.delete"),
    ("pm.cookies.one", "bru.cookies.one"),
    ("pm.cookies.all", "bru.cookies.all"),
    ("pm.cookies.idx", "bru.cookies.idx"),
    ("pm.cookies.count", "bru.cookies.count"),
    ("pm.cookies.indexOf", "bru.cookies.indexOf"),
    ("pm.cookies.find", "bru.cookies.find"),
    ("pm.cookies.filter", "bru.cookies.filter"),
    ("pm.cookies.each", "bru.cookies.each"),
    ("pm.cookies.map", "bru.cookies.map"),
    ("pm.cookies.reduce", "bru.cookies.reduce"),
    ("pm.cookies.add", "bru.cookies.add"),
    ("pm.cookies.upsert", "bru.cookies.upsert"),
    ("pm.cookies.prepend", "bru.cookies.add"),
    ("pm.cookies.insert", "bru.cookies.add"),
    ("pm.cookies.insertAfter", "bru.cookies.add"),
    ("pm.execution.skipRequest", "bru.runner.skipRequest"),
    ("pm.setEnvironmentVariable", "bru.setEnvVar"),
    ("pm.getEnvironmentVariable", "bru.getEnvVar"),
    ("pm.clearEnvironmentVariable", "bru.deleteEnvVar"),
    ("pm.clearEnvironmentVariables", "bru.deleteAllEnvVars"),
    ("pm.setGlobalVariable", "bru.setGlobalEnvVar"),
    ("pm.getGlobalVariable", "bru.getGlobalEnvVar"),
    ("pm.clearGlobalVariable", "bru.deleteGlobalEnvVar"),
    ("pm.clearGlobalVariables", "bru.deleteAllGlobalEnvVars"),
    ("responseCode.code", "res.getStatus()"),
    ("responseCode.name", "res.statusText"),
];

/// How a recognised chain is rewritten.
enum Rule {
    /// Replace the chain text; whatever follows stays.
    Replace(String),
    /// Replace the chain text, but only when it is called.
    ReplaceCall(String),
    /// Replace the chain text, but only when the chain is a whole expression statement.
    Statement(String),
    /// Rewrites that take the call's arguments apart.
    Args(ArgsRule),
}

#[derive(Clone, Copy)]
enum ArgsRule {
    EnvHas,
    ResponseText,
    SetNextRequest,
    SendRequest,
    HeaderProperty { negated: bool },
    SetHeader,
    HeaderAddFirst,
}

fn rule_for(key: &str) -> Option<Rule> {
    use ArgsRule::*;
    let call = |s: &str| Some(Rule::ReplaceCall(s.to_string()));
    match key {
        "pm.environment.has" => return Some(Rule::Args(EnvHas)),
        "pm.sendRequest" => return Some(Rule::Args(SendRequest)),
        "pm.response.text" => return Some(Rule::Args(ResponseText)),
        "pm.setNextRequest" | "pm.execution.setNextRequest" => {
            return Some(Rule::Args(SetNextRequest));
        }
        "pm.response.to.have.header" => return Some(Rule::Args(HeaderProperty { negated: false })),
        "pm.response.to.not.have.header" | "pm.response.to.have.not.header" => {
            return Some(Rule::Args(HeaderProperty { negated: true }));
        }
        "pm.request.headers.add" | "pm.request.headers.upsert" => {
            return Some(Rule::Args(SetHeader));
        }
        "pm.request.headers.prepend"
        | "pm.request.headers.insert"
        | "pm.request.headers.insertAfter" => return Some(Rule::Args(HeaderAddFirst)),
        "pm.response.to.have.status" => return call("expect(res.getStatus()).to.equal"),
        "pm.response.to.not.have.status" | "pm.response.to.have.not.status" => {
            return call("expect(res.getStatus()).to.not.equal");
        }
        "pm.response.to.have.body" => return call("expect(res.getBody()).to.equal"),
        "pm.response.to.not.have.body" | "pm.response.to.have.not.body" => {
            return call("expect(res.getBody()).to.not.equal");
        }
        "pm.getResponseHeader" => return call("res.getHeader"),
        "pm.response.headers.get" => return call("res.getHeader"),
        "pm.request.headers.remove" => {}
        "pm.response.to.be.withBody" => {
            return Some(Rule::Statement(
                "expect(res.getBody()).to.not.equal(undefined)".into(),
            ));
        }
        "pm.response.to.not.be.withBody" | "pm.response.to.be.not.withBody" => {
            return Some(Rule::Statement(
                "expect(res.getBody()).to.equal(undefined)".into(),
            ));
        }
        _ => {}
    }
    if let Some(text) = status_helper(key) {
        return Some(Rule::Statement(text));
    }
    // jsonBody and jsonSchema keep their shape and gain the body subject.
    if let Some(tail) = key.strip_prefix("pm.response.") {
        const SHAPES: [&str; 8] = [
            "to.have.jsonBody",
            "to.not.have.jsonBody",
            "not.to.have.jsonBody",
            "to.have.not.jsonBody",
            "to.have.jsonSchema",
            "to.not.have.jsonSchema",
            "not.to.have.jsonSchema",
            "to.have.not.jsonSchema",
        ];
        if SHAPES.contains(&tail) {
            return call(&format!("expect(res.getBody()).{tail}"));
        }
    }
    SIMPLE
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| Rule::Replace((*v).to_string()))
}

/// Status helpers (`pm.response.to.be.ok`): name, chai chain and its arguments.
const STATUS_HELPERS: &[(&str, &str, &str)] = &[
    ("ok", "to.be.within", "200, 299"),
    ("success", "to.be.within", "200, 299"),
    ("info", "to.be.within", "100, 199"),
    ("redirection", "to.be.within", "300, 399"),
    ("clientError", "to.be.within", "400, 499"),
    ("serverError", "to.be.within", "500, 599"),
    ("error", "to.be.at.least", "400"),
    ("accepted", "to.equal", "202"),
    ("badRequest", "to.equal", "400"),
    ("unauthorized", "to.equal", "401"),
    ("forbidden", "to.equal", "403"),
    ("notFound", "to.equal", "404"),
    ("rateLimited", "to.equal", "429"),
];

/// The assertion for `pm.response.to.be.<name>` and its `to.not.be` / `to.be.not` negations.
fn status_helper(key: &str) -> Option<String> {
    let tail = key.strip_prefix("pm.response.to.")?;
    let (negated, name) = if let Some(n) = tail.strip_prefix("not.be.") {
        (true, n)
    } else if let Some(n) = tail.strip_prefix("be.not.") {
        (true, n)
    } else {
        (false, tail.strip_prefix("be.")?)
    };
    let (_, chain, args) = STATUS_HELPERS.iter().find(|(n, ..)| *n == name)?;
    let chain = if negated {
        chain.replacen("to.", "to.not.", 1)
    } else {
        (*chain).to_string()
    };
    Some(format!("expect(res.getStatus()).{chain}({args})"))
}

/// True when the code at `at` ends an expression statement: the end, `;`, `}`, a comment,
/// or a line break not followed by a token that would continue the expression.
fn ends_statement(b: &[u8], at: usize) -> bool {
    let mut i = at;
    while i < b.len() && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    match b.get(i) {
        None | Some(b';' | b'}') => true,
        Some(b'/') => matches!(b.get(i + 1), Some(b'/' | b'*')),
        Some(b'\n' | b'\r') => {
            let n = skip_ws(b, i);
            match b.get(n) {
                None => true,
                Some(b'/') => matches!(b.get(n + 1), Some(b'/' | b'*')),
                Some(c) => !b".([`?&|,=+-*%<>:".contains(c),
            }
        }
        _ => false,
    }
}

/// Split call arguments at top-level commas, trimmed.
fn split_top_level(s: &str, sep: u8) -> Vec<&str> {
    let b = s.as_bytes();
    let (mut parts, mut start, mut depth, mut i) = (Vec::new(), 0, 0usize, 0);
    while i < b.len() {
        if let Some(end) = skip_opaque(s, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = s[start..].trim();
    if !last.is_empty() || !parts.is_empty() {
        parts.push(last);
    }
    parts
}

fn translate_code(src: &str, cx: &mut Ctx) -> String {
    if cx.depth >= MAX_DEPTH {
        return src.to_string();
    }
    cx.depth += 1;
    let out = translate_scan(src, cx);
    cx.depth -= 1;
    out
}

fn translate_scan(src: &str, cx: &mut Ctx) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    // Nested translations start from the await context of the position that spawned them.
    let (base, base_fid) = (cx.await_ok, cx.fid);
    let mut scope = scope::Scope::new(base, base_fid);
    // The last token that is not a comment, so statement helpers know a statement starts
    // here. A nested chunk is an expression, so it starts as if after `(`.
    let mut last: Option<u8> = if cx.depth > 1 { Some(b'(') } else { None };
    let mut after_kw = false;
    // A line break since `last`, and whether `last` was a word that still needs an operand:
    // a line that starts after a finished operand starts a new statement.
    let (mut broke, mut open_word) = (false, false);
    // Open `return` statements without a `;`: where the argument ends, and where it began in
    // the source and the output. One whose argument changed is reprinted with its `;`.
    let mut returns: Vec<(usize, usize, usize)> = Vec::new();
    while i < b.len() {
        close_returns(&mut returns, src, i, &mut out);
        if b[i] == b'`' {
            (cx.await_ok, cx.fid) = (scope.ok(), scope.fid());
            i = copy_template(src, i, &mut out, cx);
            (last, after_kw) = (Some(b'`'), false);
            (broke, open_word) = (false, false);
        } else if let Some(end) = skip_opaque(src, i) {
            out.push_str(&src[i..end]);
            if !src[i..end].starts_with("//") && !src[i..end].starts_with("/*") {
                (last, after_kw) = (Some(b'"'), false);
                (broke, open_word) = (false, false);
            } else if src[i..end].contains('\n') {
                broke = true;
            }
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            (cx.await_ok, cx.fid) = (scope.ok(), scope.fid());
            let after_operand = broke
                && !open_word
                && last.is_some_and(|c| is_ident_byte(c) || matches!(c, b'"' | b'`' | b']'));
            cx.at_stmt =
                after_kw || after_operand || matches!(last, None | Some(b';' | b'{' | b'}' | b')'));
            broke = false;
            if !is_property_access(b, i)
                && let Some((text, next)) = rewrite_at(src, i, end, &out, cx)
            {
                out.push_str(&text);
                i = next;
                (last, after_kw, open_word) = (Some(b')'), false, false);
                continue;
            }
            out.push_str(&src[i..end]);
            after_kw = matches!(&src[i..end], "else" | "do") && !is_property_access(b, i);
            if &src[i..end] == "return"
                && !is_property_access(b, i)
                && returns.len() < MAX_OPEN_RETURNS
                && let Some(arg_end) = unterminated_return(src, end, cx)
            {
                returns.push((arg_end, end, out.len()));
            }
            open_word = OPERAND_WORDS.contains(&&src[i..end]) && !is_property_access(b, i);
            last = Some(b[end - 1]);
            i = end;
        } else {
            scope.feed(b, i, &mut cx.next_fid);
            out.push(b[i] as char);
            if !b[i].is_ascii_whitespace() {
                (last, after_kw) = (Some(b[i]), false);
                (broke, open_word) = (false, false);
            } else if b[i] == b'\n' {
                broke = true;
            }
            i += 1;
        }
    }
    close_returns(&mut returns, src, usize::MAX, &mut out);
    (cx.await_ok, cx.fid) = (base, base_fid);
    out
}

/// Words after which a line break does not surely end the statement: operators and
/// keywords that take what follows, or whose line break has special rules.
const OPERAND_WORDS: &[&str] = &[
    "typeof",
    "new",
    "await",
    "in",
    "instanceof",
    "void",
    "delete",
    "yield",
    "of",
    "case",
    "extends",
    "var",
    "let",
    "const",
    "return",
    "throw",
    "async",
    "function",
    "class",
    "import",
    "export",
    "default",
    "if",
    "while",
    "for",
    "with",
    "switch",
    "catch",
    "break",
    "continue",
];

/// Most `return` statements tracked inside one another, so nesting keeps the scan linear.
const MAX_OPEN_RETURNS: usize = 8;

/// Ends the open `return` statements whose argument ends by `at`, adding the `;` the
/// reference's printer gives a changed one.
fn close_returns(returns: &mut Vec<(usize, usize, usize)>, src: &str, at: usize, out: &mut String) {
    while let Some(&(end, src_from, out_from)) = returns.last()
        && end <= at
    {
        returns.pop();
        let upto = at.min(src.len());
        if out[out_from..] != src[src_from..upto] {
            out.push(';');
        }
    }
}

/// Where the argument of a `return` whose keyword ends at `from` ends, when the statement
/// has an argument and no `;`. A line break ends it only where the next line surely starts a
/// new statement; where that is unsure, `None`, since a wrong `;` breaks the script.
fn unterminated_return(src: &str, from: usize, cx: &mut Ctx) -> Option<usize> {
    expression_end(src, from, cx)
        .filter(|e| !e.semicolon)
        .map(|e| e.end)
}

/// Where an expression ends, and whether a `;` follows it there.
struct ExprEnd {
    end: usize,
    semicolon: bool,
}

/// Where the expression starting at `from` ends: at a `;`, a closing bracket, the end of the
/// text, or a line break that surely starts a new statement. `None` when that is unsure.
fn expression_end(src: &str, from: usize, cx: &mut Ctx) -> Option<ExprEnd> {
    let b = src.as_bytes();
    let done = |end: Option<usize>| {
        end.map(|end| ExprEnd {
            end,
            semicolon: false,
        })
    };
    // `after_op`: the last token leaves the expression open; `postfix`: it was `++` or `--`,
    // after which a line break may or may not end the statement.
    let (mut i, mut end, mut after_op, mut postfix) = (from, None, false, false);
    loop {
        let ws = i;
        i = skip_ws(b, i);
        let broke = src[ws..i].contains('\n');
        let Some(&c) = b.get(i) else {
            return done(end);
        };
        if c == b'/' && matches!(b.get(i + 1), Some(b'/' | b'*')) {
            if broke && end.is_none() {
                return None;
            }
            i = skip_opaque(src, i)?;
            continue;
        }
        if broke {
            let e = end?;
            if postfix {
                return None;
            }
            if !after_op && !continues_line(src, i) {
                let starts_statement = c == b'}' || (is_ident_byte(c) && !c.is_ascii_digit());
                return starts_statement.then_some(ExprEnd {
                    end: e,
                    semicolon: false,
                });
            }
        }
        postfix = false;
        match c {
            b';' => {
                return end.map(|end| ExprEnd {
                    end,
                    semicolon: true,
                });
            }
            b')' | b']' | b'}' => return done(end),
            b':' if end.is_none() => return None,
            b'(' | b'[' | b'{' => {
                i = cx.find_close(src, i)? + 1;
                after_op = false;
            }
            _ if is_ident_byte(c) => {
                let e = ident_end(b, i);
                after_op = matches!(
                    &src[i..e],
                    "in" | "instanceof" | "typeof" | "void" | "delete" | "new" | "await" | "yield"
                );
                i = e;
            }
            _ => match skip_opaque(src, i) {
                Some(e) => {
                    i = e;
                    after_op = false;
                }
                None => {
                    postfix = matches!(&src[i..], s if s.starts_with("++") || s.starts_with("--"))
                        && !after_op;
                    if postfix {
                        i += 2;
                        after_op = false;
                    } else {
                        i += 1;
                        after_op = true;
                    }
                }
            },
        }
        end = Some(i);
    }
}

/// Whether the line starting at `i` carries on the expression before it: a binary operator,
/// a member access, a call or template, or the words `in` and `instanceof`.
fn continues_line(src: &str, i: usize) -> bool {
    let b = src.as_bytes();
    match b[i] {
        b'+' | b'-' => b.get(i + 1) != Some(&b[i]),
        b'!' => b.get(i + 1) == Some(&b'='),
        b'.' | b'(' | b'[' | b'`' | b'?' | b'&' | b'|' | b',' | b'=' | b'*' | b'/' | b'%'
        | b'<' | b'>' | b':' | b'^' => true,
        c if is_ident_byte(c) => matches!(&src[i..ident_end(b, i)], "in" | "instanceof"),
        _ => false,
    }
}

/// Copy a template literal, translating only its `${}` holes.
fn copy_template(src: &str, start: usize, out: &mut String, cx: &mut Ctx) -> usize {
    let b = src.as_bytes();
    let end = template_end(src, start);
    let mut i = start;
    let mut text_from = start;
    out.push('`');
    i += 1;
    text_from += 1;
    while i < end {
        match b[i] {
            b'\\' => i += 2,
            b'$' if b.get(i + 1) == Some(&b'{') => {
                let Some(close) = cx.find_close(src, i + 1) else {
                    break;
                };
                out.push_str(&src[text_from..i + 2]);
                out.push_str(&translate_code(&src[i + 2..close], cx));
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

/// Try the rewrites that start at the identifier `src[start..end]`.
fn rewrite_at(
    src: &str,
    start: usize,
    end: usize,
    out: &str,
    cx: &mut Ctx,
) -> Option<(String, usize)> {
    let rewritten = match &src[start..end] {
        "pm" | "postman" | "request" | "responseCode" => rewrite_chain(src, start, end, out, cx),
        "tests" => rewrite_tests_bracket(src, start, end, out, cx),
        "JSON" => rewrite_json_parse(src, end, cx),
        w @ ("responseBody" | "responseHeaders" | "responseTime") => {
            legacy_global(src, start, end, w, cx).map(|t| (t.to_string(), end))
        }
        _ => None,
    };
    if rewritten.is_some() {
        cx.replaced += 1;
    }
    rewritten
}

struct Seg {
    name: (usize, usize),
    has_parens: bool,
}

fn rewrite_chain(
    src: &str,
    start: usize,
    head_end: usize,
    out: &str,
    cx: &mut Ctx,
) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let head = &src[start..head_end];
    let key_head = if head == "postman" { "pm" } else { head };
    let mut segs: Vec<Seg> = Vec::new();
    let mut pos = head_end;
    while segs.len() < MAX_SEGS
        && b.get(pos) == Some(&b'.')
        && b.get(pos + 1).is_some_and(|c| is_ident_byte(*c))
    {
        let e = ident_end(b, pos + 1);
        let has_parens = src[e..].starts_with("()");
        segs.push(Seg {
            name: (pos + 1, e),
            has_parens,
        });
        pos = if has_parens { e + 2 } else { e };
        if has_parens && !(b.get(pos) == Some(&b'.')) {
            break;
        }
    }
    for take in (1..=segs.len()).rev() {
        let key_of = |full: bool| {
            let mut key = key_head.to_string();
            for (j, s) in segs[..take].iter().enumerate() {
                key.push('.');
                key.push_str(&src[s.name.0..s.name.1]);
                if full && j + 1 < take && s.has_parens {
                    key.push_str("()");
                }
            }
            key
        };
        let any_paren = segs[..take - 1].iter().any(|s| s.has_parens);
        let mut keys = Vec::new();
        if any_paren {
            keys.push(key_of(true));
        }
        keys.push(key_of(false));
        for key in keys {
            let Some(rule) = rule_for(&key) else { continue };
            let chain_end = segs[take - 1].name.1;
            let call_at = skip_ws(b, chain_end);
            let called = b.get(call_at) == Some(&b'(');
            match rule {
                // A call result cannot be assigned to: an assignment becomes the setter where
                // there is one, and any other write leaves the chain as written.
                Rule::Replace(text) if text.ends_with(')') => {
                    return match mutation_at(src, start, chain_end) {
                        None => Some((text, chain_end)),
                        Some(Mutation::Assign(rhs)) => assign_with_setter(src, &key, rhs, cx),
                        Some(Mutation::Other) => None,
                    };
                }
                Rule::Replace(text) => return Some((text, chain_end)),
                Rule::Statement(text) if cx.at_stmt => {
                    // `ok()` is read as the helper called with no arguments, as the reference does.
                    let mut end = chain_end;
                    let open = skip_ws(b, end);
                    if b.get(open) == Some(&b'(') {
                        let close = skip_ws(b, open + 1);
                        if b.get(close) != Some(&b')') {
                            continue;
                        }
                        end = close + 1;
                    }
                    if ends_statement(b, end) {
                        return Some((text, end));
                    }
                }
                Rule::Statement(_) => {}
                Rule::ReplaceCall(text) if called => return Some((text, chain_end)),
                Rule::ReplaceCall(_) => {}
                Rule::Args(ArgsRule::SendRequest) if called => {
                    if let Some(done) = send_request::rewrite(src, start, call_at, out, cx) {
                        return Some(done);
                    }
                }
                Rule::Args(kind) if called => {
                    if let Some(done) = rewrite_args(src, kind, call_at, cx) {
                        return Some(done);
                    }
                }
                Rule::Args(_) => {}
            }
        }
    }
    None
}

/// How the chain spanning `start..end` is written to, when it is.
enum Mutation {
    /// A plain `=`, with the offset of the value assigned.
    Assign(usize),
    /// A compound assignment or an increment or decrement.
    Other,
}

fn mutation_at(src: &str, start: usize, end: usize) -> Option<Mutation> {
    let b = src.as_bytes();
    const COMPOUND: [&[u8]; 15] = [
        b"**=", b">>>=", b"<<=", b">>=", b"&&=", b"||=", b"??=", b"+=", b"-=", b"*=", b"/=", b"%=",
        b"&=", b"|=", b"^=",
    ];
    if let Some((k, c @ (b'+' | b'-'))) = prev_token(b, start)
        && k > 0
        && b[k - 1] == c
    {
        return Some(Mutation::Other);
    }
    let n = skip_trivia(src, end, src.len());
    let rest = &b[n..];
    // A `++` or `--` after a line break begins the next statement instead.
    let postfix =
        !b[end..n].contains(&b'\n') && (rest.starts_with(b"++") || rest.starts_with(b"--"));
    if postfix || COMPOUND.iter().any(|op| rest.starts_with(op)) {
        return Some(Mutation::Other);
    }
    (rest.first() == Some(&b'=') && !matches!(rest.get(1), Some(b'=' | b'>')))
        .then(|| Mutation::Assign(skip_trivia(src, n + 1, src.len())))
}

/// The last byte of the token before `i`, with its index, past whitespace and block comments.
fn prev_token(b: &[u8], i: usize) -> Option<(usize, u8)> {
    let mut at = i;
    loop {
        let (k, c) = prev_significant(b, at)?;
        if c == b'/' && k > 0 && b[k - 1] == b'*' {
            at = b[..k - 1].windows(2).rposition(|w| w == b"/*")?;
        } else {
            return Some((k, c));
        }
    }
}

/// `pm.request.url = x` as `req.setUrl(x)`, and the same for the method. Only a whole
/// statement with a single right-hand expression is rewritten; anything else keeps its text.
fn assign_with_setter(src: &str, key: &str, rhs: usize, cx: &mut Ctx) -> Option<(String, usize)> {
    let setter = match key {
        "pm.request.url" => "req.setUrl",
        "pm.request.method" => "req.setMethod",
        _ => return None,
    };
    if !cx.at_stmt {
        return None;
    }
    let end = expression_end(src, rhs, cx)?.end;
    let value = &src[rhs..end];
    if split_top_level(value, b',').len() != 1 {
        return None;
    }
    let value = translate_code(value, cx);
    Some((format!("{setter}({value})"), end))
}

/// Rewrites that need the call's parsed arguments. `open` is the `(` index.
fn rewrite_args(src: &str, kind: ArgsRule, open: usize, cx: &mut Ctx) -> Option<(String, usize)> {
    let close = cx.find_close(src, open)?;
    let inner = &src[open + 1..close];
    let args: Vec<String> = split_top_level(inner, b',')
        .into_iter()
        .map(|a| translate_code(a, cx))
        .collect();
    let joined = args.join(", ");
    let done = |t: String| Some((t, close + 1));
    match kind {
        ArgsRule::SendRequest => None,
        ArgsRule::EnvHas => {
            let a = joined;
            done(format!(
                "bru.getEnvVar({a}) !== undefined && bru.getEnvVar({a}) !== null"
            ))
        }
        ArgsRule::ResponseText => done("JSON.stringify(res.getBody())".to_string()),
        ArgsRule::SetNextRequest => {
            if args.first().is_some_and(|a| a == "null") {
                done("bru.runner.stopExecution()".to_string())
            } else {
                done(format!("bru.runner.setNextRequest({joined})"))
            }
        }
        ArgsRule::HeaderProperty { negated } => {
            let mut parts = args;
            if let Some(first) = parts.first_mut() {
                first.push_str(".toLowerCase()");
            }
            let verb = if negated {
                "to.not.have.property"
            } else {
                "to.have.property"
            };
            done(format!(
                "expect(res.getHeaders()).{verb}({})",
                parts.join(", ")
            ))
        }
        ArgsRule::SetHeader => {
            let pair = args.first().and_then(|a| key_value_of(a));
            match pair {
                Some((k, v)) => done(format!("req.setHeader({k}, {v})")),
                None => done(format!("req.setHeader({joined})")),
            }
        }
        ArgsRule::HeaderAddFirst => {
            let first = args.first().cloned().unwrap_or_default();
            done(format!("req.headerList.add({first})"))
        }
    }
}

/// The `key` and `value` expressions of an object literal like `{ key: a, value: b }`.
fn key_value_of(obj: &str) -> Option<(String, String)> {
    let inner = obj.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (mut key, mut value) = (None, None);
    for prop in split_top_level(inner, b',') {
        let colon = prop.find(':')?;
        let value_text = prop[colon + 1..].trim().to_string();
        match prop[..colon].trim().trim_matches(['\'', '"']) {
            "key" => key = Some(value_text),
            "value" => value = Some(value_text),
            _ => {}
        }
    }
    Some((key?, value?))
}

/// `JSON.parse(responseBody)` becomes `res.getBody()`.
fn rewrite_json_parse(src: &str, end: usize, cx: &Ctx) -> Option<(String, usize)> {
    let rest = src[end..].strip_prefix(".parse(")?;
    let after_open = end + (src[end..].len() - rest.len());
    let b = src.as_bytes();
    let s = skip_ws(b, after_open);
    let e = ident_end(b, s);
    if &src[s..e] != "responseBody" || cx.declares("responseBody") {
        return None;
    }
    let c = skip_ws(b, e);
    (b.get(c) == Some(&b')')).then(|| ("res.getBody()".to_string(), c + 1))
}

/// Legacy globals are replaced as values only; a script's own variable of the
/// same name, or a position that binds the name, is left alone.
fn legacy_global(
    src: &str,
    start: usize,
    end: usize,
    name: &str,
    cx: &Ctx,
) -> Option<&'static str> {
    let b = src.as_bytes();
    if cx.declares(name) {
        return None;
    }
    let next = skip_ws(b, end);
    let assigned = b.get(next) == Some(&b'=') && !matches!(b.get(next + 1), Some(b'=' | b'>'));
    let key_position =
        b.get(next) == Some(&b':') && matches!(prev_significant(b, start), Some((_, b'{' | b',')));
    if assigned || key_position {
        return None;
    }
    Some(match name {
        "responseBody" => "res.getBody()",
        "responseHeaders" => "res.getHeaders()",
        _ => "res.getResponseTime()",
    })
}

/// True when `name` appears as a free identifier in code: outside strings, comments and
/// regexes, not a property (`x.name`), with `${}` holes of templates read as code.
pub(crate) fn uses_global(code: &str, name: &str) -> bool {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            let end = template_end(code, i);
            let mut k = i + 1;
            while k < end {
                match b[k] {
                    b'\\' => k += 2,
                    b'$' if b.get(k + 1) == Some(&b'{') => {
                        let Some(close) = find_close(code, k + 1) else {
                            break;
                        };
                        if uses_global(&code[k + 2..close], name) {
                            return true;
                        }
                        k = close + 1;
                    }
                    _ => k += 1,
                }
            }
            i = end;
        } else if let Some(end) = skip_opaque(code, i) {
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if &code[i..end] == name && !is_property_access(b, i) && !is_object_key(b, i, end) {
                return true;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    false
}

/// True when the identifier at `i..end` is an object key (`{k: v}`, `, k: v`), not a use.
fn is_object_key(b: &[u8], i: usize, end: usize) -> bool {
    b.get(skip_ws(b, end)) == Some(&b':')
        && matches!(prev_significant(b, i), Some((_, b'{' | b',')))
}

/// Whether a script needs `const CryptoJS = require('crypto-js');` put on top: it uses the
/// global and neither declares it nor already requires the module.
pub(crate) fn needs_cryptojs_require(code: &str) -> bool {
    uses_global(code, "CryptoJS")
        && !declares(&blank_opaque(code), "CryptoJS")
        && !requires_module(code, "crypto-js")
}

/// Globals Postman scripts use that the app's scripts do not serve.
pub(crate) const UNAVAILABLE_GLOBALS: [&str; 2] = ["cheerio", "xml2Json"];

/// The unavailable globals the script uses as free identifiers without declaring them.
pub(crate) fn unavailable_globals(code: &str) -> Vec<&'static str> {
    UNAVAILABLE_GLOBALS
        .into_iter()
        .filter(|n| uses_global(code, n) && !declares(&blank_opaque(code), n))
        .collect()
}

/// The code with strings, comments and regexes blanked, so a text search sees only code.
fn blank_opaque(code: &str) -> String {
    let (b, mut out, mut i) = (code.as_bytes(), String::new(), 0);
    while i < b.len() {
        match skip_opaque(code, i) {
            Some(end) => {
                out.push(' ');
                i = end;
            }
            None => {
                let n = code[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&code[i..i + n]);
                i += n;
            }
        }
    }
    out
}

/// Byte offset just after the last leading directive (`'use strict';`), or 0 without one.
/// Only a string-literal statement ending at `;`, a newline or the end counts.
pub(crate) fn directive_prologue_end(code: &str) -> usize {
    let b = code.as_bytes();
    let (mut end, mut i) = (0, skip_trivia(code, 0, code.len()));
    while matches!(b.get(i), Some(b'\'' | b'"')) {
        let Some(lit) = skip_opaque(code, i) else {
            break;
        };
        let next = skip_trivia(code, lit, code.len());
        let stop = if b.get(next) == Some(&b';') {
            next + 1
        } else if next >= b.len()
            || (code[lit..next].contains('\n') && !b"+-*/%.([,?:=<>&|^`".contains(&b[next]))
        {
            lit
        } else {
            break;
        };
        end = stop;
        i = skip_trivia(code, stop, code.len());
    }
    end
}

/// True when the code calls `require("name")` outside comments and strings; spacing and
/// quote style do not matter.
pub(crate) fn requires_module(code: &str, name: &str) -> bool {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(code, i) {
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if &code[i..end] == "require" && !is_property_access(b, i) {
                let open = skip_trivia(code, end, code.len());
                let arg = skip_trivia(code, open + 1, code.len());
                if b.get(open) == Some(&b'(')
                    && matches!(b.get(arg), Some(b'\'' | b'"'))
                    && let Some(lit) = skip_opaque(code, arg)
                    && code[arg + 1..lit.saturating_sub(1)] == *name
                    && b.get(skip_trivia(code, lit, code.len())) == Some(&b')')
                {
                    return true;
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    false
}

/// True when the script binds `name` itself (`var`, `let`, `const`, `function`, `catch`).
fn declares(src: &str, name: &str) -> bool {
    let b = src.as_bytes();
    let mut from = 0;
    while let Some(off) = src[from..].find(name) {
        let at = from + off;
        from = at + name.len();
        let bounded = (at == 0 || !is_ident_byte(b[at - 1]))
            && !b.get(from).is_some_and(|c| is_ident_byte(*c));
        if !bounded {
            continue;
        }
        let before = src[..at].trim_end();
        let word = before.rsplit(|c: char| !c.is_alphanumeric()).next();
        if word.is_some_and(|w| ["var", "let", "const", "function"].contains(&w)) {
            return true;
        }
    }
    false
}

/// `tests['name'] = expr;` becomes a `test(...)` call asserting the expression.
fn rewrite_tests_bracket(
    src: &str,
    start: usize,
    end: usize,
    out: &str,
    cx: &mut Ctx,
) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let statement_start = matches!(
        prev_significant(b, start),
        None | Some((_, b';' | b'{' | b'}'))
    );
    let open = skip_ws(b, end);
    if !statement_start || b.get(open) != Some(&b'[') {
        return None;
    }
    let close = cx.find_close(src, open)?;
    let name = src[open + 1..close].trim();
    let name_end = skip_opaque(name, 0)?;
    if name_end != name.len() || !matches!(name.as_bytes()[0], b'\'' | b'"' | b'`') {
        return None;
    }
    let eq = skip_ws(b, close + 1);
    if b.get(eq) != Some(&b'=') || matches!(b.get(eq + 1), Some(b'=')) {
        return None;
    }
    let rhs_start = eq + 1;
    let mut k = rhs_start;
    let mut depth = 0usize;
    while k < b.len() {
        if let Some(e) = skip_opaque(src, k) {
            k = e;
            continue;
        }
        match b[k] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b';' | b'\n' if depth == 0 => break,
            _ => {}
        }
        k += 1;
    }
    let rhs = translate_code(src[rhs_start..k].trim(), cx);
    let next = if b.get(k) == Some(&b';') { k + 1 } else { k };
    let indent: String = out
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    // Single-quoted names are re-quoted the way the reference prints new literals.
    let shown = match name.as_bytes()[0] {
        b'\'' if !name.contains(['"', '\\']) => format!("\"{}\"", &name[1..name.len() - 1]),
        _ => name.to_string(),
    };
    let text = format!(
        "test({shown}, function() {{\n{indent}  expect(Boolean({rhs})).to.be.true;\n{indent}}});"
    );
    Some((text, next))
}

/// The first `pm.`/`postman.` member chain in `code`.
pub(crate) fn first_pm_chain(code: &str) -> Option<String> {
    let mut found = Vec::new();
    collect_unsupported(code, &mut found, &mut HashSet::new());
    found.into_iter().next()
}

/// The `pm.`/`postman.` member chain that runs at the 1-based `line` and `column` of `code`, as
/// the run-time error names it: the last chain on that line starting at or before the column,
/// else the line's first chain, else the first in the code.
pub(crate) fn pm_chain_at(code: &str, line: u32, column: Option<u32>) -> Option<String> {
    let mut on_line: Vec<(usize, String)> = Vec::new();
    each_pm_chain(code, 0, &mut |at, chain| {
        let before = &code[..at];
        let row = before.matches('\n').count() + 1;
        if row == line as usize {
            let col = before[before.rfind('\n').map_or(0, |n| n + 1)..]
                .chars()
                .count()
                + 1;
            on_line.push((col, chain));
        }
    });
    let at_column = column.and_then(|c| {
        let k = on_line.iter().rposition(|(col, _)| *col <= c as usize)?;
        Some(on_line.swap_remove(k).1)
    });
    at_column
        .or_else(|| on_line.into_iter().next().map(|(_, chain)| chain))
        .or_else(|| first_pm_chain(code))
}

/// Collect the distinct `pm.`/`postman.` member chains that survive translation.
fn collect_unsupported(code: &str, found: &mut Vec<String>, seen: &mut HashSet<String>) {
    each_pm_chain(code, 0, &mut |_, chain| {
        if seen.insert(chain.clone()) {
            found.push(chain);
        }
    });
}

/// Every `pm.`/`postman.` member chain in `code`, in source order, with its offset plus `base`.
fn each_pm_chain(code: &str, base: usize, each: &mut dyn FnMut(usize, String)) {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(code, i) {
            if b[i] == b'`' {
                template_holes(code, i, end, base, each);
            }
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            let word = &code[i..end];
            let mut next = end;
            if (word == "pm" || word == "postman") && !is_property_access(b, i) {
                let mut chain = word.to_string();
                while b.get(next) == Some(&b'.')
                    && b.get(next + 1).is_some_and(|c| is_ident_byte(*c))
                {
                    let e = ident_end(b, next + 1);
                    chain.push_str(&code[next..e]);
                    next = e;
                }
                if chain.contains('.') {
                    each(base + i, chain);
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
    base: usize,
    each: &mut dyn FnMut(usize, String),
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
                each_pm_chain(&code[i + 2..close], base + i + 2, each);
                i = close + 1;
            }
            _ => i += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(src: &str) -> String {
        translate(src).code
    }

    /// Every scripted corpus fixture must reproduce the reference bytes.
    #[test]
    fn corpus_scripts_match_reference_output() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture = std::fs::read_to_string(
            root.join("tests/fixtures/postman/collections/postman-with-scripts.json"),
        )
        .unwrap();
        let expected = std::fs::read_to_string(root.join(
            "tests/fixtures/reference/2026-10-09-import-parity/expected/postman-with-scripts/Login.yml",
        ))
        .unwrap();
        let doc: serde_json::Value = serde_json::from_str(&fixture).unwrap();
        let mut seen = 0;
        let mut stack = vec![&doc];
        while let Some(v) = stack.pop() {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(lines) = m.get("exec").and_then(|e| e.as_array()) {
                        let joined: Vec<&str> = lines.iter().filter_map(|l| l.as_str()).collect();
                        let out = t(&joined.join("\n"));
                        let block: String = out.lines().map(|l| format!("        {l}\n")).collect();
                        assert!(expected.contains(&block), "missing in reference:\n{block}");
                        seen += 1;
                    }
                    stack.extend(m.values());
                }
                serde_json::Value::Array(a) => stack.extend(a.iter()),
                _ => {}
            }
        }
        assert_eq!(seen, 2);
    }

    /// The reference emits invalid JS for these (`pm.response = ...`, `function f(pm.response)`);
    /// Apinata leaves a reassigned or shadowed alias alone. Each entry is the output it gives.
    const KNOWN_REFERENCE_DEFECTS: &[(&str, &str)] = &[
        (
            "alias-5",
            "let r = pm.response;\nr = somethingElse();\nr.json();",
        ),
        (
            "alias-6",
            "const r = pm.response;\nfunction f(r) {\n  return r.json();\n}\nf(other);\nr.text();",
        ),
        // The reference also rewrites the property name `responseTime` after `res.`, which
        // gives `res.res.getResponseTime()`; Apinata keeps property names alone.
        (
            "send_request-8",
            "await bru.sendRequest(\n  {url: 'https://example.com', headers: 'X-A: 1'},\n  async function(err, res) {\n    expect(res.responseTime).to.be.below(100);\n  }\n);",
        ),
        // The reference drops a callback that is not a function literal, losing the call's
        // result handling; Apinata keeps it.
        ("send_request-39", "await bru.sendRequest('u', cb);"),
        // The reference replaces the whole enclosing call when a status helper is its argument,
        // dropping `pm.expect(`; Apinata leaves a helper that is not a statement alone.
        (
            "response_helpers-36",
            "expect(pm.response.to.be.serverError).to.be.false;",
        ),
        (
            "response_helpers-47",
            "expect(pm.response.to.be.withBody).to.be.true;",
        ),
    ];

    fn corpus() -> (Vec<serde_json::Value>, serde_json::Value) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/reference/2026-10-20-postman-script-translation");
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        let cases: Vec<serde_json::Value> = serde_json::from_str(&read("cases.json")).unwrap();
        (cases, serde_json::from_str(&read("expected.json")).unwrap())
    }

    /// Every case must reproduce the reference output, or the pinned deviation.
    #[test]
    fn corpus_matches_reference() {
        let (cases, expected) = corpus();
        let (mut checked, mut wrong) = (0, Vec::new());
        for c in &cases {
            let id = c["id"].as_str().unwrap();
            let want = match KNOWN_REFERENCE_DEFECTS.iter().find(|(d, _)| *d == id) {
                Some((_, ours)) => ours,
                None => expected[id]
                    .as_str()
                    .unwrap_or_else(|| panic!("no expected output for {id}")),
            };
            let got = t(c["script"].as_str().unwrap());
            if got != want {
                wrong.push(format!("case {id}\n--- want\n{want}\n--- got\n{got}\n"));
            }
            checked += 1;
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
        assert!(checked >= 100, "only {checked} cases");
    }

    /// Every group keeps a meaningful number of cases and every case has an expected entry.
    #[test]
    fn corpus_groups_are_populated() {
        let (cases, expected) = corpus();
        let mut counts = std::collections::BTreeMap::new();
        for c in &cases {
            *counts.entry(c["group"].as_str().unwrap()).or_insert(0) += 1;
            assert!(expected.get(c["id"].as_str().unwrap()).is_some());
        }
        for (g, n) in counts {
            assert!(n >= 6, "group {g} has {n} cases");
        }
    }

    /// Outputs below were produced by the reference on the same input.
    #[test]
    fn alias_declarations_leave_like_the_reference() {
        let cases: &[(&str, &str)] = &[
            (
                "const r = pm.response;\n\nconst b = r.json();\n\nconsole.log(b);",
                "const b = res.getBody();\n\nconsole.log(b);",
            ),
            (
                "function f() {\n  const r = pm.response;\n  return r.json();\n}\nf();",
                "function f() {\n  return res.getBody();\n}\nf();",
            ),
            (
                "const a = pm.response, x = 5;\nconsole.log(a.code, x);",
                "const x = 5;\nconsole.log(res.getStatus(), x);",
            ),
            (
                "const x = 5, a = pm.response;\nconsole.log(a.code, x);",
                "const x = 5;\nconsole.log(res.getStatus(), x);",
            ),
            (
                "const x = 5, a = pm.response, y = 6;\nconsole.log(a.code, x, y);",
                "const x = 5, y = 6;\nconsole.log(res.getStatus(), x, y);",
            ),
            (
                "// head\nconst r = pm.response; // trailing\n// after\nr.json();",
                "// after\nres.getBody();",
            ),
            (
                "// lead\n\nconst r = pm.response;\n\nr.json();",
                "res.getBody();",
            ),
            (
                "foo(); // c1\n// c2\nconst r = pm.response;\nr.json();",
                "foo(); // c1\nres.getBody();",
            ),
            (
                "  const r = pm.response;   \n  r.json();\n",
                "  res.getBody();\n",
            ),
            ("const r = pm.response;", ""),
            ("const r = pm.response\n;\nr.json();", "res.getBody();"),
            (
                "const e = pm.environment;\nconsole.log(`${e.get('b')}`);",
                "console.log(`${bru.getEnvVar('b')}`);",
            ),
            (
                "const p = pm.environment;\nconst q = p;\nq.get('a');",
                "bru.getEnvVar('a');",
            ),
            (
                "const r = pm.response.code;\nr",
                "const r = res.getStatus();\nr",
            ),
            (
                "const t = pm.test;\nt('x', function(){});",
                "test('x', function(){});",
            ),
            (
                "const { response, environment: env } = pm;\nresponse.json();\nenv.get('a');",
                "res.getBody();\nbru.getEnvVar('a');",
            ),
            (
                "const r = pm.response;\nif (r.code === 200) {\n  console.log(r.json());\n}\n",
                "if (res.getStatus() === 200) {\n  console.log(res.getBody());\n}\n",
            ),
        ];
        for (src, want) in cases {
            assert_eq!(t(src), *want, "input {src:?}");
        }
    }

    /// Where the reference would emit invalid JavaScript, the alias is left as written.
    #[test]
    fn unsafe_aliases_are_left_alone() {
        for src in [
            "let r = pm.response;\nr++;\nr.json()",
            "const r = pm.response;\nconst o = { r };\nr.json();",
            "const r = pm.response;\nconst o = { r: 1 };\nr.json();",
            "const r = pm.response;\nconst f = (r) => r.json();",
            "const r = pm.response;\nconst f = r => r.json();",
            "const r = pm.response;\ntry { x() } catch (r) { r.json() }",
            "const r = pm.response;\n{ const r = 1; }\nr.json();",
            "const r = pm.response;\nclass A { m(r) { return r.json(); } }",
            "const p = pm;\np.environment.get('a');",
            "for (const r = pm.response; r.code < 0; ) {}",
        ] {
            // The declaration survives untouched, so nothing was resolved.
            assert_eq!(t(src).lines().next(), src.lines().next(), "{src:?}");
        }
        assert_eq!(
            t("const r = pm.response;\nconst o = { r };\nr.json();"),
            "const r = pm.response;\nconst o = { r };\nr.json();"
        );
        // A resolved alias is no longer reported; one left in place still is.
        assert!(
            translate("const r = pm.response;\nr.json();")
                .unsupported
                .is_empty()
        );
        assert_eq!(
            translate("let r = pm.response;\nr = x();").unsupported,
            ["pm.response"]
        );
        // A partly resolved destructuring keeps its statement so `sendRequest` stays bound.
        assert_eq!(
            t("const { response, sendRequest } = pm;\nresponse.json();"),
            "const { response, sendRequest } = pm;\nres.getBody();"
        );
    }

    #[test]
    fn aliases_in_hostile_input_stay_linear() {
        linear(50_000, |k| "const r = pm.response;\n".repeat(k));
        linear(40_000, |k| {
            (0..k)
                .map(|i| format!("const a{i} = pm.response; a{i}.json();\n"))
                .collect()
        });
        // A chain longer than the pass limit settles after five passes.
        linear(2_000, |k| {
            (0..k)
                .map(|i| format!("const c{} = c{i};\n", i + 1))
                .collect::<String>()
                + "const c0 = pm.environment;\n"
        });
        linear(50_000, |k| "const a = 1 +\n".repeat(k));
        linear(30_000, |k| "const f = (".repeat(k) + &") => 1".repeat(k));
        linear(5_000, |k| "`${const r = pm.response; r".repeat(k));
        linear(50_000, |k| "const {a, ".repeat(k));
    }

    #[test]
    fn blank_scripts_stay_blank() {
        assert_eq!(t(""), "");
    }

    #[test]
    fn simple_table_rows() {
        assert_eq!(t("pm.environment.get('a');"), "bru.getEnvVar('a');");
        assert_eq!(t("pm.variables.set('a', 1);"), "bru.setVar('a', 1);");
        assert_eq!(
            t("pm.collectionVariables.unset('a')"),
            "bru.deleteCollectionVar('a')"
        );
        assert_eq!(t("pm.globals.get('g')"), "bru.getGlobalEnvVar('g')");
        assert_eq!(t("pm.expect(1).to.equal(1)"), "expect(1).to.equal(1)");
        assert_eq!(t("pm.expect.fail('x')"), "expect.fail('x')");
        assert_eq!(
            t("const j = pm.response.json();"),
            "const j = res.getBody();"
        );
        assert_eq!(t("pm.response.code"), "res.getStatus()");
        assert_eq!(t("pm.response.responseTime"), "res.getResponseTime()");
        assert_eq!(t("pm.response.headers.get('X')"), "res.getHeader('X')");
        assert_eq!(t("pm.request.url"), "req.getUrl()");
        assert_eq!(t("pm.request.url.getHost()"), "req.getHost()");
        assert_eq!(t("pm.request.headers.get('a')"), "req.headerList.get('a')");
        assert_eq!(t("pm.environment.name"), "bru.getEnvName()");
        assert_eq!(t("pm.response.size().body"), "res.getSize().body");
        assert_eq!(t("pm.response.size()"), "res.getSize()");
        assert_eq!(
            t("pm.cookies.jar().get(u, 'n')"),
            "bru.cookies.jar().getCookie(u, 'n')"
        );
        assert_eq!(t("pm.cookies.remove('c')"), "bru.cookies.delete('c')");
        assert_eq!(t("pm.execution.skipRequest()"), "bru.runner.skipRequest()");
        assert_eq!(t("request.method"), "req.getMethod()");
        assert_eq!(t("responseCode.code"), "res.getStatus()");
    }

    #[test]
    fn assignment_to_url_or_method_becomes_the_setter() {
        assert_eq!(
            t("pm.request.url = 'http://127.0.0.1/x';"),
            "req.setUrl('http://127.0.0.1/x');"
        );
        assert_eq!(t("pm.request.method = 'POST'"), "req.setMethod('POST')");
        assert_eq!(
            t("postman.request.url = pm.environment.get('u') + '/a'\nfoo()"),
            "req.setUrl(bru.getEnvVar('u') + '/a')\nfoo()"
        );
        assert_eq!(
            t("if (a) { pm.request.method = m }"),
            "if (a) { req.setMethod(m) }"
        );
        assert_eq!(
            t("pm.request.url =\n  base +\n  path;\nx()"),
            "req.setUrl(base +\n  path);\nx()"
        );
    }

    #[test]
    fn comparison_with_url_or_method_stays_a_read() {
        assert_eq!(
            t("if (pm.request.url == u) {}"),
            "if (req.getUrl() == u) {}"
        );
        assert_eq!(
            t("pm.request.method === 'GET'"),
            "req.getMethod() === 'GET'"
        );
    }

    /// A call result cannot be assigned to, so a target with no setter keeps its text
    /// and is reported unsupported rather than turned into invalid code.
    #[test]
    fn assignment_without_a_setter_is_left_as_written() {
        for src in [
            "pm.request.url += '/a';",
            "pm.request.method ??= 'GET';",
            "pm.request.headers = h;",
            "pm.request.body = b;",
            "pm.response.code = 200;",
            "pm.response.code++;",
            "--pm.response.code;",
            "x = pm.request.url = u;",
            "pm.request.url = a, b;",
        ] {
            let out = translate(src);
            assert_eq!(out.code, src, "{src}");
            assert!(!out.unsupported.is_empty(), "{src}");
        }
        assert_eq!(t("request.url = u;"), "request.url = u;");
    }

    #[test]
    fn assignment_is_seen_past_comments() {
        assert_eq!(t("pm.request.url /* c */ = u;"), "req.setUrl(u);");
        assert_eq!(t("pm.request.url = // c\n  u;"), "req.setUrl(u);");
        assert_eq!(
            t("pm.request.method /* c */ === 'GET'"),
            "req.getMethod() /* c */ === 'GET'"
        );
        for src in [
            "pm.request.url /* c */ += u;",
            "pm.response.code /* c */ ++;",
            "++ /* c */ pm.response.code;",
            "pm.response.code // c\n= 1;",
        ] {
            let out = translate(src);
            assert_eq!(out.code, src, "{src}");
            assert!(!out.unsupported.is_empty(), "{src}");
        }
    }

    #[test]
    fn assignment_values_of_any_shape_go_into_the_setter() {
        assert_eq!(t("pm.request.url = a ? b : c;"), "req.setUrl(a ? b : c);");
        assert_eq!(
            t("pm.request.url = `${base}/x`;"),
            "req.setUrl(`${base}/x`);"
        );
        assert_eq!(
            t("pm.request.url = (() => pm.environment.get('u'))();"),
            "req.setUrl((() => bru.getEnvVar('u'))());"
        );
        assert_eq!(
            t("pm.request.url = pm.test('a', f);"),
            "req.setUrl(test('a', f));"
        );
        assert_eq!(
            t("pm.request.url = pm.response.to.be.ok;"),
            "req.setUrl(pm.response.to.be.ok);"
        );
    }

    #[test]
    fn assignment_on_a_line_without_semicolons_becomes_the_setter() {
        assert_eq!(
            t("const a = 1\npm.request.url = u"),
            "const a = 1\nreq.setUrl(u)"
        );
        assert_eq!(
            t("pm.request.url = u\npm.request.method = m"),
            "req.setUrl(u)\nreq.setMethod(m)"
        );
        assert_eq!(
            t("f(x)\n/* c\n */ pm.request.url = 'a'\nlet s = \"b\"\npm.request.method = m"),
            "f(x)\n/* c\n */ req.setUrl('a')\nlet s = \"b\"\nreq.setMethod(m)"
        );
        assert_eq!(t("x = [1]\npm.request.url = u"), "x = [1]\nreq.setUrl(u)");
        // After an operator or a keyword that needs an operand the line carries on.
        for src in [
            "x = a +\npm.request.url = u",
            "x = typeof\npm.request.url = u",
            "x = a, b\n  = 1\npm.request.url = u, v",
        ] {
            let out = translate(src);
            assert!(!out.code.contains("setUrl"), "{src}");
            assert!(!out.code.contains("getUrl() ="), "{src}");
        }
    }

    #[test]
    fn postman_prefix_and_legacy_calls() {
        assert_eq!(
            t("postman.setEnvironmentVariable('a', 1)"),
            "bru.setEnvVar('a', 1)"
        );
        assert_eq!(
            t("postman.getEnvironmentVariable('a')"),
            "bru.getEnvVar('a')"
        );
        assert_eq!(
            t("postman.clearEnvironmentVariable('a')"),
            "bru.deleteEnvVar('a')"
        );
        assert_eq!(t("postman.getResponseHeader('A')"), "res.getHeader('A')");
    }

    #[test]
    fn complex_forms() {
        assert_eq!(
            t("pm.response.to.have.status(200);"),
            "expect(res.getStatus()).to.equal(200);"
        );
        assert_eq!(
            t("pm.response.to.not.have.status(404)"),
            "expect(res.getStatus()).to.not.equal(404)"
        );
        assert_eq!(
            t("pm.response.to.have.header('Content-Type')"),
            "expect(res.getHeaders()).to.have.property('Content-Type'.toLowerCase())"
        );
        assert_eq!(
            t("pm.response.to.have.body('x')"),
            "expect(res.getBody()).to.equal('x')"
        );
        assert_eq!(
            t("pm.response.to.have.jsonSchema(s)"),
            "expect(res.getBody()).to.have.jsonSchema(s)"
        );
        assert_eq!(
            t("pm.response.to.have.jsonBody('a')"),
            "expect(res.getBody()).to.have.jsonBody('a')"
        );
        assert_eq!(
            t("pm.response.to.be.withBody;"),
            "expect(res.getBody()).to.not.equal(undefined);"
        );
        assert_eq!(t("pm.response.text()"), "JSON.stringify(res.getBody())");
        assert_eq!(
            t("if (pm.environment.has('k')) {}"),
            "if (bru.getEnvVar('k') !== undefined && bru.getEnvVar('k') !== null) {}"
        );
        assert_eq!(t("pm.setNextRequest(null);"), "bru.runner.stopExecution();");
        assert_eq!(
            t("pm.setNextRequest('b');"),
            "bru.runner.setNextRequest('b');"
        );
        assert_eq!(
            t("pm.setNextRequest('null');"),
            "bru.runner.setNextRequest('null');"
        );
        assert_eq!(
            t("pm.execution.setNextRequest(null)"),
            "bru.runner.stopExecution()"
        );
        assert_eq!(
            t("pm.request.headers.add({ key: 'A', value: pm.variables.get('v') });"),
            "req.setHeader('A', bru.getVar('v'));"
        );
        assert_eq!(
            t("pm.request.headers.insert(h, before)"),
            "req.headerList.add(h)"
        );
    }

    #[test]
    fn nested_calls_translate_inside_arguments() {
        assert_eq!(
            t("pm.test('t', () => { pm.expect(pm.response.json().id).to.be.ok; });"),
            "test('t', () => { expect(res.getBody().id).to.be.ok; });"
        );
    }

    #[test]
    fn tests_bracket_and_legacy_globals() {
        assert_eq!(
            t("tests['ok'] = responseCode.code === 200;"),
            "test(\"ok\", function() {\n  expect(Boolean(res.getStatus() === 200)).to.be.true;\n});"
        );
        assert_eq!(
            t("var d = JSON.parse(responseBody);"),
            "var d = res.getBody();"
        );
        assert_eq!(
            t("console.log(responseBody, responseTime);"),
            "console.log(res.getBody(), res.getResponseTime());"
        );
        assert_eq!(
            t("const responseBody = 1; use(responseBody);"),
            "const responseBody = 1; use(responseBody);"
        );
    }

    #[test]
    fn strings_comments_templates_and_names_are_untouched() {
        let keep = "log('pm.environment.get(x)'); // pm.test(\n/* pm.expect( */ \"pm.test\";";
        assert_eq!(t(keep), keep);
        assert_eq!(t("`a pm.test ${pm.test}`"), "`a pm.test ${test}`");
        for keep in [
            "mypm.environment.get('a')",
            "obj.pm.test('a')",
            "pm_x.test",
            "xrequest.url",
            "other.request.url",
        ] {
            assert_eq!(t(keep), keep);
        }
    }

    #[test]
    fn unsupported_chains_are_listed_in_first_seen_order() {
        let r = translate(
            "pm.require('x'); pm.visualizer.set(t, d); pm.require(y);\npm.environment.set('a', 1); postman.setNextRequest('x'); log('pm.nope'); pm.sendRequest(x);",
        );
        assert_eq!(r.unsupported, ["pm.require", "pm.visualizer.set"]);
        assert!(r.code.contains("bru.setEnvVar"));
        assert!(r.replaced >= 2);
    }

    /// Run `f` on a thread with a small stack so unbounded recursion fails here.
    fn bounded<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .expect("translator overflowed or panicked")
    }

    /// Smallest CPU time of three translations of `src`, each on the
    /// small-stack thread. CPU time leaves out the time a busy machine keeps
    /// the thread waiting, and interference only adds, so the minimum is the
    /// least disturbed run.
    fn best_time(src: &str) -> std::time::Duration {
        (0..3)
            .map(|_| {
                let src = src.to_owned();
                bounded(move || {
                    let start = cpu_time::ThreadTime::now();
                    let _ = translate(&src);
                    start.elapsed()
                })
            })
            .min()
            .expect("three runs")
    }

    /// Asserts that translating `make(n)` costs about four times `make(n / 4)`,
    /// not sixteen times. Both are measured back to back, so the ratio does
    /// not depend on how busy the machine is, and the bound of ten sits
    /// midway between linear (4) and quadratic (16), wide enough for stack and
    /// allocator noise. The full-size run also proves the input does not
    /// overflow the small stack. Inputs whose n/4 run is under 2 ms (deep
    /// nesting) get only a loose bound, since their timing is noise.
    fn linear(n: usize, make: impl Fn(usize) -> String) {
        let small = best_time(&make(n / 4));
        let full = best_time(&make(n));
        // Below this a timing is mostly noise, so the ratio is not trusted.
        let floor = std::time::Duration::from_millis(2);
        assert!(
            full < small.max(floor) * 10,
            "size {n} took {full:?}, size {} took {small:?}: not linear",
            n / 4
        );
    }

    #[test]
    fn many_legacy_globals_translate_in_linear_time() {
        linear(20_000, |k| "x = responseBody;\n".repeat(k));
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        linear(1000, |k| "pm.setNextRequest(".repeat(k) + &")".repeat(k));
        linear(5000, |k| "tests['a'] = ".repeat(k));
        linear(5000, |k| "`${".repeat(k));
    }

    #[test]
    fn long_chains_and_regex_runs_are_bounded() {
        linear(350_000, |k| "pm.".repeat(k));
        linear(200_000, |k| "(/[/[".repeat(k));
    }

    #[test]
    fn unclosed_calls_and_many_distinct_chains_are_bounded() {
        linear(55_000, |k| "pm.setNextRequest(".repeat(k));
        linear(60_000, |k| (0..k).map(|i| format!("pm.a{i} ")).collect());
    }

    #[test]
    fn send_request_in_hostile_input_stays_bounded() {
        linear(55_000, |k| "pm.sendRequest(".repeat(k));
        linear(1000, |k| {
            "pm.sendRequest('u', function (e, r) {".repeat(k) + &"});".repeat(k)
        });
        linear(20_000, |k| {
            "pm.sendRequest('u')".to_string() + &".then(function (r) { return r; })".repeat(k)
        });
        linear(20_000, |k| {
            "pm.sendRequest('u', function (e, r) { r.json(); });\n".repeat(k)
        });
        linear(40_000, |k| "pm.sendRequest('a'); ".repeat(k));
        linear(30_000, |k| {
            "pm.sendRequest({url: 'u', header: [".to_string()
                + &"{key: 'a', value: 'b'}, ".repeat(k)
                + "]}, function (e, r) {});"
        });
        linear(20_000, |k| {
            (0..k)
                .map(|i| format!("const o{i} = {{header: [{{key: 'a', value: 'b'}}]}};\n"))
                .collect::<String>()
                + &"pm.sendRequest(o0);\n".repeat(k / 2)
        });
        linear(5_000, |k| {
            "const o = {header: [], body: {mode: 'urlencoded', urlencoded: []}, ".repeat(k)
        });
        linear(30_000, |k| {
            "pm.sendRequest(x, function (e, r) { (".repeat(k)
        });
        linear(30_000, |k| "pm.sendRequest(x, (e, r) => ".repeat(k));
        linear(5_000, |k| "pm.sendRequest(`${".repeat(k));
    }

    #[test]
    fn send_request_tracks_where_await_is_legal() {
        for (src, want) in [
            (
                "function f() { pm.sendRequest('u'); }",
                "function f() { bru.sendRequest('u'); }",
            ),
            (
                "async function f() { pm.sendRequest('u'); }",
                "async function f() { await bru.sendRequest('u'); }",
            ),
            (
                "const f = async () => pm.sendRequest('u');",
                "const f = async () => await bru.sendRequest('u');",
            ),
            (
                "const f = () => pm.sendRequest('u'), g = 1;\npm.sendRequest('v');",
                "const f = () => bru.sendRequest('u'), g = 1;\nawait bru.sendRequest('v');",
            ),
            (
                "x.forEach(n => { pm.sendRequest(n); });",
                "x.forEach(n => { bru.sendRequest(n); });",
            ),
            (
                "class A { async m() { pm.sendRequest('u'); } n() { pm.sendRequest('u'); } }",
                "class A { async m() { await bru.sendRequest('u'); } n() { bru.sendRequest('u'); } }",
            ),
            (
                "if (a) { pm.sendRequest('u'); } else { pm.sendRequest('v'); }",
                "if (a) { await bru.sendRequest('u'); } else { await bru.sendRequest('v'); }",
            ),
        ] {
            assert_eq!(t(src), want, "{src}");
        }
    }

    #[test]
    fn send_request_edge_shapes_keep_valid_output() {
        // Nothing to translate, or a shape the translator will not guess at.
        for keep in [
            "pm.sendRequest();",
            "pm.sendRequest({url: 'u', ...rest}, cb);",
            "pm.sendRequest({url: 'u', header}, cb);",
            "pm.sendRequest(",
        ] {
            assert!(t(keep).contains("pm.sendRequest"), "{keep}");
        }
        assert_eq!(
            t("await pm.sendRequest('u');"),
            "await bru.sendRequest('u');"
        );
        assert_eq!(
            t("const r = pm.sendRequest('u').foo;"),
            "const r = (await bru.sendRequest('u')).foo;"
        );
        assert!(
            translate("pm.sendRequest('u', function (e, r) {});")
                .unsupported
                .is_empty()
        );
    }

    #[test]
    fn response_access_follows_nested_scopes() {
        let wrap = |body: &str| format!("pm.sendRequest('u', function (e, res) {{ {body} }});");
        for (body, want) in [
            (
                "[1].map((res) => res.code); res.code;",
                "[1].map((res) => res.code); res.status;",
            ),
            (
                "[1].map(res => res.code); res.code;",
                "[1].map(res => res.code); res.status;",
            ),
            (
                "try {} catch (res) { res.code } res.code",
                "try {} catch (res) { res.code } res.status",
            ),
            (
                "f(function(){var res=1;res.code}); res.code",
                "f(function(){var res=1;res.code}); res.status",
            ),
            (
                "f(function (x) { res.code; }); res.code;",
                "f(function (x) { res.status; }); res.status;",
            ),
            ("if (res) { res.code; }", "if (res) { res.status; }"),
            (
                "`${[1].map(res => res.code)} ${res.code}`;",
                "`${[1].map(res => res.code)} ${res.status}`;",
            ),
        ] {
            // A long call wraps its arguments; fold that back to compare the body alone.
            let got = t(&wrap(body))
                .replace("(\n  ", "(")
                .replace(",\n  ", ", ")
                .replace("\n)", ")");
            assert_eq!(
                got,
                format!("await bru.sendRequest('u', async function(e, res) {{ {want} }});"),
                "{body}"
            );
        }
    }

    #[test]
    fn scope_tracking_stays_linear() {
        let inner = "[1].map(function (res) { return res.code; }); (res) => res.code; try {} catch (res) {}\n";
        linear(20_000, |k| {
            "pm.sendRequest('u', function (e, res) {".to_string() + &inner.repeat(k) + "});"
        });
        linear(30_000, |k| {
            "pm.sendRequest('u', function (e, res) {".to_string() + &"f((res) => {".repeat(k)
        });
        linear(30_000, |k| {
            "pm.sendRequest('u', function (e, res) { (res".to_string() + &", (res".repeat(k)
        });
        linear(5_000, |k| {
            "pm.sendRequest('u', function (e, res) { `${".to_string() + &"`${".repeat(k)
        });
    }
    /// Outputs below were produced by the reference on the same input.
    #[test]
    fn response_access_follows_hoisting_named_functions_and_ternaries() {
        let wrap = |body: &str| format!("pm.sendRequest('u', function (e, res) {{\n{body}\n}});");
        let want = |body: &str| {
            format!("await bru.sendRequest('u', async function(e, res) {{\n{body}\n}});")
        };
        for (body, out) in [
            // A `var` or function declared after its use still binds the whole function.
            (
                "  function f() {\n    return res.code;\n    var res = 1;\n  }\n  console.log(res.code);",
                "  function f() {\n    return res.code;\n    var res = 1;\n  }\n  console.log(res.status);",
            ),
            (
                "  function f() {\n    console.log(res.code);\n    function res() {}\n  }\n  console.log(res.code);",
                "  function f() {\n    console.log(res.code);\n    function res() {}\n  }\n  console.log(res.status);",
            ),
            (
                "  [1].map(function () {\n    console.log(res.code);\n    if (true) {\n      var res = 2;\n    }\n  });\n  console.log(res.code);",
                "  [1].map(function () {\n    console.log(res.code);\n    if (true) {\n      var res = 2;\n    }\n  });\n  console.log(res.status);",
            ),
            // A named function expression binds its name inside itself only.
            (
                "  setTimeout(function res() {\n    console.log(res.code);\n  }, 1);\n  console.log(res.code);",
                "  setTimeout(function res() {\n    console.log(res.code);\n  }, 1);\n  console.log(res.status);",
            ),
            (
                "  const g = function res() {\n    return res.code;\n  };\n  console.log(res.code);",
                "  const g = function res() {\n    return res.code;\n  };\n  console.log(res.status);",
            ),
            (
                "  function g() {\n    console.log(res.code);\n    const x = function res() {};\n  }\n  console.log(res.code);",
                "  function g() {\n    console.log(res.status);\n    const x = function res() {};\n  }\n  console.log(res.status);",
            ),
            // An arrow's parameter shadows its concise body, which ends at the ternary's `:`.
            (
                "  const f = ok ? res => res.code : res.code;",
                "  const f = ok ? res => res.code : res.status;",
            ),
            (
                "  const f = ok ? (res) => res.code : res.code;",
                "  const f = ok ? (res) => res.code : res.status;",
            ),
            (
                "  const f = ok ? res => a ? res.code : 1 : res.code;",
                "  const f = ok ? res => a ? res.code : 1 : res.status;",
            ),
            (
                "  const f = ok ? res => a?.b ?? res.code : res.code;",
                "  const f = ok ? res => a?.b ?? res.code : res.status;",
            ),
            (
                "  const o = { a: ok ? res => res.code : res.code };",
                "  const o = { a: ok ? res => res.code : res.status };",
            ),
            // A line break starts a declaration in code without semicolons.
            (
                "  function g() {\n    const x = 1\n    function res() {}\n    return res.json()\n  }\n  res.json()",
                "  function g() {\n    const x = 1\n    function res() {}\n    return res.json()\n  }\n  res.data",
            ),
            // A declaration in the callback's own scope does not count, even in its own body.
            (
                "  function res2() {}\n  function res() {\n    return res.code;\n  }\n  console.log(res.code);",
                "  function res2() {}\n  function res() {\n    return res.status;\n  }\n  console.log(res.status);",
            ),
            (
                "  function g() {\n    return () => res.code;\n    var res;\n  }",
                "  function g() {\n    return () => res.code;\n    var res;\n  }",
            ),
            (
                "  f(async function* res() { res.code; });\n  res.code;",
                "  f(async function* res() { res.code; });\n  res.status;",
            ),
        ] {
            assert_eq!(t(&wrap(body)), want(out), "{body}");
        }
    }

    /// The `await` an arrow's concise body cannot hold ends with that body at a ternary's `:`.
    #[test]
    fn a_concise_arrow_body_ends_at_the_ternary_colon() {
        assert_eq!(
            t("const f = ok ? () => pm.sendRequest('u') : pm.sendRequest('v');"),
            "const f = ok ? () => bru.sendRequest('u') : await bru.sendRequest('v');"
        );
    }

    /// Outputs below were produced by the reference on the same input: removing an alias
    /// reprints its statement list, and a changed `return` is reprinted with its `;`.
    #[test]
    fn reprints_follow_the_reference() {
        for (src, want) in [
            (
                "const r = pm.response;\na(); b();\nr.json();",
                "a();b();\nres.getBody();",
            ),
            (
                "const r = pm.response;\na(); r.json();",
                "a();res.getBody();",
            ),
            (
                "const r = pm.response;\nr.json(); b();",
                "res.getBody();b();",
            ),
            (
                "function f() {\n  const r = pm.response;\n  return r.json()\n}",
                "function f() {\n  return res.getBody();\n}",
            ),
            ("const r = pm.response;;\nr.json();", "res.getBody();"),
            (
                "const r = pm.response;\nfoo()\nr.json()",
                "foo()\nres.getBody()",
            ),
            ("const r = pm.response; a(); b();", "a();b();"),
            (
                "a(); const r = pm.response; b(); r.json();",
                "a();b();res.getBody();",
            ),
            (
                "const r = pm.response;\nconst x = r.json()\nconsole.log(x)",
                "const x = res.getBody()\nconsole.log(x)",
            ),
            (
                "const r = pm.response;\nfunction f() { return r.code }",
                "function f() { return res.getStatus(); }",
            ),
            (
                "const r = pm.response;\nif (x) { a(); b(); }\nr.json();",
                "if (x) { a(); b(); }\nres.getBody();",
            ),
            ("const r = pm.response;;;\nr.json();", "res.getBody();"),
            (
                "const r = pm.response;\nlet y = 1;;\nr.json();",
                "let y = 1;\nres.getBody();",
            ),
            (
                "function f() { return pm.response.code }",
                "function f() { return res.getStatus(); }",
            ),
            (
                "function f() { return pm.environment.get('a') }",
                "function f() { return bru.getEnvVar('a'); }",
            ),
            (
                "function f() { return pm.response.json().id }",
                "function f() { return res.getBody().id; }",
            ),
            (
                "const r = pm.response;\nconst f = () => { return r.code }",
                "const f = () => { return res.getStatus(); }",
            ),
            (
                "const r = pm.response;\nfunction f() { if (x) return r.code\n  return 1 }",
                "function f() { if (x) return res.getStatus();\n  return 1 }",
            ),
            (
                "function f() { return responseBody }",
                "function f() { return res.getBody(); }",
            ),
            (
                "a(); b();\nconst r = pm.response;\nc(); d();\nr.json();",
                "a();b();\nc();d();\nres.getBody();",
            ),
            (
                "function f() {\n  a(); b();\n  const r = pm.response;\n  c(); ;\n  return r.json();\n}\ng(); h();",
                "function f() {\n  a();b();\n  c();\n  return res.getBody();\n}\ng(); h();",
            ),
            (
                "const r = pm.response;\nif (x) {} b();\nr.json();",
                "if (x) {}b();\nres.getBody();",
            ),
            (
                "const r = pm.response;\na() // c\nb(); /* d */ c();\nr.json();",
                "a() // c\nb();/* d */ c();\nres.getBody();",
            ),
            (
                "const r = pm.response;\nfor (;;) { a(); b(); }\n;\nr.json();",
                "for (;;) { a(); b(); }\nres.getBody();",
            ),
            (
                "const r = pm.response;\nlet o = {a: 1}; b();\nr.json();",
                "let o = {a: 1};b();\nres.getBody();",
            ),
            (
                "const r = pm.response;\na();\t b();\nr.json();",
                "a();b();\nres.getBody();",
            ),
            (
                "const r = pm.response;\na(); b()\nr.json();",
                "a();b()\nres.getBody();",
            ),
            (
                "const r = pm.response, x = 1; a(); b();\nr.json();",
                "const x = 1; a(); b();\nres.getBody();",
            ),
            // Nothing removed and no `return` changed: the spacing stays.
            (
                "a();pm.environment.set('x', 1);b()",
                "a();bru.setEnvVar('x', 1);b()",
            ),
            ("pm.environment.set('x', 1);;", "bru.setEnvVar('x', 1);;"),
            (
                "a(); b(); pm.environment.set('x', 1);",
                "a(); b(); bru.setEnvVar('x', 1);",
            ),
        ] {
            assert_eq!(t(src), want, "input {src:?}");
        }
    }

    /// Reprinting never touches what is not a statement boundary of the changed list.
    #[test]
    fn a_return_continued_on_the_next_line_gets_its_semicolon_at_the_end() {
        // Outputs produced by the reference on the same input.
        for (next, out) in [
            ("!== 200", "res.getStatus()\n    !== 200;"),
            ("!= 200", "res.getStatus()\n    != 200;"),
            ("^ 1", "res.getStatus()\n    ^ 1;"),
        ] {
            let src = format!("function f() {{\n  return pm.response.code\n    {next}\n}}");
            assert_eq!(
                t(&src),
                format!("function f() {{\n  return {out}\n}}"),
                "{src}"
            );
        }
        assert_eq!(
            t("function f() {\n  return pm.response.json()\n    instanceof Object\n}"),
            "function f() {\n  return res.getBody()\n    instanceof Object;\n}"
        );
        assert_eq!(
            t("function f() {\n  return pm.response.headers.has('a')\n    in o\n}"),
            "function f() {\n  return res.headerList.has('a')\n    in o;\n}"
        );
        assert_eq!(
            t("function f() {\n  return pm.response.code\n  g()\n}"),
            "function f() {\n  return res.getStatus();\n  g()\n}"
        );
    }

    /// Where the next line may or may not continue the argument, no `;` is added: a missing
    /// one is cosmetic, a wrong one breaks the script.
    #[test]
    fn a_return_whose_end_is_uncertain_gets_no_semicolon() {
        for next in ["!x", "~x", "++x", "--x", "'s'", "\"s\"", "5"] {
            let src = format!("function f() {{\n  return pm.response.code\n  {next}\n}}");
            let want = format!("function f() {{\n  return res.getStatus()\n  {next}\n}}");
            assert_eq!(t(&src), want, "{src}");
        }
        for tail in ["a++", "a--"] {
            let src = format!("function f() {{\n  return pm.response.code + {tail}\n  g()\n}}");
            let want = format!("function f() {{\n  return res.getStatus() + {tail}\n  g()\n}}");
            assert_eq!(t(&src), want, "{src}");
        }
        // A word operator at the end of a line carries the argument on to the next.
        for tail in ["typeof", "a instanceof", "void", "a in"] {
            let src = format!("function f() {{\n  return pm.response.code + {tail}\n  g()\n}}");
            let want = format!("function f() {{\n  return res.getStatus() + {tail}\n  g();\n}}");
            assert_eq!(t(&src), want, "{src}");
        }
    }

    #[test]
    fn reprints_keep_what_they_must() {
        for (src, want) in [
            (
                "const r = pm.response;\nif (a) {} else {}\nr.json();",
                "if (a) {} else {}\nres.getBody();",
            ),
            (
                "const r = pm.response;\ntry {} catch (e) {} finally {}\nr.json();",
                "try {} catch (e) {} finally {}\nres.getBody();",
            ),
            (
                "const r = pm.response;\ndo {} while (x); a();\nr.json();",
                "do {} while (x);a();\nres.getBody();",
            ),
            (
                "const r = pm.response;\nif (x) ;\nr.json();",
                "if (x) ;\nres.getBody();",
            ),
            (
                "const r = pm.response;\nx = {};\nr.json();",
                "x = {};\nres.getBody();",
            ),
            (
                "const r = pm.response;\na(); // c\nr.json();",
                "a(); // c\nres.getBody();",
            ),
            (
                "const r = pm.response;\nconst s = 'a; b'; `${1}; ${2}`;\nr.json();",
                "const s = 'a; b';`${1}; ${2}`;\nres.getBody();",
            ),
            (
                "function f() { return }\nfunction g() { return pm.x }",
                "function f() { return }\nfunction g() { return pm.x }",
            ),
            (
                "const r = pm.response;\nwhile (x) {}; a();\nr.json();",
                "while (x) {}a();\nres.getBody();",
            ),
            (
                "const r = pm.response;\nfunction g() {}; a();\nr.json();",
                "function g() {}a();\nres.getBody();",
            ),
            (
                "function f() { return pm.response.code; }",
                "function f() { return res.getStatus(); }",
            ),
            (
                "function f() {\n  return pm.response.code // why\n}",
                "function f() {\n  return res.getStatus(); // why\n}",
            ),
        ] {
            assert_eq!(t(src), want, "input {src:?}");
        }
    }

    #[test]
    fn reprints_and_scope_rules_stay_linear() {
        linear(40_000, |k| "return (".repeat(k));
        linear(20_000, |k| {
            "function f() { return ".repeat(k) + &"pm.environment.get('a') }".repeat(k)
        });
        linear(40_000, |k| {
            "function f() { return pm.environment.get('a') }\n".repeat(k)
        });
        linear(40_000, |k| {
            "const r = pm.response;\n".to_string() + &"a(); ;".repeat(k) + "r.json();"
        });
        linear(30_000, |k| {
            "const r = pm.response;\n".to_string() + &"if (x) {} ".repeat(k) + "r.json();"
        });
        linear(20_000, |k| {
            "pm.sendRequest('u', function (e, res) {".to_string()
                + &"ok ? res => res.code : ".repeat(k)
                + "1 });"
        });
        linear(20_000, |k| {
            "pm.sendRequest('u', function (e, res) {".to_string()
                + &"function f() { res.code; var res; ".repeat(k)
                + "});"
        });
        linear(20_000, |k| {
            "pm.sendRequest('u', function (e, res) {".to_string()
                + &"f(function res() { res.code; ".repeat(k)
                + "});"
        });
    }

    #[test]
    fn template_nesting_is_released_when_a_scan_panics() {
        let caught = std::panic::catch_unwind(|| {
            let _level = NestingGuard::enter();
            panic!("a scan failed mid-template");
        });
        assert!(caught.is_err());
        assert_eq!(NESTING.get(), 0);
        let deep =
            "`${".repeat(MAX_DEPTH - 2) + "pm.environment.get('a')" + &"}`".repeat(MAX_DEPTH - 2);
        assert!(t(&deep).contains("bru.getEnvVar('a')"));
    }

    #[test]
    fn translated_helpers_are_not_reported_but_untranslated_ones_are() {
        let r = translate(
            "pm.response.to.be.ok;\npm.response.to.be.json;\npm.response.to.be.withBody;",
        );
        assert_eq!(r.unsupported, ["pm.response.to.be.json"]);
        assert_eq!(r.replaced, 2);
    }

    #[test]
    fn status_helpers_are_rewritten_only_as_whole_statements() {
        assert_eq!(
            t(
                "const ok = pm.response.to.be.ok;\nfoo(pm.response.to.be.ok);\nx = pm.response.to.be.ok;"
            ),
            "const ok = pm.response.to.be.ok;\nfoo(pm.response.to.be.ok);\nx = pm.response.to.be.ok;"
        );
        assert_eq!(
            t("`${pm.response.to.be.ok}`;"),
            "`${pm.response.to.be.ok}`;"
        );
        assert_eq!(
            t("a();\npm.response.to.be.ok;"),
            "a();\nexpect(res.getStatus()).to.be.within(200, 299);"
        );
    }

    #[test]
    fn response_helpers_in_hostile_input_stay_bounded() {
        linear(60_000, |k| "pm.response.to.be.ok;\n".repeat(k));
        linear(60_000, |k| "pm.response.to.be.ok\n".repeat(k));
        linear(55_000, |k| "pm.response.to.be.ok(".repeat(k));
        linear(55_000, |k| "pm.response.to.be.not.".repeat(k));
        linear(100_000, |k| "else ".repeat(k) + "pm.response.to.be.ok");
        linear(40_000, |k| "pm.response.to.be.ok ".repeat(k) + "\n.x");
        linear(5_000, |k| "`${".repeat(k) + "pm.response.to.be.ok");
    }
}

#[cfg(test)]
mod global_tests {
    use super::*;

    #[test]
    fn uses_global_key_and_member_positions() {
        let cases = [
            ("var o = {G: 1};", false),
            ("var o = {a: 1, G: 1};", false),
            ("x.G", false),
            ("var o = {G};", true),
            ("a ? G : b", true),
            ("G.x", true),
        ];
        for (code, want) in cases {
            assert_eq!(uses_global(code, "G"), want, "{code}");
        }
    }

    #[test]
    fn prologue_end_offsets() {
        let end = |s: &str| directive_prologue_end(s);
        assert_eq!(end("var a;"), 0);
        assert_eq!(end("'use strict';\nvar a;"), "'use strict';".len());
        assert_eq!(end("\"use strict\"\nvar a;"), "\"use strict\"".len());
        assert_eq!(
            end("// c\n'a'; /* x */ 'b';\nx"),
            "// c\n'a'; /* x */ 'b';".len()
        );
        assert_eq!(end("'a' + b;"), 0);
        assert_eq!(end("'a'.length;"), 0);
    }

    #[test]
    fn requires_crypto_js_by_token() {
        assert!(requires_module("require ( \"crypto-js\" )", "crypto-js"));
        assert!(!requires_module("// require('crypto-js')", "crypto-js"));
        assert!(!requires_module(
            "var s = \"require('crypto-js')\";",
            "crypto-js"
        ));
    }
}
