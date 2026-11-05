//! Saved examples: named request and response pairs kept in a request file's `examples` list,
//! written as Bruno writes them. Each edit writes only `examples` through to disk, so other
//! unsaved edits to the request stay unsaved.

use crate::collection::Collection;
use crate::document::{Document, set_value};
use crate::edit::{BodySpec, KeyValue, Param, description, text};
use crate::error::TreeError;
use crate::item_path::ItemPath;
use crate::js::{js_trim, yaml_js_string};
use crate::json_text::reindent_json;
use crate::order;
use crate::request::{RequestFile, RequestKind};
use crate::response::Response;
use crate::secret_names::{NamedSecret, bytes_hold_secret, mask_response_header, replace_secrets};
use crate::url_password::userinfo_span;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use yaml_serde::{Mapping, Value};

/// The name Bruno gives an example whose `name` is missing or null.
const UNTITLED: &str = "Untitled Example";

/// A saved example: a named request and the response it got.
#[derive(Clone)]
#[non_exhaustive]
pub struct Example {
    /// The example's name.
    pub name: String,
    /// The description, when not blank.
    pub description: Option<String>,
    /// The request half, when the entry has one.
    pub request: Option<ExampleRequest>,
    /// The response half, when the entry has one.
    pub response: Option<ExampleResponse>,
}

/// Shows the name only, so saved headers and bodies never reach a log.
impl std::fmt::Debug for Example {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Example")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The request half of a saved example, as written: placeholders are not resolved.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExampleRequest {
    /// The URL.
    pub url: String,
    /// The method, `GET` when absent.
    pub method: String,
    /// The headers, disabled ones included.
    pub headers: Vec<KeyValue>,
    /// The query and path params, disabled ones included.
    pub params: Vec<Param>,
    /// The body.
    pub body: BodySpec,
}

impl RequestFile {
    /// One example per `examples` entry, in file order, a non-mapping entry included.
    pub fn examples(&self) -> Vec<Example> {
        match self.document().get(&["examples"]) {
            Some(Value::Sequence(items)) => items.iter().map(read_example).collect(),
            _ => Vec::new(),
        }
    }

    /// Replace `examples` in memory with one entry per example, each written as
    /// [`Collection::add_example`] writes it with this file's request half; none removes the key.
    pub(crate) fn set_examples(&mut self, examples: &[Example]) {
        let entries = examples
            .iter()
            .map(|e| {
                let name = Some(js_trim(&e.name))
                    .filter(|n| !n.is_empty())
                    .unwrap_or(UNTITLED);
                let description = e.description.as_deref().unwrap_or_default();
                let response = e.response.clone().unwrap_or_else(ExampleResponse::empty);
                new_entry(self, name, description, &response)
            })
            .collect();
        self.document_mut()
            .set_list(&["examples"], entries, &[order::TOP]);
    }

    /// Whether an example is named `name`, trimmed as [`Collection::add_example`] trims it.
    pub fn has_example(&self, name: &str) -> bool {
        let name = js_trim(name);
        self.examples().iter().any(|e| e.name == name)
    }
}

/// One entry as Bruno's parser reads it; a part that is not a mapping is absent.
fn read_example(entry: &Value) -> Example {
    Example {
        name: entry
            .get("name")
            .and_then(yaml_js_string)
            .unwrap_or_else(|| UNTITLED.to_owned()),
        description: description(entry),
        request: entry
            .get("request")
            .filter(|r| r.is_mapping())
            .map(read_request),
        response: entry
            .get("response")
            .filter(|r| r.is_mapping())
            .map(read_response),
    }
}

