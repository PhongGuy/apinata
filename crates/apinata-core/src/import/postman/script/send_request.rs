//! `pm.sendRequest(...)` to `bru.sendRequest(...)`, as the reference's
//! `send-request-transformer.js` does (usebruno/bruno @1301ea17). The request object gets
//! `header` -> `headers` and its body folded into `data`, the callback becomes `async` with
//! its response accessed the way `bru.sendRequest` returns it, and the call is awaited where
//! `await` is legal. The reference reprints the call with a printer; the layout rules that
//! matter (wrap past 74 columns, fresh objects one property per line) are reproduced here.

use std::collections::HashSet;

use super::{
    Ctx, ident_end, is_ident_byte, is_property_access, prev_significant, skip_opaque, skip_ws,
    split_top_level, translate_code,
};

/// Column past which the reference's printer puts each call argument on its own line.
const WRAP_COLUMN: usize = 74;
/// Longest `.then` chain followed after a call.
const MAX_LINKS: usize = 64;

/// Marks a request shape the translator does not follow; the call is left as written.
struct Bail;

fn off(base: &str, part: &str) -> usize {
    part.as_ptr() as usize - base.as_ptr() as usize
}

/// Absolute ranges of the comma separated parts of `src[from..to]`, empty parts dropped.
fn parts(src: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let inner = &src[from..to];
    split_top_level(inner, b',')
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let s = from + off(inner, p);
            (s, s + p.len())
        })
        .collect()
}

/// Indentation (tabs count to the next multiple of 4) of the line holding `pos`. Looks back a bounded distance so a very long
/// line cannot make repeated calls quadratic.
fn line_indent(src: &str, pos: usize) -> usize {
    let b = src.as_bytes();
    let lo = pos.saturating_sub(256);
    let ls = b[lo..pos]
        .iter()
        .rposition(|c| *c == b'\n')
        .map_or(lo, |n| lo + n + 1);
    super::columns(&b[ls..pos])
}

/// Remove up to `by` leading spaces from every line after the first.
fn strip_tail(text: &str, by: usize) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
            let lead = line.bytes().take_while(|c| *c == b' ').count().min(by);
            out.push_str(&line[lead..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Add `by` spaces before every non-empty line after the first.
fn indent_tail(text: &str, by: usize) -> String {
    indent_from(text, by, 1)
}

/// Add `by` spaces before every non-empty line.
fn indent_all(text: &str, by: usize) -> String {
    indent_from(text, by, 0)
}

fn indent_from(text: &str, by: usize, from: usize) -> String {
    let pad = " ".repeat(by);
    let mut out = String::with_capacity(text.len() + by);
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if n >= from && !line.is_empty() {
            out.push_str(&pad);
        }
        out.push_str(line);
    }
    out
}

/// An object literal the way the reference's printer lays out a fresh one: one property
/// per line, and a blank line around a property that spans lines.
fn print_object(props: &[String], tab: usize) -> String {
    if props.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::from("{\n");
    let mut allow_break = false;
    for (i, p) in props.iter().enumerate() {
        let lines = indent_all(p, tab);
        let multi = lines.contains('\n');
        if multi && allow_break {
            out.push('\n');
        }
        out.push_str(&lines);
        if i + 1 < props.len() {
            out.push_str(if multi { ",\n\n" } else { ",\n" });
            allow_break = !multi;
        }
    }
    out.push_str("\n}");
    out
}

fn is_plain_ident(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(is_ident_byte)
        && !s.as_bytes()[0].is_ascii_digit()
        && s.is_ascii()
}

// ---------------------------------------------------------------------------------
// Request object
// ---------------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum PKey {
    Ident(String),
    Str(String),
    Other,
}

impl PKey {
    fn is(&self, name: &str) -> bool {
        matches!(self, PKey::Ident(n) | PKey::Str(n) if n == name)
    }

    /// The same kind of key spelled `name`: a string key is reprinted double-quoted.
    fn renamed(&self, name: &str) -> String {
        match self {
            PKey::Str(_) => format!("\"{name}\""),
            _ => name.to_string(),
        }
    }
}

struct Prop {
    range: (usize, usize),
    key: PKey,
    key_range: (usize, usize),
    val: Option<(usize, usize)>,
}

/// A property of the object literal `src[s..e]` (braces included).
fn parse_props(src: &str, s: usize, e: usize) -> Result<Vec<Prop>, Bail> {
    let mut out = Vec::new();
    for (ps, pe) in parts(src, s + 1, e - 1) {
        let t = &src[ps..pe];
        let b = t.as_bytes();
        if t.starts_with("...") {
            return Err(Bail);
        }
        let (key, ke) = match b[0] {
            q @ (b'\'' | b'"') => {
                let end = skip_opaque(t, 0).ok_or(Bail)?;
                let inner = &t[1..end.saturating_sub(1).max(1)];
                let simple = end >= 2 && b[end - 1] == q && !inner.contains('\\');
                (
                    if simple {
                        PKey::Str(inner.to_string())
                    } else {
                        PKey::Other
                    },
                    end,
                )
            }
            c if is_ident_byte(c) => {
                let end = ident_end(b, 0);
                (PKey::Ident(t[..end].to_string()), end)
            }
            _ => (PKey::Other, 0),
        };
        let colon = skip_ws(b, ke);
        if key != PKey::Other && b.get(colon) == Some(&b':') {
            let vs = ps + skip_ws(b, colon + 1);
            out.push(Prop {
                range: (ps, pe),
                key,
                key_range: (ps, ps + ke),
                val: (vs < pe).then_some((vs, pe)),
            });
        } else {
            // A shorthand `header` or `body` cannot be edited in place.
            if key.is("header") || key.is("body") {
                return Err(Bail);
            }
            out.push(Prop {
                range: (ps, pe),
                key: PKey::Other,
                key_range: (ps, ps),
                val: None,
            });
        }
    }
    Ok(out)
}

