//! Assertions as Postman tests: the response fields and operators chai spells the same way.

use serde_json::Value;

use crate::Assertion;
use crate::assertion::lhs::{Segment, parse_expression};
use crate::assertion::rhs::{Operator, Scalar, literal, split};
use crate::export::ExportNote;
use crate::js::{self, js_trim};
use crate::response::SINGLE_VALUED;

const TURNED: &str = "assertions turned into Postman tests";
const LEFT_OUT: &str = "these assertions could not be turned into Postman tests and were left out";

/// One `pm.test` line per assertion Postman can check, noting at `<path> / assertions` that
/// tests were written and which assertions, disabled ones included, were left out.
pub(super) fn tests(
    assertions: &[Assertion],
    path: &[&str],
    notes: &mut Vec<ExportNote>,
) -> Vec<String> {
    let source = format!("{} / assertions", path.join(" / "));
    let mut lines = Vec::new();
    let mut left = Vec::new();
    for assertion in assertions {
        let name = format!("{} {}", assertion.expression(), assertion.bruno_value());
        let name = js_trim(&name).to_owned();
        let expect = assertion
            .enabled()
            .then(|| expect(assertion.expression(), assertion.bruno_value()))
            .flatten();
        match expect {
            Some(expect) => lines.push(format!("pm.test({}, () => {expect});", quoted(&name))),
            None => left.push(name),
        }
    }
    if !lines.is_empty() {
        notes.push(ExportNote::changed(&source, TURNED));
    }
    if !left.is_empty() {
        // Each assertion is quoted, so no text inside one can read as a separator.
        let left: Vec<String> = left.iter().map(|name| quoted(name)).collect();
        notes.push(ExportNote::new(
            source,
            format!("{LEFT_OUT}: {}", left.join(", ")),
        ));
    }
    lines
}

/// The `pm.expect` call for `expression` against `operator value` text, when both translate.
fn expect(expression: &str, text: &str) -> Option<String> {
    let (actual, nested) = actual(expression)?;
    let (op, value) = split(text);
    let chain = match op {
        // Reading through a missing value gives core an error object, which is defined and
        // unequal to anything, while Postman throws; only these two operators would differ.
        Operator::Neq | Operator::IsDefined if nested => return None,
        Operator::Eq => format!("to.equal({})", scalar(&value)?),
        Operator::Neq => format!("to.not.equal({})", scalar(&value)?),
        Operator::Gt => format!("to.be.above({})", number(&value)?),
        Operator::Gte => format!("to.be.at.least({})", number(&value)?),
        Operator::Lt => format!("to.be.below({})", number(&value)?),
        Operator::Lte => format!("to.be.at.most({})", number(&value)?),
        Operator::Contains => format!("to.include({})", text_value(&value)?),
        Operator::NotContains => format!("to.not.include({})", text_value(&value)?),
        Operator::IsDefined => "to.not.be.undefined".to_owned(),
        Operator::IsUndefined => "to.be.undefined".to_owned(),
        Operator::IsNull => "to.be.null".to_owned(),
        _ => return None,
    };
    Some(format!("pm.expect({actual}).{chain}"))
}

/// The Postman value for a status, status text, response time, one header or a JSON body path,
/// and whether it is a body path that reads through a parent value.
fn actual(expression: &str) -> Option<(String, bool)> {
    let segments = parse_expression(expression)?;
    let (Segment::Dot(first), rest) = segments.split_first()? else {
        return None;
    };
    let plain = |text: &str| Some((text.to_owned(), false));
    match (first.as_str(), rest) {
        ("status", []) => plain("pm.response.code"),
        ("statusText", []) => plain("pm.response.status"),
        ("responseTime", []) => plain("pm.response.responseTime"),
        ("headers", [Segment::Dot(name) | Segment::Key(name)]) => Some((header(name)?, false)),
        ("body", chain) if !chain.is_empty() => {
            let path: String = chain.iter().map(Segment::render).collect();
            Some((format!("pm.response.json(){path}"), chain.len() > 1))
        }
        _ => None,
    }
}

/// Header `name` as core reads it: by its lowercase name only, the first value of a header
/// Node keeps once, other repeats joined, and undefined when absent. Postman's `get` ignores
/// case and returns the last value, so the values are gathered by hand. `set-cookie` is an
/// array in core and is left out.
fn header(name: &str) -> Option<String> {
    if name != name.to_ascii_lowercase() || name == "set-cookie" {
        return None;
    }
    let values = format!(
        "pm.response.headers.all().filter((h) => h.key.toLowerCase() === {}).map((h) => h.value)",
        quoted(name)
    );
    if SINGLE_VALUED.contains(&name) {
        return Some(format!("{values}[0]"));
    }
    let sep = if name == "cookie" { "; " } else { ", " };
    Some(format!(
        "((v) => v.length ? v.join({}) : undefined)({values})",
        quoted(sep)
    ))
}

/// The right side as a JavaScript literal. A variable is only known when the request runs, so
/// a value holding one is not translated.
fn scalar(value: &str) -> Option<String> {
    if value.contains("{{") {
        return None;
    }
    Some(match literal(value).ok()? {
        Scalar::Undefined => "undefined".to_owned(),
        Scalar::Null => "null".to_owned(),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Number(n) => js::number_to_string(n),
        Scalar::String(s) => quoted(&s),
    })
}

/// A finite number for the comparisons, which chai refuses on anything else.
fn number(value: &str) -> Option<String> {
    match literal(value).ok()? {
        Scalar::Number(n) if n.is_finite() => Some(js::number_to_string(n)),
        _ => None,
    }
}

/// A plain string for `include`.
fn text_value(value: &str) -> Option<String> {
    match literal(value).ok()? {
        Scalar::String(s) if !s.contains("{{") => Some(quoted(&s)),
        _ => None,
    }
}

/// `text` as a double-quoted JavaScript string.
fn quoted(text: &str) -> String {
    Value::from(text).to_string()
}
