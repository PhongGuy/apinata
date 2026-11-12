//! The run report: what each request did, serialized in the documented JSON shape, and its exit code.

use std::fmt::Write;

use crate::{
    AssertionOutcome, ConsoleEntry, RequestResult, RunError, TestOutcome, VarOutcome, VarResult,
    Verdict,
};
use serde::Serialize;

/// Version of the JSON report format.
const REPORT_VERSION: u32 = 1;

/// The whole run, in the key order of the JSON report.
#[derive(Serialize)]
pub(crate) struct Report {
    pub(crate) apinata: u32,
    pub(crate) load_errors: Vec<LoadErrorRow>,
    pub(crate) results: Vec<ResultRow>,
    pub(crate) summary: Summary,
}

/// A file under the selection that could not be loaded.
#[derive(Serialize)]
pub(crate) struct LoadErrorRow {
    pub path: String,
    pub message: String,
}

/// How one request ended.
#[derive(Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Outcome {
    Passed,
    Failed,
    Errored,
    Skipped,
}

fn is_one(n: &u32) -> bool {
    *n == 1
}

/// One request: `status` and `duration_ms` are `None` when nothing was sent.
#[derive(Serialize)]
pub(crate) struct ResultRow {
    /// The zero-based data row this result belongs to; absent when the run has no data file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_index: Option<usize>,
    pub path: String,
    pub name: String,
    pub outcome: Outcome,
    pub status: Option<u16>,
    /// Why a reply status failed the request under fail-on-status; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_failure: Option<String>,
    /// How many times the request was sent; absent when it was sent once.
    #[serde(skip_serializing_if = "is_one")]
    pub attempts: u32,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    /// The failure of the attempt before a cancel that came during the pause before a retry,
    /// redacted like `error`; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempt_error: Option<String>,
    /// The plain-words reading of a connect failure or an undefined variable; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// A connect failure as the one human line, hint included, in place of `error`.
    #[serde(skip)]
    pub error_line: Option<String>,
    /// Why the connection failed and where in `error_line` the sentence ends, so a front end
    /// can add advice there.
    #[serde(skip)]
    pub(crate) connect_advice_slot: Option<(crate::ConnectCause, usize)>,
    /// The same for `last_attempt_error`, which a connect failure words like `error_line`.
    #[serde(skip)]
    pub(crate) last_attempt_advice_slot: Option<(crate::ConnectCause, usize)>,
    /// How an event-stream ended; absent for an ordinary response.
    #[serde(rename = "streamEnd", skip_serializing_if = "Option::is_none")]
    pub stream_end: Option<&'static str>,
    pub warnings: Vec<String>,
    pub assertions: Vec<AssertionRow>,
    pub post_response_vars: Vec<VarRow>,
    pub tests: Vec<TestRow>,
    pub console: Vec<ConsoleRow>,
    /// Each cookie set for the request, in the order it was set; absent when none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cookies: Vec<CookieRow>,
    /// The response itself; present only when the caller asked for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponseRow>,
    #[serde(skip)]
    pub unset_var_lines: Vec<String>,
    #[serde(skip)]
    pub status_text: Option<String>,
    /// Core's verdict on whether every check was skipped, taken when the row is built.
    #[serde(skip)]
    pub unverified: bool,
    /// Core's verdict on whether the request passed with no check at all.
    #[serde(skip)]
    pub no_tests: bool,
}

