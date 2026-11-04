//! Saved examples: each one's request and response halves, as written.

use super::parts::{body, kv_table, method_badge, params_table};
use super::text::escape;
use crate::edit::ParamKind;
use crate::example::{Example, ExampleBodyKind, ExampleResponse};
use crate::export::{ExportNote, literal_credential, literal_credential_query};
use crate::js::js_trim;

/// One block per example of the request whose name trail is `source`. A binary response body
/// is left out with a note at `<source> / examples`, as is a literal credential header; an unshowable request body has no note.
pub(super) fn examples(examples: &[Example], source: &str, notes: &mut Vec<ExportNote>) -> String {
    let mut out = String::new();
    for example in examples {
        out.push_str(&format!(
            "<div class=\"example\">\n<h4>Example: {}</h4>\n",
            escape(&example.name)
        ));
        if let Some(description) = &example.description {
            out.push_str(&format!("<p>{}</p>\n", escape(description)));
        }
        if let Some(req) = &example.request {
            out.push_str("<p class=\"part\">Request</p>\n");
            out.push_str(&format!(
                "<p class=\"url\">{} <code>{}</code></p>\n",
                method_badge(&req.method),
                escape(&req.url)
            ));
            out.push_str(&params_table("Path params", &req.params, ParamKind::Path));
            out.push_str(&params_table("Query params", &req.params, ParamKind::Query));
            out.push_str(&kv_table("Headers", &req.headers));
            for header in req.headers.iter().filter(|h| literal_credential(h)) {
                notes.push(ExportNote::new(
                    format!("{source} / examples"),
                    format!(
                        "example {}: header {} holds a literal credential, written into the page",
                        example.name, header.name
                    ),
                ));
            }
            for p in req.params.iter().filter(|p| {
                p.kind == ParamKind::Query && literal_credential_query(&p.name, &p.value)
            }) {
                notes.push(ExportNote::new(
                    format!("{source} / examples"),
                    format!(
                        "example {}: query param {} holds a literal credential, written into the page",
                        example.name, p.name
                    ),
                ));
            }
            out.push_str(&body(&req.body).0);
        }
        if let Some(res) = &example.response {
            out.push_str("<p class=\"part\">Response</p>\n");
            out.push_str(&response(res));
            if res.body.as_ref().is_some_and(|b| is_binary(b.kind)) {
                notes.push(ExportNote::new(
                    format!("{source} / examples"),
                    format!("the binary body of example {} was left out", example.name),
                ));
            }
        }
        out.push_str("</div>\n");
    }
    out
}

/// The status line, the response headers and a text body; a binary or blank body renders as
/// nothing.
fn response(res: &ExampleResponse) -> String {
    let mut out = String::new();
    let status = res.status.filter(|s| *s != 0).map(|s| s.to_string());
    let line: Vec<&str> = status
        .iter()
        .chain(&res.status_text)
        .map(String::as_str)
        .collect();
    if !line.is_empty() {
        out.push_str(&format!(
            "<p class=\"status\">{}</p>\n",
            escape(&line.join(" "))
        ));
    }
    if !res.headers.is_empty() {
        out.push_str("<table>\n<caption>Response headers</caption>\n");
        out.push_str("<thead><tr><th>Name</th><th>Value</th></tr></thead>\n<tbody>\n");
        for (name, value) in &res.headers {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>\n",
                escape(name),
                escape(value)
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    }
    if let Some(b) = res
        .body
        .as_ref()
        .filter(|b| !is_binary(b.kind) && !js_trim(&b.data).is_empty())
    {
        out.push_str(&format!(
            "<p class=\"part\">Body ({})</p>\n<pre>{}</pre>\n",
            b.kind.as_str(),
            escape(&b.data)
        ));
    }
    out
}

/// Whether a body of this kind is bytes rather than text; a new kind must choose here.
fn is_binary(kind: ExampleBodyKind) -> bool {
    match kind {
        ExampleBodyKind::Binary => true,
        ExampleBodyKind::Json
        | ExampleBodyKind::Xml
        | ExampleBodyKind::Html
        | ExampleBodyKind::Text => false,
    }
}
