//! A small JavaScript tokenizer, so translated scripts compare on code rather than layout.

use super::scan::REGEX_AFTER;

/// One JavaScript token: strings by decoded value, everything else as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tok {
    Word(String),
    Num(String),
    Str(String),
    Template(String),
    Regex(String),
    Punct(String),
}

/// Multi-character punctuators, longest first so the first match is the longest.
const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>",
];

/// The tokens of `src`, whitespace and comments dropped. Unterminated literals run to the end.
pub(super) fn tokens(src: &str) -> Vec<Tok> {
    let mut lexer = Lexer {
        s: src.chars().collect(),
        i: 0,
        out: Vec::new(),
        holes: Vec::new(),
    };
    lexer.run();
    lexer.out
}

/// Whether two scripts are the same code once layout and quoting are set aside.
pub(super) fn same(a: &str, b: &str) -> bool {
    tokens(a) == tokens(b)
}

/// Whether `needle`'s tokens appear as a contiguous run in `hay`'s. An empty needle never does.
pub(super) fn contains(hay: &str, needle: &str) -> bool {
    let needle = tokens(needle);
    !needle.is_empty()
        && tokens(hay)
            .windows(needle.len())
            .any(|w| w == needle.as_slice())
}

struct Lexer {
    s: Vec<char>,
    i: usize,
    out: Vec<Tok>,
    // One brace depth per open template hole: a `}` at depth 0 closes the hole.
    holes: Vec<usize>,
}

impl Lexer {
    fn at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn starts(&self, p: &str) -> bool {
        p.chars().enumerate().all(|(k, c)| self.at(k) == Some(c))
    }

    fn punct(&mut self, p: &str) {
        self.i += p.chars().count();
        self.out.push(Tok::Punct(p.to_owned()));
    }

    fn run(&mut self) {
        while let Some(c) = self.at(0) {
            if c.is_whitespace() {
                self.i += 1;
            } else if self.starts("//") {
                while self.at(0).is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
            } else if self.starts("/*") {
                self.i += 2;
                while self.at(0).is_some() && !self.starts("*/") {
                    self.i += 1;
                }
                self.i = (self.i + 2).min(self.s.len());
            } else if c == '\'' || c == '"' {
                self.string(c);
            } else if c.is_ascii_digit()
                || (c == '.' && self.at(1).is_some_and(|d| d.is_ascii_digit()))
            {
                self.number();
            } else if is_word_char(c) {
                let start = self.i;
                while self.at(0).is_some_and(is_word_char) {
                    self.i += 1;
                }
                self.out
                    .push(Tok::Word(self.s[start..self.i].iter().collect()));
            } else if c == '`' {
                self.punct("`");
                self.template_text();
            } else if c == '/' && self.regex_allowed() {
                self.regex();
            } else if c == '{' {
                if let Some(depth) = self.holes.last_mut() {
                    *depth += 1;
                }
                self.punct("{");
            } else if c == '}' {
                match self.holes.last_mut() {
                    Some(0) => {
                        self.holes.pop();
                        self.punct("}");
                        self.template_text();
                    }
                    Some(depth) => {
                        *depth -= 1;
                        self.punct("}");
                    }
                    None => self.punct("}"),
                }
            } else if let Some(p) = PUNCTS.iter().find(|p| self.starts(p)) {
                self.punct(p);
            } else {
                self.punct(&c.to_string());
            }
        }
    }

    fn regex_allowed(&self) -> bool {
        match self.out.last() {
            None => true,
            Some(Tok::Punct(p)) => !matches!(p.as_str(), ")" | "]" | "}"),
            Some(Tok::Word(w)) => REGEX_AFTER.contains(&w.as_str()),
            Some(_) => false,
        }
    }

