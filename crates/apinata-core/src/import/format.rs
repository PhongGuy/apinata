//! The import formats each surface offers, named by stable ids, detected from the input and
//! dispatched to their parsers. A new importer adds a variant and its match arms here, so the
//! CLI, the MCP server and the app pick it up without changes of their own.

use super::har::parse_har;
use super::hoppscotch::{parse_hoppscotch, parse_hoppscotch_environments};
use super::insomnia::parse_insomnia;
use super::load::{Loaded, load, strip_bom};
use super::openapi::parse_openapi;
use super::{
    CurlError, DocumentError, ImportNote, ImportedCollection, ImportedEnvironment, ImportedRequest,
    NoteKind, PostmanError, parse_curl, parse_postman, parse_postman_environment,
};
use serde_json::{Map, Value};

/// A format a whole collection can be imported from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CollectionFormat {
    /// A Postman collection, v2.0 or v2.1.
    Postman,
    /// An OpenAPI 3 or Swagger 2.0 description.
    OpenApi,
    /// An Insomnia export, v4 or v5.
    Insomnia,
    /// A Hoppscotch collection export.
    Hoppscotch,
    /// A HAR 1.2 capture.
    Har,
}

/// A format an environment can be imported from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvironmentFormat {
    /// A Postman environment export.
    Postman,
    /// A Hoppscotch environment export.
    Hoppscotch,
}

/// A format a single request can be imported from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestFormat {
    /// A curl command line in POSIX shell syntax.
    Curl,
}

impl CollectionFormat {
    /// Every format, in the order detection tries them.
    pub const ALL: &'static [CollectionFormat] = &[
        CollectionFormat::Postman,
        CollectionFormat::OpenApi,
        CollectionFormat::Insomnia,
        CollectionFormat::Hoppscotch,
        CollectionFormat::Har,
    ];

    /// The stable id used on the command line and in tool arguments.
    pub fn id(self) -> &'static str {
        match self {
            CollectionFormat::Postman => "postman",
            CollectionFormat::OpenApi => "openapi",
            CollectionFormat::Insomnia => "insomnia",
            CollectionFormat::Hoppscotch => "hoppscotch",
            CollectionFormat::Har => "har",
        }
    }

    /// A human-readable name for messages and menus.
    pub fn label(self) -> &'static str {
        match self {
            CollectionFormat::Postman => "Postman collection v2.0 or v2.1",
            CollectionFormat::OpenApi => "OpenAPI 3 or Swagger 2.0 description",
            CollectionFormat::Insomnia => "Insomnia export v4 or v5",
            CollectionFormat::Hoppscotch => "Hoppscotch collection export",
            CollectionFormat::Har => "HAR 1.2 capture",
        }
    }

    /// The format with this id, if any.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.id() == id)
    }

    /// The first format whose shape the input has. Input that does not load, such as input over
    /// a size limit, is never detected.
    pub fn detect(input: &[u8]) -> Option<Self> {
        load(input).ok().and_then(|l| Self::detect_value(&l.value))
    }

    /// The first format whose shape the loaded document has.
    pub(crate) fn detect_value(root: &Value) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.matches(root))
    }

    /// Whether the loaded document has this format's shape. The rules only classify; the parser
    /// validates and reports versions it cannot import.
    fn matches(self, root: &Value) -> bool {
        let obj = root.as_object();
        let has = |key: &str| obj.is_some_and(|o| o.contains_key(key));
        match self {
            CollectionFormat::Postman => is_postman_collection(root),
            CollectionFormat::OpenApi => has("openapi") || has("swagger"),
            CollectionFormat::Insomnia => is_insomnia(root),
            CollectionFormat::Hoppscotch => every_object(root, is_hoppscotch_collection),
            CollectionFormat::Har => root
                .get("log")
                .filter(|log| log.is_object())
                .and_then(|log| log.get("entries"))
                .is_some_and(Value::is_array),
        }
    }
}

