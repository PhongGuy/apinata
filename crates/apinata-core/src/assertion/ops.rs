//! Applying an assertion operator to a left side and its shaped right-hand operand.

use serde_json::Value;

use super::rhs::{Operator, Rhs, Scalar};
use super::text::apply_text;

/// Applies one of Bruno's 28 operators with chai 4.5.0 semantics.
///
/// A pairing chai would throw on, or a right side of the wrong shape, is `false`.
pub(crate) fn apply(op: Operator, lhs: Option<&Value>, rhs: &Rhs) -> bool {
    use Operator::*;
    // The operator is matched on its own so that a new variant fails to compile here.
    match op {
        Eq => matches!(rhs, Rhs::One(want) if want.strict_eq(lhs)),
        Neq => matches!(rhs, Rhs::One(want) if !want.strict_eq(lhs)),
        Gt => compare(lhs, rhs, |a, b| a > b),
        Gte => compare(lhs, rhs, |a, b| a >= b),
        Lt => compare(lhs, rhs, |a, b| a < b),
        Lte => compare(lhs, rhs, |a, b| a <= b),
        In => matches!(rhs, Rhs::List(items) if items.iter().any(|item| item.strict_eq(lhs))),
        NotIn => matches!(rhs, Rhs::List(items) if !items.iter().any(|item| item.strict_eq(lhs))),
        Between => match rhs {
            Rhs::Range(Scalar::Number(low), Scalar::Number(high)) => {
                number(lhs).is_some_and(|n| *low <= n && n <= *high)
            }
            _ => false,
        },
        IsEmpty => size(lhs) == Some(0),
        IsNotEmpty => size(lhs).is_some_and(|n| n > 0),
        IsNull => matches!(lhs, Some(Value::Null)),
        IsUndefined => lhs.is_none(),
        IsDefined => lhs.is_some(),
        IsTruthy => matches!(lhs, Some(Value::Bool(true))),
        IsFalsy => matches!(lhs, Some(Value::Bool(false))),
        IsJson => matches!(lhs, Some(Value::Array(_) | Value::Object(_))),
        IsNumber => matches!(lhs, Some(Value::Number(_))),
        IsString => matches!(lhs, Some(Value::String(_))),
        IsBoolean => matches!(lhs, Some(Value::Bool(_))),
        IsArray => matches!(lhs, Some(Value::Array(_))),
        Contains | NotContains | Length | Matches | NotMatches | StartsWith | EndsWith => {
            apply_text(op, lhs, rhs)
        }
    }
}

/// The left side as a number, when it is one.
fn number(lhs: Option<&Value>) -> Option<f64> {
    lhs?.as_f64()
}

/// chai `above` and friends: both sides must already be numbers, with no coercion.
fn compare(lhs: Option<&Value>, rhs: &Rhs, holds: fn(f64, f64) -> bool) -> bool {
    match (number(lhs), rhs) {
        (Some(n), Rhs::One(Scalar::Number(b))) => holds(n, *b),
        _ => false,
    }
}

