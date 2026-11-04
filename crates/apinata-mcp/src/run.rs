//! `run_folder`: plan a run, re-resolve every planned request for the run's sensitive set and
//! `Host` check, then run it with core's runner and report outcomes without bodies.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use apinata_core::{
    Collection, ItemPath, IterationData, OAuth2Error, Outcome, RequestFile, RequestResult,
    ResolvedRequest, RunError, RunPlan, Summary, TagFilter, VarContext, assertion_sensitive_values,
    resolve,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::{self, QueryMismatchDoc};
use crate::redact::Redactor;
use crate::scripts::{self, ConsoleDoc, TestDoc};
use crate::send::{self, AssertionDoc, VarDoc};

/// `run_folder`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunArgs {
    /// A folder or request item path; `""` runs the whole collection.
    pub path: String,
    /// The environment to resolve against; the server's default when absent.
    pub environment: Option<String>,
    /// Stop at the first failed or errored request, skipping the rest.
    #[serde(default)]
    pub bail: bool,
    /// Run without scripts; script tests are reported as skipped.
    #[serde(default)]
    pub skip_scripts: bool,
    /// Milliseconds to pause between requests.
    #[serde(default)]
    pub delay_ms: u64,
    /// Send a request again, up to this many more times (0 to 10), when the send fails in transit
    /// or the answer is 502, 503 or 504. A failed assertion, test or script never retries; only
    /// GET, HEAD, OPTIONS, PUT, DELETE and TRACE retry unless `retry_unsafe` is set.
    #[serde(default)]
    pub retries: u32,
    /// Milliseconds to pause before each retry; a longer `Retry-After` wins, and no pause passes 60 seconds.
    #[serde(default)]
    pub retry_delay_ms: u64,
    /// Also retry POST and PATCH, which a repeat could apply twice.
    #[serde(default)]
    pub retry_unsafe: bool,
    /// Fail a request whose final reply status is in range even with no assertions: `error`
    /// (400 and above), `4xx` or `5xx`. Off when omitted.
    #[serde(default)]
    pub fail_on_status: Option<String>,
    /// Run only requests with one of these tags (own or inherited from folders).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Skip requests with one of these tags.
    #[serde(default)]
    pub exclude_tags: Vec<String>,
    /// For event-streams, end each after this many seconds; 30 when absent, 1 to 300.
    pub stream_max_seconds: Option<u64>,
    /// For event-streams, end each after this many events; 10000 when absent, 1 to 100000.
    pub stream_max_events: Option<u64>,
    /// Cap every request's timeout at this many seconds, 1 to 86400. The lower of a request's own
    /// timeout and this value applies; a request with none gets the cap.
    pub timeout_secs: Option<u64>,
    /// Run the folder this many times, 1 to 100 (the iteration cap). With `data`, at most one
    /// pass per row. Each pass sees `iteration` (from 0) and `iterationCount`.
    pub iterations: Option<u64>,
    /// Rows of iteration data, one object per pass, at most 100. Each key becomes a variable
    /// for that pass; values bind as strings (numbers as JavaScript prints them, null as empty,
    /// arrays and objects as JSON). Inline only: the tool never reads a data file.
    pub data: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
    /// Neither store `Set-Cookie` headers nor send the jar's cookies during this run.
    #[serde(default)]
    pub disable_cookies: bool,
}

/// The most passes one `run_folder` call may make, and the most data rows it may carry.
pub(crate) const MAX_ITERATIONS: u64 = 100;

/// The most retries one request may be given.
pub(crate) const MAX_RETRIES: u32 = 10;

impl RunArgs {
    /// The `fail_on_status` rule, or the message naming the bad value.
    pub(crate) fn status_rule(&self) -> Result<Option<apinata_core::StatusRule>, String> {
        let Some(text) = &self.fail_on_status else {
            return Ok(None);
        };
        text.parse()
            .map(Some)
            .map_err(|e| format!("`fail_on_status`: {e}"))
    }

    /// The run's iteration data and pass count, checked against the cap. Rows go through core's
    /// JSON data parser, so values bind exactly as the CLI's `--json-file-path` binds them.
    pub(crate) fn iteration_plan(&self) -> Result<(Option<IterationData>, Option<usize>), String> {
        let count = match self.iterations {
            Some(0) => return Err("`iterations` must be at least 1".to_owned()),
            Some(n) if n > MAX_ITERATIONS => {
                return Err(format!(
                    "`iterations` is {n}; a run is capped at {MAX_ITERATIONS} iterations"
                ));
            }
            other => other.map(|n| n as usize),
        };
        let data = match &self.data {
            None => None,
            Some(rows) if rows.len() as u64 > MAX_ITERATIONS => {
                return Err(format!(
                    "`data` has {} rows; a run is capped at {MAX_ITERATIONS} iterations",
                    rows.len()
                ));
            }
            Some(rows) => {
                let text = serde_json::to_string(rows).map_err(|e| e.to_string())?;
                Some(IterationData::from_json(&text).map_err(|e| format!("`data`: {e}"))?)
            }
        };
        Ok((data, count))
    }
}

