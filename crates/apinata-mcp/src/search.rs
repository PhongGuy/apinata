//! `search`: `Collection::search` over the request files' own text, as the CLI prints it.

use apinata_core::{Collection, SearchOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Arguments of `search`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchArgs {
    /// The text to find; with `regex`, an ECMAScript regular expression.
    pub query: String,
    /// Match letter case exactly. Default false: case is ignored.
    #[serde(default)]
    pub case: bool,
    /// Treat `query` as an ECMAScript regular expression. Default false: it is literal text.
    #[serde(default)]
    pub regex: bool,
    /// The most hits to return (1 to 500; larger is clamped). Giving `limit` or `offset` pages
    /// the search: the reply then has `total` and `has_more`.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many hits to skip, to fetch the next page.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// One match in one field of one request.
#[derive(Serialize, JsonSchema)]
pub(crate) struct HitDoc {
    /// The request's path, as other tools take it.
    path: String,
    /// The request's name.
    request: String,
    /// Where the match is: name, url, path_param, header_name, header_value, body, script or
    /// docs.
    field: &'static str,
    /// The header, param or form field name, the script phase or the GraphQL part; absent
    /// when the field has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    /// One line of text around the first match.
    snippet: String,
    /// Where the match starts within `snippet`, in bytes.
    match_start: usize,
    /// Where the match ends within `snippet`, in bytes.
    match_end: usize,
}

impl HitDoc {
    /// The document for one core hit; `replace` previews with the same shape.
    pub(crate) fn from_hit(h: &apinata_core::SearchHit) -> HitDoc {
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
}

/// What `search` returns.
#[derive(Serialize, JsonSchema)]
pub(crate) struct SearchDoc {
    /// The hits in tree order, at most 500.
    hits: Vec<HitDoc>,
    /// More hits existed than the 500 returned; absent when the search is paged.
    #[serde(skip_serializing_if = "Option::is_none")]
    capped: Option<bool>,
    /// How many hits matched in all; present when `limit` or `offset` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    /// More hits follow this page: call again with a larger `offset`.
    #[serde(skip_serializing_if = "Option::is_none")]
    has_more: Option<bool>,
    /// The page size used, after clamping to 500.
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

/// The hits for the query; variables stay as `{{name}}` and secret values are never read.
/// An invalid pattern in regex mode comes back as the core's message.
pub(crate) fn search(c: &Collection, args: &SearchArgs) -> Result<SearchDoc, String> {
    let options = SearchOptions::default()
        .with_case_sensitive(args.case)
        .with_regex(args.regex);
    if args.limit.is_some() || args.offset.is_some() {
        let mut options = options.with_offset(args.offset.unwrap_or(0));
        if let Some(limit) = args.limit {
            options = options.with_limit(limit);
        }
        let page = c
            .search_page(&args.query, &options)
            .map_err(|e| e.to_string())?;
        return Ok(SearchDoc {
            hits: page.hits.iter().map(HitDoc::from_hit).collect(),
            capped: None,
            total: Some(page.total),
            has_more: Some(page.has_more),
            limit: Some(page.limit),
        });
    }
    let results = c
        .search_with(&args.query, &options)
        .map_err(|e| e.to_string())?;
    Ok(SearchDoc {
        hits: results.hits.iter().map(HitDoc::from_hit).collect(),
        capped: Some(results.capped),
        total: None,
        has_more: None,
        limit: None,
    })
}
