//! `export_openapi` and `export_postman`: the core exports the CLI writes, returned as text
//! scrubbed of session secrets and cut to a byte budget. Nothing is sent or written.

use apinata_core::Collection;
use apinata_core::export::{self, ExportNote};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::redact::Redactor;
use crate::send::{DEFAULT_BODY_BYTES, MAX_BODY_BYTES};

/// `export_openapi`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportOpenapiArgs {
    /// `json` (the default) or `yaml`.
    pub format: Option<String>,
    /// The most text to return, in bytes; 32 KiB when absent, 1 MiB at most.
    pub max_bytes: Option<u64>,
}

/// `export_postman`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportPostmanArgs {
    /// The most text to return, in bytes; 32 KiB when absent, 1 MiB at most.
    pub max_bytes: Option<u64>,
    /// Export scripts exactly as written instead of translating their API calls for Postman; false when absent.
    pub keep_scripts: Option<bool>,
}

/// What an export tool returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ExportDoc {
    /// `json` or `yaml`.
    pub format: String,
    /// The export, cut to `max_bytes`.
    pub text: String,
    /// The whole export's size in bytes.
    pub total_bytes: u64,
    /// True when `text` was cut to the byte budget.
    pub truncated: bool,
    /// What the target format cannot carry, one `source: detail` line each, as `apinata export`
    /// prints them.
    pub notes: Vec<String>,
}

/// The OpenAPI format, refusing anything but `json` and `yaml`.
pub(crate) fn format(asked: Option<&str>) -> Result<&'static str, String> {
    match asked.unwrap_or("json") {
        "json" => Ok("json"),
        "yaml" => Ok("yaml"),
        other => Err(format!(
            "`format` is `{other}`; it must be `json` or `yaml`"
        )),
    }
}

/// The collection as OpenAPI 3.1 in `format`.
pub(crate) fn openapi(
    c: &Collection,
    format: &str,
    max_bytes: Option<u64>,
    seen: &Redactor,
) -> Result<ExportDoc, String> {
    let exported = export::openapi(c);
    let text = if format == "yaml" {
        exported.yaml()?
    } else {
        exported.json
    };
    Ok(doc(format, &text, &exported.notes, max_bytes, seen))
}

/// The collection as a Postman v2.1 collection, its scripts translated unless `keep_scripts`.
pub(crate) fn postman(
    c: &Collection,
    max_bytes: Option<u64>,
    keep_scripts: bool,
    seen: &Redactor,
) -> ExportDoc {
    let options = export::PostmanOptions::new().keep_scripts(keep_scripts);
    let exported = export::postman_with(c, &options);
    doc("json", &exported.json, &exported.notes, max_bytes, seen)
}

// The cut follows the scrub, so a secret is never left as a partial fragment.
fn doc(
    format: &str,
    text: &str,
    notes: &[ExportNote],
    max_bytes: Option<u64>,
    seen: &Redactor,
) -> ExportDoc {
    let max = max_bytes.unwrap_or(DEFAULT_BODY_BYTES).min(MAX_BODY_BYTES);
    let (shown, truncated) =
        seen.scrub_cut(text, usize::try_from(max).unwrap_or(usize::MAX), false);
    ExportDoc {
        format: format.to_owned(),
        text: shown,
        total_bytes: text.len() as u64,
        truncated,
        notes: notes
            .iter()
            .map(|n| seen.scrub(&format!("{}: {}", n.source, n.detail)))
            .collect(),
    }
}
