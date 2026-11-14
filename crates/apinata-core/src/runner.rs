//! Running a collection: selecting the requests a path names, in Bruno's order.

use crate::assertion::{AssertionOutcome, Verdict, evaluate, evaluate_scripted};
use crate::cancel::CancelHandle;
use crate::collection::{Collection, Folder};
use crate::context::VarContext;
use crate::cookies::SetCookie;
use crate::error::{FileError, ResolveError, UndefinedHint};
use crate::execute::{ExecError, Executor, SendTag};
use crate::item_path::ItemPath;
use crate::iteration_data::{IterationData, iteration_total};
use crate::oauth2::{AuthorizeOptions, OAuth2Error};
use crate::post_response::{VarOutcome, apply_post_response_vars_with};
use crate::prompt::missing_labels;
use crate::request::RequestFile;
use crate::resolve::{ResolvedRequest, Warning, prepare, resolve_prepared};
use crate::response::{CookieOrigin, Response};
use crate::retry::{Retry, RetryHook, send_with_retries};
use crate::script::expr::Expressions;
use crate::script::host::HostState;
use crate::script::segments::segments;
use crate::script::{
    ConsoleEntry, ExecutionMode, PhaseEnd, Record, RunnerControl, ScriptOptions, ScriptPhase,
    TestOutcome,
};
use crate::secret_names::log_values;
use crate::session_log::{ConsoleLine, Logged, Redactor, SendHold, SendSource};
use crate::stream::{StreamEnd, StreamEvent, StreamLimits, StreamOptions};
use crate::subscription::is_subscription;
use crate::tags::{TagFilter, normalize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// An owned snapshot of what a run sends: the selected requests in run order and the
/// load errors that bear on them, in the collection's order.
#[derive(Clone)]
#[non_exhaustive]
pub struct RunPlan {
    requests: Vec<(ItemPath, RequestFile, Vec<String>)>,
    load_errors: Vec<FileError>,
}

impl RunPlan {
    /// The selected requests in run order.
    pub fn requests(&self) -> impl Iterator<Item = &RequestFile> {
        self.requests.iter().map(|(_, request, _)| request)
    }

    /// The selected requests' item paths in run order, paired one to one with `requests`.
    pub fn items(&self) -> impl Iterator<Item = &ItemPath> {
        self.requests.iter().map(|(path, _, _)| path)
    }

    /// The load errors that bear on the selection, in the collection's order.
    pub fn load_errors(&self) -> &[FileError] {
        &self.load_errors
    }

    /// Why a run of this plan would do nothing: `Some` only when no request is selected and no
    /// load error exists (a plan with load errors reports those instead). `filter` is the one
    /// that narrowed the plan, so the message can name it.
    pub fn empty_message(&self, filter: &TagFilter) -> Option<String> {
        if !self.requests.is_empty() || !self.load_errors.is_empty() {
            return None;
        }
        Some(match filter.describe() {
            Some(what) => format!("no requests matched: {what}"),
            None => "no requests to run in the selected path".to_owned(),
        })
    }

    /// Each selected request's effective tags, paired one to one with `items()`.
    pub fn tags(&self) -> impl Iterator<Item = &[String]> {
        self.requests.iter().map(|(_, _, tags)| tags.as_slice())
    }

    /// The plan without the requests `filter` rejects; load errors are kept.
    pub fn filter_tags(mut self, filter: &TagFilter) -> RunPlan {
        self.requests.retain(|(_, _, tags)| filter.matches(tags));
        self
    }

    /// The plan without the requests whose item path is not in `items`, in the original plan
    /// order. Load errors are kept only when they bear on a kept request, as `plan_selected`
    /// does. Under a data file the kept requests still run once per row: this selects items,
    /// not iterations.
    pub fn retain_items(mut self, items: &[ItemPath]) -> RunPlan {
        self.requests.retain(|(path, _, _)| items.contains(path));
        let kept: Vec<&Path> = self.requests.iter().map(|(_, r, _)| r.path()).collect();
        self.load_errors
            .retain(|e| kept.iter().any(|k| affects(e.path(), k)));
        self
    }

    /// The plan narrowed to exactly the requests in `requests`, kept in the plan's run order
    /// whatever order or repeats they are given in. The plan's load errors are all kept, since
    /// its scope is unchanged. An id the plan does not hold (a folder, a request outside it or
    /// one that no longer exists) fails with [`SelectError::NotPlanned`].
    pub fn choose(mut self, requests: &[ItemPath]) -> Result<RunPlan, SelectError> {
        if let Some(item) = requests
            .iter()
            .find(|i| !self.requests.iter().any(|(p, _, _)| p == *i))
        {
            return Err(SelectError::NotPlanned { item: item.clone() });
        }
        self.requests.retain(|(path, _, _)| requests.contains(path));
        Ok(self)
    }
}

/// Shows item paths and an error count only, since request files hold header values and bodies.
impl std::fmt::Debug for RunPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let paths: Vec<&str> = self.requests.iter().map(|(p, _, _)| p.as_str()).collect();
        f.debug_struct("RunPlan")
            .field("requests", &paths)
            .field("load_errors", &self.load_errors.len())
            .finish_non_exhaustive()
    }
}

/// Why a path cannot be run.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SelectError {
    /// The path lies under `environments/`; environments are chosen, never run.
    #[error("{} is an environment, not a request", path.display())]
    Environment {
        /// The path that was given.
        path: PathBuf,
    },
    /// The path is neither the root, a folder nor a request file of the collection.
    #[error("{} is not the collection, a folder or a request file", path.display())]
    NotAnItem {
        /// The path that was given.
        path: PathBuf,
    },
    /// A chosen request id is not one of the plan's requests.
    #[error("{item} is not a request in this run")]
    NotPlanned {
        /// The id that was given.
        item: ItemPath,
    },
}

impl Collection {
    /// Plan a run of `target`: the root, a folder or a request file, spelled as the root is.
    /// A target that failed to load selects nothing; its error is kept in the plan.
    pub fn plan(&self, target: &Path) -> Result<RunPlan, SelectError> {
        let selected = self.select(target)?;
        let mut load_errors: Vec<FileError> = self
            .load_errors()
            .iter()
            .filter(|e| affects(e.path(), target))
            .cloned()
            .collect();
        // A loaded file whose name cannot be an item path (non-UTF-8, or holding `:` or `\`)
        // is not run; it is reported after the collection's own load errors instead.
        let mut requests = Vec::with_capacity(selected.len());
        for request in selected {
            match self.item_path(request.path()) {
                Some(item) => {
                    let tags = self.effective_tags(request);
                    requests.push((item, request.clone(), tags));
                }
                None => load_errors.push(FileError::new(
                    request.path(),
                    "the file name cannot be used as an item path, so it is not run",
                )),
            }
        }
        Ok(RunPlan {
            requests,
            load_errors,
        })
    }

    /// The plan for the requests equal to or under any of `items`, in the collection's run order.
    /// An item that no longer exists or is not a folder or request (opaque, draft, load-error file)
    /// is skipped, never an error, so stale or non-runnable members cannot fail a run.
    pub fn plan_selected(&self, items: &[ItemPath]) -> RunPlan {
        // Validity is judged first so a skipped member cannot keep a load error alive.
        let chosen: Vec<PathBuf> = items
            .iter()
            .filter(|i| {
                !i.is_draft()
                    && (i.is_root() || self.folder(i).is_some() || self.request(i).is_some())
            })
            .map(|i| i.to_path(self.root()))
            .collect();
        // Plan the root once and filter, so run order is the plan's and overlaps cannot repeat.
        let Ok(mut plan) = self.plan(self.root()) else {
            return RunPlan {
                requests: Vec::new(),
                load_errors: Vec::new(),
            };
        };
        plan.requests
            .retain(|(_, r, _)| chosen.iter().any(|c| r.path().starts_with(c)));
        plan.load_errors
            .retain(|e| chosen.iter().any(|c| affects(e.path(), c)));
        plan
    }

