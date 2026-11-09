//! Rewrites `bru.sendRequest` calls into Postman's `pm.sendRequest` shape.

use std::collections::{HashMap, HashSet};

use super::scan::{
    MAX_DEPTH, find_close, ident_end, is_ident_byte, is_property_access, parts, skip_opaque,
    skip_ws, template_end,
};

/// Rewrites each free-standing `bru.sendRequest(...)` for Postman; returns the code and the calls rewritten.
pub(super) fn rewrite(code: &str) -> (String, usize) {
    let mut em = Emitter {
        src: code,
        count: 0,
        configs: configs(code),
        responses: Vec::new(),
    };
    let mut out = String::with_capacity(code.len());
    em.emit(0, code.len(), 0, &mut out);
    (out, em.count)
}

/// The body mode a plain `Content-Type` header selects for an object `data`.
#[derive(Clone, Copy)]
enum Mode {
    Raw,
    Urlencoded,
    Formdata,
}

/// One property of an object literal, by byte ranges into the source.
enum Prop {
    /// `key: value`; `name` is the key's identifier or plain-string value.
    Pair {
        key: (usize, usize),
        name: Option<String>,
        ident: bool,
        val: (usize, usize),
    },
    /// `[expr]: value`.
    Computed {
        expr: (usize, usize),
        val: (usize, usize),
    },
    Short(usize, usize),
    Spread(usize, usize),
    /// A method, getter, setter or anything else not a plain value.
    Other,
}

struct Emitter<'a> {
    src: &'a str,
    count: usize,
    /// Offsets of the object literals that configs passed by name resolve to.
    configs: HashSet<usize>,
    /// Response parameters of the callbacks being emitted.
    responses: Vec<&'a str>,
}

/// What a `const|let|var NAME = ...` declaration starts with.
enum Init<'a> {
    Object(usize),
    Name(&'a str),
}

// Find every name passed as a lone first argument to `bru.sendRequest(` and follow each
// through `NAME = OTHER` declarations to the object literals it can denote. One visited set
// serves all names, since a name reached once has already contributed its objects.
fn configs(code: &str) -> HashSet<usize> {
    let mut called = Vec::new();
    let mut decls: HashMap<&str, Vec<Init>> = HashMap::new();
    scan_names(code, 0, code.len(), &mut called, &mut decls);
    let mut objects = HashSet::new();
    let mut visited = HashSet::new();
    while let Some(name) = called.pop() {
        if !visited.insert(name) {
            continue;
        }
        for init in decls.get(name).into_iter().flatten() {
            match init {
                Init::Object(at) => {
                    objects.insert(*at);
                }
                Init::Name(other) => called.push(other),
            }
        }
    }
    objects
}