/// One cookie a hop or a script set, without its value: `stored` when the jar kept it,
/// `removed` when it deleted a stored one, `reason` when the jar rejected it. Exactly one of
/// `hop` (the hop's label) and `script` says where it came from; a script's row names its
/// `phase`.
#[derive(Serialize)]
pub(crate) struct CookieRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub script: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<&'static str>,
    /// Absent for a value that never parsed, whose text could be the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub stored: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl CookieRow {
    /// The row for `c`, set from `origin`, with every text a secret could hide in redacted.
    fn new(origin: crate::CookieOrigin<'_>, c: &crate::SetCookie, result: &RequestResult) -> Self {
        use crate::{CookieOutcome, CookieRejection};
        let (stored, removed, reason) = match c.outcome {
            CookieOutcome::Stored => (true, false, None),
            CookieOutcome::Removed => (false, true, None),
            CookieOutcome::Rejected(why) => (false, false, Some(why.to_string())),
        };
        let unparsed = matches!(
            c.outcome,
            CookieOutcome::Rejected(CookieRejection::Unparseable | CookieRejection::NotUtf8)
        );
        CookieRow {
            hop: origin.hop().map(|h| result.redact(&h.label())),
            script: origin.hop().is_none(),
            phase: origin.phase().map(crate::ScriptPhase::as_str),
            name: (!unparsed).then(|| result.redact(&c.name)),
            domain: c.domain.as_deref().map(|d| result.redact(d)),
            path: c.path.as_deref().map(|p| result.redact(p)),
            stored,
            removed,
            reason,
        }
    }

    /// The cookie's name as text: `(unparsed)` for a value that never parsed.
    pub(crate) fn name_text(&self) -> &str {
        self.name.as_deref().unwrap_or("(unparsed)")
    }

    /// The jar's verdict as text: `Stored`, `Removed` or the rejection's reason.
    pub(crate) fn verdict_text(&self) -> &str {
        match (self.stored, self.removed, &self.reason) {
            (true, _, _) => "Stored",
            (_, true, _) => "Removed",
            (_, _, Some(reason)) => reason,
            _ => "Unknown",
        }
    }

    /// Where the cookie came from as text: the hop's label, or `<phase> script`.
    pub(crate) fn origin_text(&self) -> String {
        match (&self.hop, self.phase) {
            (Some(hop), _) => hop.clone(),
            (None, Some(phase)) => format!("{phase} script"),
            (None, None) => "script".to_owned(),
        }
    }

    /// The row as one text line, `set-cookie <name>: <verdict> (<from>)`, line breaks flattened
    /// so a hostile name cannot forge another line; never the value.
    pub(crate) fn line(&self) -> String {
        let (name, verdict, from) = (self.name_text(), self.verdict_text(), self.origin_text());
        format!("set-cookie {name}: {verdict} ({from})").replace(['\r', '\n'], " ")
    }
}

impl RequestResult {
    /// One line per cookie set for this request (see [`RequestResult::set_cookies`]):
    /// `set-cookie <name>: <verdict> (<from>)`, `from` being the hop's label or `<phase> script`, with
    /// every text a secret could hide in redacted and never the value.
    pub fn set_cookie_lines(&self) -> Vec<String> {
        (self.set_cookies())
            .map(|(origin, c)| CookieRow::new(origin, c, self).line())
            .collect()
    }
}

/// The response as the JSON report shows it, redacted as the session log redacts it.
#[derive(Serialize)]
pub(crate) struct ResponseRow {
    pub status: u16,
    pub headers: Vec<[String; 2]>,
    /// The body text, `null` when the bytes are not UTF-8.
    pub body: Option<String>,
    pub body_size: u64,
    pub truncated: bool,
    /// The answering peer as `ip:port`; absent when unknown. Through a proxy it is the proxy's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub via_proxy: bool,
}

impl ResponseRow {
    /// The row for `result`'s response, or `None` when no response arrived.
    pub(crate) fn of(result: &crate::RequestResult) -> Option<ResponseRow> {
        let r = result.response.as_ref()?;
        Some(ResponseRow {
            status: r.status(),
            headers: (r.headers().iter())
                .map(|(n, v)| [n.clone(), result.redact(&crate::mask_response_header(n, v))])
                .collect(),
            body: std::str::from_utf8(r.body()).ok().map(|t| result.redact(t)),
            body_size: r.size(),
            truncated: r.truncated(),
            remote_addr: r.remote_addr().map(|a| a.to_string()),
            via_proxy: r.via_proxy(),
        })
    }
}

/// One assertion and its verdict; `actual` is present only on a failure.
#[derive(Serialize)]
pub(crate) struct AssertionRow {
    pub expression: String,
    pub operator: String,
    pub value: Option<String>,
    pub verdict: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    pub detail: Option<String>,
    #[serde(skip)]
    pub description: Option<String>,
}

/// One script test or script-error entry; `message` is the failure or skip reason.
#[derive(Serialize)]
pub(crate) struct TestRow {
    pub phase: &'static str,
    pub name: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "scriptError", skip_serializing_if = "std::ops::Not::not")]
    pub script_error: bool,
}