    /// A request's own tags, then each ancestor folder's from outermost to innermost, each once.
    fn effective_tags(&self, request: &RequestFile) -> Vec<String> {
        let mut tags = request.tags();
        let mut folder = self.tree();
        // Descend through the folders whose directory holds the request; drafts sit in the
        // tree like saved files, so their ancestors are found the same way.
        while let Some(inner) = folder
            .folders()
            .iter()
            .find(|f| request.path().starts_with(f.path()))
        {
            tags.extend(inner.file().map(|f| f.tags()).unwrap_or_default());
            folder = inner;
        }
        normalize(tags)
    }

    /// The requests under `path` in run order; a draft's path selects that draft alone.
    fn select(&self, path: &Path) -> Result<Vec<&RequestFile>, SelectError> {
        if let Some(draft) = self.drafts().find(|d| d.path() == path) {
            return Ok(vec![draft]);
        }
        if let Some(requests) = find(self.tree(), path) {
            return Ok(requests);
        }
        // Environments are chosen, never run, whether or not the file loaded.
        if path.starts_with(self.root().join("environments")) {
            return Err(SelectError::Environment {
                path: path.to_path_buf(),
            });
        }
        // A file that failed to load is reported by the run, so it selects nothing rather than erring.
        if self.load_errors().iter().any(|e| e.path() == path) {
            return Ok(Vec::new());
        }
        Err(SelectError::NotAnItem {
            path: path.to_path_buf(),
        })
    }
}

/// Whether a load error at `file` bears on a run of `selected`: it lies under the selection,
/// or it is the `folder.yml` of an ancestor whose headers, auth and variables the selection inherits.
fn affects(file: &Path, selected: &Path) -> bool {
    let ancestor_folder_file = file.file_name().is_some_and(|n| n == "folder.yml")
        && file.parent().is_some_and(|dir| selected.starts_with(dir));
    file.starts_with(selected) || ancestor_folder_file
}

/// Bruno's run order: depth first, each folder's subfolders before its own requests.
fn collect<'c>(folder: &'c Folder, out: &mut Vec<&'c RequestFile>) {
    for sub in folder.folders() {
        collect(sub, out);
    }
    out.extend(folder.requests());
}

/// The requests `path` selects when it names `folder`, a folder under it or a request file in it.
fn find<'c>(folder: &'c Folder, path: &Path) -> Option<Vec<&'c RequestFile>> {
    if folder.path() == path {
        let mut out = Vec::new();
        collect(folder, &mut out);
        return Some(out);
    }
    if let Some(request) = folder.requests().iter().find(|r| r.path() == path) {
        return Some(vec![request]);
    }
    folder
        .folders()
        .iter()
        .filter(|sub| path.starts_with(sub.path()))
        .find_map(|sub| find(sub, path))
}

/// A check on each resolved request just before it is sent; `Some` refuses it with that message.
pub(crate) type Guard = Arc<dyn Fn(&ResolvedRequest) -> Option<String> + Send + Sync>;

/// How a run behaves.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct RunOptions {
    bail: bool,
    guard: Option<Guard>,
    scripts: ScriptOptions,
    mode: ExecutionMode,
    authorize: AuthorizeOptions,
    delay: Duration,
    iterations: Option<IterationData>,
    iteration_count: Option<usize>,
    stream: Option<StreamOptions>,
    ended: Option<Ended>,
    body_files: Option<BodyFiles>,
    retry: Retry,
    on_retry: Option<RetryHook>,
    fail_on_status: Option<StatusRule>,
}

/// Which reply statuses fail a request that otherwise passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusRule {
    /// 400 and above.
    Error,
    /// 400 to 499.
    Client,
    /// 500 and above.
    Server,
}

impl StatusRule {
    /// Whether `status` is in the failing range.
    pub fn matches(self, status: u16) -> bool {
        match self {
            StatusRule::Error => status >= 400,
            StatusRule::Client => (400..500).contains(&status),
            StatusRule::Server => status >= 500,
        }
    }

    /// The name `FromStr` reads back: `error`, `4xx` or `5xx`.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusRule::Error => "error",
            StatusRule::Client => "4xx",
            StatusRule::Server => "5xx",
        }
    }
}

impl fmt::Display for StatusRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for StatusRule {
    type Err = String;

    fn from_str(s: &str) -> Result<StatusRule, String> {
        match s {
            "error" => Ok(StatusRule::Error),
            "4xx" => Ok(StatusRule::Client),
            "5xx" => Ok(StatusRule::Server),
            other => Err(format!(
                "`{other}` is not a status rule; use error, 4xx or 5xx"
            )),
        }
    }
}

/// Picks the file a request's body is saved to from its run position and item path.
type BodyFiles = Arc<dyn Fn(usize, &ItemPath) -> Option<PathBuf> + Send + Sync>;

/// Called when the final hop's body read has finished.
type Ended = Arc<dyn Fn() + Send + Sync>;

impl fmt::Debug for RunOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunOptions")
            .field("bail", &self.bail)
            .field("guard", &self.guard.is_some())
            .field("scripts", &self.scripts)
            .field("mode", &self.mode)
            .field("authorize", &self.authorize)
            .field("delay", &self.delay)
            .field(
                "iterations",
                &self.iterations.as_ref().map(IterationData::len),
            )
            .field("iteration_count", &self.iteration_count)
            .field("stream", &self.stream.is_some())
            .field("ended", &self.ended.is_some())
            .field("body_files", &self.body_files.is_some())
            .field("retries", &self.retry.retries)
            .field("retry_delay", &self.retry.delay)
            .field("retry_unsafe", &self.retry.unsafe_methods)
            .field("on_retry", &self.on_retry.is_some())
            .field("fail_on_status", &self.fail_on_status)
            .finish()
    }
}

impl RunOptions {
    /// The defaults: every request runs whatever the earlier ones did.
    pub fn new() -> RunOptions {
        RunOptions::default()
    }

    /// Reads each response as a stream under `limits` and reports it as [`RunEvent`]s.
    pub fn stream(mut self, limits: StreamLimits) -> RunOptions {
        self.stream = Some(StreamOptions::new().limits(limits));
        self
    }

    /// Saves each top-level request's final body to the file `resolver` names for its run
    /// position and item path; `None` saves nothing. Turns streaming on with default limits when
    /// it is off. The saved file is complete whatever the body cap.
    pub fn body_files(
        mut self,
        resolver: impl Fn(usize, &ItemPath) -> Option<PathBuf> + Send + Sync + 'static,
    ) -> RunOptions {
        self.body_files = Some(Arc::new(resolver));
        if self.stream.is_none() {
            self.stream = Some(StreamOptions::new());
        }
        self
    }

    /// A clone whose stream options carry `sink` and whose body-end hook is `ended`; a plain
    /// clone when streaming is off.
    fn with_stream_sink(
        &self,
        sink: impl Fn(StreamEvent) + Send + Sync + 'static,
        ended: Ended,
    ) -> RunOptions {
        let mut o = self.clone();
        if let Some(s) = o.stream.take() {
            o.stream = Some(s.sink(sink));
            o.ended = Some(ended);
        }
        o
    }

    /// A clone whose sends report each pause before a retry to `hook`.
    fn with_retry_hook(
        &self,
        hook: impl Fn(u32, u32, Duration) + Send + Sync + 'static,
    ) -> RunOptions {
        let mut o = self.clone();
        o.on_retry = Some(Arc::new(hook));
        o
    }

    /// Re-sends a request up to `n` more times when the send fails in transit or is answered
    /// with 502, 503 or 504; zero (the default) never retries. Only the send repeats: scripts,
    /// variables, post-response work, tests and assertions run once, on the last attempt.
    pub fn retries(mut self, n: u32) -> RunOptions {
        self.retry.retries = n;
        self
    }

    /// The pause before each retry; a longer `Retry-After` wins, and no pause passes 60 seconds.
    pub fn retry_delay(mut self, delay: Duration) -> RunOptions {
        self.retry.delay = delay;
        self
    }

    /// Also retries POST and PATCH, which are skipped by default because a repeat can double
    /// their effect.
    pub fn retry_unsafe(mut self, on: bool) -> RunOptions {
        self.retry.unsafe_methods = on;
        self
    }

    /// Fail a request whose final reply status is in `rule`'s range, even with no assertions.
    /// `None` (the default) judges by checks alone.
    pub fn fail_on_status(mut self, rule: Option<StatusRule>) -> RunOptions {
        self.fail_on_status = rule;
        self
    }

