//! GraphQL schema introspection: the introspection request, its fetch, the schema model
//! parsed from the answer or from an SDL file, and the errors a schema load can end in.

mod complete;
mod model;
mod print;
mod sdl;

pub use complete::{Completion, CompletionKind};
pub use model::{EnumValue, Field, InputValue, Schema, SchemaType, TypeKind, TypeRef};

use std::sync::Arc;
use std::time::Duration;

use crate::execute::SendTag;
use crate::{
    AuthorizeOptions, CancelHandle, ExecError, Executor, OAuth2Error, RequestFile, RequestKind,
    ResolveError, ResolvedRequest, SendSource, VarContext,
};

const MIB: u64 = 1024 * 1024;

// graphql-js 16.10.0 getIntrospectionQuery() with default options, byte for byte. MIT, Copyright (c) GraphQL Contributors; see licenses/graphql-js-MIT.txt.
/// The introspection query POSTed to load a schema.
pub const INTROSPECTION_QUERY: &str = include_str!("introspection-16.10.0.graphql");

/// The largest introspection answer read, in bytes; a longer one is [`SchemaError::TooLarge`].
pub const SCHEMA_BODY_LIMIT: u64 = 32 * MIB;

/// The timeout of an introspection request whose settings set no limit.
pub const SCHEMA_TIMEOUT: Duration = Duration::from_secs(30);

/// Builds the introspection request for the GraphQL request `req`: resolved like Send, then a
/// JSON POST of [`INTROSPECTION_QUERY`] with a [`SCHEMA_TIMEOUT`] when the request has no limit.
///
/// Fails with [`SchemaError::NotGraphql`] for an HTTP request and [`SchemaError::Resolve`] when
/// the URL, headers or auth cannot be resolved.
pub fn introspection_request(
    req: &RequestFile,
    ctx: &VarContext,
) -> Result<ResolvedRequest, SchemaError> {
    if req.kind() != RequestKind::Graphql {
        return Err(SchemaError::NotGraphql);
    }
    let query = serde_json::to_string(INTROSPECTION_QUERY).expect("a string serializes");
    let body = format!("{{\"query\":{query}}}");
    crate::resolve::introspection(req, ctx, body, SCHEMA_TIMEOUT).map_err(SchemaError::Resolve)
}

/// Sends `req` with `executor` and parses the answer into a [`Schema`].
///
/// Fails with [`SchemaError::Send`], then [`SchemaError::Status`] for a status other than 200,
/// [`SchemaError::TooLarge`] for a truncated answer, then as [`Schema::from_json`] does.
pub async fn fetch_schema(
    executor: &Executor,
    req: &ResolvedRequest,
    cancel: &CancelHandle,
) -> Result<Schema, SchemaError> {
    fetch_introspection(executor, req, cancel)
        .await
        .map(Introspection::into_schema)
}

/// A checked introspection answer: the JSON text as received and the schema parsed from it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Introspection {
    text: String,
    schema: Schema,
}

impl Introspection {
    /// The answer's JSON text, exactly as the server sent it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The schema the answer describes.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The schema, dropping the text.
    pub fn into_schema(self) -> Schema {
        self.schema
    }
}

/// The guard [`introspect`] asks before sending; `Some` refuses with that message.
pub type IntrospectGuard = Arc<dyn Fn(&ResolvedRequest) -> Option<String> + Send + Sync>;

/// How [`introspect`] obtains a token and whether a guard may refuse the send.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct IntrospectOptions {
    authorize: AuthorizeOptions,
    guard: Option<IntrospectGuard>,
}

impl IntrospectOptions {
    /// Default token handling and no guard.
    pub fn new() -> IntrospectOptions {
        IntrospectOptions::default()
    }

    /// How an OAuth 2.0 token is obtained, as for a send.
    pub fn authorize(mut self, how: AuthorizeOptions) -> IntrospectOptions {
        self.authorize = how;
        self
    }

    /// Ask `guard` about the resolved request; a message it returns refuses the send.
    pub fn guard(mut self, guard: IntrospectGuard) -> IntrospectOptions {
        self.guard = Some(guard);
        self
    }
}

impl std::fmt::Debug for IntrospectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntrospectOptions")
            .field("guard", &self.guard.is_some())
            .finish_non_exhaustive()
    }
}