fn is_object(src: &str, s: usize, e: usize) -> bool {
    src.as_bytes().get(s) == Some(&b'{') && e > s + 1 && super::find_close(src, s) == Some(e - 1)
}

fn is_array(src: &str, s: usize, e: usize) -> bool {
    src.as_bytes().get(s) == Some(&b'[') && e > s + 1 && super::find_close(src, s) == Some(e - 1)
}

/// The text of a plain string literal that spans exactly `src[s..e]`.
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

fn rel_of(src: &str, (s, e): (usize, usize)) -> String {
    strip_tail(&src[s..e], line_indent(src, s))
}

/// A property of a fresh object, with its lower-cased string key when it has one.
type Fresh = (Option<String>, String);

/// `[{key: 'A', value: v}, ...]` as the properties of an object, `"A": v`.
fn convert_pairs(src: &str, s: usize, e: usize) -> Result<Vec<Fresh>, Bail> {
    let mut out = Vec::new();
    for (es, ee) in parts(src, s + 1, e - 1) {
        if !is_object(src, es, ee) {
            return Err(Bail);
        }
        let props = parse_props(src, es, ee)?;
        let key = props.iter().find(|p| p.key.is("key") && p.val.is_some());
        let value = props.iter().find(|p| p.key.is("value") && p.val.is_some());
        let (Some(key), Some(value)) = (key, value) else {
            continue;
        };
        let (ks, ke) = key.val.ok_or(Bail)?;
        let name = plain_string(src, ks, ke).ok_or(Bail)?;
        let text = format!(
            "\"{}\": {}",
            name.replace('"', "\\\""),
            rel_of(src, value.val.ok_or(Bail)?)
        );
        out.push((Some(name.to_lowercase()), text));
    }
    Ok(out)
}

enum VNew {
    /// A replacement value, relative to its own first line.
    Text(String),
    /// A fresh object.
    Obj(Vec<Fresh>),
    /// Edits inside the existing value, in source coordinates.
    Patch(Vec<(usize, usize, String)>),
}

struct PM {
    prop: Prop,
    /// The key as it reads after the header rename.
    eff: PKey,
    key_edit: Option<String>,
    vnew: Option<VNew>,
}

/// Rewrite the request object `src[s..e]`; `Ok(None)` when nothing changes. The text
/// returned replaces `src[s..e]`.
fn transform_options(src: &str, s: usize, e: usize, tab: usize) -> Result<Option<String>, Bail> {
    let mut m: Vec<PM> = parse_props(src, s, e)?
        .into_iter()
        .map(|prop| PM {
            eff: prop.key.clone(),
            prop,
            key_edit: None,
            vnew: None,
        })
        .collect();
    let mut appended: Vec<String> = Vec::new();

    for pm in &mut m {
        if !pm.prop.key.is("header") {
            continue;
        }
        pm.key_edit = Some(pm.prop.key.renamed("headers"));
        pm.eff = PKey::Ident("headers".into());
        if let Some((vs, ve)) = pm.prop.val
            && is_array(src, vs, ve)
        {
            pm.vnew = Some(VNew::Obj(convert_pairs(src, vs, ve)?));
        }
    }

    for idx in 0..m.len() {
        if !m[idx].prop.key.is("body") {
            continue;
        }
        let Some((vs, ve)) = m[idx].prop.val else {
            continue;
        };
        if !is_object(src, vs, ve) {
            continue;
        }
        let body = parse_props(src, vs, ve)?;
        let Some(mode) = body
            .iter()
            .find(|p| p.key.is("mode"))
            .and_then(|p| p.val)
            .and_then(|(a, b)| plain_string(src, a, b))
        else {
            continue;
        };
        let key_edit = m[idx].prop.key.renamed("data");
        match mode {
            "raw" => {
                if let Some(raw) = body.iter().find(|p| p.key.is("raw")).and_then(|p| p.val) {
                    m[idx].key_edit = Some(key_edit);
                    m[idx].vnew = Some(VNew::Text(rel_of(src, raw)));
                }
            }
            "urlencoded" | "formdata" => {
                let list = body
                    .iter()
                    .find(|p| p.key.is(mode))
                    .and_then(|p| p.val)
                    .filter(|(a, b)| is_array(src, *a, *b));
                if let Some((a, b)) = list {
                    m[idx].key_edit = Some(key_edit);
                    m[idx].vnew = Some(VNew::Obj(convert_pairs(src, a, b)?));
                    let ct = if mode == "formdata" {
                        "multipart/form-data"
                    } else {
                        "application/x-www-form-urlencoded"
                    };
                    set_header(src, &mut m, &mut appended, "Content-Type", ct, tab)?;
                }
            }
            _ => {}
        }
    }

    if appended.is_empty() && m.iter().all(|p| p.key_edit.is_none() && p.vnew.is_none()) {
        return Ok(None);
    }
    Ok(Some(render(src, s, e, &m, &appended, tab)))
}

