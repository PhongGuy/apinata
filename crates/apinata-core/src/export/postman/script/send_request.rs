//! Rewrites `bru.sendRequest` calls into Postman's `pm.sendRequest` shape.

use super::scan::{
    MAX_DEPTH, find_close, ident_end, is_ident_byte, is_property_access, parts, skip_opaque,
    skip_ws, template_end,
};

/// Rewrites each free-standing `bru.sendRequest(...)` for Postman; returns the code and the calls rewritten.
pub(super) fn rewrite(code: &str) -> (String, usize) {
    let mut em = Emitter {
        src: code,
        count: 0,
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
                } else {
                    i = end;
                }
            } else {
                i += 1;
            }
        }
        out.push_str(&src[copied..to]);
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
            } else {
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
