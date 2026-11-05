//! `graphql_schema`: a GraphQL request's schema fetched by introspection under the send rules,
//! returned as SDL or as the JSON answer, cut to a byte budget.

use std::collections::BTreeMap;

use apinata_core::{
    ApprovalInbox, Introspection, OAuth2Error, RequestFile, SchemaError, VarContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::policy::HostTrust;
use crate::redact::Redactor;
use crate::send::{self, DEFAULT_BODY_BYTES, MAX_BODY_BYTES};

/// `graphql_schema`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GraphqlSchemaArgs {
    /// The GraphQL request's item path, such as `api/users.yml`.
    pub path: String,
    /// The environment to resolve against; the server's default when absent.
    pub environment: Option<String>,
    /// Runtime variable overrides, applied over the environment.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// `sdl` (the default) for the schema as an SDL document, or `json` for the introspection
    /// answer as the server sent it.
    pub format: Option<String>,
    /// The most text to return, in bytes; 32 KiB when absent, 1 MiB at most.
    pub max_bytes: Option<u64>,
}

/// What `graphql_schema` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct GraphqlSchemaDoc {
    /// The query root type's name; absent when the schema has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_type: Option<String>,
    /// The mutation root type's name; absent when the schema has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_type: Option<String>,
    /// The subscription root type's name; absent when the schema has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    /// The count of named types, introspection and built-in types included.
    pub type_count: usize,
    /// `sdl` or `json`, as asked.
    pub format: String,
    /// The schema text, cut to `max_bytes`.
    pub text: String,
    /// The whole text's size in bytes.
    pub total_bytes: u64,
    /// True when `text` was cut to the byte budget.
    pub truncated: bool,
}

/// The output format, refusing anything but `sdl` and `json` before a byte is sent.
pub(crate) fn format(asked: Option<&str>) -> Result<&'static str, String> {
    match asked.unwrap_or("sdl") {
        "sdl" => Ok("sdl"),
        "json" => Ok("json"),
        other => Err(format!(
            "`format` is `{other}`; it must be `sdl` or `json`; nothing was sent"
        )),
    }
}

/// The tool result for `answer` in `format`, scrubbed by `redactor` and cut to `max_bytes`.
pub(crate) fn doc(
    answer: &Introspection,
    format: &str,
    max_bytes: Option<u64>,
    redactor: &Redactor,
) -> Result<GraphqlSchemaDoc, String> {
    let schema = answer.schema();
    let text = match format {
        "json" => answer.text().to_owned(),
        _ => schema
            .to_sdl()
            .map_err(|e| format!("{e}; ask for `format` `json` instead"))?,
    };
    let max = max_bytes.unwrap_or(DEFAULT_BODY_BYTES).min(MAX_BODY_BYTES);
    let (shown, truncated) =
        redactor.scrub_cut(&text, usize::try_from(max).unwrap_or(usize::MAX), false);
    Ok(GraphqlSchemaDoc {
        query_type: schema.query_type().map(str::to_owned),
        mutation_type: schema.mutation_type().map(str::to_owned),
        subscription_type: schema.subscription_type().map(str::to_owned),
        type_count: schema.types().len(),
        format: format.to_owned(),
        text: shown,
        total_bytes: text.len() as u64,
        truncated,
    })
}

/// A failed fetch as one line. A refused host goes to the trust policy, which files the same
/// approval a refused send of `item` would.
pub(crate) fn message(
    err: SchemaError,
    item: &str,
    file: &RequestFile,
    ctx: &VarContext,
    trust: &HostTrust,
    inbox: Option<&ApprovalInbox>,
    redactor: &Redactor,
) -> String {
    match err {
        SchemaError::Send(e) | SchemaError::Auth(OAuth2Error::Exec(e)) => {
            trust.refusal(&e, item, inbox, redactor)
        }
        SchemaError::Resolve(e) => send::resolve_message(&e, file, ctx),
        SchemaError::Auth(e) => send::oauth2_message(&e, file, ctx),
        SchemaError::Refused { message } => message,
        e @ SchemaError::NotGraphql => format!("{e}; nothing was sent"),
        other => other.to_string(),
    }
}
