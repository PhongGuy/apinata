//! A runner tab: bail, delay, tag filter, data file and start controls over a folder's or a
//! collection's run, a preview of the plan until a run has rows, then one row per planned
//! request as a table row (method, name, status, time, result) with a failed row's reason,
//! grouped under a header per data row, and a summary counted from the rows. A finished row
//! opens its request's tab showing that row's result.

use crate::ui::LockButton as _;
use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::{ItemPath, Outcome, RequestResult, RunError, StatusRule, Verdict};
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, Div, Entity, FocusHandle, Focusable, PathPromptOptions, Rems,
    ScrollHandle, SharedString, Subscription, TestSupportExt, Window, div, rems,
};

use crate::body_text;
use crate::response_pane::{
    check_counts, describe_error, redacted_assertion_line, shows_no_tests, test_line, tests_total,
};
use crate::runner_export::{ReportFormat, export_results};
use crate::runner_model::{PlannedRow, RowState, RunRow, RunState, RunTarget, RunnerModel};
use crate::style;
use crate::ui;
use crate::workspace::Workspace;

/// Opens a request's tab showing one runner row's result; the Shell handles it.
#[derive(Clone, Action)]
#[action(namespace = apinata, no_json)]
pub struct OpenRunResult {
    /// The root of the request's collection.
    pub root: PathBuf,
    /// The request's path in its collection.
    pub path: ItemPath,
    /// The row's result.
    pub result: Arc<RequestResult>,
}

// `RequestResult` is only `Clone`, so two actions are equal when they share one result.
impl PartialEq for OpenRunResult {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.path == other.path
            && Arc::ptr_eq(&self.result, &other.result)
    }
}

impl std::fmt::Debug for OpenRunResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRunResult")
            .field("root", &self.root)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The view of one runner tab.
pub struct RunnerView {
    model: Entity<RunnerModel>,
    selected: Option<usize>,
    options: Option<OptionInputs>,
    table_scroll: ScrollHandle,
    options_scroll: ScrollHandle,
    // The tab's Tab stop: the first thing focus lands on, ahead of the controls.
    focus: FocusHandle,
    _observer: Subscription,
}

// The delay and tag inputs, made on first render since they need a window.
struct OptionInputs {
    delay: Entity<InputState>,
    iterations: Entity<InputState>,
    tags: Entity<InputState>,
    exclude_tags: Entity<InputState>,
    retries: Entity<InputState>,
    retry_delay: Entity<InputState>,
    fail_select: Entity<SelectState<Vec<SharedString>>>,
    _select_sub: Subscription,
    _subs: [Subscription; 6],
}

impl RunnerView {
    /// A runner of `target`, a folder or `ItemPath::root()`, in the collection at `root`.
    pub fn new(
        workspace: Entity<Workspace>,
        root: PathBuf,
        target: ItemPath,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = cx.new(|_| RunnerModel::new(workspace, root, target));
        model.update(cx, |m, cx| m.restore_options(cx));
        Self::with_model(model, cx)
    }

    /// A runner of a selection of items in the collection at `root`.
    pub fn for_items(
        workspace: Entity<Workspace>,
        root: PathBuf,
        items: Vec<ItemPath>,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = cx.new(|_| RunnerModel::for_items(workspace, root, items));
        Self::with_model(model, cx)
    }

    fn with_model(model: Entity<RunnerModel>, cx: &mut Context<Self>) -> Self {
        model.update(cx, |m, cx| m.refresh_preview(cx));
        let observer = cx.observe(&model, |this, _, cx| this.sync(cx));
        RunnerView {
            model,
            selected: None,
            options: None,
            table_scroll: ScrollHandle::new(),
            options_scroll: ScrollHandle::new(),
            focus: cx.focus_handle().tab_stop(true),
            _observer: observer,
        }
    }

    /// The run this tab shows.
    pub fn model(&self) -> &Entity<RunnerModel> {
        &self.model
    }

    /// The last clicked row, by run position.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The delay, tags and exclude tags inputs' placeholders, once the tab has rendered.
    pub fn option_placeholders(&self, cx: &App) -> Vec<String> {
        let Some(o) = &self.options else {
            return Vec::new();
        };
        [&o.delay, &o.tags, &o.exclude_tags, &o.iterations]
            .map(|i| i.read(cx).presentation().placeholder().to_string())
            .to_vec()
    }

    /// The delay and iterations inputs' current text, once the tab has rendered.
    pub fn number_field_texts(&self, cx: &App) -> Option<[String; 2]> {
        let o = self.options.as_ref()?;
        Some([&o.delay, &o.iterations].map(|i| i.read(cx).value().to_string()))
    }

