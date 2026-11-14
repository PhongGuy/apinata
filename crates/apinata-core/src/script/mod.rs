//! Running collection, folder and request scripts in an embedded QuickJS sandbox.

mod bru;
mod chai;
pub(crate) mod compile;
mod cookies;
mod crypto;
mod crypto_der;
mod crypto_keys;
mod crypto_pss;
pub(crate) mod edit;
mod engine;
pub(crate) mod expr;
pub(crate) mod host;
mod host_libs;
mod jwt;
mod libs;
mod modules;
mod oauth2;
mod pem;
mod req_res;
mod run_request;
pub(crate) mod segments;
mod send;
mod texts;
mod timers;
mod web;

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use crate::CancelHandle;
use crate::assertion::Verdict;
use crate::script_trust::script_hash;
pub(crate) use engine::Segment;
use engine::{Failure, RawVerdict};
use host::HostState;
pub use texts::{
    ScriptText, ScriptTextKind, plan_script_texts, plan_script_texts_across, script_texts,
};

/// When a script runs: before the send, after the response, or as tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptPhase {
    /// Before the request is interpolated and sent.
    PreRequest,
    /// After the response arrives, before assertions.
    PostResponse,
    /// After assertions.
    Tests,
}

impl ScriptPhase {
    /// The phase's name: `pre-request`, `post-response` or `tests`.
    pub fn as_str(self) -> &'static str {
        match self {
            ScriptPhase::PreRequest => "pre-request",
            ScriptPhase::PostResponse => "post-response",
            ScriptPhase::Tests => "tests",
        }
    }

    /// The `type` of this phase's entries in a file's `scripts` list.
    pub(crate) fn script_type(self) -> &'static str {
        match self {
            ScriptPhase::PreRequest => "before-request",
            ScriptPhase::PostResponse => "after-response",
            ScriptPhase::Tests => "tests",
        }
    }
}

/// The `console` method a script called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConsoleLevel {
    /// `console.log`.
    Log,
    /// `console.debug`.
    Debug,
    /// `console.info`.
    Info,
    /// `console.warn`.
    Warn,
    /// `console.error`.
    Error,
}

impl ConsoleLevel {
    /// The method's name: `log`, `debug`, `info`, `warn` or `error`.
    pub fn as_str(self) -> &'static str {
        match self {
            ConsoleLevel::Log => "log",
            ConsoleLevel::Debug => "debug",
            ConsoleLevel::Info => "info",
            ConsoleLevel::Warn => "warn",
            ConsoleLevel::Error => "error",
        }
    }
}

/// One line a script logged, formatted as the console would print it.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct ConsoleEntry {
    /// The phase that logged it.
    pub phase: ScriptPhase,
    /// The method called.
    pub level: ConsoleLevel,
    /// The formatted arguments, joined by one space.
    pub text: String,
}

impl ConsoleEntry {
    /// A line `level` logged in `phase`.
    pub fn new(phase: ScriptPhase, level: ConsoleLevel, text: impl Into<String>) -> ConsoleEntry {
        ConsoleEntry {
            phase,
            level,
            text: text.into(),
        }
    }
}

// A script may log a secret, so Debug shows the text's length only.
impl fmt::Debug for ConsoleEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleEntry")
            .field("phase", &self.phase)
            .field("level", &self.level)
            .field("text_len", &self.text.len())
            .finish()
    }
}

/// One `test()` a script ran, or one entry standing for a script that failed or could not run.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct TestOutcome {
    /// The phase it ran in.
    pub phase: ScriptPhase,
    /// The test's name, or the entry's label such as `request tests`.
    pub name: String,
    /// How it ended.
    pub verdict: Verdict,
    /// Whether it stands for a script that threw or hit a limit; such entries fail the request
    /// but are not counted as tests.
    pub script_error: bool,
}

impl TestOutcome {
    /// A test entry; `script_error` false.
    pub fn new(phase: ScriptPhase, name: impl Into<String>, verdict: Verdict) -> TestOutcome {
        TestOutcome {
            phase,
            name: name.into(),
            verdict,
            script_error: false,
        }
    }

    pub(crate) fn skipped(phase: ScriptPhase, name: String, reason: &str) -> TestOutcome {
        TestOutcome::new(phase, name, Verdict::skipped(reason))
    }
}

