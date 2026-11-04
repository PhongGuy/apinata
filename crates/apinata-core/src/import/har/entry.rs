//! One HAR entry to a request under its host's folder, or a note on why it was not written.

use super::body::body;
use super::budget::{Budget, Prefixed};
use super::credentials::{self, Credentials, Scrub};
use super::model::{Entry, Request, text};
use super::{headers, unknown};
use crate::edit::{AuthSpec, KeyValue, Param, ParamKind};
use crate::import::{DocumentError, ImportNote, ImportedRequest, ImportedRequestItem, NoteKind};
use crate::request::RequestKind;
use crate::url_params::{parse_query_params, split_on_first};
use serde_json::Value;
use url::Url;

const NOT_A_REQUEST: &str = "entry request is not a valid HAR request";
const NOT_HTTP: &str = "entry is not an http or https request";
const UNCUT: &str = "entry URL credentials could not be removed";

/// Entry and request members that are read elsewhere or carry nothing to import (Decision 14).
const ENTRY_EXEMPT: [&str; 8] = [
    "pageref",
    "startedDateTime",
    "time",
    "response",
    "cache",
    "timings",
    "serverIPAddress",
    "connection",
];
const REQUEST_EXEMPT: [&str; 5] = [
    "httpVersion",
    "headersSize",
    "bodySize",
    "queryString",
    "cookies",
];

/// Map `entries[i]` to its host's folder name and a request whose `seq` the caller sets, or
/// push a note to `unwritten` and give `None`. A serde error is never formatted into the note,
/// since its text can quote a captured value such as a token.
pub(super) fn map(
    i: usize,
    value: Value,
    unwritten: &mut Vec<ImportNote>,
    creds: &mut Credentials,
    budget: &mut Budget,
) -> Result<Option<(String, ImportedRequestItem)>, DocumentError> {
    let mut unwritten = |budget: &mut Budget, detail| {
        unwritten.push(budget.note("", &format!("entries[{i}]"), NoteKind::Dropped, detail)?);
        Ok(None)
    };
    let Ok(entry) = serde_json::from_value::<Entry>(value) else {
        return unwritten(budget, NOT_A_REQUEST);
    };
    let Some(raw @ Value::Object(_)) = entry.request else {
        return unwritten(budget, NOT_A_REQUEST);
    };
    let Ok(request) = serde_json::from_value::<Request>(raw) else {
        return unwritten(budget, NOT_A_REQUEST);
    };
    let url = text(request.url.as_ref());
    let parsed = match Url::parse(&url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        _ => return unwritten(budget, NOT_HTTP),
    };
    // `Url` lower-cases the host and drops a default port, so equal origins share a folder.
    let host = match parsed.port() {
        Some(port) => format!("{}:{port}", parsed.host_str().unwrap_or_default()),
        None => parsed.host_str().unwrap_or_default().to_owned(),
    };
    let method = match text(request.method.as_ref()) {
        m if m.is_empty() => "GET".to_owned(),
        m => m.to_uppercase(),
    };
    let name = [entry.comment.as_ref(), request.comment.as_ref()]
        .into_iter()
        .map(text)
        .find(|c| !c.trim().is_empty())
        .unwrap_or_else(|| format!("{method} {}", parsed.path()));
    // A form the textual cut misses (such as a tab inside the slashes) must never be written
    // with its credentials, so the entry is dropped before it is charged, noted or named.
    let (cut, userinfo) = credentials::cut_userinfo(&url);
    if Url::parse(&cut).is_ok_and(|u| !u.username().is_empty() || u.password().is_some()) {
        return unwritten(budget, UNCUT);
    }
    budget.entry()?;
    let mut notes = Vec::new();
    let at = format!("{host} / {name} / ");
    unknown(&entry.rest, &ENTRY_EXEMPT, &at, budget, &mut notes)?;
    unknown(
        &request.rest,
        &REQUEST_EXEMPT,
        &format!("{at}request."),
        budget,
        &mut notes,
    )?;
    let entry_path = format!("entries[{i}].");
    let mut scrub = Scrub {
        host: parsed.host_str().unwrap_or_default(),
        at: &at,
        entry: &entry_path,
        creds,
    };
    let (url, url_auth) = credentials::apply_url(&mut scrub, cut, userinfo, budget, &mut notes)?;
    let params = params_of(&url, budget)?;
    // Unknown header members are noted before filtering, so a dropped header still reports them.
    let mut raw_headers = Vec::new();
    for (k, h) in request.headers.unwrap_or_default().into_iter().enumerate() {
        if !h.rest.is_empty() {
            let at = format!("{at}request.headers[{k}].");
            unknown(&h.rest, &["comment"], &at, budget, &mut notes)?;
        }
        let pair = KeyValue::new(text(h.name.as_ref()), text(h.value.as_ref()));
        raw_headers.push((k, pair));
    }
    // Filtering runs before the credential pass, so a dropped header captures nothing.
    let mut headers = headers::filter(raw_headers, &at, budget, &mut notes)?;
    let cookies: Vec<(String, String)> = request
        .cookies
        .unwrap_or_default()
        .into_iter()
        .map(|c| (text(c.name.as_ref()), text(c.value.as_ref())))
        .collect();
    let auth = credentials::apply(&mut scrub, &mut headers, &cookies, budget, &mut notes)?;
    // A Basic or Bearer `Authorization` header wins over URL userinfo.
    let auth = match (auth, url_auth) {
        (AuthSpec::None, Some(from_url)) => from_url,
        (auth, _) => auth,
    };
    // Charged after the credential pass, so an `Authorization` header that became the auth is
    // not charged and a `Cookie` header built from `cookies[]` is charged once.
    let mut kept = Vec::with_capacity(headers.len());
    for (_, h) in headers {
        budget.entry()?;
        kept.push(h);
    }
    let body = body(request.post_data.as_ref(), &mut scrub, budget, &mut notes)?;
    let request = ImportedRequest {
        scripts: Default::default(),
        kind: RequestKind::Http,
        method,
        url,
        params,
        headers: kept,
        body,
        auth,
        docs: String::new(),
        settings: None,
        vars: Vec::new(),
        tags: Vec::new(),
        secrets: Vec::new(),
    };
    let item = ImportedRequestItem {
        name,
        seq: 0,
        request,
        notes,
    };
    Ok(Some((host, item)))
}

/// The query pairs of the written URL, undecoded and all `Query`, each charged one entry.
fn params_of(url: &str, budget: &mut Budget) -> Result<Vec<Param>, DocumentError> {
    let query = split_on_first(url, '?').1.map(parse_query_params);
    query
        .unwrap_or_default()
        .into_iter()
        .map(|(name, value)| {
            budget.entry()?;
            Ok(Param::new(
                name,
                value.unwrap_or_default(),
                ParamKind::Query,
            ))
        })
        .collect()
}
