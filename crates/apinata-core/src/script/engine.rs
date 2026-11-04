//! One script phase: a fresh QuickJS runtime that runs a list of segments in order under
//! time, memory and console limits, and reports what they logged, tested and threw.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rquickjs::context::EvalOptions;
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Context, Ctx, Function, Object,
    Promise, Runtime, Value,
};

use crate::CancelHandle;

const PRELUDE: &str = include_str!("prelude.js");
/// The file name scripts are evaluated under; stack frames carrying it are the user's code.
pub(crate) const SCRIPT_FILE: &str = "apinata-script";
const LEVELS: [&str; 5] = ["log", "debug", "info", "warn", "error"];

/// The bounds one phase runs under.
pub(crate) struct Limits {
    pub(crate) timeout: Duration,
    pub(crate) memory: usize,
    pub(crate) console: usize,
    /// A deadline the phase may not outlive whatever its own timeout, such as the deadline of
    /// the phase whose `bru.runRequest` started it.
    pub(crate) outer: Option<Instant>,
}

/// One script body and the level that names it in errors (`collection`, `folder a/b`, `request`).
pub(crate) struct Segment {
    pub(crate) level: String,
    pub(crate) code: String,
}

/// Installs host functions into a phase's context before the scripts run.
pub(crate) type Installer =
    Box<dyn for<'js> FnOnce(&rquickjs::Ctx<'js>) -> rquickjs::Result<()> + Send>;

/// Why a phase ended early.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Failure {
    Thrown { message: String },
    Unsupported { message: String },
    Limit { message: String },
    Cancelled,
}

/// One `console` call: its level and its formatted text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawConsole {
    pub(crate) level: &'static str,
    pub(crate) text: String,
}

/// How one `test(name, fn)` ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawVerdict {
    Pass,
    Fail(String),
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawTest {
    pub(crate) name: String,
    pub(crate) verdict: RawVerdict,
}

/// Everything a phase produced; `dropped` counts console calls past the budget.
#[derive(Debug, Default)]
pub(crate) struct PhaseRun {
    pub(crate) failure: Option<Failure>,
    pub(crate) console: Vec<RawConsole>,
    pub(crate) dropped: usize,
    pub(crate) tests: Vec<RawTest>,
}

/// The time limit's error text, in seconds as the settings state it.
pub(crate) fn time_limit_message(timeout: Duration) -> String {
    format!("script exceeded the {} s limit", timeout.as_secs_f64())
}

fn memory_limit_message(memory: usize) -> String {
    format!("script exceeded the {} MiB memory limit", memory >> 20)
}

/// Console and test records, shared with the host functions and kept outside the phase task
/// so a panic, an abort or a timeout still returns what was recorded.
struct Capture {
    budget: usize,
    run: PhaseRun,
}

impl Capture {
    fn log(&mut self, level: &str, text: String) {
        if self.run.console.len() >= self.budget {
            self.run.dropped += 1;
            return;
        }
        let level = LEVELS.into_iter().find(|l| *l == level).unwrap_or("log");
        self.run.console.push(RawConsole { level, text });
    }

    fn finish(&mut self, failure: Option<Failure>) -> PhaseRun {
        PhaseRun {
            failure,
            ..std::mem::take(&mut self.run)
        }
    }
}

type Sink = Arc<Mutex<Capture>>;

