//! Alias resolution: `const r = res; r.getBody()` becomes `res.getBody()` and the
//! declaration is dropped, so the chain translation sees the whole chain. Works on a token
//! list, in a few linear passes. A name that is declared more than once, reassigned, used as
//! a parameter or as an object key is left alone.

use std::collections::HashMap;

use super::scan::{
    MAX_DEPTH, find_close, ident_end, is_ident_byte, is_property_access, skip_opaque, template_end,
};

/// Resolve and remove at most this often, which settles chains of aliases
/// (`const a = req; const b = a;`).
const MAX_PASSES: usize = 5;

/// Chains a name may stand for.
const ALIASABLE: [&str; 7] = [
    "bru",
    "req",
    "res",
    "bru.cookies",
    "bru.runner",
    "test",
    "expect",
];

/// Script heads; a name equal to one is never resolved.
const HEADS: [&str; 5] = ["bru", "req", "res", "test", "expect"];

/// Heads a destructuring may bind properties of.
const DESTRUCTURABLE: [&str; 3] = ["bru", "req", "res"];

/// Scan budget shared by the bracket matching of one pass.
struct Ctx {
    budget: usize,
}

impl Ctx {
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

#[derive(Clone, Copy, PartialEq)]
enum K {
    Ident,
    Punct(u8),
    /// A string, regex or template text; opaque to the analysis.
    Value,
}

#[derive(Clone, Copy)]
struct Tok {
    k: K,
    s: usize,
    e: usize,
}

#[derive(Default)]
struct Lexed {
    toks: Vec<Tok>,
    /// Comment byte ranges, in source order.
    comments: Vec<(usize, usize)>,
}

/// Replaces each name bound to an aliasable chain with the chain and drops its declaration;
/// returns the text unchanged when there is none.
pub(super) fn resolve(src: &str) -> String {
    if !HEADS.iter().any(|h| src.contains(h)) {
        return src.to_owned();
    }
    let budget = src.len().saturating_mul(8).saturating_add(1 << 16);
    let mut code = src.to_owned();
    // Each pass gets the full scan budget.
    for _ in 0..MAX_PASSES {
        let mut cx = Ctx { budget };
        let Some(next) = pass(&code, &mut cx) else {
            break;
        };
        code = next;
    }
    code
}

fn lex(src: &str, from: usize, to: usize, depth: usize, cx: &mut Ctx, out: &mut Lexed) {
    let b = src.as_bytes();
    let mut i = from;
    while i < to {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'`' {
            i = lex_template(src, i, depth, cx, out);
        } else if let Some(end) = skip_opaque(src, i) {
            if c == b'/' && matches!(b.get(i + 1), Some(b'/' | b'*')) {
                out.comments.push((i, end));
            } else {
                out.toks.push(Tok {
                    k: K::Value,
                    s: i,
                    e: end,
                });
            }
            i = end;
        } else if is_ident_byte(c) {
            let e = ident_end(b, i);
            out.toks.push(Tok {
                k: K::Ident,
                s: i,
                e,
            });
            i = e;
        } else {
            out.toks.push(Tok {
                k: K::Punct(c),
                s: i,
                e: i + 1,
            });
            i += 1;
        }
    }
}

/// Lex a template literal: text pieces are opaque, each `${}` hole becomes a parenthesised
/// expression so uses of an alias inside it are seen.
fn lex_template(src: &str, i: usize, depth: usize, cx: &mut Ctx, out: &mut Lexed) -> usize {
    let b = src.as_bytes();
    let value = |out: &mut Lexed, s: usize, e: usize| out.toks.push(Tok { k: K::Value, s, e });
    if depth >= MAX_DEPTH {
        let end = template_end(src, i);
        value(out, i, end);
        return end;
    }
    let (mut j, mut seg) = (i + 1, i);
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'`' => {
                value(out, seg, j + 1);
                return j + 1;
            }
            b'$' if b.get(j + 1) == Some(&b'{') => {
                let Some(close) = cx.find_close(src, j + 1) else {
                    break;
                };
                value(out, seg, j + 2);
                let punct = |c: u8, at: usize| Tok {
                    k: K::Punct(c),
                    s: at,
                    e: at,
                };
                out.toks.push(punct(b'(', j + 2));
                lex(src, j + 2, close, depth + 1, cx, out);
                out.toks.push(punct(b')', close));
                j = close + 1;
                seg = j;
            }
            _ => j += 1,
        }
    }
    let end = b.len();
    out.toks.push(Tok {
        k: K::Value,
        s: seg.min(end),
        e: end,
    });
    end
}