/// The reference's `addOrUpdateHeader`: drop any same-named string-keyed header, then append.
fn set_header(
    src: &str,
    m: &mut [PM],
    appended: &mut Vec<String>,
    name: &str,
    value: &str,
    tab: usize,
) -> Result<(), Bail> {
    let lc = name.to_lowercase();
    let new: Fresh = (Some(lc.clone()), format!("\"{name}\": \"{value}\""));
    let Some(pm) = m.iter_mut().find(|p| p.eff.is("headers")) else {
        appended.push(format!("headers: {}", print_object(&[new.1], tab)));
        return Ok(());
    };
    match &mut pm.vnew {
        Some(VNew::Obj(list)) => {
            list.retain(|(k, _)| k.as_deref() != Some(lc.as_str()));
            list.push(new);
        }
        Some(_) => pm.vnew = Some(VNew::Obj(vec![new])),
        None => {
            let (vs, ve) = pm.prop.val.ok_or(Bail)?;
            if !is_object(src, vs, ve) {
                pm.vnew = Some(VNew::Obj(vec![new]));
                return Ok(());
            }
            let old = parse_props(src, vs, ve)?;
            let kept: Vec<&Prop> = old
                .iter()
                .filter(|p| !matches!(&p.key, PKey::Str(k) if k.to_lowercase() == lc))
                .collect();
            if kept.len() + 1 == old.len() {
                // The same number of properties: the printer swaps them one by one.
                let mut edits = Vec::new();
                for (i, o) in old.iter().enumerate() {
                    let text = match kept.get(i) {
                        Some(k) if k.range == o.range => continue,
                        Some(k) => src[k.range.0..k.range.1].to_string(),
                        None => new.1.clone(),
                    };
                    edits.push((o.range.0, o.range.1, text));
                }
                pm.vnew = Some(VNew::Patch(edits));
            } else {
                let mut list: Vec<Fresh> = kept
                    .iter()
                    .map(|p| {
                        let k = match &p.key {
                            PKey::Str(k) => Some(k.to_lowercase()),
                            _ => None,
                        };
                        (k, rel_of(src, p.range))
                    })
                    .collect();
                list.push(new);
                pm.vnew = Some(VNew::Obj(list));
            }
        }
    }
    Ok(())
}

fn splice(src: &str, from: usize, to: usize, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(to - from);
    let mut at = from;
    for (s, e, t) in edits {
        if s < at || e > to {
            continue;
        }
        out.push_str(&src[at..s]);
        out.push_str(&t);
        at = e;
    }
    out.push_str(&src[at..to]);
    out
}

/// Edits of one property; fresh values are placed against `base` indentation.
fn prop_edits(src: &str, pm: &PM, base: Option<usize>, tab: usize) -> Vec<(usize, usize, String)> {
    let mut edits = Vec::new();
    if let Some(k) = &pm.key_edit {
        edits.push((pm.prop.key_range.0, pm.prop.key_range.1, k.clone()));
    }
    let (Some(v), Some((vs, ve))) = (&pm.vnew, pm.prop.val) else {
        return edits;
    };
    let at = base.unwrap_or_else(|| line_indent(src, vs));
    match v {
        VNew::Text(t) => edits.push((vs, ve, indent_tail(t, at))),
        VNew::Obj(list) => {
            let texts: Vec<String> = list.iter().map(|(_, t)| t.clone()).collect();
            edits.push((vs, ve, indent_tail(&print_object(&texts, tab), at)));
        }
        VNew::Patch(p) => edits.extend(p.iter().cloned()),
    }
    edits
}

fn render(src: &str, s: usize, e: usize, m: &[PM], appended: &[String], tab: usize) -> String {
    if appended.is_empty() {
        let edits = m
            .iter()
            .flat_map(|pm| prop_edits(src, pm, None, tab))
            .collect();
        return splice(src, s, e, edits);
    }
    // A property was added, so the object is printed afresh, one property per line.
    let mut texts: Vec<String> = m
        .iter()
        .map(|pm| {
            let base = line_indent(src, pm.prop.range.0);
            let edits = prop_edits(src, pm, Some(base), tab);
            let text = splice(src, pm.prop.range.0, pm.prop.range.1, edits);
            strip_tail(&text, base)
        })
        .collect();
    texts.extend(appended.iter().cloned());
    indent_tail(&print_object(&texts, tab), line_indent(src, s))
}

// ---------------------------------------------------------------------------------
// Callback and promise chain
// ---------------------------------------------------------------------------------

struct FnParts<'a> {
    is_async: bool,
    generator: bool,
    name: Option<&'a str>,
    arrow: bool,
    params: Vec<&'a str>,
    /// Offset in the text where the body starts.
    body_at: usize,
}