/// chai `empty`: the size of a string, array or object; `None` where chai throws.
/// Strings count UTF-8 bytes where chai counts UTF-16 units, which agree on emptiness.
fn size(lhs: Option<&Value>) -> Option<usize> {
    match lhs? {
        Value::String(s) => Some(s.len()),
        Value::Array(items) => Some(items.len()),
        Value::Object(map) => Some(map.len()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertion::rhs::{operand, split};
    use crate::interpolate::Vars;

    /// Bruno's verdict for `res.body.v` holding `lhs` (JSON text, or `undefined`) under `op_value`.
    fn holds(lhs: &str, op_value: &str) -> bool {
        let lhs: Option<Value> =
            (lhs != "undefined").then(|| serde_json::from_str(lhs).expect("lhs is JSON"));
        let (op, text) = split(op_value);
        let rhs = operand(op, &text, &Vars::new(), None).expect("rhs parses");
        apply(op, lhs.as_ref(), &rhs)
    }

    #[test]
    fn equality_comparison_membership_and_unary_operators_match_bruno() {
        // Probed with chai 4.5.0 through the QuickJS path on Node 24.
        let rows: &[(&str, &str, bool)] = &[
            ("200", "eq 200", true),
            ("200", "eq \"200\"", false),
            ("\"200\"", "eq 200", false),
            ("\"abc\"", "eq abc", true),
            ("\"abc\"", "eq \"abc\"", true),
            ("\"abc\"", "eq 'abc'", true),
            ("null", "eq null", true),
            ("undefined", "eq undefined", true),
            ("null", "eq undefined", false),
            ("true", "eq true", true),
            ("\"true\"", "eq true", false),
            ("1000", "eq 1e3", false),
            ("1", "eq 1e3", true),
            ("0", "eq 0x1F", true),
            ("0", "eq -0", true),
            ("1.5", "eq 1.5", true),
            ("{\"a\":1}", "eq {\"a\":1}", false),
            ("[1,2]", "eq [1,2]", false),
            ("\"[1,2]\"", "eq [1,2]", true),
            ("9007199254740992", "eq 9007199254740992", false),
            ("\"9007199254740993\"", "eq 9007199254740993", true),
            ("\"\"", "eq ", true),
            ("\"a b\"", "eq a b", true),
            ("200", "eq  200", true),
            ("\"Infinity\"", "eq Infinity", true),
            ("\"NaN\"", "eq NaN", true),
            ("17", "eq 017", true),
            ("200", "neq 201", true),
            ("200", "neq 200", false),
            ("\"200\"", "neq 200", true),
            ("5", "gt 4", true),
            ("5", "gt 5", false),
            ("5", "gte 5", true),
            ("\"5\"", "gt 4", false),
            ("5", "gt \"4\"", false),
            ("4", "lt 5", true),
            ("5", "lte 5", true),
            ("undefined", "gt 1", false),
            ("5", "gt abc", false),
            ("5", "lt Infinity", false),
            ("5", "gt -Infinity", true),
            ("1.5", "lt 1.5e3", false),
            ("5", "lte NaN", false),
            ("2", "in 1,2,3", true),
            ("2", "in [1, 2, 3]", true),
            ("\"a\"", "in a,b", true),
            ("\"a\"", "in \"a\",\"b\"", true),
            ("\"a,b\"", "in \"a,b\"", false),
            ("4", "notIn 1,2", true),
            ("1", "notIn [1,2]", false),
            ("null", "in null,1", true),
            ("undefined", "in undefined,1", true),
            ("\"2\"", "in 1,2", false),
            ("2", "in []", false),
            ("\"\"", "in []", true),
            ("{\"a\":1}", "notIn x", true),
            ("3", "between 1,5", true),
            ("1", "between 1,5", true),
            ("5", "between 1,5", true),
            ("6", "between 1,5", false),
            ("3", "between [1, 5]", false),
            ("3", "between 1", false),
            ("3", "between 1,5,9", true),
            ("10", "between 1,5,20", false),
            ("\"3\"", "between 1,5", false),
            ("3", "between 5,1", false),
            ("3", "between  1 , 5 ", true),
            ("2.5", "between 2,3", true),
            ("\"\"", "isEmpty", true),
            ("[]", "isEmpty", true),
            ("{}", "isEmpty", true),
            ("\"a\"", "isEmpty", false),
            ("0", "isEmpty", false),
            ("null", "isEmpty", false),
            ("undefined", "isEmpty", false),
            ("false", "isEmpty", false),
            ("\"a\"", "isNotEmpty", true),
            ("\"\"", "isNotEmpty", false),
            ("0", "isNotEmpty", false),
            ("null", "isNotEmpty", false),
            ("undefined", "isNotEmpty", false),
            ("[1]", "isNotEmpty", true),
            ("{\"a\":1}", "isNotEmpty", true),
            ("null", "isNull", true),
            ("undefined", "isNull", false),
            ("0", "isNull", false),
            ("undefined", "isUndefined", true),
            ("null", "isUndefined", false),
            ("null", "isDefined", true),
            ("undefined", "isDefined", false),
            ("0", "isDefined", true),
            ("true", "isTruthy", true),
            ("1", "isTruthy", false),
            ("\"true\"", "isTruthy", false),
            ("false", "isTruthy", false),
            ("false", "isFalsy", true),
            ("0", "isFalsy", false),
            ("null", "isFalsy", false),
            ("\"\"", "isFalsy", false),
            ("{}", "isJson", true),
            ("[]", "isJson", true),
            ("\"x\"", "isJson", false),
            ("null", "isJson", false),
            ("undefined", "isJson", false),
            ("1", "isJson", false),
            ("1", "isNumber", true),
            ("\"1\"", "isNumber", false),
            ("null", "isNumber", false),
            ("\"1\"", "isString", true),
            ("1", "isString", false),
            ("false", "isBoolean", true),
            ("\"false\"", "isBoolean", false),
            ("[]", "isArray", true),
            ("{}", "isArray", false),
            ("null", "isArray", false),
            ("null", "isNull x", true),
            ("200", "foo 200", false),
            ("\"foo 200\"", "foo 200", true),
            ("200", "eq\t200", false),
        ];
        let wrong: Vec<_> = rows
            .iter()
            .filter(|(lhs, op, want)| holds(lhs, op) != *want)
            .collect();
        assert!(
            wrong.is_empty(),
            "rows where Apinata disagrees with Bruno: {wrong:?}"
        );
    }

    #[test]
    fn text_operators_are_dispatched() {
        assert!(holds("\"abc\"", "contains b"));
        assert!(holds("[1,2]", "length 2"));
        assert!(holds("\"abc\"", "matches ^a"));
        assert!(holds("\"abc\"", "startsWith a"));
        assert!(holds("\"abc\"", "endsWith c"));
        assert!(!holds("\"abc\"", "notContains b"));
    }

    #[test]
    fn a_rhs_of_the_wrong_shape_never_passes() {
        let one = Rhs::One(Scalar::Number(1.0));
        let lhs = serde_json::json!(1);
        assert!(!apply(Operator::Eq, Some(&lhs), &Rhs::None));
        assert!(!apply(Operator::In, Some(&lhs), &one));
        assert!(!apply(Operator::Between, Some(&lhs), &one));
        assert!(!apply(Operator::Matches, Some(&lhs), &one));
    }
}
