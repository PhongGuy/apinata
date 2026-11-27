//! The run report shared by the CLI and the app: the rows a finished run produced, rendered as
//! text, JSON, JUnit XML or a self-contained HTML page.

mod html;
mod junit;
mod report;

use report::{LoadErrorRow, Report, ResponseRow, ResultRow};

use crate::RequestResult;

/// What [`ResultsReport::with_responses`] leaves out of each response, so a report kept as a
/// build artifact need not carry what the server sent. The default keeps everything.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct ResponseDetail {
    omit_headers: bool,
    omit_header_names: Vec<String>,
    omit_body: bool,
}

impl ResponseDetail {
    /// Keep the headers and the body.
    pub fn new() -> ResponseDetail {
        ResponseDetail::default()
    }

    /// Leave out every header: the response has no `headers` key.
    pub fn omit_headers(mut self) -> ResponseDetail {
        self.omit_headers = true;
        self
    }

    /// Leave out each header called `name`, compared without regard to ASCII case.
    pub fn omit_header(mut self, name: impl Into<String>) -> ResponseDetail {
        self.omit_header_names.push(name.into());
        self
    }

    /// Leave out the body: the response has no `body` key, but keeps its size.
    pub fn omit_body(mut self) -> ResponseDetail {
        self.omit_body = true;
        self
    }

    fn keeps_header(&self, name: &str) -> bool {
        !(self.omit_header_names.iter()).any(|n| n.eq_ignore_ascii_case(name))
    }
}

/// What a finished run did, ready to render. The timestamp, host name and generation time that
/// the XML and HTML carry are passed in by the caller, so the output is reproducible.
#[non_exhaustive]
pub struct ResultsReport {
    inner: Report,
}

impl ResultsReport {
    /// Build the report from the files that failed to load, as `(path, message)` pairs, and the
    /// run's results in execution order.
    pub fn new<'a>(
        load_errors: impl IntoIterator<Item = (String, String)>,
        results: impl IntoIterator<Item = &'a RequestResult>,
    ) -> ResultsReport {
        let load_errors = load_errors
            .into_iter()
            .map(|(path, message)| LoadErrorRow { path, message })
            .collect();
        let rows = results.into_iter().map(ResultRow::from).collect();
        ResultsReport {
            inner: Report::new(load_errors, rows),
        }
    }

    /// Add `advice(cause)` to the line of each connect failure, after the hint's sentence and
    /// before the transport detail, for a front end that has flags the core does not know.
    /// The JSON `hint` is left as it is.
    pub fn with_connect_advice(
        mut self,
        advice: impl Fn(crate::ConnectCause) -> Option<String>,
    ) -> ResultsReport {
        self.inner.add_connect_advice(advice);
        self
    }

    /// Add each result's response to its row in the JSON report, leaving out what `detail`
    /// omits. `results` must be the ones given to [`ResultsReport::new`], in the same order;
    /// without this call the report has no `response` key.
    pub fn with_responses<'a>(
        mut self,
        results: impl IntoIterator<Item = &'a RequestResult>,
        detail: &ResponseDetail,
    ) -> ResultsReport {
        for (row, result) in self.inner.results.iter_mut().zip(results) {
            row.response = ResponseRow::of(result, detail);
        }
        self
    }

    /// 3 if a file did not load, else 2 if a request was not sent, else 1 if one failed, else 0.
    pub fn exit_code(&self) -> u8 {
        self.inner.exit_code()
    }

    /// The human report: a line per request with its checks, then the totals.
    pub fn to_text(&self) -> String {
        self.inner.to_text()
    }

    /// The JSON report, pretty-printed with a trailing newline.
    pub fn to_json(&self) -> String {
        // The report holds only strings, numbers and lists, so serializing cannot fail.
        let mut json = serde_json::to_string_pretty(&self.inner).unwrap_or_default();
        json.push('\n');
        json
    }

    /// The report as JUnit XML. `timestamp` is `%Y-%m-%dT%H:%M:%S%.3f`; `bail` says whether
    /// the run stopped at the first failure, which words the skipped requests' message.
    pub fn to_junit(&self, timestamp: &str, hostname: &str, bail: bool) -> String {
        junit::render(&self.inner, timestamp, hostname, bail)
    }

    /// The report as a self-contained HTML page stamped with `generated`.
    pub fn to_html(&self, generated: &str) -> String {
        html::render(&self.inner, generated)
    }
}
