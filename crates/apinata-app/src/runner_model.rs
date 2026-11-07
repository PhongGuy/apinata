//! Running a folder or a whole collection from a runner tab: plan it on the collection
//! actor, run it on the tokio bridge with the shared executor, and track each row.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use apinata_core::{
    CancelHandle, ItemPath, IterationData, RequestFile, RequestResult, ResultsReport, RunEvent,
    RunOptions, StatusRule, StreamLimits, Summary, TagFilter, VarContext,
};
use gpui_kit::{App, AsyncApp, Context, Entity, Task, WeakEntity};

use crate::command::{Command, Planned};
use crate::executor;
use crate::last_response::{self, SendExtra};
use crate::prompt_vars::prompt_answers;
use crate::script_gate::{Prepared, prepare};
use crate::send_model::{extend_unique, plan_message, secret_values};
use crate::session::SavedRunnerOptions;
use crate::settings;
use crate::tokio_bridge;
use crate::workspace::Workspace;

/// Where one planned request stands.
#[derive(Debug, Clone)]
pub enum RowState {
    /// Not started yet.
    Queued,
    /// Being resolved and sent.
    Running,
    /// Ended, whatever its outcome.
    Done(Arc<RequestResult>),
}

/// One planned request of a run.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunRow {
    /// The request's item path.
    pub path: ItemPath,
    /// The request's name.
    pub name: String,
    /// The request's HTTP method.
    pub method: String,
    /// Where it stands.
    pub state: RowState,
    /// The zero-based data row it runs under; `None` in a run without a data file.
    pub iteration: Option<usize>,
}

/// One request an idle runner will send, as the plan lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRow {
    /// The request's item path.
    pub path: ItemPath,
    /// The request's name.
    pub name: String,
    /// The request's HTTP method.
    pub method: String,
}

/// Where a run stands as a whole.
#[derive(Debug, Clone)]
pub enum RunState {
    /// Nothing started yet.
    Idle,
    /// Waiting for the collection actor's plan.
    Planning,
    /// The plan is running.
    Running,
    /// Every request ended; a cancelled run ends here too.
    Finished(Summary),
    /// Cancelled before the plan arrived.
    Cancelled,
    /// No run happened: a plan error, a closed collection or no HTTP client.
    Failed(String),
}

/// The failure shown when the collection never answers the plan.
pub const PLAN_UNANSWERED: &str = "the collection closed before the run was planned";

/// What a runner runs: one folder (or the whole collection at the root) or a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTarget {
    /// A folder, or `ItemPath::root()` for the whole collection.
    Scope(ItemPath),
    /// A selection of requests and folders, in the order given.
    Items(Vec<ItemPath>),
}

/// A runner tab's run of one folder, or of the whole collection at `ItemPath::root()`.
pub struct RunnerModel {
    workspace: Entity<Workspace>,
    root: PathBuf,
    target: RunTarget,
    bail: bool,
    delay_ms: u64,
    retries: u32,
    retry_delay_ms: u64,
    retry_unsafe: bool,
    fail_on_status: Option<StatusRule>,
    iteration_count: Option<usize>,
    tags: String,
    exclude_tags: String,
    data_file: Option<PathBuf>,
    state: RunState,
    rows: Vec<RunRow>,
    plan_len: usize,
    iteration: usize,
    load_errors: Vec<String>,
    // The same errors as (relative path, message), the shape the exported report carries.
    load_error_rows: Vec<(String, String)>,
    unsaved: Vec<String>,
    cancel: Option<CancelHandle>,
    // A prompt-variable or trust dialog is open for this run.
    waiting: bool,
    task: Option<Task<()>>,
    // The saved plan with each request's effective tags, unfiltered, and that filtered by
    // the current tags; refiltered on every tag edit so typing never waits on the actor.
    planned: Vec<(PlannedRow, Vec<String>)>,
    preview: Vec<PlannedRow>,
    preview_loaded: bool,
    preview_rounds: Option<usize>,
    preview_task: Option<Task<()>>,
    // The parsed tags, exclude tags and data file the shown rows ran with.
    ran_with: RunChoice,
    // What a finished send is scrubbed with before it is kept: each planned request's file
    // by path, the run's context, and the secrets every planned request substitutes.
    run_files: HashMap<ItemPath, RequestFile>,
    run_context: Option<VarContext>,
    run_secrets: Vec<String>,
    // The row (as an index into `rows`) waiting out the pause before a retry, with the attempt
    // about to start and the most there may be.
    retrying: Option<(usize, u32, u32)>,
}