    /// Whether the first failed or errored request skips every later one.
    pub fn bail(mut self, bail: bool) -> RunOptions {
        self.bail = bail;
        self
    }

    /// Check each request after it resolves and before it is sent. Returning a message refuses
    /// it: the request errors with [`RunError::Refused`] and nothing is sent.
    #[allow(clippy::type_complexity)]
    pub fn guard(
        mut self,
        guard: Arc<dyn Fn(&ResolvedRequest) -> Option<String> + Send + Sync>,
    ) -> RunOptions {
        self.guard = Some(guard);
        self
    }

    /// Whether scripts run and under which limits; off by default.
    pub fn scripts(mut self, s: ScriptOptions) -> RunOptions {
        self.scripts = s;
        self
    }

    /// Which caller is sending; a run is [`ExecutionMode::Runner`] by default.
    pub fn execution_mode(mut self, m: ExecutionMode) -> RunOptions {
        self.mode = m;
        self
    }

    /// How each request's oauth2 token may be obtained; [`AuthorizeOptions::new`] by default.
    pub fn authorize(mut self, how: AuthorizeOptions) -> RunOptions {
        self.authorize = how;
        self
    }

    /// Pause between consecutive sent requests; zero (the default) means none.
    pub fn delay(mut self, delay: Duration) -> RunOptions {
        self.delay = delay;
        self
    }

    /// Run the plan once per row, each row bound as [`VarScope::IterationData`](crate::VarScope::IterationData).
    pub fn iterations(mut self, data: IterationData) -> RunOptions {
        self.iterations = Some(data);
        self
    }

    /// Run at most `n` iterations. Without data that is `n` passes with no row; with data it
    /// caps the rows and never repeats one. `0` is raised to `1`.
    pub fn iteration_count(mut self, n: usize) -> RunOptions {
        self.iteration_count = Some(n.max(1));
        self
    }
}

/// How one request of a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// Sent, and no assertion failed.
    Passed,
    /// Sent, and at least one assertion failed.
    Failed,
    /// It could not be resolved or sent.
    Errored,
    /// Never resolved or sent, because the run bailed or was cancelled.
    Skipped,
    /// Its send was cancelled while in flight.
    Cancelled,
}

/// Why a request of a run errored.
#[derive(Clone, thiserror::Error)]
#[non_exhaustive]
pub enum RunError {
    /// The request could not be resolved against its variables.
    #[error(transparent)]
    Resolve(ResolveError),
    /// The request could not be sent or its response not read.
    #[error(transparent)]
    Exec(ExecError),
    /// The run's guard refused the resolved request, so it was not sent.
    #[error("{message}")]
    Refused {
        /// Why the guard refused it.
        message: String,
    },
    /// A pre-request script threw, hit a limit or called an unsupported member, so nothing was sent.
    #[error("{} script error, {message}", phase.as_str())]
    Script {
        /// The phase whose script failed.
        phase: ScriptPhase,
        /// What went wrong, naming the script's level and line when known.
        message: String,
    },
    /// The request's oauth2 token could not be obtained, so nothing was sent.
    #[error(transparent)]
    OAuth2(OAuth2Error),
    /// A subscription's handshake was refused or its socket ended with an error.
    #[error("{message}")]
    Subscription {
        /// What ended it; the URL it names is redacted.
        message: String,
    },
}

impl RunError {
    /// The suggestion for an undefined variable, whether raised directly or by the oauth2 step.
    pub fn hint(&self) -> Option<&UndefinedHint> {
        match self {
            RunError::Resolve(e) | RunError::OAuth2(OAuth2Error::Resolve(e)) => e.hint(),
            _ => None,
        }
    }

    /// The hint rendered for the command line by [`UndefinedHint::cli_text`].
    pub fn cli_hint(&self) -> Option<String> {
        match self {
            RunError::Resolve(e) | RunError::OAuth2(OAuth2Error::Resolve(e)) => e.cli_hint(),
            _ => None,
        }
    }

    /// The hint rendered for the MCP server by [`UndefinedHint::mcp_text`].
    pub fn mcp_hint(&self) -> Option<String> {
        match self {
            RunError::Resolve(e) | RunError::OAuth2(OAuth2Error::Resolve(e)) => e.mcp_hint(),
            _ => None,
        }
    }
}

// A script's or subscription's message may quote what it read, so Debug shows its length only.
impl fmt::Debug for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Resolve(e) => f.debug_tuple("Resolve").field(e).finish(),
            RunError::Exec(e) => f.debug_tuple("Exec").field(e).finish(),
            RunError::Refused { message } => {
                f.debug_struct("Refused").field("message", message).finish()
            }
            RunError::Script { phase, message } => f
                .debug_struct("Script")
                .field("phase", phase)
                .field("message_len", &message.len())
                .finish(),
            RunError::OAuth2(e) => f.debug_tuple("OAuth2").field(e).finish(),
            RunError::Subscription { message } => f
                .debug_struct("Subscription")
                .field("message_len", &message.len())
                .finish(),
        }
    }
}

/// One request's result: its outcome, and whatever of the response and assertions it reached.
#[derive(Clone)]
#[non_exhaustive]
pub struct RequestResult {
    /// The request's item path.
    pub path: ItemPath,
    /// The request's display name.
    pub name: String,
    /// How it ended.
    pub outcome: Outcome,
    /// Why it errored or was cancelled.
    pub error: Option<RunError>,
    /// What the resolved request does not honour.
    pub warnings: Vec<Warning>,
    /// The response, when one arrived.
    pub response: Option<Response>,
    /// Each post-response variable and what became of it; empty when no response arrived.
    pub post_response_vars: Vec<VarOutcome>,
    /// Each enabled assertion and its verdict, when a response arrived.
    pub assertions: Vec<AssertionOutcome>,
    /// Each script test and script-error entry, in the order the phases recorded them.
    pub tests: Vec<TestOutcome>,
    /// What the scripts logged, capped per request.
    pub console: Vec<ConsoleEntry>,
    /// Sensitive values the scripts read, for callers that redact output.
    script_sensitive: Vec<String>,
    /// Every value the session log hides for this send, the script reads included.
    log_values: Vec<String>,
    /// The host policy's refusals of the scripts' own sends.
    script_refusals: Vec<ExecError>,
    /// The cookies the scripts wrote, with their phases, when no response arrived to carry them.
    script_cookies: Vec<(ScriptPhase, SetCookie)>,
    /// The zero-based data row it ran under; `None` when the run had no data file.
    pub iteration: Option<usize>,
    /// The reply status that failed the request under `fail_on_status`; `None` otherwise.
    pub status_failure: Option<u16>,
    /// The rule `status_failure` matched.
    pub(crate) status_rule: Option<StatusRule>,
    /// How many times the request was sent; more than one when retries were on.
    attempts: u32,
    /// Set only when a cancel came during the pause before a retry and the attempt before it
    /// failed without a response: that failure, which `error` (the cancel) would otherwise hide.
    pub last_attempt_error: Option<RunError>,
    /// What the scripts asked of the runner; empty outside a run.
    control: RunnerControl,
    /// The person declined a network change before this send, which ends the run.
    network_declined: bool,
}

// The sensitive values a script read are secrets, so Debug shows their count only.
impl fmt::Debug for RequestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestResult")
            .field("path", &self.path)
            .field("name", &self.name)
            .field("outcome", &self.outcome)
            .field("error", &self.error)
            .field("last_attempt_error", &self.last_attempt_error)
            .field("warnings", &self.warnings)
            .field("response", &self.response)
            .field("post_response_vars", &self.post_response_vars)
            .field("assertions", &self.assertions)
            .field("tests", &self.tests)
            .field("console", &self.console)
            .field("script_sensitive", &self.script_sensitive.len())
            .field("log_values", &self.log_values.len())
            .field("script_refusals", &self.script_refusals)
            .field("script_cookies", &self.script_cookies)
            .field("iteration", &self.iteration)
            .field("status_failure", &self.status_failure)
            .finish()
    }
}

