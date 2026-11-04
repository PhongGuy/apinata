//! Resolves a text offset in a JSON document to the node under it, for copy actions.

use crate::Assertion;
use crate::assertion::lhs::Segment;

/// The JSON node under a text offset.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JsonNode {
    /// The assertion left-hand side that reads this node, or `None` when no such expression exists.
    pub path: Option<String>,
    /// The node's value as text: a string unquoted, anything else as written in the source.
    pub value: String,
    /// An enabled assertion that holds for this scalar in the very document it was read from:
    /// `eq` with the value (quoted when it would otherwise read as another type), `isNull` for
    /// null. `None` for a container, a node without a path and a value no right side can state.
    pub assertion: Option<Assertion>,
}

/// The node of the token nearest to `offset` (a byte offset) on its line, or `None` for invalid
/// JSON and for a line without a token.
///
/// A token is a scalar, an object key (which stands for its member's value) or a bracket.
/// The token holding the character right of the caret wins, else the nearest, a tie going
/// left; commas and colons are not tokens. A path is returned only when it reads back exactly
/// this node.
pub fn json_node_at(text: &str, offset: usize) -> Option<JsonNode> {
    // Validity is checked once by serde_json, so the scanner below may trust the shape.
    serde_json::from_str::<serde_json::Value>(text).ok()?;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let mut scanner = Scanner { text, at: 0 };
    let root = scanner.value();
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    let mut tokens = Vec::new();
    collect(&root, (line_start, line_end), &mut tokens);
    let nearest = tokens
        .iter()
        .min_by_key(|t| (!t.holds(offset), t.distance(offset), t.start))?;
    let (span, steps) = locate(&root, nearest.node)?;
    let path = steps
        .filter(|steps| reads_back(text, steps, span))
        .map(|steps| {
            steps
                .iter()
                .fold("res.body".to_owned(), |mut acc, step: &Segment| {
                    acc.push_str(&step.render());
                    acc
                })
        });
    let assertion = path
        .as_deref()
        .and_then(|path| scalar_assertion(text, span, path));
    Some(JsonNode {
        assertion,
        path,
        value: render(text, span)?,
    })
}

/// The assertion on the scalar at `span`, kept only when the assertion engine passes it against
/// `text`: the first of the plain and the double-quoted right side that does.
fn scalar_assertion(text: &str, span: &Span, path: &str) -> Option<Assertion> {
    if !matches!(span.children, Children::None) {
        return None;
    }
    let candidates: Vec<Assertion> =
        match serde_json::from_str::<serde_json::Value>(&text[span.start..span.end]).ok()? {
            serde_json::Value::Null => vec![Assertion::new(path, "isNull", None)],
            serde_json::Value::Bool(b) => vec![Assertion::new(path, "eq", Some(b.to_string()))],
            serde_json::Value::Number(n) => {
                let shown = crate::js::number_to_string(n.as_f64()?);
                vec![Assertion::new(path, "eq", Some(shown))]
            }
            serde_json::Value::String(s) => vec![
                Assertion::new(path, "eq", Some(s.clone())),
                Assertion::new(path, "eq", Some(format!("\"{s}\""))),
            ],
            _ => return None,
        };
    let res = crate::Response::new(200).with_text_body(text);
    let vars = crate::interpolate::Vars::new();
    candidates.into_iter().find(|a| {
        let out = crate::assertion::evaluate_with(std::slice::from_ref(a), &res, &vars, None);
        matches!(out[0].verdict(), crate::Verdict::Pass)
    })
}

/// Whether following `steps` through the parsed document yields exactly the value at `span`.
///
/// This walks own keys itself (the last repeat wins, as in JavaScript) instead of calling the
/// assertion evaluator, which declines any object holding a `__proto__` key, siblings included.
/// `__proto__` itself is never addressed, since reading it gives the prototype.
fn reads_back(text: &str, steps: &[Segment], span: &Span) -> bool {
    let Ok(want) = serde_json::from_str::<serde_json::Value>(&text[span.start..span.end]) else {
        return false;
    };
    let Ok(mut node) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    for step in steps {
        let next = match (step, &node) {
            (Segment::Dot(k) | Segment::Key(k), serde_json::Value::Object(map))
                if k != "__proto__" =>
            {
                map.get(k)
            }
            (Segment::Index(i), serde_json::Value::Array(items)) => {
                i.parse::<usize>().ok().and_then(|i| items.get(i))
            }
            _ => None,
        };
        let Some(next) = next else { return false };
        node = next.clone();
    }
    node == want
}