// The plan narrowed to the requests the runner's checkboxes leave in, in plan order.
fn choose_checked(
    plan: apinata_core::RunPlan,
    unchecked: &HashSet<ItemPath>,
) -> Result<apinata_core::RunPlan, apinata_core::SelectError> {
    let chosen: Vec<ItemPath> = (plan.items())
        .filter(|item| !unchecked.contains(*item))
        .cloned()
        .collect();
    plan.choose(&chosen)
}

type PlanAnswer = async_channel::Receiver<Result<Planned, crate::command::PlanError>>;

type RunChoice = (
    Vec<String>,
    Vec<String>,
    Option<PathBuf>,
    u64,
    bool,
    Option<usize>,
    (u32, u64, bool),
    Option<StatusRule>,
);

impl RunnerModel {
    /// An idle runner for `target` in the collection at `root`.
    pub fn new(workspace: Entity<Workspace>, root: PathBuf, target: ItemPath) -> Self {
        Self::with_target(workspace, root, RunTarget::Scope(target))
    }

    /// An idle runner for a selection of items in the collection at `root`.
    pub fn for_items(workspace: Entity<Workspace>, root: PathBuf, items: Vec<ItemPath>) -> Self {
        Self::with_target(workspace, root, RunTarget::Items(items))
    }

    fn with_target(workspace: Entity<Workspace>, root: PathBuf, target: RunTarget) -> Self {
        RunnerModel {
            workspace,
            root,
            target,
            bail: false,
            delay_ms: 0,
            retries: 0,
            retry_delay_ms: 0,
            retry_unsafe: false,
            fail_on_status: None,
            iteration_count: None,
            tags: String::new(),
            exclude_tags: String::new(),
            data_file: None,
            state: RunState::Idle,
            rows: Vec::new(),
            plan_len: 0,
            iteration: 0,
            load_errors: Vec::new(),
            load_error_rows: Vec::new(),
            unsaved: Vec::new(),
            cancel: None,
            waiting: false,
            task: None,
            planned: Vec::new(),
            preview: Vec::new(),
            preview_loaded: false,
            preview_rounds: None,
            preview_task: None,
            ran_with: Default::default(),
            run_files: HashMap::new(),
            run_context: None,
            run_secrets: Vec::new(),
            retrying: None,
        }
    }

    /// The collection root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The selected environment of the runner's collection.
    pub fn selected_environment(&self, cx: &App) -> Option<String> {
        self.workspace.read(cx).selected_environment(&self.root, cx)
    }

    /// What this runner runs.
    pub fn target(&self) -> &RunTarget {
        &self.target
    }

    /// Whether the next run stops after its first failed or errored request.
    pub fn bail(&self) -> bool {
        self.bail
    }

    /// Sets bail for the next run.
    pub fn set_bail(&mut self, bail: bool, cx: &mut Context<Self>) {
        self.bail = bail;
        self.save_options(cx);
        cx.notify();
    }

    /// The pause between requests of the next run, in milliseconds.
    pub fn delay_ms(&self) -> u64 {
        self.delay_ms
    }

    /// Sets the pause between requests for the next run.
    pub fn set_delay_ms(&mut self, ms: u64, cx: &mut Context<Self>) {
        self.delay_ms = ms;
        self.save_options(cx);
        cx.notify();
    }

    /// The extra attempts the next run gives a request that fails in transit; 0 turns retries off.
    pub fn retries(&self) -> u32 {
        self.retries
    }

    /// Sets the retries of the next run.
    pub fn set_retries(&mut self, retries: u32, cx: &mut Context<Self>) {
        self.retries = retries;
        self.save_options(cx);
        cx.notify();
    }

    /// The pause before each retry of the next run, in milliseconds.
    pub fn retry_delay_ms(&self) -> u64 {
        self.retry_delay_ms
    }