fn is_postman_collection(root: &Value) -> bool {
    // Unwrap the export envelope, then accept a v2 `info` block or a v1 `requests` and `order`
    // pair, so that an old export reaches the parser and gets its schema error.
    let root = match root.get("collection") {
        Some(inner @ Value::Object(_)) => inner,
        _ => root,
    };
    let info_is_postman = root
        .get("info")
        .and_then(Value::as_object)
        .is_some_and(|info| {
            info.contains_key("_postman_id")
                || info
                    .get("schema")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.contains("postman"))
        });
    let v1 = is_array_at(root, "requests") && is_array_at(root, "order");
    info_is_postman || v1
}

/// A v4 export envelope, or a v5 document whose `type` reads `<letters>.insomnia[.rest]/5.`.
fn is_insomnia(root: &Value) -> bool {
    let v4 = root.get("_type").and_then(Value::as_str) == Some("export")
        && root.get("__export_format").is_some_and(Value::is_number);
    let v5 = root
        .get("type")
        .and_then(Value::as_str)
        .and_then(|t| t.split_once('/'))
        .is_some_and(|(kind, version)| {
            let prefix = kind
                .strip_suffix(".insomnia.rest")
                .or_else(|| kind.strip_suffix(".insomnia"));
            prefix.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphabetic()))
                && version.starts_with("5.")
        });
    v4 || v5
}

fn is_hoppscotch_collection(o: &Map<String, Value>) -> bool {
    let arrays = ["requests", "folders"]
        .iter()
        .all(|k| o.get(*k).is_some_and(Value::is_array));
    arrays && !o.contains_key("order") && o.get("v").is_none_or(is_integer)
}

fn is_hoppscotch_environment(o: &Map<String, Value>) -> bool {
    o.get("variables").is_some_and(Value::is_array)
        && !o.contains_key("requests")
        && !o.contains_key("folders")
        && (o.contains_key("name") || o.get("v").is_some_and(is_integer))
}

/// The root object passes `rule`, or the root is a non-empty array of objects that all pass it.
fn every_object(root: &Value, rule: fn(&Map<String, Value>) -> bool) -> bool {
    match root {
        Value::Object(o) => rule(o),
        Value::Array(items) => {
            !items.is_empty() && items.iter().all(|i| i.as_object().is_some_and(rule))
        }
        _ => false,
    }
}

fn is_array_at(root: &Value, key: &str) -> bool {
    root.get(key).is_some_and(Value::is_array)
}

fn is_integer(v: &Value) -> bool {
    v.is_i64() || v.is_u64()
}

impl EnvironmentFormat {
    /// Every format, in the order detection tries them.
    pub const ALL: &'static [EnvironmentFormat] =
        &[EnvironmentFormat::Postman, EnvironmentFormat::Hoppscotch];

    /// The stable id used on the command line and in tool arguments.
    pub fn id(self) -> &'static str {
        match self {
            EnvironmentFormat::Postman => "postman",
            EnvironmentFormat::Hoppscotch => "hoppscotch",
        }
    }

    /// A human-readable name for messages and menus.
    pub fn label(self) -> &'static str {
        match self {
            EnvironmentFormat::Postman => "Postman environment",
            EnvironmentFormat::Hoppscotch => "Hoppscotch environment export",
        }
    }

    /// The format with this id, if any.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.id() == id)
    }

    /// The first format whose shape the input has. Input that does not load, such as input over
    /// a size limit, is never detected.
    pub fn detect(input: &[u8]) -> Option<Self> {
        load(input).ok().and_then(|l| Self::detect_value(&l.value))
    }

    /// The first format whose shape the loaded document has.
    pub(crate) fn detect_value(root: &Value) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.matches(root))
    }

    fn matches(self, root: &Value) -> bool {
        match self {
            EnvironmentFormat::Postman => {
                // Postman globals share the environment shape but carry a different scope, and
                // a `v` key marks a Hoppscotch file.
                let scoped = match root.get("_postman_variable_scope") {
                    None => true,
                    Some(scope) => scope.as_str() == Some("environment"),
                };
                let shaped = is_array_at(root, "values") && root.get("name").is_some();
                shaped && scoped && root.get("v").is_none()
            }
            EnvironmentFormat::Hoppscotch => every_object(root, is_hoppscotch_environment),
        }
    }
}