/// A value's byte span and the members or items inside it.
struct Span {
    start: usize,
    end: usize,
    children: Children,
}

enum Children {
    None,
    Items(Vec<Span>),
    /// Key text, key span (quotes included), value.
    Members(Vec<(String, usize, usize, Span)>),
}

/// Walks the spans of text already known to be valid JSON.
struct Scanner<'a> {
    text: &'a str,
    at: usize,
}

impl Scanner<'_> {
    fn skip_space(&mut self) {
        let rest = &self.text[self.at..];
        self.at += rest.len() - rest.trim_start_matches([' ', '\t', '\n', '\r']).len();
    }

    fn byte(&self) -> u8 {
        self.text.as_bytes()[self.at]
    }

    fn value(&mut self) -> Span {
        self.skip_space();
        let start = self.at;
        let children = match self.byte() {
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                loop {
                    self.skip_space();
                    if self.byte() == b'}' {
                        self.at += 1;
                        break;
                    }
                    if self.byte() == b',' {
                        self.at += 1;
                        continue;
                    }
                    let key_start = self.at;
                    self.string();
                    let key_end = self.at;
                    let key = serde_json::from_str::<String>(&self.text[key_start..key_end])
                        .unwrap_or_default();
                    self.skip_space();
                    self.at += 1; // the colon
                    members.push((key, key_start, key_end, self.value()));
                }
                Children::Members(members)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    match self.byte() {
                        b']' => {
                            self.at += 1;
                            break;
                        }
                        b',' => self.at += 1,
                        _ => items.push(self.value()),
                    }
                }
                Children::Items(items)
            }
            b'"' => {
                self.string();
                Children::None
            }
            _ => {
                let rest = &self.text[self.at..];
                let len = rest
                    .find([',', ']', '}', ' ', '\t', '\n', '\r'])
                    .unwrap_or(rest.len());
                self.at += len;
                Children::None
            }
        };
        Span {
            start,
            end: self.at,
            children,
        }
    }

    /// Moves past the string starting at the cursor.
    fn string(&mut self) {
        self.at += 1;
        loop {
            match self.byte() {
                b'\\' => self.at += 2,
                b'"' => break,
                _ => self.at += 1,
            }
        }
        self.at += 1;
    }
}

/// A clickable piece of text and the start of the node it stands for.
struct Token {
    start: usize,
    end: usize,
    node: usize,
}

