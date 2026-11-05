//! Alias resolution: `const r = pm.response; r.json()` becomes `pm.response.json()` and the
//! declaration is dropped, as the reference's `preprocessAliases` does (usebruno/bruno
//! @1301ea17). Works on a token list, in a few linear passes. A name that is declared more
//! than once, reassigned, used as a parameter or as an object key is left alone, where the
//! reference would emit invalid JavaScript. The statement list a removed declaration leaves
//! is reprinted the way the reference's printer does.

use std::collections::HashMap;

use super::{
    Ctx, MAX_DEPTH, ident_end, is_ident_byte, is_property_access, skip_opaque, template_end,
};

/// The reference repeats resolve and remove at most this often, which also settles
/// chains of aliases (`const a = pm.environment; const b = a;`).
const MAX_PASSES: usize = 5;

/// Chains the reference resolves (`varInitsToReplace`, minus the bare `pm`, which it cannot
/// resolve either).
const ALIASABLE: [&str; 10] = [
    "pm.request",
    "pm.response",
    "pm.test",
    "pm.expect",
    "pm.environment",
    "pm.variables",
    "pm.collectionVariables",
    "pm.execution",
    "pm.globals",
    "pm.cookies",
];

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

/// Resolve every alias in `src`; returns the text unchanged when there is none.
pub(super) fn resolve(src: &str, cx: &mut Ctx) -> String {
    if !src.contains("pm") && !src.contains("postman") {
        return src.to_string();
    }
    // Each pass gets the full scan budget; the translation proper starts with its own.
    let budget = cx.budget;
    let mut code = src.to_string();
    for _ in 0..MAX_PASSES {
        cx.budget = budget;
        let Some(next) = pass(&code, cx) else { break };
        code = next;
    }
    cx.budget = budget;
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
    /// A destructuring of `pm` leaves its statement unless every property resolved.
    complete: bool,
}

struct Scan<'a, 'c> {
    src: &'a str,
    toks: &'a [Tok],
    comments: &'a [(usize, usize)],
    mate: &'a [Option<usize>],
    cx: &'c mut Ctx,
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
        cx,
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
    let removed = sc
        .cands
        .iter()
        .filter(|c| !matches!(c.removal, Removal::Keep))
        .count();
    sc.cx.replaced += removed;
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    // Where whole statements left, in the new text: their lists are reprinted.
    let mut removed_at = Vec::new();
    for (s, e, text, stmt) in edits {
        if s < at {
            continue;
        }
        out.push_str(&src[at..s]);
        if stmt {
            removed_at.push(out.len());
        }
        out.push_str(text);
        at = e;
    }
    out.push_str(&src[at..]);
    Some(reprint_lists(&out, &removed_at, sc.cx))
}

