//! Declarative assertions: parsing, and evaluation against a response.

pub(crate) mod lhs;
pub(crate) mod ops;
pub(crate) mod rhs;
pub(crate) mod text;

use std::cell::RefCell;

use serde_json::Value;
use yaml_serde::Value as Yaml;

use crate::document::{Fields, Origin, write_entry};
use crate::interpolate::{Vars, interpolate_tracked};
use crate::post_response::not_approved_reason;
use crate::script::expr::{ExprError, Expressions};
use crate::{RequestFile, ResolveError, Response, VarContext, js};
use lhs::Unsupported;
use rhs::RhsError;

/// One declarative assertion from a request's `runtime.assertions` list.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Assertion {
    expression: String,
    operator: String,
    value: Option<String>,
    enabled: bool,
    description: Option<String>,
    bruno_value: String,
    origin: Option<Origin>,
}

/// Bruno's key order for an assertion entry.
const ASSERTION_ORDER: &[&str] = &["expression", "operator", "value", "description", "disabled"];

/// The operators `parseAssertionOperator` accepts without a right side.
const UNARY: &[&str] = &[
    "isEmpty",
    "isNotEmpty",
    "isNull",
    "isUndefined",
    "isDefined",
    "isTruthy",
    "isFalsy",
    "isJson",
    "isNumber",
    "isString",
    "isBoolean",
    "isArray",
];

/// The operators `parseAssertionOperator` accepts with a right side.
const BINARY: &[&str] = &[
    "eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "in",
    "notIn",
    "contains",
    "notContains",
    "length",
    "matches",
    "notMatches",
    "startsWith",
    "endsWith",
    "between",
];

/// Compares what a reader sees; where the entry was read from does not matter.
impl PartialEq for Assertion {
    fn eq(&self, other: &Self) -> bool {
        (
            &self.expression,
            &self.operator,
            &self.value,
            self.enabled,
            &self.description,
            &self.bruno_value,
        ) == (
            &other.expression,
            &other.operator,
            &other.value,
            other.enabled,
            &other.description,
            &other.bruno_value,
        )
    }
}

impl Eq for Assertion {}

impl Assertion {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &Assertion) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An enabled assertion without a description.
    pub fn new(
        expression: impl Into<String>,
        operator: impl Into<String>,
        value: Option<String>,
    ) -> Assertion {
        let operator = operator.into();
        let bruno_value = match &value {
            Some(value) => format!("{operator} {value}"),
            None => operator.clone(),
        };
        Assertion {
            expression: expression.into(),
            operator,
            value,
            enabled: true,
            description: None,
            bruno_value,
            origin: None,
        }
    }

    /// Switch the assertion on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Replace the left side, keeping the operator, value and description.
    pub fn set_expression(&mut self, expression: impl Into<String>) {
        self.expression = expression.into();
    }

    /// Replace the operator and value as `new` builds them, keeping the description.
    pub fn set_operator(&mut self, operator: impl Into<String>, value: Option<String>) {
        let Assertion {
            operator,
            value,
            bruno_value,
            ..
        } = Assertion::new("", operator, value);
        (self.operator, self.value, self.bruno_value) = (operator, value, bruno_value);
    }

    /// The left side, such as `res.status`.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// The operator as written, such as `eq`; empty when the entry has none.
    pub fn operator(&self) -> &str {
        &self.operator
    }

    /// The right side as text; `None` when absent or null.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Whether the assertion runs; only `disabled: true` turns it off.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The description, when it has visible text.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Converts one list entry as Bruno's filestore `toBrunoAssertions` does
    /// (bruno-filestore/src/formats/yml/common/assertions.ts:115-146).
    pub(crate) fn from_yaml(entry: &Yaml) -> Assertion {
        let field = |key: &str| entry.as_mapping().and_then(|map| map.get(key));
        let text = |key: &str| field(key).and_then(js::yaml_js_string);
        let operator = text("operator");
        let value = text("value");
        // The template literal prints a missing operator as `undefined` and a null one as `null`.
        let bruno_value = match (&value, field("operator")) {
            (Some(value), None) => format!("undefined {value}"),
            (Some(value), Some(Yaml::Null)) => format!("null {value}"),
            (Some(value), Some(_)) => format!("{} {value}", operator.as_deref().unwrap_or("")),
            (None, _) => operator.clone().unwrap_or_default(),
        };
        let description = match field("description") {
            Some(Yaml::Mapping(map)) => map.get("content").and_then(Yaml::as_str),
            Some(other) => other.as_str(),
            None => None,
        }
        .filter(|d| !js::js_trim(d).is_empty());
        let mut assertion = Assertion {
            expression: text("expression").unwrap_or_default(),
            operator: operator.unwrap_or_default(),
            value,
            enabled: !js::is_disabled(entry),
            description: description.map(str::to_owned),
            bruno_value,
            origin: None,
        };
        assertion.origin = Some(Origin::new(entry.clone(), assertion.fields()));
        assertion
    }

    /// The entry Bruno writes: as read when unchanged, else with the operator re-read from
    /// `operator value` as `parseAssertionOperator` does.
    pub(crate) fn to_yaml(&self) -> Yaml {
        write_entry(self.origin.as_ref(), self.fields(), ASSERTION_ORDER)
    }

    fn fields(&self) -> Fields {
        let (operator, value) = parse_operator(&self.bruno_value);
        vec![
            ("expression", Some(Yaml::from(self.expression.as_str()))),
            ("operator", Some(Yaml::from(operator))),
            ("value", value.map(Yaml::from)),
            ("disabled", (!self.enabled).then_some(Yaml::Bool(true))),
        ]
    }

    /// The `operator value` string Bruno's assertion runtime receives.
    pub(crate) fn bruno_value(&self) -> &str {
        &self.bruno_value
    }
}

