//! The ready-made script and test snippets offered to editors.

/// The script slots a snippet can be inserted into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SnippetPhase {
    /// The pre-request script.
    PreRequest,
    /// The post-response script.
    PostResponse,
    /// The tests script.
    Tests,
}

/// One insertable snippet; `select` is the placeholder substring of `code` to highlight.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Snippet {
    /// Stable identifier, also used for the button's element id.
    pub id: &'static str,
    /// Button text.
    pub label: &'static str,
    /// Code inserted at the caret.
    pub code: &'static str,
    /// Placeholder inside `code` selected after the insert.
    pub select: &'static str,
    /// Slots this snippet is offered in.
    pub phases: &'static [SnippetPhase],
}

use SnippetPhase::{PostResponse, PreRequest, Tests};

const fn snippet(
    id: &'static str,
    label: &'static str,
    phases: &'static [SnippetPhase],
    code: &'static str,
    select: &'static str,
) -> Snippet {
    Snippet {
        id,
        label,
        code,
        select,
        phases,
    }
}

static TABLE: [Snippet; 9] = [
    snippet(
        "status-200",
        "Status is 200",
        &[Tests],
        "test(\"Status is as expected\", function () {\n  expect(res.getStatus()).to.equal(200);\n});",
        "200",
    ),
    snippet(
        "time-under",
        "Response time under N ms",
        &[Tests],
        "test(\"Response time is under the limit\", function () {\n  expect(res.getResponseTime()).to.be.below(500);\n});",
        "500",
    ),
    snippet(
        "content-type-json",
        "Content-Type is JSON",
        &[Tests],
        "test(\"Content-Type is JSON\", function () {\n  expect(String(res.getHeader(\"content-type\")).toLowerCase()).to.include(\"json\");\n});",
        "\"json\"",
    ),
    snippet(
        "body-contains",
        "Body contains text",
        &[Tests],
        "const text = \"apinata\";\ntest(\"Body contains \" + text, function () {\n  expect(JSON.stringify(res.getBody())).to.include(text);\n});",
        "\"apinata\"",
    ),
    snippet(
        "json-field",
        "JSON field equals",
        &[Tests],
        "const field = \"name\";\ntest(\"JSON field \" + field + \" has the expected value\", function () {\n  expect(res.getBody()[field]).to.equal(\"apinata\");\n});",
        "\"name\"",
    ),
    snippet(
        "header-exists",
        "Header exists",
        &[Tests],
        "const header = \"x-trace\";\ntest(\"Header \" + header + \" exists\", function () {\n  expect(res.getHeader(header)).to.exist;\n});",
        "\"x-trace\"",
    ),
    snippet(
        "var-from-body",
        "Set a variable from the body",
        &[Tests, PostResponse],
        "bru.setVar(\"id\", res.getBody().id);",
        "\"id\"",
    ),
    snippet(
        "set-header",
        "Set a header",
        &[PreRequest],
        "req.setHeader(\"x-trace\", \"1\");",
        "\"x-trace\"",
    ),
    snippet(
        "set-var",
        "Set a variable",
        &[PreRequest, PostResponse],
        "bru.setVar(\"name\", \"value\");",
        "\"name\"",
    ),
];

/// Every snippet, in the order the buttons are shown.
pub fn script_snippets() -> &'static [Snippet] {
    &TABLE
}