    // Highlights the row; a finished one also opens its request showing this run's result.
    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        let model = self.model.read(cx);
        if let Some(RunRow {
            path,
            state: RowState::Done(result),
            ..
        }) = model.rows().get(ix)
        {
            let action = OpenRunResult {
                root: model.root().to_path_buf(),
                path: path.clone(),
                result: Arc::clone(result),
            };
            window.dispatch_action(action.boxed_clone(), cx);
        }
        cx.notify();
    }

    /// The retries and retry delay inputs' current text, once the tab has rendered.
    pub fn retry_field_texts(&self, cx: &App) -> Option<[String; 2]> {
        let o = self.options.as_ref()?;
        Some([&o.retries, &o.retry_delay].map(|i| i.read(cx).value().to_string()))
    }

    // Each edit goes straight to the model; a delay that is not all digits is ignored.
    // The inputs are made just above when missing, so they are always present at the end.
    #[allow(clippy::expect_used)]
    fn options(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &OptionInputs {
        if self.options.is_none() {
            let model = self.model.read(cx);
            // Placeholders show an example value; the labels beside the inputs name them.
            let texts = [
                (model.delay_ms().to_string(), "0"),
                (
                    model
                        .iteration_count()
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    "1",
                ),
                (model.tags().to_owned(), "smoke, auth"),
                (model.exclude_tags().to_owned(), "slow"),
                (model.retries().to_string(), "2"),
                (model.retry_delay_ms().to_string(), "500"),
            ];
            let [delay, iterations, tags, exclude_tags, retries, retry_delay] =
                texts.map(|(text, hint)| {
                    cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder(hint)
                            .default_value(text)
                    })
                });
            let watch = |input: &Entity<InputState>,
                         push: fn(&mut RunnerModel, String, &mut Context<RunnerModel>),
                         digits_only: bool,
                         window: &mut Window,
                         cx: &mut Context<Self>| {
                cx.subscribe_in(
                    input,
                    window,
                    move |this: &mut Self, input, event, window, cx| {
                        if matches!(event, InputEvent::Change) {
                            let text = input.read(cx).value().to_string();
                            // The number fields hold digits only; strip anything else and
                            // let the resulting change event carry the cleaned text.
                            if digits_only && !text.bytes().all(|b| b.is_ascii_digit()) {
                                let clean: String =
                                    text.chars().filter(char::is_ascii_digit).collect();
                                input.update(cx, |s, cx| s.set_value(clean, window, cx));
                                return;
                            }
                            this.model.update(cx, |m, cx| push(m, text, cx));
                        }
                    },
                )
            };
            let subs = [
                watch(
                    &delay,
                    |m, text, cx| {
                        // A cleared field means no delay; any other non-digit text is ignored.
                        if text.is_empty() {
                            m.set_delay_ms(0, cx);
                        } else if text.bytes().all(|b| b.is_ascii_digit())
                            && let Ok(ms) = text.parse()
                        {
                            m.set_delay_ms(ms, cx);
                        }
                    },
                    true,
                    window,
                    cx,
                ),
                watch(
                    &iterations,
                    |m, text, cx| {
                        // A cleared field means no limit; other non-digit text is ignored.
                        if text.is_empty() {
                            m.set_iteration_count(None, cx);
                        } else if text.bytes().all(|b| b.is_ascii_digit())
                            && let Ok(n) = text.parse()
                        {
                            m.set_iteration_count(Some(n), cx);
                        }
                    },
                    true,
                    window,
                    cx,
                ),
                watch(
                    &retries,
                    |m, text, cx| {
                        // A cleared field means no retries; other non-digit text is ignored.
                        if text.is_empty() {
                            m.set_retries(0, cx);
                        } else if text.bytes().all(|b| b.is_ascii_digit())
                            && let Ok(n) = text.parse()
                        {
                            m.set_retries(n, cx);
                        }
                    },
                    true,
                    window,
                    cx,
                ),
                watch(
                    &retry_delay,
                    |m, text, cx| {
                        if text.is_empty() {
                            m.set_retry_delay_ms(0, cx);
                        } else if text.bytes().all(|b| b.is_ascii_digit())
                            && let Ok(ms) = text.parse()
                        {
                            m.set_retry_delay_ms(ms, cx);
                        }
                    },
                    true,
                    window,
                    cx,
                ),
                watch(&tags, RunnerModel::set_tags, false, window, cx),
                watch(
                    &exclude_tags,
                    RunnerModel::set_exclude_tags,
                    false,
                    window,
                    cx,
                ),
            ];
            let labels: Vec<SharedString> = FAIL_ON_STATUS_CHOICES
                .iter()
                .map(|(label, _)| (*label).into())
                .collect();
            let current = FAIL_ON_STATUS_CHOICES
                .iter()
                .position(|(_, rule)| *rule == self.model.read(cx).fail_on_status())
                .unwrap_or(0);
            let fail_select =
                cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(current)), window, cx));
            let _select_sub = cx.subscribe_in(
                &fail_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some((_, rule)) = FAIL_ON_STATUS_CHOICES
                            .iter()
                            .find(|(l, _)| *l == label.as_ref())
                    {
                        this.model
                            .update(cx, |m, cx| m.set_fail_on_status(*rule, cx));
                    }
                },
            );
            self.options = Some(OptionInputs {
                fail_select,
                _select_sub,
                delay,
                iterations,
                tags,
                exclude_tags,
                retries,
                retry_delay,
                _subs: subs,
            });
        }
        self.options
            .as_ref()
            .expect("the option inputs are made above when missing")
    }

    // Asks for one data file; a cancelled or failed prompt leaves the current one.
    fn pick_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(path) = paths.into_iter().next() {
                    this.model
                        .update(cx, |m, cx| m.set_data_file(Some(path), cx));
                }
            });
        })
        .detach();
    }

    // Drops a selection that a new run cleared away.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let rows = self.model.read(cx).rows().len();
        if self.selected.is_some_and(|ix| ix >= rows) {
            self.selected = None;
        }
        cx.notify();
    }
}

/// The Result cell of a row waiting out the pause before a retry.
pub fn retrying_label(attempt: u32, of: u32) -> String {
    format!("Retrying, attempt {attempt} of {of}")
}

/// The Result cell of a finished row: `result_word`, then the send count when it was sent more
/// than once.
pub fn result_label(r: &RequestResult) -> String {
    match r.attempts() {
        0 | 1 => result_word(r).to_owned(),
        n => format!("{} ({n} attempts)", result_word(r)),
    }
}

// `retrying` is the model's (attempt, of) for this row while it waits out a retry pause.
fn row_label(state: &RowState, retrying: Option<(u32, u32)>) -> String {
    match (state, retrying) {
        (RowState::Queued, _) => "Queued".to_owned(),
        (RowState::Running, Some((attempt, of))) => retrying_label(attempt, of),
        (RowState::Running, None) => "Running".to_owned(),
        (RowState::Done(r), _) => result_label(r),
    }
}

/// The Result cell: "No tests" for a passed request with no checks, "Unverified" for one whose
/// checks were all skipped, else the outcome word.
pub fn result_word(r: &RequestResult) -> &'static str {
    if shows_no_tests(r) {
        "No tests"
    } else if all_skipped(r) {
        "Unverified"
    } else {
        outcome_word(r.outcome)
    }
}

// A passed request that had checks, none of which reached a verdict.
fn all_skipped(r: &RequestResult) -> bool {
    r.outcome == Outcome::Passed && r.is_untested() && !shows_no_tests(r)
}

/// A row's checks line when any check was skipped: the Tests tab's total, word for word.
pub fn row_checks(r: &RequestResult) -> Option<String> {
    (check_counts(r).skipped > 0)
        .then(|| tests_total(r))
        .flatten()
}

/// Finished rows counted by their Result cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// Passed: with a verdict, with every check skipped (`unverified`) or with no check
    /// (`no_tests`).
    pub passed: usize,
    /// Of `passed`, those whose checks were all skipped.
    pub unverified: usize,
    /// Failed.
    pub failed: usize,
    /// Errored.
    pub errored: usize,
    /// Never sent because the run bailed or was cancelled.
    pub not_sent: usize,
    /// Cancelled.
    pub cancelled: usize,
    /// Of `passed`, those with nothing checked.
    pub no_tests: usize,
    /// Assertions and script tests skipped across the finished rows.
    pub skipped_tests: usize,
}

