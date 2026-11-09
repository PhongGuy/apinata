//! Alias analysis. A name bound to a piece of the script API (`const r = res`) keeps its
//! declaration with the value translated (`const r = pm.response`), and member chains on it
//! are translated where they are used. An alias is safe when its name is declared once and
//! never rebound or shadowed; any other is left as written and reported. The analysis also
//! records which names the script declares, so its own `test` or a shadowed jar is kept.

use std::collections::{HashMap, HashSet};

use super::scan::{
    MAX_DEPTH, find_close, ident_end, is_ident_byte, is_property_access, skip_opaque, template_end,
};

/// Chains a name may stand for, with the Postman object each one is.
const ALIASABLE: [(&str, &str); 7] = [
    ("bru", "pm"),
    ("req", "pm.request"),
    ("res", "pm.response"),
    ("bru.cookies", "pm.cookies"),
    ("bru.runner", "pm.execution"),
    ("test", "pm.test"),
    ("expect", "pm.expect"),
];

/// Script heads; a name equal to one is never an alias.
const HEADS: [&str; 5] = ["bru", "req", "res", "test", "expect"];

/// Heads a destructuring may bind properties of.
const DESTRUCTURABLE: [&str; 3] = ["bru", "req", "res"];

/// The Postman object an aliasable chain is.
pub(super) fn postman_object(chain: &str) -> Option<&'static str> {
    ALIASABLE
        .iter()
        .find(|(c, _)| *c == chain)
        .map(|(_, pm)| *pm)
}

fn aliasable(key: &str) -> Option<&'static str> {
    ALIASABLE.iter().find(|(c, _)| *c == key).map(|(c, _)| *c)
}

/// Scan budget for the bracket matching of template holes.
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

#[derive(Default, Clone, Copy)]
struct Name {
    decls: u32,
    /// Reassigned, destructured into, a loop target or a parameter.
    bad: bool,
    param: bool,
}

/// What the analysis found about the names of one script.
#[derive(Default)]
pub(super) struct Bindings {
    aliases: HashMap<String, &'static str>,
    /// Chains of aliases left as written, first seen first.
    pub unsafe_chains: Vec<&'static str>,
    names: HashMap<String, Name>,
    /// The scan ran out of budget: nothing is known to be safe.
    exhausted: bool,
}

impl Bindings {
    /// The chain a safe alias stands for.
    pub fn alias(&self, name: &str) -> Option<&'static str> {
        self.aliases.get(name).copied()
    }

    /// Safe aliases with the chains they stand for.
    pub fn aliases(&self) -> impl Iterator<Item = (&str, &'static str)> {
        self.aliases.iter().map(|(n, c)| (n.as_str(), *c))
    }

    /// Declared exactly once and never rebound, so every use is the declared value.
    pub fn usable(&self, name: &str) -> bool {
        !self.exhausted && self.names.get(name).is_some_and(|n| n.decls == 1 && !n.bad)
    }

    /// The script declares `name` or takes it as a parameter.
    pub fn binds(&self, name: &str) -> bool {
        self.names.get(name).is_some_and(|n| n.decls > 0 || n.param)
    }
}

