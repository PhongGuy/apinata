//! Post-response variables: `set-variable` actions that run after the response arrives.

use std::fmt;

use yaml_serde::{Mapping, Value};

use crate::assertion::lhs::{Unsupported, evaluate_expression, supported_shape};
use crate::assertion::truncated_reason;
use crate::document::{Fields, Origin, write_entry};
use crate::edit::disabled;
use crate::script::expr::{ExprError, Expressions};
use crate::{RequestFile, Response, VarContext, js};

/// Bruno's key order for a `set-variable` action.
const ACTION_ORDER: &[&str] = &[
    "type",
    "phase",
    "selector",
    "variable",
    "description",
    "disabled",
];

/// One post-response variable: a `set-variable` action with `phase: after-response`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PostResponseVar {
    name: String,
    expression: String,
    enabled: bool,
    origin: Option<Origin>,
}

impl PostResponseVar {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &PostResponseVar) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An enabled variable taking `expression`'s value into `name`.
    pub fn new(name: impl Into<String>, expression: impl Into<String>) -> PostResponseVar {
        PostResponseVar {
            name: name.into(),
            expression: expression.into(),
            enabled: true,
            origin: None,
        }
    }

    /// Rename the variable.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Change the response expression.
    pub fn set_expression(&mut self, expression: impl Into<String>) {
        self.expression = expression.into();
    }

    /// Switch the action on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The variable to set; may be empty or invalid, which evaluation reports.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The response expression whose value the variable takes, such as `res.body.id`.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Whether the action is switched on (`disabled` is not `true`).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Why sending would skip this variable, known without a response: an expression outside
    /// the supported shape, or a missing or invalid name. `None` when it may be evaluated.
    pub fn skip_hint(&self) -> Option<String> {
        let expression = js::js_trim(&self.expression);
        if !supported_shape(expression) {
            return Some(needs_js_reason(expression));
        }
        name_problem(&self.name)
    }

    /// Read one action, or `None` when it is not an after-response `set-variable`.
    fn from_yaml(item: &Value) -> Option<PostResponseVar> {
        if !item.is_mapping()
            || item.get("type").and_then(Value::as_str) != Some("set-variable")
            || item.get("phase").and_then(Value::as_str) != Some("after-response")
        {
            return None;
        }
        // `ensureString`: absent or null reads as empty, any other value as JS `String(v)`.
        let text = |v: Option<&Value>| v.and_then(js::yaml_js_string).unwrap_or_default();
        let mut var = PostResponseVar::new(
            text(item.get("variable").and_then(|v| v.get("name"))),
            text(item.get("selector").and_then(|s| s.get("expression"))),
        );
        var.enabled = !js::is_disabled(item);
        var.origin = Some(Origin::new(item.clone(), var.fields()));
        Some(var)
    }

    /// The action's YAML: as read when unchanged, else Bruno's form with unknown keys kept.
    pub(crate) fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), ACTION_ORDER)
    }

    fn fields(&self) -> Fields {
        let pair = |a: (&str, &str), b: (&str, &str)| {
            let mut map = Mapping::new();
            map.insert(Value::from(a.0), Value::from(a.1));
            map.insert(Value::from(b.0), Value::from(b.1));
            Value::Mapping(map)
        };
        vec![
            ("type", Some(Value::from("set-variable"))),
            ("phase", Some(Value::from("after-response"))),
            (
                "selector",
                Some(pair(("expression", &self.expression), ("method", "jsonq"))),
            ),
            (
                "variable",
                Some(pair(("name", &self.name), ("scope", "runtime"))),
            ),
            ("disabled", disabled(self.enabled)),
        ]
    }
}

/// Compares the public fields only; where a variable was read from does not matter.
impl PartialEq for PostResponseVar {
    fn eq(&self, other: &Self) -> bool {
        (&self.name, &self.expression, self.enabled)
            == (&other.name, &other.expression, other.enabled)
    }
}

impl Eq for PostResponseVar {}

