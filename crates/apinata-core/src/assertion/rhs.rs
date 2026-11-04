//! The right-hand side of an assertion: its operator and literal value.

use serde_json::Value;

use crate::ResolveError;
use crate::interpolate::{Vars, interpolate};
use crate::js;
use crate::script::expr::{ExprError, Expressions};

/// One of Bruno's 28 assertion operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operator {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    NotIn,
    Contains,
    NotContains,
    Length,
    Matches,
    NotMatches,
    StartsWith,
    EndsWith,
    Between,
    IsEmpty,
    IsNotEmpty,
    IsNull,
    IsUndefined,
    IsDefined,
    IsTruthy,
    IsFalsy,
    IsJson,
    IsNumber,
    IsString,
    IsBoolean,
    IsArray,
}

/// Operator names as Bruno spells them in the `operator value` text.
const NAMES: [(&str, Operator); 28] = [
    ("eq", Operator::Eq),
    ("neq", Operator::Neq),
    ("gt", Operator::Gt),
    ("gte", Operator::Gte),
    ("lt", Operator::Lt),
    ("lte", Operator::Lte),
    ("in", Operator::In),
    ("notIn", Operator::NotIn),
    ("contains", Operator::Contains),
    ("notContains", Operator::NotContains),
    ("length", Operator::Length),
    ("matches", Operator::Matches),
    ("notMatches", Operator::NotMatches),
    ("startsWith", Operator::StartsWith),
    ("endsWith", Operator::EndsWith),
    ("between", Operator::Between),
    ("isEmpty", Operator::IsEmpty),
    ("isNotEmpty", Operator::IsNotEmpty),
    ("isNull", Operator::IsNull),
    ("isUndefined", Operator::IsUndefined),
    ("isDefined", Operator::IsDefined),
    ("isTruthy", Operator::IsTruthy),
    ("isFalsy", Operator::IsFalsy),
    ("isJson", Operator::IsJson),
    ("isNumber", Operator::IsNumber),
    ("isString", Operator::IsString),
    ("isBoolean", Operator::IsBoolean),
    ("isArray", Operator::IsArray),
];

impl Operator {
    /// The operator Bruno spells `name`, matched case-sensitively.
    pub(crate) fn from_name(name: &str) -> Option<Operator> {
        NAMES.iter().find(|(n, _)| *n == name).map(|(_, op)| *op)
    }

    /// Whether the operator takes no right-hand value.
    pub(crate) fn is_unary(self) -> bool {
        use Operator::*;
        matches!(
            self,
            IsEmpty
                | IsNotEmpty
                | IsNull
                | IsUndefined
                | IsDefined
                | IsTruthy
                | IsFalsy
                | IsJson
                | IsNumber
                | IsString
                | IsBoolean
                | IsArray
        )
    }
}

/// Splits `operator value` as Bruno's `parseAssertionOperator` does (assert-runtime.js:235-310).
///
/// Only a single space separates the operator. Unary operators drop their value, and text that
/// does not start with a known operator is an `eq` against the whole, untrimmed text.
pub(crate) fn split(text: &str) -> (Operator, String) {
    if text.is_empty() {
        return (Operator::Eq, String::new());
    }
    let trimmed = js::js_trim(text);
    let (name, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
    match Operator::from_name(name) {
        Some(op) if op.is_unary() => (op, String::new()),
        Some(op) => (op, rest.to_owned()),
        None => (Operator::Eq, text.to_owned()),
    }
}

/// A JavaScript primitive produced by a right-hand literal; `undefined`, `NaN` and `-0` survive.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Scalar {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
}