/// For each bracket token the index of its partner, for matched pairs only.
fn match_brackets(toks: &[Tok]) -> Vec<Option<usize>> {
    let mut mate = vec![None; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.k {
            K::Punct(b'(' | b'[' | b'{') => stack.push(i),
            K::Punct(b')' | b']' | b'}') => {
                if let Some(o) = stack.pop() {
                    mate[o] = Some(i);
                    mate[i] = Some(o);
                }
            }
            _ => {}
        }
    }
    mate
}

#[derive(Default)]
struct Name {
    decls: u32,
    bad: bool,
}

/// How a resolved declaration leaves the source.
enum Removal {
    /// The whole statement, from the `const` token to the last token of the statement.
    Stmt(usize, usize),
    /// A byte range inside a statement that keeps other declarators.
    Range(usize, usize),
    Keep,
}

struct Cand<'a> {
    binds: Vec<(&'a str, &'static str)>,
    removal: Removal,
    /// A destructuring of a head leaves its statement unless every property resolved.
    complete: bool,
}

struct Scan<'a> {
    src: &'a str,
    toks: &'a [Tok],
    comments: &'a [(usize, usize)],
    mate: &'a [Option<usize>],
    names: HashMap<&'a str, Name>,
    cands: Vec<Cand<'a>>,
    /// Tokens that declare a name or sit in a declaring pattern; never replaced.
    skip: Vec<bool>,
    stack: Vec<u8>,
    exhausted: bool,
    /// Work left in this pass; hostile input exhausts it and the pass is dropped.
    work: usize,
}

fn pass(src: &str, cx: &mut Ctx) -> Option<String> {
    let mut lx = Lexed::default();
    lex(src, 0, src.len(), 0, cx, &mut lx);
    let mate = match_brackets(&lx.toks);
    let mut sc = Scan {
        src,
        toks: &lx.toks,
        comments: &lx.comments,
        mate: &mate,
        names: HashMap::new(),
        cands: Vec::new(),
        skip: vec![false; lx.toks.len()],
        stack: Vec::new(),
        exhausted: false,
        work: src.len().saturating_mul(4).saturating_add(1 << 16),
    };
    sc.run();
    if sc.exhausted {
        return None;
    }
    let edits = sc.edits();
    if edits.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for (s, e, text) in edits {
        if s < at {
            continue;
        }
        out.push_str(&src[at..s]);
        out.push_str(text);
        at = e;
    }
    out.push_str(&src[at..]);
    Some(out)
}

const CONTROL: [&str; 6] = ["if", "while", "for", "switch", "with", "catch"];
const COMPOUND: [&str; 15] = [
    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**=", "<<=", ">>=", ">>>=", "&&=", "||=",
    "??=",
];

