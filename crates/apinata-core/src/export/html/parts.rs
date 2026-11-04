//! Small page parts shared by the header, folder sections and request articles: tables, auth
//! labels, method badges and the notes on credentials written into the page.

use super::text::escape;
use crate::edit::{AuthSpec, BodySpec, KeyValue, MultipartValue, Param, ParamKind};
use crate::export::{
    ExportNote, literal_credential, literal_credential_query, url_query_credentials,
};
use crate::js::js_trim;

/// The mode label: "No auth", "Basic", "Bearer token", "API key", "Digest", "OAuth 1.0",
/// "OAuth 2.0", "AWS Signature v4", "WSSE", "NTLM", "Akamai EdgeGrid", or an `Unsupported` mode as
/// written. Unescaped.
pub(super) fn auth_label(spec: &AuthSpec) -> String {
    match spec {
        AuthSpec::None => "No auth",
        AuthSpec::Inherit => "Inherit",
        AuthSpec::Basic { .. } => "Basic",
        AuthSpec::Bearer { .. } => "Bearer token",
        AuthSpec::ApiKey { .. } => "API key",
        AuthSpec::OAuth1(_) => "OAuth 1.0",
        AuthSpec::OAuth2(_) => "OAuth 2.0",
        AuthSpec::Digest { .. } => "Digest",
        AuthSpec::AwsV4 { .. } => "AWS Signature v4",
        AuthSpec::Wsse { .. } => "WSSE",
        AuthSpec::Ntlm { .. } => "NTLM",
        AuthSpec::EdgeGrid(_) => "Akamai EdgeGrid",
        AuthSpec::Unsupported { mode } => mode,
    }
    .to_owned()
}

/// The `Auth:` line for `spec`.
pub(super) fn auth_line(spec: &AuthSpec) -> String {
    format!(
        "<p class=\"auth\">Auth: {}</p>\n",
        escape(&auth_label(spec))
    )
}

/// A Name/Value/Description table with `caption`; empty when `rows` is empty.
pub(super) fn kv_table(caption: &str, rows: &[KeyValue]) -> String {
    let rows = rows
        .iter()
        .map(|r| row(&r.name, &r.value, r.description.as_deref(), r.enabled));
    table(caption, rows)
}

/// A params table of `kind` only, captioned `caption`; empty when no param has that kind.
pub(super) fn params_table(caption: &str, params: &[Param], kind: ParamKind) -> String {
    let rows = params
        .iter()
        .filter(|p| p.kind == kind)
        .map(|p| row(&p.name, &p.value, p.description.as_deref(), p.enabled));
    table(caption, rows)
}

/// The body as written, labelled by its YAML type, with the note detail for a kind the page
/// cannot show. A blank text body and an empty field or file list render as nothing.
pub(super) fn body(spec: &BodySpec) -> (String, Option<String>) {
    let part = |caption: &str, data: &str| match js_trim(data).is_empty() {
        true => String::new(),
        false => format!(
            "<p class=\"part\">{caption}</p>\n<pre>{}</pre>\n",
            escape(data)
        ),
    };
    let text = |label: &str, data: &str| part(&format!("Body ({label})"), data);
    let html = match spec {
        BodySpec::None => String::new(),
        BodySpec::Json(data) => text("json", data),
        BodySpec::Xml(data) => text("xml", data),
        BodySpec::Text(data) => text("text", data),
        BodySpec::Sparql(data) => text("sparql", data),
        BodySpec::FormUrlEncoded(fields) => kv_table("Body (form-urlencoded)", fields),
        BodySpec::Multipart(fields) => {
            let rows = fields.iter().map(|f| {
                let value = match &f.value {
                    MultipartValue::Text(t) => t.clone(),
                    MultipartValue::Files(paths) => format!("file: {}", paths.join(", ")),
                };
                row(&f.name, &value, f.description.as_deref(), f.enabled)
            });
            table("Body (multipart-form)", rows)
        }
        BodySpec::File(files) if files.is_empty() => String::new(),
        BodySpec::File(files) => {
            let mut out = "<p class=\"part\">Body (file)</p>\n<ul>\n".to_owned();
            for file in files {
                let tag = if file.selected {
                    " <span class=\"tag\">selected</span>"
                } else {
                    ""
                };
                out.push_str(&format!("<li>{}{tag}</li>\n", escape(&file.path)));
            }
            out.push_str("</ul>\n");
            out
        }
        BodySpec::Graphql { query, variables } => {
            text("graphql", query) + &part("Variables", variables)
        }
        BodySpec::Unsupported { kind } => return unsupported(kind),
    };
    (html, None)
}

