//! `generate_code`: a resolved request as a code snippet, with every secret withheld.

use std::collections::BTreeMap;

use apinata_core::{ResolveError, ResolvedRequest, Target, VarContext, generate, resolve};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::redact::Redactor;
use crate::send;

/// `generate_code`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerateArgs {
    /// The request's item path, such as `users/get.yml`.
    pub path: String,
    /// The language or tool to write: `curl`, `python-requests` and the other ids `apinata gen`
    /// takes.
    pub target: String,
    /// The environment to resolve against; the server's default when absent.
    pub environment: Option<String>,
    /// Runtime variable overrides, applied over the environment.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

/// What `generate_code` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct GeneratedCode {
    pub target: String,
    /// The snippet, with secret values replaced by `<secret>`.
    pub code: String,
    /// What the snippet cannot reproduce, as `apinata gen` prints it: scripts that were not run,
    /// for one. Empty when there is nothing to say.
    pub warnings: Vec<String>,
}

/// The target `id` names, or the error listing the valid ids.
pub(crate) fn target(id: &str) -> Result<Target, String> {
    id.parse::<Target>().map_err(|e| e.to_string())
}

/// Whether `err` only needs a cached OAuth 2.0 token loaded into memory.
pub(crate) fn needs_cached_token(err: &ResolveError) -> bool {
    matches!(err, ResolveError::OAuth2NoToken { .. })
}

/// The snippet for `req`, scrubbed of its sensitive values. The values join `learned` so the
/// session redactor knows them too.
pub(crate) fn render(req: &ResolvedRequest, target: Target, learned: &mut Redactor) -> String {
    learned.add_values(req.sensitive_values());
    learned.add_basic_auth(req.headers());
    learned.add_basic_auth(&send::url_basic_auth(req.url()));
    learned.scrub(&generate(req, target))
}

/// Resolve `file` under `ctx` once, for the caller to retry after loading a cached token.
pub(crate) fn resolve_once(
    file: &apinata_core::RequestFile,
    ctx: &VarContext,
) -> Result<ResolvedRequest, ResolveError> {
    resolve(file, ctx)
}