/// Counts over a run's results.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Summary {
    /// Results in the report, which are in execution order; script jumps can make it differ from
    /// the plan size.
    pub requests: usize,
    /// Requests that passed.
    pub passed: usize,
    /// Passed requests whose checks were all skipped (see [`RequestResult::is_unverified`]); a
    /// subset of `passed`, not an extra outcome.
    pub unverified: usize,
    /// Passed requests with no check at all (see [`RequestResult::has_no_checks`]); a subset of
    /// `passed`, disjoint from `unverified`.
    pub no_tests: usize,
    /// Requests with a failed assertion.
    pub failed: usize,
    /// Requests that could not be resolved or sent.
    pub errored: usize,
    /// Requests never sent because the run bailed or was cancelled.
    pub skipped: usize,
    /// Requests cancelled in flight.
    pub cancelled: usize,
    /// Assertions skipped across every result.
    pub skipped_assertions: usize,
    /// Script tests that passed, script-error entries not counted.
    pub passed_tests: usize,
    /// Script tests that failed, script-error entries not counted.
    pub failed_tests: usize,
    /// Script tests skipped, script-error entries not counted.
    pub skipped_tests: usize,
}

/// What a run produced: the plan's load errors, each request's result in run order, and counts.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunReport {
    /// The plan's load errors, in the collection's order.
    pub load_errors: Vec<FileError>,
    /// One result per request run or skipped, in execution order: a request jumped over has
    /// none and a repeated one has several. Each is the allocation its
    /// `RequestFinished` event carried.
    pub results: Vec<Arc<RequestResult>>,
    /// Counts over `results`.
    pub summary: Summary,
}

impl RunReport {
    /// The distinct item paths of the failing results (see [`RequestResult::is_failure`]), in
    /// the order each first failed in `results`, which is execution order. A path that failed in
    /// several iterations or repeats appears once.
    pub fn failed_items(&self) -> Vec<ItemPath> {
        let mut items: Vec<ItemPath> = Vec::new();
        for result in self.results.iter().filter(|r| r.is_failure()) {
            if !items.contains(&result.path) {
                items.push(result.path.clone());
            }
        }
        items
    }
}

/// Progress of a run, reported as it happens.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RunEvent {
    /// The run began with `total` requests.
    Started {
        /// The number of planned requests.
        total: usize,
    },
    /// Data row `index` of `total` begins; only an iterated run reports it.
    IterationStarted {
        /// The zero-based row.
        index: usize,
        /// The number of rows.
        total: usize,
    },
    /// The request at `index` is about to be resolved and sent.
    RequestStarted {
        /// Its position in run order within the current iteration.
        index: usize,
        /// Its item path.
        path: ItemPath,
    },
    /// The request at `index` will be sent again after `delay`; reported just before the pause,
    /// once per retry, so N attempts report N-1 times.
    Retrying {
        /// Its position in run order within the current iteration.
        index: usize,
        /// Its item path.
        path: ItemPath,
        /// The attempt about to start, counting the first as 1.
        attempt: u32,
        /// How many attempts there may be: the retries plus the first send.
        of: u32,
        /// The pause before that attempt.
        delay: Duration,
    },
    /// The final response's headers arrived; only a streaming run reports it.
    ResponseStarted {
        /// The request's position in run order within the current iteration.
        index: usize,
        /// The status code.
        status: u16,
        /// The response headers, lowercase names.
        headers: Vec<(String, String)>,
        /// Whether the body is a Server-Sent Events stream.
        event_stream: bool,
    },
    /// The next piece of the response body.
    ResponseChunk {
        /// The request's position in run order within the current iteration.
        index: usize,
        /// The bytes received.
        bytes: Vec<u8>,
    },
    /// Bytes of the final body received so far; only a run saving to a file reports it.
    ResponseProgress {
        /// The request's position in run order within the current iteration.
        index: usize,
        /// All bytes received, including those past the body cap.
        received: u64,
        /// The expected size when the server declared one for an unencoded body.
        total: Option<u64>,
    },
    /// The body read ended, before the request's post-response work begins.
    ResponseEnded {
        /// The request's position in run order within the current iteration.
        index: usize,
    },
    /// The request at `index` ended, whether sent or skipped.
    RequestFinished {
        /// Its position in run order within the current iteration.
        index: usize,
        /// How it ended; the report holds the same allocation.
        result: Arc<RequestResult>,
    },
    /// Every request has ended.
    Finished {
        /// Counts over the results.
        summary: Summary,
    },
}

/// The most jumps scripts may make in one run before it ends.
const MAX_JUMPS: usize = 10_000;

/// Where a run goes after a request.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Continue at this plan index; one past the end finishes the run.
    To(usize),
    /// End the run, with a warning for the request that caused it.
    End(Option<Warning>),
}

/// The next step after the request at `current`, given what its scripts asked. A stop wins
/// over a jump; a jump goes to the first request of that name, backwards included.
fn step(names: &[String], current: usize, control: &RunnerControl, jumps: &mut usize) -> Step {
    if control.stop {
        return Step::End(None);
    }
    let Some(next) = &control.next else {
        return Step::To(current + 1);
    };
    let Some(name) = next else {
        return Step::End(None);
    };
    let Some(target) = names.iter().position(|n| n == name) else {
        return Step::End(Some(Warning::NextRequestNotFound));
    };
    if *jumps >= MAX_JUMPS {
        return Step::End(Some(Warning::NextRequestLoop));
    }
    *jumps += 1;
    Step::To(target)
}

/// Run `plan` in order on `exec`, once per data row when the options carry rows, reporting
/// progress to `on_event`, and return every result. The future owns its inputs, so it can be spawned.
pub async fn run(
    plan: RunPlan,
    ctx: VarContext,
    exec: Arc<Executor>,
    opts: RunOptions,
    cancel: CancelHandle,
    mut on_event: Box<dyn FnMut(RunEvent) + Send>,
) -> RunReport {
    on_event(RunEvent::Started {
        total: plan.requests.len(),
    });
    let names: Vec<String> = plan
        .requests
        .iter()
        .map(|(_, r, _)| r.name().to_owned())
        .collect();
    let rows = iteration_total(opts.iterations.as_ref(), opts.iteration_count);
    let mut walk = Walk {
        results: Vec::with_capacity(names.len() * rows),
        stopped: false,
        jumps: 0,
        sent_any: false,
    };
    if opts.iterations.is_none() && opts.iteration_count.is_none() {
        // The single pass is iteration 0 of 1 for variables, but carries no iteration index.
        // A standalone send is no run, so it leaves the iteration variables undefined.
        let row;
        let ctx = if opts.mode == ExecutionMode::Standalone {
            ctx
        } else {
            row = ctx.for_iteration(None, 0, rows);
            row
        };
        walk.pass(
            &plan,
            &names,
            &ctx,
            None,
            &exec,
            &opts,
            &cancel,
            &mut *on_event,
        )
        .await;
    } else {
        // A bail, a cancel or a script stop in one iteration ends every later one too.
        for iteration in 0..rows {
            if opts
                .iterations
                .as_ref()
                .is_some_and(|d| d.row(iteration).is_none())
            {
                break;
            }
            if walk.stopped || cancel.is_cancelled() {
                break;
            }
            on_event(RunEvent::IterationStarted {
                index: iteration,
                total: rows,
            });
            let ctx = ctx.for_iteration(opts.iterations.as_ref(), iteration, rows);
            walk.pass(
                &plan,
                &names,
                &ctx,
                Some(iteration),
                &exec,
                &opts,
                &cancel,
                &mut *on_event,
            )
            .await;
        }
    }
    let results = walk.results;
    let summary = summarize(&results);
    on_event(RunEvent::Finished {
        summary: summary.clone(),
    });
    RunReport {
        load_errors: plan.load_errors,
        results,
        summary,
    }
}

/// What carries over from one iteration's walk of the plan to the next.
struct Walk {
    results: Vec<Arc<RequestResult>>,
    /// A bail or a script ended the run.
    stopped: bool,
    /// Script jumps made so far, across every iteration.
    jumps: usize,
    /// Whether any request has been sent, so the delay never precedes the first.
    sent_any: bool,
}