    /// Sets the pause before each retry for the next run.
    pub fn set_retry_delay_ms(&mut self, ms: u64, cx: &mut Context<Self>) {
        self.retry_delay_ms = ms;
        self.save_options(cx);
        cx.notify();
    }

    /// Whether the next run also retries POST and PATCH.
    pub fn retry_unsafe(&self) -> bool {
        self.retry_unsafe
    }

    /// Sets whether the next run also retries POST and PATCH.
    pub fn set_retry_unsafe(&mut self, on: bool, cx: &mut Context<Self>) {
        self.retry_unsafe = on;
        self.save_options(cx);
        cx.notify();
    }

    /// The reply statuses that fail a request in the next run; `None`, the default, judges none.
    pub fn fail_on_status(&self) -> Option<StatusRule> {
        self.fail_on_status
    }

    /// Sets which reply statuses fail a request in the next run.
    pub fn set_fail_on_status(&mut self, rule: Option<StatusRule>, cx: &mut Context<Self>) {
        self.fail_on_status = rule;
        self.save_options(cx);
        cx.notify();
    }

    /// The iteration count of the next run; `None`, the default, adds no limit.
    pub fn iteration_count(&self) -> Option<usize> {
        self.iteration_count
    }

    /// Sets the iteration count of the next run, `None` for no limit; zero becomes one.
    pub fn set_iteration_count(&mut self, n: Option<usize>, cx: &mut Context<Self>) {
        self.iteration_count = n.map(|n| n.max(1));
        self.save_options(cx);
        cx.notify();
    }

    /// The comma-separated tags the next run keeps, as typed.
    pub fn tags(&self) -> &str {
        &self.tags
    }

    /// Sets the tags the next run keeps.
    pub fn set_tags(&mut self, text: String, cx: &mut Context<Self>) {
        self.tags = text;
        self.refilter();
        cx.notify();
    }

    /// The comma-separated tags the next run skips, as typed.
    pub fn exclude_tags(&self) -> &str {
        &self.exclude_tags
    }

    /// Sets the tags the next run skips.
    pub fn set_exclude_tags(&mut self, text: String, cx: &mut Context<Self>) {
        self.exclude_tags = text;
        self.refilter();
        cx.notify();
    }

    /// The CSV or JSON file whose rows the next run iterates over, if any.
    pub fn data_file(&self) -> Option<&Path> {
        self.data_file.as_deref()
    }

    /// Whether the data file is set but no longer exists.
    pub fn data_file_missing(&self) -> bool {
        self.data_file.as_deref().is_some_and(|p| !p.exists())
    }

    // The session key of this runner's options; a selection has none and is never remembered.
    fn saved_key(&self) -> Option<String> {
        match &self.target {
            RunTarget::Scope(item) => Some(item.as_str().to_owned()),
            RunTarget::Items(_) => None,
        }
    }

    /// Applies the options saved for this runner. Restoring writes nothing back, and a
    /// data file that has gone missing is kept so the view can say so.
    pub fn restore_options(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.saved_key() else { return };
        let saved = {
            let session = self.workspace.read(cx).session().read(cx).get();
            session
                .collections
                .get(&self.root)
                .and_then(|c| c.runners.get(&key))
                .cloned()
        };
        let Some(saved) = saved else { return };
        self.bail = saved.bail;
        self.delay_ms = saved.delay_ms;
        self.retries = saved.retries;
        self.retry_delay_ms = saved.retry_delay_ms;
        self.retry_unsafe = saved.retry_unsafe;
        self.fail_on_status = saved.fail_on_status;
        self.iteration_count = saved.iteration_count.map(|n| n.max(1));
        self.data_file = saved.data_file;
        self.preview_rounds = (self.data_file.as_deref())
            .and_then(|p| read_data(p).ok())
            .map(|d| d.len());
        cx.notify();
    }

