//! `replace`: `Collection::replace` over the request files, previewing unless told otherwise.

use std::collections::BTreeSet;

use apinata_core::{Collection, ReplaceScope, SearchOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::search::HitDoc;

/// Arguments of `replace`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceArgs {
    /// The text to find; with `regex`, an ECMAScript regular expression.
    pub pattern: String,
    /// What to put in its place; with `regex`, `$1` style groups are expanded.
    pub replacement: String,
    /// Treat `pattern` as an ECMAScript regular expression. Default false: it is literal text.
    #[serde(default)]
    pub regex: bool,
    /// Match letter case exactly. Default false: case is ignored.
    #[serde(default)]
    pub case: bool,
    /// Only list the hits and write nothing. Default true: set false to apply the replacement.
    pub dry_run: Option<bool>,
}

/// A request that could not be edited or saved.
#[derive(Serialize, JsonSchema)]
pub(crate) struct FailureDoc {
    /// The request's name.
    request: String,
    /// Why it was left as on disk.
    reason: String,
}

/// What `replace` returns: the hits for a dry run, the report for a real one.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ReplaceDoc {
    /// True when nothing was written.
    dry_run: bool,
    /// One line, as `apinata replace` prints it.
    summary: String,
    /// Dry run only: the matches a real run would replace; the list stops at the search cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    hits: Option<Vec<HitDoc>>,
    /// Dry run only: true when the hit list was cut at the cap, so more files may match.
    #[serde(skip_serializing_if = "Option::is_none")]
    capped: Option<bool>,
    /// Real run only: matches replaced in saved files.
    #[serde(skip_serializing_if = "Option::is_none")]
    replaced: Option<usize>,
    /// Real run only: request files rewritten.
    #[serde(skip_serializing_if = "Option::is_none")]
    files_changed: Option<usize>,
    /// Real run only: the paths of the rewritten files.
    #[serde(skip_serializing_if = "Option::is_none")]
    changed_files: Option<Vec<String>>,
    /// Real run only: requests with matches left alone because the app holds unsaved edits.
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_unsaved: Option<Vec<String>>,
    /// Real run only: requests that could not be edited or saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    failed: Option<Vec<FailureDoc>>,
    /// Real run only: a script changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    scripts_changed: Option<bool>,
    /// Present when a script changed: what that means for trust.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

/// Preview the replacement unless `dry_run` is false; an invalid regex is the core's message.
pub(crate) fn replace(c: &mut Collection, args: &ReplaceArgs) -> Result<ReplaceDoc, String> {
    let options = SearchOptions::default()
        .with_case_sensitive(args.case)
        .with_regex(args.regex);
    if args.dry_run.unwrap_or(true) {
        let results = c
            .search_with(&args.pattern, &options)
            .map_err(|e| e.to_string())?;
        let n = results
            .hits
            .iter()
            .map(|h| h.path.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let noun = if n == 1 { "file" } else { "files" };
        let at_least = if results.capped { "at least " } else { "" };
        return Ok(ReplaceDoc {
            dry_run: true,
            summary: format!("Would replace in {at_least}{n} {noun}"),
            hits: Some(results.hits.iter().map(HitDoc::from_hit).collect()),
            capped: Some(results.capped),
            replaced: None,
            files_changed: None,
            changed_files: None,
            skipped_unsaved: None,
            failed: None,
            scripts_changed: None,
            note: None,
        });
    }
    let report = c
        .replace(
            &args.pattern,
            &args.replacement,
            &options,
            ReplaceScope::all(),
        )
        .map_err(|e| e.to_string())?;
    Ok(ReplaceDoc {
        dry_run: false,
        summary: report.summary(),
        hits: None,
        capped: None,
        replaced: Some(report.replaced),
        files_changed: Some(report.files_changed),
        changed_files: Some(
            report
                .changed_paths
                .iter()
                .map(|p| p.as_str().to_owned())
                .collect(),
        ),
        skipped_unsaved: Some(report.skipped_unsaved.clone()),
        failed: Some(
            report
                .failed
                .iter()
                .map(|(request, reason)| FailureDoc {
                    request: request.clone(),
                    reason: reason.clone(),
                })
                .collect(),
        ),
        scripts_changed: Some(report.scripts_changed),
        note: report.scripts_changed.then(|| {
            "Changed scripts have new hashes, so the Apinata app will ask the user to trust \
             them again before they run."
                .to_owned()
        }),
    })
}