impl Walk {
    /// One walk of the plan under `ctx`, marking each result with `iteration`.
    #[allow(clippy::too_many_arguments)]
    async fn pass(
        &mut self,
        plan: &RunPlan,
        names: &[String],
        ctx: &VarContext,
        iteration: Option<usize>,
        exec: &Arc<Executor>,
        opts: &RunOptions,
        cancel: &CancelHandle,
        on_event: &mut (dyn FnMut(RunEvent) + Send),
    ) {
        let mut index = 0;
        while let Some((path, request, _)) = plan.requests.get(index) {
            let mut next = index + 1;
            // The pause watches the handle, so a cancel during it ends the run at once.
            if !self.stopped && !cancel.is_cancelled() && self.sent_any && !opts.delay.is_zero() {
                cancel.run(tokio::time::sleep(opts.delay)).await;
            }
            // A bail, a cancel or a script that ended the run turns every later request into a
            // skipped result that is never resolved, sent or announced as started.
            let mut result = if self.stopped || cancel.is_cancelled() {
                RequestResult::new(path.clone(), request, Outcome::Skipped)
            } else {
                on_event(RunEvent::RequestStarted {
                    index,
                    path: path.clone(),
                });
                self.sent_any = true;
                let mut result = if opts.stream.is_some() {
                    send_streaming_one(
                        index,
                        path.clone(),
                        request,
                        ctx,
                        exec,
                        opts,
                        cancel,
                        on_event,
                    )
                    .await
                } else {
                    send_retrying_one(
                        index,
                        path.clone(),
                        request,
                        ctx,
                        exec,
                        opts,
                        cancel,
                        on_event,
                    )
                    .await
                };
                self.stopped = result.outcome == Outcome::Cancelled
                    || result.network_declined
                    || opts.bail && matches!(result.outcome, Outcome::Failed | Outcome::Errored);
                if !self.stopped {
                    match step(names, index, &result.control, &mut self.jumps) {
                        Step::To(to) => next = to,
                        Step::End(warning) => {
                            result.warnings.extend(warning);
                            self.stopped = true;
                        }
                    }
                }
                result
            };
            result.iteration = iteration;
            let result = Arc::new(result);
            on_event(RunEvent::RequestFinished {
                index,
                result: Arc::clone(&result),
            });
            self.results.push(result);
            index = next;
        }
    }
}

/// What the body-read sink and the end hook push to the walk.
enum Wire {
    Event(StreamEvent),
    End,
    Retry(u32, u32, Duration),
}

/// [`send_one`] reporting each pause before a retry as [`RunEvent::Retrying`], ahead of the
/// request's own finish.
#[allow(clippy::too_many_arguments)]
async fn send_retrying_one(
    index: usize,
    path: ItemPath,
    request: &RequestFile,
    ctx: &VarContext,
    exec: &Arc<Executor>,
    opts: &RunOptions,
    cancel: &CancelHandle,
    on_event: &mut (dyn FnMut(RunEvent) + Send),
) -> RequestResult {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let opts = opts.with_retry_hook(move |attempt, of, delay| {
        let _ = tx.send((attempt, of, delay));
    });
    let mut announce = |(attempt, of, delay)| {
        on_event(RunEvent::Retrying {
            index,
            path: path.clone(),
            attempt,
            of,
            delay,
        })
    };
    let fut = send_one(path.clone(), request, ctx, exec, &opts, cancel);
    tokio::pin!(fut);
    let result = loop {
        tokio::select! {
            biased;
            Some(retry) = rx.recv() => announce(retry),
            r = &mut fut => break r,
        }
    };
    while let Ok(retry) = rx.try_recv() {
        announce(retry);
    }
    result
}

/// [`send_one`] with a sink attached: response events are forwarded as they arrive, and all of
/// them precede the return, so they come before the request's own finish.
#[allow(clippy::too_many_arguments)]
async fn send_streaming_one(
    index: usize,
    path: ItemPath,
    request: &RequestFile,
    ctx: &VarContext,
    exec: &Arc<Executor>,
    opts: &RunOptions,
    cancel: &CancelHandle,
    on_event: &mut (dyn FnMut(RunEvent) + Send),
) -> RequestResult {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let end_tx = tx.clone();
    let retry_tx = tx.clone();
    let mut opts = opts.with_stream_sink(
        move |e| {
            let _ = tx.send(Wire::Event(e));
        },
        Arc::new(move || {
            let _ = end_tx.send(Wire::End);
        }),
    );
    opts.on_retry = Some(Arc::new(move |attempt, of, delay| {
        let _ = retry_tx.send(Wire::Retry(attempt, of, delay));
    }));
    // The resolver picks this request's file on the clone that carries the sink.
    if let Some(file) = opts.body_files.as_ref().and_then(|pick| pick(index, &path))
        && let Some(s) = opts.stream.take()
    {
        opts.stream = Some(s.body_file(file));
    }
    // The end marker counts only after a start, so a send that failed before headers has none.
    let mut started = false;
    let mut forward = |wire: Wire| match wire {
        Wire::Event(StreamEvent::Started {
            status,
            headers,
            event_stream,
        }) => {
            started = true;
            on_event(RunEvent::ResponseStarted {
                index,
                status,
                headers,
                event_stream,
            });
        }
        Wire::Event(StreamEvent::Chunk(bytes)) => {
            on_event(RunEvent::ResponseChunk { index, bytes })
        }
        Wire::Event(StreamEvent::Progress { received, total }) => {
            on_event(RunEvent::ResponseProgress {
                index,
                received,
                total,
            })
        }
        Wire::Retry(attempt, of, delay) => on_event(RunEvent::Retrying {
            index,
            path: path.clone(),
            attempt,
            of,
            delay,
        }),
        Wire::End => {
            if started {
                on_event(RunEvent::ResponseEnded { index });
            }
        }
    };
    let fut = send_one(path.clone(), request, ctx, exec, &opts, cancel);
    tokio::pin!(fut);
    let result = loop {
        tokio::select! {
            biased;
            Some(wire) = rx.recv() => forward(wire),
            r = &mut fut => break r,
        }
    };
    // The sender lives in the options until the send returns, so what is queued is all there is.
    while let Ok(wire) = rx.try_recv() {
        forward(wire);
    }
    result
}

/// Resolve, send and evaluate one request; a resolve failure, an oauth2 token failure, a guard
/// refusal or a send failure makes it `Errored`, except a cancel, which makes it `Cancelled`.
/// With scripts on, the pre-request phase runs before the token fetch and interpolation, and
/// the post-response and tests phases around the assertions; with scripts off, each non-empty
/// tests script is recorded as skipped once a response arrives.
pub async fn send_one(
    path: ItemPath,
    request: &RequestFile,
    ctx: &VarContext,
    exec: &Arc<Executor>,
    opts: &RunOptions,
    cancel: &CancelHandle,
) -> RequestResult {
    send_at(path, request, ctx, exec, opts, cancel, Nesting::top(opts)).await
}

/// The deepest `bru.runRequest` nesting below a top-level send.
pub(crate) const MAX_NESTING: usize = 16;

/// What a send's scripts need to run another request through the same exchange.
#[derive(Clone)]
pub(crate) struct Nesting {
    depth: usize,
    opts: Arc<RunOptions>,
    levels: Arc<[tokio::sync::Mutex<()>]>,
    /// The deadline of the calling phase, which this send's phases may not outlive.
    deadline: Option<Instant>,
    /// The top-level send's hold, staging every log item of the send and its nested sends.
    hold: Option<SendHold>,
}

impl Nesting {
    /// The handle of a top-level send: depth 0, the send's options minus the run-only fields.
    fn top(opts: &RunOptions) -> Nesting {
        let opts = RunOptions {
            bail: false,
            guard: opts.guard.clone(),
            scripts: opts.scripts.clone(),
            mode: opts.mode,
            authorize: opts.authorize.clone(),
            delay: Duration::ZERO,
            iterations: None,
            iteration_count: None,
            stream: None,
            ended: None,
            body_files: None,
            retry: Retry::default(),
            on_retry: None,
            fail_on_status: None,
        };
        Nesting {
            depth: 0,
            opts: Arc::new(opts),
            levels: (0..=MAX_NESTING)
                .map(|_| tokio::sync::Mutex::new(()))
                .collect(),
            deadline: None,
            hold: None,
        }
    }