/// One line a script logged.
#[derive(Serialize)]
pub(crate) struct ConsoleRow {
    pub phase: &'static str,
    pub level: &'static str,
    pub text: String,
}

/// One post-response variable and what happened to it; `reason` is present only on a skip or
/// a failure.
#[derive(Serialize)]
pub(crate) struct VarRow {
    pub name: String,
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Counts of request outcomes, plus the assertions that were skipped.
#[derive(Serialize)]
pub(crate) struct Summary {
    pub requests: usize,
    pub passed: usize,
    /// Passed requests whose every check was skipped; a subset of `passed`.
    pub unverified: usize,
    /// Passed requests with no check at all; a subset of `passed`.
    pub no_tests: usize,
    pub failed: usize,
    pub errored: usize,
    /// Requests never sent.
    pub skipped: usize,
    pub skipped_assertions: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
    pub skipped_tests: usize,
}

impl From<&AssertionOutcome> for AssertionRow {
    fn from(outcome: &AssertionOutcome) -> AssertionRow {
        let a = outcome.assertion();

        let (verdict, actual, detail) = match outcome.verdict() {
            Verdict::Pass => ("pass", None, None),
            Verdict::Fail { actual, detail, .. } => ("fail", Some(actual.clone()), detail.clone()),
            Verdict::Skipped { reason, .. } => ("skipped", None, Some(reason.clone())),
        };
        AssertionRow {
            expression: a.expression().to_owned(),
            operator: a.operator().to_owned(),
            value: a.value().map(str::to_owned),
            verdict,
            actual,
            detail,
            description: a.description().map(str::to_owned),
        }
    }
}

impl From<&TestOutcome> for TestRow {
    fn from(t: &TestOutcome) -> TestRow {
        let (status, message) = match &t.verdict {
            Verdict::Pass => ("pass", None),
            Verdict::Fail { detail, .. } => ("fail", detail.clone()),
            Verdict::Skipped { reason, .. } => ("skipped", Some(reason.clone())),
        };
        TestRow {
            phase: t.phase.as_str(),
            name: t.name.clone(),
            status,
            message,
            script_error: t.script_error,
        }
    }
}

impl AssertionRow {
    /// The row with its actual value and detail redacted by `result`; a server can echo a
    /// secret into the left side, while the expression and value are the file's own text.
    fn redacted(mut self, result: &crate::RequestResult) -> AssertionRow {
        self.actual = self.actual.map(|a| result.redact(&a));
        self.detail = self.detail.map(|d| result.redact(&d));
        self
    }
}

impl TestRow {
    /// The row with its name and message redacted by `result`.
    fn redacted(mut self, result: &crate::RequestResult) -> TestRow {
        self.name = result.redact(&self.name);
        self.message = self.message.map(|m| result.redact(&m));
        self
    }
}

impl From<&ConsoleEntry> for ConsoleRow {
    fn from(c: &ConsoleEntry) -> ConsoleRow {
        ConsoleRow {
            phase: c.phase.as_str(),
            level: c.level.as_str(),
            text: c.text.clone(),
        }
    }
}

impl From<&VarOutcome> for VarRow {
    fn from(outcome: &VarOutcome) -> VarRow {
        let (result, reason) = match outcome.result() {
            VarResult::Set => ("set", None),
            VarResult::Undefined => ("undefined", None),
            VarResult::Skipped { reason } => ("skipped", Some(reason.clone())),
            VarResult::Failed { error } => ("failed", Some(error.clone())),
        };
        VarRow {
            name: outcome.name().to_owned(),
            result,
            reason,
        }
    }
}

impl From<&crate::RequestResult> for ResultRow {
    fn from(result: &crate::RequestResult) -> ResultRow {
        use crate::Outcome as Core;
        // The CLI never cancels; an outcome it has no row for reads as errored.
        let outcome = match result.outcome {
            Core::Passed => Outcome::Passed,
            Core::Failed => Outcome::Failed,
            Core::Skipped => Outcome::Skipped,
            _ => Outcome::Errored,
        };
        let response = result.response.as_ref();
        ResultRow {
            iteration_index: result.iteration,
            path: result.path.as_str().to_owned(),
            name: result.name.clone(),
            outcome,
            status: response.map(|r| r.status()),
            status_failure: result.status_failure_reason(),
            attempts: result.attempts(),
            // Rounds half up like bruno-cli's Math.round, since the time is never negative.
            duration_ms: response
                .map(|r| (r.response_time().as_secs_f64() * 1000.0).round() as u64),
            // Script-derived text is redacted as the session log redacts it.
            error: result.error.as_ref().map(|e| result.redact(&e.to_string())),
            // Worded as `error_text` words a failure: the hinted line for a connect failure.
            last_attempt_error: result.last_attempt_error.as_ref().map(|e| {
                let text = match e.connect_hint() {
                    Some(_) => e.hinted_line(),
                    None => e.to_string(),
                };
                result.redact(&text)
            }),
            hint: (result.error.as_ref())
                .and_then(|e| e.connect_hint().map(|h| h.text()).or_else(|| e.cli_hint()))
                .map(|h| result.redact(&h)),
            error_line: (result.error.as_ref())
                .filter(|e| e.connect_hint().is_some())
                .map(|e| result.redact(&e.hinted_line())),
            connect_advice_slot: (result.error.as_ref()).and_then(|e| advice_slot(result, e)),
            last_attempt_advice_slot: (result.last_attempt_error.as_ref())
                .and_then(|e| advice_slot(result, e)),
            stream_end: response.and_then(|r| r.stream_end()).map(|e| e.label()),
            warnings: result.warnings.iter().map(ToString::to_string).collect(),
            assertions: (result.assertions.iter())
                .map(|a| AssertionRow::from(a).redacted(result))
                .collect(),
            post_response_vars: result.post_response_vars.iter().map(VarRow::from).collect(),
            tests: (result.tests.iter())
                .map(|t| TestRow::from(t).redacted(result))
                .collect(),
            console: (result.redacted_console().iter())
                .map(ConsoleRow::from)
                .collect(),
            cookies: (result.set_cookies())
                .map(|(origin, c)| CookieRow::new(origin, c, result))
                .collect(),
            response: None,
            unset_var_lines: result
                .post_response_vars
                .iter()
                .filter(|v| {
                    matches!(
                        v.result(),
                        VarResult::Skipped { .. } | VarResult::Failed { .. }
                    )
                })
                .map(ToString::to_string)
                .collect(),
            status_text: response.map(|r| r.status_text().to_owned()),
            unverified: result.is_unverified(),
            no_tests: result.has_no_checks(),
        }
    }
}

/// Where in `error`'s hinted line its hint's sentence ends and why the connection failed, or
/// `None` when it is no connect failure.
fn advice_slot(result: &RequestResult, error: &RunError) -> Option<(crate::ConnectCause, usize)> {
    // Redacting the sentence on its own must agree with redacting the whole line.
    let prefix = result.redact(&error.hinted_prefix()?);
    let line = result.redact(&error.hinted_line());
    line.starts_with(&prefix)
        .then(|| error.connect_cause().map(|c| (c, prefix.len())))
        .flatten()
}

impl Report {
    /// Put `advice(cause)` into each connect failure's line, right after the hint's sentence.
    pub(crate) fn add_connect_advice(
        &mut self,
        advice: impl Fn(crate::ConnectCause) -> Option<String>,
    ) {
        let insert = |slot: Option<(crate::ConnectCause, usize)>, line: Option<&mut String>| {
            let (Some((cause, at)), Some(line)) = (slot, line) else {
                return;
            };
            if let Some(text) = advice(cause).filter(|_| line.is_char_boundary(at)) {
                line.insert_str(at, &format!(" {text}"));
            }
        };
        for row in &mut self.results {
            insert(row.connect_advice_slot.take(), row.error_line.as_mut());
            insert(
                row.last_attempt_advice_slot.take(),
                row.last_attempt_error.as_mut(),
            );
        }
    }