/// `existing` actions with their post-response variables replaced by `vars`. The variables
/// fill the old slots in order, so other actions keep their place; leftover slots are dropped
/// and extra variables go after the last old slot, or at the end. A non-sequence is replaced.
pub(crate) fn merge_actions(existing: Option<&Value>, vars: &[PostResponseVar]) -> Vec<Value> {
    let old = existing.and_then(Value::as_sequence).map_or(&[][..], |s| s);
    let mut fresh = vars.iter().map(PostResponseVar::to_yaml);
    let last_slot = old
        .iter()
        .rposition(|item| PostResponseVar::from_yaml(item).is_some());
    let mut merged = Vec::with_capacity(old.len() + vars.len());
    for (i, item) in old.iter().enumerate() {
        if PostResponseVar::from_yaml(item).is_none() {
            merged.push(item.clone());
        } else if let Some(var) = fresh.next() {
            merged.push(var);
        }
        if Some(i) == last_slot {
            merged.extend(fresh.by_ref());
        }
    }
    merged.extend(fresh);
    merged
}

/// The post-response variables in an `actions` list, in order, disabled ones included.
pub(crate) fn post_response_vars_in(actions: Option<&Value>) -> Vec<PostResponseVar> {
    actions
        .and_then(Value::as_sequence)
        .map_or_else(Vec::new, |items| {
            items
                .iter()
                .filter_map(PostResponseVar::from_yaml)
                .collect()
        })
}

/// Whether an `actions` value holds anything other than post-response variables.
pub(crate) fn has_other_actions(actions: Option<&Value>) -> bool {
    match actions {
        Some(Value::Sequence(items)) => items
            .iter()
            .any(|item| PostResponseVar::from_yaml(item).is_none()),
        Some(Value::Mapping(items)) => !items.is_empty(),
        _ => false,
    }
}

/// What evaluating one post-response variable did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VarResult {
    /// The variable now holds the expression's value.
    Set,
    /// The expression was `undefined`, which the variable now holds.
    Undefined,
    /// Nothing was stored, for the given reason.
    Skipped {
        /// Why the variable was not set.
        reason: String,
    },
    /// The JavaScript expression threw or hit a limit; nothing was stored.
    Failed {
        /// The error, such as `ReferenceError: x is not defined`.
        error: String,
    },
}

/// One post-response variable and what evaluating it did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VarOutcome {
    name: String,
    expression: String,
    result: VarResult,
}

impl VarOutcome {
    /// The variable's name as written, possibly empty or invalid.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The response expression as written.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Whether the variable was set, set to `undefined`, skipped, or failed.
    pub fn result(&self) -> &VarResult {
        &self.result
    }
}

impl fmt::Display for VarOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "post-response variable `{}` ", self.name)?;
        match &self.result {
            VarResult::Set => f.write_str("set"),
            VarResult::Undefined => f.write_str("set to undefined"),
            VarResult::Skipped { reason } => write!(f, "not set: {reason}"),
            VarResult::Failed { error } => write!(f, "not set: {error}"),
        }
    }
}

/// Evaluates `req`'s merged, enabled post-response variables against `res` in order and
/// stores each into `ctx`'s runtime variables; skipped ones leave the store untouched.
pub fn apply_post_response_vars(
    req: &RequestFile,
    res: &Response,
    ctx: &VarContext,
) -> Vec<VarOutcome> {
    apply_post_response_vars_with(req, res, ctx, None)
}

/// [`apply_post_response_vars`], evaluating expressions outside the native subset with `js`
/// when scripts run.
pub(crate) fn apply_post_response_vars_with(
    req: &RequestFile,
    res: &Response,
    ctx: &VarContext,
    js: Option<&Expressions>,
) -> Vec<VarOutcome> {
    ctx.post_response_vars_for(req)
        .into_iter()
        .map(|var| {
            let result = apply_one(&var, res, ctx, js);
            VarOutcome {
                name: var.name,
                expression: var.expression,
                result,
            }
        })
        .collect()
}

