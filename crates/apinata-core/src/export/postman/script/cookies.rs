//! Renames cookie-jar methods to Postman's where the receiver is a jar.

use std::collections::{HashMap, HashSet};

use super::scan::{ident_end, is_ident_byte, is_property_access, skip_opaque, skip_ws};

pub(super) const JAR_METHODS: &[(&str, &str)] = &[
    ("getCookie", "get"),
    ("getCookies", "getAll"),
    ("setCookie", "set"),
    ("deleteCookie", "unset"),
    ("deleteCookies", "clear"),
];

/// Renames jar methods to Postman's where the receiver is a jar; returns the code and the renames.
pub(super) fn rename_jar_methods(code: &str) -> (String, usize) {
    if !JAR_METHODS.iter().any(|(m, _)| code.contains(m)) {
        return (code.to_owned(), 0);
    }
    let words = words(code);
    let closes = closing_parens(code);
    let jars = jar_names(code, &words, &closes);
    let b = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut at = 0;
    let mut renamed = 0;
    for &(s, e) in &words {
        // The receiver ends after a jar call's `)` or after a name bound to one.
        let receiver_end = match jar_call(code, s, &closes) {
            Some(end) => end,
            None if jars.contains(&code[s..e]) => e,
            None => continue,
        };
        let Some((m, to)) = called_method(b, receiver_end) else {
            continue;
        };
        out.push_str(&code[at..m]);
        out.push_str(to);
        at = ident_end(b, m);
        renamed += 1;
    }
    out.push_str(&code[at..]);
    (out, renamed)
}

/// Free-standing identifiers outside literals and comments, as byte ranges in order.
fn words(code: &str) -> Vec<(usize, usize)> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(code, i) {
            i = end;
        } else if is_ident_byte(b[i]) {
            let end = ident_end(b, i);
            if !is_property_access(b, i) && !b[i].is_ascii_digit() {
                out.push((i, end));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Each `(` outside literals mapped to its `)`, in one pass so many jar calls stay linear.
fn closing_parens(code: &str) -> HashMap<usize, usize> {
    let b = code.as_bytes();
    let (mut out, mut open) = (HashMap::new(), Vec::new());
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_opaque(code, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' => open.push(i),
            b')' => {
                if let Some(o) = open.pop() {
                    out.insert(o, i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The end of a `bru.cookies.jar(...)` or `pm.cookies.jar(...)` call starting at `s`.
fn jar_call(code: &str, s: usize, closes: &HashMap<usize, usize>) -> Option<usize> {
    let rest = &code[s..];
    let len = ["bru.cookies.jar", "pm.cookies.jar"]
        .iter()
        .find(|h| rest.starts_with(**h))?
        .len();
    let b = code.as_bytes();
    if b.get(s + len).is_some_and(|c| is_ident_byte(*c)) {
        return None;
    }
    let open = skip_ws(b, s + len);
    (b.get(open) == Some(&b'(')).then_some(())?;
    closes.get(&open).map(|c| c + 1)
}

// A name declared with exactly a jar call as its value is a jar wherever it is used; scopes
// are not tracked.
fn jar_names<'a>(
    code: &'a str,
    words: &[(usize, usize)],
    closes: &HashMap<usize, usize>,
) -> HashSet<&'a str> {
    let b = code.as_bytes();
    let mut out = HashSet::new();
    for pair in words.windows(2) {
        let [(ks, ke), (ns, ne)] = [pair[0], pair[1]];
        if !matches!(&code[ks..ke], "const" | "let" | "var") || skip_ws(b, ke) != ns {
            continue;
        }
        let eq = skip_ws(b, ne);
        if b.get(eq) != Some(&b'=') || matches!(b.get(eq + 1), Some(b'=' | b'>')) {
            continue;
        }
        let Some(end) = jar_call(code, skip_ws(b, eq + 1), closes) else {
            continue;
        };
        let after = skip_ws(b, end);
        if !b
            .get(after)
            .is_some_and(|c| b".?([`+-*/%&|^<>=".contains(c))
        {
            out.insert(&code[ns..ne]);
        }
    }
    out
}

/// A `.method(` after `receiver_end` naming a jar method: the method's start and new name.
fn called_method(b: &[u8], receiver_end: usize) -> Option<(usize, &'static str)> {
    let dot = skip_ws(b, receiver_end);
    (b.get(dot) == Some(&b'.')).then_some(())?;
    let m = skip_ws(b, dot + 1);
    let end = ident_end(b, m);
    let name = std::str::from_utf8(&b[m..end]).ok()?;
    let (_, to) = JAR_METHODS.iter().find(|(from, _)| *from == name)?;
    (b.get(skip_ws(b, end)) == Some(&b'(')).then_some((m, *to))
}
