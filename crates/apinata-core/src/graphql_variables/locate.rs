//! Finds the cursor's place in half-typed variables JSON, which no strict parser accepts.

use std::ops::Range;

/// One step from the root value down to the cursor's place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Key(String),
    Index(usize),
}

/// Whether the cursor names a key or sits where a value goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Key,
    Value,
}

/// Where the cursor sits and what surrounds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    /// Key role: the steps to the enclosing object. Value role: the steps to the value itself.
    pub(crate) path: Vec<Step>,
    pub(crate) role: Role,
    /// The token under the cursor (quotes included), or `offset..offset` in a gap.
    pub(crate) token: Range<usize>,
    /// The token's text from after its opening quote, if any, up to the cursor.
    pub(crate) prefix: String,
    /// Key role: the raw keys of the enclosing object, before and after the cursor, except the
    /// key under the cursor. Value role: empty.
    pub(crate) present: Vec<String>,
}

/// The cursor's place in half-typed JSON, or None off a key or value position, inside a
/// placeholder, at a non-char-boundary offset, or at the root value position.
pub(crate) fn locate(text: &str, offset: usize) -> Option<Place> {
    if !text.is_char_boundary(offset) {
        return None;
    }
    let mut scan = Scan::default();
    // A key place keeps scanning so its object also reports the keys after the cursor;
    // the depth is the stack height of that object.
    let mut found: Option<(Place, usize)> = None;
    for token in tokens(text) {
        if found.is_none() {
            // The first token that decides the cursor's position answers from the state
            // there; a position that takes no key or value is None, never a later state.
            let decided = if token.contains(offset) {
                let in_placeholder = token.kind == Kind::Placeholder
                    || token
                        .inner
                        .iter()
                        .any(|span| span.start <= offset && offset <= span.end);
                if in_placeholder {
                    return None;
                }
                let from = match token.kind {
                    Kind::Str { .. } => (token.span.start + 1).min(offset),
                    _ => token.span.start,
                };
                Some(scan.place(token.span.clone(), &text[from..offset]))
            } else if offset < token.span.start
                || (offset == token.span.start && token.kind.is_punctuation())
            {
                Some(scan.place(offset..offset, ""))
            } else {
                None
            };
            if let Some(place) = decided {
                let place = place?;
                if place.role == Role::Value {
                    return Some(place);
                }
                found = Some((place, scan.stack.len()));
            }
        }
        let key = scan.step(&token, text);
        if let (Some((place, depth)), Some((at, raw))) = (&mut found, key)
            && at == *depth
            && token.span != place.token
        {
            place.present.push(raw);
        }
        if found
            .as_ref()
            .is_some_and(|(_, depth)| scan.stack.len() < *depth)
        {
            break;
        }
    }
    // Every token ended before the cursor, so the state at the end of the text answers.
    match found {
        Some((place, _)) => Some(place),
        None => scan.place(offset..offset, ""),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open { object: bool },
    Close { object: bool },
    Colon,
    Comma,
    Str { closed: bool },
    Word,
    Placeholder,
    Other,
}

impl Kind {
    fn is_punctuation(self) -> bool {
        matches!(
            self,
            Kind::Open { .. } | Kind::Close { .. } | Kind::Colon | Kind::Comma
        )
    }
}

struct Token {
    kind: Kind,
    span: Range<usize>,
    /// Placeholders inside a string token.
    inner: Vec<Range<usize>>,
}

impl Token {
    // A closed string ends before its closing quote; an open-ended token keeps its end edge.
    fn contains(&self, offset: usize) -> bool {
        let Range { start, end } = self.span;
        match self.kind {
            Kind::Str { closed: true } => start <= offset && offset < end,
            Kind::Str { closed: false } | Kind::Word | Kind::Placeholder => {
                start <= offset && offset <= end
            }
            _ => false,
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '+' | '.')
}

// Splits the text into punctuation, strings, placeholders and bare words. A string with no
// closing quote ends at the line break, since JSON strings hold none; a placeholder inside a
// string stays string content.
fn tokens(text: &str) -> Vec<Token> {
    let mut spans = crate::placeholders(text)
        .into_iter()
        .map(|(range, _)| range)
        .filter(|range| !text[range.clone()].contains('\n'))
        .peekable();
    let mut chars = text.char_indices().peekable();
    let mut out = Vec::new();
    let token = |kind, span| Token {
        kind,
        span,
        inner: Vec::new(),
    };
    while let Some(&(at, c)) = chars.peek() {
        while spans.next_if(|span| span.start < at).is_some() {}
        if let Some(span) = spans.next_if(|span| span.start == at) {
            while chars.next_if(|&(i, _)| i < span.end).is_some() {}
            out.push(token(Kind::Placeholder, span));
            continue;
        }
        chars.next();
        let next = at + c.len_utf8();
        match c {
            '{' | '[' => out.push(token(Kind::Open { object: c == '{' }, at..next)),
            '}' | ']' => out.push(token(Kind::Close { object: c == '}' }, at..next)),
            ':' => out.push(token(Kind::Colon, at..next)),
            ',' => out.push(token(Kind::Comma, at..next)),
            '"' => {
                let (mut end, mut closed, mut inner) = (text.len(), false, Vec::new());
                while let Some(&(i, c)) = chars.peek() {
                    if c == '\n' || c == '\r' {
                        end = i;
                        break;
                    }
                    while spans.next_if(|span| span.start < i).is_some() {}
                    if let Some(span) = spans.next_if(|span| span.start == i) {
                        while chars.next_if(|&(j, _)| j < span.end).is_some() {}
                        inner.push(span);
                        continue;
                    }
                    chars.next();
                    if c == '\\' {
                        chars.next_if(|&(_, n)| n != '\n' && n != '\r');
                    } else if c == '"' {
                        (end, closed) = (i + 1, true);
                        break;
                    }
                }
                out.push(Token {
                    kind: Kind::Str { closed },
                    span: at..end,
                    inner,
                });
            }
            c if c.is_whitespace() => {}
            c if is_word_char(c) => {
                let mut end = next;
                while let Some((i, c)) = chars.next_if(|&(_, c)| is_word_char(c)) {
                    end = i + c.len_utf8();
                }
                out.push(token(Kind::Word, at..end));
            }
            _ => out.push(token(Kind::Other, at..next)),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    ExpectKey,
    AfterKey,
    ExpectValue,
    AfterValue,
}

struct Frame {
    object: bool,
    state: State,
    /// Object: the latest key. Array: unused.
    key: Option<String>,
    /// Array: the index of the current element. Object: unused.
    index: usize,
    /// The step from the parent to this frame; None for the root value.
    step: Option<Step>,
    present: Vec<String>,
}

impl Frame {
    // The step to the value this frame expects next.
    fn value_step(&self) -> Option<Step> {
        if self.object {
            self.key.clone().map(Step::Key)
        } else {
            Some(Step::Index(self.index))
        }
    }
}

#[derive(Default)]
struct Scan {
    stack: Vec<Frame>,
    root_done: bool,
}

impl Scan {
    fn expects_value(&self) -> bool {
        match self.stack.last() {
            Some(top) => top.state == State::ExpectValue,
            None => !self.root_done,
        }
    }

    // The place for a cursor at `token` in the current state; the root value has none.
    fn place(&self, token: Range<usize>, prefix: &str) -> Option<Place> {
        let top = self.stack.last()?;
        let mut path: Vec<Step> = self.stack.iter().filter_map(|f| f.step.clone()).collect();
        let role = match (top.object, top.state) {
            (true, State::ExpectKey) => Role::Key,
            (_, State::ExpectValue) => {
                path.push(top.value_step()?);
                Role::Value
            }
            _ => return None,
        };
        Some(Place {
            path,
            role,
            token,
            prefix: prefix.to_owned(),
            present: if role == Role::Key {
                top.present.clone()
            } else {
                Vec::new()
            },
        })
    }

    // Advances the frame stack over one token. Returns a key it read, with the stack height
    // of the object that holds it.
    fn step(&mut self, token: &Token, text: &str) -> Option<(usize, String)> {
        match token.kind {
            Kind::Open { object } => {
                if !self.expects_value() {
                    return None;
                }
                let step = match self.stack.last_mut() {
                    Some(parent) => {
                        parent.state = State::AfterValue;
                        parent.value_step()
                    }
                    None => {
                        self.root_done = true;
                        None
                    }
                };
                self.stack.push(Frame {
                    object,
                    state: if object {
                        State::ExpectKey
                    } else {
                        State::ExpectValue
                    },
                    key: None,
                    index: 0,
                    step,
                    present: Vec::new(),
                });
            }
            Kind::Close { object } => {
                if let Some(at) = self.stack.iter().rposition(|f| f.object == object) {
                    self.stack.truncate(at);
                }
            }
            Kind::Colon => {
                if let Some(top) = self.stack.last_mut()
                    && top.object
                    && top.state == State::AfterKey
                {
                    top.state = State::ExpectValue;
                }
            }
            Kind::Comma => {
                if let Some(top) = self.stack.last_mut() {
                    if top.object {
                        top.state = State::ExpectKey;
                    } else {
                        top.index += 1;
                        top.state = State::ExpectValue;
                    }
                }
            }
            Kind::Str { .. } | Kind::Word | Kind::Placeholder => {
                let depth = self.stack.len();
                match self.stack.last_mut() {
                    Some(top) if top.object && top.state == State::ExpectKey => {
                        let raw = raw_key(token, text).to_owned();
                        top.key = Some(raw.clone());
                        top.present.push(raw.clone());
                        top.state = State::AfterKey;
                        return Some((depth, raw));
                    }
                    Some(top) if top.state == State::ExpectValue => top.state = State::AfterValue,
                    None => self.root_done = true,
                    _ => {}
                }
            }
            Kind::Other => {}
        }
        None
    }
}

// A key's text as written: a string without its quotes, anything else whole.
fn raw_key<'t>(token: &Token, text: &'t str) -> &'t str {
    let Range { start, end } = token.span;
    match token.kind {
        Kind::Str { closed: true } => &text[start + 1..end - 1],
        Kind::Str { closed: false } => &text[start + 1..end],
        _ => &text[start..end],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    fn at(marked: &str) -> Option<Place> {
        let offset = marked.find('|').expect("a cursor");
        locate(&marked.replacen('|', "", 1), offset)
    }

    fn k(name: &str) -> Step {
        Step::Key(name.to_owned())
    }

    fn check(
        marked: &str,
        role: Role,
        path: &[Step],
        token: Range<usize>,
        prefix: &str,
        present: &[&str],
    ) {
        let p = at(marked).unwrap_or_else(|| panic!("no place in {marked:?}"));
        assert_eq!(p.role, role, "{marked:?}");
        assert_eq!(p.path, path, "{marked:?}");
        assert_eq!(p.token, token, "{marked:?}");
        assert_eq!(p.prefix, prefix, "{marked:?}");
        assert_eq!(p.present, present, "{marked:?}");
    }

    #[test]
    fn key_positions() {
        check("{|}", Role::Key, &[], 1..1, "", &[]);
        check("{\"|\"}", Role::Key, &[], 1..3, "", &[]);
        check("{\"i|", Role::Key, &[], 1..3, "i", &[]);
        check("{i|}", Role::Key, &[], 1..2, "i", &[]);
        check("{\"id\": 1, \"|\"}", Role::Key, &[], 10..12, "", &["id"]);
        check(
            "{\"|\", \"role\": \"ADMIN\"}",
            Role::Key,
            &[],
            1..3,
            "",
            &["role"],
        );
        check(
            "{\"id\": 1, \"ro|le\": 2}",
            Role::Key,
            &[],
            10..16,
            "ro",
            &["id"],
        );
        check("{|\"id\": 1}", Role::Key, &[], 1..5, "", &[]);
        check("{{|", Role::Key, &[], 2..2, "", &[]);
    }

    #[test]
    fn nested_keys_carry_their_path() {
        check(
            "{\"filter\": {\"a\": 1, \"|\"}}",
            Role::Key,
            &[k("filter")],
            20..22,
            "",
            &["a"],
        );
        check(
            "{\"list\": [{\"x\": 1}, {|}]}",
            Role::Key,
            &[k("list"), Step::Index(1)],
            21..21,
            "",
            &[],
        );
    }

    #[test]
    fn escapes_multibyte_text_and_line_breaks() {
        check(
            "{\"a\\\"b\": 1, \"|\"}",
            Role::Key,
            &[],
            12..14,
            "",
            &["a\\\"b"],
        );
        check(
            "{\"\u{f8}\": \"\u{e9}\", \"|\"}",
            Role::Key,
            &[],
            13..15,
            "",
            &["\u{f8}"],
        );
        check("{\"i|\n}", Role::Key, &[], 1..3, "i", &[]);
        check("{\"a\": \"x\n, \"|\"}", Role::Key, &[], 11..13, "", &["a"]);
        check("{\"i|\r\n}", Role::Key, &[], 1..3, "i", &[]);
        check(
            "{\"a\": \"x\r\n, \"|\"}",
            Role::Key,
            &[],
            12..14,
            "",
            &["a"],
        );
    }

    #[test]
    fn value_positions() {
        check("{\"role\": |}", Role::Value, &[k("role")], 9..9, "", &[]);
        check(
            "{\"role\": \"AD|\"}",
            Role::Value,
            &[k("role")],
            9..13,
            "AD",
            &[],
        );
        check("{\"ok\": tr|}", Role::Value, &[k("ok")], 7..9, "tr", &[]);
        check("{\"n\": 12|}", Role::Value, &[k("n")], 6..8, "12", &[]);
        check(
            "{\"ids\": [1, |]}",
            Role::Value,
            &[k("ids"), Step::Index(1)],
            12..12,
            "",
            &[],
        );
        check(
            "{\"ids\": [|",
            Role::Value,
            &[k("ids"), Step::Index(0)],
            9..9,
            "",
            &[],
        );
        check(
            "{\"f\": {\"roles\": [\"A|\"]}}",
            Role::Value,
            &[k("f"), k("roles"), Step::Index(0)],
            17..20,
            "A",
            &[],
        );
        check(
            "{\"f\": {\"role\": \"A|",
            Role::Value,
            &[k("f"), k("role")],
            15..17,
            "A",
            &[],
        );
    }

    #[test]
    fn placeholders_are_single_values() {
        check(
            "{\"id\": {{id}}, \"|\"}",
            Role::Key,
            &[],
            15..17,
            "",
            &["id"],
        );
        check(
            "{\"name\": \"a{{x}}b\", \"|\"}",
            Role::Key,
            &[],
            20..22,
            "",
            &["name"],
        );
        for marked in [
            "{\"id\": {{i|d}}}",
            "{\"id\": \"{{i|d}}\"}",
            "{\"id\": |{{id}}}",
            "{\"id\": {{id}}|}",
        ] {
            assert_eq!(at(marked), None, "{marked:?}");
        }
    }

    #[test]
    fn no_place_off_keys_and_values() {
        for marked in [
            "{\"a\": 1 |}",
            "{\"a\"| : 1}",
            "|{}",
            "{\"a\": \"x\"|}",
            "|",
            "{\"a\": 1 |,",
            "{\"a\": 1 |, \"b\": ",
            "|{",
            "|{\n\"a\": 1",
        ] {
            assert_eq!(at(marked), None, "{marked:?}");
        }
        assert_eq!(locate("{\"\u{f8}\"}", 3), None);
    }
}