impl RunArgs {
    /// The retry settings, checked against the cap.
    pub(crate) fn retry_plan(&self) -> Result<(u32, Duration, bool), String> {
        if self.retries > MAX_RETRIES {
            return Err(format!(
                "`retries` is {}; a request is capped at {MAX_RETRIES} retries",
                self.retries
            ));
        }
        Ok((
            self.retries,
            Duration::from_millis(self.retry_delay_ms),
            self.retry_unsafe,
        ))
    }
}

impl RunArgs {
    /// The run's tag filter, each tag trimmed and empties dropped as the CLI does.
    pub(crate) fn tag_filter(&self) -> TagFilter {
        let clean = |tags: &[String]| tags.iter().flat_map(|t| TagFilter::parse_list(t)).collect();
        TagFilter::new(clean(&self.tags), clean(&self.exclude_tags))
    }
}

/// Counts over a run's results.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct RunSummary {
    pub requests: usize,
    pub passed: usize,
    /// Passed requests whose checks were all skipped; a subset of `passed`.
    pub unverified: usize,
    /// Passed requests with no check at all; a subset of `passed`.
    pub no_tests: usize,
    pub failed: usize,
    pub errored: usize,
    /// Requests never sent (the run bailed or was cancelled).
    pub skipped: usize,
    pub cancelled: usize,
}

/// One request's outcome: `passed`, `failed`, `errored`, `skipped` or `cancelled`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct RunItem {
    pub path: String,
    pub name: String,
    pub outcome: String,
    pub status: Option<u16>,
    /// How many times the request was sent; absent when once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    pub error: Option<String>,
    /// Why a reply status failed the request under `fail_on_status`; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_failure: Option<String>,
    /// The failure of the attempt before a cancel that came during the pause before a retry;
    /// absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_error: Option<String>,
    pub assertions: Vec<AssertionDoc>,
    pub post_response_vars: Vec<VarDoc>,
    pub tests: Vec<TestDoc>,
    pub console: Vec<ConsoleDoc>,
    /// How an event-stream ended; absent for an ordinary response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_end: Option<String>,
    /// The count of events parsed from a streamed body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<usize>,
    /// Present when the request's URL query and enabled query params disagree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_mismatch: Option<QueryMismatchDoc>,
}

/// What `run_folder` returns; no bodies.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct RunResult {
    pub summary: RunSummary,
    /// Every request's outcome in run order; empty when the run iterated (see `iterations`).
    pub results: Vec<RunItem>,
    /// For a run with `iterations` or `data`: one entry per pass, absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<Vec<IterationDoc>>,
    pub load_errors: Vec<String>,
}

/// One pass of an iterated run: its data row and the requests it ran.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct IterationDoc {
    /// The pass number, from 0.
    pub index: usize,
    /// The row's variables as strings; empty when the run has no data.
    pub data: serde_json::Map<String, serde_json::Value>,
    pub results: Vec<RunItem>,
}

/// A planned run with its sensitive set and each request file, for judging results.
pub(crate) struct PreparedRun {
    pub plan: RunPlan,
    pub ctx: VarContext,
    pub files: HashMap<ItemPath, RequestFile>,
}

/// The run of `path`: the folder or request it names, or the whole collection for `""`, keeping
/// only the requests `filter` keeps.
pub(crate) fn plan(c: &Collection, path: &str, filter: &TagFilter) -> Result<RunPlan, String> {
    let target = if path.is_empty() {
        c.root().to_path_buf()
    } else {
        match ItemPath::new(path) {
            Some(item) => c.root().join(item.as_str()),
            None => return Err(format!("`{path}` is not an item path")),
        }
    };
    c.plan(&target)
        .map(|plan| plan.filter_tags(filter))
        .map_err(|e| e.to_string())
}