    /// The deadline this send's phases inherit from the phase that nested it, if any.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// The hold this send's log items are staged in, when a log is attached.
    pub(crate) fn hold(&self) -> Option<&SendHold> {
        self.hold.as_ref()
    }
}

/// Runs the request at `rel` from a script of the send `state` belongs to, one level deeper,
/// on that send's context, executor and cancel handle, with no phase outliving `deadline`.
/// `Err` is the `{message}` text.
pub(crate) async fn run_nested(
    state: &HostState,
    rel: &str,
    deadline: Option<Instant>,
) -> Result<RequestResult, String> {
    let nesting = &state.nesting;
    let depth = nesting.depth + 1;
    if depth > MAX_NESTING {
        return Err(format!(
            "bru.runRequest: maximum nesting depth ({MAX_NESTING}) exceeded"
        ));
    }
    let (path, request) = nested_target(state.ctx.root(), rel)?;
    // Calls made from one level queue on the next level's lock, so one runtime per level lives.
    let _level = nesting.levels[depth].lock().await;
    let child = Nesting {
        depth,
        deadline,
        ..nesting.clone()
    };
    let opts = Arc::clone(&child.opts);
    // A caller phase that ends drops this future, and the guard then stops the nested run.
    let (cancel, _stop) = state.cancel.child_until_dropped();
    Ok(send_at(
        path,
        &request,
        &state.ctx,
        &state.exec,
        &opts,
        &cancel,
        child,
    )
    .await)
}

/// The item path and file of the request a script names: `.yml` is added unless present, and
/// any path that is not a loadable request under `root` is refused as written.
fn nested_target(root: &Path, rel: &str) -> Result<(ItemPath, RequestFile), String> {
    let invalid = || format!("bru.runRequest: invalid request path - {rel}");
    let file = if rel.ends_with(".yml") {
        rel.to_owned()
    } else {
        format!("{rel}.yml")
    };
    ItemPath::new(&file).ok_or_else(invalid)?;
    // Both sides are canonicalised so a symlink cannot leave the root; the item path is then
    // taken from the canonical file, which a missing file never reaches.
    let (Ok(canonical_root), Ok(canonical)) =
        (root.canonicalize(), root.join(&file).canonicalize())
    else {
        return Err(invalid());
    };
    let inner = canonical
        .strip_prefix(&canonical_root)
        .map_err(|_| invalid())?;
    let segments: Option<Vec<&str>> = inner.components().map(|c| c.as_os_str().to_str()).collect();
    let path = segments
        .and_then(|s| ItemPath::new(&s.join("/")))
        .ok_or_else(invalid)?;
    let request = RequestFile::load(&canonical).map_err(|_| invalid())?;
    Ok((path, request))
}

/// [`send_one`] at the nesting level `nesting` describes.
async fn send_at(
    path: ItemPath,
    request: &RequestFile,
    ctx: &VarContext,
    exec: &Arc<Executor>,
    opts: &RunOptions,
    cancel: &CancelHandle,
    mut nesting: Nesting,
) -> RequestResult {
    // A top-level send opens the hold its nested sends share; the guard publishes it when this
    // frame ends, so a dropped send still publishes what it staged.
    let mut _guard = None;
    if nesting.depth == 0
        && nesting.hold.is_none()
        && let Some(log) = exec.session_log()
    {
        let (hold, guard) = SendHold::open(log.clone());
        nesting.hold = Some(hold);
        _guard = Some(guard);
    }
    let hold = nesting.hold.clone();
    let mut result = RequestResult::new(path, request, Outcome::Errored);
    // The record collects every phase's entries, so whichever step ends the send, they are kept.
    let mut record = Record::default();
    exchange(
        &mut result,
        &mut record,
        request,
        ctx,
        exec,
        opts,
        cancel,
        nesting,
    )
    .await;
    // Expressions read after the last phase, so the shared scope is tainted once more here.
    if let Some(state) = &record.state {
        ctx.runtime_vars().taint(&lock_sensitive(state));
        let refused = state.refused.lock().unwrap_or_else(PoisonError::into_inner);
        result.script_refusals = refused.clone();
        // The scripts' cookies join the response they were set around; with no response
        // the result keeps them itself.
        let mut cookies = state.cookies.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = std::mem::take(&mut *cookies);
        match result.response.take() {
            Some(r) => result.response = Some(r.with_script_cookies(rows)),
            None => result.script_cookies = rows,
        }
    }
    // Runner control means something only inside a run.
    if let Some(state) = record
        .state
        .take()
        .filter(|_| opts.mode != ExecutionMode::Standalone)
    {
        let control = state.control.lock().unwrap_or_else(PoisonError::into_inner);
        result.control = control.clone();
    }
    result.console = record.console();
    result.tests = record.tests;
    result.script_sensitive = record.sensitive;
    // The result keeps what the session log hides, so a shell printing script text redacts
    // it as the console dock does.
    result.log_values = log_values(request, ctx);
    result.log_values.extend(record.substituted);
    if let Some(response) = &result.response {
        result
            .log_values
            .extend_from_slice(response.derived_credentials());
    }
    result
        .log_values
        .extend_from_slice(&result.script_sensitive);
    // Console lines are collected at the send's end, so they all carry one time. The values
    // scripts read join the hold before its guard drops, so every staged item loses them too.
    if let Some(hold) = &hold {
        let at = SystemTime::now();
        let r = Redactor::new(&result.log_values);
        for line in &result.console {
            let line = ConsoleLine::capture(
                at,
                &result.name,
                Some(line.phase),
                line.level,
                &line.text,
                &r,
            );
            hold.stage(Logged::Console(Arc::new(line)));
        }
        hold.learn(result.script_sensitive_values());
    }
    result
}

