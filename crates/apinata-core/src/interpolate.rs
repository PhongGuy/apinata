//! Expands Bruno's `{{...}}` placeholders over a variable map, with lodash `get` paths.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use regress::Regex;
use serde_json::Value;

use crate::dynamic::{Generator, mock};
use crate::error::ResolveError;
use crate::js;
use crate::prompt::prompt_label;

/// The lookup map for one request: variable names to values, later layers already merged in.
pub(crate) type Vars = serde_json::Map<String, Value>;

/// Expands every `{{name}}` in `text`, recursively expanding the values it pulls in, with
/// `{{$name}}` dynamic values drawn from an OS-seeded generator.
pub(crate) fn interpolate(text: &str, vars: &Vars) -> Result<String, ResolveError> {
    interpolate_tracked(text, vars, None, &RefCell::new(Generator::system()))
}

/// Collects the expanded values of placeholders that read a sensitive variable, across calls.
pub(crate) struct Tracker {
    /// Top-level variable names whose values are sensitive.
    names: BTreeSet<String>,
    /// Values known to be sensitive wherever they appear, such as secrets a script copied.
    tainted: Vec<String>,
    /// Non-empty values in the order first substituted, without repeats.
    values: RefCell<Vec<String>>,
}

impl Tracker {
    /// A tracker treating the variables named in `names` as sensitive.
    pub(crate) fn new(names: BTreeSet<String>) -> Tracker {
        Tracker {
            names,
            tainted: Vec::new(),
            values: RefCell::new(Vec::new()),
        }
    }

    /// This tracker also recording each of `values` found inside any substituted value.
    pub(crate) fn tainted(mut self, values: Vec<String>) -> Tracker {
        self.tainted = values.into_iter().filter(|v| !v.is_empty()).collect();
        self
    }

    /// Whether no sensitive value has been recorded yet.
    pub(crate) fn is_empty(&self) -> bool {
        self.values.borrow().is_empty()
    }

    /// The recorded values, in first-substituted order.
    pub(crate) fn into_values(self) -> Vec<String> {
        self.values.into_inner()
    }

    /// Records `value` when placeholder `name` reads a sensitive variable: the literal key
    /// when `vars` holds it, else the first key of its lodash path. Any tainted value inside
    /// `value` is recorded whatever the name.
    fn note(&self, name: &str, vars: &Vars, value: &str) {
        if value.is_empty() {
            return;
        }
        let read = if vars.contains_key(name) {
            Some(name.to_owned())
        } else {
            to_path(name).into_iter().next()
        };
        let mut values = self.values.borrow_mut();
        let mut record = |found: &str| {
            if !values.iter().any(|seen| seen == found) {
                values.push(found.to_owned());
            }
        };
        if read.is_some_and(|key| self.names.contains(&key)) {
            record(value);
        }
        for tainted in self.tainted.iter().filter(|t| value.contains(t.as_str())) {
            record(tainted);
        }
    }
}

/// [`interpolate`] with dynamic values from `generator`, also recording sensitive
/// substitutions in `tracker` when one is given.
pub(crate) fn interpolate_tracked(
    text: &str,
    vars: &Vars,
    tracker: Option<&Tracker>,
    generator: &RefCell<Generator>,
) -> Result<String, ResolveError> {
    let mut state = State {
        vars,
        tracker,
        generator,
        stack: Vec::new(),
        results: HashMap::new(),
    };
    // Bruno's `prepareMock` order: known `{{$name}}` go first, so no variable shadows them.
    let text = mock(text, &mut generator.borrow_mut());
    let out = state.expand(&text)?;
    // Bruno sends what its loop leaves unresolved as literal text; Apinata refuses to.
    // A leftover prompt, such as one a script or `process.env` wrote, is asked for by label.
    match next_placeholder(&out) {
        Some((_, name, _)) => Err(match prompt_label(name) {
            Some(label) => ResolveError::PromptRequired {
                labels: vec![label.to_owned()],
            },
            None => ResolveError::UndefinedVariable {
                name: name.to_owned(),
                hint: None,
            },
        }),
        None => Ok(out),
    }
}