fn lock(sink: &Sink) -> std::sync::MutexGuard<'_, Capture> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `segments` in order in a fresh sandbox, after `installers` add their host functions.
/// A throw, a limit or `cancel` ends the phase; records made until then are kept.
pub(crate) async fn run_phase(
    segments: Vec<Segment>,
    installers: Vec<Installer>,
    limits: Limits,
    cancel: CancelHandle,
) -> PhaseRun {
    let sink = Arc::new(Mutex::new(Capture {
        budget: limits.console,
        run: PhaseRun::default(),
    }));
    let mut task = tokio::spawn(drive(
        segments,
        installers,
        limits,
        cancel.clone(),
        Arc::clone(&sink),
    ));
    // A caller that drops this future, as an ended outer phase drops a nested run, stops the
    // phase too: the interrupt handler alone misses a script that mostly awaits.
    let _abort = AbortOnDrop(task.abort_handle());
    // Race cancellation here rather than inside `drive`: wrapping the `async_with` future
    // there breaks the phase future's `Send` proof.
    let Some(joined) = cancel.run(&mut task).await else {
        task.abort();
        let _ = task.await;
        return lock(&sink).finish(Some(Failure::Cancelled));
    };
    let failure = match joined {
        Ok(failure) => failure,
        Err(e) if e.is_panic() => Some(thrown("the script host panicked")),
        Err(_) => Some(thrown("the script host stopped")),
    };
    lock(&sink).finish(failure)
}

/// Aborts a spawned task when dropped; a no-op once the task has finished.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn thrown(message: impl Into<String>) -> Failure {
    Failure::Thrown {
        message: message.into(),
    }
}

/// How the evaluation inside the context ended, before the limits are taken into account.
enum Outcome {
    Done,
    /// A JavaScript throw: `String(value)`, its line in the wrapped source if known, and the
    /// segment that was running.
    Threw {
        text: String,
        line: Option<u32>,
        segment: usize,
        unsupported: bool,
    },
    /// The engine failed without a JavaScript value, as an interrupt or allocation failure does.
    Aborted {
        text: String,
    },
}

/// The phase body, run on its own task: build the runtime, evaluate, then classify the end.
async fn drive(
    segments: Vec<Segment>,
    installers: Vec<Installer>,
    limits: Limits,
    cancel: CancelHandle,
    sink: Sink,
) -> Option<Failure> {
    // A limit too large to add to the clock means no deadline rather than a panic.
    let deadline = earliest(Instant::now().checked_add(limits.timeout), limits.outer);
    let Ok(rt) = AsyncRuntime::new() else {
        return Some(thrown("the script host could not start"));
    };
    rt.set_memory_limit(limits.memory).await;
    // The interrupt handler stops running bytecode at the deadline or on cancel; the flag
    // lets the classification below tell a time limit from an ordinary throw.
    let timed_out = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&timed_out);
    let stop = cancel.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        let hit = deadline.is_some_and(|d| Instant::now() >= d);
        if hit {
            flag.store(true, Relaxed);
        }
        hit || stop.is_cancelled()
    })))
    .await;
    let Ok(context) = AsyncContext::full(&rt).await else {
        return Some(thrown("the script host could not start"));
    };

    let (source, starts) = wrap(&segments);
    let evaluation =
        context.async_with(async move |ctx| evaluate(&ctx, source, installers, sink).await);
    let budget = deadline.map_or(limits.timeout, |d| {
        d.saturating_duration_since(Instant::now())
    });
    let outcome = match tokio::time::timeout(budget, evaluation).await {
        Ok(outcome) => outcome,
        Err(_) => {
            timed_out.store(true, Relaxed);
            Outcome::Aborted {
                text: String::new(),
            }
        }
    };
    // A worker kept busy, as by a spinning nested run, can hold back the timer above until
    // the script has finished late, so the clock decides.
    if deadline.is_some_and(|d| Instant::now() >= d) {
        timed_out.store(true, Relaxed);
    }
    // Promises the scripts left running, such as un-awaited async tests, settle before the end.
    if matches!(outcome, Outcome::Done) {
        let remaining = deadline.map_or(limits.timeout, |d| {
            d.saturating_duration_since(Instant::now())
        });
        if tokio::time::timeout(remaining, rt.idle()).await.is_err() {
            timed_out.store(true, Relaxed);
        }
    }

    if cancel.is_cancelled() {
        return Some(Failure::Cancelled);
    }
    if timed_out.load(Relaxed) {
        return Some(Failure::Limit {
            message: time_limit_message(limits.timeout),
        });
    }
    let (text, line, segment) = match outcome {
        Outcome::Done => return None,
        Outcome::Threw {
            text,
            unsupported: true,
            ..
        } => {
            return Some(Failure::Unsupported { message: text });
        }
        Outcome::Threw {
            text,
            line,
            segment,
            ..
        } => (text, line, Some(segment)),
        Outcome::Aborted { text } => (text, None, None),
    };
    // The heap is freed by the time the error is seen, so out of memory is recognised by its
    // message or by usage still near the limit; `throw null` on a small heap stays a throw.
    if out_of_memory(&text, rt.memory_usage().await.malloc_size, limits.memory) {
        return Some(Failure::Limit {
            message: memory_limit_message(limits.memory),
        });
    }
    // An untranslated Postman call finds no `pm` global; name the call the importer reported.
    let text = match segment {
        Some(running) => name_missing_pm(&segments, running, text),
        None => text,
    };
    // An engine failure without a JavaScript value belongs to no script line.
    Some(thrown(match segment {
        Some(running) => attribute(&segments, &starts, line, running, &text),
        None => text,
    }))
}