    /// Build the report and count its outcomes.
    pub(crate) fn new(load_errors: Vec<LoadErrorRow>, results: Vec<ResultRow>) -> Report {
        let count = |o: Outcome| results.iter().filter(|r| r.outcome == o).count();
        // A script-error entry fails its request but is not a test, so the counts leave it out.
        let tests = |status: &str| {
            (results.iter().flat_map(|r| &r.tests))
                .filter(|t| !t.script_error && t.status == status)
                .count()
        };
        let summary = Summary {
            requests: results.len(),
            passed: count(Outcome::Passed),
            unverified: results.iter().filter(|r| r.unverified).count(),
            no_tests: results.iter().filter(|r| r.no_tests).count(),
            failed: count(Outcome::Failed),
            errored: count(Outcome::Errored),
            skipped: count(Outcome::Skipped),
            skipped_assertions: results
                .iter()
                .flat_map(|r| &r.assertions)
                .filter(|a| a.verdict == "skipped")
                .count(),
            passed_tests: tests("pass"),
            failed_tests: tests("fail"),
            skipped_tests: tests("skipped"),
        };
        Report {
            apinata: REPORT_VERSION,
            load_errors,
            results,
            summary,
        }
    }

    /// 3 if a file did not load, else 2 if a request was not sent, else 1 if one failed, else 0.
    pub(crate) fn exit_code(&self) -> u8 {
        if !self.load_errors.is_empty() {
            3
        } else if self.summary.errored > 0 {
            2
        } else if self.summary.failed > 0 {
            1
        } else {
            0
        }
    }