/// [`interpolate_tracked`] for script getters: an undefined placeholder stays as written,
/// and text whose expansion cycles comes back unchanged.
pub(crate) fn interpolate_lenient(
    text: &str,
    vars: &Vars,
    tracker: &Tracker,
    generator: &RefCell<Generator>,
) -> String {
    let mut state = State {
        vars,
        tracker: Some(tracker),
        generator,
        stack: Vec::new(),
        results: HashMap::new(),
    };
    let mocked = mock(text, &mut generator.borrow_mut());
    state.expand(&mocked).unwrap_or_else(|_| text.to_owned())
}

/// A rescan loop that keeps finding fresh placeholders past this many passes is treated as a cycle.
const MAX_PASSES: usize = 1000;

/// One `interpolate` call: the placeholders being expanded and each finished result.
struct State<'a> {
    vars: &'a Vars,
    tracker: Option<&'a Tracker>,
    generator: &'a RefCell<Generator>,
    /// Placeholders being expanded, so that re-entering one is reported as a cycle.
    stack: Vec<String>,
    /// Bruno's `results` map: each placeholder's final text, an undefined one kept literal.
    results: HashMap<String, String>,
}

impl State<'_> {
    /// Port of Bruno's `replace`: substitute every placeholder, then rescan, because values
    /// can join into new placeholders. A pass whose matches were all memoized ends the loop.
    fn expand(&mut self, text: &str) -> Result<String, ResolveError> {
        let mut current = text.to_owned();
        for _ in 0..MAX_PASSES {
            let mut fresh = false;
            let mut out = String::with_capacity(current.len());
            let mut rest = current.as_str();
            while let Some((start, name, end)) = next_placeholder(rest) {
                out.push_str(&rest[..start]);
                if let Some(done) = self.results.get(name) {
                    out.push_str(done);
                } else {
                    fresh = true;
                    let result = self.resolve(name, &rest[start..end])?;
                    out.push_str(&result);
                    self.results.insert(name.to_owned(), result);
                }
                rest = &rest[end..];
            }
            out.push_str(rest);
            if !fresh {
                return Ok(out);
            }
            current = out;
        }
        let name = next_placeholder(&current).map_or("", |(_, name, _)| name);
        Err(ResolveError::CyclicVariable {
            name: name.to_owned(),
        })
    }

    /// One placeholder's text: its value expanded in turn, or the placeholder itself when undefined.
    fn resolve(&mut self, name: &str, literal: &str) -> Result<String, ResolveError> {
        if self.stack.iter().any(|entered| entered == name) {
            return Err(ResolveError::CyclicVariable {
                name: name.to_owned(),
            });
        }
        let Some(value) = lookup(name, self.vars) else {
            return Ok(literal.to_owned());
        };
        let text = mock(&js::to_text(value), &mut self.generator.borrow_mut());
        self.stack.push(name.to_owned());
        let expanded = self.expand(&text);
        self.stack.pop();
        if let (Some(tracker), Ok(value)) = (self.tracker, &expanded) {
            tracker.note(name, self.vars, value);
        }
        expanded
    }
}

/// Every `{{name}}` in `text` as the resolver finds them, left to right: byte range of the
/// whole placeholder and the name.
pub fn placeholders(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some((start, name, end)) = next_placeholder(&text[from..]) {
        found.push((from + start..from + end, name));
        from += end;
    }
    found
}

