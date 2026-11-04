//! The JUnit XML run report, in the shape CI dashboards already parse.

use std::borrow::Cow;
use std::fmt::Write;

use crate::results_report::report::{Outcome, Report, ResultRow};

/// One `<testcase>`; `child` is its rendered `<failure>`, `<error>` or `<skipped>` element.
struct Case {
    name: String,
    status: &'static str,
    child: Option<String>,
}

/// One `<testsuite>` before rendering; `skip_message` marks a skipped request.
struct Suite<'a> {
    name: Cow<'a, str>,
    file: &'a str,
    errors: usize,
    seconds: f64,
    cases: Vec<Case>,
    skip_message: Option<&'static str>,
    /// How many times the request was sent.
    attempts: u32,
    /// The failure a cancel in the pause before a retry would otherwise hide.
    last_attempt_error: Option<&'a str>,
    /// One `set-cookie` line per cookie the request's hops and scripts set.
    cookies: Vec<String>,
}

impl Suite<'_> {
    /// Whether the suite has a `<system-out>` note to write.
    fn has_note(&self) -> bool {
        self.attempts > 1 || self.last_attempt_error.is_some() || !self.cookies.is_empty()
    }
}

/// Render `report` as JUnit XML in bruno-cli's shape; `bail` picks the skipped-request message.
pub(crate) fn render(report: &Report, timestamp: &str, hostname: &str, bail: bool) -> String {
    let load_errors = report.load_errors.iter().map(|e| Suite {
        name: Cow::Borrowed(&e.path),
        file: &e.path,
        errors: 1,
        seconds: 0.0,
        cases: vec![error_case(&e.message)],
        skip_message: None,
        attempts: 1,
        last_attempt_error: None,
        cookies: Vec::new(),
    });
    let skip_message = if bail {
        "Request skipped due to bail"
    } else {
        "Request Skipped"
    };
    let requests = report
        .results
        .iter()
        .map(|r| request_suite(r, skip_message));
    let suites: Vec<Suite> = load_errors.chain(requests).collect();

    let mut out = String::from("<?xml version=\"1.0\"?>\n");
    if suites.is_empty() {
        out.push_str("<testsuites/>");
        return out;
    }
    out.push_str("<testsuites>\n");
    for suite in &suites {
        write_suite(&mut out, suite, timestamp, hostname);
    }
    out.push_str("</testsuites>");
    out
}

/// A skipped request keeps its testcases and gains a suite-level skip; an errored one has its
/// testcases replaced by a single error case, matching the reference reporter's precedence.
fn request_suite<'a>(r: &'a ResultRow, skip_message: &'static str) -> Suite<'a> {
    let seconds = r.duration_ms.unwrap_or(0) as f64 / 1000.0;
    // A data-file run names each suite after its one-based row so the rows stay apart.
    let name = match r.iteration_index {
        Some(i) => Cow::Owned(format!("{} (iteration {})", r.name, i + 1)),
        None => Cow::Borrowed(r.name.as_str()),
    };
    let mut suite = Suite {
        name,
        file: &r.path,
        errors: 0,
        seconds,
        cases: vec![],
        skip_message: None,
        attempts: r.attempts,
        last_attempt_error: r.last_attempt_error.as_deref(),
        cookies: r.cookies.iter().map(|c| c.line()).collect(),
    };
    if r.outcome == Outcome::Errored {
        suite.errors = 1;
        suite.cases = vec![error_case(r.error_text().unwrap_or_default())];
        return suite;
    }
    for a in &r.assertions {
        let name = match &a.value {
            Some(v) => format!("{} {} {v}", a.expression, a.operator),
            None => format!("{} {}", a.expression, a.operator),
        };
        let message = match (&a.detail, &a.actual) {
            (Some(d), _) => d.clone(),
            (None, Some(actual)) => format!("actual: {actual}"),
            (None, None) => String::new(),
        };
        suite.cases.push(verdict_case(name, a.verdict, &message));
    }
    // Script tests run pre-request, then tests, then post-response; a stable sort keeps file
    // order within each phase.
    let mut tests: Vec<_> = r.tests.iter().collect();
    tests.sort_by_key(|t| match t.phase {
        "pre-request" => 0,
        "tests" => 1,
        _ => 2,
    });
    for t in tests {
        let message = t.message.as_deref().unwrap_or_default();
        suite
            .cases
            .push(verdict_case(t.name.clone(), t.status, message));
    }
    if let Some(reason) = &r.status_failure {
        suite
            .cases
            .push(verdict_case("[status]".into(), "fail", reason));
    }
    if r.outcome == Outcome::Skipped {
        suite.skip_message = Some(skip_message);
    }
    suite
}