    // Template text runs to the closing backtick or the next `${`, escapes kept raw.
    fn template_text(&mut self) {
        let start = self.i;
        while let Some(c) = self.at(0) {
            if c == '`' || self.starts("${") {
                break;
            }
            self.i += if c == '\\' { 2 } else { 1 };
        }
        self.i = self.i.min(self.s.len());
        if self.i > start {
            self.out
                .push(Tok::Template(self.s[start..self.i].iter().collect()));
        }
        if self.at(0) == Some('`') {
            self.punct("`");
        } else if self.at(0).is_some() {
            self.punct("${");
            self.holes.push(0);
        }
    }

    fn string(&mut self, quote: char) {
        self.i += 1;
        let mut value = String::new();
        while let Some(c) = self.at(0) {
            self.i += 1;
            if c == quote {
                break;
            }
            if c != '\\' {
                value.push(c);
                continue;
            }
            let Some(e) = self.at(0) else { break };
            self.i += 1;
            match e {
                'n' => value.push('\n'),
                't' => value.push('\t'),
                'r' => value.push('\r'),
                'b' => value.push('\u{8}'),
                'f' => value.push('\u{c}'),
                'v' => value.push('\u{b}'),
                '0' => value.push('\0'),
                'x' => self.hex_escape(&mut value, 2),
                'u' if self.at(0) == Some('{') => {
                    let close = (self.i..self.s.len()).find(|&k| self.s[k] == '}');
                    if let Some(close) = close {
                        let hex: String = self.s[self.i + 1..close].iter().collect();
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            value.push(ch);
                        }
                        self.i = close + 1;
                    }
                }
                'u' => self.hex_escape(&mut value, 4),
                '\r' => {
                    if self.at(0) == Some('\n') {
                        self.i += 1;
                    }
                }
                '\n' | '\u{2028}' | '\u{2029}' => {}
                other => value.push(other),
            }
        }
        self.out.push(Tok::Str(value));
    }

    fn hex_escape(&mut self, value: &mut String, len: usize) {
        let end = (self.i + len).min(self.s.len());
        let hex: String = self.s[self.i..end].iter().collect();
        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            value.push(ch);
        }
        self.i = end;
    }

    // Numbers keep their spelling: digits, one fraction, an exponent with its sign, suffixes.
    fn number(&mut self) {
        let start = self.i;
        let mut dot = false;
        while let Some(c) = self.at(0) {
            if c == '.' && !dot {
                dot = true;
            } else if matches!(c, '+' | '-') && matches!(self.s[self.i - 1], 'e' | 'E') {
                let hex = self.s[start..self.i].iter().any(|c| matches!(c, 'x' | 'X'));
                if hex {
                    break;
                }
            } else if !(c.is_ascii_alphanumeric() || c == '_') {
                break;
            }
            self.i += 1;
        }
        self.out
            .push(Tok::Num(self.s[start..self.i].iter().collect()));
    }

    // A regex body ends at an unescaped `/` outside a class, or at a line end; flags follow.
    fn regex(&mut self) {
        let start = self.i;
        self.i += 1;
        let mut class = false;
        while let Some(c) = self.at(0) {
            if c == '\n' {
                break;
            }
            self.i += 1;
            match c {
                '\\' => self.i = (self.i + 1).min(self.s.len()),
                '[' => class = true,
                ']' => class = false,
                '/' if !class => break,
                _ => {}
            }
        }
        while self.at(0).is_some_and(is_word_char) {
            self.i += 1;
        }
        self.out
            .push(Tok::Regex(self.s[start..self.i].iter().collect()));
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[cfg(test)]
mod tests {
    use super::{Tok, contains, same, tokens};

    fn w(s: &str) -> Tok {
        Tok::Word(s.into())
    }
    fn p(s: &str) -> Tok {
        Tok::Punct(s.into())
    }

    #[test]
    fn whitespace_and_comments_between_tokens_are_dropped() {
        assert!(same("a  =\n b; // c\n/* d */", "a=b;"));
        assert!(same("x(\n  1,\n  2\n)", "x(1, 2)"));
    }

    #[test]
    fn strings_compare_by_value() {
        assert!(same("x('a')", "x(\"a\")"));
        assert!(same(r#"'it\'s'"#, r#""it's""#));
        assert!(same(r#""A\x42\n""#, "'AB\\n'"));
        assert!(!same("'a b'", "'ab'"));
        assert_eq!(tokens(r#""a\"b""#), [Tok::Str("a\"b".into())]);
    }

    #[test]
    fn different_code_differs() {
        assert!(!same("a.b", "a.c"));
        assert!(!same("a = 1", "a == 1"));
        assert!(!same("a + b", "a + b;"));
    }

    #[test]
    fn punctuators_take_the_longest_match() {
        assert_eq!(tokens("a===b"), [w("a"), p("==="), w("b")]);
        assert_eq!(
            tokens("a?.b ?? c"),
            [w("a"), p("?."), w("b"), p("??"), w("c")]
        );
        assert_eq!(
            tokens("x => {...y}"),
            [w("x"), p("=>"), p("{"), p("..."), w("y"), p("}")]
        );
    }

    #[test]
    fn templates_keep_their_text_and_tokenize_their_holes() {
        let t = |s: &str| Tok::Template(s.into());
        assert_eq!(
            tokens("`a ${b + `c ${d}`} e`"),
            [
                p("`"),
                t("a "),
                p("${"),
                w("b"),
                p("+"),
                p("`"),
                t("c "),
                p("${"),
                w("d"),
                p("}"),
                p("`"),
                p("}"),
                t(" e"),
                p("`")
            ]
        );
        assert!(same("`a ${ b }`", "`a ${b}`"));
        assert!(!same("`a  ${b}`", "`a ${b}`"));
        assert_eq!(
            tokens("`${ {a: 1}.a }`"),
            [
                p("`"),
                p("${"),
                p("{"),
                w("a"),
                p(":"),
                Tok::Num("1".into()),
                p("}"),
                p("."),
                w("a"),
                p("}"),
                p("`")
            ]
        );
    }

    #[test]
    fn a_slash_is_a_regex_only_where_an_expression_starts() {
        assert_eq!(
            tokens("a / b / c"),
            [w("a"), p("/"), w("b"), p("/"), w("c")]
        );
        assert_eq!(
            tokens("x = /a b/g.test(s)"),
            [
                w("x"),
                p("="),
                Tok::Regex("/a b/g".into()),
                p("."),
                w("test"),
                p("("),
                w("s"),
                p(")")
            ]
        );
        assert_eq!(
            tokens("return /x/"),
            [w("return"), Tok::Regex("/x/".into())]
        );
        assert_eq!(
            tokens("(a) / 2"),
            [p("("), w("a"), p(")"), p("/"), Tok::Num("2".into())]
        );
    }

    #[test]
    fn numbers_keep_their_spelling() {
        assert_eq!(
            tokens("1.5e3 + 0x1F"),
            [Tok::Num("1.5e3".into()), p("+"), Tok::Num("0x1F".into())]
        );
    }

    #[test]
    fn unterminated_literals_end_the_input_without_panicking() {
        assert_eq!(tokens("'abc"), [Tok::Str("abc".into())]);
        assert_eq!(
            tokens("`ab ${x"),
            [p("`"), Tok::Template("ab ".into()), p("${"), w("x")]
        );
        assert!(tokens("/* open").is_empty());
    }

    #[test]
    fn contains_finds_a_run_of_tokens() {
        let hay = "if (pm.a(\"x\")) {\n  y();\n}";
        assert!(contains(hay, "pm.a('x')) {"));
        assert!(contains(hay, "y ( ) ;"));
        assert!(!contains(hay, "pm.a('y')"));
        assert!(!contains("ab", "a"));
        assert!(!contains("x(1)", ""));
    }
}