/// Port of `parseAssertionOperator` (bruno-filestore `common/assertions.ts`): the first word is
/// the operator when Bruno knows it, else the whole text is the value of an `eq`.
fn parse_operator(text: &str) -> (&str, Option<&str>) {
    if text.is_empty() {
        return ("eq", Some(text));
    }
    let trimmed = js::js_trim(text);
    let (first, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
    if UNARY.contains(&first) {
        (first, None)
    } else if BINARY.contains(&first) {
        (first, Some(rest))
    } else {
        ("eq", Some(text))
    }
}

/// The result of one assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// The assertion held.
    Pass,
    /// The assertion did not hold, or its right side named an undefined variable.
    #[non_exhaustive]
    Fail {
        /// The left side as `JSON.stringify` prints it, or `undefined`.
        actual: String,
        /// Why the comparison could not run, such as an undefined right-side variable.
        detail: Option<String>,
    },
    /// The assertion needs a JavaScript runtime, or reads a body truncated at the size limit,
    /// so it counts as neither a pass nor a fail.
    #[non_exhaustive]
    Skipped {
        /// Which side needs JavaScript and its text, or the truncated `res.body` expression.
        reason: String,
    },
}

impl Verdict {
    /// A failed verdict with the left side's `actual` text and an optional `detail`.
    pub fn fail(actual: impl Into<String>, detail: Option<String>) -> Verdict {
        Verdict::Fail {
            actual: actual.into(),
            detail,
        }
    }

    /// A skipped verdict that says `reason`.
    pub fn skipped(reason: impl Into<String>) -> Verdict {
        Verdict::Skipped {
            reason: reason.into(),
        }
    }
}

/// One enabled assertion and its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AssertionOutcome {
    assertion: Assertion,
    verdict: Verdict,
}

impl AssertionOutcome {
    /// An outcome pairing `assertion` with its `verdict`.
    pub fn new(assertion: Assertion, verdict: Verdict) -> AssertionOutcome {
        AssertionOutcome { assertion, verdict }
    }

    /// The assertion as read from the request file.
    pub fn assertion(&self) -> &Assertion {
        &self.assertion
    }

    /// Whether it passed, failed or was skipped.
    pub fn verdict(&self) -> &Verdict {
        &self.verdict
    }
}

/// Evaluates a request's enabled assertions against `res`, in file order.
///
/// Right sides interpolate the variables the request sees in `ctx`, folder and request
/// variables included.
pub fn evaluate(req: &RequestFile, res: &Response, ctx: &VarContext) -> Vec<AssertionOutcome> {
    evaluate_with(&req.assertions(), res, &ctx.vars_for(req), None)
}