    // Remembers the current options in the session; all-default options drop the entry.
    fn save_options(&self, cx: &mut Context<Self>) {
        let Some(key) = self.saved_key() else { return };
        if self.root.to_str().is_none() {
            return;
        }
        let options = SavedRunnerOptions {
            bail: self.bail,
            delay_ms: self.delay_ms,
            retries: self.retries,
            retry_delay_ms: self.retry_delay_ms,
            retry_unsafe: self.retry_unsafe,
            fail_on_status: self.fail_on_status,
            iteration_count: self.iteration_count,
            // The session file stores paths as strings, so a path that is not UTF-8 stays out.
            data_file: self.data_file.clone().filter(|p| p.to_str().is_some()),
        };
        let session = self.workspace.read(cx).session().clone();
        let stored = session
            .read(cx)
            .get()
            .collections
            .get(&self.root)
            .and_then(|c| c.runners.get(&key));
        let wanted = (options != SavedRunnerOptions::default()).then_some(&options);
        if stored == wanted {
            return;
        }
        let root = self.root.clone();
        session.update(cx, |p, cx| {
            p.update(cx, |s| match wanted {
                Some(o) => {
                    s.collections
                        .entry(root)
                        .or_default()
                        .runners
                        .insert(key, o.clone());
                }
                None => {
                    if let Some(c) = s.collections.get_mut(&root) {
                        c.runners.remove(&key);
                    }
                }
            })
        });
    }

    /// Sets or clears the next run's data file.
    pub fn set_data_file(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.data_file = path;
        // An unreadable file shows no round count here and fails at Run as before.
        self.preview_rounds = (self.data_file.as_deref())
            .and_then(|p| read_data(p).ok())
            .map(|d| d.len());
        self.save_options(cx);
        cx.notify();
    }