// A test's name and messages may quote what the script read, so Debug shows lengths and kinds.
impl fmt::Debug for TestOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.verdict {
            Verdict::Pass => "pass",
            Verdict::Fail { .. } => "fail",
            Verdict::Skipped { .. } => "skipped",
        };
        f.debug_struct("TestOutcome")
            .field("phase", &self.phase)
            .field("name_len", &self.name.len())
            .field("verdict", &verdict)
            .field("script_error", &self.script_error)
            .finish()
    }
}

/// Which caller is sending, as `req.getExecutionMode()` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ExecutionMode {
    /// A single send outside a run.
    Standalone,
    /// One request of a run.
    #[default]
    Runner,
    /// A run from the command line; behaves as `Runner`.
    Cli,
}

/// Whether scripts run, and the bounds each phase runs under.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptOptions {
    enabled: bool,
    timeout: Duration,
    memory_limit: u64,
    /// The hashes of the texts allowed to run; `None` runs every text.
    approved: Option<BTreeSet<String>>,
}

impl Default for ScriptOptions {
    fn default() -> ScriptOptions {
        ScriptOptions {
            enabled: false,
            timeout: Duration::from_secs(30),
            memory_limit: 512 * 1024 * 1024,
            approved: None,
        }
    }
}

impl ScriptOptions {
    /// Scripts off, with a 30 s time limit and a 512 MiB memory limit per phase.
    pub fn new() -> ScriptOptions {
        ScriptOptions::default()
    }

    /// Whether scripts run.
    pub fn enabled(mut self, on: bool) -> ScriptOptions {
        self.enabled = on;
        self
    }

    /// The wall time one phase may take, awaited sends and sleeps included.
    pub fn timeout(mut self, t: Duration) -> ScriptOptions {
        self.timeout = t;
        self
    }

    /// The memory one phase's runtime may allocate, in bytes.
    pub fn memory_limit(mut self, bytes: u64) -> ScriptOptions {
        self.memory_limit = bytes;
        self
    }

    /// Run only segments and expressions whose [`script_hash`] is in `hashes`.
    pub fn only_approved(mut self, hashes: BTreeSet<String>) -> ScriptOptions {
        self.approved = Some(hashes);
        self
    }

    /// Whether `text` may run: always when no approved set is configured.
    pub(crate) fn allows(&self, text: &str) -> bool {
        self.approved
            .as_ref()
            .is_none_or(|hashes| hashes.contains(&script_hash(text)))
    }

    /// Whether an approved set is configured.
    pub(crate) fn restricted(&self) -> bool {
        self.approved.is_some()
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The engine's limits for a phase that may log `console` more entries.
    pub(crate) fn limits(&self, console: usize) -> engine::Limits {
        engine::Limits {
            timeout: self.timeout,
            memory: usize::try_from(self.memory_limit).unwrap_or(usize::MAX),
            console,
            outer: None,
        }
    }
}

/// The console entries one request keeps across its phases.
const CONSOLE_CAP: usize = 1_000;

/// What scripts asked of the runner: skip this request, end the run, or jump. `next` is
/// `Some(None)` for `setNextRequest(null)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunnerControl {
    pub(crate) skip: bool,
    pub(crate) stop: bool,
    pub(crate) next: Option<Option<String>>,
}

/// How a phase ended, as far as the send is concerned.
pub(crate) enum PhaseEnd {
    /// It ran to the end, or its failure was recorded as an entry.
    Ran,
    /// The pre-request phase failed with this message, so nothing may be sent.
    Failed(String),
    Cancelled,
}

/// What one request's phases recorded, gathered across them under one console cap.
#[derive(Default)]
pub(crate) struct Record {
    pub(crate) tests: Vec<TestOutcome>,
    pub(crate) console: Vec<ConsoleEntry>,
    pub(crate) sensitive: Vec<String>,
    /// The sensitive values resolution substituted into the request, OS environment included.
    pub(crate) substituted: Vec<String>,
    /// The send's shared state, kept so the runner can read what scripts asked of it.
    pub(crate) state: Option<Arc<HostState>>,
    dropped: Option<(ScriptPhase, usize)>,
}