impl<'a> Scan<'a> {
    fn text(&self, i: usize) -> &'a str {
        let t = self.toks[i];
        &self.src[t.s..t.e]
    }

    fn punct(&self, i: usize) -> Option<u8> {
        match self.toks.get(i)?.k {
            K::Punct(c) => Some(c),
            _ => None,
        }
    }

    fn is_ident(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| t.k == K::Ident)
    }

    /// Charge `n` tokens of work; once the budget is gone the pass gives up.
    fn charge(&mut self, n: usize) {
        self.work = self.work.saturating_sub(n);
        if self.work == 0 {
            self.exhausted = true;
        }
    }

    fn newline_between(&self, a: usize, b: usize) -> bool {
        self.src[self.toks[a].e..self.toks[b].s].contains('\n')
    }

    fn declare(&mut self, i: usize) {
        self.skip[i] = true;
        self.names.entry(self.text(i)).or_default().decls += 1;
    }

    fn mark_bad(&mut self, i: usize) {
        self.names.entry(self.text(i)).or_default().bad = true;
    }

    /// Mark every identifier inside the bracket at `open` as bad (parameters).
    fn bad_group(&mut self, open: usize) {
        let Some(close) = self.mate[open] else { return };
        self.charge(close - open);
        for k in open + 1..close {
            if self.is_ident(k) {
                self.mark_bad(k);
            }
        }
    }

    fn run(&mut self) {
        for i in 0..self.toks.len() {
            if self.exhausted {
                return;
            }
            match self.toks[i].k {
                K::Value => {}
                K::Punct(c) => self.on_punct(i, c),
                K::Ident => self.on_ident(i),
            }
        }
    }

    fn on_punct(&mut self, i: usize, c: u8) {
        match c {
            b'(' | b'[' | b'{' => self.stack.push(c),
            b')' | b']' | b'}' => {
                self.stack.pop();
                // `name(params) {` is a method or a function body.
                if c == b')'
                    && self.punct(i + 1) == Some(b'{')
                    && let Some(open) = self.mate[i]
                    && open > 0
                    && self.is_ident(open - 1)
                    && !CONTROL.contains(&self.text(open - 1))
                {
                    self.bad_group(open);
                }
            }
            b'=' if self.punct(i + 1) == Some(b'>') && self.toks[i + 1].s == self.toks[i].e => {
                // Arrow function: the parameters shadow whatever they are called.
                if i > 0 && self.is_ident(i - 1) {
                    self.mark_bad(i - 1);
                } else if i > 0
                    && self.punct(i - 1) == Some(b')')
                    && let Some(open) = self.mate[i - 1]
                {
                    self.bad_group(open);
                }
            }
            _ => {}
        }
    }

    fn on_ident(&mut self, i: usize) {
        let w = self.text(i);
        if is_property_access(self.src.as_bytes(), self.toks[i].s) {
            return;
        }
        match w {
            "var" | "let" | "const" => self.declaration(i),
            "function" => {
                let mut j = i + 1;
                if self.punct(j) == Some(b'*') {
                    j += 1;
                }
                if self.is_ident(j) {
                    self.declare(j);
                    j += 1;
                }
                if self.punct(j) == Some(b'(') {
                    self.bad_group(j);
                }
            }
            "class" if self.is_ident(i + 1) => self.declare(i + 1),
            "catch" if self.punct(i + 1) == Some(b'(') => self.bad_group(i + 1),
            _ => {}
        }
        if !self.skip[i] && (self.assigned(i) || self.object_key(i)) {
            self.mark_bad(i);
        }
    }

    /// True when the identifier at `i` is the target of `=`, `+=`, `++` and the like.
    fn assigned(&self, i: usize) -> bool {
        let mut ops = String::new();
        let mut j = i + 1;
        let mut at = self.toks[i].e;
        while ops.len() < 4
            && let Some(c) = self.punct(j)
            && (ops.is_empty() || self.toks[j].s == at)
        {
            ops.push(c as char);
            at = self.toks[j].e;
            j += 1;
        }
        let plain = ops.starts_with('=') && !ops.starts_with("==") && !ops.starts_with("=>");
        if plain
            || ops.starts_with("++")
            || ops.starts_with("--")
            || COMPOUND.iter().any(|o| ops.starts_with(o))
        {
            return true;
        }
        // Prefix `++r` / `--r`.
        i >= 2
            && matches!(self.punct(i - 1), Some(b'+' | b'-'))
            && self.punct(i - 2) == self.punct(i - 1)
            && self.toks[i - 1].s == self.toks[i - 2].e
            && self.toks[i - 1].e == self.toks[i].s
    }

    /// An object literal key or shorthand property: replacing it would be invalid.
    fn object_key(&self, i: usize) -> bool {
        self.stack.last() == Some(&b'{')
            && i > 0
            && matches!(self.punct(i - 1), Some(b'{' | b','))
            && matches!(self.punct(i + 1), Some(b'}' | b',' | b':'))
    }

    fn cont_prev(&self, i: usize) -> bool {
        matches!(self.toks[i].k, K::Punct(c) if !b")]}".contains(&c))
    }

    fn cont_next(&self, i: usize) -> bool {
        match self.toks[i].k {
            K::Punct(c) => b".([+-*/%<>=&|?:^,".contains(&c),
            K::Value => self.src.as_bytes()[self.toks[i].s] == b'`',
            K::Ident => false,
        }
    }

    /// Index of the token that ends the expression starting at `i`: a top-level `,` or `;`,
    /// a closing bracket, or the first token of the next statement under ASI.
    fn expr_end(&mut self, mut j: usize) -> usize {
        let len = self.toks.len();
        while j < len && !self.exhausted {
            self.charge(1);
            match self.punct(j) {
                Some(b',' | b';' | b')' | b']' | b'}') => return j,
                Some(b'(' | b'[' | b'{') => match self.mate[j] {
                    Some(close) => j = close + 1,
                    None => return len,
                },
                _ => j += 1,
            }
            if j < len
                && self.newline_between(j - 1, j)
                && !self.cont_prev(j - 1)
                && !self.cont_next(j)
            {
                return j;
            }
        }
        len
    }

    /// A script head followed by `.name` segments, as the chain and its last token; the chain
    /// only counts when it is an aliasable one.
    fn chain(&self, i: usize) -> Option<(&'static str, usize)> {
        let head = self.text_at(i)?;
        if !HEADS.contains(&head) {
            return None;
        }
        let mut key = String::from(head);
        let mut last = i;
        while self.punct(last + 1) == Some(b'.') && self.is_ident(last + 2) {
            key.push('.');
            key.push_str(self.text(last + 2));
            last += 2;
        }
        let known = ALIASABLE.iter().find(|k| **k == key)?;
        Some((known, last))
    }

    fn text_at(&self, i: usize) -> Option<&'a str> {
        self.is_ident(i).then(|| self.text(i))
    }

    /// Register one `var`/`let`/`const` statement: every name it declares, and any declarator
    /// that aliases a script chain.
    fn declaration(&mut self, kw: usize) {
        let usable_scope = matches!(self.stack.last(), None | Some(b'{'));
        let mut i = kw + 1;
        let mut first = true;
        // Last token of the previous declarator, for removing a later one with its comma.
        let mut prev_last = kw;
        loop {
            if self.exhausted || i >= self.toks.len() {
                return;
            }
            let target = i;
            let mut binds: Vec<(&'a str, &'static str)> = Vec::new();
            let mut total_props = 0;
            let mut flat = None;
            match self.toks[i].k {
                K::Ident => {
                    self.declare(i);
                    i += 1;
                }
                K::Punct(b'{' | b'[') => {
                    let Some(close) = self.mate[i] else { return };
                    self.charge(close - i);
                    let simple = self.toks[i].k == K::Punct(b'{') && self.pattern(i, close);
                    for k in i + 1..close {
                        if self.is_ident(k) && !self.skip[k] {
                            self.declare(k);
                        }
                    }
                    if simple {
                        flat = Some((i, close));
                    }
                    i = close + 1;
                }
                _ => return,
            }
            let pattern = matches!(self.toks[target].k, K::Punct(_));
            let mut alias: Option<(&'a str, &'static str)> = None;
            let mut chain_last = None;
            let init_end;
            if self.punct(i) == Some(b'=') && !matches!(self.punct(i + 1), Some(b'=' | b'>')) {
                let init = i + 1;
                init_end = self.expr_end(init);
                if pattern {
                    // `{ a, b: c } = bru` only when the initializer is exactly a head.
                    if let Some((open, close)) = flat
                        && init_end == init + 1
                        && let Some(head) = self.text_at(init)
                        && DESTRUCTURABLE.contains(&head)
                    {
                        total_props = self.pattern_binds(open, close, head, &mut binds);
                    }
                } else if let Some((value, last)) = self.chain(init)
                    && init_end == last + 1
                {
                    alias = Some((self.text(target), value));
                    chain_last = Some(last);
                }
            } else {
                init_end = i;
            }
            let more = self.punct(init_end) == Some(b',');
            let stmt_last = if self.punct(init_end) == Some(b';') {
                init_end
            } else {
                init_end.saturating_sub(1)
            };
            if usable_scope {
                if let (Some(bind), Some(last)) = (alias, chain_last) {
                    let removal = if first && !more {
                        Removal::Stmt(kw, stmt_last)
                    } else if first {
                        match self.toks.get(init_end + 1) {
                            Some(n) => Removal::Range(self.toks[target].s, n.s),
                            None => Removal::Keep,
                        }
                    } else {
                        Removal::Range(self.toks[prev_last].e, self.toks[last].e)
                    };
                    self.cands.push(Cand {
                        binds: vec![bind],
                        removal,
                        complete: true,
                    });
                } else if !binds.is_empty() {
                    let complete = binds.len() == total_props;
                    let removal = if first && !more && complete {
                        Removal::Stmt(kw, stmt_last)
                    } else {
                        Removal::Keep
                    };
                    self.cands.push(Cand {
                        binds,
                        removal,
                        complete,
                    });
                }
            }
            if !more {
                return;
            }
            prev_last = init_end - 1;
            i = init_end + 1;
            first = false;
        }
    }

    /// True when the `{ ... }` at `open..close` is a flat list of `name` or `key: name`.
    fn pattern(&self, open: usize, close: usize) -> bool {
        let mut k = open + 1;
        while k < close {
            if !self.is_ident(k) {
                return false;
            }
            k += 1;
            if self.punct(k) == Some(b':') {
                if !self.is_ident(k + 1) {
                    return false;
                }
                k += 2;
            }
            match self.punct(k) {
                Some(b',') => k += 1,
                _ if k == close => {}
                _ => return false,
            }
        }
        true
    }

    /// Bindings of a flat destructuring of `head`; returns the number of properties listed.
    fn pattern_binds(
        &mut self,
        open: usize,
        close: usize,
        head: &str,
        binds: &mut Vec<(&'a str, &'static str)>,
    ) -> usize {
        let (mut k, mut total) = (open + 1, 0);
        while k < close {
            let key = self.text(k);
            let local = if self.punct(k + 1) == Some(b':') {
                k += 2;
                self.text(k)
            } else {
                key
            };
            total += 1;
            let chain = format!("{head}.{key}");
            if let Some(known) = ALIASABLE.iter().find(|c| **c == chain) {
                binds.push((local, known));
            }
            k += 1;
            if self.punct(k) == Some(b',') {
                k += 1;
            }
        }
        total
    }

    fn usable(&self, name: &str) -> bool {
        self.names.get(name).is_some_and(|n| n.decls == 1 && !n.bad)
    }

    /// Byte span of a removed statement: leading own-line comments, the statement, a
    /// trailing same-line comment and the whitespace up to what follows.
    fn stmt_span(&self, kw: usize, last: usize) -> (usize, usize) {
        let b = self.src.as_bytes();
        let kw_s = self.toks[kw].s;
        let prev_end = if kw > 0 { self.toks[kw - 1].e } else { 0 };
        let from_idx = self.comments.partition_point(|c| c.0 < prev_end);
        let mut lead = None;
        for c in &self.comments[from_idx..] {
            if c.1 > kw_s {
                break;
            }
            let own_line = kw == 0 || prev_end == 0 || self.src[prev_end..c.0].contains('\n');
            if lead.is_none() && own_line {
                lead = Some(c.0);
            }
        }
        let mut from = lead.unwrap_or(kw_s);
        if lead.is_none()
            && kw > 0
            && matches!(b[prev_end.saturating_sub(1)], b';' | b'{' | b'}')
            && self.src[prev_end..kw_s]
                .bytes()
                .all(|c| c == b' ' || c == b'\t')
        {
            from = prev_end;
        }
        let mut to = self.toks[last].e;
        loop {
            while to < b.len() && matches!(b[to], b' ' | b'\t' | b'\r') {
                to += 1;
            }
            match self.comments.binary_search_by_key(&to, |c| c.0) {
                Ok(k) => to = self.comments[k].1,
                Err(_) => break,
            }
        }
        while to < b.len() && b[to].is_ascii_whitespace() {
            to += 1;
        }
        (from, to)
    }

    /// Removals and replacements as `(start, end, text)`, in source order.
    fn edits(&self) -> Vec<(usize, usize, &'static str)> {
        let mut out = Vec::new();
        let mut value: HashMap<&str, &'static str> = HashMap::new();
        for c in &self.cands {
            for (name, v) in &c.binds {
                if !HEADS.contains(name) && self.usable(name) {
                    value.insert(name, v);
                }
            }
            let all = c.binds.iter().all(|(n, _)| value.contains_key(n));
            match c.removal {
                Removal::Stmt(kw, last) if all && c.complete => {
                    let (s, e) = self.stmt_span(kw, last);
                    out.push((s, e, ""));
                }
                Removal::Range(s, e) if all => out.push((s, e, "")),
                _ => {}
            }
        }
        if value.is_empty() {
            return Vec::new();
        }
        for (i, t) in self.toks.iter().enumerate() {
            if t.k != K::Ident || self.skip[i] {
                continue;
            }
            if let Some(v) = value.get(&self.src[t.s..t.e])
                && !is_property_access(self.src.as_bytes(), t.s)
            {
                out.push((t.s, t.e, *v));
            }
        }
        out.sort_by_key(|e| e.0);
        out
    }
}