/// Counts the finished rows by their Result cell; queued and running rows are left out.
pub fn tally(rows: &[RunRow]) -> Tally {
    let mut t = Tally::default();
    for row in rows {
        let RowState::Done(r) = &row.state else {
            continue;
        };
        t.skipped_tests += check_counts(r).skipped;
        if all_skipped(r) {
            t.unverified += 1;
        }
        if shows_no_tests(r) {
            t.no_tests += 1;
        }
        let slot = match r.outcome {
            Outcome::Passed => &mut t.passed,
            Outcome::Failed => &mut t.failed,
            Outcome::Skipped => &mut t.not_sent,
            Outcome::Cancelled => &mut t.cancelled,
            _ => &mut t.errored,
        };
        *slot += 1;
    }
    t
}

/// "{n} request(s): {p} passed ({u} unverified, {t} no tests), {f} failed" then non-zero
/// errored, not sent, cancelled, then "; {k} check(s) skipped" when any check was skipped. The
/// parenthesis lists only non-zero parts; both are already inside `passed`.
pub fn summary_line(t: &Tally) -> String {
    let n = t.passed + t.failed + t.errored + t.not_sent + t.cancelled;
    let noun = if n == 1 { "request" } else { "requests" };
    let parts: Vec<String> = [(t.unverified, "unverified"), (t.no_tests, "no tests")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect();
    let unverified = if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    };
    let mut line = format!(
        "{n} {noun}: {} passed{unverified}, {} failed",
        t.passed, t.failed
    );
    let extras = [
        (t.errored, "errored"),
        (t.not_sent, "not sent"),
        (t.cancelled, "cancelled"),
    ];
    for (count, word) in extras.into_iter().filter(|(count, _)| *count > 0) {
        line.push_str(&format!(", {count} {word}"));
    }
    // Skipped checks are not requests, so they follow apart from the request counts.
    if t.skipped_tests > 0 {
        let noun = if t.skipped_tests == 1 {
            "check"
        } else {
            "checks"
        };
        line.push_str(&format!("; {} {noun} skipped", t.skipped_tests));
    }
    line
}

/// Danger with any failed or errored; Warning with anything unverified or any check skipped;
/// Success with a pass; else Muted.
pub fn summary_tone(t: &Tally) -> ui::Tone {
    if t.failed + t.errored > 0 {
        ui::Tone::Danger
    } else if t.unverified > 0 || t.skipped_tests > 0 {
        ui::Tone::Warning
    } else if t.passed > 0 {
        ui::Tone::Success
    } else {
        ui::Tone::Muted
    }
}

// The colour of a row's Result cell: semantic for a pass or a failure, muted otherwise.
fn row_tone(state: &RowState) -> ui::Tone {
    match state {
        RowState::Done(r) if all_skipped(r) => ui::Tone::Warning,
        RowState::Done(r) if shows_no_tests(r) => ui::Tone::Muted,
        RowState::Done(r) if r.outcome == Outcome::Passed => ui::Tone::Success,
        RowState::Done(r) if matches!(r.outcome, Outcome::Failed | Outcome::Errored) => {
            ui::Tone::Danger
        }
        _ => ui::Tone::Muted,
    }
}

fn outcome_word(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Passed => "Passed",
        Outcome::Failed => "Failed",
        Outcome::Errored => "Errored",
        Outcome::Skipped => "Not sent",
        Outcome::Cancelled => "Cancelled",
        _ => "Done",
    }
}

// A run error as the response pane names it, redacted as the session log redacts it. The
// sentence quotes the error's own fields, such as a secret host.
fn error_line(e: &RunError, r: &RequestResult, env: Option<&str>) -> String {
    let view = describe_error(e, &r.redact(&e.to_string()), env);
    match view.sentence {
        Some(sentence) => r.redact(&format!("{}: {sentence}", view.title)),
        None => view.title.to_owned(),
    }
}

// Why a finished row did not pass: the run error as the response pane names it, else the
// first failing assertion, else the first failing test, else the outcome word. Script-derived
// text is redacted as the session log redacts it; the status has its own column.
fn failure_reason(r: &RequestResult, env: Option<&str>) -> Option<String> {
    if !matches!(
        r.outcome,
        Outcome::Failed | Outcome::Errored | Outcome::Cancelled
    ) {
        return None;
    }
    let detail = r
        .error
        .as_ref()
        .map(|e| error_line(e, r, env))
        .or_else(|| {
            (r.assertions.iter())
                .find(|o| matches!(o.verdict(), Verdict::Fail { .. }))
                .map(|o| redacted_assertion_line(o, r).1)
        })
        .or_else(|| {
            (r.tests.iter())
                .find(|t| t.script_error || matches!(t.verdict, Verdict::Fail { .. }))
                .map(|t| r.redact(&test_line(t).1))
        })
        .or_else(|| r.status_failure.map(|code| format!("Status {code}")))
        .unwrap_or_else(|| outcome_word(r.outcome).to_owned());
    // A cancel in the pause before a retry hides the failure of the attempt before it.
    let earlier = (r.last_attempt_error.as_ref()).map(|e| error_line(e, r, env));
    Some(match earlier {
        Some(earlier) => format!("{detail}; last attempt: {earlier}"),
        None => detail,
    })
}

/// "1 request", "3 requests", "2 requests for each of 3 data rows", or "No requests to run".
pub fn plan_count_text(requests: usize, rounds: Option<usize>) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    match (requests, rounds) {
        (0, _) => "No requests to run".to_owned(),
        (n, None) => plural(n, "request"),
        (n, Some(r)) => format!(
            "{} for each of {}",
            plural(n, "request"),
            plural(r, "data row")
        ),
    }
}

/// The Run button's text: "Run", or with the checked count once it is known.
pub fn run_label(chosen: Option<usize>) -> String {
    match chosen {
        None => "Run".to_owned(),
        Some(1) => "Run 1 request".to_owned(),
        Some(n) => format!("Run {n} requests"),
    }
}

// Each nesting level of the plan preview indents this many rems.
const PLAN_INDENT: f32 = 1.25;

/// One line of the plan preview: a folder heading or a request, at its nesting depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanEntry {
    /// A folder, by its path in the collection.
    Folder {
        /// The folder's item path.
        path: String,
        /// How many shown folders enclose it.
        depth: usize,
    },
    /// A request, by its index in the preview.
    Request {
        /// The index into the preview rows.
        ix: usize,
        /// How many shown folders enclose it.
        depth: usize,
    },
}

