//! How a response prints: the status line, the head, pretty JSON and the binary placeholder.
//! Pure text helpers shared by the CLI and any other shell that shows a response.

use crate::http_version::HttpVersion;
use crate::response::Response;

/// `HTTP/<version> <status> <reason>`; the version is the last hop's, HTTP/1.1 when unknown,
/// and the reason is left off when the server sent none.
pub fn status_line(response: &Response) -> String {
    let version = (response.timeline().last())
        .and_then(|hop| hop.version())
        .unwrap_or(HttpVersion::Http11);
    match response.status_text() {
        "" => format!("{version} {}", response.status()),
        reason => format!("{version} {} {reason}", response.status()),
    }
}

/// The status line and one `name: value` line per header, each ending in a newline. Every
/// line passes through `redact`, so a secret in a header value never prints.
pub fn head(response: &Response, redact: &dyn Fn(&str) -> String) -> String {
    let mut out = format!("{}\n", status_line(response));
    for (name, value) in response.headers() {
        let value = crate::mask_response_header(name, value);
        out.push_str(&redact(&format!("{name}: {value}")));
        out.push('\n');
    }
    out
}

/// `body` as indented JSON with its key order and number text kept, or `None` when it is
/// not UTF-8 JSON.
pub fn pretty_json(body: &[u8]) -> Option<String> {
    crate::json_text::reindent_json(std::str::from_utf8(body).ok()?)
}

/// Whether `body` must not reach a terminal: it is not valid UTF-8.
pub fn is_binary(body: &[u8]) -> bool {
    std::str::from_utf8(body).is_err()
}

/// The line printed in place of a binary body of `size` bytes.
pub fn binary_placeholder(size: u64) -> String {
    format!("[binary body, {size} bytes; use -o FILE]")
}