    /// The human report: one line per load error and request, its warnings and assertions
    /// indented beneath, then the totals.
    pub(crate) fn to_text(&self) -> String {
        let mut out = String::new();
        for e in &self.load_errors {
            let _ = writeln!(out, "{} (load error: {})", e.path, e.message);
        }
        let mut iteration = None;
        for r in &self.results {
            if let Some(label) = r.iteration_label(&mut iteration) {
                let _ = writeln!(out, "{label}");
            }
            let _ = writeln!(out, "{}", r.headline());
            if let Some(reason) = &r.status_failure {
                let _ = writeln!(out, "   {reason}");
            }
            // A connect failure's hint is already inside its error line.
            if let (Some(hint), None) = (&r.hint, &r.error_line) {
                let _ = writeln!(out, "   hint: {hint}");
            }
            for w in &r.warnings {
                let _ = writeln!(out, "   ! {w}");
            }
            for line in &r.unset_var_lines {
                let _ = writeln!(out, "   ! {line}");
            }
            for a in &r.assertions {
                let _ = writeln!(out, "   {}", a.line());
            }
            for t in &r.tests {
                let _ = writeln!(out, "   {}", t.line());
            }
            for c in &r.console {
                let _ = writeln!(out, "{}", c.lines());
            }
        }
        let s = &self.summary;
        let assertions = self.results.iter().flat_map(|r| &r.assertions);
        let passed = assertions.clone().filter(|a| a.verdict == "pass").count();
        let failed = assertions.filter(|a| a.verdict == "fail").count();
        let _ = writeln!(out, "\n{}", s.requests_line());
        let _ = writeln!(
            out,
            "Assertions: {passed} passed, {failed} failed, {} skipped",
            s.skipped_assertions
        );
        let _ = writeln!(
            out,
            "Tests: {} passed, {} failed, {} skipped",
            s.passed_tests, s.failed_tests, s.skipped_tests
        );
        out
    }
}

impl Summary {
    /// `Requests: N (P passed (U unverified), F failed, T without tests, E errored, S not
    /// sent)`, leaving out a zero unverified or without-tests part. A request with no check is
    /// counted apart from the passed ones here, though the JSON summary keeps it inside `passed`.
    pub(crate) fn requests_line(&self) -> String {
        let unverified = if self.unverified > 0 {
            format!(" ({} unverified)", self.unverified)
        } else {
            String::new()
        };
        let untested = if self.no_tests > 0 {
            format!(", {} without tests", self.no_tests)
        } else {
            String::new()
        };
        format!(
            "Requests: {} ({} passed{unverified}, {} failed{untested}, {} errored, {} not sent)",
            self.requests,
            self.passed.saturating_sub(self.no_tests),
            self.failed,
            self.errored,
            self.skipped
        )
    }
}

impl ResultRow {
    /// `Iteration N` (one-based) when this result starts a new data row after `previous`,
    /// which it then advances; `None` for a run without a data file or within the same row.
    pub(crate) fn iteration_label(&self, previous: &mut Option<usize>) -> Option<String> {
        let index = self.iteration_index?;
        if *previous == Some(index) {
            return None;
        }
        *previous = Some(index);
        Some(format!("Iteration {}", index + 1))
    }

