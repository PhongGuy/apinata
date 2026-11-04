//! The HTML run report: one self-contained page with no scripts and no external resources.

use std::fmt::Write;

use crate::results_report::report::{Outcome, Report, ResultRow};

/// Colour tokens on `:root`, a dark override, and a badge colour per outcome class.
const STYLE: &str = r#":root {
  --bg: #ffffff; --fg: #1f2328; --muted: #656d76; --border: #d0d7de; --panel: #f6f8fa;
  --pass: #1a7f37; --fail: #cf222e; --error: #9a6700; --skip: #6e7781;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0d1117; --fg: #e6edf3; --muted: #8d96a0; --border: #30363d; --panel: #161b22;
    --pass: #3fb950; --fail: #f85149; --error: #d29922; --skip: #8d96a0;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0 auto; max-width: 1100px; padding: 16px; background: var(--bg); color: var(--fg);
  font: 14px/1.5 system-ui, sans-serif;
}
h1 { font-size: 20px; margin: 0; }
h2 { font-size: 16px; margin: 24px 0 8px; }
h3 { font-size: 14px; margin: 16px 0 4px; }
.muted { color: var(--muted); }
.summary { display: flex; flex-wrap: wrap; gap: 8px 24px; padding: 12px; margin-top: 16px;
  background: var(--panel); border: 1px solid var(--border); border-radius: 6px; }
details { border: 1px solid var(--border); border-radius: 6px; margin: 8px 0; }
summary { display: flex; flex-wrap: wrap; gap: 4px 12px; align-items: baseline; padding: 8px 12px;
  cursor: pointer; background: var(--panel); border-radius: 6px; }
details > div { padding: 4px 12px 12px; overflow-x: auto; }
.badge { font-size: 12px; font-weight: 600; text-transform: uppercase; color: var(--bg);
  padding: 0 6px; border-radius: 4px; }
.passed .badge, .pass { --c: var(--pass); }
.failed .badge, .fail { --c: var(--fail); }
.errored .badge { --c: var(--error); }
.skipped .badge, .skipped { --c: var(--skip); }
.badge { background: var(--c); }
td.pass, td.fail, td.skipped { color: var(--c); font-weight: 600; }
.name { font-weight: 600; }
.error { color: var(--fail); }
.warning { color: var(--error); }
table { border-collapse: collapse; width: 100%; }
th, td { text-align: left; vertical-align: top; padding: 4px 8px; border-bottom: 1px solid var(--border);
  white-space: pre-wrap; overflow-wrap: anywhere; }
th { color: var(--muted); font-weight: 600; }
pre { margin: 0; padding: 8px; background: var(--panel); border-radius: 4px; white-space: pre-wrap;
  overflow-wrap: anywhere; }
ul { margin: 0; padding-left: 20px; }
"#;