fn read_request(request: &Value) -> ExampleRequest {
    let list = |key: &str| -> Vec<Value> {
        request
            .get(key)
            .and_then(Value::as_sequence)
            .cloned()
            .unwrap_or_default()
    };
    ExampleRequest {
        url: request
            .get("url")
            .and_then(yaml_js_string)
            .unwrap_or_default(),
        method: request
            .get("method")
            .and_then(yaml_js_string)
            .unwrap_or_else(|| "GET".to_owned()),
        headers: list("headers").iter().map(KeyValue::from_yaml).collect(),
        params: list("params").iter().map(Param::from_yaml).collect(),
        body: BodySpec::from_yaml(request.get("body")),
    }
}

fn read_response(response: &Value) -> ExampleResponse {
    let headers = response
        .get("headers")
        .and_then(Value::as_sequence)
        .map(|list| {
            list.iter()
                .map(|h| (text(h, "name"), text(h, "value")))
                .collect()
        })
        .unwrap_or_default();
    let body = response.get("body").filter(|b| b.is_mapping()).map(|b| {
        ExampleBody::new(
            ExampleBodyKind::from_type(&text(b, "type")),
            text(b, "data"),
        )
    });
    ExampleResponse {
        status: response.get("status").and_then(read_status),
        status_text: response
            .get("statusText")
            .and_then(yaml_js_string)
            .filter(|t| !t.is_empty()),
        headers,
        body,
    }
}

/// A status as Bruno's `Number(status)` reads it, kept only when it is a valid `u16`.
fn read_status(status: &Value) -> Option<u16> {
    match status {
        Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
        Value::String(s) => js_trim(s).parse().ok(),
        _ => None,
    }
}

/// The kind of a saved response body, as its YAML `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExampleBodyKind {
    /// JSON text.
    Json,
    /// XML text.
    Xml,
    /// HTML text.
    Html,
    /// Any other text.
    Text,
    /// Bytes, stored as base64.
    Binary,
}

impl ExampleBodyKind {
    /// The YAML `type`: "json", "xml", "html", "text", "binary".
    pub fn as_str(self) -> &'static str {
        match self {
            ExampleBodyKind::Json => "json",
            ExampleBodyKind::Xml => "xml",
            ExampleBodyKind::Html => "html",
            ExampleBodyKind::Text => "text",
            ExampleBodyKind::Binary => "binary",
        }
    }

    /// The kind stored as YAML `type`; an unknown or missing one is text.
    fn from_type(kind: &str) -> ExampleBodyKind {
        match kind {
            "json" => ExampleBodyKind::Json,
            "xml" => ExampleBodyKind::Xml,
            "html" => ExampleBodyKind::Html,
            "binary" => ExampleBodyKind::Binary,
            _ => ExampleBodyKind::Text,
        }
    }

    /// Bruno's `getBodyType` for a `Content-Type` value.
    pub fn from_content_type(content_type: &str) -> ExampleBodyKind {
        let lower = content_type.to_lowercase();
        if lower.contains("application/json") {
            return ExampleBodyKind::Json;
        }
        if lower.contains("text/xml") || lower.contains("application/xml") {
            return ExampleBodyKind::Xml;
        }
        if lower.contains("text/html") {
            return ExampleBodyKind::Html;
        }
        // Media is binary unless it is an XML dialect such as SVG, which is text.
        let mime = lower.split(';').next().unwrap_or_default().trim();
        let media = ["image/", "audio/", "video/"]
            .iter()
            .any(|prefix| mime.starts_with(prefix));
        let binary = !mime.ends_with("+xml")
            && (media || mime == "application/pdf" || mime == "application/octet-stream");
        if binary {
            ExampleBodyKind::Binary
        } else {
            ExampleBodyKind::Text
        }
    }
}

/// A saved response body: its kind and its stored text (base64 for binary).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExampleBody {
    /// The body's kind.
    pub kind: ExampleBodyKind,
    /// The body as stored.
    pub data: String,
}

impl ExampleBody {
    /// A body of `kind` holding `data`.
    pub fn new(kind: ExampleBodyKind, data: impl Into<String>) -> ExampleBody {
        ExampleBody {
            kind,
            data: data.into(),
        }
    }
}

