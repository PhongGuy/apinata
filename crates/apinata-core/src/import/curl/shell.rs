//! Splitting a command line into words the way a POSIX shell would, without expanding or
//! running anything.

use super::{CurlError, CurlSyntax};

/// One shell word after quote removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    pub text: String,
    /// Byte offset in the input where the word starts.
    pub offset: usize,
}

/// The shell operator that ended the command, and everything after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tail {
    pub offset: usize,
    pub operator: String,
}

/// The words of the first command, and the operator that ended it if any.
#[derive(Debug)]
pub(crate) struct Split {
    pub words: Vec<Word>,
    pub tail: Option<Tail>,
}

/// Splits `command` into the words of its first command. Quotes, backslashes and `$'...'` are
/// removed as a POSIX shell (with bash's ANSI-C quoting) would; `$` expansions and backticks
/// stay literal. Fails on an unclosed quote or on Windows cmd or PowerShell line syntax.
pub(crate) fn split(command: &str) -> Result<Split, CurlError> {
    let scan = Scanner {
        command,
        chars: command.char_indices().collect(),
    };
    let mut words = Vec::new();
    let mut text = String::new();
    // The byte where the current word started; `None` between words, so `''` still makes one.
    let mut start: Option<usize> = None;
    let mut i = 0;
    while let Some(&(offset, c)) = scan.chars.get(i) {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                if let Some(offset) = start.take() {
                    words.push(Word {
                        text: std::mem::take(&mut text),
                        offset,
                    });
                }
                i += 1;
            }
            '#' if start.is_none() => {
                while scan.at(i).is_some_and(|c| c != '\n') {
                    i += 1;
                }
            }
            // A backslash escapes the next character, and before a line end it joins two lines.
            '\\' => match scan.at(i + 1) {
                None => i += 1,
                Some(_) if scan.line_end(i + 1) > 0 => i += 1 + scan.line_end(i + 1),
                Some(next) => {
                    start.get_or_insert(offset);
                    text.push(next);
                    i += 2;
                }
            },
            '\'' => {
                start.get_or_insert(offset);
                i = scan.single_quoted(i, &mut text)?;
            }
            '"' => {
                start.get_or_insert(offset);
                i = scan.double_quoted(i, &mut text)?;
            }
            '$' if scan.at(i + 1) == Some('\'') => {
                start.get_or_insert(offset);
                i = scan.ansi_c_quoted(i, &mut text)?;
            }
            '^' if scan.at(i + 1) == Some('"') || scan.line_end(i + 1) > 0 => {
                return Err(CurlError::Syntax {
                    offset,
                    form: CurlSyntax::Cmd,
                });
            }
            '`' if scan.line_end(i + 1) > 0 => {
                return Err(CurlError::Syntax {
                    offset,
                    form: CurlSyntax::PowerShell,
                });
            }
            // An operator ends the command; an `&` glued to more text is part of a URL.
            '|' | ';' | '<' | '>' | '&' => {
                let operator = match (c, scan.at(i + 1)) {
                    ('&', Some('&')) => "&&".to_owned(),
                    ('&', Some(next)) if !next.is_whitespace() => {
                        start.get_or_insert(offset);
                        text.push('&');
                        i += 1;
                        continue;
                    }
                    _ => c.to_string(),
                };
                if let Some(offset) = start.take() {
                    words.push(Word {
                        text: std::mem::take(&mut text),
                        offset,
                    });
                }
                return Ok(Split {
                    words,
                    tail: Some(Tail { offset, operator }),
                });
            }
            _ => {
                start.get_or_insert(offset);
                text.push(c);
                i += 1;
            }
        }
    }
    if let Some(offset) = start {
        words.push(Word { text, offset });
    }
    Ok(Split { words, tail: None })
}

/// The input with its characters indexed, so quoted runs can look ahead and report offsets.
struct Scanner<'a> {
    command: &'a str,
    chars: Vec<(usize, char)>,
}