/// Globals an imported script may use that are missing here, with what to tell the user.
const MISSING_GLOBAL_HINTS: [(&str, &str); 3] = [
    ("CryptoJS", "use require('crypto-js')"),
    ("cheerio", "cheerio is not available here"),
    ("xml2Json", "xml2Json is not available here"),
];

/// Rewrites "ReferenceError: pm is not defined" (or `postman`) to name the first `pm.` call
/// in the running segment; any other text passes through.
fn name_missing_pm(segments: &[Segment], running: usize, text: String) -> String {
    if let Some((_, hint)) = MISSING_GLOBAL_HINTS
        .iter()
        .find(|(name, _)| text == format!("ReferenceError: {name} is not defined"))
    {
        return format!("{text}; {hint}");
    }
    let global = match text.as_str() {
        "ReferenceError: pm is not defined" => "pm",
        "ReferenceError: postman is not defined" => "postman",
        _ => return text,
    };
    let chain = segments
        .get(running)
        .and_then(|s| crate::import::first_pm_chain(&s.code))
        .unwrap_or_else(|| global.to_owned());
    format!("{chain} is not available; see the import report")
}

/// The sooner of two optional deadlines, where `None` is no deadline.
pub(crate) fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        _ => a.or(b),
    }
}

/// Whether an engine failure was the memory limit: its message, or usage still near the limit.
fn out_of_memory(text: &str, used: i64, limit: usize) -> bool {
    text == "InternalError: out of memory" || used + (1 << 20) >= limit as i64
}

/// Evaluates `code` as one expression in a fresh synchronous runtime under `limits`, with each
/// of `vars` a global and `installers` run over them. `Ok(None)` is `undefined` or a value
/// without JSON form; a throw reads as `String(error)`, without level or line.
pub(crate) fn eval_expression(
    code: &str,
    vars: &serde_json::Map<String, serde_json::Value>,
    installers: Vec<Installer>,
    limits: &Limits,
) -> Result<Option<serde_json::Value>, Failure> {
    let deadline = Instant::now().checked_add(limits.timeout);
    let rt = Runtime::new().map_err(|_| thrown("the script host could not start"))?;
    rt.set_memory_limit(limits.memory);
    let timed_out = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&timed_out);
    rt.set_interrupt_handler(Some(Box::new(move || {
        let hit = deadline.is_some_and(|d| Instant::now() >= d);
        if hit {
            flag.store(true, Relaxed);
        }
        hit
    })));
    let context = Context::full(&rt).map_err(|_| thrown("the script host could not start"))?;
    // Console output from an expression has nowhere to go, so the sink keeps none of it.
    let sink = Arc::new(Mutex::new(Capture {
        budget: 0,
        run: PhaseRun::default(),
    }));
    let outcome = context.with(|ctx| {
        if let Err(e) = set_vars(&ctx, vars).and_then(|()| install(&ctx, installers, sink)) {
            return Err(Outcome::Aborted {
                text: format!("the script host could not start: {e}"),
            });
        }
        let mut options = EvalOptions::default();
        options.strict = false;
        options.filename = Some(SCRIPT_FILE.to_owned());
        let value = ctx
            .eval_with_options::<Value, _>(code, options)
            .catch(&ctx)
            .map_err(|e| caught(&ctx, e))?;
        if value.is_undefined() {
            return Ok(None);
        }
        let json = ctx
            .json_stringify(value)
            .catch(&ctx)
            .map_err(|e| caught(&ctx, e))?;
        Ok(json
            .and_then(|text| text.to_string().ok())
            .and_then(|text| serde_json::from_str(&text).ok()))
    });
    let text = match outcome {
        Ok(value) => return Ok(value),
        Err(_) if timed_out.load(Relaxed) => {
            return Err(Failure::Limit {
                message: time_limit_message(limits.timeout),
            });
        }
        Err(Outcome::Threw {
            text,
            unsupported: true,
            ..
        }) => return Err(Failure::Unsupported { message: text }),
        Err(Outcome::Threw { text, .. } | Outcome::Aborted { text }) => text,
        Err(Outcome::Done) => String::new(),
    };
    if out_of_memory(&text, rt.memory_usage().malloc_size, limits.memory) {
        return Err(Failure::Limit {
            message: memory_limit_message(limits.memory),
        });
    }
    Err(thrown(text))
}

