//! Member completion for the script editors: after `bru.`, `req.` or `res.` the menu offers
//! what the script runtime implements, one level deep.

use gpui_kit::component::input::CompletionProvider;
use gpui_kit::component::{Rope, RopeExt as _};
use gpui_kit::{App, Result, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Range, TextEdit,
};

/// Members offered after `bru.`; only what the script runtime implements.
#[rustfmt::skip]
pub const BRU: &[&str] = &[
    "cwd", "getEnvName", "getCollectionName", "isSafeMode", "getProcessEnv", "interpolate",
    "hasVar", "getVar", "setVar", "deleteVar", "deleteAllVars", "getAllVars", "hasEnvVar",
    "getEnvVar", "setEnvVar", "deleteEnvVar", "deleteAllEnvVars", "getAllEnvVars",
    "hasGlobalEnvVar", "getGlobalEnvVar", "setGlobalEnvVar", "deleteGlobalEnvVar",
    "deleteAllGlobalEnvVars", "getAllGlobalEnvVars", "hasCollectionVar", "getCollectionVar",
    "setCollectionVar", "deleteCollectionVar", "deleteAllCollectionVars", "getAllCollectionVars",
    "getFolderVar", "getRequestVar", "runner", "setNextRequest", "getTestResults",
    "getAssertionResults", "cookies", "sleep", "sendRequest", "runRequest", "getOauth2CredentialVar",
    "resetOauth2Credential",
];

/// Members offered after `req.`; only what the script runtime implements.
#[rustfmt::skip]
pub const REQ: &[&str] = &[
    "url", "method", "headers", "body", "timeout", "name", "pathParams", "tags", "getUrl",
    "setUrl", "getHost", "getPath", "getQueryString", "getMethod", "setMethod", "getName",
    "getAuthMode", "getPathParams", "getTags", "getExecutionMode", "getHeaders", "getHeader",
    "setHeader", "deleteHeader", "deleteHeaders", "setHeaders", "getBody", "setBody",
    "getTimeout", "setTimeout", "setMaxRedirects",
];

/// Members offered after `res.`; only what the script runtime implements.
#[rustfmt::skip]
pub const RES: &[&str] = &[
    "status", "statusText", "headers", "body", "responseTime", "url", "getStatus",
    "getStatusText", "getHeader", "getHeaders", "getBody", "getResponseTime", "getUrl", "getSize",
    "setBody",
];

/// Items at `offset` (bytes) when the text before it is `bru.`, `req.` or `res.` (not part of a
/// longer identifier) plus an identifier prefix; each item's `text_edit` covers that prefix.
pub fn items(text: &Rope, offset: usize) -> Vec<CompletionItem> {
    // Only the cursor's line matters, so copy just that line up to the cursor.
    let point = text.offset_to_point(offset.min(text.len()));
    let line_start = text.line_start_offset(point.row);
    let line = text.slice_line(point.row).to_string();
    let line = &line[..point.column.min(line.len())];
    let prefix = trailing_name(line);
    let Some(before) = line[..line.len() - prefix.len()].strip_suffix('.') else {
        return Vec::new();
    };
    // The whole identifier before the dot must be the object, so `mybru.` offers nothing.
    let members = match trailing_name(before) {
        "bru" => BRU,
        "req" => REQ,
        "res" => RES,
        _ => return Vec::new(),
    };
    // A member typed in full opens no menu, so Enter after it still breaks the line.
    if members.contains(&prefix) {
        return Vec::new();
    }
    let end = line_start + line.len();
    let start = end - prefix.len();
    let range = Range::new(text.offset_to_position(start), text.offset_to_position(end));
    members
        .iter()
        .filter(|m| m.starts_with(prefix))
        .map(|&m| CompletionItem {
            label: m.to_owned(),
            kind: Some(CompletionItemKind::METHOD),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: m.to_owned(),
            })),
            ..Default::default()
        })
        .collect()
}

// The run of JavaScript identifier characters that ends `text`.
fn trailing_name(text: &str) -> &str {
    let start = text
        .bytes()
        .rposition(|b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'$'))
        .map_or(0, |i| i + 1);
    &text[start..]
}

/// The script editors' provider; triggers on `.` and identifier characters.
pub struct ScriptCompletions;

impl CompletionProvider for ScriptCompletions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        Task::ready(Ok(CompletionResponse::Array(items(text, offset))))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text == "."
            || (!new_text.is_empty()
                && new_text
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
    }
}