impl Scanner<'_> {
    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).map(|&(_, c)| c)
    }

    fn offset(&self, i: usize) -> usize {
        self.chars
            .get(i)
            .map_or(self.command.len(), |&(offset, _)| offset)
    }

    /// How many characters the line end at `i` spans: 1 for `\n`, 2 for `\r\n`, else 0.
    fn line_end(&self, i: usize) -> usize {
        match (self.at(i), self.at(i + 1)) {
            (Some('\n'), _) => 1,
            (Some('\r'), Some('\n')) => 2,
            _ => 0,
        }
    }

    fn unclosed(&self, open: usize) -> CurlError {
        CurlError::Syntax {
            offset: self.offset(open),
            form: CurlSyntax::UnclosedQuote,
        }
    }

    /// Copies a `'...'` run opened at `open` verbatim; returns the index after the closing quote.
    fn single_quoted(&self, open: usize, text: &mut String) -> Result<usize, CurlError> {
        let mut i = open + 1;
        loop {
            match self.at(i) {
                None => return Err(self.unclosed(open)),
                Some('\'') => return Ok(i + 1),
                Some(c) => text.push(c),
            }
            i += 1;
        }
    }

    /// Copies a `"..."` run, where a backslash escapes only `$`, `` ` ``, `"`, `\` and a line end.
    fn double_quoted(&self, open: usize, text: &mut String) -> Result<usize, CurlError> {
        let mut i = open + 1;
        loop {
            match self.at(i) {
                None => return Err(self.unclosed(open)),
                Some('"') => return Ok(i + 1),
                Some('\\') if self.line_end(i + 1) > 0 => i += self.line_end(i + 1),
                Some('\\') if matches!(self.at(i + 1), Some('$' | '`' | '"' | '\\')) => {
                    i += 1;
                    text.extend(self.at(i));
                }
                Some(c) => text.push(c),
            }
            i += 1;
        }
    }

    /// Decodes a bash `$'...'` run opened by the `$` at `open`. An unknown escape keeps its
    /// backslash, and a numeric escape that names no character keeps its source text.
    fn ansi_c_quoted(&self, open: usize, text: &mut String) -> Result<usize, CurlError> {
        let mut i = open + 2;
        loop {
            match self.at(i) {
                None => return Err(self.unclosed(open)),
                Some('\'') => return Ok(i + 1),
                Some('\\') => {
                    let Some(escape) = self.at(i + 1) else {
                        return Err(self.unclosed(open));
                    };
                    let simple = match escape {
                        'a' => Some('\u{7}'),
                        'b' => Some('\u{8}'),
                        'e' | 'E' => Some('\u{1b}'),
                        'f' => Some('\u{c}'),
                        'n' => Some('\n'),
                        'r' => Some('\r'),
                        't' => Some('\t'),
                        'v' => Some('\u{b}'),
                        '\\' | '\'' | '"' | '?' => Some(escape),
                        _ => None,
                    };
                    if let Some(c) = simple {
                        text.push(c);
                        i += 2;
                        continue;
                    }
                    let (digits_from, radix, max) = match escape {
                        '0'..='7' => (i + 1, 8, 3),
                        'x' => (i + 2, 16, 2),
                        'u' => (i + 2, 16, 4),
                        'U' => (i + 2, 16, 8),
                        _ => {
                            text.push('\\');
                            i += 1;
                            continue;
                        }
                    };
                    let mut end = digits_from;
                    while end - digits_from < max && self.at(end).is_some_and(|c| c.is_digit(radix))
                    {
                        end += 1;
                    }
                    let digits = &self.command[self.offset(digits_from)..self.offset(end)];
                    match u32::from_str_radix(digits, radix)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        Some(c) => text.push(c),
                        None => text.push_str(&self.command[self.offset(i)..self.offset(end)]),
                    }
                    i = end;
                }
                Some(c) => {
                    text.push(c);
                    i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{CurlError, CurlSyntax};

    fn words(command: &str) -> Vec<String> {
        split(command)
            .expect("splits")
            .words
            .into_iter()
            .map(|w| w.text)
            .collect()
    }

    #[test]
    fn posix_quoting_and_concatenation() {
        assert_eq!(
            words(r#"curl 'a b' "c d" e\ f g'h'"i""#),
            ["curl", "a b", "c d", "e f", "ghi"]
        );
        assert_eq!(words(r#"curl '' "" x"#), ["curl", "", "", "x"]);
        assert_eq!(
            words(r#"curl 'it'\''s' "say \"hi\" \$x \a""#),
            ["curl", "it's", r#"say "hi" $x \a"#]
        );
    }

    #[test]
    fn dollar_text_stays_literal() {
        assert_eq!(
            words(r#"curl $HOME "${TOKEN}" $(id) '{{x}}'"#),
            ["curl", "$HOME", "${TOKEN}", "$(id)", "{{x}}"]
        );
    }

    #[test]
    fn ansi_c_quotes_decode_bash_escapes() {
        assert_eq!(
            words(r"curl $'a\nb\t\'q\' \\ \x41é\101\e\^'"),
            ["curl", "a\nb\t'q' \\ AéA\u{1b}\\^"]
        );
    }

    #[test]
    fn continuations_and_newlines_separate_words() {
        assert_eq!(
            words("curl \\\n  -H 'A: b' \\\r\n  https://x.test"),
            ["curl", "-H", "A: b", "https://x.test"]
        );
        assert_eq!(
            words("curl https://x.test\n    -H 'A: b'"),
            ["curl", "https://x.test", "-H", "A: b"]
        );
        assert_eq!(words("curl \"a\\\nb\""), ["curl", "ab"]);
        assert_eq!(words("curl x \\"), ["curl", "x"]);
    }

    #[test]
    fn offsets_point_at_the_word_start() {
        let offsets: Vec<usize> = split("curl  -d 'x'")
            .expect("splits")
            .words
            .iter()
            .map(|w| w.offset)
            .collect();
        assert_eq!(offsets, [0, 6, 9]);
    }

    #[test]
    fn an_operator_ends_the_command() {
        let split_one = split("curl https://x.test | jq . ; echo done").expect("splits");
        let texts: Vec<&str> = split_one.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["curl", "https://x.test"]);
        assert_eq!(
            split_one.tail,
            Some(Tail {
                offset: 20,
                operator: "|".into()
            })
        );
        for (command, operator) in [
            ("curl x > out", ">"),
            ("curl x && y", "&&"),
            ("curl x &", "&"),
            ("curl x; y", ";"),
            ("curl x <in", "<"),
        ] {
            assert_eq!(
                split(command).expect("splits").tail.map(|t| t.operator),
                Some(operator.to_owned()),
                "{command}"
            );
        }
    }

    #[test]
    fn an_ampersand_inside_a_word_is_text() {
        assert_eq!(
            words("curl https://x.test/?a=1&b=2&c"),
            ["curl", "https://x.test/?a=1&b=2&c"]
        );
        assert_eq!(
            split("curl https://x.test/?a=1&b=2").expect("splits").tail,
            None
        );
    }

    #[test]
    fn comments_run_to_the_end_of_the_line() {
        assert_eq!(words("curl x # note 'unclosed\n"), ["curl", "x"]);
        assert_eq!(words("curl a#b"), ["curl", "a#b"]);
    }

    #[test]
    fn unclosed_quotes_name_the_opening_offset() {
        let err = |c: &str| split(c).expect_err("fails");
        assert_eq!(
            err("curl 'abc"),
            CurlError::Syntax {
                offset: 5,
                form: CurlSyntax::UnclosedQuote
            }
        );
        assert_eq!(
            err("curl x \"abc"),
            CurlError::Syntax {
                offset: 7,
                form: CurlSyntax::UnclosedQuote
            }
        );
        assert_eq!(
            err("curl $'abc"),
            CurlError::Syntax {
                offset: 5,
                form: CurlSyntax::UnclosedQuote
            }
        );
        // POSIX ends the single quote at `\'`, which leaves the later `"` open.
        assert_eq!(
            err(r#"curl -d '{"m": "Don\'t stop"}' https://x.test"#),
            CurlError::Syntax {
                offset: 27,
                form: CurlSyntax::UnclosedQuote
            }
        );
        // Offsets count bytes: `é` is two.
        assert_eq!(
            err("curl 'é' 'x"),
            CurlError::Syntax {
                offset: 10,
                form: CurlSyntax::UnclosedQuote
            }
        );
    }

    #[test]
    fn cmd_and_powershell_forms_are_refused() {
        let cmd = "curl ^\"https://x.test/^\" ^\n  -H ^\"accept: */*^\"";
        assert_eq!(
            split(cmd).expect_err("cmd"),
            CurlError::Syntax {
                offset: 5,
                form: CurlSyntax::Cmd
            }
        );
        let ps = "curl https://x.test `\n  -H 'a: b'";
        assert_eq!(
            split(ps).expect_err("ps"),
            CurlError::Syntax {
                offset: 20,
                form: CurlSyntax::PowerShell
            }
        );
        assert_eq!(words("curl 'a^\"b' x^y"), ["curl", "a^\"b", "x^y"]);
    }
}