fn error_case(message: &str) -> Case {
    Case {
        name: "Test suite has no errors".into(),
        status: "fail",
        child: Some(format!(
            r#"<error type="error" message="{}"/>"#,
            att_escape(message)
        )),
    }
}

fn verdict_case(name: String, status: &'static str, message: &str) -> Case {
    let child = match status {
        "fail" => Some(format!(
            r#"<failure type="failure" message="{}"/>"#,
            att_escape(message)
        )),
        "skipped" => Some(format!(r#"<skipped message="{}"/>"#, att_escape(message))),
        _ => None,
    };
    Case {
        name,
        status,
        child,
    }
}

fn write_suite(out: &mut String, suite: &Suite, timestamp: &str, hostname: &str) {
    let count = |status: &str| suite.cases.iter().filter(|c| c.status == status).count();
    let failures = if suite.errors > 0 { 0 } else { count("fail") };
    let skipped = count("skipped") + usize::from(suite.skip_message.is_some());
    let classname = att_escape(strip_extension(suite.file));
    let _ = write!(
        out,
        r#"  <testsuite name="{}" file="{}" errors="{}" failures="{failures}" skipped="{skipped}" tests="{}" timestamp="{}" hostname="{}" time="{:.3}""#,
        att_escape(&suite.name),
        att_escape(suite.file),
        suite.errors,
        suite.cases.len(),
        att_escape(timestamp),
        att_escape(hostname),
        suite.seconds,
    );
    if suite.cases.is_empty() && suite.skip_message.is_none() && !suite.has_note() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    // Each testcase gets an equal share of the request's time; an empty suite has no shares.
    let share = suite.seconds / suite.cases.len().max(1) as f64;
    for case in &suite.cases {
        let _ = write!(
            out,
            r#"    <testcase name="{}" status="{}" classname="{classname}" time="{share:.3}""#,
            att_escape(&case.name),
            case.status,
        );
        match &case.child {
            Some(child) => {
                let _ = write!(out, ">\n      {child}\n    </testcase>\n");
            }
            None => out.push_str("/>\n"),
        }
    }
    if let Some(message) = suite.skip_message {
        let _ = writeln!(out, r#"    <skipped message="{message}"/>"#);
    }
    // Suite-level system-out is the schema-valid place for a note about the whole request.
    let mut note = Vec::new();
    if suite.attempts > 1 {
        note.push(format!("attempts: {}", suite.attempts));
    }
    if let Some(error) = suite.last_attempt_error {
        note.push(format!("last attempt error: {error}"));
    }
    note.extend(suite.cookies.iter().cloned());
    if !note.is_empty() {
        let _ = writeln!(
            out,
            "    <system-out>{}</system-out>",
            text_escape(&note.join("\n"))
        );
    }
    out.push_str("  </testsuite>\n");
}

/// The path without its final extension, as the reference reporter's `classname`.
fn strip_extension(path: &str) -> &str {
    let name_start = path.rfind('/').map_or(0, |i| i + 1);
    match path[name_start..].rfind('.') {
        Some(dot) if dot > 0 => &path[..name_start + dot],
        _ => path,
    }
}

/// Escape an attribute value as xmlbuilder's `attEscape` does (`>` stays literal); characters
/// illegal in XML 1.0 become U+FFFD instead of aborting the report.
fn att_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'.. => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

/// Escape element text: markup characters become entities, line breaks stay, and characters
/// illegal in XML 1.0 become U+FFFD.
fn text_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(c),
            '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'.. => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::results_report::report::{Outcome, Report, ResultRow, sample};

    const TS: &str = "2026-10-12T10:00:00.000";

    #[test]
    fn sample_report_matches_bruno_shape() {
        insta::assert_snapshot!(render(&sample(), TS, "ci-box", true), @r#"
        <?xml version="1.0"?>
        <testsuites>
          <testsuite name="broken.yml" file="broken.yml" errors="1" failures="0" skipped="0" tests="1" timestamp="2026-10-12T10:00:00.000" hostname="ci-box" time="0.000">
            <testcase name="Test suite has no errors" status="fail" classname="broken" time="0.000">
              <error type="error" message="bad yaml &amp; &lt;stuff>"/>
            </testcase>
          </testsuite>
          <testsuite name="List users" file="users/list.yml" errors="0" failures="0" skipped="1" tests="5" timestamp="2026-10-12T10:00:00.000" hostname="ci-box" time="0.120">
            <testcase name="res.status eq 200" status="pass" classname="users/list" time="0.024"/>
            <testcase name="res.body isJson" status="skipped" classname="users/list" time="0.024">
              <skipped message="not supported"/>
            </testcase>
            <testcase name="sets token" status="pass" classname="users/list" time="0.024"/>
            <testcase name="has users" status="pass" classname="users/list" time="0.024"/>
            <testcase name="stores id" status="pass" classname="users/list" time="0.024"/>
          </testsuite>
          <testsuite name="Create &quot;user&quot;" file="users/create.yml" errors="0" failures="2" skipped="0" tests="2" timestamp="2026-10-12T10:00:00.000" hostname="ci-box" time="1.500">
            <testcase name="res.status eq 200" status="fail" classname="users/create" time="0.750">
              <failure type="failure" message="expected 201 to equal 200"/>
            </testcase>
            <testcase name="id is set" status="fail" classname="users/create" time="0.750">
              <failure type="failure" message="expected 1 to equal 2&#xA;at line 3"/>
            </testcase>
          </testsuite>
          <testsuite name="Get order" file="orders/get.yml" errors="1" failures="0" skipped="0" tests="1" timestamp="2026-10-12T10:00:00.000" hostname="ci-box" time="0.000">
            <testcase name="Test suite has no errors" status="fail" classname="orders/get" time="0.000">
              <error type="error" message="undefined variable `host`"/>
            </testcase>
          </testsuite>
          <testsuite name="Delete order" file="orders/delete.yml" errors="0" failures="0" skipped="1" tests="0" timestamp="2026-10-12T10:00:00.000" hostname="ci-box" time="0.000">
            <skipped message="Request skipped due to bail"/>
          </testsuite>
        </testsuites>
        "#);
    }

    #[test]
    fn skipped_request_without_bail() {
        let xml = render(&sample(), TS, "h", false);
        assert!(
            xml.contains(r#"<skipped message="Request Skipped"/>"#),
            "{xml}"
        );
        assert!(!xml.contains("due to bail"));
    }

    fn one(result: ResultRow) -> Report {
        Report::new(vec![], vec![result])
    }

    fn bare(name: &str, outcome: Outcome) -> ResultRow {
        let mut r = sample().results.remove(3);
        r.name = name.into();
        r.outcome = outcome;
        r
    }

    #[test]
    fn skipped_wins_over_error() {
        let mut r = bare("s", Outcome::Skipped);
        r.error = Some("boom".into());
        let xml = render(&one(r), TS, "h", false);
        assert!(
            xml.contains(r#"errors="0" failures="0" skipped="1" tests="0""#),
            "{xml}"
        );
        assert!(!xml.contains("Test suite has no errors"), "{xml}");
    }

    #[test]
    fn a_connect_failure_carries_the_hinted_line() {
        let mut r = bare("c", Outcome::Errored);
        r.error = Some("u: could not connect: refused".into());
        r.error_line = Some("u: Connection refused: Is it running? (refused)".into());
        let xml = render(&one(r), TS, "h", false);
        assert!(
            xml.contains(r#"message="u: Connection refused: Is it running? (refused)""#),
            "{xml}"
        );
        assert!(!xml.contains("could not connect"), "{xml}");
    }

    #[test]
    fn the_carried_failure_joins_the_attempts_note_escaped() {
        let mut r = bare("c", Outcome::Errored);
        r.attempts = 2;
        r.error = Some("cancelled".into());
        r.last_attempt_error = Some("refused <x> & y".into());
        let xml = render(&one(r), TS, "h", false);
        assert!(
            xml.contains(
                "<system-out>attempts: 2\nlast attempt error: refused &lt;x&gt; &amp; y</system-out>"
            ),
            "{xml}"
        );
    }

    #[test]
    fn a_carried_failure_after_one_attempt_still_gets_a_note() {
        let mut r = bare("c", Outcome::Errored);
        r.error = Some("cancelled".into());
        r.last_attempt_error = Some("refused".into());
        let xml = render(&one(r), TS, "h", false);
        assert!(
            xml.contains("<system-out>last attempt error: refused</system-out>"),
            "{xml}"
        );
    }

    #[test]
    fn empty_report_is_an_empty_root() {
        let xml = render(&Report::new(vec![], vec![]), TS, "h", false);
        assert_eq!(xml, "<?xml version=\"1.0\"?>\n<testsuites/>");
    }

    #[test]
    fn illegal_xml_characters_become_replacement_and_whitespace_is_escaped() {
        let r = bare("a\u{1}b\tc\rd", Outcome::Skipped);
        let xml = render(&one(r), TS, "h", false);
        assert!(xml.contains("name=\"a\u{FFFD}b&#x9;c&#xD;d\""), "{xml}");
    }

    #[test]
    fn errored_suite_reports_zero_failures_even_with_failed_checks() {
        let mut r = sample().results.remove(1);
        r.outcome = Outcome::Errored;
        r.error = Some("boom".into());
        let xml = render(&one(r), TS, "h", false);
        assert!(
            xml.contains(r#"errors="1" failures="0" skipped="0" tests="1""#),
            "{xml}"
        );
    }
}