/// Shows the kind and the data's length only, so a saved body never reaches a log.
impl std::fmt::Debug for ExampleBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExampleBody")
            .field("kind", &self.kind)
            .field("len", &self.data.len())
            .finish()
    }
}

/// The response half of a saved example.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct ExampleResponse {
    /// The status code; `None` or 0 is left out.
    pub status: Option<u16>,
    /// The status text; blank is left out.
    pub status_text: Option<String>,
    /// The response headers as (name, value), in order.
    pub headers: Vec<(String, String)>,
    /// The body, when there is one.
    pub body: Option<ExampleBody>,
}

impl ExampleResponse {
    /// `status` with no status text, headers or body.
    pub fn new(status: u16) -> ExampleResponse {
        ExampleResponse {
            status: Some(status),
            status_text: None,
            headers: Vec::new(),
            body: None,
        }
    }

    /// No status, status text, headers or body.
    fn empty() -> ExampleResponse {
        ExampleResponse {
            status: None,
            status_text: None,
            headers: Vec::new(),
            body: None,
        }
    }

    /// Status, status text (none when empty), headers as received, and the body: binary as
    /// base64 when the type is binary or the bytes are not UTF-8, else the decoded text, with
    /// JSON that parses re-indented two spaces deep.
    pub fn from_response(response: &Response) -> ExampleResponse {
        let content_type = response
            .headers()
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map_or("", |(_, value)| value.as_str());
        let bytes = response.body();
        let body = match (
            ExampleBodyKind::from_content_type(content_type),
            std::str::from_utf8(bytes),
        ) {
            (ExampleBodyKind::Binary, _) | (_, Err(_)) => {
                ExampleBody::new(ExampleBodyKind::Binary, STANDARD.encode(bytes))
            }
            // JSON is laid out as the editor shows it; text that does not parse stays as received.
            (ExampleBodyKind::Json, Ok(text)) => ExampleBody::new(
                ExampleBodyKind::Json,
                reindent_json(text).unwrap_or_else(|| text.to_owned()),
            ),
            (kind, Ok(text)) => ExampleBody::new(kind, text),
        };
        let status_text = response.status_text();
        ExampleResponse {
            status: Some(response.status()),
            status_text: (!status_text.is_empty()).then(|| status_text.to_owned()),
            headers: response.headers().to_vec(),
            body: Some(body),
        }
    }

    /// The line added to an example's description when its body was withheld.
    pub const WITHHELD_NOTE: &str = "Response body withheld: it held a secret value.";

    /// Bodies at or above this many bytes cannot be saved as an example (5 MiB).
    pub const BODY_LIMIT: u64 = 5 * 1024 * 1024;

    /// Whether `response` can become an example: not truncated and under [`Self::BODY_LIMIT`].
    pub fn savable(response: &Response) -> bool {
        !response.truncated() && (response.body().len() as u64) < Self::BODY_LIMIT
    }

    /// Make this example safe to commit. Set-Cookie and credential headers are always masked;
    /// with `secrets`, each value in a header or text body becomes its placeholder, and a binary
    /// body holding one is removed, since its bytes cannot be rewritten.
    pub fn scrub(&mut self, secrets: Option<&[NamedSecret]>) -> Scrubbed {
        for (name, value) in &mut self.headers {
            *value = mask_response_header(name, value);
        }
        let Some(secrets) = secrets else {
            return Scrubbed { withheld: false };
        };
        for (_, value) in &mut self.headers {
            *value = replace_secrets(value, secrets);
        }
        let withheld = match self.body.as_mut() {
            Some(body) if body.kind == ExampleBodyKind::Binary => {
                // Text that is not base64 is checked as it stands.
                let bytes = STANDARD
                    .decode(&body.data)
                    .unwrap_or_else(|_| body.data.clone().into_bytes());
                bytes_hold_secret(&bytes, secrets)
            }
            Some(body) => {
                body.data = replace_secrets(&body.data, secrets);
                false
            }
            None => false,
        };
        if withheld {
            self.body = None;
        }
        Scrubbed { withheld }
    }