    /// Asks the actor for the saved plan (`Command::PlanSaved`) and keeps it for the preview.
    pub fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let (sent, answer) = self.send_plan(cx);
        if sent.is_none() {
            self.preview_task = None;
            return self.keep_plan(Vec::new(), cx);
        }
        // Replacing the task drops, and so cancels, an older refresh.
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let planned = match answer.recv().await {
                Ok(Ok(Planned { plan, .. })) => (plan.items())
                    .zip(plan.requests())
                    .zip(plan.tags())
                    .map(|((path, request), tags)| {
                        let row = PlannedRow {
                            path: path.clone(),
                            name: request.name().to_owned(),
                            method: request.method().to_owned(),
                        };
                        (row, tags.to_vec())
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let _ = this.update(cx, |m, cx| m.keep_plan(planned, cx));
        }));
    }

    // Asks the actor for the saved plan of the target; `None` means the collection is gone.
    fn send_plan(&self, cx: &mut Context<Self>) -> (Option<u64>, PlanAnswer) {
        let (reply, answer) = async_channel::bounded(1);
        let command = match &self.target {
            RunTarget::Scope(item) => Command::PlanSaved {
                item: item.clone(),
                reply,
            },
            RunTarget::Items(items) => Command::PlanSavedItems {
                items: items.clone(),
                reply,
            },
        };
        let sent = self
            .workspace
            .update(cx, |w, cx| w.send(&self.root, command, cx));
        (sent, answer)
    }

    /// The kept plan under the current tags and exclude tags, in run order.
    pub fn preview(&self) -> &[PlannedRow] {
        &self.preview
    }

    /// The data rows each request runs for, when a readable data file is set.
    pub fn preview_rounds(&self) -> Option<usize> {
        self.preview_rounds
    }

    /// Whether the saved plan has arrived at least once, so the preview is not just empty.
    pub fn preview_loaded(&self) -> bool {
        self.preview_loaded
    }

    /// Whether the next run includes request `item`; the choice is shared by every runner of
    /// the collection.
    pub fn is_checked(&self, item: &ItemPath, cx: &App) -> bool {
        self.workspace.read(cx).run_checked(&self.root, item)
    }

    /// Includes `items` in, or leaves them out of, the next run of this collection's runners.
    pub fn set_checked(&mut self, items: &[ItemPath], on: bool, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.workspace
            .update(cx, |w, _| w.set_run_checked(&root, items, on));
        cx.notify();
    }

    /// The collection's folder names as the sidebar shows them, by item path.
    pub fn folder_names(&self, cx: &App) -> HashMap<String, String> {
        let Some(snapshot) = self.workspace.read(cx).snapshot(&self.root, cx) else {
            return HashMap::new();
        };
        (snapshot.content.rows.iter())
            .filter(|row| row.kind == crate::snapshot::RowKind::Folder)
            .map(|row| (row.path.as_str().to_owned(), row.name.clone()))
            .collect()
    }

    /// How many of the previewed requests are checked.
    pub fn checked_count(&self, cx: &App) -> usize {
        let unchecked = self.workspace.read(cx).run_unchecked(&self.root);
        (self.preview.iter())
            .filter(|row| !unchecked.contains(&row.path))
            .count()
    }

    fn keep_plan(&mut self, planned: Vec<(PlannedRow, Vec<String>)>, cx: &mut Context<Self>) {
        self.planned = planned;
        self.preview_loaded = true;
        self.refilter();
        cx.notify();
    }

    fn refilter(&mut self) {
        let filter = TagFilter::new(
            TagFilter::parse_list(&self.tags),
            TagFilter::parse_list(&self.exclude_tags),
        );
        self.preview = (self.planned.iter())
            .filter(|(_, tags)| filter.matches(tags))
            .map(|(row, _)| row.clone())
            .collect();
    }

    /// Whether the shown rows ran with other tags, exclude tags, data file, delay, bail or retries
    /// than are set now.
    pub fn results_stale(&self) -> bool {
        !self.rows.is_empty() && self.ran_with != self.choice()
    }

    fn choice(&self) -> RunChoice {
        let tags = TagFilter::parse_list(&self.tags);
        let exclude = TagFilter::parse_list(&self.exclude_tags);
        (
            tags,
            exclude,
            self.data_file.clone(),
            self.delay_ms,
            self.bail,
            self.iteration_count,
            (self.retries, self.retry_delay_ms, self.retry_unsafe),
            self.fail_on_status,
        )
    }

    /// The finished run as the report the CLI would write for it, or `None` until a run has
    /// finished. A request the run never reached has no result and is not in it.
    pub fn export_report(&self) -> Option<ResultsReport> {
        if !matches!(self.state, RunState::Finished(_)) {
            return None;
        }
        let results = self.rows.iter().filter_map(|row| match &row.state {
            RowState::Done(result) => Some(result.as_ref()),
            _ => None,
        });
        Some(ResultsReport::new(self.load_error_rows.clone(), results))
    }

    /// Whether the shown run stopped at its first failure.
    pub fn ran_with_bail(&self) -> bool {
        self.ran_with.4
    }

    /// The run's state.
    pub fn state(&self) -> &RunState {
        &self.state
    }

    /// The attempt about to start and the most there may be, for row `ix` while it waits out
    /// the pause before a retry; `None` otherwise.
    pub fn retrying(&self, ix: usize) -> Option<(u32, u32)> {
        self.retrying
            .filter(|(row, ..)| *row == ix)
            .map(|(_, attempt, of)| (attempt, of))
    }

    /// The planned requests in run order.
    pub fn rows(&self) -> &[RunRow] {
        &self.rows
    }

    /// The plan's load errors as "{path relative to root}: {message}".
    pub fn load_errors(&self) -> &[String] {
        &self.load_errors
    }

    /// Names of the requests in the run's scope that had unsaved edits when Run was pressed,
    /// in path order; the run sends their saved files instead.
    pub fn unsaved(&self) -> &[String] {
        &self.unsaved
    }

    /// Whether a run is planning or running.
    pub fn is_running(&self) -> bool {
        matches!(self.state, RunState::Planning | RunState::Running)
    }

    /// Whether a prompt-variable or trust dialog is open for this run.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// The distinct request paths that failed in the finished run (see
    /// [`RequestResult::is_failure`]), in row order; empty until a run has finished.
    pub fn failed_items(&self) -> Vec<ItemPath> {
        if !matches!(self.state, RunState::Finished(_)) {
            return Vec::new();
        }
        let mut items: Vec<ItemPath> = Vec::new();
        for row in &self.rows {
            if let RowState::Done(result) = &row.state
                && result.is_failure()
                && !items.contains(&row.path)
            {
                items.push(row.path.clone());
            }
        }
        items
    }

    /// Starts a fresh run, clearing the last one's rows. Returns `false`, doing nothing,
    /// while a run is under way; failures to start become `Failed` states.
    /// With every previewed request unchecked there is nothing to run, and it does nothing.
    pub fn start(&mut self, cx: &mut Context<Self>) -> bool {
        if self.preview_loaded && !self.preview.is_empty() && self.checked_count(cx) == 0 {
            return false;
        }
        self.start_with(None, cx)
    }

    /// Runs again only the requests that failed in the finished run, with the same options
    /// and every data row for those requests. Returns `false`, doing nothing, while a run is
    /// under way or when nothing failed.
    pub fn rerun_failed(&mut self, cx: &mut Context<Self>) -> bool {
        let failed = self.failed_items();
        if self.is_running() || failed.is_empty() {
            return false;
        }
        self.start_with(Some(failed), cx)
    }

    // `only` narrows this one run to those items through core's plan selection; `start`
    // passes `None`, so a normal run is always the full plan.
    fn start_with(&mut self, only: Option<Vec<ItemPath>>, cx: &mut Context<Self>) -> bool {
        if self.is_running() {
            return false;
        }
        self.rows.clear();
        self.ran_with = self.choice();
        self.load_errors.clear();
        self.load_error_rows.clear();
        (self.plan_len, self.iteration) = (0, 0);
        self.unsaved = self.unsaved_names(cx);
        let data = match self.data_file.as_deref().map(read_data).transpose() {
            Ok(data) => data,
            Err(why) => {
                self.finish(RunState::Failed(why), cx);
                return true;
            }
        };
        let filter = TagFilter::new(
            TagFilter::parse_list(&self.tags),
            TagFilter::parse_list(&self.exclude_tags),
        );
        let Some(exec) = executor::shared(cx) else {
            self.finish(RunState::Failed("the HTTP client is not ready".into()), cx);
            return true;
        };
        let unchecked = self.workspace.read(cx).run_unchecked(&self.root);
        let (sent, answer) = self.send_plan(cx);
        if sent.is_none() {
            self.finish(RunState::Failed(PLAN_UNANSWERED.into()), cx);
            return true;
        }
        let cancel = CancelHandle::new();
        self.cancel = Some(cancel.clone());
        self.state = RunState::Planning;
        cx.notify();
        let (root, bail, delay) = (self.root.clone(), self.bail, self.delay_ms);
        let (retries, retry_delay, retry_unsafe) =
            (self.retries, self.retry_delay_ms, self.retry_unsafe);
        let count = self.iteration_count;
        let fail_on_status = self.fail_on_status;
        self.task = Some(cx.spawn(async move |this, cx| {
            // A reply channel that closes unanswered means the actor is gone.
            let planned = match answer.recv().await {
                Ok(Ok(planned)) => planned,
                Ok(Err(e)) => {
                    let failed = RunState::Failed(plan_message(&e));
                    return drop(this.update(cx, |m, cx| m.finish(failed, cx)));
                }
                Err(_) => {
                    let failed = RunState::Failed(PLAN_UNANSWERED.into());
                    return drop(this.update(cx, |m, cx| m.finish(failed, cx)));
                }
            };
            // Filtered first, so the script question and the rows see only the kept requests.
            let Planned { plan, context } = planned;
            let plan = plan.filter_tags(&filter);
            let plan = match choose_checked(plan, &unchecked) {
                Ok(plan) => plan,
                Err(e) => {
                    let failed = RunState::Failed(e.to_string());
                    return drop(this.update(cx, |m, cx| m.finish(failed, cx)));
                }
            };
            let mut planned = Planned {
                plan: match &only {
                    Some(items) => plan.retain_items(items),
                    None => plan,
                },
                context,
            };
            // One question covers every kept request: the union of their prompts in plan
            // order. A cancel here, by button or by `cancel()`, starts nothing.
            let mut labels: Vec<String> = Vec::new();
            for request in planned.plan.requests() {
                for label in apinata_core::prompt_labels(request, &planned.context) {
                    if !labels.contains(&label) {
                        labels.push(label);
                    }
                }
            }
            if !labels.is_empty() {
                let memory = this
                    .update(cx, |m, cx| m.workspace.read(cx).prompt_memory(&root, cx))
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let _ = this.update(cx, |m, cx| m.wait(true, cx));
                let Some(answers) = prompt_answers(labels, memory, cx).await else {
                    return drop(this.update(cx, |m, cx| m.finish(RunState::Cancelled, cx)));
                };
                let _ = this.update(cx, |m, cx| m.wait(false, cx));
                planned.context = planned.context.with_prompt_values(answers);
            }
            // Requests that inherit their timeout take the Preferences default.
            if let Ok(context) = this.update(cx, |_, cx| {
                settings::with_request_timeout(&planned.context, cx)
            }) {
                planned.context = context;
            }
            let scripts = match prepare(root, &planned, cx) {
                Prepared::Ready(scripts) => scripts,
                Prepared::Ask(question) => {
                    let _ = this.update(cx, |m, cx| m.wait(true, cx));
                    question.ask(cx).await
                }
            };
            let _ = this.update(cx, |m, cx| {
                m.waiting = false;
                m.rows = planned
                    .plan
                    .items()
                    .zip(planned.plan.requests())
                    .map(|(path, request)| RunRow {
                        path: path.clone(),
                        name: request.name().to_owned(),
                        method: request.method().to_owned(),
                        state: RowState::Queued,
                        iteration: (data.is_some() || count.is_some()).then_some(0),
                    })
                    .collect();
                m.plan_len = m.rows.len();
                m.run_files = (planned.plan.items().cloned())
                    .zip(planned.plan.requests().cloned())
                    .collect();
                m.run_secrets = Vec::new();
                for file in m.run_files.values() {
                    extend_unique(&mut m.run_secrets, secret_values(file, &planned.context));
                }
                m.run_context = Some(planned.context.clone());
                m.load_errors = planned
                    .plan
                    .load_errors()
                    .iter()
                    .map(|e| {
                        let path = e.path().strip_prefix(&m.root).unwrap_or(e.path());
                        format!("{}: {}", path.display(), e.message())
                    })
                    .collect();
                m.load_error_rows = (planned.plan.load_errors().iter())
                    .map(|e| {
                        let rel = e.path().strip_prefix(&m.root).unwrap_or(e.path());
                        // Slash-joined like the CLI, so the report reads the same on every platform.
                        let path = (rel.components())
                            .map(|c| c.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/");
                        (path, e.message().to_owned())
                    })
                    .collect();
                m.state = RunState::Running;
                cx.notify();
            });
            let mut opts = RunOptions::new()
                .bail(bail)
                .scripts(scripts)
                .delay(Duration::from_millis(delay))
                .retries(retries)
                .retry_delay(Duration::from_millis(retry_delay))
                .retry_unsafe(retry_unsafe)
                .fail_on_status(fail_on_status)
                .stream(
                    StreamLimits::new()
                        .max_duration(Some(Duration::from_secs(30)))
                        .max_events(Some(10_000)),
                );
            if let Some(data) = data {
                opts = opts.iterations(data);
            }
            if let Some(count) = count {
                opts = opts.iteration_count(count);
            }
            let (tx, rx) = async_channel::unbounded::<RunEvent>();
            let on_event = Box::new(move |event: RunEvent| drop(tx.try_send(event)));
            let run = cx.update(|cx| {
                let run =
                    apinata_core::run(planned.plan, planned.context, exec, opts, cancel, on_event);
                tokio_bridge::spawn(cx, run)
            });
            drain(this, rx, cx).await;
            drop(run);
        }));
        true
    }

    /// Cancels the run. While planning it is dropped at once and shows `Cancelled`; while
    /// running the remaining requests are cancelled and the run ends with its summary.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        match self.state {
            RunState::Planning => {
                self.task = None;
                self.finish(RunState::Cancelled, cx);
            }
            RunState::Running => {
                if let Some(cancel) = &self.cancel {
                    cancel.cancel();
                }
            }
            _ => {}
        }
    }

    // The dirty requests at or under any target item, drafts included; a folder prefix ends
    // at a `/` so `users` never claims `users2/`.
    fn unsaved_names(&self, cx: &App) -> Vec<String> {
        let Some(snapshot) = self.workspace.read(cx).snapshot(&self.root, cx) else {
            return Vec::new();
        };
        let items: Vec<&ItemPath> = match &self.target {
            RunTarget::Scope(item) => vec![item],
            RunTarget::Items(items) => items.iter().collect(),
        };
        let covers = |p: &ItemPath| {
            items.iter().any(|t| {
                t.is_root() || p == *t || p.as_str().starts_with(&format!("{}/", t.as_str()))
            })
        };
        let mut dirty: Vec<_> = (snapshot.content.requests.iter())
            .filter(|(p, r)| r.dirty && covers(p))
            .collect();
        dirty.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        dirty.into_iter().map(|(_, r)| r.name.clone()).collect()
    }

    // Never drops `self.task`: the task itself calls this.
    fn finish(&mut self, state: RunState, cx: &mut Context<Self>) {
        self.state = state;
        self.retrying = None;
        self.cancel = None;
        self.waiting = false;
        cx.notify();
    }

    fn wait(&mut self, waiting: bool, cx: &mut Context<Self>) {
        self.waiting = waiting;
        cx.notify();
    }

    // Keeps a finished send as a run. The context's clones share the token cache, so a
    // lookup after the send also sees a token the run fetched or refreshed.
    fn record(&self, result: Arc<RequestResult>, cx: &App) {
        let mut secrets = self.run_secrets.clone();
        if let (Some(file), Some(ctx)) = (self.run_files.get(&result.path), &self.run_context) {
            extend_unique(&mut secrets, secret_values(file, ctx));
        }
        let extra = SendExtra {
            request: None,
            origin: Some(last_response::RUN_ORIGIN.into()),
        };
        let item = result.path.clone();
        let _ = last_response::record(self.root.clone(), item, result, secrets, extra, cx);
    }

    /// Folds one event of the run into the rows; the run's own channel calls it.
    pub fn apply(&mut self, event: RunEvent, cx: &mut Context<Self>) {
        // Each later iteration gets its own queued copy of the plan's rows, so its events
        // never overwrite the rows of an earlier one.
        let at = |index: usize, m: &Self| index + m.iteration * m.plan_len;
        match event {
            RunEvent::Retrying {
                index, attempt, of, ..
            } => self.retrying = Some((at(index, self), attempt, of)),
            RunEvent::IterationStarted { index, .. } => {
                self.retrying = None;
                self.iteration = index;
                if index > 0 {
                    let fresh: Vec<RunRow> = (self.rows[..self.plan_len].iter())
                        .map(|row| RunRow {
                            state: RowState::Queued,
                            iteration: Some(index),
                            ..row.clone()
                        })
                        .collect();
                    self.rows.extend(fresh);
                }
            }
            RunEvent::RequestStarted { index, .. } => {
                self.retrying = None;
                let ix = at(index, self);
                if let Some(row) = self.rows.get_mut(ix) {
                    row.state = RowState::Running;
                }
            }
            RunEvent::RequestFinished { index, result } => {
                self.retrying = None;
                let ix = at(index, self);
                if result.response.is_some() {
                    self.record(Arc::clone(&result), cx);
                }
                if let Some(row) = self.rows.get_mut(ix) {
                    row.state = RowState::Done(result);
                }
            }
            RunEvent::Finished { summary } => return self.finish(RunState::Finished(summary), cx),
            _ => {}
        }
        cx.notify();
    }
}

// Reads and parses a data file: JSON when its extension is `.json` in any case, else CSV.
fn read_data(path: &Path) -> Result<IterationData, String> {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let fail = |why: String| format!("cannot read data file {name}: {why}");
    let text = std::fs::read_to_string(path).map_err(|e| fail(e.to_string()))?;
    let json = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    let parsed = if json {
        IterationData::from_json(&text)
    } else {
        IterationData::from_csv(&text)
    };
    parsed.map_err(|e| fail(e.to_string()))
}

// The sender drops when the run ends, closing the channel; a run that dies before its
// Finished event must not leave the model Running.
async fn drain(
    this: WeakEntity<RunnerModel>,
    rx: async_channel::Receiver<RunEvent>,
    cx: &mut AsyncApp,
) {
    while let Ok(event) = rx.recv().await {
        let _ = this.update(cx, |m, cx| m.apply(event, cx));
    }
    let _ = this.update(cx, |m, cx| {
        if m.is_running() {
            m.finish(RunState::Failed("the run ended unexpectedly".into()), cx);
        }
    });
}
