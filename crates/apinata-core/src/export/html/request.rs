//! The request article: heading, URL, docs, params, headers, body, the auth mode and examples.

use super::examples::examples;
use super::parts::{
    auth_label, body, credential_notes, kv_table, method_badge, params_table,
    query_credential_notes, url_credential_notes,
};
use super::text::{docs, escape};
use crate::collection::Collection;
use crate::edit::{AuthSpec, ParamKind};
use crate::export::ExportNote;
use crate::js::js_trim;
use crate::request::RequestFile;

/// The article for `req` at anchor `id`, each part omitted when empty. Notes on credential
/// headers, an unshowable body and binary example bodies go to `notes` under `source`, the
/// request's name trail.
pub(super) fn article(
    collection: &Collection,
    req: &RequestFile,
    id: &str,
    source: &str,
    notes: &mut Vec<ExportNote>,
) -> String {
    let mut out = format!(
        "<article id=\"{id}\">\n<h3>{} {}</h3>\n",
        method_badge(req.method()),
        escape(req.name())
    );
    if !js_trim(req.url()).is_empty() {
        out.push_str(&format!(
            "<p class=\"url\"><code>{}</code></p>\n",
            escape(req.url())
        ));
    }
    out.push_str(&docs(&req.docs(), id, 4));
    let params = req.shown_params();
    out.push_str(&params_table("Path params", &params, ParamKind::Path));
    out.push_str(&params_table("Query params", &params, ParamKind::Query));
    let headers = req.headers();
    out.push_str(&kv_table("Headers", &headers));
    credential_notes(&headers, source, notes);
    query_credential_notes(&params, source, notes);
    url_credential_notes(req.url(), &params, source, notes);
    let (body, body_note) = body(&req.body());
    out.push_str(&body);
    if let Some(detail) = body_note {
        notes.push(ExportNote::new(format!("{source} / body"), detail));
    }
    // An inheriting request names the mode the levels above hand it, never "Inherit".
    let label = match req.auth() {
        AuthSpec::Inherit => {
            let inherited = collection
                .item_path(req.path())
                .map_or(AuthSpec::None, |item| {
                    collection.inherited_auth(&item).auth().clone()
                });
            format!("Inherited ({})", auth_label(&inherited))
        }
        own => auth_label(&own),
    };
    out.push_str(&format!("<p class=\"auth\">Auth: {}</p>\n", escape(&label)));
    out.push_str(&examples(&req.examples(), source, notes));
    out.push_str("</article>\n");
    out
}
