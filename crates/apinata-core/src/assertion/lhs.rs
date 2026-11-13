//! The left-hand side subset Apinata evaluates without a JavaScript runtime.

use serde_json::{Map, Value, json};

use crate::Response;
use crate::js;

/// One property access after `res`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    /// `.name`
    Dot(String),
    /// `['key']` or `["key"]`
    Key(String),
    /// `[digits]`
    Index(String),
}

/// Names every object inherits from `Object.prototype`; reading one the object lacks needs JavaScript.
const PROTOTYPE_NAMES: [&str; 12] = [
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "__proto__",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];

/// Why an expression was not evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsupported {
    /// It falls outside the subset below and needs a JavaScript runtime.
    NeedsJs,
    /// It reads `res.body` and the body was cut at the size limit.
    Truncated,
}

/// Evaluates a response expression against `res` as Bruno's QuickJS sandbox would.
///
/// `Ok(None)` is `undefined`. Only `res.status`, `res.statusText`, `res.responseTime`,
/// `res.headers.<k>` and `res.body` property chains are supported; anything else is
/// `Err(NeedsJs)`, and any `res.body` chain on a truncated body is `Err(Truncated)`.
pub(crate) fn evaluate_expression(
    expression: &str,
    res: &Response,
) -> Result<Option<Value>, Unsupported> {
    use Unsupported::NeedsJs;
    let segments = parse_expression(expression).ok_or(NeedsJs)?;
    match root(&segments).ok_or(NeedsJs)? {
        Root::Status => Ok(Some(json!(res.status()))),
        Root::StatusText => Ok(Some(json!(res.status_text()))),
        Root::ResponseTime => {
            // Bruno's measureResponseTime ends in Math.round over milliseconds; integer
            // nanoseconds keep the half exact where f64 milliseconds would not.
            let millis = (res.response_time().as_nanos() + 500_000) / 1_000_000;
            Ok(Some(json!(u64::try_from(millis).unwrap_or(u64::MAX))))
        }
        Root::Header(only) => walk(
            Some(Value::Object(res.headers_value())),
            std::slice::from_ref(only),
        )
        .ok_or(NeedsJs),
        Root::Body(_) if res.truncated() => Err(Unsupported::Truncated),
        Root::Body(chain) => walk(Some(res.body_value()), chain).ok_or(NeedsJs),
    }
}

/// Whether `expression` falls inside the evaluated subset, judged without a response; a
/// `res.body` chain may still need JavaScript once the actual body is known.
pub(crate) fn supported_shape(expression: &str) -> bool {
    parse_expression(expression).is_some_and(|segments| root(&segments).is_some())
}

/// The response property an expression starts from, with the accessors that follow it.
enum Root<'a> {
    Status,
    StatusText,
    ResponseTime,
    Header(&'a Segment),
    Body(&'a [Segment]),
}

/// The accessors after a trimmed `res`; `None` when the text is not such a chain.
pub(crate) fn parse_expression(expression: &str) -> Option<Vec<Segment>> {
    js::js_trim(expression)
        .strip_prefix("res")
        .and_then(parse_segments)
}

/// Dispatches on the first accessor; `None` for any root or arity outside the subset.
fn root(segments: &[Segment]) -> Option<Root<'_>> {
    let (Segment::Dot(first), rest) = segments.split_first()? else {
        return None;
    };
    match (first.as_str(), rest) {
        ("status", []) => Some(Root::Status),
        ("statusText", []) => Some(Root::StatusText),
        ("responseTime", []) => Some(Root::ResponseTime),
        ("headers", [only]) => Some(Root::Header(only)),
        ("body", chain) => Some(Root::Body(chain)),
        _ => None,
    }
}

/// Parses the accessors after `res`; `None` when any character falls outside the grammar.
fn parse_segments(mut text: &str) -> Option<Vec<Segment>> {
    let mut segments = Vec::new();
    while !text.is_empty() {
        if let Some(after) = text.strip_prefix('.') {
            let end = after
                .find(|c: char| !is_name_char(c))
                .unwrap_or(after.len());
            let name = &after[..end];
            if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            segments.push(Segment::Dot(name.to_owned()));
            text = &after[end..];
        } else {
            let (inner, rest) = text.strip_prefix('[')?.split_once(']')?;
            segments.push(bracket(inner)?);
            text = rest;
        }
    }
    Some(segments)
}

/// Characters a `.name` accessor may contain.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Characters a quoted key cannot carry (`]` ends the bracket for the parser below).
const KEY_FORBIDDEN: [char; 8] = ['\\', '\'', '"', '\n', '\r', '\u{2028}', '\u{2029}', ']'];