impl Scalar {
    /// JavaScript `===` against a left side, where `None` is `undefined`; objects never match.
    pub(crate) fn strict_eq(&self, value: Option<&Value>) -> bool {
        match (self, value) {
            (Scalar::Undefined, None) => true,
            (Scalar::Null, Some(Value::Null)) => true,
            (Scalar::Bool(a), Some(Value::Bool(b))) => a == b,
            (Scalar::Number(a), Some(Value::Number(b))) => b.as_f64() == Some(*a),
            (Scalar::String(a), Some(Value::String(b))) => a == b,
            _ => false,
        }
    }

    /// JavaScript `String(value)`.
    pub(crate) fn to_js_string(&self) -> String {
        match self {
            Scalar::Undefined => "undefined".to_owned(),
            Scalar::Null => "null".to_owned(),
            Scalar::Bool(b) => b.to_string(),
            Scalar::Number(n) => js::number_to_string(*n),
            Scalar::String(s) => s.clone(),
        }
    }
}

/// The right side needs a JavaScript runtime to evaluate, so the assertion is skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeedsJs;

/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Evaluates a right-hand literal as bruno-cli's `executeQuickJsVm` template-literal path does
/// (sandbox/quickjs/index.js:84-120, the entry point at line 96).
///
/// Numbers come first, then the four keywords, then one pair of outer quotes is stripped and the
/// rest becomes a template literal. Only template text with no escapes or substitutions is a
/// plain string here; anything else needs the JavaScript runtime.
pub(crate) fn literal(text: &str) -> Result<Scalar, NeedsJs> {
    if text.is_empty() {
        return Ok(Scalar::String(String::new()));
    }
    let text = js::js_trim(text);
    if let Some(n) = js::parse_number(text) {
        // Bruno returns unsafe integers as text so they are not rounded, and reparses integral
        // values with parseInt, which is why `1e3` is 1 and `0x1F` is 0.
        if n > MAX_SAFE_INTEGER {
            return Ok(Scalar::String(text.to_owned()));
        }
        if n.fract() == 0.0 {
            return Ok(Scalar::Number(parse_int(text)));
        }
        return Ok(Scalar::Number(n));
    }
    match text {
        "true" => return Ok(Scalar::Bool(true)),
        "false" => return Ok(Scalar::Bool(false)),
        "null" => return Ok(Scalar::Null),
        "undefined" => return Ok(Scalar::Undefined),
        _ => {}
    }
    let text = remove_quotes(text);
    if text.contains(['`', '\\']) || text.contains("${") {
        return Err(NeedsJs);
    }
    // Template literals normalise CR and CRLF line terminators to LF.
    Ok(Scalar::String(
        text.replace("\r\n", "\n").replace('\r', "\n"),
    ))
}

/// JavaScript `parseInt(text, 10)` on already trimmed text: a sign, then the leading digit run.
fn parse_int(text: &str) -> f64 {
    let (negative, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let end = unsigned
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(unsigned.len());
    let magnitude = match unsigned[..end].parse::<f64>() {
        Ok(m) if end > 0 => m,
        _ => f64::NAN,
    };
    if negative { -magnitude } else { magnitude }
}

/// Bruno's `removeQuotes`: strips one pair of matching outer `"` or `'`, so a lone quote is empty.
fn remove_quotes(text: &str) -> &str {
    for quote in ['"', '\''] {
        if text.starts_with(quote) && text.ends_with(quote) {
            return text.get(1..text.len().saturating_sub(1)).unwrap_or("");
        }
    }
    text
}

/// A right-hand operand shaped for its operator.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Rhs {
    /// A unary operator takes no operand.
    None,
    /// The single value of every other binary operator.
    One(Scalar),
    /// The candidates of `in` and `notIn`.
    List(Vec<Scalar>),
    /// The bounds of `between`; a missing bound is `Undefined`.
    Range(Scalar, Scalar),
    /// The regular expression source of `matches` and `notMatches`.
    Pattern(String),
}

/// Why a right-hand operand could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RhsError {
    NeedsJs,
    /// The JavaScript template of a right-side item threw or was unsupported.
    Script(ExprError),
    Variable(ResolveError),
}