impl RequestFormat {
    /// Every format, in the order detection tries them.
    pub const ALL: &'static [RequestFormat] = &[RequestFormat::Curl];

    /// The stable id used on the command line and in tool arguments.
    pub fn id(self) -> &'static str {
        match self {
            RequestFormat::Curl => "curl",
        }
    }

    /// A human-readable name for messages and menus.
    pub fn label(self) -> &'static str {
        match self {
            RequestFormat::Curl => "curl command (POSIX shell)",
        }
    }

    /// The format with this id, if any.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.id() == id)
    }

    /// The first format whose shape the input has, without fully parsing it.
    pub fn detect(input: &str) -> Option<Self> {
        // The first word must be `curl` or a path to it, and something must follow it.
        let text = input.trim_start();
        let end = text.find(char::is_whitespace)?;
        let word = &text[..end];
        (word == "curl" || word.ends_with("/curl")).then_some(RequestFormat::Curl)
    }
}

impl NoteKind {
    /// The stable id used in machine-readable output.
    pub fn id(self) -> &'static str {
        match self {
            NoteKind::Dropped => "dropped",
            NoteKind::Converted => "converted",
            NoteKind::Renamed => "renamed",
            NoteKind::NeedsInput => "needs-input",
        }
    }
}

/// A single request parsed from any request format, with its notes.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ParsedRequest {
    /// The request.
    pub request: ImportedRequest,
    /// What was not carried over unchanged, in input order.
    pub notes: Vec<ImportNote>,
}

/// Why an input could not be parsed in the detected or named format.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    /// Detection matched no format.
    #[error("not a recognized {kind} format; supported: {supported}")]
    Unrecognized {
        /// What was being imported: `collection`, `environment` or `request`.
        kind: &'static str,
        /// The supported formats' labels, joined by `, `.
        supported: String,
    },
    /// A Postman input is malformed or unsupported.
    #[error(transparent)]
    Postman(#[from] PostmanError),
    /// A curl command is malformed.
    #[error(transparent)]
    Curl(#[from] CurlError),
    /// An OpenAPI or Swagger input is malformed or unsupported.
    #[error("OpenAPI: {0}")]
    OpenApi(DocumentError),
    /// An Insomnia input is malformed or unsupported.
    #[error("Insomnia: {0}")]
    Insomnia(DocumentError),
    /// A Hoppscotch input is malformed or unsupported.
    #[error("Hoppscotch: {0}")]
    Hoppscotch(DocumentError),
    /// A HAR input is malformed or unsupported.
    #[error("HAR: {0}")]
    Har(DocumentError),
    /// A singular import was given input holding more than one collection or environment.
    #[error(
        "the input holds {collections} collections and {environments} environments, but this import takes one"
    )]
    Multiple {
        /// How many collections the input holds.
        collections: usize,
        /// How many environments the input holds.
        environments: usize,
    },
    /// The input exceeds a size limit.
    #[error("the input is larger than {limit} {unit}")]
    TooLarge {
        /// The limit exceeded.
        limit: u64,
        /// What the limit counts: `bytes`, `values` or `events`.
        unit: &'static str,
    },
}

/// One collection parsed from an import, with the environments that came with it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportBundle {
    /// The collection.
    pub collection: ImportedCollection,
    /// Its environments, in input order.
    pub environments: Vec<ImportedEnvironment>,
}

/// How an OpenAPI import groups operations into folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum GroupBy {
    /// One folder per tag.
    #[default]
    Tags,
    /// One folder per leading path segment.
    Path,
}

