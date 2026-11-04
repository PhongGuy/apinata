//! JavaScript expressions of post-response variables and assertions, one runtime each.

use std::sync::Arc;

use serde_json::Value;

use super::host::{self, HostState};
use super::{ScriptOptions, bru, engine};
use crate::interpolate::Vars;
use crate::js;

/// Why an expression produced no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExprError {
    /// It threw or hit a limit, with the message.
    Failed(String),
    /// It used a member Apinata does not support, with the message.
    Unsupported(String),
}

/// Evaluates expressions for one send, over its host state and the phase limits.
pub(crate) struct Expressions {
    state: Arc<HostState>,
    opts: ScriptOptions,
}

impl Expressions {
    /// Records the scope's sensitive values, `process` aside, in `state.sensitive` once.
    pub(crate) fn new(state: &Arc<HostState>, opts: &ScriptOptions) -> Expressions {
        let expressions = Expressions {
            state: Arc::clone(state),
            opts: opts.clone(),
        };
        expressions.scope();
        expressions
    }

    /// `code` as an expression; `Ok(None)` is `undefined`.
    pub(crate) fn value(&self, code: &str) -> Result<Option<Value>, ExprError> {
        let installers = host::expression_installers(&self.state);
        engine::eval_expression(code, &self.scope(), installers, &self.opts.limits(0)).map_err(
            |failure| match failure {
                engine::Failure::Unsupported { message } => ExprError::Unsupported(message),
                engine::Failure::Thrown { message } | engine::Failure::Limit { message } => {
                    ExprError::Failed(message)
                }
                engine::Failure::Cancelled => ExprError::Failed("cancelled".to_owned()),
            },
        )
    }

    /// Whether `text` is approved to run under these options.
    pub(crate) fn allows(&self, text: &str) -> bool {
        self.opts.allows(text)
    }

    /// Whether only approved texts run, so JavaScript must run exactly as written.
    pub(crate) fn restricted(&self) -> bool {
        self.opts.restricted()
    }

    /// `text` as the body of a template literal.
    pub(crate) fn template(&self, text: &str) -> Result<String, ExprError> {
        Ok(match self.value(&format!("`{text}`"))? {
            Some(Value::String(text)) => text,
            Some(other) => js::stringify(&other),
            None => "undefined".to_owned(),
        })
    }

    /// The request's variables as of now, so assertions see what the variables before them
    /// captured; their sensitive values are recorded for redaction on every read, except
    /// `process`, whose values are recorded only as an expression reads them.
    fn scope(&self) -> Vars {
        let (vars, sensitive) = self.state.ctx.lookup_for(&self.state.request);
        for name in sensitive.iter().filter(|name| *name != "process") {
            if let Some(value) = vars.get(name) {
                bru::note_sensitive(&self.state, value);
            }
        }
        vars
    }
}
