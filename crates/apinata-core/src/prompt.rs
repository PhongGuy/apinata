//! Prompt variables: the `{{?label}}` grammar and the scan for the labels a request needs.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::LazyLock;

use regress::Regex;

use crate::context::{VarContext, json_strings, yaml_strings};
use crate::interpolate::interpolate_lenient;
use crate::network::{CertificateKind, domain_matches};
use crate::request::RequestFile;
use crate::resolve::{effective_auth, raw_headers, with_scheme};

/// The reference's `PROMPT_VARIABLE_TEMPLATE_PATTERN`; group 1 is the label.
const LABEL: &str = r"[^{}\s](?:[^{}]*[^{}\s])?";
static TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"\{{\{{\?({LABEL})\}}\}}")).expect("prompt template compiles")
});
static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^\?({LABEL})$")).expect("prompt name compiles"));

/// The label of the placeholder name `?label`, when `label` matches `[^{}\s](?:[^{}]*[^{}\s])?`.
pub fn prompt_label(name: &str) -> Option<&str> {
    let found = NAME_RE.find(name)?;
    found.group(1).map(|range| &name[range])
}

/// Every prompt label `req` needs under `ctx`, deduped, in first-seen order. Any answers
/// `ctx` holds are ignored, so they never change which labels are found.
pub fn prompt_labels(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let unanswered = ctx.with_prompt_values(std::iter::empty::<(String, String)>());
    let mut labels = Vec::new();
    for text in scan_texts(req, &unanswered) {
        for found in TEMPLATE_RE.find_iter(&text) {
            let Some(range) = found.group(1) else {
                continue;
            };
            let label = &text[range];
            if !labels.iter().any(|seen| seen == label) {
                labels.push(label.to_owned());
            }
        }
    }
    labels
}

/// The labels `req` needs that `ctx` holds no answer for, in scan order.
pub(crate) fn missing_labels(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let mut labels = prompt_labels(req, ctx);
    labels.retain(|label| !ctx.has_prompt_value(label));
    labels
}

/// The texts a prompt scan reads, in the reference's push order: the matching certificate,
/// the merged variables, the body, the merged headers, the params, the auth, then the URL.
/// Containers are walked by value only, never by key.
fn scan_texts(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let doc = req.document();
    let defaults = ctx.defaults_for(req);
    let vars = ctx.prompt_scan_vars(req);
    let headers = raw_headers(req, &defaults);
    let mut found = Vec::new();
    vars.values()
        .for_each(|value| json_strings(value, &mut found));
    if let Some(body) = doc.get(&[req.root_key(), "body"]) {
        yaml_strings(body, &mut found);
    }
    found.extend(headers.iter().map(|(_, value)| value.as_str()));
    if let Some(params) = doc.get(&[req.root_key(), "params"]) {
        yaml_strings(params, &mut found);
    }
    // An auth mode resolution rejects scans nothing; resolve reports it instead.
    if let Ok(Some(auth)) = effective_auth(doc.get(&[req.root_key(), "auth"]), &defaults) {
        auth.values()
            .for_each(|value| yaml_strings(value, &mut found));
    }
    found.push(req.url());
    let mut texts = certificate_texts(req, ctx);
    texts.extend(found.into_iter().map(str::to_owned));
    texts
}

/// The paths and passphrase of the first enabled certificate whose domain matches the URL,
/// both expanded leniently so an unanswered prompt stays as written.
fn certificate_texts(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let (vars, _) = ctx.lookup_for(req);
    let tracker = ctx.tracker(BTreeSet::new());
    let generator = RefCell::new(ctx.generator());
    let lenient = |text: &str| interpolate_lenient(text, &vars, &tracker, &generator);
    let url = with_scheme(lenient(req.url()));
    let Some(cert) = ctx
        .network()
        .certificates
        .iter()
        .filter(|cert| cert.enabled)
        .find(|cert| domain_matches(&lenient(&cert.domain), &url))
    else {
        return Vec::new();
    };
    let mut texts = match &cert.kind {
        CertificateKind::Pem {
            certificate,
            private_key,
        } => vec![certificate.clone(), private_key.clone()],
        CertificateKind::Pkcs12 { file } => vec![file.clone()],
    };
    texts.push(cert.passphrase.clone());
    texts
}