/// Render `report` as one self-contained HTML page; `generated` is the time shown in its header.
pub(crate) fn render(report: &Report, generated: &str) -> String {
    let mut out = String::from("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>Apinata run report</title>\n");
    let _ = write!(out, "<style>\n{STYLE}</style>\n</head>\n<body>\n");
    let _ = writeln!(
        out,
        "<header>\n<h1>Apinata run report</h1>\n<p class=\"muted\">Generated {}</p>\n</header>",
        escape(generated)
    );

    let s = &report.summary;
    let assertions = report.results.iter().flat_map(|r| &r.assertions);
    let passed = assertions.clone().filter(|a| a.verdict == "pass").count();
    let failed = assertions.filter(|a| a.verdict == "fail").count();
    let _ = writeln!(
        out,
        "<section class=\"summary\">\n\
         <span>{}</span>\n\
         <span>Assertions: {passed} passed, {failed} failed, {} skipped</span>\n\
         <span>Tests: {} passed, {} failed, {} skipped</span>\n</section>",
        s.requests_line(),
        s.skipped_assertions,
        s.passed_tests,
        s.failed_tests,
        s.skipped_tests
    );

    if !report.load_errors.is_empty() {
        out.push_str("<h2>Load errors</h2>\n<ul>\n");
        for e in &report.load_errors {
            let _ = writeln!(
                out,
                "<li><span class=\"name\">{}</span>: <span class=\"error\">{}</span></li>",
                escape(&e.path),
                escape(&e.message)
            );
        }
        out.push_str("</ul>\n");
    }

    if !report.results.is_empty() {
        out.push_str("<h2>Requests</h2>\n");
    }
    let mut iteration = None;
    for r in &report.results {
        if let Some(label) = r.iteration_label(&mut iteration) {
            let _ = writeln!(out, "<h3>{label}</h3>");
        }
        write_request(&mut out, r);
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// One collapsible request: the headline in `<summary>`, then each non-empty section.
fn write_request(out: &mut String, r: &ResultRow) {
    let outcome = match r.outcome {
        Outcome::Passed => "passed",
        Outcome::Failed => "failed",
        Outcome::Errored => "errored",
        Outcome::Skipped => "skipped",
    };
    // Any request that did not pass starts expanded so the reader lands on what went wrong.
    let open = if r.outcome == Outcome::Passed {
        ""
    } else {
        " open"
    };
    let _ = write!(
        out,
        "<details class=\"request {outcome}\"{open}>\n<summary><span class=\"badge\">{outcome}</span> \
         <span class=\"name\">{}</span> <span class=\"muted\">{}</span>",
        escape(&r.name),
        escape(&r.path)
    );
    if let Some(code) = r.status {
        let text = r.status_text.as_deref().unwrap_or("");
        let status = format!("{code} {text}");
        let _ = write!(out, " <span>{}</span>", escape(status.trim_end()));
    }
    if let Some(ms) = r.duration_ms {
        let _ = write!(out, " <span class=\"muted\">{ms} ms</span>");
    }
    if r.attempts > 1 {
        let _ = write!(out, " <span class=\"muted\">{} attempts</span>", r.attempts);
    }
    out.push_str("</summary>\n<div>\n");

    if let Some(error) = r.error_text() {
        let _ = writeln!(out, "<p class=\"error\">{}</p>", escape(error));
    }
    if let Some(reason) = &r.status_failure {
        let _ = writeln!(out, "<p class=\"error\">{}</p>", escape(reason));
    }
    if let Some(error) = &r.last_attempt_error {
        let _ = writeln!(
            out,
            "<p class=\"error\">last attempt error: {}</p>",
            escape(error)
        );
    }
    for w in &r.warnings {
        let _ = writeln!(out, "<p class=\"warning\">{}</p>", escape(w));
    }
    if !r.assertions.is_empty() {
        out.push_str("<h3>Assertions</h3>\n<table>\n<tr><th>Expression</th><th>Operator</th>");
        out.push_str("<th>Value</th><th>Verdict</th><th>Result</th></tr>\n");
        for a in &r.assertions {
            // The result cell reads like the text report: the actual value, then the detail.
            let result = match (&a.actual, &a.detail) {
                (Some(actual), Some(d)) => format!("actual {actual}; {d}"),
                (Some(actual), None) => format!("actual {actual}"),
                (None, Some(d)) => d.clone(),
                (None, None) => String::new(),
            };
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"{v}\">{v}</td><td>{}</td></tr>",
                escape(&a.expression),
                escape(&a.operator),
                escape(a.value.as_deref().unwrap_or("")),
                escape(&result),
                v = a.verdict
            );
        }
        out.push_str("</table>\n");
    }
    if !r.tests.is_empty() {
        out.push_str("<h3>Tests</h3>\n<table>\n");
        out.push_str("<tr><th>Phase</th><th>Name</th><th>Status</th><th>Message</th></tr>\n");
        for t in &r.tests {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td class=\"{s}\">{s}</td><td>{}</td></tr>",
                t.phase,
                escape(&t.name),
                escape(t.message.as_deref().unwrap_or("")),
                s = t.status
            );
        }
        out.push_str("</table>\n");
    }
    if !r.post_response_vars.is_empty() {
        out.push_str("<h3>Post-response variables</h3>\n<ul>\n");
        for v in &r.post_response_vars {
            let reason = v
                .reason
                .as_deref()
                .map(|reason| format!(" ({})", escape(reason)))
                .unwrap_or_default();
            let _ = writeln!(out, "<li>{}: {}{reason}</li>", escape(&v.name), v.result);
        }
        out.push_str("</ul>\n");
    }
    if !r.console.is_empty() {
        out.push_str("<h3>Console</h3>\n<pre>");
        for (i, c) in r.console.iter().enumerate() {
            let sep = if i == 0 { "" } else { "\n" };
            let _ = write!(out, "{sep}[{}] {}", c.level, escape(&c.text));
        }
        out.push_str("</pre>\n");
    }
    // What each hop and script did to the jar, never a cookie's value.
    if !r.cookies.is_empty() {
        out.push_str("<h3>Cookies</h3>\n<table>\n");
        out.push_str("<tr><th>From</th><th>Name</th><th>Verdict</th></tr>\n");
        for c in &r.cookies {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&c.origin_text()),
                escape(c.name_text()),
                escape(c.verdict_text())
            );
        }
        out.push_str("</table>\n");
    }
    out.push_str("</div>\n</details>\n");
}