/// The label and note for a body kind the page cannot show.
fn unsupported(kind: &str) -> (String, Option<String>) {
    let html = format!("<p class=\"part\">Body ({})</p>\n", escape(kind));
    (html, Some(format!("the {kind} body cannot be shown")))
}

/// One table row; a disabled row is classed and tagged after its name.
fn row(name: &str, value: &str, description: Option<&str>, enabled: bool) -> String {
    let (open, tag) = if enabled {
        ("<tr>", "")
    } else {
        (
            "<tr class=\"disabled\">",
            " <span class=\"tag\">disabled</span>",
        )
    };
    let description = description.map(escape).unwrap_or_default();
    format!(
        "{open}<td>{}{tag}</td><td>{}</td><td>{description}</td></tr>\n",
        escape(name),
        escape(value),
    )
}

/// A Name/Value/Description table of ready rows; empty when there are none.
fn table(caption: &str, rows: impl Iterator<Item = String>) -> String {
    let body: String = rows.collect();
    if body.is_empty() {
        return String::new();
    }
    let mut out = format!("<table>\n<caption>{}</caption>\n", escape(caption));
    out.push_str("<thead><tr><th>Name</th><th>Value</th><th>Description</th></tr></thead>\n");
    out.push_str("<tbody>\n");
    out.push_str(&body);
    out.push_str("</tbody>\n</table>\n");
    out
}

/// The method badge: class `m-<method>` for the standard methods, else `m-other`.
pub(super) fn method_badge(method: &str) -> String {
    let lower = method.to_ascii_lowercase();
    let known = [
        "get", "post", "put", "patch", "delete", "head", "options", "trace",
    ];
    let class = if known.contains(&lower.as_str()) {
        lower.as_str()
    } else {
        "other"
    };
    format!("<span class=\"method m-{class}\">{}</span>", escape(method))
}

/// One note per header that `literal_credential` flags, at `<source> / headers`.
pub(super) fn credential_notes(headers: &[KeyValue], source: &str, notes: &mut Vec<ExportNote>) {
    for header in headers.iter().filter(|h| literal_credential(h)) {
        notes.push(ExportNote::new(
            format!("{source} / headers"),
            format!(
                "header {} holds a literal credential, written into the page",
                header.name
            ),
        ));
    }
}

/// One note per query param that `literal_credential_query` flags, at `<source> / params`.
pub(super) fn query_credential_notes(params: &[Param], source: &str, notes: &mut Vec<ExportNote>) {
    for p in params
        .iter()
        .filter(|p| p.kind == ParamKind::Query && literal_credential_query(&p.name, &p.value))
    {
        notes.push(ExportNote::new(
            format!("{source} / params"),
            format!(
                "query param {} holds a literal credential, written into the page",
                p.name
            ),
        ));
    }
}

/// One note per literal credential in `url`'s query string that `params` does not list,
/// at `<source> / url`.
pub(super) fn url_credential_notes(
    url: &str,
    params: &[Param],
    source: &str,
    notes: &mut Vec<ExportNote>,
) {
    for name in url_query_credentials(url, params) {
        notes.push(ExportNote::new(
            format!("{source} / url"),
            format!("query param {name} holds a literal credential, written into the page"),
        ));
    }
}
