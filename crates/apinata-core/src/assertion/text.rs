//! Text assertion operators: contains, length, matches, startsWith and endsWith.

use serde_json::Value;

use super::rhs::{Operator, Rhs, Scalar};
use crate::js;

/// JavaScript `String(value)`: arrays join their items with commas, with `null` items empty.
pub(crate) fn to_js_string(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    to_js_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        other => js::to_text(other),
    }
}

/// Applies a text operator with chai 4.5.0 and chai-string 1.5.0 semantics; a pairing chai would
/// throw on, and any non-text operator, is `false`.
pub(crate) fn apply_text(op: Operator, lhs: Option<&Value>, rhs: &Rhs) -> bool {
    match (op, rhs) {
        (Operator::Contains, Rhs::One(needle)) => includes(lhs, needle).unwrap_or(false),
        (Operator::NotContains, Rhs::One(needle)) => {
            includes(lhs, needle).is_some_and(|found| !found)
        }
        (Operator::Length, Rhs::One(want)) => length_of(lhs, want),
        (Operator::Matches, Rhs::Pattern(pattern)) => matches(lhs, pattern).unwrap_or(false),
        (Operator::NotMatches, Rhs::Pattern(pattern)) => {
            matches(lhs, pattern).is_some_and(|found| !found)
        }
        (Operator::StartsWith, Rhs::One(Scalar::String(prefix))) => {
            matches!(lhs, Some(Value::String(s)) if s.starts_with(prefix.as_str()))
        }
        (Operator::EndsWith, Rhs::One(Scalar::String(suffix))) => {
            matches!(lhs, Some(Value::String(s)) if s.ends_with(suffix.as_str()))
        }
        _ => false,
    }
}

/// chai `include`: substring search on strings, `===` membership on arrays; `None` where chai
/// throws, which fails both the assertion and its negation.
fn includes(lhs: Option<&Value>, needle: &Scalar) -> Option<bool> {
    match lhs? {
        Value::String(s) => Some(s.contains(needle.to_js_string().as_str())),
        Value::Array(items) => Some(items.iter().any(|item| needle.strict_eq(Some(item)))),
        _ => None,
    }
}

/// chai `lengthOf`: strings count UTF-16 units, arrays their items, objects their `length` key,
/// and the count is compared with loose `==`.
fn length_of(lhs: Option<&Value>, want: &Scalar) -> bool {
    match lhs {
        Some(Value::String(s)) => loose_eq(&Value::from(s.encode_utf16().count()), want),
        Some(Value::Array(items)) => loose_eq(&Value::from(items.len()), want),
        Some(Value::Object(map)) => map.get("length").is_some_and(|len| loose_eq(len, want)),
        _ => false,
    }
}

/// JavaScript `==` between a JSON value and a scalar; arrays and objects compare as their string.
fn loose_eq(left: &Value, right: &Scalar) -> bool {
    match (left, right) {
        (Value::Null, Scalar::Null | Scalar::Undefined) => true,
        (Value::Null, _) | (_, Scalar::Null | Scalar::Undefined) => false,
        (Value::Array(_) | Value::Object(_), _) => {
            loose_eq(&Value::String(to_js_string(left)), right)
        }
        (Value::String(a), Scalar::String(b)) => a == b,
        // The remaining pairs mix numbers, booleans and strings, which `==` compares as numbers.
        _ => to_number(left) == scalar_number(right),
    }
}

/// JavaScript `Number(v)` for a primitive JSON value.
fn to_number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) => js::parse_number(s).unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// JavaScript `Number(v)` for a right-hand scalar.
fn scalar_number(value: &Scalar) -> f64 {
    match value {
        Scalar::Number(n) => *n,
        Scalar::Bool(b) => f64::from(u8::from(*b)),
        Scalar::String(s) => js::parse_number(s).unwrap_or(f64::NAN),
        Scalar::Null => 0.0,
        Scalar::Undefined => f64::NAN,
    }
}