/// The steps of [`send_one`], leaving the outcome, error, response and assertions in `result`.
#[allow(clippy::too_many_arguments)]
async fn exchange(
    result: &mut RequestResult,
    record: &mut Record,
    request: &RequestFile,
    ctx: &VarContext,
    exec: &Arc<Executor>,
    opts: &RunOptions,
    cancel: &CancelHandle,
    nesting: Nesting,
) {
    let prepared = match prepare(request, ctx) {
        Ok(prepared) => prepared,
        Err(e) => {
            result.error = Some(RunError::Resolve(e));
            return;
        }
    };
    // An unanswered prompt stops the request before any script state is built or script runs.
    let labels = missing_labels(request, ctx);
    if !labels.is_empty() {
        result.error = Some(RunError::Resolve(ResolveError::PromptRequired { labels }));
        return;
    }
    let written_host = prepared.written_host();
    // The script host takes `nesting`, so the hold and source are read first.
    let hold = nesting.hold().cloned();
    let source = if nesting.depth > 0 {
        SendSource::Script
    } else if opts.mode == ExecutionMode::Standalone {
        SendSource::Request
    } else {
        SendSource::Runner
    };
    // The pre-request phase edits the prepared templates, which are then interpolated as edited.
    let state = opts.scripts.is_enabled().then(|| {
        Arc::new(HostState::new(
            ctx,
            request,
            prepared.clone(),
            opts.mode,
            Arc::clone(exec),
            opts.guard.clone(),
            cancel.clone(),
            nesting,
        ))
    });
    record.state.clone_from(&state);
    let prepared = match &state {
        None => prepared,
        Some(state) => {
            match record
                .run(state, ScriptPhase::PreRequest, &opts.scripts, cancel)
                .await
            {
                PhaseEnd::Ran => {}
                PhaseEnd::Failed(message) => {
                    let phase = ScriptPhase::PreRequest;
                    result.error = Some(RunError::Script { phase, message });
                    return;
                }
                PhaseEnd::Cancelled => return cancelled(result),
            }
            let edited = state
                .prepared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            edited.clone()
        }
    };
    // A pre-request skip ends the request before the token fetch and drops any jump it set.
    if let Some(state) = &state
        && opts.mode != ExecutionMode::Standalone
    {
        let mut control = state.control.lock().unwrap_or_else(PoisonError::into_inner);
        if control.skip {
            control.next = None;
            result.outcome = Outcome::Skipped;
            return;
        }
    }
    // The token is fetched here, so its settings see whatever the pre-request phase set.
    // A token fetch joins this send's hold, so its entry publishes with the send's.
    let token_tag = SendTag {
        hold: hold.clone(),
        ..SendTag::default()
    };
    match exec
        .prepare_auth_as(request, ctx, &opts.authorize, cancel, &token_tag)
        .await
    {
        Ok(()) => {}
        Err(OAuth2Error::Cancelled) => return cancelled(result),
        Err(OAuth2Error::Exec(e @ ExecError::NetworkDeclined)) => return declined(result, &e),
        Err(e) => {
            // The token step resolves the request's oauth2 fields itself, so its undefined
            // variables get their hint here, where the request and context are at hand.
            let e = match e {
                OAuth2Error::Resolve(inner) => {
                    OAuth2Error::Resolve(inner.with_hint(|name| ctx.undefined_hint(request, name)))
                }
                other => other,
            };
            result.error = Some(RunError::OAuth2(e));
            return;
        }
    }
    let mut resolved = match resolve_prepared(prepared, ctx) {
        Ok(resolved) => resolved,
        Err(e) => {
            result.error = Some(RunError::Resolve(e));
            return;
        }
    };
    // What the scripts read joins the request's secrets before the guard and the executor judge it.
    // Later phases read a GraphQL object body as it was sent.
    if let Some(state) = &state {
        resolved.add_sensitive_values(&lock_sensitive(state), &written_host);
        state
            .prepared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .settle_graphql_body(resolved.body());
    }
    // What was substituted, OS environment values included, hides in the result as in the entry.
    record.substituted = resolved.sensitive_values().to_vec();
    result.warnings = resolved.warnings().to_vec();
    if state.is_some() {
        result.warnings.retain(|w| *w != Warning::ScriptsNotRun);
    }
    if let Some(message) = opts.guard.as_ref().and_then(|guard| guard(&resolved)) {
        result.error = Some(RunError::Refused { message });
        return;
    }
    // The entry hides the request's secret candidates and what the scripts read so far.
    let mut extra = Vec::new();
    if exec.session_log().is_some() {
        extra = log_values(request, ctx);
        if let Some(state) = &state {
            extra.extend(lock_sensitive(state));
        }
    }
    let tag = SendTag {
        source,
        extra,
        hold,
        stream: opts.stream.clone(),
        ..SendTag::default()
    };
    // PAC fallbacks join the warnings before the result is judged, so a failed send keeps them.
    let done = send_with_retries(
        exec,
        &resolved,
        cancel,
        &tag,
        opts.retry,
        opts.on_retry.as_ref(),
    )
    .await;
    result.attempts = done.attempts;
    result
        .warnings
        .extend(done.pac.into_iter().map(Warning::ProxyAutoConfig));
    if let Ok(response) = &done.sent {
        result.warnings.extend(response.warnings().iter().cloned());
    }
    // A cancel in the pause before a retry keeps the attempt before it: its response, or its
    // failure in `last_attempt_error`. Nothing after the response runs.
    if done.cancelled {
        result.outcome = Outcome::Cancelled;
        result.error = Some(RunError::Exec(ExecError::Cancelled));
        match done.sent {
            Ok(response) => result.response = Some(response),
            Err(e) => result.last_attempt_error = Some(RunError::Exec(e)),
        }
        return;
    }
    let response = match done.sent {
        Ok(response) => {
            // The body read is over; post-response work starts after this marker.
            if let Some(ended) = &opts.ended {
                ended();
            }
            response
        }
        Err(e @ ExecError::NetworkDeclined) => return declined(result, &e),
        Err(e) => {
            if matches!(e, ExecError::Cancelled) {
                result.outcome = Outcome::Cancelled;
            }
            result.error = Some(RunError::Exec(e));
            return;
        }
    };
    // Capture before asserting so assertions see the new values through the context. With
    // scripts on, expressions read `res`, so the response is set first.
    let scripted = state.as_ref().map(|state| {
        let _ = state.response.set(response.clone());
        Expressions::new(state, &opts.scripts)
    });
    result.post_response_vars =
        apply_post_response_vars_with(request, &response, ctx, scripted.as_ref());
    // Kept in the result before the later phases, so a cancel there still reports it.
    result.response = Some(response);
    match &state {
        Some(state) => {
            let phase = ScriptPhase::PostResponse;
            if let PhaseEnd::Cancelled = record.run(state, phase, &opts.scripts, cancel).await {
                return cancelled(result);
            }
        }
        None => {
            record.tests = segments(request, ctx, ScriptPhase::Tests)
                .into_iter()
                .map(|s| TestOutcome::skipped(ScriptPhase::Tests, s.name(), "scripts are off"))
                .collect();
        }
    }
    // Assertions read the body a post-response script set; the result keeps the original.
    let effective = state.as_ref().and_then(|s| s.edited_response());
    if let Some(response) = effective.as_ref().or(result.response.as_ref()) {
        result.assertions = match &scripted {
            Some(scripted) => evaluate_scripted(request, response, ctx, scripted),
            None => evaluate(request, response, ctx),
        };
        // A later phase without scripts records nothing, so the secrets in scope are kept here.
        if let Some(state) = &state {
            record.sensitive = lock_sensitive(state);
        }
    }
    if let Some(state) = &state {
        let _ = state.assertions.set(result.assertions.clone());
    }
    if let Some(state) = &state
        && let PhaseEnd::Cancelled = record
            .run(state, ScriptPhase::Tests, &opts.scripts, cancel)
            .await
    {
        return cancelled(result);
    }
    let failed = result
        .assertions
        .iter()
        .any(|a| matches!(a.verdict(), Verdict::Fail { .. }))
        || record
            .tests
            .iter()
            .any(|t| matches!(t.verdict, Verdict::Fail { .. }));
    // Only the final reply is judged, so a retried 503 that ended in 200 is not a failure.
    if let (Some(rule), Some(response)) = (opts.fail_on_status, &result.response)
        && rule.matches(response.status())
    {
        result.status_failure = Some(response.status());
        result.status_rule = Some(rule);
    }
    result.outcome = if failed || result.status_failure.is_some() {
        Outcome::Failed
    } else {
        Outcome::Passed
    };
    // A subscription that ended in error errors the request, after its transcript was judged.
    if is_subscription(&resolved)
        && let Some(StreamEnd::Error(message)) =
            result.response.as_ref().and_then(Response::stream_end)
    {
        result.error = Some(RunError::Subscription {
            message: message.clone(),
        });
        result.outcome = Outcome::Errored;
    }
}

/// Marks `result` cancelled because the person declined a network change, which ends the run.
/// The error keeps the reason, so reports say why nothing was sent.
fn declined(result: &mut RequestResult, e: &ExecError) {
    result.outcome = Outcome::Cancelled;
    result.error = Some(RunError::Exec(e.clone()));
    result.network_declined = true;
}

/// The sensitive values the scripts and expressions of a send have read so far.
fn lock_sensitive(state: &HostState) -> Vec<String> {
    let sensitive = state.sensitive.lock();
    sensitive.unwrap_or_else(PoisonError::into_inner).clone()
}

/// Marks a send cancelled by a script phase, as a cancelled send is marked.
fn cancelled(result: &mut RequestResult) {
    result.outcome = Outcome::Cancelled;
    result.error = Some(RunError::Exec(ExecError::Cancelled));
}

/// Counts each outcome and every skipped assertion across `results`.
fn summarize(results: &[Arc<RequestResult>]) -> Summary {
    let mut summary = Summary {
        requests: results.len(),
        ..Summary::default()
    };
    for result in results {
        match result.outcome {
            Outcome::Passed => {
                summary.passed += 1;
                summary.unverified += usize::from(result.is_unverified());
                summary.no_tests += usize::from(result.has_no_checks());
            }
            Outcome::Failed => summary.failed += 1,
            Outcome::Errored => summary.errored += 1,
            Outcome::Skipped => summary.skipped += 1,
            Outcome::Cancelled => summary.cancelled += 1,
        }
        summary.skipped_assertions += result
            .assertions
            .iter()
            .filter(|a| matches!(a.verdict(), Verdict::Skipped { .. }))
            .count();
        for test in result.tests.iter().filter(|t| !t.script_error) {
            match test.verdict {
                Verdict::Pass => summary.passed_tests += 1,
                Verdict::Fail { .. } => summary.failed_tests += 1,
                Verdict::Skipped { .. } => summary.skipped_tests += 1,
            }
        }
    }
    summary
}