/// The preview rows with a heading before each folder's run, in run order. Run order keeps a
/// folder's requests together, so a heading opens wherever a row's folders differ from the
/// row before. Folders at or above `scope`, the runner's own folder, are not shown.
pub fn plan_entries(rows: &[PlannedRow], scope: &str) -> Vec<PlanEntry> {
    let mut out = Vec::new();
    let mut open: Vec<String> = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        let path = row.path.as_str();
        let chain: Vec<String> = (path.match_indices('/'))
            .map(|(at, _)| path[..at].to_owned())
            .filter(|f| scope != f && !scope.starts_with(&format!("{f}/")))
            .collect();
        let kept = open.iter().zip(&chain).take_while(|(a, b)| a == b).count();
        for (depth, folder) in chain.iter().enumerate().skip(kept) {
            out.push(PlanEntry::Folder {
                path: folder.clone(),
                depth,
            });
        }
        out.push(PlanEntry::Request {
            ix,
            depth: chain.len(),
        });
        open = chain;
    }
    out
}

// A box's accessible name; the kit's box has no mixed state to report, so the name carries it.
fn partly_named(name: &str, mixed: bool) -> String {
    if mixed {
        format!("{name}, partly checked")
    } else {
        name.to_owned()
    }
}

// The indices of the preview rows under folder `path`.
fn folder_members(rows: &[PlannedRow], path: &str) -> Vec<usize> {
    let prefix = format!("{path}/");
    (rows.iter().enumerate())
        .filter(|(_, row)| row.path.as_str().starts_with(&prefix))
        .map(|(ix, _)| ix)
        .collect()
}

/// The "Fail on status" choices, in the order the select lists them.
pub const FAIL_ON_STATUS_CHOICES: [(&str, Option<StatusRule>); 4] = [
    ("Off", None),
    ("400 and above", Some(StatusRule::Error)),
    ("4xx only", Some(StatusRule::Client)),
    ("5xx only", Some(StatusRule::Server)),
];

// A muted caption naming the input beside it.
fn option_label(id: &'static str, text: &'static str, cx: &App) -> impl IntoElement + Styled {
    div()
        .id(id)
        .test_support()
        .aria_label(text)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

const RETRIES_HINT: &str =
    "Extra attempts after a transport error, a timeout, or a 502, 503 or 504. 0 turns retries off.";
const RETRY_DELAY_HINT: &str = "Wait this long between attempts.";
const RETRY_UNSAFE_NOTE: &str =
    "A retried POST or PATCH may repeat a side effect on the server. Off by default.";
const RETRY_UNSAFE_TOOLTIP: &str = "By default only idempotent methods are retried. Turn this on only if the server tolerates duplicates.";

// An option label that explains itself on hover.
fn hinted_label(
    id: &'static str,
    text: &'static str,
    hint: &'static str,
    cx: &App,
) -> impl IntoElement + Styled {
    div()
        .id(id)
        .test_support()
        .aria_label(text)
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(hint).build(window, cx)
        })
}

// A label and its input as one unit, so a wrapping options row never splits them.
fn labelled(label: impl IntoElement, input: impl IntoElement) -> Div {
    h_flex()
        .flex_none()
        .gap_2()
        .items_center()
        .child(label)
        .child(input)
}

// A cell when the row has a value, else an empty one that keeps the columns aligned.
fn cell_or_gap(cell_el: Option<impl IntoElement>, width: Rems) -> AnyElement {
    match cell_el {
        Some(el) => el.into_any_element(),
        None => cell(width).into_any_element(),
    }
}

// A list that takes only the room the pane has left and scrolls past it, so whatever
// follows it (the summary, the dock) is never painted over.
fn shrink_scroll(
    id: &'static str,
) -> impl ParentElement + IntoElement + StatefulInteractiveElement + Styled + use<> {
    div()
        .id(id)
        .test_support()
        .w_full()
        .flex_initial()
        .min_h_0()
        .overflow_scroll()
}

// A results or plan list of `rows` rows: it shrinks long before the options do, down to a
// floor that a short list stays under, so it is never padded.
fn list_box<E: Styled>(mut el: E, rows: usize) -> E {
    let floor = (style::RUN_ROW_FLOOR.0 * (rows + 1) as f32).min(style::RUN_LIST_FLOOR.0);
    el = el.flex_initial().min_h(gpui_kit::Rems(floor));
    el.style().flex_shrink = Some(style::RUN_LIST_SHRINK);
    el
}

// A fixed-width table cell; the header and every row share these widths so columns align.
fn cell(width: Rems) -> Div {
    div()
        .w(width)
        .flex_none()
        .overflow_hidden()
        .whitespace_nowrap()
}