/// [`evaluate`], running the JavaScript sides outside the native subset with `scripted`.
pub(crate) fn evaluate_scripted(
    req: &RequestFile,
    res: &Response,
    ctx: &VarContext,
    scripted: &Expressions,
) -> Vec<AssertionOutcome> {
    evaluate_with(&req.assertions(), res, &ctx.vars_for(req), Some(scripted))
}

/// The sensitive values the right sides of `req`'s enabled assertions interpolate, in first-use
/// order without repeats, so a caller can scrub a secret that only an assertion reads.
pub fn assertion_sensitive_values(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let (vars, sensitive) = ctx.lookup_for(req);
    let tracker = ctx.tracker(sensitive);
    let generator = RefCell::new(ctx.generator());
    for assertion in req.assertions().iter().filter(|a| a.enabled()) {
        let (_, text) = rhs::split(assertion.bruno_value());
        // A failed expansion still leaves what it substituted before the error in the tracker.
        let _ = interpolate_tracked(&text, &vars, Some(&tracker), &generator);
    }
    tracker.into_values()
}

/// Evaluates the enabled `assertions` against `res`, interpolating right sides from `vars` and
/// running JavaScript sides with `scripted` when present.
pub(crate) fn evaluate_with(
    assertions: &[Assertion],
    res: &Response,
    vars: &Vars,
    scripted: Option<&Expressions>,
) -> Vec<AssertionOutcome> {
    assertions
        .iter()
        .filter(|a| a.enabled())
        .map(|a| AssertionOutcome {
            assertion: a.clone(),
            verdict: verdict(a, res, vars, scripted),
        })
        .collect()
}

/// The skip reason for an expression that reads a truncated body.
pub(crate) fn truncated_reason(expression: &str) -> String {
    format!(
        "the response body was truncated at the size limit, so `{expression}` was not evaluated"
    )
}