impl Segment {
    /// The accessor for an object key, or `None` when this grammar cannot spell it.
    pub(crate) fn for_key(key: &str) -> Option<Segment> {
        if key == "__proto__" {
            return None;
        }
        let dotted = !key.is_empty()
            && !key.starts_with(|c: char| c.is_ascii_digit())
            && key.chars().all(is_name_char);
        if dotted {
            Some(Segment::Dot(key.to_owned()))
        } else if key.contains(KEY_FORBIDDEN) {
            None
        } else {
            Some(Segment::Key(key.to_owned()))
        }
    }

    /// The accessor for an array position.
    pub(crate) fn for_index(index: usize) -> Segment {
        Segment::Index(index.to_string())
    }

    /// The accessor as source text, appended after the value it reads from.
    pub(crate) fn render(&self) -> String {
        match self {
            Segment::Dot(name) => format!(".{name}"),
            Segment::Key(key) => format!("['{key}']"),
            Segment::Index(digits) => format!("[{digits}]"),
        }
    }
}

/// The contents of one `[...]`: a quoted key without escapes, or a canonical array index.
fn bracket(inner: &str) -> Option<Segment> {
    for quote in ['\'', '"'] {
        if let Some(key) = inner
            .strip_prefix(quote)
            .and_then(|k| k.strip_suffix(quote))
        {
            let plain = !key.contains(KEY_FORBIDDEN);
            return plain.then(|| Segment::Key(key.to_owned()));
        }
    }
    // QuickJS rounds a numeric literal past 15 digits before using it as a key.
    (js::is_canonical_index(inner) && inner.len() <= 15).then(|| Segment::Index(inner.to_owned()))
}

/// Follows `chain` from `value`; the outer `None` means the walk needs JavaScript.
///
/// Reading through `undefined` or `null` stops with QuickJS's TypeError object as the value.
fn walk(mut value: Option<Value>, chain: &[Segment]) -> Option<Option<Value>> {
    for segment in chain {
        let (key, dotted) = match segment {
            Segment::Dot(k) => (k, true),
            Segment::Key(k) | Segment::Index(k) => (k, false),
        };
        value = match value {
            None => return Some(Some(type_error(key, dotted, "undefined"))),
            Some(Value::Null) => return Some(Some(type_error(key, dotted, "null"))),
            // QuickJS turns a `__proto__` key into the object's prototype, so reads need JavaScript.
            Some(Value::Object(map)) if map.contains_key("__proto__") => return None,
            Some(Value::Object(mut map)) => match map.remove(key) {
                Some(found) => Some(found),
                None if PROTOTYPE_NAMES.contains(&key.as_str()) => return None,
                None => None,
            },
            Some(Value::Array(mut items)) => {
                // Only canonical indexes are plain reads on an array; `length` and methods are not.
                if dotted || !js::is_canonical_index(key) {
                    return None;
                }
                let index = key.parse::<usize>().ok().filter(|i| *i < items.len());
                index.map(|i| items.swap_remove(i))
            }
            Some(_) => return None,
        };
    }
    Some(value)
}