/// Plan the run of `path` under `filter` and re-resolve each planned request: its sensitive values
/// and assertion values join the run's set, and a `Host` header naming another host refuses the
/// whole run before anything is sent. A request that fails to resolve is left for the run to
/// report, unless it has a `Host` header. A capture can still change a header mid-run, so each
/// request is checked again as it is sent (see [`host_guard`]); `scripted` requests, whose
/// pre-request script may set headers, are left to that check alone. The redactor comes back on
/// success and failure alike.
pub(crate) fn prepare(
    c: &Collection,
    path: &str,
    filter: &TagFilter,
    ctx: VarContext,
    scripted: &HashSet<ItemPath>,
) -> (Redactor, Result<PreparedRun, String>) {
    let mut redactor = Redactor::default();
    let plan = match plan(c, path, filter) {
        Ok(plan) => plan,
        Err(e) => return (redactor, Err(e)),
    };
    // A run that would send nothing is refused as a tool error, as the CLI refuses it.
    if let Some(message) = plan.empty_message(filter) {
        return (redactor, Err(message));
    }
    // Every request's sensitive values join the set before any refusal is decided.
    let mut fronting = None;
    let mut files = HashMap::new();
    for (item, file) in plan.items().zip(plan.requests()) {
        redactor.add_values(assertion_sensitive_values(file, &ctx));
        let refusal = match resolve(file, &ctx) {
            Ok(resolved) => {
                redactor.add_values(resolved.sensitive_values());
                redactor.add_basic_auth(resolved.headers());
                redactor.add_basic_auth(&send::url_basic_auth(resolved.url()));
                let checked = !scripted.contains(item);
                send::fronted(&resolved)
                    .filter(|_| checked)
                    .map(|host| format!("the Host header `{host}` differs from the URL's host"))
            }
            Err(_) if scripted.contains(item) => None,
            Err(_) if has_host_header(c, item, file) => {
                Some("the Host header could not be checked before the run".to_owned())
            }
            Err(_) => None,
        };
        if fronting.is_none()
            && let Some(why) = refusal
        {
            fronting = Some(format!("`{}`: {why}; nothing was sent", item.as_str()));
        }
        files.insert(item.clone(), file.clone());
    }
    if let Some(refused) = fronting {
        return (redactor, Err(refused));
    }
    let run = PreparedRun { plan, ctx, files };
    (redactor, Ok(run))
}

/// Whether a `Host` header, in any case, is enabled on `file` or on the folders and collection
/// it inherits headers from.
fn has_host_header(c: &Collection, item: &ItemPath, file: &RequestFile) -> bool {
    let mut headers = file.headers();
    headers.extend(c.file().headers());
    let mut folder = item.parent();
    while let Some(path) = folder {
        if let Some(f) = c.folder(&path).and_then(|f| f.file()) {
            headers.extend(f.headers());
        }
        folder = path.parent();
    }
    headers
        .iter()
        .any(|h| h.enabled && h.name.trim().eq_ignore_ascii_case("host"))
}

/// The send-time check of each run request: its sensitive values join `seen` first, since a
/// request that resolved only after a capture was never registered before the run; then a
/// `Host` header naming a host other than its URL's refuses it.
pub(crate) fn host_guard(req: &ResolvedRequest, seen: &Mutex<Redactor>) -> Option<String> {
    let mut found = Redactor::default();
    found.add_values(req.sensitive_values());
    found.add_basic_auth(req.headers());
    found.add_basic_auth(&send::url_basic_auth(req.url()));
    seen.lock().unwrap_or_else(|e| e.into_inner()).merge(&found);
    send::fronted(req).map(|host| {
        format!("the Host header `{host}` differs from the URL's host; nothing was sent")
    })
}

/// The run's summary in the tool's shape.
pub(crate) fn summary(s: &Summary) -> RunSummary {
    RunSummary {
        requests: s.requests,
        passed: s.passed,
        unverified: s.unverified,
        no_tests: s.no_tests,
        failed: s.failed,
        errored: s.errored,
        skipped: s.skipped,
        cancelled: s.cancelled,
    }
}

/// An outcome as the tool spells it.
pub(crate) fn outcome(o: Outcome) -> &'static str {
    match o {
        Outcome::Passed => "passed",
        Outcome::Failed => "failed",
        Outcome::Errored => "errored",
        Outcome::Skipped => "skipped",
        Outcome::Cancelled => "cancelled",
        _ => "unknown",
    }
}