/// Sets each variable as a global, skipping names the engine already defines so a variable
/// called `JSON` or `String` cannot break the host's own code.
fn set_vars<'js>(
    ctx: &Ctx<'js>,
    vars: &serde_json::Map<String, serde_json::Value>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    for (name, value) in vars {
        if globals.contains_key(name.as_str())? {
            continue;
        }
        globals.set(name.as_str(), ctx.json_parse(value.to_string())?)?;
    }
    Ok(())
}

/// Joins the segments into one async function, each in its own async wrapper so `return`
/// leaves only that segment. Returns the source and each segment's first line in it.
pub(crate) fn wrap(segments: &[Segment]) -> (String, Vec<u32>) {
    let mut source = String::from("(async () => {\n");
    let mut starts = Vec::with_capacity(segments.len());
    let mut line = 2;
    for (i, segment) in segments.iter().enumerate() {
        source.push_str(&format!("__apinata.segment = {i}; await (async () => {{\n"));
        starts.push(line + 1);
        source.push_str(&segment.code);
        source.push_str("\n})();\n");
        line += segment.code.matches('\n').count() as u32 + 3;
    }
    source.push_str("})()");
    (source, starts)
}

/// `<level> script, line <n>: <text>`, the line relative to its segment; without a line the
/// segment that was running names the level.
pub(crate) fn attribute(
    segments: &[Segment],
    starts: &[u32],
    line: Option<u32>,
    running: usize,
    text: &str,
) -> String {
    let located = line.and_then(|line| {
        let index = starts.iter().rposition(|start| *start <= line)?;
        Some((index, line - starts[index] + 1))
    });
    match located {
        Some((index, line)) => {
            let quote = quoted_line(&segments[index].code, line, text);
            format!(
                "{} script, line {line}: {text}{quote}",
                segments[index].level
            )
        }
        None => match segments.get(running) {
            Some(segment) => format!("{} script: {text}", segment.level),
            None => text.to_owned(),
        },
    }
}

/// ` in "<line>"` for an engine-raised TypeError, ReferenceError or RangeError, quoting the
/// user's own line trimmed and cut to `QUOTE_MAX` characters; empty for any other text.
fn quoted_line(code: &str, line: u32, text: &str) -> String {
    const QUOTE_MAX: usize = 120;
    let engine_raised = ["TypeError:", "ReferenceError:", "RangeError:"]
        .iter()
        .any(|prefix| text.starts_with(prefix));
    let source = code.lines().nth(line as usize - 1).map(str::trim);
    match source {
        Some(source) if engine_raised && !source.is_empty() => {
            let cut: String = source.chars().take(QUOTE_MAX).collect();
            let ellipsis = if cut.len() < source.len() { "..." } else { "" };
            format!(" in \"{cut}{ellipsis}\"")
        }
        _ => String::new(),
    }
}

