//! `graphql_format` and `graphql_validate`: a GraphQL query formatted or checked, from raw text
//! or from a GraphQL request's saved query. Nothing is sent.

use apinata_core::{
    BodySpec, Collection, CompiledSchema, QuerySeverity, QuerySyntaxError, RequestKind,
    SCHEMA_BODY_LIMIT, Schema, check_query, format_query,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::send;
use crate::writes::{self, Failure};

const ONE_OF: &str = "give exactly one of `path` and `query`";

/// `graphql_format`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GraphqlFormatArgs {
    /// A GraphQL request's item path, such as `api/users.yml`; its saved query is formatted.
    pub path: Option<String>,
    /// Raw query text to format; nothing is saved.
    pub query: Option<String>,
}

/// What `graphql_format` returns; exactly one field is present.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct GraphqlFormatDoc {
    /// With `path`: whether the saved query changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<bool>,
    /// With `query`: the formatted text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

/// `graphql_validate`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GraphqlValidateArgs {
    /// A GraphQL request's item path, such as `api/users.yml`; its saved query is checked.
    pub path: Option<String>,
    /// Raw query text to check.
    pub query: Option<String>,
    /// A schema as SDL or an introspection JSON answer; without it only syntax is checked.
    pub schema: Option<String>,
}

/// What `graphql_validate` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct GraphqlValidateDoc {
    /// Whether the query was checked against a schema.
    pub schema_checked: bool,
    /// The findings in text order, at most 200 and then an `info` entry counting the rest.
    pub diagnostics: Vec<QueryDiagnosticDoc>,
}

/// One finding in a query.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct QueryDiagnosticDoc {
    /// Byte offset where the finding starts.
    pub start: usize,
    /// Byte offset where it ends.
    pub end: usize,
    /// 1-based line of `start`.
    pub line: usize,
    /// 1-based column of `start`, counted in characters.
    pub column: usize,
    /// `error`, `warning` or `info`.
    pub severity: String,
    /// What is wrong.
    pub message: String,
}

/// Which of `path` and `query` was given; both or neither is refused.
pub(crate) enum Source {
    Path(String),
    Query(String),
}

pub(crate) fn source(path: Option<String>, query: Option<String>) -> Result<Source, String> {
    match (path, query) {
        (Some(p), None) => Ok(Source::Path(p)),
        (None, Some(q)) => Ok(Source::Query(q)),
        _ => Err(ONE_OF.to_owned()),
    }
}

/// The 1-based line and char column of byte `offset`; `\n`, `\r\n` and a lone `\r` each end a
/// line, as the GraphQL lexer counts them.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let (mut line, mut col) = (1, 1);
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if i >= offset {
            break;
        }
        let lone_cr = c == '\r' && chars.peek().is_none_or(|&(_, next)| next != '\n');
        if c == '\n' || lone_cr {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// A format failure as the client reads it; a syntax error carries its position.
fn format_error(text: &str, e: &QuerySyntaxError, wrote_nothing: bool) -> String {
    let tail = if wrote_nothing {
        "; nothing was written"
    } else {
        ""
    };
    match e {
        QuerySyntaxError::Syntax { range, message } => {
            let (line, col) = position(text, range.start);
            format!("syntax error at {line}:{col}: {message}{tail}")
        }
        other => format!("{other}{tail}"),
    }
}

/// The saved query and variables of the GraphQL request at `item`.
fn request_query(c: &Collection, item: &str) -> Result<(String, String), String> {
    let file = send::request_file(c, item)?;
    match file.body() {
        BodySpec::Graphql { query, variables } if file.kind() == RequestKind::Graphql => {
            Ok((query, variables))
        }
        _ => Err(format!("`{item}` is not a GraphQL request")),
    }
}

/// Format the saved query of request `item`, writing it back only when it changed.
pub(crate) fn format_saved(c: &mut Collection, item: &str) -> Result<GraphqlFormatDoc, String> {
    let path = writes::item_path(item)?;
    let changed = writes::write_retrying(c, |c| {
        let (query, variables) = request_query(c, item).map_err(Failure::refused)?;
        let formatted =
            format_query(&query).map_err(|e| Failure::refused(format_error(&query, &e, true)))?;
        if formatted == query {
            return Ok(false);
        }
        let r = c
            .request_mut(&path)
            .ok_or_else(|| Failure::refused(format!("no request at {item}")))?;
        let file = r.path().to_owned();
        r.set_body(BodySpec::Graphql {
            query: formatted,
            variables,
        });
        r.save().map_err(|e| match Failure::save(e) {
            Failure::Failed { message, .. } => Failure::at(&file, message),
            conflict => conflict,
        })?;
        Ok(true)
    })?;
    Ok(GraphqlFormatDoc {
        changed: Some(changed),
        query: None,
    })
}

/// Format raw `text`.
pub(crate) fn format_text(text: &str) -> Result<GraphqlFormatDoc, String> {
    let query = format_query(text).map_err(|e| format_error(text, &e, false))?;
    Ok(GraphqlFormatDoc {
        changed: None,
        query: Some(query),
    })
}

/// The compiled form of `text`, an SDL or introspection schema; errors name `schema`.
pub(crate) fn compile(text: &str) -> Result<CompiledSchema, String> {
    if text.len() as u64 > SCHEMA_BODY_LIMIT {
        return Err(format!(
            "`schema` is larger than {} bytes",
            SCHEMA_BODY_LIMIT
        ));
    }
    let schema = Schema::from_text(text).map_err(|e| format!("`schema` is not usable: {e}"))?;
    CompiledSchema::new(&schema).map_err(|e| format!("`schema` is not usable: {e}"))
}

/// The saved query of request `item` in the reloaded `c`.
pub(crate) fn saved_query(c: &mut Collection, item: &str) -> Result<String, String> {
    request_query(c, item).map(|(query, _)| query)
}

fn severity(s: QuerySeverity) -> &'static str {
    match s {
        QuerySeverity::Error => "error",
        QuerySeverity::Warning => "warning",
        _ => "info",
    }
}

/// Check `text` against `schema` and shape the findings; core already caps the list.
pub(crate) fn validate(text: &str, schema: Option<&CompiledSchema>) -> GraphqlValidateDoc {
    let check = check_query(text, schema);
    let diagnostics: Vec<QueryDiagnosticDoc> = check
        .diagnostics()
        .iter()
        .map(|d| {
            let range = d.range();
            let (line, column) = position(text, range.start);
            QueryDiagnosticDoc {
                start: range.start,
                end: range.end,
                line,
                column,
                severity: severity(d.severity()).to_owned(),
                message: d.message().to_owned(),
            }
        })
        .collect();
    GraphqlValidateDoc {
        schema_checked: check.schema_checked(),
        diagnostics,
    }
}