    /// A `Response` showing this example: base64 decoded for a binary body (invalid base64
    /// shows the text as is), no body when `body` is `None`.
    pub fn to_response(&self) -> Response {
        let mut response = Response::new(self.status.unwrap_or(0));
        if let Some(text) = &self.status_text {
            response = response.with_status_text(text.as_str());
        }
        for (name, value) in &self.headers {
            response = response.with_header(name, value.as_str());
        }
        match &self.body {
            None => response,
            Some(body) if body.kind == ExampleBodyKind::Binary => match STANDARD.decode(&body.data)
            {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    response.with_body(bytes, text)
                }
                Err(_) => response.with_text_body(body.data.as_str()),
            },
            Some(body) => response.with_text_body(body.data.as_str()),
        }
    }
}

/// What [`ExampleResponse::scrub`] did beyond masking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Scrubbed {
    /// True when a binary body held a secret and was removed.
    pub withheld: bool,
}

impl Scrubbed {
    /// `description` with [`ExampleResponse::WITHHELD_NOTE`] on a line of its own when the
    /// body was withheld, else `description` as given.
    pub fn description(&self, description: &str) -> String {
        match (self.withheld, description.is_empty()) {
            (false, _) => description.to_owned(),
            (true, true) => ExampleResponse::WITHHELD_NOTE.to_owned(),
            (true, false) => format!("{description}\n{}", ExampleResponse::WITHHELD_NOTE),
        }
    }
}

impl Collection {
    /// The name a new example of the request at `item` would get: `name` trimmed as
    /// [`Self::add_example`] trims it. `NotFound` without an HTTP request there, `InvalidName`
    /// for a blank name, `ExampleNameTaken` when another example already has it.
    pub fn new_example_name(&self, item: &ItemPath, name: &str) -> Result<String, TreeError> {
        let request = self
            .request(item)
            .filter(|r| r.kind() == RequestKind::Http)
            .ok_or_else(|| TreeError::NotFound { item: item.clone() })?;
        let name = checked_name(name)?;
        if request.has_example(name) {
            return Err(TreeError::ExampleNameTaken {
                item: item.clone(),
                name: name.to_owned(),
            });
        }
        Ok(name.to_owned())
    }

    /// Append an example named `name` (trimmed) with `description` (trimmed; blank is left
    /// out) to the request at `item`. Its request half is the request's current URL, method,
    /// headers, params and body as written, unsaved edits included and nothing resolved, except
    /// that the url's userinfo (username and password) is dropped.
    /// Only `examples` is written to disk; other unsaved edits stay unsaved. Returns the new
    /// example's index. `NotFound` for a missing or GraphQL request, `InvalidName` for a blank name,
    /// `Conflict`/`Io` when the write fails.
    pub fn add_example(
        &mut self,
        item: &ItemPath,
        name: &str,
        description: &str,
        response: &ExampleResponse,
    ) -> Result<usize, TreeError> {
        let name = checked_name(name)?;
        let not_found = || TreeError::NotFound { item: item.clone() };
        let request = self
            .request_mut(item)
            .filter(|r| r.kind() == RequestKind::Http)
            .ok_or_else(not_found)?;
        let entry = new_entry(request, name, description, response);
        let doc = request.document_mut();
        let mut items = saved_examples(doc);
        items.push(entry);
        let index = items.len() - 1;
        doc.write_through_list(&["examples"], items, &[order::TOP])?;
        Ok(index)
    }