/// Evaluates one variable, then checks its name, as the expression runs first in Bruno.
fn apply_one(
    var: &PostResponseVar,
    res: &Response,
    ctx: &VarContext,
    scripted: Option<&Expressions>,
) -> VarResult {
    let expression = js::js_trim(&var.expression);
    let value = match evaluate_expression(expression, res) {
        Ok(value) => value,
        // A `res` chain that falls back to JavaScript needs no approval.
        Err(Unsupported::NeedsJs)
            if let Some(scripted) = scripted
                && !supported_shape(expression)
                && !scripted.allows(expression) =>
        {
            let reason = not_approved_reason(expression);
            return VarResult::Skipped { reason };
        }
        Err(Unsupported::NeedsJs) if let Some(scripted) = scripted => {
            match scripted.value(expression) {
                Ok(value) => value,
                Err(ExprError::Failed(error)) => return VarResult::Failed { error },
                Err(ExprError::Unsupported(reason)) => return VarResult::Skipped { reason },
            }
        }
        Err(Unsupported::NeedsJs) => {
            let reason = needs_js_reason(expression);
            return VarResult::Skipped { reason };
        }
        Err(Unsupported::Truncated) => {
            let reason = truncated_reason(expression);
            return VarResult::Skipped { reason };
        }
    };
    if let Some(reason) = name_problem(&var.name) {
        return VarResult::Skipped { reason };
    }
    let result = if value.is_some() {
        VarResult::Set
    } else {
        VarResult::Undefined
    };
    ctx.runtime_vars().set(var.name.clone(), value);
    result
}

/// The skip reason for a JavaScript expression whose text is not approved to run.
pub(crate) fn not_approved_reason(expression: &str) -> String {
    format!("the expression `{expression}` is not approved")
}

/// The skip reason for an expression outside the evaluated subset; `expression` is trimmed.
fn needs_js_reason(expression: &str) -> String {
    format!("`{expression}` needs a JavaScript runtime")
}

/// Why `name` cannot hold a runtime variable, or `None` when it can.
fn name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("the variable has no name".to_owned());
    }
    let valid = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-');
    (!name.chars().all(valid)).then(|| {
        format!(
            "`{name}` is not a valid variable name; use ASCII letters, digits, `-`, `_` and `.`"
        )
    })
}

/// Why `name` breaks the variable-name rule shared with post-response variables, or `None`.
/// Deliberately stricter than what the resolver can look up.
pub fn variable_name_problem(name: &str) -> Option<String> {
    name_problem(name)
}

#[cfg(test)]
mod name_tests {
    use super::{PostResponseVar, variable_name_problem};

    // Actions read from YAML that differs only in a key the app does not model are equal,
    // yet only `same_source` tells them apart; identical reads agree on both.
    #[test]
    fn same_source_also_compares_the_yaml_an_action_was_read_from() {
        let old = "type: set-variable\nphase: after-response\nselector:\n  expression: res.body.id\nvariable:\n  name: id\nx-note: old\n";
        let new = old.replace("x-note: old", "x-note: new");
        let read =
            |text: &str| PostResponseVar::from_yaml(&yaml_serde::from_str(text).unwrap()).unwrap();
        let (a, b) = (read(old), read(&new));
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&read(old)));
        assert!(!a.same_source(&PostResponseVar::new("id", "res.body.id")));
    }

    #[test]
    fn unresolvable_names_are_reported_and_plain_ones_pass() {
        for bad in [
            "", "   ", "{{tok}}", " tok ", "tok ", " tok", "{tok}", "a b", "a}}",
        ] {
            let reason = variable_name_problem(bad);
            assert!(
                reason.as_deref().is_some_and(|r| r.contains("name")),
                "{bad:?}: {reason:?}"
            );
        }
        for good in ["tok", "api.key", "my-var_1"] {
            assert_eq!(variable_name_problem(good), None, "{good}");
        }
    }
}