/// Bruno's `match`: `new RegExp(pattern).test(String(lhs))` over UTF-16 code units, as JavaScript
/// matches; `undefined` never matches, and `None` is a pattern that does not compile.
fn matches(lhs: Option<&Value>, pattern: &str) -> Option<bool> {
    let regex = regress::Regex::new(pattern).ok()?;
    let Some(lhs) = lhs else { return Some(false) };
    let units: Vec<u16> = to_js_string(lhs).encode_utf16().collect();
    Some(regex.find_from_ucs2(&units, 0).next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertion::rhs::{operand, split};
    use crate::interpolate::Vars;
    use serde_json::json;

    /// Bruno's verdict for `res.body.v` holding `lhs` (JSON text, or `undefined`) under `op_value`.
    fn holds(lhs: &str, op_value: &str) -> bool {
        let lhs: Option<Value> =
            (lhs != "undefined").then(|| serde_json::from_str(lhs).expect("lhs is JSON"));
        let (op, text) = split(op_value);
        let rhs = operand(op, &text, &Vars::new(), None).expect("rhs parses");
        apply_text(op, lhs.as_ref(), &rhs)
    }

    fn disagreements<'a>(rows: &[(&'a str, &'a str, bool)]) -> Vec<(&'a str, &'a str, bool)> {
        rows.iter()
            .copied()
            .filter(|(lhs, op, want)| holds(lhs, op) != *want)
            .collect()
    }

    #[test]
    fn text_operators_match_bruno_verdicts() {
        // Probed with chai 4.5.0 and chai-string 1.5.0 through the QuickJS path on Node 24.
        let rows: &[(&str, &str, bool)] = &[
            ("\"abc\"", "contains b", true),
            ("\"abc\"", "contains x", false),
            ("\"abc\"", "notContains x", true),
            ("\"a1c\"", "contains 1", true),
            ("\"null\"", "contains null", true),
            ("[1,2]", "contains 2", true),
            ("[1,2]", "contains \"2\"", false),
            ("[\"a\"]", "contains a", true),
            ("{\"a\":1}", "contains a", false),
            ("{\"a\":1}", "notContains a", false),
            ("5", "contains 5", false),
            ("5", "notContains 5", false),
            ("undefined", "contains x", false),
            ("undefined", "notContains x", false),
            ("null", "notContains x", false),
            ("\"abc\"", "contains ", true),
            ("\"a0c\"", "contains -0", true),
            ("\"abc\"", "length 3", true),
            ("\"abc\"", "length 2", false),
            ("[1,2]", "length 2", true),
            ("\"😀\"", "length 2", true),
            ("\"😀\"", "length 1", false),
            ("\"abc\"", "length \"3\"", true),
            ("5", "length 1", false),
            ("undefined", "length 0", false),
            ("null", "length 0", false),
            ("{}", "length 0", false),
            ("{\"length\":3}", "length 3", true),
            ("{\"length\":\"3\"}", "length 3", true),
            ("true", "length 1", false),
            ("\"\"", "length 0", true),
            ("\"abc\"", "length abc", false),
            ("\"abc\"", "matches ^a", true),
            ("\"abc\"", "matches /^a/", true),
            ("\"abc\"", "matches ^x", false),
            ("\"abc\"", "notMatches ^x", true),
            ("\"abc\"", "notMatches ^a", false),
            ("123", "matches ^1", true),
            ("null", "matches null", true),
            ("undefined", "matches .*", false),
            ("undefined", "notMatches .*", true),
            ("true", "matches ^tr", true),
            ("[1,2]", "matches ^1,2$", true),
            ("{\"a\":1}", "matches object", true),
            ("\"ABC\"", "matches /abc/i", false),
            ("\"abc\"", "matches (", false),
            ("\"abc\"", "notMatches (", false),
            ("\"😀\"", "matches ^.$", false),
            ("\"😀\"", "matches ^..$", true),
            ("\"abc\"", "matches /", false),
            ("\"a b\"", "matches a b", true),
            ("[null,1]", "matches ^,1$", true),
            ("[[1,2],3]", "matches ^1,2,3$", true),
            ("1.5", "matches ^1\\.5$", true),
            ("\"abc\"", "startsWith a", true),
            ("\"abc\"", "startsWith x", false),
            ("\"123\"", "startsWith 1", false),
            ("\"123\"", "startsWith \"1\"", true),
            ("123", "startsWith 1", false),
            ("\"abc\"", "endsWith c", true),
            ("\"abc\"", "endsWith \"bc\"", true),
            ("undefined", "startsWith a", false),
            ("\"abc\"", "startsWith ab c", false),
            ("\"abc\"", "endsWith ", true),
            ("\"abc\"", "endsWith x", false),
        ];
        assert_eq!(
            disagreements(rows),
            [],
            "rows where Apinata disagrees with Bruno"
        );
    }

    #[test]
    fn regex_matching_counts_utf16_code_units_like_javascript() {
        // `new RegExp(p).test(t)` on Node 24; regress over UTF-8 disagrees on the first five.
        let rows: &[(&str, &str, bool)] = &[
            ("^.$", "😀", false),
            ("^..$", "😀", true),
            ("^[^a]$", "😀", false),
            ("^\\uD83D", "😀", true),
            ("^\\S$", "😀", false),
            ("^[😀]$", "😀", false),
            ("é", "café", true),
            ("^\\w+$", "abc", true),
            ("^a.c$", "a\nc", false),
            ("^$", "", true),
            ("\\d{2}", "a12", true),
        ];
        for (pattern, text, want) in rows {
            let rhs = Rhs::Pattern((*pattern).to_owned());
            assert_eq!(
                apply_text(Operator::Matches, Some(&json!(text)), &rhs),
                *want,
                "/{pattern}/ on {text:?}"
            );
            assert_eq!(
                apply_text(Operator::NotMatches, Some(&json!(text)), &rhs),
                !*want,
                "/{pattern}/ on {text:?}"
            );
        }
    }

    #[test]
    fn values_convert_to_strings_like_javascript() {
        let rows = [
            (json!(null), "null"),
            (json!(true), "true"),
            (json!(1.5), "1.5"),
            (json!(1e21), "1e+21"),
            (json!("s"), "s"),
            (json!([1, null, [2, 3], "x"]), "1,,2,3,x"),
            (json!({"a": 1}), "[object Object]"),
            (json!([{"a": 1}]), "[object Object]"),
        ];
        for (value, want) in rows {
            assert_eq!(to_js_string(&value), want, "{value}");
        }
    }

    #[test]
    fn length_converts_an_array_or_object_length_to_a_primitive_first() {
        // JavaScript `==` turns an object operand into its string before comparing.
        let rows: &[(&str, &str, bool)] = &[
            ("{\"length\":[3]}", "length 3", true),
            ("{\"length\":[]}", "length 0", true),
            ("{\"length\":[1,2]}", "length 3", false),
            ("{\"length\":{}}", "length [object Object]", true),
            ("{\"length\":null}", "length 0", false),
        ];
        assert_eq!(
            disagreements(rows),
            [],
            "rows where Apinata disagrees with JavaScript"
        );
    }
}