/// The name of the first `{{name}` that has one closing brace or none, which the resolver
/// would send as literal text. `None` when every `{{` opens a real placeholder or is not
/// a variable reference at all.
pub fn unclosed_placeholder(text: &str) -> Option<&str> {
    let is_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'$';
    let is_rest = |c: u8| c.is_ascii_alphanumeric() || b"_.$:-".contains(&c);
    let mut from = 0;
    while let Some(offset) = text[from..].find("{{") {
        let start = from + offset;
        // A complete placeholder is the resolver's business; skip past it.
        if let Some((0, _, end)) = next_placeholder(&text[start..]) {
            from = start + end;
            continue;
        }
        // Name characters are ASCII, so the byte length is a valid char boundary.
        let after = &text[start + 2..];
        let bytes = after.as_bytes();
        let len = match bytes.first() {
            Some(&c) if is_start(c) => bytes.iter().take_while(|&&c| is_rest(c)).count(),
            _ => 0,
        };
        let rest = &after[len..];
        if len > 0 && (rest.is_empty() || (rest.starts_with('}') && !rest.starts_with("}}"))) {
            return Some(&after[..len]);
        }
        from = start + 2;
    }
    None
}

/// The first match of Bruno's `/\{\{([^}]+)\}\}/`: its start, name and end in `text`.
pub(crate) fn next_placeholder(text: &str) -> Option<(usize, &str, usize)> {
    let mut from = 0;
    while let Some(offset) = text[from..].find("{{") {
        let start = from + offset;
        let after = &text[start + 2..];
        let run = after.find('}').unwrap_or(after.len());
        if run > 0 && after[run..].starts_with("}}") {
            return Some((start, &after[..run], start + 2 + run + 2));
        }
        // Not a placeholder here; the regex would retry one character later.
        from = start + 1;
    }
    None
}

/// Port of lodash `get`: a literal key wins, otherwise a deep path walks objects and arrays.
pub(crate) fn lookup<'a>(name: &str, vars: &'a Vars) -> Option<&'a Value> {
    if let Some(value) = vars.get(name) {
        return Some(value);
    }
    DEEP_PROP.find(name)?;
    let mut keys = to_path(name).into_iter();
    let mut current = vars.get(&keys.next()?)?;
    for key in keys {
        current = match current {
            Value::Object(map) => map.get(&key)?,
            Value::Array(items) => items.get(canonical_index(&key)?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// The array index `key` spells, if it is canonical.
fn canonical_index(key: &str) -> Option<usize> {
    js::is_canonical_index(key)
        .then(|| key.parse().ok())
        .flatten()
}

/// lodash `reIsDeepProp`: a key with `.` or `[...]` is a path unless the object has it literally.
// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static DEEP_PROP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\.|\[(?:[^[\]]*|(["'])(?:(?!\1)[^\\]|\\.)*?\1)\]"#)
        .expect("lodash reIsDeepProp compiles")
});
/// lodash `rePropName`, as used by `stringToPath`.
// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static PROP_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::with_flags(r#"[^.[\]]+|\[(?:(-?\d+(?:\.\d+)?)|(["'])((?:(?!\2)[^\\]|\\.)*?)\2)\]|(?=(?:\.|\[\])(?:\.|\[\]|$))"#, "g")
        .expect("lodash rePropName compiles")
});
/// lodash `reEscapeChar`: a backslash and the character it escapes inside a quoted key.
// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static ESCAPE_CHAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\(\\)?").expect("lodash reEscapeChar compiles"));

/// Port of lodash `stringToPath`.
pub(crate) fn to_path(path: &str) -> Vec<String> {
    let mut keys = Vec::new();
    if path.starts_with('.') {
        keys.push(String::new());
    }
    for m in PROP_NAME.find_iter(path) {
        let key = match (m.group(1), m.group(2), m.group(3)) {
            (_, Some(_), Some(inner)) => unescape(&path[inner]),
            (Some(number), _, _) => path[number].to_owned(),
            _ => path[m.range()].to_owned(),
        };
        keys.push(key);
    }
    keys
}

