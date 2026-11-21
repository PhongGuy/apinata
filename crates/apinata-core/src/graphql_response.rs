//! A GraphQL response's top-level `errors` array in brief, so a shell can say a 200 failed.

use serde_json::Value;

/// What a GraphQL response's non-empty top-level `errors` array says, in brief.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlErrors {
    /// How many errors the array holds.
    pub count: usize,
    /// The first error's message on one line, when it has one.
    pub message: Option<String>,
    /// The first error's `path`, its segments joined with dots, when it has one.
    pub path: Option<String>,
    /// Whether `data` came back non-null beside the errors.
    pub partial: bool,
}

impl GraphqlErrors {
    /// One line: `2 GraphQL errors, partial data: <first message> (at <path>)`, each part
    /// after the count present only when it applies.
    pub fn summary(&self) -> String {
        let noun = if self.count == 1 { "error" } else { "errors" };
        let mut out = format!("{} GraphQL {noun}", self.count);
        if self.partial {
            out.push_str(", partial data");
        }
        if let Some(message) = &self.message {
            out.push_str(": ");
            out.push_str(message);
            if let Some(path) = &self.path {
                out.push_str(&format!(" (at {path})"));
            }
        }
        out
    }
}

/// The errors `body` reports when it is a JSON object with a non-empty top-level `errors`
/// array; `None` for any other body, JSON or not.
pub fn graphql_errors(body: &[u8]) -> Option<GraphqlErrors> {
    // Most bodies carry no errors key at all, so skip parsing them.
    if !body.windows(8).any(|w| w == b"\"errors\"") {
        return None;
    }
    let Value::Object(top) = serde_json::from_slice::<Value>(body).ok()? else {
        return None;
    };
    let errors = top.get("errors")?.as_array().filter(|a| !a.is_empty())?;
    let first = &errors[0];
    let message = (first.get("message").and_then(Value::as_str))
        .map(|m| m.split_whitespace().collect::<Vec<_>>().join(" "));
    let path = (first.get("path").and_then(Value::as_array))
        .map(|segments| {
            let parts: Vec<String> = (segments.iter())
                .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_owned))
                .collect();
            parts.join(".")
        })
        .filter(|p| !p.is_empty());
    Some(GraphqlErrors {
        count: errors.len(),
        message,
        path,
        partial: top.get("data").is_some_and(|d| !d.is_null()),
    })
}