    /// Set example `index`'s name (trimmed), written through. `NotFound` for a missing
    /// request or index, `InvalidName` for a blank name.
    pub fn rename_example(
        &mut self,
        item: &ItemPath,
        index: usize,
        name: &str,
    ) -> Result<(), TreeError> {
        let name = checked_name(name)?;
        self.edit_examples(item, index, |items| {
            let entry = &mut items[index];
            // An entry that is not a mapping cannot hold a name, so it becomes one that does.
            if !entry.is_mapping() {
                *entry = Value::Mapping(Mapping::new());
            }
            if let Value::Mapping(map) = entry {
                set_value(map, &["name"], Value::from(name), &[order::EXAMPLE]);
            }
        })
    }

    /// Remove example `index`, written through; the last one removes `examples`.
    /// `NotFound` for a missing request or index.
    pub fn delete_example(&mut self, item: &ItemPath, index: usize) -> Result<(), TreeError> {
        self.edit_examples(item, index, |items| {
            items.remove(index);
        })
    }

    /// Apply `edit` to the saved `examples` of the request at `item`, whose entry `index`
    /// must exist, and write the result through.
    fn edit_examples(
        &mut self,
        item: &ItemPath,
        index: usize,
        edit: impl FnOnce(&mut Vec<Value>),
    ) -> Result<(), TreeError> {
        let not_found = || TreeError::NotFound { item: item.clone() };
        let doc = self
            .request_mut(item)
            .filter(|r| r.kind() == RequestKind::Http)
            .ok_or_else(not_found)?
            .document_mut();
        let mut items = saved_examples(doc);
        if index >= items.len() {
            return Err(not_found());
        }
        edit(&mut items);
        doc.write_through_list(&["examples"], items, &[order::TOP])?;
        Ok(())
    }
}

/// What [`scrub_example_userinfo`] changed, or would change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScrubReport {
    changes: Vec<ScrubbedExample>,
    written: bool,
}

impl ScrubReport {
    /// Each example whose URL held userinfo, in tree order.
    pub fn changes(&self) -> &[ScrubbedExample] {
        &self.changes
    }

    /// Whether any file was saved.
    pub fn written(&self) -> bool {
        self.written
    }
}

/// One saved example whose recorded URL held userinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScrubbedExample {
    item: ItemPath,
    example: String,
    url: String,
}

impl ScrubbedExample {
    /// The request file holding the example.
    pub fn item(&self) -> &ItemPath {
        &self.item
    }

    /// The example's name.
    pub fn example(&self) -> &str {
        &self.example
    }