impl Record {
    /// Runs `phase`'s scripts for the send `state` describes and records what they produced.
    /// A failure in a later phase becomes an entry; one in the pre-request phase is returned.
    pub(crate) async fn run(
        &mut self,
        state: &Arc<HostState>,
        phase: ScriptPhase,
        opts: &ScriptOptions,
        cancel: &CancelHandle,
    ) -> PhaseEnd {
        let segments = segments::segments(&state.request, &state.ctx, phase);
        // A phase without scripts starts no runtime.
        if segments.is_empty() {
            return PhaseEnd::Ran;
        }
        let label = match phase {
            ScriptPhase::PostResponse => "Post-response script",
            _ => "Test script",
        };
        // Fail closed: one unapproved segment keeps the whole phase from running, since later
        // segments may depend on what it would have set.
        if let Some(denied) = segments.iter().find(|s| !opts.allows(&s.code)) {
            if phase == ScriptPhase::PreRequest {
                return PhaseEnd::Failed(format!(
                    "{} script is not approved; approve it in the Apinata app",
                    denied.level
                ));
            }
            let entry = TestOutcome::skipped(phase, label.to_owned(), "the script is not approved");
            self.tests.push(entry);
            return PhaseEnd::Ran;
        }
        // The phase ends by its own limit or its caller's deadline, and so does any run it nests.
        let mut limits = opts.limits(CONSOLE_CAP.saturating_sub(self.console.len()));
        limits.outer = engine::earliest(
            Instant::now().checked_add(limits.timeout),
            state.nesting.deadline(),
        );
        let installers = host::installers(state, phase, limits.outer);
        let run = engine::run_phase(segments, installers, limits, cancel.clone()).await;
        self.sensitive = (state.sensitive.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        state.ctx.runtime_vars().taint(&self.sensitive);
        self.console
            .extend(run.console.into_iter().map(|c| ConsoleEntry {
                phase,
                level: level(c.level),
                text: c.text,
            }));
        if run.dropped > 0 {
            let before = self.dropped.map_or(0, |(_, n)| n);
            self.dropped = Some((phase, before + run.dropped));
        }
        self.tests
            .extend(run.tests.into_iter().map(|t| TestOutcome {
                phase,
                name: t.name,
                verdict: match t.verdict {
                    RawVerdict::Pass => Verdict::Pass,
                    RawVerdict::Fail(m) => Verdict::fail("", Some(m)),
                    RawVerdict::Skipped(m) => Verdict::skipped(m),
                },
                script_error: false,
            }));
        // A thrown error or a hit limit also reaches the console as an error of its phase.
        if let Some(Failure::Thrown { message } | Failure::Limit { message }) = &run.failure {
            self.console.push(ConsoleEntry {
                phase,
                level: ConsoleLevel::Error,
                text: message.clone(),
            });
        }
        match run.failure {
            None => PhaseEnd::Ran,
            Some(Failure::Cancelled) => PhaseEnd::Cancelled,
            Some(
                Failure::Thrown { message }
                | Failure::Unsupported { message }
                | Failure::Limit { message },
            ) if phase == ScriptPhase::PreRequest => PhaseEnd::Failed(message),
            Some(Failure::Unsupported { message }) => {
                let entry = TestOutcome::skipped(phase, label.to_owned(), &message);
                self.tests.push(entry);
                PhaseEnd::Ran
            }
            Some(Failure::Thrown { message } | Failure::Limit { message }) => {
                self.tests.push(TestOutcome {
                    phase,
                    name: format!("{label} error"),
                    verdict: Verdict::fail("", Some(message)),
                    script_error: true,
                });
                PhaseEnd::Ran
            }
        }
    }

    /// The console entries, ending with one `warn` entry counting those past the cap.
    pub(crate) fn console(&mut self) -> Vec<ConsoleEntry> {
        let mut console = std::mem::take(&mut self.console);
        if let Some((phase, n)) = self.dropped.take() {
            console.push(ConsoleEntry {
                phase,
                level: ConsoleLevel::Warn,
                text: format!("{n} more console entries were dropped"),
            });
        }
        console
    }
}

fn level(name: &str) -> ConsoleLevel {
    match name {
        "debug" => ConsoleLevel::Debug,
        "info" => ConsoleLevel::Info,
        "warn" => ConsoleLevel::Warn,
        "error" => ConsoleLevel::Error,
        _ => ConsoleLevel::Log,
    }
}