/// The reference reprints a statement list it removed a statement from: statements that
/// shared a line are joined, and empty statements are dropped. `removed_at` are offsets in
/// `src` where a statement left; only their lists change.
fn reprint_lists(src: &str, removed_at: &[usize], cx: &mut Ctx) -> String {
    if removed_at.is_empty() {
        return src.to_string();
    }
    let mut lx = Lexed::default();
    lex(src, 0, src.len(), 0, cx, &mut lx);
    let (toks, mate) = (&lx.toks, match_brackets(&lx.toks));
    // The bracket each token sits directly in; a closing bracket sits where its opener does.
    let mut parent = vec![None; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.k {
            K::Punct(b')' | b']' | b'}') if mate[i].is_some() => {
                stack.pop();
                parent[i] = mate[i].and_then(|o| parent[o]);
            }
            K::Punct(b'(' | b'[' | b'{') if mate[i].is_some() => {
                parent[i] = stack.last().copied();
                stack.push(i);
            }
            _ => parent[i] = stack.last().copied(),
        }
    }
    let mut lists: Vec<Option<usize>> = Vec::new();
    for &p in removed_at {
        let t = toks.partition_point(|t| t.s < p);
        let list = match toks.get(t) {
            None => None,
            Some(tok) if tok.k == K::Punct(b'}') && mate[t].is_some() => mate[t],
            Some(_) => parent[t],
        };
        if list.is_none_or(|o| toks[o].k == K::Punct(b'{')) && !lists.contains(&list) {
            lists.push(list);
        }
    }
    let rp = Reprint {
        src,
        toks,
        comments: &lx.comments,
        mate: &mate,
        parent: &parent,
    };
    let in_list = |i: usize| lists.contains(&parent[i]);
    let empty: Vec<bool> = (0..toks.len())
        .map(|i| in_list(i) && rp.empty_statement(i))
        .collect();
    // Empty statements go with the blanks before them; leading ones with the blanks after.
    let mut drops: Vec<Option<(usize, usize)>> = vec![None; toks.len()];
    let mut leading = true;
    for i in 0..toks.len() {
        if !empty[i] {
            leading = false;
            continue;
        }
        drops[i] = Some(match (i.checked_sub(1), leading) {
            (Some(p), false) => (toks[p].e, toks[i].e),
            _ => (toks[i].s, super::skip_ws(src.as_bytes(), toks[i].e)),
        });
    }
    let mut edits: Vec<(usize, usize)> = drops.iter().flatten().copied().collect();
    for i in 0..toks.len() {
        if empty[i] || !in_list(i) || !rp.statement_end(i) {
            continue;
        }
        let Some(next) = (i + 1..toks.len()).find(|n| !empty[*n]) else {
            continue;
        };
        if !rp.statement_start(next) {
            continue;
        }
        // The blanks left after this end and any empty statements dropped behind it.
        let from = drops[next - 1].map_or(toks[i].e, |d| d.1.max(toks[i].e));
        if let Some(to) = rp.joinable(from, toks[next].s) {
            edits.push((from, to));
        }
    }
    edits.sort_unstable();
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for (s, e) in edits {
        if s < at {
            continue;
        }
        out.push_str(&src[at..s]);
        at = e;
    }
    out.push_str(&src[at..]);
    out
}

struct Reprint<'a> {
    src: &'a str,
    toks: &'a [Tok],
    comments: &'a [(usize, usize)],
    mate: &'a [Option<usize>],
    parent: &'a [Option<usize>],
}