/// lodash `subString.replace(reEscapeChar, '$1')`: drop a backslash, keep an escaped one.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for m in ESCAPE_CHAR.find_iter(text) {
        out.push_str(&text[last..m.range().start]);
        if let Some(kept) = m.group(1) {
            out.push_str(&text[kept]);
        }
        last = m.range().end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unclosed_placeholder_names_the_variable() {
        assert_eq!(unclosed_placeholder("http://h/{{host}"), Some("host"));
        assert_eq!(unclosed_placeholder("a {{host"), Some("host"));
        assert_eq!(unclosed_placeholder("{{a}} then {{b.c-d}"), Some("b.c-d"));
    }

    #[test]
    fn text_that_is_not_an_unclosed_placeholder_is_left_alone() {
        for ok in [
            "{{host}}",
            "{{ host}",
            "{{{",
            r#"{"a":{"b":1}}"#,
            r#"{"a":{"b":{"c":1}}}"#,
            "query { a { b } }",
            "{{#each x}}..{{/each}}",
            "{{> partial}}",
            "{{!c}}",
            "{{}",
            "",
        ] {
            assert_eq!(unclosed_placeholder(ok), None, "{ok}");
        }
    }

    fn vars(value: serde_json::Value) -> Vars {
        value.as_object().cloned().expect("object")
    }

    fn sample() -> Vars {
        vars(json!({
            "host": "h",
            "n": 42.0,
            "b": true,
            "z": null,
            "o": {"k": "v", "n": 1.5},
            "a.b": "literal",
            "a": {"b": "nested", "x y": 1, "list": [10, {"port": 8080}]},
            "arr": ["zero", "one"],
            "s": "str",
            "q": "a\"b",
            "echo-host": "https://echo.test",
            "process": {"env": {"a.b": "dotted", "HOME": "/h"}}
        }))
    }

    fn undefined(name: &str) -> ResolveError {
        ResolveError::UndefinedVariable {
            name: name.to_owned(),
            hint: None,
        }
    }

    #[test]
    fn placeholders_render_values_like_bruno() {
        let v = sample();
        assert_eq!(
            interpolate("{{host}}/x?n={{n}}&b={{b}}", &v),
            Ok("h/x?n=42&b=true".to_owned())
        );
        assert_eq!(
            interpolate("{{o}}", &v),
            Ok(r#"{"k":"v","n":1.5}"#.to_owned())
        );
        assert_eq!(interpolate("{{z}}", &v), Ok("null".to_owned()));
        assert_eq!(
            interpolate("{{echo-host}}", &v),
            Ok("https://echo.test".to_owned())
        );
        assert_eq!(
            interpolate("no braces {here}", &v),
            Ok("no braces {here}".to_owned())
        );
        // Never JSON-escaped, as in Bruno (see Decisions on escapeJSONStrings).
        assert_eq!(
            interpolate(r#"{"k":"{{q}}"}"#, &v),
            Ok(r#"{"k":"a"b"}"#.to_owned())
        );
    }

    #[test]
    fn paths_follow_lodash_get() {
        let v = sample();
        let cases = [
            ("{{a.b}}", "literal"),
            ("{{a.list[1].port}}", "8080"),
            ("{{a.list.1.port}}", "8080"),
            ("{{a['x y']}}", "1"),
            ("{{a[\"x y\"]}}", "1"),
            ("{{a[b]}}", "nested"),
            ("{{arr[1]}}", "one"),
            ("{{arr.0}}", "zero"),
            ("{{process.env.HOME}}", "/h"),
            ("{{process.env['a.b']}}", "dotted"),
        ];
        for (text, expected) in cases {
            assert_eq!(interpolate(text, &v), Ok(expected.to_owned()), "{text}");
        }
        for path in [
            "arr[-1]",
            "arr[01]",
            ".a",
            "a..b",
            "z.x",
            "a.list[2]",
            "s.length",
            "arr.length",
            "missing",
        ] {
            assert_eq!(
                interpolate(&format!("{{{{{path}}}}}"), &v),
                Err(undefined(path)),
                "{path}"
            );
        }
    }

    #[test]
    fn placeholder_text_is_not_trimmed_and_needs_content() {
        let v = sample();
        assert_eq!(interpolate("{{ host }}", &v), Err(undefined(" host ")));
        assert_eq!(interpolate("{{{host}}", &v), Err(undefined("{host")));
        assert_eq!(interpolate("{{host}}}", &v), Ok("h}".to_owned()));
        assert_eq!(interpolate("{{}}", &v), Ok("{{}}".to_owned()));
        assert_eq!(interpolate("{{host", &v), Ok("{{host".to_owned()));
        assert_eq!(interpolate("é{{host}}ü", &v), Ok("éhü".to_owned()));
        assert_eq!(interpolate("{{ä}}", &v), Err(undefined("ä")));
        assert_eq!(interpolate("{{$nope}}", &v), Err(undefined("$nope")));
    }

    #[test]
    fn known_dynamic_names_resolve_without_a_variable() {
        let v = sample();
        let out = interpolate("{{$guid}}", &v).expect("resolves");
        assert_eq!(out.len(), 36);
    }

    #[test]
    fn values_resolve_recursively() {
        let v = vars(json!({
            "base": "http://{{host}}",
            "host": "{{h2}}",
            "h2": "example.test",
            "x": "{{y}}",
            "y": 1.0,
            "obj": {"u": "{{h2}}"},
            "deep": "{{missing}}"
        }));
        assert_eq!(
            interpolate("{{base}}/p", &v),
            Ok("http://example.test/p".to_owned())
        );
        assert_eq!(interpolate("{{x}}-{{x}}", &v), Ok("1-1".to_owned()));
        assert_eq!(
            interpolate("{{obj}}", &v),
            Ok(r#"{"u":"example.test"}"#.to_owned())
        );
        assert_eq!(interpolate("{{deep}}", &v), Err(undefined("missing")));
    }

    #[test]
    fn placeholders_split_across_values_are_rescanned() {
        let v = vars(json!({"a": "{{", "b": "host}}", "host": "h"}));
        assert_eq!(interpolate("{{a}}{{b}}", &v), Ok("h".to_owned()));
        let v = vars(json!({"a": "{{", "b": "host}}"}));
        assert_eq!(interpolate("{{a}}{{b}}", &v), Err(undefined("host")));
    }

    #[test]
    fn a_rescan_that_rebuilds_its_own_placeholder_is_a_cycle() {
        let v = vars(json!({"o": "{{", "c": "}}", "n": "{{o}}n{{c}}"}));
        assert_eq!(
            interpolate("{{n}}", &v),
            Err(ResolveError::CyclicVariable {
                name: "n".to_owned()
            })
        );
    }

    #[test]
    fn repeated_placeholders_expand_once() {
        // Without memoizing, this doubling chain would take 2^64 expansions.
        let mut v = Vars::new();
        for i in 0..64 {
            v.insert(
                format!("v{i}"),
                json!(format!("{{{{v{0}}}}}{{{{v{0}}}}}", i + 1)),
            );
        }
        v.insert("v64".to_owned(), json!(""));
        assert_eq!(interpolate("[{{v0}}]", &v), Ok("[]".to_owned()));
    }

    #[test]
    fn a_shared_dependency_is_not_a_cycle() {
        let v = vars(json!({"a": "{{b}}+{{c}}", "b": "{{d}}", "c": "{{d}}", "d": "x"}));
        assert_eq!(interpolate("{{a}}", &v), Ok("x+x".to_owned()));
    }

    #[test]
    fn cycles_fail_naming_the_reentered_variable() {
        let v = vars(json!({"a": "{{b}}", "b": "{{a}}", "s": "x{{s}}"}));
        assert_eq!(
            interpolate("{{a}}", &v),
            Err(ResolveError::CyclicVariable {
                name: "a".to_owned()
            })
        );
        assert_eq!(
            interpolate("{{s}}", &v),
            Err(ResolveError::CyclicVariable {
                name: "s".to_owned()
            })
        );
    }
}
