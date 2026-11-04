//! `apinata replace`: preview or apply a text replacement across a collection's requests.

use std::path::PathBuf;
use std::process::ExitCode;

use apinata_core::{Collection, ReplaceReport, ReplaceScope, SearchOptions};
use serde::Serialize;

use crate::{EXIT_USAGE, ReplaceArgs, collection_arg, trust};

#[derive(Serialize)]
struct HitDoc {
    path: String,
    request: String,
    field: &'static str,
    key: Option<String>,
    snippet: String,
    match_start: usize,
    match_end: usize,
}

#[derive(Serialize)]
struct DryRunDoc {
    dry_run: bool,
    hits: Vec<HitDoc>,
    capped: bool,
}

#[derive(Serialize)]
struct ReportDoc {
    replaced: usize,
    files_changed: usize,
    scripts_changed: bool,
    skipped_unsaved: Vec<String>,
    failed: Vec<FailureDoc>,
    summary: String,
}

#[derive(Serialize)]
struct FailureDoc {
    request: String,
    reason: String,
}

/// Preview (`--dry-run`) or apply the replacement: 0 on success even with no matches, 3 when
/// the collection cannot be opened or the regex is invalid.
pub fn run(args: &ReplaceArgs) -> ExitCode {
    match execute(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// The leading collection (present with three words), the pattern and the replacement.
fn split_words(args: &ReplaceArgs) -> (Option<PathBuf>, &str, &str) {
    match &args.third {
        None => (None, &args.first, &args.second),
        Some(replacement) => (Some(PathBuf::from(&args.first)), &args.second, replacement),
    }
}

fn execute(args: &ReplaceArgs) -> anyhow::Result<()> {
    let (leading, pattern, replacement) = split_words(args);
    let named = collection_arg::pick(leading.as_deref(), args.collection.as_deref())?;
    let root = collection_arg::with_hint(
        trust::collection_root(named.as_deref()),
        &collection_arg::optional_hint("apinata replace [COLLECTION] <PATTERN> <REPLACEMENT>"),
    )?;
    let mut collection = Collection::open(&root)?;
    let options = SearchOptions::default()
        .with_case_sensitive(args.case)
        .with_regex(args.regex);
    if args.dry_run {
        let results = collection.search_with(pattern, &options)?;
        if args.json {
            let hits = results
                .hits
                .iter()
                .map(|h| HitDoc {
                    path: h.path.as_str().to_owned(),
                    request: h.request_name.clone(),
                    field: h.field.as_str(),
                    key: h.key.clone(),
                    snippet: h.snippet.clone(),
                    match_start: h.match_range.start,
                    match_end: h.match_range.end,
                })
                .collect();
            let doc = DryRunDoc {
                dry_run: true,
                hits,
                capped: results.capped,
            };
            outln!("{}", serde_json::to_string_pretty(&doc)?);
        } else {
            out!("{}", dry_run_text(&results));
        }
        return Ok(());
    }
    let report = collection.replace(pattern, replacement, &options, ReplaceScope::all())?;
    // Problems go to stderr so stdout stays the one summary line.
    for name in &report.skipped_unsaved {
        eprintln!("skipped (unsaved edits): {name}");
    }
    for (name, reason) in &report.failed {
        eprintln!("failed: {name}: {reason}");
    }
    if args.json {
        outln!("{}", serde_json::to_string_pretty(&report_doc(&report))?);
    } else {
        outln!("{}", report.summary());
    }
    Ok(())
}

fn report_doc(report: &ReplaceReport) -> ReportDoc {
    ReportDoc {
        replaced: report.replaced,
        files_changed: report.files_changed,
        scripts_changed: report.scripts_changed,
        skipped_unsaved: report.skipped_unsaved.clone(),
        failed: report
            .failed
            .iter()
            .map(|(request, reason)| FailureDoc {
                request: request.clone(),
                reason: reason.clone(),
            })
            .collect(),
        summary: report.summary(),
    }
}

/// Hits grouped under their file path, one indented line per hit.
fn dry_run_text(results: &apinata_core::SearchResults) -> String {
    let mut out = String::new();
    let mut current: Option<&str> = None;
    for h in &results.hits {
        if current != Some(h.path.as_str()) {
            out.push_str(h.path.as_str());
            out.push('\n');
            current = Some(h.path.as_str());
        }
        let key = h
            .key
            .as_deref()
            .map(|k| format!(" {k}"))
            .unwrap_or_default();
        out.push_str(&format!("  {}{key}  {}\n", h.field.as_str(), h.snippet));
    }
    if results.capped {
        out.push_str(&format!(
            "(showing the first {} hits)\n",
            apinata_core::SEARCH_HIT_CAP
        ));
    }
    out
}