impl RequestResult {
    /// The sentence naming the reply status that failed the request, when `fail_on_status` did.
    pub fn status_failure_reason(&self) -> Option<String> {
        let code = self.status_failure?;
        let rule = self.status_rule.map_or("error", StatusRule::as_str);
        Some(format!(
            "status {code} is in the fail-on-status range ({rule})"
        ))
    }

    /// How many times the request was sent: one unless retries were on and the send was retried.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether the request failed in a way worth re-running: it errored, an assertion or a
    /// script test failed (script-error entries are not tests, but they fail the request, so
    /// its outcome is `Failed`). A pass with only skipped checks is not a failure, and a request
    /// that was never sent or was cancelled is not one either.
    pub fn is_failure(&self) -> bool {
        matches!(self.outcome, Outcome::Errored | Outcome::Failed)
            || self
                .assertions
                .iter()
                .any(|a| matches!(a.verdict(), Verdict::Fail { .. }))
            || self
                .tests
                .iter()
                .any(|t| !t.script_error && matches!(t.verdict, Verdict::Fail { .. }))
    }

    /// Whether no check reached a verdict: no assertion and no script test has a `Pass` or
    /// `Fail` verdict (none at all, or every one `Skipped`); script-error entries are not tests.
    pub fn is_untested(&self) -> bool {
        let decided = |v: &Verdict| matches!(v, Verdict::Pass | Verdict::Fail { .. });
        !self.assertions.iter().any(|a| decided(a.verdict()))
            && !self
                .tests
                .iter()
                .any(|t| !t.script_error && decided(&t.verdict))
    }

    /// Whether the request passed only because every check was skipped: at least one assertion
    /// or script test exists, none reached a verdict. A passed request with no checks at all is
    /// not unverified.
    pub fn is_unverified(&self) -> bool {
        let has_checks = !self.assertions.is_empty() || self.tests.iter().any(|t| !t.script_error);
        self.outcome == Outcome::Passed && has_checks && self.is_untested()
    }

    /// Whether the request passed with no check at all: no assertion and no script test
    /// (script-error entries are not tests). Disjoint from [`Self::is_unverified`].
    pub fn has_no_checks(&self) -> bool {
        self.outcome == Outcome::Passed
            && self.assertions.is_empty()
            && self.tests.iter().all(|t| t.script_error)
    }

    /// A result rebuilt from kept parts: no error and no warnings.
    pub fn from_parts(
        path: ItemPath,
        name: impl Into<String>,
        outcome: Outcome,
        response: Option<Response>,
        assertions: Vec<AssertionOutcome>,
    ) -> RequestResult {
        RequestResult {
            path,
            name: name.into(),
            outcome,
            error: None,
            warnings: Vec::new(),
            response,
            post_response_vars: Vec::new(),
            assertions,
            tests: Vec::new(),
            console: Vec::new(),
            script_sensitive: Vec::new(),
            log_values: Vec::new(),
            script_refusals: Vec::new(),
            script_cookies: Vec::new(),
            iteration: None,
            status_failure: None,
            status_rule: None,
            attempts: 1,
            last_attempt_error: None,
            control: RunnerControl::default(),
            network_declined: false,
        }
    }

    /// A result for `request` that has sent nothing, ending as `outcome`.
    fn new(path: ItemPath, request: &RequestFile, outcome: Outcome) -> RequestResult {
        RequestResult {
            path,
            name: request.name().to_owned(),
            outcome,
            error: None,
            warnings: Vec::new(),
            response: None,
            post_response_vars: Vec::new(),
            assertions: Vec::new(),
            tests: Vec::new(),
            console: Vec::new(),
            script_sensitive: Vec::new(),
            log_values: Vec::new(),
            script_refusals: Vec::new(),
            script_cookies: Vec::new(),
            iteration: None,
            status_failure: None,
            status_rule: None,
            attempts: 1,
            last_attempt_error: None,
            control: RunnerControl::default(),
            network_declined: false,
        }
    }

    /// Every cookie set for this request, each with where it came from, in the order it was
    /// set: the response's rows (see [`Response::set_cookies`]), or with no response the
    /// rows of the cookies its scripts wrote.
    pub fn set_cookies(&self) -> impl Iterator<Item = (CookieOrigin<'_>, &SetCookie)> {
        let own = self
            .script_cookies
            .iter()
            .map(|(phase, c)| (CookieOrigin::Script(*phase), c));
        (self.response.iter().flat_map(Response::set_cookies)).chain(own)
    }

    /// The sensitive values the scripts read or substituted, for redacting reports and logs.
    pub fn script_sensitive_values(&self) -> &[String] {
        &self.script_sensitive
    }

    /// The values the executor computed for this send (signatures, nonces, tokens); empty when
    /// the send produced no response.
    pub fn derived_credentials(&self) -> &[String] {
        self.response
            .as_ref()
            .map_or(&[], Response::derived_credentials)
    }

    /// `text` with every value the session log hides for this send replaced by `[redacted]`,
    /// for a caller that prints script-derived text such as a test name or an error.
    pub fn redact(&self, text: &str) -> String {
        Redactor::new(&self.log_values).redact(text)
    }

    /// A header value as the session log shows it: replaced whole for a credential header,
    /// else redacted like [`RequestResult::redact`].
    pub fn redact_header(&self, name: &str, value: &str) -> String {
        Redactor::new(&self.log_values).redact_header(name, value)
    }

    /// A URL as the session log shows it: the whole userinfo dropped, then redacted.
    pub fn redact_url(&self, url: &str) -> String {
        Redactor::new(&self.log_values).redact_url(url)
    }

    /// The console lines with their text redacted as the session log redacts them.
    pub fn redacted_console(&self) -> Vec<ConsoleEntry> {
        let r = Redactor::new(&self.log_values);
        (self.console.iter())
            .map(|c| ConsoleEntry::new(c.phase, c.level, r.redact(&c.text)))
            .collect()
    }

    /// The host policy's refusals of the scripts' own sends, once per host, so a caller can
    /// ask the user to trust those hosts.
    pub fn script_refusals(&self) -> &[ExecError] {
        &self.script_refusals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::RunnerControl;

    fn plan_with(load_errors: Vec<FileError>) -> RunPlan {
        RunPlan {
            requests: Vec::new(),
            load_errors,
        }
    }

    #[test]
    fn an_empty_plan_explains_itself_unless_load_errors_speak() {
        let none = TagFilter::default();
        let nope = TagFilter::new(vec!["nope".into()], vec![]);
        assert_eq!(
            plan_with(vec![]).empty_message(&nope).as_deref(),
            Some("no requests matched: tags nope")
        );
        assert_eq!(
            plan_with(vec![]).empty_message(&none).as_deref(),
            Some("no requests to run in the selected path")
        );
        let broken = plan_with(vec![FileError::new(Path::new("a.yml"), "bad")]);
        assert_eq!(broken.empty_message(&nope), None);
    }

    #[test]
    fn steps_follow_runner_control() {
        let names: Vec<String> = ["A", "B", "A"].map(String::from).to_vec();
        let to = |n: Option<&str>| RunnerControl {
            next: Some(n.map(String::from)),
            ..RunnerControl::default()
        };
        let mut jumps = 0;
        assert_eq!(
            step(&names, 0, &RunnerControl::default(), &mut jumps),
            Step::To(1)
        );
        assert_eq!(step(&names, 2, &to(Some("A")), &mut jumps), Step::To(0));
        assert_eq!(step(&names, 0, &to(None), &mut jumps), Step::End(None));
        assert_eq!(
            step(&names, 0, &to(Some("Z")), &mut jumps),
            Step::End(Some(Warning::NextRequestNotFound))
        );
        let stop = RunnerControl {
            stop: true,
            next: Some(Some("B".into())),
            ..RunnerControl::default()
        };
        assert_eq!(step(&names, 0, &stop, &mut jumps), Step::End(None));
        let mut many = MAX_JUMPS;
        assert_eq!(
            step(&names, 0, &to(Some("B")), &mut many),
            Step::End(Some(Warning::NextRequestLoop))
        );
    }
}