/// Installs the capture functions, the prelude and the installers, then runs the wrapped
/// source to completion and describes how it ended.
async fn evaluate<'js>(
    ctx: &Ctx<'js>,
    source: String,
    installers: Vec<Installer>,
    sink: Sink,
) -> Outcome {
    if let Err(e) = install(ctx, installers, sink) {
        return Outcome::Aborted {
            text: format!("the script host could not start: {e}"),
        };
    }
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some(SCRIPT_FILE.to_owned());
    let result = match ctx
        .eval_with_options::<Promise, _>(source, options)
        .catch(ctx)
    {
        Ok(promise) => promise.into_future::<Value>().await.catch(ctx).map(drop),
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => Outcome::Done,
        Err(e) => caught(ctx, e),
    }
}

/// Describes a caught error: a JavaScript throw with its text and line, or an engine failure.
fn caught<'js>(ctx: &Ctx<'js>, error: CaughtError<'js>) -> Outcome {
    match error {
        CaughtError::Exception(e) => {
            let error = e.as_object();
            // An unsupported member reads as its bare message, without the `Error: ` prefix.
            let unsupported = error.get("__apinataUnsupported").unwrap_or(false);
            let text = match unsupported {
                true => error.get("message").unwrap_or_default(),
                false => to_string(ctx, e.clone().into_value()),
            };
            Outcome::Threw {
                text,
                line: error_line(error),
                segment: running_segment(ctx),
                unsupported,
            }
        }
        // Error-like objects that are not `Error` instances, such as chai's `AssertionError`,
        // still carry a stack that locates the throw.
        CaughtError::Value(value) => Outcome::Threw {
            line: value.as_object().and_then(error_line),
            text: to_string(ctx, value),
            segment: running_segment(ctx),
            unsupported: false,
        },
        CaughtError::Error(e) => Outcome::Aborted {
            text: e.to_string(),
        },
    }
}

fn install<'js>(ctx: &Ctx<'js>, installers: Vec<Installer>, sink: Sink) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;
    let logs = Arc::clone(&sink);
    let log = move |level: String, text: String| lock(&logs).log(&level, text);
    host.set("log", Function::new(ctx.clone(), log)?)?;
    // The tests settled so far, as JSON for `bru.getTestResults`.
    let settled = Arc::clone(&sink);
    let results = move || -> String {
        let capture = lock(&settled);
        let rows = capture.run.tests.iter().map(|t| match &t.verdict {
            RawVerdict::Pass => serde_json::json!({"status": "pass", "description": t.name}),
            RawVerdict::Fail(m) => {
                serde_json::json!({"status": "fail", "description": t.name, "error": m})
            }
            RawVerdict::Skipped(m) => {
                serde_json::json!({"status": "skipped", "description": t.name, "error": m})
            }
        });
        serde_json::Value::Array(rows.collect()).to_string()
    };
    host.set("results", Function::new(ctx.clone(), results)?)?;
    let test = move |name: String, kind: String, message: String| {
        let verdict = match kind.as_str() {
            "pass" => RawVerdict::Pass,
            "fail" => RawVerdict::Fail(message),
            _ => RawVerdict::Skipped(message),
        };
        lock(&sink).run.tests.push(RawTest { name, verdict });
    };
    host.set("test", Function::new(ctx.clone(), test)?)?;
    ctx.globals().set("__host", host)?;
    ctx.eval::<(), _>(PRELUDE)?;
    installers.into_iter().try_for_each(|install| install(ctx))
}

/// JavaScript's `String(value)`, which is how errors and thrown values read in messages.
fn to_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> String {
    ctx.globals()
        .get::<_, Function>("String")
        .and_then(|string| string.call((value,)))
        .unwrap_or_default()
}