    /// The URL without its userinfo.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Find every saved example whose recorded request URL holds a username or password, and with
/// `write` remove it, saving only the files that held one (just their `examples`, as the other
/// example edits do). The request's own URL and every other file are never changed. The report
/// carries the scrubbed URLs only. A failed save stops the walk; files saved before it stay.
pub fn scrub_example_userinfo(c: &mut Collection, write: bool) -> Result<ScrubReport, TreeError> {
    let mut items = Vec::new();
    if let Some(root) = c.folder(&ItemPath::root()) {
        request_files(c, root, &mut items);
    }
    let mut report = ScrubReport {
        changes: Vec::new(),
        written: false,
    };
    for item in items {
        let Some(request) = c
            .request_mut(&item)
            .filter(|r| r.kind() == RequestKind::Http)
        else {
            continue;
        };
        let doc = request.document_mut();
        let mut entries = saved_examples(doc);
        let mut changed = false;
        for entry in &mut entries {
            let Some(Value::String(url)) = entry.get_mut("request").and_then(|r| r.get_mut("url"))
            else {
                continue;
            };
            let scrubbed = without_userinfo(url);
            if scrubbed == *url {
                continue;
            }
            *url = scrubbed.clone();
            changed = true;
            report.changes.push(ScrubbedExample {
                item: item.clone(),
                example: read_example(entry).name,
                url: scrubbed,
            });
        }
        if changed && write {
            doc.write_through_list(&["examples"], entries, &[order::TOP])?;
            report.written = true;
        }
    }
    Ok(report)
}

/// The paths of every request file under `folder`, in tree order.
fn request_files(c: &Collection, folder: &crate::collection::Folder, out: &mut Vec<ItemPath>) {
    out.extend(
        folder
            .requests()
            .iter()
            .filter_map(|r| c.item_path(r.path())),
    );
    for sub in folder.folders() {
        request_files(c, sub, out);
    }
}

/// `name` trimmed, or `InvalidName` when nothing is left.
fn checked_name(name: &str) -> Result<&str, TreeError> {
    let trimmed = js_trim(name);
    if trimmed.is_empty() {
        return Err(TreeError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(trimmed)
}

/// The `examples` list as last saved; anything but a sequence counts as empty.
fn saved_examples(doc: &Document) -> Vec<Value> {
    match doc.get_saved(&["examples"]) {
        Some(Value::Sequence(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// One `examples` entry, its keys inserted in Bruno's writer order. The request half copies
/// the draft's `http` values verbatim, so placeholders stay unresolved, except that the url
/// loses its userinfo.
fn new_entry(
    source: &RequestFile,
    name: &str,
    description: &str,
    response: &ExampleResponse,
) -> Value {
    let mut entry = Mapping::new();
    entry.insert("name".into(), name.into());
    let description = js_trim(description);
    if !description.is_empty() {
        entry.insert("description".into(), description.into());
    }

    let doc = source.document();
    let mut request = Mapping::new();
    request.insert("url".into(), without_userinfo(source.url()).into());
    request.insert("method".into(), source.method().into());
    for key in ["headers", "params"] {
        if let Some(Value::Sequence(list)) = doc.get(&["http", key])
            && !list.is_empty()
        {
            request.insert(key.into(), Value::Sequence(list.clone()));
        }
    }
    if let Some(body) = doc.get(&["http", "body"]).filter(|b| !b.is_null()) {
        request.insert("body".into(), body.clone());
    }
    entry.insert("request".into(), Value::Mapping(request));

    entry.insert("response".into(), Value::Mapping(response_map(response)));
    Value::Mapping(entry)
}

/// `url` without the userinfo of the authority the executor sends to: the one after `scheme://`,
/// or the leading one of a schemeless url, which is sent as `http://`. All other text is kept.
/// A scheme may be a placeholder (`{{proto}}://`), and the userinfo is everything before the
/// authority's last `@`, as the URL standard reads it. Examples already on disk are rewritten only
/// by [`scrub_example_userinfo`] (`apinata item scrub-userinfo`).
fn without_userinfo(url: &str) -> String {
    match userinfo_span(url) {
        Some((start, at)) => format!("{}{}", &url[..start], &url[at + 1..]),
        None => url.to_owned(),
    }
}

/// The `response` mapping, leaving out what Bruno leaves out: a zero status, a blank status
/// text and an empty header list.
fn response_map(response: &ExampleResponse) -> Mapping {
    let mut map = Mapping::new();
    if let Some(status) = response.status.filter(|s| *s > 0) {
        map.insert("status".into(), Value::Number(status.into()));
    }
    if let Some(text) = response
        .status_text
        .as_deref()
        .filter(|t| !js_trim(t).is_empty())
    {
        map.insert("statusText".into(), text.into());
    }
    if !response.headers.is_empty() {
        let headers = response
            .headers
            .iter()
            .map(|(name, value)| {
                let mut header = Mapping::new();
                header.insert("name".into(), name.as_str().into());
                header.insert("value".into(), value.as_str().into());
                Value::Mapping(header)
            })
            .collect();
        map.insert("headers".into(), Value::Sequence(headers));
    }
    if let Some(body) = &response.body {
        let mut data = Mapping::new();
        data.insert("type".into(), body.kind.as_str().into());
        data.insert("data".into(), body.data.as_str().into());
        map.insert("body".into(), Value::Mapping(data));
    }
    map
}