/// Analyse `src`; returns it with the value of each safe alias declaration translated, the
/// number of values translated, and the bindings.
pub(super) fn resolve(src: &str) -> (String, usize, Bindings) {
    if !HEADS.iter().any(|h| src.contains(h)) && !src.contains("jar") {
        return (src.to_owned(), 0, Bindings::default());
    }
    let mut cx = Ctx {
        budget: src.len().saturating_mul(8).saturating_add(1 << 16),
    };
    let mut lx = Lexed::default();
    lex(src, 0, src.len(), 0, &mut cx, &mut lx);
    let mate = match_brackets(&lx.toks);
    let mut sc = Scan {
        src,
        toks: &lx.toks,
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
        let bindings = Bindings {
            exhausted: true,
            ..Bindings::default()
        };
        return (src.to_owned(), 0, bindings);
    }
    sc.settle()
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

/// Where an alias's value comes from.
#[derive(Clone, Copy)]
enum Source<'a> {
    Chain(&'static str),
    /// Another name, itself possibly an alias.
    Name(&'a str),
}

struct Cand<'a> {
    binds: Vec<(&'a str, Source<'a>)>,
    /// Byte span and chain of a value written as a script chain, translated when safe.
    value: Option<(usize, usize, &'static str)>,
    /// In statement position and, for a destructuring, every key aliasable.
    ok: bool,
}

struct Scan<'a> {
    src: &'a str,
    toks: &'a [Tok],
    mate: &'a [Option<usize>],
    names: HashMap<&'a str, Name>,
    cands: Vec<Cand<'a>>,
    /// Tokens that declare a name or open a declaring pattern.
    skip: Vec<bool>,
    stack: Vec<u8>,
    exhausted: bool,
    /// Work left; hostile input exhausts it and the analysis is dropped.
    work: usize,
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

    fn text_at(&self, i: usize) -> Option<&'a str> {
        self.is_ident(i).then(|| self.text(i))
    }

    /// Charge `n` tokens of work; once the budget is gone the analysis gives up.
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

    fn mark_bad(&mut self, i: usize, param: bool) {
        let name = self.names.entry(self.text(i)).or_default();
        name.bad = true;
        name.param |= param;
    }

    /// Mark every identifier inside the bracket at `open` as rebound.
    fn bad_group(&mut self, open: usize, param: bool) {
        let Some(close) = self.mate[open] else { return };
        self.charge(close - open);
        for k in open + 1..close {
            if self.is_ident(k) {
                self.mark_bad(k, param);
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

    /// Whether the token at `i` starts a `for (` or `for await (` head's first slot.
    fn loop_target(&self, first: usize) -> bool {
        first >= 2
            && self.punct(first - 1) == Some(b'(')
            && (self.text_at(first - 2) == Some("for")
                || (self.text_at(first - 2) == Some("await")
                    && first >= 3
                    && self.text_at(first - 3) == Some("for")))
    }

    fn on_punct(&mut self, i: usize, c: u8) {
        match c {
            b'(' | b'[' | b'{' => self.stack.push(c),
            b')' | b']' | b'}' => {
                self.stack.pop();
                let Some(open) = self.mate[i] else { return };
                // `name(params) {` is a method or a function body.
                if c == b')'
                    && self.punct(i + 1) == Some(b'{')
                    && open > 0
                    && self.is_ident(open - 1)
                    && !CONTROL.contains(&self.text(open - 1))
                {
                    self.bad_group(open, true);
                }
                // A pattern assigned to, or the target of a `for (... of|in`.
                let pattern = c == b'}' || (c == b']' && self.array_start(open));
                let assigned = self.punct(i + 1) == Some(b'=')
                    && !matches!(self.punct(i + 2), Some(b'=' | b'>'));
                let iterated =
                    matches!(self.text_at(i + 1), Some("of" | "in")) && self.loop_target(open);
                if pattern && !self.skip[open] && (assigned || iterated) {
                    self.bad_group(open, false);
                }
            }
            b'=' if self.punct(i + 1) == Some(b'>') && self.toks[i + 1].s == self.toks[i].e => {
                // Arrow function: the parameters shadow whatever they are called.
                if i > 0 && self.is_ident(i - 1) {
                    self.mark_bad(i - 1, true);
                } else if i > 0
                    && self.punct(i - 1) == Some(b')')
                    && let Some(open) = self.mate[i - 1]
                {
                    self.bad_group(open, true);
                }
            }
            _ => {}
        }
    }

    /// A `[` that starts an array rather than indexing what precedes it.
    fn array_start(&self, open: usize) -> bool {
        match open.checked_sub(1).map(|p| self.toks[p].k) {
            None => true,
            Some(K::Ident) => matches!(
                self.text(open - 1),
                "return"
                    | "typeof"
                    | "case"
                    | "do"
                    | "else"
                    | "in"
                    | "of"
                    | "new"
                    | "void"
                    | "await"
                    | "yield"
                    | "let"
                    | "const"
                    | "var"
            ),
            Some(K::Value) => false,
            Some(K::Punct(p)) => !matches!(p, b')' | b']'),
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
                    self.bad_group(j, true);
                }
            }
            "class" if self.is_ident(i + 1) => self.declare(i + 1),
            "catch" if self.punct(i + 1) == Some(b'(') => self.bad_group(i + 1, true),
            _ => {}
        }
        if self.skip[i] {
            return;
        }
        let iterated = matches!(self.text_at(i + 1), Some("of" | "in")) && self.loop_target(i);
        if self.assigned(i) || iterated {
            self.mark_bad(i, false);
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

    /// A script head followed by `.name` segments that together form an aliasable chain, with
    /// its last token.
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
        Some((aliasable(&key)?, last))
    }

    /// A declaration keyword where a statement starts: not the bare body of an `if`, a loop,
    /// `else` or `do`, where a `var` would be hoisted out of its condition.
    fn statement_position(&self, kw: usize) -> bool {
        let Some(p) = kw.checked_sub(1) else {
            return true;
        };
        match self.toks[p].k {
            K::Ident => !matches!(self.text(p), "else" | "do"),
            K::Punct(b')') => !self.mate[p].is_some_and(|o| {
                o > 0 && matches!(self.text_at(o - 1), Some("if" | "for" | "while" | "with"))
            }),
            _ => true,
        }
    }

    /// Register one `var`/`let`/`const` statement: every name it declares, and any declarator
    /// whose value is a script chain, a lone head destructured, or another name.
    fn declaration(&mut self, kw: usize) {
        let ok_scope =
            matches!(self.stack.last(), None | Some(b'{')) && self.statement_position(kw);
        let mut i = kw + 1;
        loop {
            if self.exhausted || i >= self.toks.len() {
                return;
            }
            let target = i;
            let mut flat = None;
            match self.toks[i].k {
                K::Ident => {
                    self.declare(i);
                    i += 1;
                }
                K::Punct(b'{' | b'[') => {
                    let Some(close) = self.mate[i] else { return };
                    self.charge(close - i);
                    self.skip[i] = true;
                    if self.toks[i].k == K::Punct(b'{') && self.pattern(i, close) {
                        flat = Some((i, close));
                    }
                    for k in i + 1..close {
                        if self.is_ident(k) && !self.skip[k] {
                            self.declare(k);
                        }
                    }
                    i = close + 1;
                }
                _ => return,
            }
            let init_end;
            if self.punct(i) == Some(b'=') && !matches!(self.punct(i + 1), Some(b'=' | b'>')) {
                let init = i + 1;
                init_end = self.expr_end(init);
                let whole = |last: usize| init_end == last + 1;
                if self.toks[target].k != K::Ident {
                    if let Some((open, close)) = flat
                        && whole(init)
                        && let Some(head) = self.text_at(init)
                        && DESTRUCTURABLE.contains(&head)
                    {
                        let (binds, all) = self.pattern_binds(open, close, head);
                        let chain = aliasable(head).unwrap_or("bru");
                        let t = self.toks[init];
                        self.cands.push(Cand {
                            binds,
                            value: Some((t.s, t.e, chain)),
                            ok: ok_scope && all,
                        });
                    }
                } else if let Some((chain, last)) = self.chain(init)
                    && whole(last)
                {
                    let (s, e) = (self.toks[init].s, self.toks[last].e);
                    self.cands.push(Cand {
                        binds: vec![(self.text(target), Source::Chain(chain))],
                        value: Some((s, e, chain)),
                        ok: ok_scope,
                    });
                } else if let Some(other) = self.text_at(init)
                    && whole(init)
                    && !HEADS.contains(&other)
                {
                    self.cands.push(Cand {
                        binds: vec![(self.text(target), Source::Name(other))],
                        value: None,
                        ok: ok_scope,
                    });
                }
            } else {
                init_end = i;
            }
            if self.punct(init_end) != Some(b',') {
                return;
            }
            i = init_end + 1;
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

    /// Bindings of a flat destructuring of `head`, and whether every key was aliasable.
    fn pattern_binds(
        &self,
        open: usize,
        close: usize,
        head: &str,
    ) -> (Vec<(&'a str, Source<'a>)>, bool) {
        let (mut k, mut binds, mut all) = (open + 1, Vec::new(), true);
        while k < close {
            let key = self.text(k);
            let local = if self.punct(k + 1) == Some(b':') {
                k += 2;
                self.text(k)
            } else {
                key
            };
            match aliasable(&format!("{head}.{key}")) {
                Some(chain) => binds.push((local, Source::Chain(chain))),
                None => all = false,
            }
            k += 1;
            if self.punct(k) == Some(b',') {
                k += 1;
            }
        }
        (binds, all)
    }

    fn usable(&self, name: &str) -> bool {
        !HEADS.contains(&name) && self.names.get(name).is_some_and(|n| n.decls == 1 && !n.bad)
    }

    // Decide every candidate. A name's chain and safety follow `NAME = OTHER` links, memoised
    // so long chains of names stay linear; a cycle resolves to nothing.
    fn settle(self) -> (String, usize, Bindings) {
        // A declaration is safe when it is in place and every name it binds is usable.
        let cand_safe: Vec<bool> = self
            .cands
            .iter()
            .map(|c| c.ok && c.binds.iter().all(|(n, _)| self.usable(n)))
            .collect();
        let mut by_name: HashMap<&str, (Source, bool)> = HashMap::new();
        for (c, safe) in self.cands.iter().zip(&cand_safe) {
            for (local, source) in &c.binds {
                by_name.entry(local).or_insert((*source, *safe));
            }
        }
        let mut memo: HashMap<&str, (Option<&'static str>, bool)> = HashMap::new();
        let mut resolve_name = |start: &'a str| -> (Option<&'static str>, bool) {
            let mut path = Vec::new();
            let mut seen = HashSet::new();
            let mut at = start;
            let mut result = (None, false);
            loop {
                if let Some(r) = memo.get(at) {
                    result = *r;
                    break;
                }
                if !seen.insert(at) {
                    break;
                }
                path.push(at);
                match by_name.get(at) {
                    Some((Source::Chain(chain), _)) => {
                        result = (Some(*chain), true);
                        break;
                    }
                    Some((Source::Name(other), _)) => at = other,
                    None => break,
                }
            }
            // Walk back: each link is safe when its own declaration is and the next one is.
            for name in path.into_iter().rev() {
                let own = by_name.get(name).is_some_and(|(_, safe)| *safe);
                result = (result.0, result.1 && own);
                memo.insert(name, result);
            }
            result
        };
        let mut bindings = Bindings::default();
        let mut edits = Vec::new();
        for (c, safe) in self.cands.iter().zip(&cand_safe) {
            let mut all_safe = *safe;
            for (local, _) in &c.binds {
                let (chain, safe) = resolve_name(local);
                match chain {
                    Some(chain) if safe => {
                        bindings.aliases.insert((*local).to_owned(), chain);
                    }
                    Some(chain) => {
                        all_safe = false;
                        if !bindings.unsafe_chains.contains(&chain) {
                            bindings.unsafe_chains.push(chain);
                        }
                    }
                    None => all_safe = false,
                }
            }
            if let Some((s, e, chain)) = c.value {
                if all_safe {
                    edits.push((s, e, postman_object(chain).unwrap_or("pm")));
                } else {
                    if !bindings.unsafe_chains.contains(&chain) {
                        bindings.unsafe_chains.push(chain);
                    }
                }
            }
        }
        bindings.names = self
            .names
            .iter()
            .map(|(n, v)| ((*n).to_owned(), *v))
            .collect();
        edits.sort_unstable_by_key(|e| e.0);
        let mut out = String::with_capacity(self.src.len());
        let mut at = 0;
        for (s, e, text) in &edits {
            out.push_str(&self.src[at..*s]);
            out.push_str(text);
            at = *e;
        }
        out.push_str(&self.src[at..]);
        (out, edits.len(), bindings)
    }
}