/// Split a function or arrow expression that spans the whole `text`.
fn parse_fn<'a>(text: &'a str, cx: &mut Ctx) -> Option<FnParts<'a>> {
    let b = text.as_bytes();
    let word = |i: usize| &text[i..ident_end(b, i)];
    let mut i = 0;
    let mut is_async = false;
    if word(0) == "async" && b.get(5).is_some_and(|c| c.is_ascii_whitespace()) {
        is_async = true;
        i = skip_ws(b, 5);
    }
    let params_of = |open: usize, cx: &mut Ctx| -> Option<(Vec<&'a str>, usize)> {
        let close = cx.find_close(text, open)?;
        let ps: Vec<&str> = split_top_level(&text[open + 1..close], b',')
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        Some((ps, close))
    };
    if word(i) == "function" {
        i += 8;
        i = skip_ws(b, i);
        let generator = b.get(i) == Some(&b'*');
        if generator {
            i = skip_ws(b, i + 1);
        }
        let mut name = None;
        if b.get(i).is_some_and(|c| is_ident_byte(*c)) {
            let e = ident_end(b, i);
            name = Some(&text[i..e]);
            i = skip_ws(b, e);
        }
        if b.get(i) != Some(&b'(') {
            return None;
        }
        let (params, close) = params_of(i, cx)?;
        let body_at = skip_ws(b, close + 1);
        if b.get(body_at) != Some(&b'{') || cx.find_close(text, body_at)? != text.len() - 1 {
            return None;
        }
        return Some(FnParts {
            is_async,
            generator,
            name,
            arrow: false,
            params,
            body_at,
        });
    }
    let (params, after) = if b.get(i) == Some(&b'(') {
        let (ps, close) = params_of(i, cx)?;
        (ps, close + 1)
    } else if b.get(i).is_some_and(|c| is_ident_byte(*c)) {
        let e = ident_end(b, i);
        (vec![&text[i..e]], e)
    } else {
        return None;
    };
    let arrow = skip_ws(b, after);
    if !text[arrow..].starts_with("=>") {
        return None;
    }
    let body_at = skip_ws(b, arrow + 2);
    if body_at >= text.len() {
        return None;
    }
    if b[body_at] == b'{' && cx.find_close(text, body_at)? != text.len() - 1 {
        return None;
    }
    Some(FnParts {
        is_async,
        generator: false,
        name: None,
        arrow: true,
        params,
        body_at,
    })
}

/// The function printed the reference's way, made `async`; an `async` one is kept as
/// written unless `normalize` asks for the reprint anyway.
fn print_async(text: &str, f: &FnParts, normalize: bool) -> String {
    let body = &text[f.body_at..];
    if f.is_async && !normalize {
        return text.to_string();
    }
    let params = f.params.join(", ");
    if f.arrow {
        if f.params.len() == 1 && is_plain_ident(f.params[0]) {
            format!("async {} => {body}", f.params[0])
        } else {
            format!("async ({params}) => {body}")
        }
    } else {
        let star = if f.generator { "*" } else { "" };
        let name = f.name.map(|n| format!(" {n}")).unwrap_or_default();
        format!("async function{star}{name}({params}) {body}")
    }
}

fn prev_word(b: &[u8], pos: usize) -> &str {
    match prev_significant(b, pos) {
        Some((k, c)) if is_ident_byte(c) => {
            let mut s = k + 1;
            while s > 0 && is_ident_byte(b[s - 1]) {
                s -= 1;
            }
            std::str::from_utf8(&b[s..k + 1]).unwrap_or("")
        }
        _ => "",
    }
}