impl From<NeedsJs> for RhsError {
    fn from(_: NeedsJs) -> Self {
        RhsError::NeedsJs
    }
}

impl From<ResolveError> for RhsError {
    fn from(e: ResolveError) -> Self {
        RhsError::Variable(e)
    }
}

/// Builds the operand as Bruno's `evaluateRhsOperand` does (assert-runtime.js:355-403).
///
/// Lists and ranges split on commas before interpolating, so a variable holding a comma stays
/// one item; patterns are interpolated but never trimmed or parsed as literals.
///
/// With `scripted`, an item outside the literal subset runs as a template literal, as the
/// reference does once its quotes are removed.
pub(crate) fn operand(
    op: Operator,
    text: &str,
    vars: &Vars,
    scripted: Option<&Expressions>,
) -> Result<Rhs, RhsError> {
    // Under an approved set only the approved raw text may run, so an item whose variables
    // changed it (and could have injected code) is skipped rather than run.
    let scalar = |raw: &str, value: String| -> Result<Scalar, RhsError> {
        match (literal(&value), scripted) {
            (Err(NeedsJs), Some(scripted))
                if !scripted.allows(text) || (scripted.restricted() && value != raw) =>
            {
                Err(RhsError::Script(ExprError::Unsupported(
                    crate::post_response::not_approved_reason(text),
                )))
            }
            (Err(NeedsJs), Some(scripted)) => scripted
                .template(remove_quotes(js::js_trim(&value)))
                .map(Scalar::String)
                .map_err(RhsError::Script),
            (result, _) => Ok(result?),
        }
    };
    let item = |raw: &str| {
        let raw = js::js_trim(raw);
        scalar(raw, interpolate(raw, vars)?)
    };
    match op {
        _ if op.is_unary() => Ok(Rhs::None),
        Operator::In | Operator::NotIn => {
            let inner = strip_pair(text, '[', ']');
            Ok(Rhs::List(
                inner.split(',').map(item).collect::<Result<_, _>>()?,
            ))
        }
        Operator::Between => {
            let mut bounds = text
                .split(',')
                .map(item)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter();
            let low = bounds.next().unwrap_or(Scalar::Undefined);
            let high = bounds.next().unwrap_or(Scalar::Undefined);
            Ok(Rhs::Range(low, high))
        }
        Operator::Matches | Operator::NotMatches => {
            Ok(Rhs::Pattern(interpolate(strip_pair(text, '/', '/'), vars)?))
        }
        _ => Ok(Rhs::One(scalar(text, interpolate(text, vars)?)?)),
    }
}

/// Whether [`operand`] would run JavaScript for the raw `text` before any interpolation,
/// judging each item as it splits them.
pub(crate) fn needs_script(op: Operator, text: &str) -> bool {
    let needs_js = |raw: &str| literal(js::js_trim(raw)).is_err();
    match op {
        _ if op.is_unary() => false,
        Operator::In | Operator::NotIn => strip_pair(text, '[', ']').split(',').any(needs_js),
        Operator::Between => text.split(',').any(needs_js),
        Operator::Matches | Operator::NotMatches => false,
        _ => needs_js(text),
    }
}