/// One result in the tool's shape; `exec_error` answers a send failure (and files approvals).
pub(crate) fn item(
    result: &RequestResult,
    run: &PreparedRun,
    exec_error: &dyn Fn(&apinata_core::ExecError, &str) -> String,
) -> RunItem {
    let file = run.files.get(&result.path);
    let describe = |e: &RunError| match (e, file) {
        (RunError::Resolve(e), Some(file)) => send::resolve_message(e, file, &run.ctx),
        (RunError::Exec(e), _) => exec_error(e, result.path.as_str()),
        (RunError::OAuth2(OAuth2Error::Exec(e)), _) => exec_error(e, result.path.as_str()),
        (RunError::OAuth2(OAuth2Error::Resolve(e)), Some(file)) => {
            send::resolve_message(e, file, &run.ctx)
        }
        (other, _) => other.to_string(),
    };
    let error = result.error.as_ref().map(describe);
    // The tool result is scrubbed of every sensitive value afterwards, this text included.
    let last_attempt_error = result.last_attempt_error.as_ref().map(describe);
    let (stream_end, events) = result
        .response
        .as_ref()
        .map_or((None, None), send::stream_fields);
    RunItem {
        path: result.path.as_str().to_owned(),
        name: result.name.clone(),
        outcome: outcome(result.outcome).to_owned(),
        status: result.response.as_ref().map(|r| r.status()),
        attempts: (result.attempts() > 1).then(|| result.attempts()),
        error,
        status_failure: result.status_failure_reason(),
        last_attempt_error,
        assertions: match file {
            Some(file) => result
                .assertions
                .iter()
                .map(|a| send::assertion_doc(a, file, &run.ctx))
                .collect(),
            None => Vec::new(),
        },
        post_response_vars: result
            .post_response_vars
            .iter()
            .map(send::var_doc)
            .collect(),
        tests: scripts::test_docs(result),
        console: scripts::console_docs(result),
        stream_end,
        events,
        query_mismatch: file.and_then(dto::query_mismatch_doc),
    }
}

/// Group a run's items by pass. Every pass that ran gets an entry with its data row, values
/// scrubbed of the run's sensitive set; passes that never started (a bail) have none.
pub(crate) fn group(
    items: Vec<(Option<usize>, RunItem)>,
    data: Option<&IterationData>,
    seen: &Redactor,
) -> Vec<IterationDoc> {
    let mut docs: Vec<IterationDoc> = Vec::new();
    for (iteration, item) in items {
        let index = iteration.unwrap_or(0);
        if docs.last().is_none_or(|d| d.index != index) {
            let row = data.and_then(|d| d.row(index)).unwrap_or(&[]);
            docs.push(IterationDoc {
                index,
                data: row
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(seen.scrub(v))))
                    .collect(),
                results: Vec::new(),
            });
        }
        if let Some(doc) = docs.last_mut() {
            doc.results.push(item);
        }
    }
    docs
}

#[cfg(test)]
mod tests {
    use super::*;
    use apinata_core::{
        CancelHandle, ContextOptions, ExecOptions, Executor, RunEvent, RunOptions, run,
    };
    use std::sync::Arc;

    /// Runs one GET to `url` with a 30 s retry pause, cancels once the pause starts, and
    /// returns the item the tool reports for it.
    async fn cancelled_item(url: &str) -> serde_json::Value {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
        let req = format!(
            "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
        );
        std::fs::write(dir.path().join("opencollection.yml"), root).expect("root");
        std::fs::write(dir.path().join("r.yml"), req).expect("request");
        let c = Collection::open(dir.path()).expect("opens");
        let plan = c.plan(dir.path()).expect("plan");
        let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
        let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
        let cancel = CancelHandle::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let on_event = Box::new(move |e: RunEvent| {
            if let RunEvent::Retrying { .. } = e {
                let _ = tx.send(());
            }
        });
        let options = RunOptions::new()
            .retries(2)
            .retry_delay(Duration::from_secs(30));
        let prepared = PreparedRun {
            plan: plan.clone(),
            ctx: ctx.clone(),
            files: HashMap::new(),
        };
        let task = tokio::spawn(run(plan, ctx, exec, options, cancel.clone(), on_event));
        tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("a retry is announced");
        cancel.cancel();
        let report = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("run ends at once")
            .expect("joins");
        let refuse = |e: &apinata_core::ExecError, _: &str| e.to_string();
        let item = item(&report.results[0], &prepared, &refuse);
        serde_json::to_value(item).expect("serializes")
    }

    #[tokio::test]
    async fn a_cancel_in_the_pause_carries_the_failure_of_the_attempt_before() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://127.0.0.1:{}/", l.local_addr().expect("addr").port());
        std::thread::spawn(move || {
            while let Ok((conn, _)) = l.accept() {
                drop(conn);
            }
        });
        let item = cancelled_item(&url).await;
        assert_eq!(item["outcome"], "cancelled", "{item}");
        assert_eq!(item["error"], "cancelled", "{item}");
        assert!(
            item["last_attempt_error"]
                .as_str()
                .is_some_and(|e| e.contains("error sending request")),
            "{item}"
        );
    }

    #[tokio::test]
    async fn a_cancel_after_a_502_keeps_the_status_and_has_no_last_attempt_error() {
        use wiremock::matchers::any;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let s = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(502))
            .mount(&s)
            .await;
        let item = cancelled_item(&s.uri()).await;
        assert_eq!(item["outcome"], "cancelled", "{item}");
        assert_eq!(item["status"], 502, "{item}");
        assert!(item.get("last_attempt_error").is_none(), "{item}");
    }
}