    /// The line shown for a failure: the hinted connect line when there is one, else `error`.
    pub(crate) fn error_text(&self) -> Option<&str> {
        self.error_line.as_deref().or(self.error.as_deref())
    }

    /// bruno-cli's per-request line: the path without `.yml`, then the status or why none.
    fn headline(&self) -> String {
        let line = self.outcome_line();
        match self.attempts {
            0 | 1 => line,
            n => format!("{line} ({n} attempts)"),
        }
    }

    fn outcome_line(&self) -> String {
        let name = self.path.strip_suffix(".yml").unwrap_or(&self.path);
        match (self.outcome, self.status, &self.error) {
            (Outcome::Skipped, ..) => format!("{name} (skipped by --bail)"),
            (_, _, Some(error)) => {
                format!("{name} ({})", self.error_line.as_ref().unwrap_or(error))
            }
            (_, status, _) => {
                // A status without a reason phrase, such as 599, reads `(599)` with no trailing space.
                let status = match (status, self.status_text.as_deref().unwrap_or("")) {
                    (Some(code), "") => code.to_string(),
                    (Some(code), text) => format!("{code} {text}"),
                    (None, text) => text.to_owned(),
                };
                format!("{name} ({status}) - {} ms", self.duration_ms.unwrap_or(0))
            }
        }
    }
}

impl AssertionRow {
    /// A `✓`, `✕` or `-` mark and the label, with the actual value or skip reason after it.
    fn line(&self) -> String {
        let label = match (&self.description, &self.value) {
            (Some(d), _) => d.clone(),
            (None, Some(v)) => format!("{}: {} {v}", self.expression, self.operator),
            (None, None) => format!("{}: {}", self.expression, self.operator),
        };
        match (self.verdict, &self.actual, &self.detail) {
            ("pass", ..) => format!("✓ {label}"),
            ("fail", actual, detail) => {
                let actual = actual.as_deref().unwrap_or("");
                match detail {
                    Some(d) => format!("✕ {label} (actual {actual}; {d})"),
                    None => format!("✕ {label} (actual {actual})"),
                }
            }
            (_, _, Some(reason)) => format!("- {label} (skipped: {reason})"),
            _ => format!("- {label} (skipped)"),
        }
    }
}

impl TestRow {
    /// A `✓`, `✕` or `-` mark and the name, with the failure message or skip reason after it.
    fn line(&self) -> String {
        match (self.status, &self.message) {
            ("pass", _) => format!("✓ {}", self.name),
            ("fail", Some(m)) => format!("✕ {} ({m})", self.name),
            ("fail", None) => format!("✕ {}", self.name),
            (_, Some(m)) => format!("- {} (skipped: {m})", self.name),
            _ => format!("- {} (skipped)", self.name),
        }
    }
}

impl ConsoleRow {
    /// `   [level] text`, each further line of the text indented under the text itself.
    fn lines(&self) -> String {
        let indent = " ".repeat("   [] ".len() + self.level.len());
        let text = self.text.replace('\n', &format!("\n{indent}"));
        format!("   [{}] {text}", self.level)
    }
}

/// A report exercising every outcome, verdict and escaping case the renderers handle.
#[cfg(test)]
pub(crate) fn sample() -> Report {
    fn result(path: &str, name: &str, outcome: Outcome) -> ResultRow {
        ResultRow {
            iteration_index: None,
            path: path.into(),
            name: name.into(),
            outcome,
            status: None,
            status_failure: None,
            attempts: 1,
            duration_ms: None,
            error: None,
            last_attempt_error: None,
            hint: None,
            error_line: None,
            connect_advice_slot: None,
            last_attempt_advice_slot: None,
            stream_end: None,
            warnings: vec![],
            assertions: vec![],
            post_response_vars: vec![],
            tests: vec![],
            console: vec![],
            cookies: vec![],
            response: None,
            unset_var_lines: vec![],
            status_text: None,
            unverified: false,
            no_tests: false,
        }
    }
    fn assertion(
        op: &str,
        value: Option<&str>,
        verdict: &'static str,
        detail: Option<&str>,
    ) -> AssertionRow {
        AssertionRow {
            expression: if value.is_some() {
                "res.status".into()
            } else {
                "res.body".into()
            },
            operator: op.into(),
            value: value.map(Into::into),
            verdict,
            actual: (verdict == "fail").then(|| "201".into()),
            detail: detail.map(Into::into),
            description: None,
        }
    }
    fn test(
        phase: &'static str,
        name: &str,
        status: &'static str,
        message: Option<&str>,
    ) -> TestRow {
        TestRow {
            phase,
            name: name.into(),
            status,
            message: message.map(Into::into),
            script_error: false,
        }
    }
    let mut list = result("users/list.yml", "List users", Outcome::Passed);
    list.status = Some(200);
    list.status_text = Some("OK".into());
    list.duration_ms = Some(120);
    list.assertions = vec![
        assertion("eq", Some("200"), "pass", None),
        assertion("isJson", None, "skipped", Some("not supported")),
    ];
    list.tests = vec![
        test("pre-request", "sets token", "pass", None),
        test("post-response", "stores id", "pass", None),
        test("tests", "has users", "pass", None),
    ];
    list.console = vec![ConsoleRow {
        phase: "tests",
        level: "log",
        text: "3 users".into(),
    }];
    list.post_response_vars = vec![VarRow {
        name: "userId".into(),
        result: "set",
        reason: None,
    }];
    let mut create = result("users/create.yml", "Create \"user\"", Outcome::Failed);
    create.status = Some(201);
    create.status_text = Some("Created".into());
    create.duration_ms = Some(1500);
    create.warnings = vec!["header <x> ignored".into()];
    create.assertions = vec![assertion(
        "eq",
        Some("200"),
        "fail",
        Some("expected 201 to equal 200"),
    )];
    create.tests = vec![test(
        "tests",
        "id is set",
        "fail",
        Some("expected 1 to equal 2\nat line 3"),
    )];
    let mut get = result("orders/get.yml", "Get order", Outcome::Errored);
    get.error = Some("undefined variable `host`".into());
    let delete = result("orders/delete.yml", "Delete order", Outcome::Skipped);
    Report::new(
        vec![LoadErrorRow {
            path: "broken.yml".into(),
            message: "bad yaml & <stuff>".into(),
        }],
        vec![list, create, get, delete],
    )
}

#[cfg(test)]
mod hint_tests {
    use super::*;