impl Focusable for RunnerView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RunnerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inputs = self.options(window, cx);
        let (delay, iterations, tags, exclude_tags, retries, retry_delay) = (
            inputs.delay.clone(),
            inputs.iterations.clone(),
            inputs.tags.clone(),
            inputs.exclude_tags.clone(),
            inputs.retries.clone(),
            inputs.retry_delay.clone(),
        );
        let fail_select = inputs.fail_select.clone();
        let model = self.model.read(cx);
        let data_name = model.data_file().map(|path| {
            let name = (path.file_name().unwrap_or(path.as_os_str()))
                .to_string_lossy()
                .into_owned();
            if model.data_file_missing() {
                format!("{name} (missing)")
            } else {
                name
            }
        });
        let m4 = self.model.clone();
        let field = |state: &Entity<InputState>, id: &'static str, width| {
            ui::padded_input(
                Input::new(state)
                    .id(id)
                    .w(width)
                    .bg(ui::field_fill(false, cx)),
                state,
            )
        };
        let running = model.is_running();
        let (m1, m2, m3) = (self.model.clone(), self.model.clone(), self.model.clone());
        let m5 = self.model.clone();
        // Options wrap and keep their content height; a pane too short for them and the list's
        // floor caps them, and they scroll under a visible bar.
        let options =
            h_flex()
                .id("run-options")
                .test_support()
                .w_full()
                .flex_initial()
                .min_h_0()
                .pr(style::SCROLLBAR_GUTTER)
                .overflow_y_scroll()
                .track_scroll(&self.options_scroll)
                .flex_wrap()
                .gap_2()
                .items_center()
                .child(
                    ui::checkbox("run-bail", model.bail())
                        .label("Stop after the first failure")
                        .on_click(move |on: &bool, _, cx| {
                            m1.update(cx, |m, cx| m.set_bail(*on, cx))
                        }),
                )
                .child(labelled(
                    option_label("run-delay-label", "Delay (ms)", cx),
                    field(&delay, "run-delay", style::SHORT_INPUT_WIDTH),
                ))
                .child(labelled(
                    option_label("run-tags-label", "Tags", cx),
                    field(&tags, "run-tags", style::FORM_INPUT_WIDTH),
                ))
                .child(labelled(
                    option_label("run-exclude-tags-label", "Exclude tags", cx),
                    field(&exclude_tags, "run-exclude-tags", style::FORM_INPUT_WIDTH),
                ))
                .child(labelled(
                    option_label("run-iterations-label", "Iterations", cx),
                    field(&iterations, "run-iterations", style::SHORT_INPUT_WIDTH),
                ))
                .child(labelled(
                    hinted_label("run-retries-label", "Retries", RETRIES_HINT, cx),
                    field(&retries, "run-retries", style::SHORT_INPUT_WIDTH),
                ))
                .child(labelled(
                    hinted_label(
                        "run-retry-delay-label",
                        "Retry delay (ms)",
                        RETRY_DELAY_HINT,
                        cx,
                    ),
                    field(&retry_delay, "run-retry-delay", style::SHORT_INPUT_WIDTH),
                ))
                .child(labelled(
                    option_label("run-fail-on-status-label", "Fail on status", cx),
                    Select::new(&fail_select)
                        .id("run-fail-on-status")
                        .w(style::FORM_INPUT_WIDTH),
                ))
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .child(
                            ui::checkbox("run-retry-unsafe", model.retry_unsafe())
                                .label("Also retry POST and PATCH")
                                .tooltip(RETRY_UNSAFE_TOOLTIP)
                                .on_click(move |on: &bool, _, cx| {
                                    m5.update(cx, |m, cx| m.set_retry_unsafe(*on, cx))
                                }),
                        )
                        .child(
                            div()
                                .id("run-retry-unsafe-note")
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .whitespace_normal()
                                .aria_label(RETRY_UNSAFE_NOTE)
                                .text_color(cx.theme().muted_foreground)
                                .child(RETRY_UNSAFE_NOTE),
                        ),
                )
                .child(
                    Button::new("run-data-file")
                        .label("Data file…")
                        .on_click(cx.listener(|this, _, window, cx| this.pick_data(window, cx))),
                )
                .children(data_name.map(|name| {
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .id("run-data-name")
                                .test_support()
                                .aria_label(name.clone())
                                .text_size(style::TEXT_CONTROL)
                                .child(name),
                        )
                        .child(Button::new("run-data-clear").label("Clear").on_click(
                            move |_, _, cx| m4.update(cx, |m, cx| m.set_data_file(None, cx)),
                        ))
                }));
        // The export buttons appear once a run exists and work once it has finished.
        let finished = matches!(model.state(), RunState::Finished(_));
        let exports = (running || finished).then(|| {
            let button = |id: &'static str, format: ReportFormat| {
                let m = self.model.clone();
                Button::new(id)
                    .label(format.label())
                    .lock(!finished, cx)
                    .on_click(move |_, window, cx| export_results(&m, format, window, cx))
            };
            h_flex()
                .gap_1()
                .items_center()
                .child(option_label("run-export-label", "Export", cx))
                .child(button("run-export-junit", ReportFormat::Junit))
                .child(button("run-export-json", ReportFormat::Json))
                .child(button("run-export-html", ReportFormat::Html))
        });
        // Shown only for a finished run that has failures to run again.
        let rerunnable = finished && !model.failed_items().is_empty();
        let m4 = self.model.clone();
        // The checked count, once the plan has arrived; a plan with no requests at all keeps
        // Run open so its load errors can still be reported.
        let chosen = (model.preview_loaded() && !model.preview().is_empty())
            .then(|| model.checked_count(cx));
        // The action row never scrolls, so Run stays in view however short the pane.
        let actions = h_flex()
            .id("run-actions")
            .test_support()
            .w_full()
            .flex_none()
            .justify_end()
            .gap_2()
            .children(exports)
            .when(running, |d| {
                d.child(
                    Button::new("run-cancel")
                        .label("Cancel")
                        .on_click(move |_, _, cx| m3.update(cx, |m, cx| m.cancel(cx))),
                )
            })
            .when(rerunnable, |d| {
                d.child(
                    Button::new("run-rerun-failed")
                        .label("Re-run failed")
                        .on_click(move |_, _, cx| {
                            m4.update(cx, |m, cx| m.rerun_failed(cx));
                        }),
                )
            })
            .child(
                Button::new("run-start")
                    .primary()
                    .min_w(style::SEND_SLOT_WIDTH)
                    .label(run_label(chosen))
                    .lock_filled(running || chosen == Some(0), cx)
                    .on_click(move |_, _, cx| {
                        m2.update(cx, |m, cx| m.start(cx));
                    }),
            );
        let options_box = v_flex()
            .id("run-options-box")
            .test_support()
            .relative()
            .w_full()
            .flex_initial()
            .min_h(style::INPUT_HEIGHT)
            .child(options)
            .children(ui::scroll_bar_y(
                "run-options-scrollbar",
                &self.options_scroll,
            ));
        let waiting = model.is_waiting().then(|| {
            option_label("run-waiting", "Waiting for your answer", cx).text_size(style::TEXT_SMALL)
        });
        // Until a run produces rows, the tab lists what Run would send.
        let previewing = model.rows().is_empty()
            && !matches!(model.state(), RunState::Running | RunState::Finished(_));
        let plan = previewing.then(|| {
            let count = plan_count_text(model.preview().len(), model.preview_rounds());
            let preview = model.preview();
            let checked: Vec<bool> = (preview.iter())
                .map(|row| model.is_checked(&row.path, cx))
                .collect();
            let scope = match model.target() {
                RunTarget::Scope(item) => item.as_str().to_owned(),
                RunTarget::Items(_) => String::new(),
            };
            let entries = plan_entries(preview, &scope);
            let folder_names = model.folder_names(cx);
            let rows = entries.iter().enumerate().map(|(k, entry)| match entry {
                PlanEntry::Folder { path, depth } => {
                    let members = folder_members(preview, path);
                    let on = members.iter().filter(|&&ix| checked[ix]).count();
                    let items: Vec<ItemPath> =
                        members.iter().map(|&ix| preview[ix].path.clone()).collect();
                    let name = (folder_names.get(path).cloned())
                        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_owned());
                    let mixed = on > 0 && on < members.len();
                    let a11y = partly_named(&name, mixed);
                    let m = self.model.clone();
                    h_flex()
                        .id(("run-plan-folder", k))
                        .test_support()
                        .aria_label(name.clone())
                        .pl(rems(0.75 + PLAN_INDENT * *depth as f32))
                        .gap_2()
                        .child(
                            ui::checkbox(("run-plan-folder-check", k), on == members.len())
                                .mixed(mixed)
                                .accessible_name(a11y)
                                .on_click(move |on: &bool, _, cx| {
                                    m.update(cx, |m, cx| m.set_checked(&items, *on, cx))
                                }),
                        )
                        .child(name)
                        .into_any_element()
                }
                PlanEntry::Request { ix, depth } => {
                    let (ix, row) = (*ix, &preview[*ix]);
                    let text = format!("{} {}", row.method.to_ascii_uppercase(), row.name);
                    let (m, item) = (self.model.clone(), [row.path.clone()]);
                    ListItem::new(("run-plan-item", ix))
                        .pl(rems(0.75 + PLAN_INDENT * *depth as f32))
                        .child(
                            h_flex()
                                .id(("run-plan-row", ix))
                                .test_support()
                                .aria_label(text.clone())
                                .gap_2()
                                .child(
                                    ui::checkbox(("run-plan-check", ix), checked[ix])
                                        .accessible_name(text)
                                        .on_click(move |on: &bool, _, cx| {
                                            m.update(cx, |m, cx| m.set_checked(&item, *on, cx))
                                        }),
                                )
                                .child(ui::method_tag(&row.method))
                                .child(row.name.clone()),
                        )
                        .into_any_element()
                }
            });
            let on = checked.iter().filter(|c| **c).count();
            let all: Vec<ItemPath> = preview.iter().map(|row| row.path.clone()).collect();
            let m = self.model.clone();
            let select_all = (!preview.is_empty()).then(|| {
                let mixed = on > 0 && on < preview.len();
                ui::checkbox("run-select-all", on == preview.len())
                    .mixed(mixed)
                    .label("Select all")
                    .accessible_name(partly_named("Select all", mixed))
                    .on_click(move |on: &bool, _, cx| {
                        m.update(cx, |m, cx| m.set_checked(&all, *on, cx))
                    })
            });
            list_box(shrink_scroll("run-plan"), entries.len()).child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex().gap_4().items_center().children(select_all).child(
                            div()
                                .id("run-plan-count")
                                .test_support()
                                .aria_label(count.clone())
                                .text_size(style::TEXT_SMALL)
                                .text_color(cx.theme().muted_foreground)
                                .child(count),
                        ),
                    )
                    .children(rows),
            )
        });
        let errors = model.load_errors().iter().enumerate().map(|(k, e)| {
            div()
                .id(("run-load-error", k))
                .test_support()
                .child(e.clone())
        });
        let notice = (!model.unsaved().is_empty()).then(|| {
            let text = format!(
                "Unsaved edits are not part of this run: {}",
                model.unsaved().join(", ")
            );
            div()
                .id("runner-unsaved-notice")
                .test_support()
                .aria_label(text.clone())
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(text)
        });
        let stale = (model.results_stale() && !model.is_running()).then(|| {
            option_label(
                "run-stale",
                "Results from the last run. Run again to apply the changed options.",
                cx,
            )
            .text_size(style::TEXT_SMALL)
        });
        let has_errors = !model.load_errors().is_empty();
        let row_count = model.rows().len();
        let view = cx.entity();
        let muted = cx.theme().muted_foreground;
        let col = style::SHORT_FORM_LABEL_WIDTH;
        // The header pads like a list item so its columns sit over the rows'.
        let header = (!model.rows().is_empty()).then(|| {
            h_flex()
                .id("run-table-header")
                .test_support()
                .px_3()
                .gap_2()
                .text_size(style::TEXT_SMALL)
                .text_color(muted)
                .child(cell(col).child("Method"))
                .child(
                    div()
                        .id("run-table-name")
                        .test_support()
                        .flex_1()
                        .min_w(style::RUN_NAME_MIN_WIDTH)
                        .child("Name"),
                )
                .child(cell(col).child("Status"))
                .child(cell(col).child("Time"))
                .child(cell(style::RUN_RESULT_WIDTH).child("Result"))
        });
        let env = model.selected_environment(cx);
        let mut rows = Vec::new();
        for (ix, row) in model.rows().iter().enumerate() {
            // A header leads the first row of each data row's iteration.
            if let Some(i) = row.iteration
                && (ix == 0 || model.rows()[ix - 1].iteration != row.iteration)
            {
                let text = format!("Iteration {}", i + 1);
                rows.push(
                    div()
                        .id(("run-iteration", i))
                        .test_support()
                        .aria_label(text.clone())
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child(text)
                        .into_any_element(),
                );
            }
            let view = view.clone();
            let name = format!("{} {}", row.method.to_ascii_uppercase(), row.name);
            // The reason sits on its own line so a long one wraps instead of widening the row.
            let (reason, checks, response) = match &row.state {
                RowState::Done(r) => (
                    failure_reason(r, env.as_deref()),
                    row_checks(r),
                    r.response.as_ref(),
                ),
                _ => (None, None, None),
            };
            let status = response.map(|resp| {
                let code = resp.status();
                cell(col)
                    .id(("run-status", ix))
                    .test_support()
                    .aria_label(code.to_string())
                    .text_color(ui::status_class(code).color(cx))
                    .child(code.to_string())
            });
            let time = response.map(|resp| {
                let text = body_text::format_time(resp.response_time());
                cell(col)
                    .id(("run-time", ix))
                    .test_support()
                    .aria_label(text.clone())
                    .text_color(muted)
                    .child(text)
            });
            let result = row_label(&row.state, model.retrying(ix));
            let item = ListItem::new(("run-row", ix))
                .selected(self.selected == Some(ix))
                .on_click(move |_, window, cx| view.update(cx, |v, cx| v.select(ix, window, cx)))
                .child(
                    v_flex()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    cell(col)
                                        .id(("run-method", ix))
                                        .test_support()
                                        .child(ui::method_tag(&row.method)),
                                )
                                .child(
                                    div()
                                        .id(("run-name", ix))
                                        .test_support()
                                        .aria_label(name)
                                        .flex_1()
                                        .min_w(style::RUN_NAME_MIN_WIDTH)
                                        .truncate()
                                        .child(row.name.clone()),
                                )
                                .child(cell_or_gap(status, col))
                                .child(cell_or_gap(time, col))
                                .child(
                                    cell(style::RUN_RESULT_WIDTH)
                                        .id(("run-outcome", ix))
                                        .test_support()
                                        .aria_label(result.clone())
                                        .text_color(row_tone(&row.state).colour(cx))
                                        .child(result),
                                ),
                        )
                        // An empty Method cell indents each detail line under the Name.
                        .children(
                            [("run-reason", reason), ("run-checks", checks)]
                                .into_iter()
                                .filter_map(|(id, text)| Some((id, text?)))
                                .map(|(id, text)| {
                                    h_flex().gap_2().child(cell(col)).child(
                                        div()
                                            .id((id, ix))
                                            .test_support()
                                            .aria_label(text.clone())
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(style::TEXT_SMALL)
                                            .text_color(muted)
                                            .child(text),
                                    )
                                }),
                        ),
                );
            rows.push(item.into_any_element());
        }
        let status = match model.state() {
            RunState::Finished(_) => {
                let t = tally(model.rows());
                Some(("run-summary", summary_line(&t), Some(summary_tone(&t))))
            }
            RunState::Cancelled => Some(("run-summary", "Cancelled".to_owned(), None)),
            RunState::Failed(message) => Some(("run-failed", message.clone(), None)),
            RunState::Idle | RunState::Planning | RunState::Running => None,
        };
        v_flex()
            .id("runner")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .p(style::PANE_INSET)
            .pl(style::tab_content_inset(window))
            .pt(style::tab_content_inset(window))
            .gap_2()
            .child(actions)
            .child(options_box)
            // One-line status keeps its height, so the list always starts below it; load
            // errors can run long, so they alone shrink and scroll.
            .children(waiting.map(|d| d.flex_none()))
            .children(notice.map(|d| d.flex_none()))
            .children(stale.map(|d| d.flex_none()))
            .when(has_errors, |d| {
                d.child(shrink_scroll("run-notes").child(v_flex().gap_2().children(errors)))
            })
            .children(plan)
            .when(!rows.is_empty(), |d| {
                // Below its least width the table scrolls sideways in its own area, so the
                // Name column never collapses.
                // A sideways scrollbar under the box says columns are hidden, wherever the visible ones end.
                d.child(
                    list_box(div().relative().w_full(), row_count)
                        .flex()
                        .flex_col()
                        .child(
                            shrink_scroll("run-table")
                                .track_scroll(&self.table_scroll)
                                .child(
                                    v_flex()
                                        .min_w(style::RUN_TABLE_MIN_WIDTH)
                                        .pr(style::SCROLLBAR_GUTTER)
                                        .gap_2()
                                        .children(header)
                                        .children(rows),
                                ),
                        )
                        .children(ui::scroll_bar_x("run-table-scrollbar", &self.table_scroll))
                        .children(ui::scroll_bar_y(
                            "run-table-scrollbar-y",
                            &self.table_scroll,
                        )),
                )
            })
            .children(status.map(|(id, text, tone)| {
                div()
                    .flex_none()
                    .id(id)
                    .test_support()
                    .aria_label(text.clone())
                    .when_some(tone, |d, tone| d.text_color(tone.colour(cx)))
                    .child(text)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ItemPath, Outcome, RequestResult, RowState, RunRow, Tally, failure_reason, plan_count_text,
        result_word, row_checks, row_tone, summary_line, summary_tone, tally,
    };
    use crate::ui::Tone;
    use apinata_core::{Assertion, AssertionOutcome, Verdict};
    use std::sync::Arc;

    fn bare(outcome: Outcome) -> RequestResult {
        let path = ItemPath::new("a.yml").expect("a valid item path");
        RequestResult::from_parts(path, "a", outcome, None, Vec::new())
    }

    #[test]
    fn a_row_with_no_detail_still_names_its_outcome() {
        assert_eq!(
            failure_reason(&bare(Outcome::Failed), None).as_deref(),
            Some("Failed")
        );
        assert_eq!(
            failure_reason(&bare(Outcome::Errored), None).as_deref(),
            Some("Errored")
        );
        assert_eq!(
            failure_reason(&bare(Outcome::Cancelled), None).as_deref(),
            Some("Cancelled")
        );
        assert_eq!(failure_reason(&bare(Outcome::Passed), None), None);
    }

    #[test]
    fn a_run_error_reads_as_its_friendly_title_and_sentence() {
        use apinata_core::{ConnectCause, ExecError, ResolveError, RunError};
        let mut r = bare(Outcome::Errored);
        r.error = Some(RunError::Exec(ExecError::connect(
            "http://127.0.0.1:1/",
            "tcp connect error",
            ConnectCause::Refused,
        )));
        assert_eq!(
            failure_reason(&r, None).as_deref(),
            Some("Connection refused: Nothing is listening at 127.0.0.1:1. Is the server running?")
        );
        r.error = Some(RunError::Resolve(ResolveError::UndefinedVariable {
            name: "missing".into(),
            hint: None,
        }));
        assert_eq!(
            failure_reason(&r, Some("dev")).as_deref(),
            Some("Undefined variable: missing is not defined in dev.")
        );
    }

    #[test]
    fn a_retrying_row_names_its_attempt() {
        assert_eq!(super::retrying_label(2, 3), "Retrying, attempt 2 of 3");
    }

    #[test]
    fn a_cancel_in_the_pause_adds_the_failure_of_the_attempt_before() {
        use apinata_core::{ConnectCause, ExecError, RunError};
        let mut r = bare(Outcome::Cancelled);
        r.error = Some(RunError::Exec(ExecError::Cancelled));
        r.last_attempt_error = Some(RunError::Exec(ExecError::connect(
            "http://127.0.0.1:1/",
            "tcp connect error",
            ConnectCause::Refused,
        )));
        let reason = failure_reason(&r, None).expect("a reason");
        assert!(reason.contains("cancelled; last attempt: "), "{reason}");
        assert!(
            reason.ends_with(
                "last attempt: Connection refused: Nothing is listening at 127.0.0.1:1. Is the server running?"
            ),
            "{reason}"
        );
        r.last_attempt_error = None;
        assert!(!failure_reason(&r, None).unwrap().contains("last attempt"));
    }

    #[test]
    fn a_declined_network_prompt_reads_as_a_cancel_in_the_row() {
        use apinata_core::{ExecError, RunError};
        let mut r = bare(Outcome::Cancelled);
        r.error = Some(RunError::Exec(ExecError::NetworkDeclined));
        let reason = failure_reason(&r, None).expect("a reason");
        assert!(!reason.contains("failed"), "{reason}");
        assert!(
            reason.starts_with("Cancelled") && reason.contains("not confirmed"),
            "{reason}"
        );
    }

    #[test]
    fn the_plan_count_reads_naturally() {
        assert_eq!(plan_count_text(0, None), "No requests to run");
        assert_eq!(plan_count_text(1, None), "1 request");
        assert_eq!(plan_count_text(3, None), "3 requests");
        assert_eq!(
            plan_count_text(2, Some(1)),
            "2 requests for each of 1 data row"
        );
        assert_eq!(
            plan_count_text(1, Some(3)),
            "1 request for each of 3 data rows"
        );
    }

    #[test]
    fn the_run_button_names_the_checked_count() {
        use super::run_label;
        assert_eq!(run_label(None), "Run");
        assert_eq!(run_label(Some(0)), "Run 0 requests");
        assert_eq!(run_label(Some(1)), "Run 1 request");
        assert_eq!(run_label(Some(7)), "Run 7 requests");
    }

    #[test]
    fn plan_entries_head_each_folder_once_below_the_scope() {
        use super::{PlanEntry, PlannedRow, plan_entries};
        let rows: Vec<PlannedRow> = ["f/g/x.yml", "f/y.yml", "h/z.yml", "a.yml"]
            .iter()
            .map(|p| PlannedRow {
                path: ItemPath::new(p).expect("a valid item path"),
                name: (*p).to_owned(),
                method: "get".to_owned(),
            })
            .collect();
        let folder = |path: &str, depth| PlanEntry::Folder {
            path: path.to_owned(),
            depth,
        };
        let request = |ix, depth| PlanEntry::Request { ix, depth };
        assert_eq!(
            plan_entries(&rows, ""),
            [
                folder("f", 0),
                folder("f/g", 1),
                request(0, 2),
                request(1, 1),
                folder("h", 0),
                request(2, 1),
                request(3, 0),
            ]
        );
        // A runner of folder f shows no heading for f itself.
        assert_eq!(
            plan_entries(&rows[..2], "f"),
            [folder("f/g", 0), request(0, 1), request(1, 0)]
        );
    }

    fn checked(outcome: Outcome, verdict: Verdict) -> RequestResult {
        let path = ItemPath::new("a.yml").expect("a valid item path");
        let a = AssertionOutcome::new(
            Assertion::new("res.status", "eq", Some("200".into())),
            verdict,
        );
        RequestResult::from_parts(path, "a", outcome, None, vec![a])
    }

    fn row(state: RowState) -> RunRow {
        RunRow {
            path: ItemPath::new("a.yml").expect("a valid item path"),
            name: "a".into(),
            method: "GET".into(),
            state,
            iteration: None,
        }
    }

    fn done(r: RequestResult) -> RunRow {
        row(RowState::Done(Arc::new(r)))
    }

    #[test]
    fn an_unchecked_pass_reads_no_tests() {
        assert_eq!(result_word(&bare(Outcome::Passed)), "No tests");
        assert_eq!(
            result_word(&checked(Outcome::Passed, Verdict::Pass)),
            "Passed"
        );
        assert_eq!(
            result_word(&checked(Outcome::Failed, Verdict::fail("500", None))),
            "Failed"
        );
        assert_eq!(result_word(&bare(Outcome::Errored)), "Errored");
        assert_eq!(result_word(&bare(Outcome::Skipped)), "Not sent");
        assert_eq!(result_word(&bare(Outcome::Cancelled)), "Cancelled");
    }

    #[test]
    fn the_summary_counts_rows_not_core_passes() {
        let rows = [
            done(checked(Outcome::Passed, Verdict::Pass)),
            done(bare(Outcome::Passed)),
            done(checked(Outcome::Failed, Verdict::fail("500", None))),
            done(bare(Outcome::Errored)),
            row(RowState::Queued),
        ];
        let t = tally(&rows);
        let want = Tally {
            passed: 2,
            failed: 1,
            errored: 1,
            no_tests: 1,
            ..Tally::default()
        };
        assert_eq!(t, want);
        assert_eq!(
            summary_line(&t),
            "4 requests: 2 passed (1 no tests), 1 failed, 1 errored"
        );
        assert_eq!(summary_tone(&t), Tone::Danger);
        let green = tally(&[done(checked(Outcome::Passed, Verdict::Pass))]);
        assert_eq!(summary_line(&green), "1 request: 1 passed, 0 failed");
        assert_eq!(summary_tone(&green), Tone::Success);
        let unchecked = tally(&[done(bare(Outcome::Passed))]);
        assert_eq!(
            summary_line(&unchecked),
            "1 request: 1 passed (1 no tests), 0 failed"
        );
        assert_eq!(summary_tone(&unchecked), Tone::Success);
    }

    fn scripts_off(r: RequestResult) -> RequestResult {
        use apinata_core::{ScriptPhase, TestOutcome};
        let mut r = r;
        r.tests = vec![TestOutcome::new(
            ScriptPhase::Tests,
            "request tests",
            Verdict::skipped("scripts are off"),
        )];
        r
    }

    #[test]
    fn skipped_tests_are_stated_as_the_tests_tab_states_them() {
        let r = scripts_off(checked(Outcome::Passed, Verdict::Pass));
        assert_eq!(
            row_checks(&r).as_deref(),
            Some("1 passed, 0 failed, 1 skipped")
        );
        assert_eq!(
            row_checks(&checked(Outcome::Passed, Verdict::Pass)),
            None,
            "nothing skipped, nothing to add"
        );
        let t = tally(&[done(r), done(checked(Outcome::Passed, Verdict::Pass))]);
        assert_eq!(t.skipped_tests, 1);
        assert_eq!(
            summary_line(&t),
            "2 requests: 2 passed, 0 failed; 1 check skipped"
        );
        assert_eq!(summary_tone(&t), Tone::Warning);
        // A request whose every check was skipped is not "No tests".
        let all_skipped = scripts_off(bare(Outcome::Passed));
        assert_eq!(result_word(&all_skipped), "Unverified");
        assert_eq!(
            row_checks(&all_skipped).as_deref(),
            Some("0 passed, 0 failed, 1 skipped")
        );
        let two = tally(&[
            done(all_skipped.clone()),
            done(all_skipped),
            done(bare(Outcome::Passed)),
        ]);
        assert_eq!(
            summary_line(&two),
            "3 requests: 3 passed (2 unverified, 1 no tests), 0 failed; 2 checks skipped"
        );
        assert_eq!(
            (two.passed, two.unverified, two.no_tests, two.not_sent),
            (3, 2, 1, 0)
        );
        assert_eq!(summary_tone(&two), Tone::Warning);
    }

    #[test]
    fn unsent_requests_read_as_not_sent_and_do_not_double_count_unverified() {
        let rows = [
            done(scripts_off(bare(Outcome::Passed))),
            done(bare(Outcome::Skipped)),
            done(bare(Outcome::Cancelled)),
        ];
        let t = tally(&rows);
        assert_eq!(
            summary_line(&t),
            "3 requests: 1 passed (1 unverified), 0 failed, 1 not sent, 1 cancelled; 1 check skipped"
        );
        assert_eq!(row_tone(&rows[0].state), Tone::Warning);
        assert_eq!(summary_tone(&tally(&rows[1..2])), Tone::Muted);
        let failed = tally(&[
            done(bare(Outcome::Failed)),
            done(scripts_off(bare(Outcome::Passed))),
        ]);
        assert_eq!(summary_tone(&failed), Tone::Danger);
    }
}