/// Whether `body` assigns to `name` anywhere.
fn reassigns(body: &str, name: &str) -> bool {
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(body, i) {
            i = end;
            continue;
        }
        if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if &body[i..end] == name && !is_property_access(b, i) {
                let rest = &body[skip_ws(b, end)..];
                let plain =
                    rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>");
                let compound = [
                    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**=", "<<=", ">>=", ">>>=",
                    "&&=", "||=", "??=", "++", "--",
                ]
                .iter()
                .any(|o| rest.starts_with(o));
                if plain || compound {
                    return true;
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    false
}

/// Whether a `.then` handler hands its response parameter on untouched, so the next
/// handler in the chain receives the response too.
fn returns_param(text: &str, f: &FnParts, name: &str) -> bool {
    let body = &text[f.body_at..];
    if !body.starts_with('{') {
        return body.trim() == name;
    }
    let inner = body[1..body.len() - 1].trim_end();
    let last = inner.trim_end_matches(';').trim_end();
    let tail_ok = last.strip_suffix(name).is_some_and(|head| {
        let head = head.trim_end();
        head.strip_suffix("return")
            .is_some_and(|h| !h.as_bytes().last().is_some_and(|c| is_ident_byte(*c)))
    });
    if !tail_ok {
        return false;
    }
    // Every return in the body must be `return name`.
    let b = inner.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(inner, i) {
            i = end;
            continue;
        }
        if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if &inner[i..end] == "return" && !is_property_access(b, i) {
                let after = skip_ws(b, end);
                let e2 = ident_end(b, after);
                if &inner[after..e2] != name {
                    return false;
                }
                let rest = inner[e2..].trim_start_matches([' ', '\t']);
                if !(rest.is_empty() || rest.starts_with([';', '}', '\n', '\r'])) {
                    return false;
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    true
}

/// `text` with the response access of its body (from `body_at`) rewritten for `name`.
fn with_response(text: &str, body_at: usize, name: &str) -> String {
    let body = &text[body_at..];
    // A first pass finds the nested functions that declare the name anywhere in their body,
    // since `var` and function declarations bind it before their own line.
    let mut hoisted = HashSet::new();
    let mut scan = Response {
        name,
        hoisted: &mut hoisted,
        collect: true,
    };
    scan.rewrite(body, 0, false, 0);
    scan.collect = false;
    format!("{}{}", &text[..body_at], scan.rewrite(body, 0, false, 0))
}

struct ScopeFrame {
    /// A function body, which owns the declarations inside it.
    func: bool,
    /// The name is rebound in this frame or one around it.
    shadow: bool,
    open: u8,
    /// Conditional `?`s in this frame still waiting for their `:`.
    pending_colons: u32,
}

/// Rewrites `name.json()`, `name.code` and the like in a callback body, leaving alone
/// whatever a nested function that rebinds `name` (parameter, declaration, its own name as
/// an expression, `catch`) refers to. The callback's own scope is not nested, so its own
/// redeclarations do not stop it.
struct Response<'n, 'h> {
    name: &'n str,
    /// Absolute offsets of the nested function bodies that declare the name.
    hoisted: &'h mut HashSet<usize>,
    /// The first pass only fills `hoisted`.
    collect: bool,
}

impl Response<'_, '_> {
    fn rewrite(&mut self, src: &str, base: usize, shadowed: bool, depth: usize) -> String {
        let name = self.name;
        let b = src.as_bytes();
        let mut out = String::with_capacity(src.len());
        let mut frames: Vec<ScopeFrame> = Vec::new();
        // Where each open frame starts, for the hoisting record.
        let mut starts: Vec<usize> = Vec::new();
        let mut occ: Vec<usize> = Vec::new();
        let mut opens: Vec<usize> = Vec::new();
        // Index of the `)` or `>` after which the next function body shadows the name, and
        // the last closed parenthesis group: its index, whether it is a control head, `catch`.
        let mut pending: Option<usize> = None;
        let mut last_close: Option<(usize, bool, bool)> = None;
        // The `(` of a function expression named after the response.
        let mut named_fn: Option<usize> = None;
        let mut i = 0;
        let cur = |frames: &Vec<ScopeFrame>| shadowed || frames.last().is_some_and(|f| f.shadow);
        let push = |frames: &mut Vec<ScopeFrame>, starts: &mut Vec<usize>, at, frame| {
            frames.push(frame);
            starts.push(at);
        };
        while i < b.len() {
            if b[i] == b'`' {
                let end = super::template_end(src, i);
                let mut k = i + 1;
                let mut from = i;
                while k < end {
                    match b[k] {
                        b'\\' => k += 2,
                        b'$' if b.get(k + 1) == Some(&b'{') => {
                            let Some(close) = super::find_close(src, k + 1).filter(|c| *c < end)
                            else {
                                break;
                            };
                            out.push_str(&src[from..k + 2]);
                            let hole = &src[k + 2..close];
                            if depth < super::MAX_DEPTH {
                                let inner = cur(&frames);
                                out.push_str(&self.rewrite(hole, base + k + 2, inner, depth + 1));
                            } else {
                                out.push_str(hole);
                            }
                            from = close;
                            k = close + 1;
                        }
                        _ => k += 1,
                    }
                }
                out.push_str(&src[from..end]);
                i = end;
                continue;
            }
            if let Some(end) = skip_opaque(src, i) {
                out.push_str(&src[i..end]);
                i = end;
                continue;
            }
            let c = b[i];
            if is_ident_byte(c) {
                let end = ident_end(b, i);
                let word = &src[i..end];
                if word == name && !is_property_access(b, i) {
                    occ.push(i);
                    let next = skip_ws(b, end);
                    if src[next..].starts_with("=>") {
                        pending = Some(next + 1);
                    }
                    match function_named(b, i) {
                        Some(false) if b.get(next) == Some(&b'(') => named_fn = Some(next),
                        _ => {}
                    }
                    let declares = matches!(prev_word(b, i), "var" | "let" | "const" | "class")
                        || function_named(b, i) == Some(true);
                    if declares && let Some(k) = frames.iter().rposition(|f| f.func) {
                        if self.collect {
                            self.hoisted.insert(base + starts[k]);
                        }
                        for f in &mut frames[k..] {
                            f.shadow = true;
                        }
                    }
                    if !cur(&frames)
                        && let Some((text, next)) = response_member(src, end, name)
                    {
                        out.push_str(&text);
                        i = next;
                        continue;
                    }
                }
                out.push_str(word);
                i = end;
                continue;
            }
            let inherit = cur(&frames);
            let frame = |open, func, shadow| ScopeFrame {
                func,
                shadow,
                open,
                pending_colons: 0,
            };
            match c {
                b'(' => {
                    opens.push(i);
                    push(&mut frames, &mut starts, i, frame(b'(', false, inherit));
                }
                b'[' => push(&mut frames, &mut starts, i, frame(b'[', false, inherit)),
                b'{' => {
                    let (func, shadow) = match prev_significant(b, i) {
                        Some((k, b'>')) if k > 0 && b[k - 1] == b'=' => (true, pending == Some(k)),
                        Some((k, b')')) => match last_close {
                            Some((at, control, catch)) if at == k => (
                                !control && !catch,
                                (!control || catch) && pending == Some(k),
                            ),
                            _ => (false, false),
                        },
                        _ => (false, false),
                    };
                    let hoist = func && self.hoisted.contains(&(base + i));
                    push(
                        &mut frames,
                        &mut starts,
                        i,
                        frame(b'{', func, inherit || shadow || hoist),
                    );
                }
                b')' | b']' | b'}' => {
                    while frames.last().is_some_and(|f| f.open == b'>') {
                        frames.pop();
                        starts.pop();
                    }
                    frames.pop();
                    starts.pop();
                    if c == b')'
                        && let Some(open) = opens.pop()
                    {
                        let next = skip_ws(b, i + 1);
                        let head = prev_word(b, open);
                        let control = matches!(head, "if" | "for" | "while" | "switch" | "with");
                        let catch = head == "catch";
                        last_close = Some((i, control, catch));
                        let binds = src[next..].starts_with("=>") || b.get(next) == Some(&b'{');
                        let lo = occ.partition_point(|p| *p < open);
                        let in_params = occ.get(lo).is_some_and(|p| *p < i);
                        if binds && !control && (in_params || named_fn == Some(open)) {
                            pending = Some(i);
                        }
                    }
                }
                b',' | b';' => {
                    while frames.last().is_some_and(|f| f.open == b'>') {
                        frames.pop();
                        starts.pop();
                    }
                }
                b'?' if is_conditional(b, i) => {
                    if let Some(f) = frames.last_mut() {
                        f.pending_colons += 1;
                    }
                }
                b':' => {
                    // The `:` of a conditional ends the concise arrow bodies opened since its `?`.
                    while frames
                        .last()
                        .is_some_and(|f| f.open == b'>' && f.pending_colons == 0)
                    {
                        frames.pop();
                        starts.pop();
                    }
                    if let Some(f) = frames.last_mut() {
                        f.pending_colons = f.pending_colons.saturating_sub(1);
                    }
                }
                b'>' if i > 0 && b[i - 1] == b'=' => {
                    let params_shadow = match prev_significant(b, i - 1) {
                        Some((k, b')')) => pending == Some(k),
                        _ => pending == Some(i),
                    };
                    if params_shadow {
                        pending = Some(i);
                        if b.get(skip_ws(b, i + 1)) != Some(&b'{') {
                            push(&mut frames, &mut starts, i, frame(b'>', true, true));
                        }
                    }
                }
                _ => {}
            }
            out.push(c as char);
            i += 1;
        }
        out
    }
}

/// For an identifier at `i` named by `function`, whether that function is a declaration
/// (`Some(true)`, it binds the enclosing scope) or an expression (`Some(false)`, it binds
/// only inside itself).
fn function_named(b: &[u8], i: usize) -> Option<bool> {
    let mut at = prev_significant(b, i)?;
    if at.1 == b'*' {
        at = prev_significant(b, at.0)?;
    }
    let mut kw = at.0 + 1;
    while kw > 0 && is_ident_byte(b[kw - 1]) {
        kw -= 1;
    }
    if &b[kw..at.0 + 1] != b"function" {
        return None;
    }
    if prev_word(b, kw) == "async" {
        kw = prev_significant(b, kw).map_or(0, |(k, _)| k + 1 - "async".len());
    }
    Some(matches!(
        prev_significant(b, kw),
        None | Some((_, b';' | b'{' | b'}'))
    ))
}

/// Whether the `?` at `i` opens a conditional rather than being part of `?.` or `??`.
pub(super) fn is_conditional(b: &[u8], i: usize) -> bool {
    let optional_member =
        b.get(i + 1) == Some(&b'.') && !b.get(i + 2).is_some_and(u8::is_ascii_digit);
    !optional_member && b.get(i + 1) != Some(&b'?') && !(i > 0 && b[i - 1] == b'?')
}

fn json_property(prop: &str) -> Option<&'static str> {
    Some(match prop {
        "json" | "text" => "data",
        "code" => "status",
        "status" => "statusText",
        _ => return None,
    })
}

/// `res.json()` -> `res.data`, `res.code` -> `res.status`, `res.status` -> `res.statusText`
/// for the identifier `name` ending at `end`; returns the text and where scanning resumes.
fn response_member(src: &str, end: usize, name: &str) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let j = skip_ws(b, end);
    let (prop, member_end) = match b.get(j)? {
        b'.' if b.get(j + 1) != Some(&b'.') => {
            let k = skip_ws(b, j + 1);
            let e = ident_end(b, k);
            (&src[k..e], e)
        }
        b'[' => {
            let k = skip_ws(b, j + 1);
            let e = skip_opaque(src, k)
                .filter(|e| matches!(b.get(k), Some(b'\'' | b'"')) && *e > k + 1)?;
            let close = skip_ws(b, e);
            if b.get(close) != Some(&b']') {
                return None;
            }
            (&src[k + 1..e - 1], close + 1)
        }
        _ => return None,
    };
    let mapped = json_property(prop)?;
    let at = skip_ws(b, member_end);
    let next = if b.get(at) == Some(&b'(') {
        super::find_close(src, at)? + 1
    } else {
        member_end
    };
    Some((format!("{name}.{mapped}"), next))
}

// ---------------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------------

fn ends_with_word(out: &str, w: &str) -> bool {
    let t = out.trim_end();
    t.strip_suffix(w).is_some_and(|head| {
        !head
            .as_bytes()
            .last()
            .is_some_and(|c| is_ident_byte(*c) || *c == b'.')
    })
}

/// Whether an `await` placed here must be parenthesised: it sits inside a binary or
/// unary expression, or something is applied to it.
fn needs_parens(out: &str, src: &str, end: usize) -> bool {
    let b = src.as_bytes();
    let mut j = skip_ws(b, end);
    while j < b.len() && b[j] == b'/' && matches!(b.get(j + 1), Some(b'/' | b'*')) {
        j = skip_ws(b, skip_opaque(src, j).unwrap_or(b.len()));
    }
    let after = match b.get(j) {
        Some(
            b'.' | b'[' | b'(' | b'`' | b'?' | b'*' | b'/' | b'%' | b'+' | b'-' | b'<' | b'>'
            | b'=' | b'&' | b'|' | b'^',
        ) => true,
        Some(c) if is_ident_byte(*c) => {
            let w = &src[j..ident_end(b, j)];
            w == "in" || w == "instanceof"
        }
        _ => false,
    };
    let t = out.trim_end();
    let before = match t.as_bytes().last() {
        Some(b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'!' | b'~') => {
            !t.ends_with("=>")
        }
        Some(b'>') => !t.ends_with("=>"),
        Some(b'=') => {
            t.len() >= 2 && matches!(t.as_bytes()[t.len() - 2], b'=' | b'!' | b'<' | b'>')
        }
        Some(b'.') => t.ends_with("..."),
        _ => ["typeof", "void", "delete", "in", "instanceof"]
            .iter()
            .any(|w| ends_with_word(out, w)),
    };
    after || before
}

struct Link {
    open: usize,
    close: usize,
    method: &'static str,
    /// Start of the `.` of this link.
    dot: usize,
}

fn chain_links(src: &str, mut end: usize, cx: &mut Ctx) -> Vec<Link> {
    let b = src.as_bytes();
    let mut links = Vec::new();
    while links.len() < MAX_LINKS {
        let dot = skip_ws(b, end);
        if b.get(dot) != Some(&b'.') {
            break;
        }
        let k = skip_ws(b, dot + 1);
        let e = ident_end(b, k);
        let method = match &src[k..e] {
            "then" => "then",
            "catch" => "catch",
            "finally" => "finally",
            _ => break,
        };
        let open = skip_ws(b, e);
        if b.get(open) != Some(&b'(') {
            break;
        }
        let Some(close) = cx.find_close(src, open) else {
            break;
        };
        links.push(Link {
            open,
            close,
            method,
            dot,
        });
        end = close + 1;
    }
    links
}

fn is_nullish(s: &str) -> bool {
    s == "null" || s == "undefined"
}

/// Translate the text of a `.then` link, rewriting the response access of its handler
/// while `rewrite` allows. Returns the new text and whether rewriting may go on.
fn translate_link(src: &str, link: &Link, mut rewrite: bool, cx: &mut Ctx) -> (String, bool) {
    let args = parts(src, link.open + 1, link.close);
    let mut out = String::new();
    out.push_str(&src[link.dot..link.open + 1]);
    let mut at = link.open + 1;
    for (n, (s, e)) in args.iter().enumerate() {
        out.push_str(&src[at..*s]);
        at = *e;
        let text = &src[*s..*e];
        // A handler is translated as an async function so a `pm.sendRequest` inside it
        // gets its `await`; it stays async only if one was added, as in the reference.
        let handler = parse_fn(text, cx);
        let prefixed = handler.as_ref().is_some_and(|f| !f.is_async);
        let mut text = text.to_string();
        if n == 0 && rewrite && link.method == "then" && !is_nullish(&text) {
            match &handler {
                Some(f) if f.params.first().is_some_and(|p| is_plain_ident(p)) => {
                    let name = f.params[0];
                    let returns = returns_param(&text, f, name);
                    let reassigned = reassigns(&text[f.body_at..], name);
                    text = with_response(&text, f.body_at, name);
                    rewrite = returns && !reassigned;
                }
                _ => rewrite = false,
            }
        }
        let marked = if prefixed {
            format!("async {text}")
        } else {
            text
        };
        let own_fid = cx.next_fid + 1;
        let done = translate_code(&marked, cx);
        let needs_async = handler.is_some() && cx.awaited.contains(&own_fid);
        if needs_async && prefixed {
            match parse_fn(&done, cx) {
                Some(f) => out.push_str(&print_async(&done, &f, true)),
                None => out.push_str(&done),
            }
        } else {
            out.push_str(if prefixed {
                done.strip_prefix("async ").unwrap_or(&done)
            } else {
                &done
            });
        }
    }
    out.push_str(&src[at..link.close + 1]);
    (out, rewrite)
}

/// Rewrite the `pm.sendRequest(` call whose `(` is at `open`. `None` leaves it as written.
pub(super) fn rewrite(
    src: &str,
    start: usize,
    open: usize,
    out: &str,
    cx: &mut Ctx,
) -> Option<(String, usize)> {
    let close = cx.find_close(src, open)?;
    let args = parts(src, open + 1, close);
    let &(a0s, a0e) = args.first()?;
    if src[a0s..].starts_with("...") {
        return None;
    }
    let tab = cx.tab;
    let (ok, fid) = (cx.await_ok, cx.fid);

    let mut first = src[a0s..a0e].to_string();
    if is_object(src, a0s, a0e) {
        match transform_options(src, a0s, a0e, tab) {
            Ok(Some(t)) => first = t,
            Ok(None) => {}
            Err(Bail) => return None,
        }
    }
    let first = translate_code(&first, cx);

    let mut shown = vec![strip_tail(&first, line_indent(src, a0s))];
    if let Some(&(a1s, a1e)) = args.get(1) {
        let text = &src[a1s..a1e];
        let second = match parse_fn(text, cx) {
            Some(f) => {
                let name = f.params.get(1).filter(|p| is_plain_ident(p)).copied();
                let text = match name {
                    Some(n) => with_response(text, f.body_at, n),
                    None => text.to_string(),
                };
                translate_code(&print_async(&text, &f, false), cx)
            }
            // Not a function literal: kept as the caller wrote it.
            None => translate_code(text, cx),
        };
        shown.push(strip_tail(&second, line_indent(src, a1s)));
    }
    let printed_args = print_args(&shown, tab);
    let call = indent_tail(
        &format!("bru.sendRequest{printed_args}"),
        line_indent(src, start),
    );

    // The promise chain after the call, if any.
    let mut end = close + 1;
    let mut chain = String::new();
    let links = chain_links(src, close + 1, cx);
    let mut rewriting = true;
    for link in &links {
        chain.push_str(&src[end..link.dot]);
        let (text, again) = translate_link(src, link, rewriting, cx);
        rewriting = again && link.method != "catch";
        chain.push_str(&text);
        end = link.close + 1;
    }

    let add_await = ok && !ends_with_word(out, "await");
    if add_await {
        cx.awaited.insert(fid);
    }
    let wrap = add_await && needs_parens(out, src, end);
    let mut text = String::new();
    if wrap {
        text.push('(');
    }
    if add_await {
        text.push_str("await ");
    }
    text.push_str(&call);
    text.push_str(&chain);
    if wrap {
        text.push(')');
    }
    Some((text, end))
}

/// The argument list: on one line, or one argument per line when its first line is too long.
fn print_args(shown: &[String], tab: usize) -> String {
    let mut width = 0;
    for (i, a) in shown.iter().enumerate() {
        let first = a.split('\n').next().unwrap_or("");
        width += first.chars().count() + if i > 0 { 2 } else { 0 };
        if a.contains('\n') {
            break;
        }
    }
    if width > WRAP_COLUMN {
        let body: Vec<String> = shown.iter().map(|a| indent_all(a, tab)).collect();
        format!("(\n{}\n)", body.join(",\n"))
    } else {
        format!("({})", shown.join(", "))
    }
}

// ---------------------------------------------------------------------------------
// Request objects held in variables
// ---------------------------------------------------------------------------------

/// `const opts = {...}; pm.sendRequest(opts, cb)`: the reference rewrites the declaration
/// itself, following `const a = b` chains. Done up front on the whole script.
pub(super) fn hoist_declared_options(src: &str, cx: &mut Ctx) -> String {
    if !src.contains("sendRequest") {
        return src.to_string();
    }
    let b = src.as_bytes();
    let mut wanted: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(src, i) {
            i = end;
            continue;
        }
        if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if matches!(&src[i..end], "pm" | "postman")
                && !is_property_access(b, i)
                && src[end..].starts_with(".sendRequest")
            {
                let open = skip_ws(b, end + 12);
                if b.get(open) == Some(&b'(') {
                    let s = skip_ws(b, open + 1);
                    let e = ident_end(b, s);
                    let after = skip_ws(b, e);
                    if e > s
                        && is_plain_ident(&src[s..e])
                        && matches!(b.get(after), Some(b',' | b')'))
                        && !wanted.iter().any(|w| w == &src[s..e])
                        && wanted.len() < 64
                    {
                        wanted.push(src[s..e].to_string());
                    }
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    if wanted.is_empty() {
        return src.to_string();
    }

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut done = 0;
    // Each round resolves the declarations of the names found so far; a declaration that
    // is another variable adds that name for the next round.
    for _ in 0..8 {
        if done == wanted.len() {
            break;
        }
        let batch: Vec<String> = wanted[done..].to_vec();
        done = wanted.len();
        let mut i = 0;
        while i < b.len() {
            if let Some(end) = skip_opaque(src, i) {
                i = end;
                continue;
            }
            if !is_ident_byte(b[i]) {
                i += 1;
                continue;
            }
            let end = ident_end(b, i);
            if matches!(&src[i..end], "var" | "let" | "const") && !is_property_access(b, i) {
                let ns = skip_ws(b, end);
                let ne = ident_end(b, ns);
                let eq = skip_ws(b, ne);
                if ne > ns
                    && batch.iter().any(|w| w == &src[ns..ne])
                    && b.get(eq) == Some(&b'=')
                    && !matches!(b.get(eq + 1), Some(b'=' | b'>'))
                {
                    let is = skip_ws(b, eq + 1);
                    if b.get(is) == Some(&b'{') {
                        if let Some(c) = cx.find_close(src, is)
                            && let Ok(Some(t)) = transform_options(src, is, c + 1, cx.tab)
                        {
                            edits.push((is, c + 1, t));
                        }
                    } else if b.get(is).is_some_and(|c| is_ident_byte(*c)) {
                        let ie = ident_end(b, is);
                        let next = skip_ws(b, ie);
                        let name = &src[is..ie];
                        if is_plain_ident(name)
                            && matches!(b.get(next), None | Some(b';' | b',' | b'\n' | b')'))
                            && !wanted.iter().any(|w| w == name)
                            && wanted.len() < 64
                        {
                            wanted.push(name.to_string());
                        }
                    }
                }
            }
            i = end;
        }
    }
    if edits.is_empty() {
        return src.to_string();
    }
    splice(src, 0, src.len(), edits)
}