/// QuickJS's error for reading a property of `undefined` or `null`, without its `stack`.
fn type_error(key: &str, dotted: bool, of: &str) -> Value {
    let message = if dotted {
        format!("cannot read property '{key}' of {of}")
    } else {
        format!("cannot read property of {of}")
    };
    let mut error = Map::new();
    error.insert("name".to_owned(), json!("TypeError"));
    error.insert("message".to_owned(), json!(message));
    Value::Object(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn response() -> Response {
        let body = r#"{"a":null,"list":[{"id":7}],"k.e y":1,"1":"one","s":"str","toString":3}"#;
        Response::new(404)
            .with_status_text("Not Found")
            .with_header("Content-Type", "application/json")
            .with_header("Set-Cookie", "a=1")
            .with_header("Set-Cookie", "b=2")
            .with_response_time(Duration::from_micros(12_900))
            .with_text_body(body)
    }

    fn eval(expression: &str) -> Option<Value> {
        evaluate_expression(expression, &response())
            .unwrap_or_else(|reason| panic!("{expression} skipped: {reason:?}"))
    }

    fn type_error(message: &str) -> Option<Value> {
        Some(json!({"name": "TypeError", "message": message}))
    }

    #[test]
    fn supported_left_sides_evaluate_as_quickjs_does() {
        // Values probed through QuickJS 0.32.0 with Bruno's response shim on 2026-10-09.
        assert_eq!(eval("res.status"), Some(json!(404)));
        assert_eq!(eval(" res.status "), Some(json!(404)));
        assert_eq!(eval("res.statusText"), Some(json!("Not Found")));
        assert_eq!(eval("res.responseTime"), Some(json!(13)));
        assert_eq!(
            eval("res.headers['content-type']"),
            Some(json!("application/json"))
        );
        assert_eq!(eval("res.headers[\"Content-Type\"]"), None);
        assert_eq!(eval("res.headers.missing"), None);
        assert_eq!(
            eval("res.headers['set-cookie']"),
            Some(json!(["a=1", "b=2"]))
        );
        assert_eq!(eval("res.body.list[0].id"), Some(json!(7)));
        assert_eq!(eval("res.body.list[5]"), None);
        assert_eq!(eval("res.body['k.e y']"), Some(json!(1)));
        assert_eq!(eval("res.body[1]"), Some(json!("one")));
        assert_eq!(eval("res.body['1']"), Some(json!("one")));
        assert_eq!(eval("res.body.toString"), Some(json!(3)));
        assert_eq!(eval("res.body.$x"), None);
        assert_eq!(eval("res.body.s"), Some(json!("str")));
        let whole = eval("res.body").expect("defined");
        assert_eq!(whole["list"], json!([{"id": 7}]));
    }

    #[test]
    fn reading_through_undefined_or_null_yields_the_quickjs_error_object() {
        assert_eq!(
            eval("res.body.list[1].id"),
            type_error("cannot read property 'id' of undefined")
        );
        assert_eq!(
            eval("res.body.list[1][\"id\"]"),
            type_error("cannot read property of undefined")
        );
        assert_eq!(
            eval("res.body.a['x']"),
            type_error("cannot read property of null")
        );
        assert_eq!(
            eval("res.body.a[0]"),
            type_error("cannot read property of null")
        );
        assert_eq!(
            eval("res.body.missing.x.y"),
            type_error("cannot read property 'x' of undefined")
        );
    }

    #[test]
    fn a_text_body_is_a_string() {
        let res = Response::new(200).with_text_body("plain");
        assert_eq!(
            evaluate_expression("res.body", &res),
            Ok(Some(json!("plain")))
        );
        assert!(evaluate_expression("res.body.x", &res).is_err());
    }

    #[test]
    fn everything_else_is_skipped_with_a_reason() {
        let skipped = [
            "",
            "res",
            "res.url",
            "res('a')",
            "res.jq('x')",
            "res.getStatus()",
            "token",
            "bru.getEnvVar('x')",
            "req.url",
            "res.body.list.length",
            "res.body.s.length",
            "res.body.s[0]",
            "res.body.list.foo",
            "res.body.hasOwnProperty",
            "res.body[01]",
            "res.body[-1]",
            "res.body[ 0 ]",
            "res.body['a\\'b']",
            "res.body?.a",
            "res.body.a + 1",
            "res['body']",
            "res.status.x",
            "res.headers",
            "res.headers.a.b",
            "res.headers.toString",
            "res.body.",
            "res.body.1",
            "res.body.list[0",
            "res.status === 404",
            "`${res.status}`",
            "res.body.é",
        ];
        for expression in skipped {
            let reason = evaluate_expression(expression, &response()).expect_err(expression);
            assert_eq!(reason, Unsupported::NeedsJs, "{expression}");
        }
    }

    #[test]
    fn bracket_indexes_quickjs_would_round_are_skipped() {
        assert_eq!(eval("res.body[999999999999999]"), None);
        for expression in [
            "res.body[9999999999999999]",
            "res.body.list[99999999999999999999]",
        ] {
            assert!(
                evaluate_expression(expression, &response()).is_err(),
                "{expression}"
            );
        }
    }

    #[test]
    fn an_object_with_a_proto_key_is_skipped() {
        let res =
            Response::new(200).with_text_body(r#"{"__proto__":{"a":1},"b":2,"c":{"__proto__":1}}"#);
        assert!(evaluate_expression("res.body.b", &res).is_err());
        assert!(evaluate_expression("res.body['__proto__']", &res).is_err());
        let clean = Response::new(200).with_text_body(r#"{"c":{"__proto__":1},"d":2}"#);
        assert_eq!(
            evaluate_expression("res.body.d", &clean),
            Ok(Some(json!(2)))
        );
        assert!(evaluate_expression("res.body.c.x", &clean).is_err());
    }

    #[test]
    fn response_time_rounds_half_up_on_exact_nanoseconds() {
        // Here f64 milliseconds land just below the half and would round down.
        let res = Response::new(200).with_response_time(Duration::from_nanos(1_029_470_500_000));
        assert_eq!(
            evaluate_expression("res.responseTime", &res),
            Ok(Some(json!(1_029_471)))
        );
        let below = Response::new(200).with_response_time(Duration::from_nanos(1_029_470_499_999));
        assert_eq!(
            evaluate_expression("res.responseTime", &below),
            Ok(Some(json!(1_029_470)))
        );
    }
}
