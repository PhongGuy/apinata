//! The variables one request can see, as editors show them: each name's scope, rendered
//! value and secrecy, answered by the same lookup and interpolation that sending uses.

use std::cell::RefCell;
use std::fmt;

use crate::context::VarContext;
use crate::dynamic::dynamic_variables;
use crate::interpolate::{interpolate_tracked, lookup, to_path};
use crate::js;
use crate::jwt::{DecodedJwt, JwtError, decode_jwt};
use crate::request::RequestFile;

/// The layer a variable's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VarScope {
    /// A selected global environment, its overrides and stored secrets.
    GlobalEnvironment,
    /// The collection's `request.variables`.
    Collection,
    /// The selected environment, its overrides and stored secrets.
    Environment,
    /// A folder's `request.variables`, the innermost that defines it.
    Folder,
    /// The request's own `runtime.variables`.
    Request,
    /// The current row of a run's data file.
    IterationData,
    /// A value captured after a response.
    Runtime,
    /// `process.env`: the OS environment with the collection `.env` over it.
    ProcessEnv,
    /// A `{{$name}}` value generated on each send.
    Dynamic,
    /// A cached OAuth 2.0 token field, `$oauth2.<id>.<field>`.
    OAuth2,
}

/// What one placeholder resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VariableInfo {
    scope: VarScope,
    value: Option<String>,
    secret: bool,
}

impl VariableInfo {
    /// The layer the value comes from.
    pub fn scope(&self) -> VarScope {
        self.scope
    }

    /// The rendered value; `None` for a secret, a dynamic name, or anything that read a secret.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Whether the value is secret and so never carried.
    pub fn is_secret(&self) -> bool {
        self.secret
    }
}

/// One layer's definition of a variable, as listed by [`VariableSet::overview`].
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScopedVariable {
    name: String,
    scope: VarScope,
    value: Option<String>,
    secret: bool,
    shadowed: bool,
}

impl ScopedVariable {
    /// The variable's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The layer this definition is in.
    pub fn scope(&self) -> VarScope {
        self.scope
    }

    /// The effective row carries the rendered value a send uses; a shadowed row carries its
    /// own text as written. `None` when secret.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Whether the value is secret and so never carried.
    pub fn is_secret(&self) -> bool {
        self.secret
    }

    /// Whether a higher layer defines the name too, so a send does not use this row.
    pub fn is_shadowed(&self) -> bool {
        self.shadowed
    }
}

// Values may be secrets, so Debug shows no value.
impl fmt::Debug for ScopedVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedVariable")
            .field("name", &self.name)
            .field("scope", &self.scope)
            .field("secret", &self.secret)
            .field("shadowed", &self.shadowed)
            .finish_non_exhaustive()
    }
}

/// The variables a request can see. It reads the runtime scope live, so captures made after
/// it was built are seen.
#[derive(Clone)]
#[non_exhaustive]
pub struct VariableSet {
    ctx: VarContext,
    req: RequestFile,
}

impl VariableSet {
    pub(crate) fn new(ctx: VarContext, req: RequestFile) -> VariableSet {
        VariableSet { ctx, req }
    }

    /// What placeholder `name` (a lodash path, `$dynamic` or `process.env.X`) resolves to;
    /// `None` when sending it would fail as undefined.
    pub fn get(&self, name: &str) -> Option<VariableInfo> {
        // Dynamic names are substituted before any lookup, so no variable can shadow them.
        if is_dynamic(name) {
            return Some(VariableInfo {
                scope: VarScope::Dynamic,
                value: None,
                secret: false,
            });
        }
        let captured = self.ctx.runtime_vars().snapshot();
        let (vars, sensitive) = self.ctx.lookup_with(&self.req, &captured);
        let raw = js::to_text(lookup(name, &vars)?);
        // The key read is chosen as `Tracker::note` chooses it: literal first, then the path head.
        let key = if vars.contains_key(name) {
            name.to_owned()
        } else {
            to_path(name).into_iter().next()?
        };
        let scope = self.ctx.scope_of(&self.req, &key, &captured);
        let key_sensitive = sensitive.contains(&key);
        let tracker = self.ctx.tracker(sensitive);
        let generator = RefCell::new(self.ctx.generator());
        // A value that cannot expand (a cycle, an undefined inner name) is shown as written.
        let rendered = interpolate_tracked(&raw, &vars, Some(&tracker), &generator).unwrap_or(raw);
        // A value that carries one a script read as sensitive is secret even when copied whole.
        let tainted = self.ctx.runtime_vars().taints(&rendered);
        let secret = key_sensitive || !tracker.is_empty() || tainted;
        Some(VariableInfo {
            scope,
            value: (!secret).then_some(rendered),
            secret,
        })
    }