/// One assertion's verdict, in Bruno's order: operator split, left side, right side, operator.
fn verdict(
    assertion: &Assertion,
    res: &Response,
    vars: &Vars,
    scripted: Option<&Expressions>,
) -> Verdict {
    let (op, text) = rhs::split(assertion.bruno_value());
    let expression = js::js_trim(assertion.expression());
    let lhs = match lhs::evaluate_expression(expression, res) {
        Ok(lhs) => lhs,
        // A `res` chain that falls back to JavaScript needs no approval.
        Err(Unsupported::NeedsJs)
            if let Some(scripted) = scripted
                && !lhs::supported_shape(expression)
                && !scripted.allows(expression) =>
        {
            let reason = not_approved_reason(expression);
            return Verdict::Skipped { reason };
        }
        Err(Unsupported::NeedsJs) if let Some(scripted) = scripted => {
            match scripted.value(expression) {
                Ok(lhs) => lhs,
                Err(ExprError::Failed(error)) => return Verdict::fail("undefined", Some(error)),
                Err(ExprError::Unsupported(reason)) => return Verdict::Skipped { reason },
            }
        }
        Err(Unsupported::NeedsJs) => {
            let reason = format!("left side `{expression}` needs a JavaScript runtime");
            return Verdict::Skipped { reason };
        }
        Err(Unsupported::Truncated) => {
            let reason = truncated_reason(expression);
            return Verdict::Skipped { reason };
        }
    };
    let fail = |lhs: Option<&Value>, detail: Option<String>| Verdict::Fail {
        actual: lhs.map_or_else(|| "undefined".to_owned(), js::stringify),
        detail,
    };
    match rhs::operand(op, &text, vars, scripted) {
        Err(RhsError::NeedsJs) => Verdict::Skipped {
            reason: format!("right side `{text}` needs a JavaScript runtime"),
        },
        Err(RhsError::Script(ExprError::Failed(error))) => fail(lhs.as_ref(), Some(error)),
        Err(RhsError::Script(ExprError::Unsupported(reason))) => Verdict::Skipped { reason },
        // An undefined variable is never compared as literal `{{x}}` text.
        Err(RhsError::Variable(ResolveError::UndefinedVariable { name, .. })) => fail(
            lhs.as_ref(),
            Some(format!("variable `{{{{{name}}}}}` is undefined")),
        ),
        Err(RhsError::Variable(ResolveError::CyclicVariable { name })) => fail(
            lhs.as_ref(),
            Some(format!("variable `{{{{{name}}}}}` refers to itself")),
        ),
        Err(RhsError::Variable(_)) => fail(lhs.as_ref(), None),
        Ok(operand) if ops::apply(op, lhs.as_ref(), &operand) => Verdict::Pass,
        Ok(_) => fail(lhs.as_ref(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestFile;
    use serde_json::json;
    use std::path::Path;

    const HEAD: &str =
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n";

    fn load(rest: &str) -> Vec<Assertion> {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("r.yml");
        std::fs::write(&path, format!("{HEAD}{rest}")).expect("write");
        RequestFile::load(&path).expect("loads").assertions()
    }

    // Assertions read from YAML that differs only in a key the app does not model are equal,
    // yet only `same_source` tells them apart; identical reads agree on both.
    #[test]
    fn same_source_also_compares_the_yaml_an_assertion_was_read_from() {
        let old = "expression: res.status\noperator: eq\nvalue: \"200\"\nx-note: old\n";
        let new = old.replace("x-note: old", "x-note: new");
        let read = |text: &str| Assertion::from_yaml(&yaml_serde::from_str(text).unwrap());
        let (a, b) = (read(old), read(&new));
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&read(old)));
        assert!(!a.same_source(&Assertion::new("res.status", "eq", Some("200".into()))));
    }

    /// (expression, operator, value, enabled, description, Bruno's "operator value" string)
    type Row<'a> = (
        &'a str,
        &'a str,
        Option<&'a str>,
        bool,
        Option<&'a str>,
        &'a str,
    );

    fn rows(list: &[Assertion]) -> Vec<Row<'_>> {
        list.iter()
            .map(|a| {
                (
                    a.expression(),
                    a.operator(),
                    a.value(),
                    a.enabled(),
                    a.description(),
                    a.bruno_value(),
                )
            })
            .collect()
    }

    #[test]
    fn a_vendored_fixture_yields_its_assertion() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/bruno/bruno-tests/response-time/delayed-body.yml");
        let list = RequestFile::load(path).expect("loads").assertions();
        assert_eq!(
            rows(&list),
            [("res.status", "eq", Some("200"), true, None, "eq 200")]
        );
    }

    #[test]
    fn entries_convert_as_bruno_filestore_does() {
        let list = load(concat!(
            "runtime:\n  assertions:\n",
            "    - expression: res.status\n      operator: eq\n      value: 200\n",
            "    - expression: res.body\n      operator: isJson\n",
            "    - expression: res.body.a\n      operator: eq\n      value: null\n      disabled: true\n      description: \"  note \"\n",
            "    - expression: res.body.b\n      operator: in\n      value: [1, 2.50]\n      description:\n        content: why\n",
            "    - expression: 5\n      value: x\n",
            "    - expression: res.body.c\n      operator: ~\n      value: x\n      description: \"  \"\n",
            "    - expression: res.body.d\n      operator: matches\n      value: {a: 1}\n      disabled: \"true\"\n",
            "    - not a mapping\n",
        ));
        assert_eq!(
            rows(&list),
            [
                ("res.status", "eq", Some("200"), true, None, "eq 200"),
                ("res.body", "isJson", None, true, None, "isJson"),
                ("res.body.a", "eq", None, false, Some("  note "), "eq"),
                (
                    "res.body.b",
                    "in",
                    Some("1,2.5"),
                    true,
                    Some("why"),
                    "in 1,2.5"
                ),
                ("5", "", Some("x"), true, None, "undefined x"),
                ("res.body.c", "", Some("x"), true, None, "null x"),
                (
                    "res.body.d",
                    "matches",
                    Some("[object Object]"),
                    true,
                    None,
                    "matches [object Object]"
                ),
                ("", "", None, true, None, ""),
            ]
        );
    }

    #[test]
    fn files_without_a_list_have_no_assertions() {
        assert_eq!(load(""), []);
        assert_eq!(load("runtime:\n  assertions: nope\n"), []);
    }

    #[test]
    fn new_builds_the_operator_value_string() {
        let a = Assertion::new("res.status", "eq", Some("201".to_owned()));
        assert_eq!(
            rows(&[a]),
            [("res.status", "eq", Some("201"), true, None, "eq 201")]
        );
        let b = Assertion::new("res.body", "isJson", None);
        assert_eq!(b.bruno_value(), "isJson");
    }

    fn run(
        expression: &str,
        op: &str,
        value: Option<&str>,
        res: &Response,
        vars: serde_json::Value,
    ) -> Verdict {
        let vars = vars.as_object().cloned().expect("an object");
        let assertion = Assertion::new(expression, op, value.map(str::to_owned));
        let mut outcomes = evaluate_with(&[assertion], res, &vars, None);
        assert_eq!(outcomes.len(), 1);
        outcomes.remove(0).verdict
    }

    fn body(text: &str) -> Response {
        Response::new(200).with_text_body(text)
    }

    fn fail(actual: &str) -> Verdict {
        Verdict::Fail {
            actual: actual.to_owned(),
            detail: None,
        }
    }

    /// A failure caused by an undefined right-side variable.
    fn undefined(actual: &str, name: &str) -> Verdict {
        Verdict::Fail {
            actual: actual.to_owned(),
            detail: Some(format!("variable `{{{{{name}}}}}` is undefined")),
        }
    }

    #[test]
    fn bruno_runtime_spec_cases_transcribed() {
        // packages/bruno-js/tests/runtime.spec.js at 1301ea17, quickjs runtime.
        let not_found = Response::new(404).with_text_body("{\"error\":\"not_found\"}");
        assert_eq!(
            run("res.status", "eq", Some("404"), &not_found, json!({})),
            Verdict::Pass
        );
        assert_eq!(
            run(
                "res.body.error",
                "eq",
                Some("not_found"),
                &not_found,
                json!({})
            ),
            Verdict::Pass
        );
        // Bruno passes these; Apinata skips templates and bare variables by design.
        assert!(matches!(
            run("res.status", "eq", Some("${200+0}"), &not_found, json!({})),
            Verdict::Skipped { .. }
        ));
        assert!(matches!(
            run(
                "token",
                "eq",
                Some("one"),
                &not_found,
                json!({"token": "one"})
            ),
            Verdict::Skipped { .. }
        ));
        for json_body in [
            "{\"a\":1}",
            "{\"a\":{\"b\":[1]}}",
            "[1,2]",
            "[\"a\",\"b\"]",
            "[]",
            "[{\"id\":1}]",
        ] {
            assert_eq!(
                run("res.body", "isJson", None, &body(json_body), json!({})),
                Verdict::Pass,
                "{json_body}"
            );
        }
        assert_eq!(
            run("res.body", "isJson", None, &body("\"string\""), json!({})),
            fail("\"string\"")
        );
        assert_eq!(
            run("res.body", "isJson", None, &body("null"), json!({})),
            fail("null")
        );
        // unresolved-variables.spec.js: Bruno compares against the literal text; Apinata fails.
        assert_eq!(
            run(
                "res.status",
                "eq",
                Some("{{expectedStatus}}"),
                &not_found,
                json!({})
            ),
            undefined("404", "expectedStatus")
        );
    }

    #[test]
    fn actual_is_the_left_side_stringified_the_javascript_way() {
        let res = body("{\"s\":\"abc\",\"o\":{\"x\":1.50,\"n\":1e21},\"big\":1e21}");
        assert_eq!(
            run("res.status", "eq", Some("201"), &res, json!({})),
            fail("200")
        );
        assert_eq!(
            run("res.body.s", "eq", Some("x"), &res, json!({})),
            fail("\"abc\"")
        );
        assert_eq!(
            run("res.body.o", "isArray", None, &res, json!({})),
            fail("{\"x\":1.5,\"n\":1e+21}")
        );
        assert_eq!(
            run("res.body.big", "eq", Some("1"), &res, json!({})),
            fail("1e+21")
        );
        assert_eq!(
            run("res.body.none", "isDefined", None, &res, json!({})),
            fail("undefined")
        );
    }

    #[test]
    fn reading_through_undefined_behaves_like_bruno_quickjs() {
        let res = body("{}");
        assert_eq!(
            run("res.body.missing.b", "isDefined", None, &res, json!({})),
            Verdict::Pass
        );
        assert_eq!(
            run("res.body.missing.b", "isJson", None, &res, json!({})),
            Verdict::Pass
        );
        assert_eq!(
            run("res.body.missing.b", "eq", Some("1"), &res, json!({})),
            fail("{\"name\":\"TypeError\",\"message\":\"cannot read property 'b' of undefined\"}")
        );
    }

    #[test]
    fn right_sides_interpolate_skip_or_fail() {
        let res = body("{\"n\":404,\"s\":\"a b\"}");
        assert_eq!(
            run(
                "res.body.n",
                "eq",
                Some("{{code}}"),
                &res,
                json!({"code": 404})
            ),
            Verdict::Pass
        );
        assert_eq!(
            run(
                "res.body.s",
                "eq",
                Some("{{q}}"),
                &res,
                json!({"q": "\"a b\""})
            ),
            Verdict::Pass
        );
        assert_eq!(
            run("res.body.n", "in", Some("1,{{nope}}"), &res, json!({})),
            undefined("404", "nope")
        );
        assert_eq!(
            run("res.body.s", "matches", Some("("), &res, json!({})),
            fail("\"a b\"")
        );
        assert_eq!(
            run("res.body.s", "notMatches", Some("("), &res, json!({})),
            fail("\"a b\"")
        );
        let skipped = run("res.body.s", "eq", Some("${x}"), &res, json!({}));
        assert_eq!(
            skipped,
            Verdict::Skipped {
                reason: "right side `${x}` needs a JavaScript runtime".to_owned()
            }
        );
        assert!(
            matches!(run("res.url", "eq", Some("{{nope}}"), &res, json!({})), Verdict::Skipped { reason } if reason.contains("res.url"))
        );
        assert_eq!(
            run("res.body.s", "foo", Some("1"), &res, json!({})),
            fail("\"a b\"")
        );
        assert_eq!(
            run("res.body.s", "eq", Some("a b"), &res, json!({})),
            Verdict::Pass
        );
    }

    #[test]
    fn disabled_assertions_produce_no_outcome() {
        let list = load(concat!(
            "runtime:\n  assertions:\n",
            "    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
            "    - expression: res.status\n      operator: eq\n      value: \"500\"\n      disabled: true\n",
        ));
        let outcomes = evaluate_with(&list, &Response::new(200), &Vars::new(), None);
        let kept: Vec<_> = outcomes
            .iter()
            .map(|o| (o.assertion().expression(), o.verdict().clone()))
            .collect();
        assert_eq!(kept, [("res.status", Verdict::Pass)]);
    }

    #[test]
    fn a_whole_body_dump_prints_numbers_the_javascript_way() {
        let res = body("{\"big\":12345678901234567890,\"z\":-0.0}");
        assert_eq!(
            run("res.body", "isArray", None, &res, json!({})),
            fail("{\"big\":12345678901234567000,\"z\":0}")
        );
    }

    #[test]
    fn a_failure_names_the_undefined_variable() {
        let res = body("{\"n\":1}");
        let verdict = run("res.body.n", "eq", Some("{{missing}}"), &res, json!({}));
        let Verdict::Fail { detail, .. } = verdict else {
            panic!("expected a failure, got {verdict:?}");
        };
        assert_eq!(
            detail.as_deref(),
            Some("variable `{{missing}}` is undefined")
        );
        assert_eq!(
            run("res.body.n", "eq", Some("2"), &res, json!({})),
            fail("1")
        );
    }

    #[test]
    fn a_failure_names_the_cyclic_variable() {
        let res = body("{\"n\":1}");
        let verdict = run(
            "res.body.n",
            "eq",
            Some("{{a}}"),
            &res,
            json!({"a": "{{a}}"}),
        );
        let Verdict::Fail { detail, .. } = verdict else {
            panic!("expected a failure, got {verdict:?}");
        };
        assert_eq!(detail.as_deref(), Some("variable `{{a}}` refers to itself"));
    }
}