impl Reprint<'_> {
    fn ident(&self, i: usize) -> Option<&str> {
        let t = self.toks.get(i)?;
        (t.k == K::Ident).then(|| &self.src[t.s..t.e])
    }

    fn punct(&self, i: usize) -> Option<u8> {
        match self.toks.get(i)?.k {
            K::Punct(c) => Some(c),
            _ => None,
        }
    }

    /// A `;` with nothing before it in its statement.
    fn empty_statement(&self, i: usize) -> bool {
        if self.punct(i) != Some(b';') {
            return false;
        }
        match i.checked_sub(1) {
            None => true,
            Some(p) => {
                Some(p) == self.parent[i]
                    || (self.parent[p] == self.parent[i]
                        && (self.punct(p) == Some(b';') || self.block_close(p)))
            }
        }
    }

    fn statement_end(&self, i: usize) -> bool {
        self.punct(i) == Some(b';') || self.block_close(i)
    }

    fn statement_start(&self, i: usize) -> bool {
        match self.toks[i].k {
            K::Punct(c) => !b")]};,.".contains(&c),
            K::Ident => !matches!(self.ident(i), Some("else" | "catch" | "finally" | "while")),
            K::Value => true,
        }
    }

    /// A `}` that ends a statement: a block, a control body or a function declaration.
    fn block_close(&self, i: usize) -> bool {
        if self.punct(i) != Some(b'}') {
            return false;
        }
        let Some(open) = self.mate[i] else {
            return false;
        };
        let Some(before) = open.checked_sub(1) else {
            return true;
        };
        match self.toks[before].k {
            K::Punct(b';' | b'{' | b'}') => true,
            K::Ident => matches!(self.ident(before), Some("else" | "try" | "finally" | "do")),
            K::Punct(b')') => {
                let Some(paren) = self.mate[before] else {
                    return false;
                };
                let Some(head) = paren.checked_sub(1) else {
                    return false;
                };
                match self.ident(head) {
                    Some("if" | "for" | "while" | "with" | "switch" | "catch") => true,
                    Some(_) => self.declares_function(head),
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Whether the name at `name` is that of a function declaration (`function name(`
    /// starting a statement).
    fn declares_function(&self, name: usize) -> bool {
        let mut kw = name;
        if kw > 0 && self.punct(kw - 1) == Some(b'*') {
            kw -= 1;
        }
        let Some(kw) = kw
            .checked_sub(1)
            .filter(|k| self.ident(*k) == Some("function"))
        else {
            return false;
        };
        let start = match kw.checked_sub(1) {
            Some(a) if self.ident(a) == Some("async") => a,
            _ => kw,
        };
        start == 0 || matches!(self.punct(start - 1), Some(b';' | b'{' | b'}'))
    }

    /// The end of the blanks at `from` when dropping them joins two statements on one line:
    /// only spaces and tabs before the next statement, or before a block comment that the
    /// next statement follows on the same line.
    fn joinable(&self, from: usize, next: usize) -> Option<usize> {
        let b = self.src.as_bytes();
        let mut to = from;
        while to < next && matches!(b[to], b' ' | b'\t') {
            to += 1;
        }
        if to == from {
            return None;
        }
        if to == next {
            return Some(to);
        }
        let k = self.comments.binary_search_by_key(&to, |c| c.0).ok()?;
        let (s, e) = self.comments[k];
        let block = self.src[s..].starts_with("/*");
        (block && !self.src[s..next].contains('\n') && e <= next).then_some(to)
    }
}

const CONTROL: [&str; 6] = ["if", "while", "for", "switch", "with", "catch"];
const COMPOUND: [&str; 15] = [
    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**=", "<<=", ">>=", ">>>=", "&&=", "||=",
    "??=",
];

impl<'a> Scan<'a, '_> {
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

    /// `pm` or `postman` followed by `.name` segments, as the normalised chain and its last
    /// token; the chain only counts when it is a known resolvable one.
    fn chain(&self, i: usize) -> Option<(&'static str, usize)> {
        if !matches!(self.text_at(i)?, "pm" | "postman") {
            return None;
        }
        let mut key = String::from("pm");
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
    /// that aliases a `pm` chain.
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
                        total_props = self.pattern_binds(i, close, &mut binds);
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
                    // `{ a, b: c } = pm` only when the initializer is exactly `pm`.
                    let is_pm = init_end == init + 1
                        && matches!(self.text_at(init), Some("pm" | "postman"));
                    if !is_pm {
                        binds.clear();
                    }
                } else if let Some((value, last)) = self.chain(init)
                    && init_end == last + 1
                {
                    alias = Some((self.text(target), value));
                    chain_last = Some(last);
                }
            } else {
                init_end = i;
                binds.clear();
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

    /// Bindings of a flat destructuring of `pm`; returns the number of properties listed.
    fn pattern_binds(
        &mut self,
        open: usize,
        close: usize,
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
            let chain = format!("pm.{key}");
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
    /// Each edit says whether it removes a whole statement.
    fn edits(&self) -> Vec<(usize, usize, &'static str, bool)> {
        let mut out = Vec::new();
        let mut value: HashMap<&str, &'static str> = HashMap::new();
        for c in &self.cands {
            for (name, v) in &c.binds {
                if !matches!(*name, "pm" | "postman") && self.usable(name) {
                    value.insert(name, v);
                }
            }
            let all = c.binds.iter().all(|(n, _)| value.contains_key(n));
            match c.removal {
                Removal::Stmt(kw, last) if all && c.complete => {
                    let (s, e) = self.stmt_span(kw, last);
                    out.push((s, e, "", true));
                }
                Removal::Range(s, e) if all => out.push((s, e, "", false)),
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
                out.push((t.s, t.e, *v, false));
            }
        }
        out.sort_by_key(|e| e.0);
        out
    }
}