/// Strips an outer `open`…`close` pair; a single character is kept, as JavaScript's
/// `substring(1, 0)` swaps its arguments.
fn strip_pair(text: &str, open: char, close: char) -> &str {
    if text.len() >= 2 && text.starts_with(open) && text.ends_with(close) {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One literal's result as `kind value`, with `-0` and `NaN` spelled out.
    fn show(text: &str) -> String {
        match literal(text) {
            Err(NeedsJs) => "skip".to_owned(),
            Ok(Scalar::Undefined) => "undefined".to_owned(),
            Ok(Scalar::Null) => "null".to_owned(),
            Ok(Scalar::Bool(b)) => b.to_string(),
            Ok(Scalar::Number(n)) if n == 0.0 && n.is_sign_negative() => "number -0".to_owned(),
            Ok(Scalar::Number(n)) => format!("number {}", js::number_to_string(n)),
            Ok(Scalar::String(s)) => format!("string {s}"),
        }
    }

    fn num(n: f64) -> Scalar {
        Scalar::Number(n)
    }

    fn st(s: &str) -> Scalar {
        Scalar::String(s.to_owned())
    }

    #[test]
    fn literals_follow_the_quickjs_template_literal_path() {
        // Probed through executeQuickJsVm's pre-checks (sandbox/quickjs/index.js:96-120) on Node 24.
        let rows: &[(&str, &str)] = &[
            ("true", "true"),
            ("false", "false"),
            ("null", "null"),
            ("undefined", "undefined"),
            ("\"hello\"", "string hello"),
            ("'world'", "string world"),
            ("\"42\"", "string 42"),
            ("42", "number 42"),
            ("3.14", "number 3.14"),
            ("  42  ", "number 42"),
            ("9007199254740991", "number 9007199254740991"),
            ("9007199254740992", "string 9007199254740992"),
            ("9007199254740993", "string 9007199254740993"),
            ("-9007199254740993", "number -9007199254740992"),
            ("1e3", "number 1"),
            ("1.5e3", "number 1"),
            ("0x1F", "number 0"),
            ("0X1f", "number 0"),
            ("0b11", "number 0"),
            ("0o17", "number 0"),
            ("017", "number 17"),
            ("00012", "number 12"),
            ("1.0", "number 1"),
            ("-0", "number -0"),
            ("+5", "number 5"),
            (".5", "number 0.5"),
            ("5.", "number 5"),
            (".0", "number NaN"),
            ("+.5e1", "number NaN"),
            ("1e-3", "number 0.001"),
            ("1.5e-7", "number 1.5e-7"),
            ("-5", "number -5"),
            ("-1e300", "number -1"),
            ("1e300", "string 1e300"),
            ("Infinity", "string Infinity"),
            ("Infinity ", "string Infinity"),
            ("-Infinity", "number -Infinity"),
            ("NaN", "string NaN"),
            ("9007199254740991.5", "string 9007199254740991.5"),
            ("99999999999999999999", "string 99999999999999999999"),
            ("-99999999999999999999", "number -100000000000000000000"),
            ("+0x10", "string +0x10"),
            ("-0x10", "string -0x10"),
            ("1_000", "string 1_000"),
            ("1,5", "string 1,5"),
            ("123abc", "string 123abc"),
            ("12 34", "string 12 34"),
            ("abc", "string abc"),
            ("hello world", "string hello world"),
            ("{\"a\":1}", "string {\"a\":1}"),
            ("[1,2]", "string [1,2]"),
            ("TRUE", "string TRUE"),
            (" null ", "null"),
            ("\"null\"", "string null"),
            ("\"\"", "string "),
            ("\"", "string "),
            ("'", "string "),
            ("'\"'", "string \""),
            ("\"a", "string \"a"),
            ("  \"x\"  ", "string x"),
            ("\t7\t", "number 7"),
            ("\u{a0}7", "number 7"),
            ("   ", "number NaN"),
            ("", "string "),
            ("a\r\nb", "string a\nb"),
            ("\"a\rb\"", "string a\nb"),
            ("$", "string $"),
            ("{", "string {"),
            ("$ {x}", "string $ {x}"),
            ("${1+1}", "skip"),
            ("\"${1+1}\"", "skip"),
            ("a`b", "skip"),
            ("a\\b", "skip"),
        ];
        let wrong: Vec<_> = rows
            .iter()
            .map(|(t, want)| (*t, show(t), *want))
            .filter(|(_, got, want)| got != want)
            .collect();
        assert!(wrong.is_empty(), "(input, got, want): {wrong:?}");
    }

    #[test]
    fn operator_text_splits_like_parse_assertion_operator() {
        let rows: &[(&str, Operator, &str)] = &[
            ("eq 200", Operator::Eq, "200"),
            ("eq  200", Operator::Eq, " 200"),
            ("  eq 200  ", Operator::Eq, "200"),
            ("matches a b", Operator::Matches, "a b"),
            ("in", Operator::In, ""),
            ("eq", Operator::Eq, ""),
            ("isNull x", Operator::IsNull, ""),
            ("isArray", Operator::IsArray, ""),
            ("foo 1", Operator::Eq, "foo 1"),
            (" foo ", Operator::Eq, " foo "),
            ("eq\t200", Operator::Eq, "eq\t200"),
            ("", Operator::Eq, ""),
        ];
        for (text, op, value) in rows {
            assert_eq!(split(text), (*op, (*value).to_owned()), "{text:?}");
        }
        let unary = NAMES.iter().filter(|(_, op)| op.is_unary()).count();
        assert_eq!((NAMES.len(), unary), (28, 12));
        assert_eq!(Operator::from_name("notIn"), Some(Operator::NotIn));
        assert_eq!(Operator::from_name("NotIn"), None);
    }

    #[test]
    fn strict_equality_and_string_conversion_follow_javascript() {
        assert!(num(200.0).strict_eq(Some(&json!(200))));
        assert!(num(-0.0).strict_eq(Some(&json!(0))));
        assert!(!num(f64::NAN).strict_eq(Some(&json!(0))));
        assert!(!st("200").strict_eq(Some(&json!(200))));
        assert!(Scalar::Undefined.strict_eq(None));
        assert!(!Scalar::Null.strict_eq(None));
        assert!(!st("[1]").strict_eq(Some(&json!([1]))));
        assert!(!st("{}").strict_eq(Some(&json!({}))));
        let strings: Vec<String> = [
            Scalar::Undefined,
            Scalar::Null,
            Scalar::Bool(true),
            num(-0.0),
            num(f64::NAN),
            num(f64::NEG_INFINITY),
            num(1e21),
            st("a"),
        ]
        .iter()
        .map(Scalar::to_js_string)
        .collect();
        assert_eq!(
            strings,
            [
                "undefined",
                "null",
                "true",
                "0",
                "NaN",
                "-Infinity",
                "1e+21",
                "a"
            ]
        );
    }

    fn vars(value: Value) -> Vars {
        value.as_object().cloned().expect("an object")
    }

    #[test]
    fn lists_split_on_commas_before_each_item_is_interpolated() {
        let v = vars(json!({"two": 2, "pair": "x,y", "q": "\"s\""}));
        let list = |text: &str| operand(Operator::In, text, &v, None);
        assert_eq!(
            list("1,2,3"),
            Ok(Rhs::List(vec![num(1.0), num(2.0), num(3.0)]))
        );
        assert_eq!(
            list("[1, {{two}}]"),
            Ok(Rhs::List(vec![num(1.0), num(2.0)]))
        );
        assert_eq!(list("\"a,b\""), Ok(Rhs::List(vec![st("\"a"), st("b\"")])));
        assert_eq!(list("[]"), Ok(Rhs::List(vec![st("")])));
        assert_eq!(list("{{pair}}"), Ok(Rhs::List(vec![st("x,y")])));
        assert_eq!(
            list("{{q}}, null"),
            Ok(Rhs::List(vec![st("s"), Scalar::Null]))
        );
        assert_eq!(
            operand(Operator::NotIn, "[a]", &v, None),
            Ok(Rhs::List(vec![st("a")]))
        );
    }

    #[test]
    fn between_keeps_brackets_and_only_two_bounds() {
        let v = Vars::new();
        let range = |text: &str| operand(Operator::Between, text, &v, None);
        assert_eq!(range("1,5"), Ok(Rhs::Range(num(1.0), num(5.0))));
        assert_eq!(range(" 1 , 5 "), Ok(Rhs::Range(num(1.0), num(5.0))));
        assert_eq!(range("[1, 5]"), Ok(Rhs::Range(st("[1"), st("5]"))));
        assert_eq!(range("1"), Ok(Rhs::Range(num(1.0), Scalar::Undefined)));
        assert_eq!(range("1,5,9"), Ok(Rhs::Range(num(1.0), num(5.0))));
        assert_eq!(range(""), Ok(Rhs::Range(st(""), Scalar::Undefined)));
    }

    #[test]
    fn patterns_strip_slashes_and_are_interpolated_but_not_trimmed() {
        let v = vars(json!({"p": "x+"}));
        let pattern = |text: &str| operand(Operator::Matches, text, &v, None);
        assert_eq!(pattern("/^a/"), Ok(Rhs::Pattern("^a".to_owned())));
        assert_eq!(pattern("/abc/i"), Ok(Rhs::Pattern("/abc/i".to_owned())));
        assert_eq!(pattern("/"), Ok(Rhs::Pattern("/".to_owned())));
        assert_eq!(pattern("//"), Ok(Rhs::Pattern(String::new())));
        assert_eq!(pattern(" ^{{p}}"), Ok(Rhs::Pattern(" ^x+".to_owned())));
        assert_eq!(
            operand(Operator::NotMatches, "/{{p}}/", &v, None),
            Ok(Rhs::Pattern("x+".to_owned()))
        );
    }

    #[test]
    fn single_values_interpolate_then_parse() {
        let v = vars(json!({"n": 42, "q": "\"x\"", "t": "${1}", "ws": "  "}));
        assert_eq!(
            operand(Operator::Eq, "{{n}}", &v, None),
            Ok(Rhs::One(num(42.0)))
        );
        assert_eq!(
            operand(Operator::Eq, "{{q}}", &v, None),
            Ok(Rhs::One(st("x")))
        );
        assert!(
            matches!(operand(Operator::Eq, "{{ws}}", &v, None), Ok(Rhs::One(Scalar::Number(n))) if n.is_nan())
        );
        assert_eq!(
            operand(Operator::Eq, "{{t}}", &v, None),
            Err(RhsError::NeedsJs)
        );
        assert_eq!(
            operand(Operator::Gt, "${1+1}", &v, None),
            Err(RhsError::NeedsJs)
        );
        let missing = ResolveError::UndefinedVariable {
            name: "nope".to_owned(),
            hint: None,
        };
        assert_eq!(
            operand(Operator::Eq, "{{nope}}", &v, None),
            Err(RhsError::Variable(missing.clone()))
        );
        assert_eq!(
            operand(Operator::In, "1,{{nope}}", &v, None),
            Err(RhsError::Variable(missing.clone()))
        );
        assert_eq!(
            operand(Operator::Matches, "{{nope}}", &v, None),
            Err(RhsError::Variable(missing))
        );
    }

    #[test]
    fn unary_operators_ignore_their_text() {
        let v = Vars::new();
        assert_eq!(
            operand(Operator::IsNull, "{{nope}}", &v, None),
            Ok(Rhs::None)
        );
        assert_eq!(operand(Operator::IsArray, "${x}", &v, None), Ok(Rhs::None));
    }

    #[test]
    fn only_items_that_run_javascript_need_a_script() {
        assert!(!needs_script(Operator::In, "[1, 'a']"));
        assert!(needs_script(Operator::In, "[1, `${2}`]"));
        assert!(needs_script(Operator::Between, "1, `${2}`"));
        assert!(!needs_script(Operator::Matches, "`${x}`"));
        assert!(!needs_script(Operator::IsNull, "`${x}`"));
        assert!(needs_script(Operator::Eq, "`${200}`"));
        assert!(!needs_script(Operator::Eq, "{{x}}"));
    }
}