/// Escape text for an HTML element body or a double- or single-quoted attribute.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::results_report::report::sample;

    fn page() -> String {
        render(&sample(), "2026-10-12 10:00:00 UTC")
    }

    #[test]
    fn a_connect_failure_shows_the_hinted_line() {
        let mut report = sample();
        let row = &mut report.results[1];
        row.error = Some("http://h/: could not connect: refused".into());
        row.error_line = Some("http://h/: Connection refused: Is it running? (refused)".into());
        let html = render(&report, "2026-10-12 10:00:00 UTC");
        assert!(
            html.contains(
                "<p class=\"error\">http://h/: Connection refused: Is it running? (refused)</p>"
            ),
            "{html}"
        );
        assert!(!html.contains("could not connect"), "{html}");
    }

    #[test]
    fn a_retried_request_shows_its_attempts_and_the_carried_failure() {
        let mut report = sample();
        let row = &mut report.results[1];
        row.attempts = 3;
        row.last_attempt_error = Some("refused <x>".into());
        let html = render(&report, "2026-10-12 10:00:00 UTC");
        assert!(html.contains("3 attempts"), "{html}");
        assert!(
            html.contains("<p class=\"error\">last attempt error: refused &lt;x&gt;</p>"),
            "{html}"
        );
    }

    #[test]
    fn a_single_attempt_shows_neither() {
        let html = page();
        assert!(
            !html.contains("attempts") && !html.contains("last attempt"),
            "{html}"
        );
    }

    #[test]
    fn page_is_self_contained() {
        let html = page();
        assert!(html.starts_with("<!doctype html>"), "{html}");
        assert!(html.ends_with("</html>\n"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(!html.contains("<link"));
        assert!(html.contains("@media (prefers-color-scheme: dark)"));
        assert!(!html.to_lowercase().contains("bruno"));
    }

    #[test]
    fn every_text_is_escaped() {
        let html = page();
        assert!(html.contains("Create &quot;user&quot;") || html.contains("Create &#34;user&#34;"));
        assert!(html.contains("bad yaml &amp; &lt;stuff&gt;"));
        assert!(html.contains("header &lt;x&gt; ignored"));
        assert!(!html.contains("<stuff>") && !html.contains("<x>"));
    }

    #[test]
    fn only_passed_requests_start_closed() {
        let html = page();
        assert_eq!(html.matches("<details").count(), 4);
        assert_eq!(
            html.matches("<details class=\"request passed\">").count(),
            1
        );
        assert_eq!(html.matches(" open>").count(), 3);
        assert!(html.contains("2026-10-12 10:00:00 UTC"));
        assert!(html.contains("expected 201 to equal 200"));
        assert!(html.contains("not supported"));
        assert!(html.contains("3 users"));
        assert!(html.contains("userId"));
    }

    #[test]
    fn page_snapshot() {
        insta::assert_snapshot!("html_report", page());
    }
}