/// Choices a collection import offers beyond the input itself.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CollectionOptions {
    /// How OpenAPI requests are grouped into folders.
    pub group_by: GroupBy,
    /// The collection name for a HAR log, which has none of its own.
    pub name: Option<String>,
}

fn unrecognized<F: Copy>(
    kind: &'static str,
    all: &[F],
    label: fn(F) -> &'static str,
) -> FormatError {
    let supported = all.iter().map(|&f| label(f)).collect::<Vec<_>>().join(", ");
    FormatError::Unrecognized { kind, supported }
}

/// Load `input` once for detection and dispatch. With no named format, or for Postman, which
/// has no `DocumentError` of its own, a size or budget failure ends the import as `TooLarge`;
/// any other load error is left for the format to report.
fn load_bounded(
    input: &[u8],
    own_errors: bool,
) -> Result<Result<Loaded, DocumentError>, FormatError> {
    match load(input) {
        Err(DocumentError::TooLarge { limit, unit }) if !own_errors => {
            Err(FormatError::TooLarge { limit, unit })
        }
        loaded => Ok(loaded),
    }
}

/// Run a document parser on the loaded input, reporting a load or parse failure as `wrap`.
fn run<T>(
    loaded: Result<Loaded, DocumentError>,
    wrap: fn(DocumentError) -> FormatError,
    parse: impl FnOnce(Loaded) -> Result<T, DocumentError>,
) -> Result<T, FormatError> {
    loaded.and_then(parse).map_err(wrap)
}

/// Run a Postman parser on `input` without its BOM, shifting a parse offset back onto the
/// input as given.
fn postman_with_bom<T>(
    input: &[u8],
    parse: fn(&[u8]) -> Result<T, PostmanError>,
) -> Result<T, PostmanError> {
    let json = strip_bom(input);
    parse(json).map_err(|e| match e {
        PostmanError::Parse { offset, message } => PostmanError::Parse {
            offset: offset + (input.len() - json.len()),
            message,
        },
        e => e,
    })
}

/// Parse every collection in `input`, in `format` or in the detected format when `None`. Never
/// returns an empty list.
pub fn parse_collections(
    format: Option<CollectionFormat>,
    input: &[u8],
    options: &CollectionOptions,
) -> Result<Vec<ImportBundle>, FormatError> {
    let own_errors = format.is_some_and(|f| f != CollectionFormat::Postman);
    let loaded = load_bounded(input, own_errors)?;
    let format = format
        .or_else(|| {
            let loaded = loaded.as_ref().ok()?;
            CollectionFormat::detect_value(&loaded.value)
        })
        .ok_or_else(|| {
            unrecognized("collection", CollectionFormat::ALL, CollectionFormat::label)
        })?;
    match format {
        CollectionFormat::Postman => Ok(vec![ImportBundle {
            collection: postman_with_bom(input, parse_postman)?,
            environments: Vec::new(),
        }]),
        CollectionFormat::OpenApi => {
            run(loaded, FormatError::OpenApi, |l| parse_openapi(l, options))
        }
        CollectionFormat::Insomnia => run(loaded, FormatError::Insomnia, |l| {
            parse_insomnia(l, options)
        }),
        CollectionFormat::Hoppscotch => run(loaded, FormatError::Hoppscotch, |l| {
            parse_hoppscotch(l, options)
        }),
        CollectionFormat::Har => run(loaded, FormatError::Har, |l| parse_har(l, options)),
    }
}

/// Parse every environment in `input`, in `format` or in the detected format when `None`.
pub fn parse_environments(
    format: Option<EnvironmentFormat>,
    input: &[u8],
) -> Result<Vec<ImportedEnvironment>, FormatError> {
    let own_errors = format.is_some_and(|f| f != EnvironmentFormat::Postman);
    let loaded = load_bounded(input, own_errors)?;
    let format = format
        .or_else(|| {
            let loaded = loaded.as_ref().ok()?;
            EnvironmentFormat::detect_value(&loaded.value)
        })
        .ok_or_else(|| {
            unrecognized(
                "environment",
                EnvironmentFormat::ALL,
                EnvironmentFormat::label,
            )
        })?;
    match format {
        EnvironmentFormat::Postman => Ok(vec![postman_with_bom(input, parse_postman_environment)?]),
        EnvironmentFormat::Hoppscotch => run(
            loaded,
            FormatError::Hoppscotch,
            parse_hoppscotch_environments,
        ),
    }
}