    /// Every definition of every plain name, per name highest layer first: the effective row
    /// is exactly what [`Self::get`] (and so a send) resolves, and each lower layer that
    /// defines the name too follows as a shadowed row. Secrets carry no value in any row.
    pub fn overview(&self) -> Vec<ScopedVariable> {
        let captured = self.ctx.runtime_vars().snapshot();
        let mut out = Vec::new();
        for name in self
            .names()
            .into_iter()
            .filter(|n| !n.contains('.') && !n.starts_with('$'))
        {
            let Some(effective) = self.get(&name) else {
                continue;
            };
            let top = effective.scope();
            out.push(ScopedVariable {
                name: name.clone(),
                scope: top,
                value: effective.value.clone(),
                secret: effective.secret,
                shadowed: false,
            });
            // The layer that wins is the first one of the effective scope; a scope above all
            // layers (iteration data, a token field) means every layer is shadowed.
            let mut winner_seen = false;
            for (scope, value) in self.ctx.layers_of(&self.req, &name, &captured) {
                let Some(value) = value else { continue };
                if scope == top && !winner_seen {
                    winner_seen = true;
                    continue;
                }
                let secret = match scope {
                    VarScope::Environment => self.ctx.env_name_sensitive(&name),
                    VarScope::GlobalEnvironment => self.ctx.global_name_sensitive(&name),
                    _ => false,
                };
                out.push(ScopedVariable {
                    name: name.clone(),
                    scope,
                    value: (!secret).then(|| js::to_text(&value)),
                    secret,
                    shadowed: true,
                });
            }
        }
        out
    }

    /// Decodes `text` as a JWT after expanding its variables, so a secret reference works.
    /// Only the decoded claims leave; the expanded text is dropped here, never returned.
    pub fn decode_jwt(&self, text: &str) -> Result<DecodedJwt, JwtError> {
        let captured = self.ctx.runtime_vars().snapshot();
        let (vars, _) = self.ctx.lookup_with(&self.req, &captured);
        let generator = RefCell::new(self.ctx.generator());
        let expanded =
            interpolate_tracked(text, &vars, None, &generator).map_err(|_| JwtError::Unresolved)?;
        decode_jwt(&expanded)
    }

    /// Names for autocomplete: plain names sorted, then the `.env` keys as `process.env.KEY`,
    /// then every dynamic name in table order.
    pub fn names(&self) -> Vec<String> {
        let (vars, _) = self.ctx.lookup_for(&self.req);
        let mut names: Vec<String> = vars
            .keys()
            .filter(|n| *n != "process" && !is_dynamic(n))
            .cloned()
            .collect();
        names.sort();
        // Only keys a plain dotted path can spell; others need bracket syntax nobody types.
        let mut dotenv: Vec<String> = self
            .ctx
            .dotenv_keys()
            .iter()
            .filter(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .map(|k| format!("process.env.{k}"))
            .collect();
        dotenv.sort();
        dotenv.dedup();
        names.extend(dotenv);
        names.extend(dynamic_variables().iter().map(|n| format!("${n}")));
        names
    }

    /// Changes whenever the runtime scope or the script overlay this set reads changes; cache anything derived
    /// from `get` on the set's identity and this number.
    pub fn revision(&self) -> u64 {
        let overlay = self.ctx.env_overlay().revision();
        self.ctx.runtime_vars().revision().wrapping_add(overlay)
    }

    /// Whether `other` was built for the same request path and the same `runtime.variables`
    /// rows, compared as written; the rest of the request never reaches a lookup.
    pub fn same_request_inputs(&self, other: &VariableSet) -> bool {
        let rows = |set: &VariableSet| set.req.document().get(&["runtime", "variables"]).cloned();
        self.req.path() == other.req.path() && rows(self) == rows(other)
    }
}

/// Whether `name` is `$` followed by a name the dynamic table generates.
fn is_dynamic(name: &str) -> bool {
    name.strip_prefix('$')
        .is_some_and(|rest| dynamic_variables().contains(&rest))
}

// Values may be secrets, so Debug shows names only.
impl fmt::Debug for VariableSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableSet")
            .field("request", &self.req.path())
            .field("names", &self.names())
            .finish_non_exhaustive()
    }
}