/// The line of the innermost user-code frame in the stack, or a syntax error's own line.
pub(crate) fn error_line(error: &Object<'_>) -> Option<u32> {
    let stack: Option<String> = error.get("stack").ok().flatten();
    let marker = format!("{SCRIPT_FILE}:");
    let from_stack = stack.as_deref().and_then(|stack| {
        let rest = stack.split(&marker).nth(1)?;
        rest.split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
    from_stack.or_else(|| error.get::<_, Option<u32>>("lineNumber").ok().flatten())
}

fn running_segment(ctx: &Ctx<'_>) -> usize {
    ctx.globals()
        .get::<_, Object>("__apinata")
        .and_then(|apinata| apinata.get("segment"))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CancelHandle;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn seg(level: &str, code: &str) -> Segment {
        Segment {
            level: level.to_owned(),
            code: code.to_owned(),
        }
    }
    fn limits(ms: u64, mib: usize) -> Limits {
        Limits {
            timeout: Duration::from_millis(ms),
            memory: mib << 20,
            console: 1_000,
            outer: None,
        }
    }
    async fn run(code: &str) -> PhaseRun {
        run_phase(
            vec![seg("request", code)],
            Vec::new(),
            limits(2_000, 64),
            CancelHandle::new(),
        )
        .await
    }
    fn texts(run: &PhaseRun) -> Vec<String> {
        run.console
            .iter()
            .map(|c| format!("{}:{}", c.level, c.text))
            .collect()
    }

    #[test]
    fn the_phase_future_is_send_and_static() {
        fn assert_send<T: Send + 'static>(_: &T) {}
        let phase = run_phase(
            vec![seg("request", "1")],
            Vec::new(),
            limits(1_000, 16),
            CancelHandle::new(),
        );
        assert_send(&phase);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn phases_run_concurrently_on_spawned_tasks() {
        let tasks: Vec<_> = (0..8)
            .map(|i| {
                let code = format!("await null; console.log({i})");
                tokio::spawn(run_phase(
                    vec![seg("request", &code)],
                    Vec::new(),
                    limits(2_000, 16),
                    CancelHandle::new(),
                ))
            })
            .collect();
        for (i, task) in tasks.into_iter().enumerate() {
            let run = task.await.expect("joins");
            assert_eq!(run.failure, None);
            assert_eq!(texts(&run), [format!("log:{i}")]);
        }
    }

    #[tokio::test]
    async fn segments_run_in_order_and_return_leaves_only_its_own() {
        let segments = vec![
            seg("collection", "console.log('a'); return; console.log('x')"),
            seg("request", "console.log('b')"),
        ];
        let run = run_phase(segments, Vec::new(), limits(2_000, 16), CancelHandle::new()).await;
        assert_eq!(run.failure, None);
        assert_eq!(texts(&run), ["log:a", "log:b"]);
    }

    #[tokio::test]
    async fn the_largest_timeout_neither_panics_nor_stops_a_trivial_script() {
        let limits = Limits {
            timeout: Duration::MAX,
            ..limits(1, 16)
        };
        let seg = vec![seg("request", "console.log('ok')")];
        let run = run_phase(seg, Vec::new(), limits, CancelHandle::new()).await;
        assert_eq!(texts(&run), ["log:ok"]);
        assert_eq!(run.failure, None);
    }

    #[tokio::test]
    async fn a_throw_ends_the_phase_and_names_the_level_and_line() {
        let segments = vec![
            seg("folder users", "console.log('a')\nthrow new Error('boom')"),
            seg("request", "console.log('never')"),
        ];
        let run = run_phase(segments, Vec::new(), limits(2_000, 16), CancelHandle::new()).await;
        assert_eq!(texts(&run), ["log:a"]);
        let message = "folder users script, line 2: Error: boom".to_owned();
        assert_eq!(run.failure, Some(Failure::Thrown { message }));
    }

    #[tokio::test]
    async fn a_syntax_error_names_the_level_and_line() {
        let segments = vec![seg("collection", "1"), seg("request", "let a = 1;\n)")];
        let run = run_phase(segments, Vec::new(), limits(2_000, 16), CancelHandle::new()).await;
        match run.failure {
            Some(Failure::Thrown { message }) => {
                assert!(
                    message.starts_with("request script, line 2: SyntaxError"),
                    "{message}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn scripts_run_in_sloppy_mode() {
        let run = run("undeclared = 5; console.log(undeclared, this === globalThis)").await;
        assert_eq!(run.failure, None);
        assert_eq!(texts(&run), ["log:5 true"]);
    }

    #[tokio::test]
    async fn throwing_null_is_a_throw_not_a_memory_limit() {
        let run = run("throw null").await;
        assert_eq!(
            run.failure,
            Some(Failure::Thrown {
                message: "request script: null".into()
            })
        );
    }

    #[tokio::test]
    async fn an_unsupported_member_is_classified() {
        let run = run("__apinata.unsupported('bru.sleep')").await;
        let message = "bru.sleep is not supported by Apinata".to_owned();
        assert_eq!(run.failure, Some(Failure::Unsupported { message }));
    }

    #[test]
    fn time_limits_read_in_seconds() {
        assert_eq!(
            time_limit_message(Duration::from_secs(30)),
            "script exceeded the 30 s limit"
        );
        assert_eq!(
            time_limit_message(Duration::from_millis(200)),
            "script exceeded the 0.2 s limit"
        );
    }

    /// `run_phase` on a private runtime and thread, failing instead of hanging if the limit misses.
    fn limited(code: &'static str) -> PhaseRun {
        crate::hang_guard::guarded(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            rt.block_on(run_phase(
                vec![seg("request", code)],
                Vec::new(),
                limits(200, 64),
                CancelHandle::new(),
            ))
        })
    }

    #[test]
    fn a_runaway_loop_stops_at_the_time_limit() {
        let code = "try { while (true) {} } catch (e) { console.log('caught') }";
        let run = limited(code);
        assert!(run.console.is_empty());
        let message = "script exceeded the 0.2 s limit".to_owned();
        assert_eq!(run.failure, Some(Failure::Limit { message }));
    }

    #[test]
    fn a_never_settling_await_stops_at_the_time_limit() {
        let run = limited("await new Promise(() => {})");
        let message = "script exceeded the 0.2 s limit".to_owned();
        assert_eq!(run.failure, Some(Failure::Limit { message }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_stops_a_runaway_or_waiting_phase() {
        for code in ["while (true) {}", "await new Promise(() => {})"] {
            let cancel = CancelHandle::new();
            let trigger = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                trigger.cancel();
            });
            // Held to half of a 10 s limit: an ignored cancel only stops at the full limit.
            let start = Instant::now();
            let out = run_phase(
                vec![seg("request", code)],
                Vec::new(),
                limits(10_000, 16),
                cancel,
            )
            .await;
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "{code}: {:?}",
                start.elapsed()
            );
            assert_eq!(out.failure, Some(Failure::Cancelled), "{code}");
        }
    }

    #[tokio::test]
    async fn a_throw_without_a_line_still_names_its_segment() {
        let segments = vec![seg("collection", "1"), seg("request", "throw null")];
        let out = run_phase(segments, Vec::new(), limits(2_000, 16), CancelHandle::new()).await;
        assert_eq!(
            out.failure,
            Some(Failure::Thrown {
                message: "request script: null".into()
            })
        );
    }

    #[tokio::test]
    async fn an_allocation_loop_stops_at_the_memory_limit() {
        let code = "const a = []; while (true) a.push(new Array(1000).fill(1.5));";
        let out = run_phase(
            vec![seg("request", code)],
            Vec::new(),
            limits(10_000, 16),
            CancelHandle::new(),
        )
        .await;
        let message = "script exceeded the 16 MiB memory limit".to_owned();
        assert_eq!(out.failure, Some(Failure::Limit { message }));
        assert_eq!(texts(&run("console.log('ok')").await), ["log:ok"]);
    }

    #[tokio::test]
    async fn console_levels_and_formatting() {
        let run = run(
            r#"console.log("a", 1, true, null, undefined, {b: [1, "x"]});
    console.debug(function named() {});
    console.info(new Error("e"), /r/g);
    console.warn(new Set([1, 2]), new Map([["k", 1]]));
    console.error(NaN, 10n);
    const o = {}; o.o = o; console.log(o);"#,
        )
        .await;
        assert_eq!(run.failure, None);
        assert_eq!(
            texts(&run),
            [
                r#"log:a 1 true null undefined {"b":[1,"x"]}"#,
                "debug:function named() {\n    [native code]\n}",
                "info:Error: e /r/g",
                r#"warn:Set [1,2] Map [["k",1]]"#,
                "error:NaN 10",
                "log:[object Object]",
            ]
        );
    }

    #[tokio::test]
    async fn console_keeps_its_budget_and_counts_the_rest() {
        let run = run("for (let i = 0; i < 1005; i++) console.log(i)").await;
        assert_eq!(run.console.len(), 1_000);
        assert_eq!(run.console[999].text, "999");
        assert_eq!(run.dropped, 5);
    }

    #[tokio::test]
    async fn test_records_pass_fail_and_skip() {
        let run = run(r#"test("ok", () => {});
    test("bad", () => { throw new Error("nope"); });
    test("async bad", async () => { await null; throw new TypeError("late"); });
    test("blank", () => { throw {}; });
    test("later", () => __apinata.unsupported("bru.sendRequest"));"#)
        .await;
        assert_eq!(run.failure, None);
        let mut tests = run.tests.clone();
        tests.sort_by(|a, b| a.name.cmp(&b.name));
        let t = |name: &str, verdict| RawTest {
            name: name.into(),
            verdict,
        };
        assert_eq!(
            tests,
            [
                t("async bad", RawVerdict::Fail("late".into())),
                t("bad", RawVerdict::Fail("nope".into())),
                t(
                    "blank",
                    RawVerdict::Fail("An unexpected error occurred.".into())
                ),
                t(
                    "later",
                    RawVerdict::Skipped("bru.sendRequest is not supported by Apinata".into())
                ),
                t("ok", RawVerdict::Pass),
            ]
        );
    }

    #[tokio::test]
    async fn the_sandbox_has_no_file_network_or_process_globals() {
        let run = run(r#"const names = ["process", "std", "os", "fetch", "XMLHttpRequest", "WebSocket", "Deno", "Bun", "navigator", "print", "scriptArgs", "importScripts"];
    console.log(names.filter((n) => typeof globalThis[n] !== "undefined").join(",") || "none");
    let r = "imported"; try { await import("/etc/hostname"); } catch (e) { r = "blocked"; } console.log(r);"#)
        .await;
        assert_eq!(texts(&run), ["log:none", "log:blocked"]);
    }

    #[tokio::test]
    async fn installers_add_host_functions() {
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&calls);
        let install: Installer = Box::new(move |ctx| {
            let f = rquickjs::Function::new(ctx.clone(), move |s: String| {
                sink.lock().expect("lock").push(s);
            })?;
            ctx.globals().set("probe", f)
        });
        let run = run_phase(
            vec![seg("request", "probe('hi')")],
            vec![install],
            limits(2_000, 16),
            CancelHandle::new(),
        )
        .await;
        assert_eq!(run.failure, None);
        assert_eq!(*calls.lock().expect("lock"), ["hi"]);
    }

    #[tokio::test]
    async fn a_panicking_host_function_is_a_script_error() {
        let install: Installer = Box::new(|ctx| {
            let f = rquickjs::Function::new(ctx.clone(), || -> i32 { panic!("host bug") })?;
            ctx.globals().set("boom", f)
        });
        let run = run_phase(
            vec![seg("request", "boom()")],
            vec![install],
            limits(2_000, 16),
            CancelHandle::new(),
        )
        .await;
        match run.failure {
            Some(Failure::Thrown { message }) => assert!(message.contains("panicked"), "{message}"),
            other => panic!("{other:?}"),
        }
    }
}