    #[test]
    fn a_hint_prints_under_the_row_and_serializes_only_when_present() {
        let mut report = sample();
        let errored = report
            .results
            .iter_mut()
            .find(|r| r.error.is_some())
            .unwrap();
        errored.hint = Some("base is defined in environment dev; pass --env dev".into());
        let text = report.to_text();
        assert!(
            text.contains("(undefined variable `host`)\n   hint: base is defined in environment dev; pass --env dev\n"),
            "{text}"
        );
        let json = serde_json::to_value(&report.results).unwrap();
        let hints: Vec<_> = json
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r.get("hint"))
            .collect();
        assert_eq!(
            hints,
            ["base is defined in environment dev; pass --env dev"]
        );
    }

    /// A report whose errored row is a connect failure: the hint is also inside its error line.
    fn connect_failure_report() -> Report {
        let mut report = sample();
        let errored = (report.results.iter_mut())
            .find(|r| r.error.is_some())
            .unwrap();
        errored.hint = Some("Nothing is listening at 127.0.0.1:1. Is the server running?".into());
        errored.error_line = Some(
            "http://127.0.0.1:1/x: Connection refused: Nothing is listening at 127.0.0.1:1. Is the server running? (tcp connect error)"
                .into(),
        );
        report
    }

    #[test]
    fn a_connect_failure_shows_its_hint_once_in_the_text_report() {
        let text = connect_failure_report().to_text();
        let hint = "Nothing is listening at 127.0.0.1:1. Is the server running?";
        assert_eq!(text.matches(hint).count(), 1, "{text}");
        assert!(!text.contains("   hint: Nothing"), "{text}");
    }

    #[test]
    fn a_connect_failure_keeps_its_hint_in_the_json_report() {
        let json = serde_json::to_value(&connect_failure_report().results).unwrap();
        let hints: Vec<_> = (json.as_array().unwrap().iter())
            .filter_map(|r| r.get("hint"))
            .collect();
        assert_eq!(
            hints,
            ["Nothing is listening at 127.0.0.1:1. Is the server running?"]
        );
    }
}
