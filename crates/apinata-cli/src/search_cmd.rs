//! `apinata search`: the hits of `Collection::search` as text or JSON.

use std::path::PathBuf;
use std::process::ExitCode;

use apinata_core::{Collection, SEARCH_HIT_CAP, SearchOptions, SearchResults};
use serde::Serialize;

use crate::{EXIT_USAGE, SearchArgs, collection_arg, trust};

/// One hit as JSON; the same shape the MCP `search` tool returns.
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
struct ResultsDoc {
    hits: Vec<HitDoc>,
    /// Absent when paging, where `total` and `has_more` say it.
    #[serde(skip_serializing_if = "Option::is_none")]
    capped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(skip)]
    offset: usize,
}

/// Print the hits for `args.query`: 0 when searched (even with no hits), 3 when the collection
/// cannot be opened.
pub fn run(args: &SearchArgs) -> ExitCode {
    let results = match search(args) {
        Ok(results) => results,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    if args.json {
        return print_json(&results);
    }
    out!("{}", text(&results));
    ExitCode::SUCCESS
}

/// The plain capped search, or one page of it when `--limit` or `--offset` is given.
fn search(args: &SearchArgs) -> anyhow::Result<ResultsDoc> {
    let collection = open(args)?;
    let query = split_query(args).1;
    if args.limit.is_none() && args.offset.is_none() {
        return Ok(doc(&collection.search(query)));
    }
    let mut options = SearchOptions::default().with_offset(args.offset.unwrap_or(0));
    if let Some(limit) = args.limit {
        options = options.with_limit(limit);
    }
    let page = collection.search_page(query, &options)?;
    Ok(ResultsDoc {
        hits: page.hits.iter().map(hit_doc).collect(),
        capped: None,
        total: Some(page.total),
        has_more: Some(page.has_more),
        limit: Some(page.limit),
        offset: page.offset,
    })
}

fn open(args: &SearchArgs) -> anyhow::Result<Collection> {
    let (leading, _) = split_query(args);
    let named = collection_arg::pick(leading.as_deref(), args.collection.as_deref())?;
    let root: PathBuf = collection_arg::with_hint(
        trust::collection_root(named.as_deref()),
        &collection_arg::optional_hint("apinata search [COLLECTION] <QUERY>"),
    )?;
    Ok(Collection::open(&root)?)
}

/// The leading collection (present with two words) and the query, which is always last.
fn split_query(args: &SearchArgs) -> (Option<PathBuf>, &str) {
    match args.words.as_slice() {
        [query] => (None, query),
        [collection, query] => (Some(PathBuf::from(collection)), query),
        _ => unreachable!("clap takes one or two words"),
    }
}

fn hit_doc(h: &apinata_core::SearchHit) -> HitDoc {
    HitDoc {
        path: h.path.as_str().to_owned(),
        request: h.request_name.clone(),
        field: h.field.as_str(),
        key: h.key.clone(),
        snippet: h.snippet.clone(),
        match_start: h.match_range.start,
        match_end: h.match_range.end,
    }
}

fn doc(results: &SearchResults) -> ResultsDoc {
    ResultsDoc {
        hits: results.hits.iter().map(hit_doc).collect(),
        capped: Some(results.capped),
        total: None,
        has_more: None,
        limit: None,
        offset: 0,
    }
}

fn print_json(results: &ResultsDoc) -> ExitCode {
    match serde_json::to_string_pretty(results) {
        Ok(json) => {
            outln!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// One line per hit: the request path, the field with its key, then the snippet; a closing line
/// when the cap cut the list short.
fn text(results: &ResultsDoc) -> String {
    let mut out = String::new();
    for h in &results.hits {
        let key = h
            .key
            .as_deref()
            .map(|k| format!(" {k}"))
            .unwrap_or_default();
        out.push_str(&format!("{}  {}{key}  {}\n", h.path, h.field, h.snippet));
    }
    if results.capped == Some(true) {
        out.push_str(&format!("(showing the first {SEARCH_HIT_CAP} hits)\n"));
    }
    if let Some(total) = results.total {
        let (offset, shown) = (results.offset, results.hits.len());
        if shown == 0 {
            out.push_str(&format!("(no hits at offset {offset} of {total})\n"));
        } else {
            out.push_str(&format!(
                "(showing {}-{} of {total})\n",
                offset + 1,
                offset + shown
            ));
        }
    }
    out
}