/// Fetches the schema of the GraphQL request `req` as Send would send it: its OAuth 2.0 token
/// prepared first, then [`introspection_request`], the guard, and [`fetch_introspection`].
///
/// Fails with [`SchemaError::NotGraphql`] before anything else, [`SchemaError::Auth`] when the
/// token cannot be had, [`SchemaError::Refused`] when the guard refuses, then as the two steps do.
pub async fn introspect(
    executor: &Executor,
    req: &RequestFile,
    ctx: &VarContext,
    options: &IntrospectOptions,
    cancel: &CancelHandle,
) -> Result<Introspection, SchemaError> {
    if req.kind() != RequestKind::Graphql {
        return Err(SchemaError::NotGraphql);
    }
    executor
        .prepare_auth(req, ctx, &options.authorize, cancel)
        .await
        .map_err(SchemaError::Auth)?;
    let resolved = introspection_request(req, ctx)?;
    if let Some(message) = options.guard.as_ref().and_then(|guard| guard(&resolved)) {
        return Err(SchemaError::Refused { message });
    }
    fetch_introspection(executor, &resolved, cancel).await
}

/// Sends `req` with `executor` and checks the answer, keeping its text beside the [`Schema`].
///
/// Fails as [`fetch_schema`] does.
pub async fn fetch_introspection(
    executor: &Executor,
    req: &ResolvedRequest,
    cancel: &CancelHandle,
) -> Result<Introspection, SchemaError> {
    let tag = SendTag {
        source: SendSource::GraphqlSchema,
        ..SendTag::default()
    };
    let response = executor
        .send_as(req, cancel, &tag)
        .await
        .map_err(SchemaError::Send)?;
    if response.status() != 200 {
        return Err(SchemaError::Status {
            status: response.status(),
            text: response.status_text().to_owned(),
        });
    }
    if response.truncated() {
        return Err(SchemaError::TooLarge {
            limit: SCHEMA_BODY_LIMIT,
        });
    }
    let text = response.text().to_owned();
    let schema = Schema::from_json(&text)?;
    Ok(Introspection { text, schema })
}

impl Schema {
    /// Parses an SDL schema document such as `type Query { me: User }`.
    ///
    /// Roots follow a `schema` block, else the types named Query, Mutation and Subscription;
    /// the built-in scalars are present even when undeclared. Fails with [`SchemaError::Sdl`].
    pub fn from_sdl(text: &str) -> Result<Schema, SchemaError> {
        sdl::parse(text)
    }

    /// Writes the schema as an SDL document that [`Schema::from_sdl`] reads back to the same
    /// types. Introspection types and the built-in scalars are left out, and directive
    /// definitions are not part of the model, so only `@deprecated` is written.
    pub fn to_sdl(&self) -> String {
        print::print(self)
    }

    /// Parses a schema file: introspection JSON when the text starts with `{`, SDL otherwise.
    pub fn from_text(text: &str) -> Result<Schema, SchemaError> {
        let body = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if body.starts_with('{') {
            Schema::from_json(body)
        } else {
            Schema::from_sdl(text)
        }
    }
}

/// Why a GraphQL schema could not be loaded or parsed.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// The request is not a GraphQL request.
    #[error("only a GraphQL request has a schema")]
    NotGraphql,
    /// The introspection request could not be resolved.
    #[error(transparent)]
    Resolve(ResolveError),
    /// The introspection request could not be sent or its answer not read.
    #[error(transparent)]
    Send(ExecError),
    /// The server answered with a status other than 200.
    #[error("the server answered {status} {text}")]
    Status {
        /// The HTTP status code.
        status: u16,
        /// The status text.
        text: String,
    },
    /// The answer was cut off at the schema size limit.
    #[error("the response is larger than the {} MiB schema limit", limit / MIB)]
    TooLarge {
        /// The limit in bytes.
        limit: u64,
    },
    /// The text is not JSON.
    #[error("the text is not JSON: {detail}")]
    NotJson {
        /// What the JSON parser reported.
        detail: String,
    },
    /// The JSON holds no `__schema` object.
    #[error("no schema in the response")]
    NoData,
    /// The JSON holds no `__schema` object but a non-empty `errors` list.
    #[error("the server answered with GraphQL errors: {}", messages.join("; "))]
    Graphql {
        /// Each error's message, in order.
        messages: Vec<String>,
    },
    /// The OAuth 2.0 token the request needs could not be had.
    #[error(transparent)]
    Auth(OAuth2Error),
    /// The caller's guard refused the resolved request, so nothing was sent.
    #[error("{message}")]
    Refused {
        /// Why the guard refused it.
        message: String,
    },
    /// The text is not a valid SDL schema document.
    #[error("the schema is not valid SDL at line {line}, column {column}: {detail}")]
    Sdl {
        /// The 1-based line of the problem.
        line: usize,
        /// The 1-based column of the problem, in characters.
        column: usize,
        /// What is wrong.
        detail: String,
    },
    /// The `__schema` object does not have the introspection shape.
    #[error("the schema is malformed: {detail}")]
    Invalid {
        /// What is wrong, naming the type where one applies.
        detail: String,
    },
}
