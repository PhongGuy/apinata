//! Tracks, while the translator scans forward, whether an `await` is legal at the
//! current position (top level and `async` functions yes, plain functions no), and which
//! function the position belongs to.

use super::{is_ident_byte, prev_significant, skip_ws};

/// What stood before a `(`, which decides what a following `{` or `=>` opens.
#[derive(Clone, Copy, PartialEq)]
enum Pre {
    Other,
    Control,
    /// Parameters of a `function`; carries whether it is `async`.
    Func(bool),
    /// A call or a method head `name(...)`; carries whether it follows `async`.
    Method(bool),
    AsyncParen,
}

#[derive(Clone, Copy)]
struct Frame {
    open: u8,
    pre: Pre,
    ok: bool,
    /// The function this frame lies in.
    fid: u32,
}

pub(super) struct Scope {
    frames: Vec<Frame>,
    base: bool,
    base_fid: u32,
    /// End of the last closed `(...)` group and what preceded it.
    last_paren: Option<(usize, Pre)>,
}

const CONTROL: [&str; 6] = ["if", "for", "while", "switch", "with", "catch"];
const NOT_A_NAME: [&str; 14] = [
    "return",
    "typeof",
    "in",
    "of",
    "else",
    "do",
    "case",
    "yield",
    "await",
    "new",
    "delete",
    "void",
    "throw",
    "instanceof",
];

impl Scope {
    pub(super) fn new(base: bool, base_fid: u32) -> Self {
        Scope {
            frames: Vec::new(),
            base,
            base_fid,
            last_paren: None,
        }
    }

    /// Whether `await` is legal at the current position.
    pub(super) fn ok(&self) -> bool {
        self.frames.last().map_or(self.base, |f| f.ok)
    }

    /// Identifier of the innermost function around the current position.
    pub(super) fn fid(&self) -> u32 {
        self.frames.last().map_or(self.base_fid, |f| f.fid)
    }

    fn push(&mut self, open: u8, pre: Pre, ok: bool, fid: u32) {
        self.frames.push(Frame { open, pre, ok, fid });
    }

    /// An expression-bodied arrow ends at a comma, a semicolon or the closing bracket.
    fn pop_concise(&mut self) {
        while self.frames.last().is_some_and(|f| f.open == b'>') {
            self.frames.pop();
        }
    }

    /// Feed the punctuation byte at `i` of the source being scanned; `ids` hands out
    /// function identifiers.
    pub(super) fn feed(&mut self, b: &[u8], i: usize, ids: &mut u32) {
        let (ok, fid) = (self.ok(), self.fid());
        match b[i] {
            b'(' => {
                let pre = classify_paren(b, i);
                self.push(b'(', pre, ok, fid);
            }
            b'[' => self.push(b'[', Pre::Other, ok, fid),
            b'{' => match self.brace_fn(b, i) {
                Some(fn_ok) => {
                    *ids += 1;
                    self.push(b'{', Pre::Other, fn_ok, *ids);
                }
                None => self.push(b'{', Pre::Other, ok, fid),
            },
            b')' | b']' | b'}' => {
                self.pop_concise();
                if let Some(f) = self.frames.pop()
                    && b[i] == b')'
                {
                    self.last_paren = Some((i + 1, f.pre));
                }
            }
            b',' | b';' => self.pop_concise(),
            // An expression-bodied arrow; a block body is handled when its brace opens.
            b'>' if i > 0 && b[i - 1] == b'=' && b.get(skip_ws(b, i + 1)) != Some(&b'{') => {
                *ids += 1;
                let fn_ok = self.arrow_async(b, i - 1);
                self.push(b'>', Pre::Other, fn_ok, *ids);
            }
            _ => {}
        }
    }

    /// For a `{` opening a function body, whether `await` is legal inside it.
    fn brace_fn(&self, b: &[u8], i: usize) -> Option<bool> {
        match prev_significant(b, i) {
            Some((k, b'>')) if k > 0 && b[k - 1] == b'=' => Some(self.arrow_async(b, k - 1)),
            Some((k, b')')) => match self.last_paren {
                Some((end, Pre::Func(a) | Pre::Method(a))) if end == k + 1 => Some(a),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether the arrow whose `=` sits at `eq` is `async`.
    fn arrow_async(&self, b: &[u8], eq: usize) -> bool {
        match prev_significant(b, eq) {
            Some((k, b')')) => {
                matches!(self.last_paren, Some((end, Pre::AsyncParen)) if end == k + 1)
            }
            Some((k, c)) if is_ident_byte(c) => {
                let (start, _) = word_ending(b, k + 1);
                word_before(b, start) == "async"
            }
            _ => false,
        }
    }
}

/// The identifier ending just before `end`, with its start.
fn word_ending(b: &[u8], end: usize) -> (usize, &str) {
    let mut s = end;
    while s > 0 && is_ident_byte(b[s - 1]) {
        s -= 1;
    }
    (s, std::str::from_utf8(&b[s..end]).unwrap_or(""))
}

/// The identifier that ends before position `pos` (whitespace skipped), or "".
fn word_before(b: &[u8], pos: usize) -> &str {
    match prev_significant(b, pos) {
        Some((k, c)) if is_ident_byte(c) => word_ending(b, k + 1).1,
        _ => "",
    }
}

fn classify_paren(b: &[u8], i: usize) -> Pre {
    let Some((k, c)) = prev_significant(b, i) else {
        return Pre::Other;
    };
    if c == b'*' {
        // `function* (`
        return match prev_significant(b, k) {
            Some((p, d)) if is_ident_byte(d) && word_ending(b, p + 1).1 == "function" => {
                Pre::Func(word_before(b, word_ending(b, p + 1).0) == "async")
            }
            _ => Pre::Other,
        };
    }
    if !is_ident_byte(c) {
        return Pre::Other;
    }
    let (start, word) = word_ending(b, k + 1);
    match word {
        "function" => return Pre::Func(word_before(b, start) == "async"),
        "async" => return Pre::AsyncParen,
        w if CONTROL.contains(&w) => return Pre::Control,
        w if NOT_A_NAME.contains(&w) => return Pre::Other,
        _ => {}
    }
    // `function name(` and `function* name(` declare a function; any other name is a call
    // or a method head.
    match prev_significant(b, start) {
        Some((p, d)) if is_ident_byte(d) => {
            let (ps, w) = word_ending(b, p + 1);
            if w == "function" {
                return Pre::Func(word_before(b, ps) == "async");
            }
            Pre::Method(w == "async")
        }
        Some((p, b'*')) => match prev_significant(b, p) {
            Some((q, d)) if is_ident_byte(d) && word_ending(b, q + 1).1 == "function" => {
                Pre::Func(word_before(b, word_ending(b, q + 1).0) == "async")
            }
            _ => Pre::Method(false),
        },
        _ => Pre::Method(false),
    }
}