impl Token {
    /// Whether the character right of a caret at `offset` belongs to the token.
    fn holds(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    fn distance(&self, offset: usize) -> usize {
        if offset < self.start {
            self.start - offset
        } else {
            offset.saturating_sub(self.end)
        }
    }
}

/// Gathers the tokens of `span` that lie within `line` (start, end), skipping other subtrees.
fn collect(span: &Span, line: (usize, usize), out: &mut Vec<Token>) {
    if span.end <= line.0 || span.start >= line.1 {
        return;
    }
    let emit = |out: &mut Vec<Token>, start: usize, end: usize, node: usize| {
        if start >= line.0 && end <= line.1 {
            out.push(Token { start, end, node });
        }
    };
    match &span.children {
        Children::None => emit(out, span.start, span.end, span.start),
        Children::Items(items) => {
            emit(out, span.start, span.start + 1, span.start);
            emit(out, span.end - 1, span.end, span.start);
            for item in items {
                collect(item, line, out);
            }
        }
        Children::Members(members) => {
            emit(out, span.start, span.start + 1, span.start);
            emit(out, span.end - 1, span.end, span.start);
            for (_, key_start, key_end, value) in members {
                emit(out, *key_start, *key_end, value.start);
                collect(value, line, out);
            }
        }
    }
}

/// The span starting at `target` and the accessors leading to it; the steps are `None` when
/// the grammar cannot spell one of them or an earlier twin of a repeated key is on the way.
fn locate(root: &Span, target: usize) -> Option<(&Span, Option<Vec<Segment>>)> {
    let mut span = root;
    let mut steps = Some(Vec::new());
    while span.start != target {
        let inside = |s: &Span| s.start <= target && target < s.end;
        match &span.children {
            Children::None => return None,
            Children::Items(items) => {
                let index = items.iter().position(inside)?;
                push(&mut steps, Some(Segment::for_index(index)));
                span = &items[index];
            }
            Children::Members(members) => {
                let index = members.iter().position(|m| inside(&m.3))?;
                let key = &members[index].0;
                // A repeated key reads its last occurrence, so an earlier twin has no path.
                let repeated = members[index + 1..].iter().any(|m| &m.0 == key);
                push(&mut steps, Segment::for_key(key).filter(|_| !repeated));
                span = &members[index].3;
            }
        }
    }
    Some((span, steps))
}

/// Appends one accessor, or poisons the path when the grammar cannot spell it.
fn push(path: &mut Option<Vec<Segment>>, step: Option<Segment>) {
    match (path.as_mut(), step) {
        (Some(steps), Some(step)) => steps.push(step),
        _ => *path = None,
    }
}

/// The node's copy text: a string unquoted, anything else as written. A container spanning
/// lines loses the indentation of the line it starts on, which is whitespace outside strings.
fn render(text: &str, span: &Span) -> Option<String> {
    let raw = &text[span.start..span.end];
    match &span.children {
        Children::None if raw.starts_with('"') => serde_json::from_str::<String>(raw).ok(),
        Children::None => Some(raw.to_owned()),
        _ => {
            let line_start = text[..span.start].rfind('\n').map_or(0, |i| i + 1);
            let line = &text[line_start..span.start];
            let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
            let lines: Vec<&str> = raw
                .split('\n')
                .enumerate()
                .map(|(i, l)| {
                    if i == 0 {
                        l
                    } else {
                        l.strip_prefix(indent).unwrap_or(l)
                    }
                })
                .collect();
            Some(lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Response;
    use crate::assertion::lhs::evaluate_expression;
    use serde_json::Value;

    /// The node whose text is `marker` (first occurrence) must evaluate through its own path.
    fn at(text: &str, needle: &str) -> JsonNode {
        let offset = text.find(needle).expect("needle in text");
        json_node_at(text, offset).unwrap_or_else(|| panic!("no node at {needle:?}"))
    }

    fn round_trips(text: &str, node: &JsonNode) {
        let Some(path) = &node.path else { return };
        let res = Response::new(200).with_text_body(text);
        let got = evaluate_expression(path, &res)
            .unwrap_or_else(|e| panic!("{path} not evaluated: {e:?}"))
            .unwrap_or_else(|| panic!("{path} undefined"));
        let want = match serde_json::from_str::<Value>(&node.value) {
            Ok(v) if !matches!(got, Value::String(_)) => v,
            _ => Value::String(node.value.clone()),
        };
        assert_eq!(got, want, "path {path}");
    }

    #[test]
    fn scalars_give_raw_text_and_a_path() {
        let text = r#"{"name":"a\"b","n":1.50,"t":true,"z":null}"#;
        let s = at(text, "a\\");
        assert_eq!(s.value, "a\"b");
        assert_eq!(s.path.as_deref(), Some("res.body.name"));
        assert_eq!(at(text, "1.50").value, "1.50");
        assert_eq!(at(text, "true").value, "true");
        assert_eq!(at(text, "null").value, "null");
        for needle in ["a\\", "1.50", "true", "null"] {
            round_trips(text, &at(text, needle));
        }
    }

    #[test]
    fn containers_give_the_source_slice_and_root_is_res_body() {
        let text = r#"{"a":[1.50,{"b":2,"b":3}],"e":{ },"l":[]}"#;
        let root = json_node_at(text, 0).unwrap();
        assert_eq!(root.path.as_deref(), Some("res.body"));
        assert_eq!(root.value, text);
        let arr = at(text, "[1");
        assert_eq!(arr.path.as_deref(), Some("res.body.a"));
        assert_eq!(arr.value, r#"[1.50,{"b":2,"b":3}]"#);
        assert_eq!(at(text, "{ }").value, "{ }");
        assert_eq!(at(text, "[]").path.as_deref(), Some("res.body.l"));
        for needle in ["[1", "{ }", "[]"] {
            round_trips(text, &at(text, needle));
        }
    }

    #[test]
    fn a_nested_container_loses_only_its_lines_indent() {
        let text = "{\n  \"a\": [\n    1.0,\n    2\n  ]\n}";
        let arr = at(text, "[\n");
        assert_eq!(arr.value, "[\n  1.0,\n  2\n]");
        let compact = |s: &str| s.split_whitespace().collect::<String>();
        assert_eq!(compact(&arr.value), "[1.0,2]");
    }

    #[test]
    fn nested_array_indexes_and_members() {
        let text = r#"[{"id":7},{"id":8,"tags":["x","y"]}]"#;
        let y = at(text, "\"y\"");
        assert_eq!(y.path.as_deref(), Some("res.body[1].tags[1]"));
        assert_eq!(y.value, "y");
        round_trips(text, &y);
        round_trips(text, &at(text, "8"));
    }

    #[test]
    fn a_key_resolves_to_its_member_value() {
        let text = r#"{"first":{"x":1},"second":2}"#;
        let on_key = at(text, "first");
        assert_eq!(on_key.path.as_deref(), Some("res.body.first"));
        assert_eq!(on_key.value, r#"{"x":1}"#);
        assert_eq!(at(text, "second").value, "2");
    }

    #[test]
    fn a_caret_between_tokens_takes_the_nearest_token_on_its_line() {
        let text = "{\n  \"a\": [\n    1, 2\n  ],\n  \"b\": 3\n}\n";
        // Right of `[` at the end of its line: the array, never the root.
        let after_bracket = text.find("[\n").unwrap() + 1;
        let n = json_node_at(text, after_bracket).unwrap();
        assert_eq!(n.path.as_deref(), Some("res.body.a"));
        // On the comma between 1 and 2: equally near, so the left token.
        let comma = text.find(", 2").unwrap();
        assert_eq!(json_node_at(text, comma + 1).unwrap().value, "1");
        // One column from the right value, closer to it than to the key.
        let colon = text.find(": 3").unwrap();
        assert_eq!(json_node_at(text, colon + 2).unwrap().value, "3");
        // Indentation resolves to the first token of the line.
        let indent = text.find("  \"b\"").unwrap();
        assert_eq!(
            json_node_at(text, indent).unwrap().path.as_deref(),
            Some("res.body.b")
        );
        // After a trailing comma: the value before it.
        let end_of_a_close = text.find("],").unwrap() + 2;
        assert_eq!(
            json_node_at(text, end_of_a_close).unwrap().path.as_deref(),
            Some("res.body.a")
        );
        // The root is reachable on its own braces.
        assert_eq!(
            json_node_at(text, 0).unwrap().path.as_deref(),
            Some("res.body")
        );
        let close = text.rfind('}').unwrap();
        assert_eq!(
            json_node_at(text, close + 1).unwrap().path.as_deref(),
            Some("res.body")
        );
    }

    #[test]
    fn a_line_without_a_token_has_no_node() {
        // The trailing empty line, a blank line and a whitespace-only line.
        let text = "{\n  \"a\": 1,\n\n   \n  \"b\": 2\n}\n";
        assert!(json_node_at(text, text.len()).is_none());
        let blank = text.find("\n\n").unwrap() + 1;
        assert!(json_node_at(text, blank).is_none());
        let spaces = text.find("   \n").unwrap() + 2;
        assert!(json_node_at(text, spaces).is_none());
        assert!(json_node_at("{}\n", 3).is_none());
    }

    #[test]
    fn a_long_raw_line_picks_the_nearest_token_not_the_root() {
        let text = r#"{"a":[1,2],"b":3}"#;
        let colon = text.find(":3").unwrap();
        assert_eq!(json_node_at(text, colon).unwrap().value, "3");
        // Just before `1` the character under the caret is the item, not the array.
        let past_bracket = text.find("[1").unwrap() + 1;
        assert_eq!(
            json_node_at(text, past_bracket).unwrap().path.as_deref(),
            Some("res.body.a[0]")
        );
    }

    #[test]
    fn keys_pick_the_matching_accessor() {
        let text = r#"{"a-b":1,"x y":2,"it's":3,"q\"":4,"é":5,"7up":6,"$k_1":7,"":8,"a]":9}"#;
        assert_eq!(at(text, "a-b").path.as_deref(), Some("res.body['a-b']"));
        assert_eq!(at(text, "x y").path.as_deref(), Some("res.body['x y']"));
        assert_eq!(at(text, "it's").path, None);
        assert_eq!(at(text, "q\\").path, None);
        assert_eq!(at(text, "é").path.as_deref(), Some("res.body['é']"));
        assert_eq!(at(text, "7up").path.as_deref(), Some("res.body['7up']"));
        assert_eq!(at(text, "$k_1").path.as_deref(), Some("res.body.$k_1"));
        assert_eq!(at(text, "\"\":8").path.as_deref(), Some("res.body['']"));
        assert_eq!(at(text, "a]").path, None);
        for needle in ["a-b", "x y", "é", "7up", "$k_1", "\"\":8"] {
            round_trips(text, &at(text, needle));
        }
    }

    #[test]
    fn unicode_offsets_work_on_byte_positions() {
        let text = r#"{"k":"héllo wörld"}"#;
        let node = at(text, "wörld");
        assert_eq!(node.value, "héllo wörld");
        // An offset inside a multibyte character is not a position the editor can report.
        let inside = text.find('é').unwrap() + 1;
        assert!(json_node_at(text, inside).is_none());
    }

    #[test]
    fn invalid_json_and_out_of_range_give_none() {
        assert!(json_node_at("", 0).is_none());
        assert!(json_node_at("{\"a\":", 2).is_none());
        assert!(json_node_at("not json", 1).is_none());
        assert!(json_node_at("{\"a\":1} x", 1).is_none());
        assert!(json_node_at("{\"a\":1}", 99).is_none());
    }

    #[test]
    fn a_top_level_scalar_is_the_root() {
        let n = json_node_at("\"hi\"", 1).unwrap();
        assert_eq!(n.path.as_deref(), Some("res.body"));
        assert_eq!(n.value, "hi");
    }

    #[test]
    fn every_returned_path_reads_back_its_node() {
        let text = r#"{"__proto__":{"x":1},"y":2,"r":1,"r":2,"o":{"__proto__":3,"k":4}}"#;
        // `__proto__` and what lies beneath it have no path; a plain key beside it keeps its own.
        assert_eq!(at(text, "__proto__\":{").path, None);
        assert_eq!(at(text, "x").path, None);
        assert_eq!(at(text, "2,\"r").path.as_deref(), Some("res.body.y"));
        assert_eq!(at(text, "k").path.as_deref(), Some("res.body.o.k"));
        // The earlier twin of a repeated key cannot be addressed; the last one can.
        let first = json_node_at(text, text.find("\"r\":1").unwrap()).unwrap();
        assert_eq!((first.value.as_str(), first.path.as_deref()), ("1", None));
        let last = json_node_at(text, text.rfind("\"r\":2").unwrap()).unwrap();
        assert_eq!(last.path.as_deref(), Some("res.body.r"));
        let clean = r#"{"r":1,"r":2,"p":{"q":[true]}}"#;
        let last = json_node_at(clean, clean.rfind("\"r\"").unwrap()).unwrap();
        assert_eq!(last.path.as_deref(), Some("res.body.r"));
        round_trips(clean, &last);
        let first = json_node_at(clean, clean.find("\"r\"").unwrap()).unwrap();
        assert_eq!(first.path, None);
        // Every node of a document, found from every offset, either has no path or reads back.
        let doc = r#"{"a":[1,{"b":null}],"a b":"s","a":0}"#;
        for offset in 0..=doc.len() {
            if let Some(node) = json_node_at(doc, offset) {
                round_trips(doc, &node);
            }
        }
    }

    #[test]
    fn a_repeated_or_proto_key_spares_the_nodes_around_it() {
        // Only the earlier twin loses its path; unique nodes, even in the same object, keep theirs.
        let dup = r#"{"u":1,"d":{"a":1},"d":{"a":2,"b":[3]},"w":{"z":true}}"#;
        let node = at(dup, "z");
        assert_eq!(node.path.as_deref(), Some("res.body.w.z"));
        round_trips(dup, &node);
        let node = at(dup, "3");
        assert_eq!(node.path.as_deref(), Some("res.body.d.b[0]"));
        round_trips(dup, &node);
        assert_eq!(at(dup, "u").path.as_deref(), Some("res.body.u"));
        assert_eq!(at(dup, "a\":1}").path, None);
        // In JavaScript JSON.parse makes `__proto__` an own key, so siblings read normally.
        let proto = r#"{"__proto__":{"p":1},"s":{"t":[5]},"u":"v"}"#;
        assert_eq!(at(proto, "5").path.as_deref(), Some("res.body.s.t[0]"));
        assert_eq!(at(proto, "t\"").path.as_deref(), Some("res.body.s.t"));
        assert_eq!(at(proto, "v\"").path.as_deref(), Some("res.body.u"));
        assert_eq!(at(proto, "p\"").path, None);
        assert_eq!(at(proto, "__proto__").path, None);
    }

    /// The assertion the node offers must pass against the very document it came from.
    fn asserts(text: &str, needle: &str) -> Option<(String, String, Option<String>)> {
        let node = at(text, needle);
        let a = node.assertion?;
        let res = Response::new(200).with_text_body(text);
        let out = crate::assertion::evaluate_with(
            std::slice::from_ref(&a),
            &res,
            &crate::interpolate::Vars::new(),
            None,
        );
        assert!(
            matches!(out[0].verdict(), crate::Verdict::Pass),
            "{a:?} gave {:?}",
            out[0].verdict()
        );
        Some((
            a.expression().to_owned(),
            a.operator().to_owned(),
            a.value().map(str::to_owned),
        ))
    }

    fn row(expr: &str, op: &str, value: Option<&str>) -> Option<(String, String, Option<String>)> {
        Some((expr.to_owned(), op.to_owned(), value.map(str::to_owned)))
    }

    #[test]
    fn a_scalar_offers_an_assertion_that_passes_on_its_own_response() {
        let text = r#"{"name":"ann","n":1.50,"t":false,"z":null,"i":42,"e":1e3}"#;
        assert_eq!(
            asserts(text, "ann"),
            row("res.body.name", "eq", Some("ann"))
        );
        assert_eq!(asserts(text, "1.50"), row("res.body.n", "eq", Some("1.5")));
        assert_eq!(
            asserts(text, "false"),
            row("res.body.t", "eq", Some("false"))
        );
        assert_eq!(asserts(text, "null"), row("res.body.z", "isNull", None));
        assert_eq!(asserts(text, "42"), row("res.body.i", "eq", Some("42")));
        assert_eq!(asserts(text, "1e3"), row("res.body.e", "eq", Some("1000")));
    }

    #[test]
    fn a_string_that_reads_as_another_type_or_needs_trimming_is_quoted() {
        let text = r#"{"a":"42","b":"true","c":"null","d":" pad ","e":"","f":"\"q\""}"#;
        assert_eq!(asserts(text, "42"), row("res.body.a", "eq", Some("\"42\"")));
        assert_eq!(
            asserts(text, "true"),
            row("res.body.b", "eq", Some("\"true\""))
        );
        assert_eq!(
            asserts(text, "null"),
            row("res.body.c", "eq", Some("\"null\""))
        );
        assert_eq!(
            asserts(text, " pad"),
            row("res.body.d", "eq", Some("\" pad \""))
        );
        assert!(asserts(text, "\"\",").is_some());
        assert!(asserts(text, "q").is_some());
    }

    #[test]
    fn nested_and_awkward_keys_use_the_path_the_engine_reads() {
        let text = r#"{"a b":[{"x-y":"v"}],"k":{"1":true}}"#;
        assert_eq!(
            asserts(text, "v\""),
            row("res.body['a b'][0]['x-y']", "eq", Some("v"))
        );
        assert!(asserts(text, "true").is_some());
        assert!(asserts(r#"[1,2]"#, "2").is_some());
    }

    #[test]
    fn containers_and_unreadable_values_offer_no_assertion() {
        let text =
            r#"{"o":{"a":1},"l":[1],"s":"back\\slash","t":"{{x}}","big":12345678901234567890}"#;
        assert!(at(text, "{\"a").assertion.is_none());
        assert!(at(text, "[1]").assertion.is_none());
        assert!(at(text, "back").assertion.is_none());
        assert!(at(text, "{{x}}").assertion.is_none());
        assert!(at(text, "1234567").assertion.is_none());
        // A node with no path has nothing to assert on.
        let dup = r#"{"d":{"a":1},"d":{"a":2}}"#;
        assert!(at(dup, "1").assertion.is_none());
    }
}
