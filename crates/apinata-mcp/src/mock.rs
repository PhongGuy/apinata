//! Argument and result types for the mock tools. Core's mock types stay out of the tool
//! schemas; each is copied into a plain document here.

use apinata_core::mock::{MockHit, MockRoute};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `mock_start`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MockStartArgs {
    /// The environment to resolve route URLs against; the server's default when absent.
    pub env: Option<String>,
    /// The loopback port to listen on; a free one when absent or 0.
    pub port: Option<u16>,
    /// Milliseconds to wait before every response.
    pub delay_ms: Option<u64>,
}

/// What `mock_start` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct MockStarted {
    /// The base URL the mock answers on, such as `http://127.0.0.1:4010`.
    pub url: String,
    pub routes: Vec<MockRouteDoc>,
    /// Requests left out of the route table, and why.
    pub warnings: Vec<String>,
}

/// One route the mock serves.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct MockRouteDoc {
    pub method: String,
    pub path: String,
    /// How many saved examples back the route.
    pub examples: usize,
}

impl From<&MockRoute> for MockRouteDoc {
    fn from(route: &MockRoute) -> Self {
        MockRouteDoc {
            method: route.method.clone(),
            path: route.path.clone(),
            examples: route.examples,
        }
    }
}

/// What `mock_stop` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct MockStopped {
    /// The base URL that was released.
    pub url: String,
    /// How many requests the mock answered.
    pub hits: usize,
}

/// `mock_hits`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MockHitsArgs {
    /// How many hits to skip from the start of the log; pass the last result's `next`.
    pub since: Option<usize>,
}

/// What `mock_hits` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct MockHits {
    pub hits: Vec<MockHitDoc>,
    /// The length of the log; pass it as `since` to see only later hits.
    pub next: usize,
}

/// One request the mock answered.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct MockHitDoc {
    /// RFC 3339 UTC, to the second.
    pub at: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// The saved request that answered, or null for a miss.
    pub request: Option<String>,
    /// The example that answered, or null for a miss.
    pub example: Option<String>,
    /// Whether the example was the fallback rather than a match.
    pub fallback: bool,
}

impl From<MockHit> for MockHitDoc {
    fn from(hit: MockHit) -> Self {
        MockHitDoc {
            at: hit.at,
            method: hit.method,
            path: hit.path,
            status: hit.status,
            request: hit.request,
            example: hit.example,
            fallback: hit.fallback,
        }
    }
}

/// The hits after the first `since`, with the log length as the next offset.
pub(crate) fn hits_since(log: Vec<MockHit>, since: usize) -> MockHits {
    let next = log.len();
    MockHits {
        hits: log.into_iter().skip(since).map(Into::into).collect(),
        next,
    }
}