/// Parse the one collection in `input`, in `format` or in the detected format when `None`.
/// Input holding several collections, or any environments, is `Multiple`.
pub fn parse_collection(
    format: Option<CollectionFormat>,
    input: &[u8],
) -> Result<ImportedCollection, FormatError> {
    single(parse_collections(
        format,
        input,
        &CollectionOptions::default(),
    )?)
}

/// Parse the one environment in `input`, in `format` or in the detected format when `None`.
/// Input holding several environments is `Multiple`.
pub fn parse_environment(
    format: Option<EnvironmentFormat>,
    input: &[u8],
) -> Result<ImportedEnvironment, FormatError> {
    single_environment(parse_environments(format, input)?)
}

/// The collection of the only bundle, provided it brought no environments.
fn single(mut bundles: Vec<ImportBundle>) -> Result<ImportedCollection, FormatError> {
    match bundles.as_slice() {
        [only] if only.environments.is_empty() => Ok(bundles.remove(0).collection),
        _ => Err(FormatError::Multiple {
            collections: bundles.len(),
            environments: bundles.iter().map(|b| b.environments.len()).sum(),
        }),
    }
}

/// The only environment.
fn single_environment(
    mut envs: Vec<ImportedEnvironment>,
) -> Result<ImportedEnvironment, FormatError> {
    if envs.len() == 1 {
        return Ok(envs.remove(0));
    }
    Err(FormatError::Multiple {
        collections: 0,
        environments: envs.len(),
    })
}

/// Parse a single request in `format`, or in the detected format when `None`.
pub fn parse_request(
    format: Option<RequestFormat>,
    input: &str,
) -> Result<ParsedRequest, FormatError> {
    let format = format
        .or_else(|| RequestFormat::detect(input))
        .ok_or_else(|| unrecognized("request", RequestFormat::ALL, RequestFormat::label))?;
    match format {
        RequestFormat::Curl => {
            let parsed = parse_curl(input)?;
            Ok(ParsedRequest {
                request: parsed.request,
                notes: parsed.notes,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{parse_postman, parse_postman_environment};

    fn bundle(name: &str, environments: usize) -> ImportBundle {
        let json = format!(
            r#"{{"info":{{"name":"{name}","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"}},"item":[]}}"#
        );
        let env = parse_postman_environment(br#"{"name":"e","values":[]}"#).expect("env");
        ImportBundle {
            collection: parse_postman(json.as_bytes()).expect("collection"),
            environments: vec![env; environments],
        }
    }

    fn counts(result: Result<impl std::fmt::Debug, FormatError>) -> (usize, usize) {
        match result {
            Err(FormatError::Multiple {
                collections,
                environments,
            }) => (collections, environments),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_singular_collection_call_takes_exactly_one_bare_collection() {
        assert_eq!(single(vec![bundle("a", 0)]).expect("one").name, "a");
        assert_eq!(counts(single(vec![bundle("a", 0), bundle("b", 1)])), (2, 1));
        assert_eq!(counts(single(vec![bundle("a", 2)])), (1, 2));
        assert_eq!(counts(single(Vec::new())), (0, 0));
    }

    #[test]
    fn the_singular_environment_call_takes_exactly_one() {
        let env = parse_postman_environment(br#"{"name":"e","values":[]}"#).expect("env");
        assert_eq!(
            single_environment(vec![env.clone()]).expect("one").name,
            "e"
        );
        assert_eq!(counts(single_environment(vec![env.clone(), env])), (0, 2));
    }
}