/// Collect named-config calls and declarations in `code[from..to]`, template holes included.
fn scan_names<'a>(
    code: &'a str,
    from: usize,
    to: usize,
    called: &mut Vec<&'a str>,
    decls: &mut HashMap<&'a str, Vec<Init<'a>>>,
) {
    let b = code.as_bytes();
    let mut i = from;
    while i < to {
        if b[i] == b'`' {
            let end = template_end(code, i).min(to);
            let mut k = i + 1;
            while k < end {
                match b[k] {
                    b'\\' => k += 2,
                    b'$' if b.get(k + 1) == Some(&b'{') => {
                        let Some(close) = find_close(code, k + 1).filter(|c| *c < end) else {
                            break;
                        };
                        scan_names(code, k + 2, close, called, decls);
                        k = close;
                    }
                    _ => k += 1,
                }
            }
            i = end;
        } else if let Some(end) = skip_opaque(code, i) {
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i).min(to);
            if !is_property_access(b, i) {
                match &code[i..end] {
                    "bru" if code[end..to].starts_with(".sendRequest") => {
                        let open = skip_ws(b, end + ".sendRequest".len());
                        if b.get(open) == Some(&b'(')
                            && let Some((name, after)) = ident_at(code, skip_ws(b, open + 1))
                            && matches!(b.get(skip_ws(b, after)), Some(b',' | b')'))
                        {
                            called.push(name);
                        }
                    }
                    "const" | "let" | "var" => {
                        if let Some((name, init)) = declaration(code, end) {
                            decls.entry(name).or_default().push(init);
                        }
                    }
                    _ => {}
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
}

/// The identifier (not starting with a digit) at `at`, with its end.
fn ident_at(code: &str, at: usize) -> Option<(&str, usize)> {
    let b = code.as_bytes();
    let first = *b.get(at)?;
    if !is_ident_byte(first) || first.is_ascii_digit() {
        return None;
    }
    let end = ident_end(b, at);
    Some((&code[at..end], end))
}

/// `NAME = {` or `NAME = OTHER` ended by `;`, `,`, a line break or the end, after a keyword.
fn declaration(code: &str, keyword_end: usize) -> Option<(&str, Init<'_>)> {
    let b = code.as_bytes();
    let (name, after) = ident_at(code, skip_ws(b, keyword_end))?;
    let eq = skip_ws(b, after);
    if b.get(eq) != Some(&b'=') || matches!(b.get(eq + 1), Some(b'=' | b'>')) {
        return None;
    }
    let value = skip_ws(b, eq + 1);
    if b.get(value) == Some(&b'{') {
        return Some((name, Init::Object(value)));
    }
    let (other, end) = ident_at(code, value)?;
    let mut k = end;
    while matches!(b.get(k), Some(b' ' | b'\t')) {
        k += 1;
    }
    matches!(b.get(k), None | Some(b';' | b',' | b'\n' | b'\r'))
        .then_some((name, Init::Name(other)))
}

impl Emitter<'_> {
    // Copy `src[from..to]` to `out`, literals and comments as they are, descending into
    // template holes and rewriting each `bru.sendRequest(` call reached.
    fn emit(&mut self, from: usize, to: usize, depth: usize, out: &mut String) {
        let (src, b) = (self.src, self.src.as_bytes());
        if depth >= MAX_DEPTH {
            out.push_str(&src[from..to]);
            return;
        }
        let (mut i, mut copied) = (from, from);
        while i < to {
            if b[i] == b'`' {
                out.push_str(&src[copied..i]);
                i = self.template(i, to, depth, out);
                copied = i;
            } else if let Some(end) = skip_opaque(src, i) {
                i = end.min(to);
            } else if is_ident_byte(b[i]) {
                let end = ident_end(b, i).min(to);
                if !is_property_access(b, i)
                    && let Some(close) = self.send_request_at(i, end, to)
                {
                    out.push_str(&src[copied..i]);
                    self.call(i, close, depth, out);
                    i = close + 1;
                    copied = i;
                } else if !is_property_access(b, i)
                    && self.responses.contains(&&src[i..end])
                    && let Some((text, next)) = response_member(src, i, end)
                {
                    out.push_str(&src[copied..i]);
                    out.push_str(&text);
                    i = next;
                    copied = i;
                } else {
                    i = end;
                }
            } else if b[i] == b'{'
                && self.configs.contains(&i)
                && let Some(close) = find_close(src, i).filter(|c| *c < to)
            {
                out.push_str(&src[copied..i]);
                let before = self.count;
                let mut text = String::new();
                if self.options(i, close + 1, depth + 1, &mut text) {
                    out.push_str(&text);
                } else {
                    self.count = before;
                    out.push_str(&src[i..=close]);
                }
                i = close + 1;
                copied = i;
            } else {
                i += 1;
            }
        }
        out.push_str(&src[copied..to]);
    }

    // Rewrite a function or arrow callback as a plain Postman `function(E, R) {...}`, reading
    // the response the Postman way inside its body. False when `s..e` is no such function.
    fn callback(&mut self, s: usize, e: usize, depth: usize, out: &mut String) -> bool {
        let Some(cb) = Callback::parse(self.src, s, e) else {
            return false;
        };
        if depth >= MAX_DEPTH {
            out.push_str(&self.src[s..e]);
            return true;
        }
        let names: Vec<Option<&str>> = cb
            .params
            .iter()
            .take(2)
            .map(|&(ps, pe)| {
                ident_at(self.src, ps)
                    .filter(|(_, end)| *end == pe)
                    .map(|(n, _)| n)
            })
            .collect();
        let error = names.first().copied().flatten().unwrap_or("error");
        let response = names.get(1).copied().flatten().unwrap_or("response");
        if cb.is_async {
            out.push_str("async ");
        }
        out.push_str(&format!("function({error}, {response}) {{"));
        self.responses.push(response);
        match cb.body {
            Body::Block(open, close) => {
                self.emit(open + 1, close, depth, out);
                out.push('}');
            }
            Body::Expr(bs, be) => {
                out.push_str(" return ");
                self.emit(bs, be, depth, out);
                out.push_str("; }");
            }
        }
        self.responses.pop();
        true
    }

    /// Copy the template at `start`, emitting its holes one level deeper; returns its end.
    fn template(&mut self, start: usize, to: usize, depth: usize, out: &mut String) -> usize {
        let (src, b) = (self.src, self.src.as_bytes());
        let end = template_end(src, start).min(to);
        let (mut i, mut copied) = (start + 1, start);
        while i < end {
            match b[i] {
                b'\\' => i += 2,
                b'$' if b.get(i + 1) == Some(&b'{') => {
                    let Some(close) = find_close(src, i + 1).filter(|c| *c < end) else {
                        break;
                    };
                    out.push_str(&src[copied..i + 2]);
                    self.emit(i + 2, close, depth + 1, out);
                    i = close;
                    copied = close;
                }
                _ => i += 1,
            }
        }
        out.push_str(&src[copied..end]);
        end
    }

    /// The closing paren of a `bru.sendRequest(` call whose head spans `start..head_end`.
    fn send_request_at(&self, start: usize, head_end: usize, to: usize) -> Option<usize> {
        let (src, b) = (self.src, self.src.as_bytes());
        const MEMBER: &str = ".sendRequest";
        if &src[start..head_end] != "bru" || !src[head_end..to].starts_with(MEMBER) {
            return None;
        }
        let name_end = head_end + MEMBER.len();
        if ident_end(b, head_end + 1) != name_end {
            return None;
        }
        let open = skip_ws(b, name_end);
        (b.get(open) == Some(&b'('))
            .then(|| find_close(src, open))
            .flatten()
            .filter(|c| *c < to)
    }

    // Emit `pm.sendRequest(` with the first two arguments; a shape that cannot be followed
    // keeps the whole call as written and counts nothing.
    fn call(&mut self, start: usize, close: usize, depth: usize, out: &mut String) {
        let open = start + "bru.sendRequest".len();
        let open = skip_ws(self.src.as_bytes(), open);
        let before = self.count;
        let mut text = String::from("pm.sendRequest(");
        let mut ok = true;
        for (n, &(s, e)) in parts(self.src, open + 1, close).iter().take(2).enumerate() {
            if n > 0 {
                text.push_str(", ");
            }
            if n == 0 && self.is_object(s, e) {
                ok = self.options(s, e, depth + 1, &mut text);
            } else if n == 0 || !self.callback(s, e, depth + 1, &mut text) {
                self.emit(s, e, depth + 1, &mut text);
            }
            if !ok {
                break;
            }
        }
        if ok {
            text.push(')');
            out.push_str(&text);
            self.count += 1;
        } else {
            self.count = before;
            out.push_str(&self.src[start..=close]);
        }
    }

    fn is_object(&self, s: usize, e: usize) -> bool {
        self.src.as_bytes()[s] == b'{' && find_close(self.src, s) == Some(e - 1)
    }

    // Rewrite a request options object in place: `headers` becomes `header` and `data`
    // becomes a Postman body. False when a `data` object cannot become key-value pairs.
    fn options(&mut self, s: usize, e: usize, depth: usize, out: &mut String) -> bool {
        if depth >= MAX_DEPTH {
            out.push_str(&self.src[s..e]);
            return true;
        }
        let props: Vec<((usize, usize), Prop)> = parts(self.src, s + 1, e - 1)
            .into_iter()
            .map(|r| (r, self.prop(r.0, r.1)))
            .collect();
        let mode = props
            .iter()
            .find_map(|(_, p)| match p {
                Prop::Pair { name, val, .. } if name.as_deref() == Some("headers") => {
                    Some(self.mode(val.0, val.1))
                }
                Prop::Short(a, b) if &self.src[*a..*b] == "headers" => Some(Mode::Raw),
                _ => None,
            })
            .unwrap_or(Mode::Raw);
        let mut copied = s;
        for ((ps, pe), prop) in &props {
            match prop {
                Prop::Pair { key, name, .. } if name.as_deref() == Some("headers") => {
                    self.emit(copied, key.0, depth, out);
                    out.push_str("header");
                    copied = key.1;
                }
                Prop::Pair { key, name, val, .. } if name.as_deref() == Some("data") => {
                    self.emit(copied, key.0, depth, out);
                    out.push_str("body");
                    self.emit(key.1, val.0, depth, out);
                    if !self.body(val.0, val.1, mode, depth, out) {
                        return false;
                    }
                    copied = val.1;
                }
                Prop::Short(a, b) if matches!(&self.src[*a..*b], "headers" | "data") => {
                    self.emit(copied, *ps, depth, out);
                    if &self.src[*a..*b] == "headers" {
                        out.push_str("header: headers");
                    } else {
                        out.push_str("body: { mode: \"raw\", raw: data }");
                    }
                    copied = *pe;
                }
                _ => {}
            }
        }
        self.emit(copied, e, depth, out);
        true
    }

    /// The body mode a headers object selects through its first `Content-Type` key.
    fn mode(&self, s: usize, e: usize) -> Mode {
        if !self.is_object(s, e) {
            return Mode::Raw;
        }
        let value = parts(self.src, s + 1, e - 1)
            .into_iter()
            .find_map(|(ps, pe)| match self.prop(ps, pe) {
                Prop::Pair {
                    name: Some(n), val, ..
                } if n.eq_ignore_ascii_case("content-type") => {
                    Some(plain_string(self.src, val.0, val.1).map(str::to_ascii_lowercase))
                }
                _ => None,
            });
        match value.flatten() {
            Some(v) if v.contains("application/x-www-form-urlencoded") => Mode::Urlencoded,
            Some(v) if v.contains("multipart/form-data") => Mode::Formdata,
            _ => Mode::Raw,
        }
    }

    // Write the Postman body for the `data` value at `s..e`: raw, or for an object under a
    // form content type, its properties as key-value pairs.
    fn body(&mut self, s: usize, e: usize, mode: Mode, depth: usize, out: &mut String) -> bool {
        let name = match mode {
            Mode::Urlencoded if self.is_object(s, e) => "urlencoded",
            Mode::Formdata if self.is_object(s, e) => "formdata",
            _ => {
                out.push_str("{ mode: \"raw\", raw: ");
                self.emit(s, e, depth, out);
                out.push_str(" }");
                return true;
            }
        };
        out.push_str(&format!("{{ mode: \"{name}\", {name}: ["));
        for (n, (ps, pe)) in parts(self.src, s + 1, e - 1).into_iter().enumerate() {
            if n > 0 {
                out.push_str(", ");
            }
            match self.prop(ps, pe) {
                Prop::Pair {
                    key, ident, val, ..
                } => {
                    out.push_str("{ key: ");
                    if ident {
                        out.push_str(&format!("\"{}\"", &self.src[key.0..key.1]));
                    } else {
                        out.push_str(&self.src[key.0..key.1]);
                    }
                    out.push_str(", value: ");
                    self.emit(val.0, val.1, depth, out);
                    out.push_str(" }");
                }
                Prop::Computed { expr, val } => {
                    out.push_str("{ key: ");
                    self.emit(expr.0, expr.1, depth, out);
                    out.push_str(", value: ");
                    self.emit(val.0, val.1, depth, out);
                    out.push_str(" }");
                }
                Prop::Short(a, b) => {
                    let name = &self.src[a..b];
                    out.push_str(&format!("{{ key: \"{name}\", value: {name} }}"));
                }
                Prop::Spread(a, b) => {
                    out.push_str("...Object.entries(");
                    self.emit(a, b, depth, out);
                    out.push_str(").map(([key, value]) => ({ key: key, value: value }))");
                }
                Prop::Other => return false,
            }
        }
        out.push_str("] }");
        true
    }

    /// Classify the object property at `s..e`.
    fn prop(&self, s: usize, e: usize) -> Prop {
        let (src, b) = (self.src, self.src.as_bytes());
        if src[s..e].starts_with("...") {
            return Prop::Spread(skip_ws(b, s + 3), e);
        }
        let (key_end, name, ident) = match b[s] {
            b'\'' | b'"' => {
                let end = skip_opaque(src, s).unwrap_or(e).min(e);
                (end, plain_string(src, s, end).map(str::to_owned), false)
            }
            b'[' => {
                let Some(close) = find_close(src, s).filter(|c| *c < e) else {
                    return Prop::Other;
                };
                let colon = skip_ws(b, close + 1);
                if b.get(colon) != Some(&b':') {
                    return Prop::Other;
                }
                let expr = parts(src, s + 1, close);
                return match expr.as_slice() {
                    [expr] => Prop::Computed {
                        expr: *expr,
                        val: (skip_ws(b, colon + 1), e),
                    },
                    _ => Prop::Other,
                };
            }
            c if is_ident_byte(c) => {
                let end = ident_end(b, s).min(e);
                if end == e {
                    return Prop::Short(s, e);
                }
                let word = !c.is_ascii_digit();
                (end, word.then(|| src[s..end].to_owned()), word)
            }
            _ => return Prop::Other,
        };
        let colon = skip_ws(b, key_end);
        if colon >= e || b[colon] != b':' {
            return Prop::Other;
        }
        Prop::Pair {
            key: (s, key_end),
            name,
            ident,
            val: (skip_ws(b, colon + 1), e),
        }
    }
}

/// The value of a quoted string spanning exactly `s..e` that has no escapes.
fn plain_string(src: &str, s: usize, e: usize) -> Option<&str> {
    let t = &src[s..e];
    let q = *t.as_bytes().first()?;
    (matches!(q, b'\'' | b'"')
        && t.len() >= 2
        && skip_opaque(t, 0) == Some(t.len())
        && t.as_bytes()[t.len() - 1] == q
        && !t.contains('\\'))
    .then(|| &t[1..t.len() - 1])
}

/// `R.data`, `R.status` or `R.statusText` at `start`, as Postman reads them, and where it ends.
fn response_member(src: &str, start: usize, head_end: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let dot = skip_ws(b, head_end);
    if b.get(dot) != Some(&b'.') || b.get(dot + 1) == Some(&b'.') {
        return None;
    }
    let (member, end) = ident_at(src, skip_ws(b, dot + 1))?;
    let head = &src[start..head_end];
    let text = match member {
        "data" => format!("{head}.json()"),
        "status" => format!("{head}.code"),
        "statusText" => format!("{head}.status"),
        _ => return None,
    };
    Some((text, end))
}

enum Body {
    /// The braces of a block body.
    Block(usize, usize),
    Expr(usize, usize),
}

/// A function or arrow function spanning an argument.
struct Callback {
    is_async: bool,
    params: Vec<(usize, usize)>,
    body: Body,
}

impl Callback {
    fn parse(src: &str, s: usize, e: usize) -> Option<Callback> {
        let b = src.as_bytes();
        let mut p = s;
        let mut is_async = false;
        if let Some(("async", end)) = ident_at(src, p) {
            let next = skip_ws(b, end);
            if !src[next..e].starts_with("=>") && next < e {
                is_async = true;
                p = next;
            }
        }
        let params;
        let body_at;
        let is_function = matches!(ident_at(src, p), Some(("function", _)));
        if let Some(("function", end)) = ident_at(src, p) {
            let mut k = skip_ws(b, end);
            if let Some((_, name_end)) = ident_at(src, k) {
                k = skip_ws(b, name_end);
            }
            if b.get(k) != Some(&b'(') {
                return None;
            }
            let close = find_close(src, k).filter(|c| *c < e)?;
            params = parts(src, k + 1, close);
            body_at = skip_ws(b, close + 1);
            if b.get(body_at) != Some(&b'{') {
                return None;
            }
        } else {
            let after = if b.get(p) == Some(&b'(') {
                let close = find_close(src, p).filter(|c| *c < e)?;
                params = parts(src, p + 1, close);
                close + 1
            } else {
                let (_, end) = ident_at(src, p)?;
                params = vec![(p, end)];
                end
            };
            let arrow = skip_ws(b, after);
            if !src[arrow..e].starts_with("=>") {
                return None;
            }
            body_at = skip_ws(b, arrow + 2);
        }
        let body = if b.get(body_at) == Some(&b'{') && find_close(src, body_at) == Some(e - 1) {
            Body::Block(body_at, e - 1)
        } else if is_function || body_at >= e {
            return None;
        } else {
            Body::Expr(body_at, e)
        };
        Some(Callback {
            is_async,
            params,
            body,
        })
    }
}
