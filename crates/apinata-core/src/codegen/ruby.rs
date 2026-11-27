//! The Ruby target: a script sending the request with the standard library's `Net::HTTP` and
//! printing the status and body. Net::HTTP gives a body sent without a Content-Type the form
//! type, and it never follows redirects, which the script says when the request would.

use crate::execute::form_payload;
use crate::resolve::{Body, ResolvedRequest};

use super::{BOUNDARY, Segment, multipart_segments, omitted_auth, sent_headers, strict_url};

/// The methods Net::HTTP has a request class for, by the class name.
const CLASSES: [(&str, &str); 8] = [
    ("GET", "Get"),
    ("HEAD", "Head"),
    ("POST", "Post"),
    ("PUT", "Put"),
    ("DELETE", "Delete"),
    ("OPTIONS", "Options"),
    ("TRACE", "Trace"),
    ("PATCH", "Patch"),
];

/// `text` as a double-quoted Ruby literal. A `#` is escaped only where it would start an
/// interpolation, so the literal reads like the JSON form of the text wherever it can.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '#' if matches!(chars.peek(), Some('{' | '$' | '@')) => out.push_str("\\#"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A file's bytes read as a binary string.
fn binread(path: &std::path::Path) -> String {
    format!("File.binread({})", literal(&path.to_string_lossy()))
}

/// A script sending the request through `Net::HTTP.start`. Multipart bodies are written by
/// hand as binary strings, since `set_form` cannot type a text part.
pub(super) fn net_http(req: &ResolvedRequest) -> String {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, false) {
        out.push_str(&format!("# {note}\n"));
    }
    if req.follow_redirects() {
        out.push_str("# Net::HTTP does not follow redirects; this prints the first response.\n");
    }
    out.push_str("require \"net/http\"\n\n");
    out.push_str(&format!("uri = URI({})\n", literal(&strict_url(req.url()))));
    let method = req.method();
    match CLASSES.iter().find(|(m, _)| *m == method) {
        Some((_, class)) => out.push_str(&format!("request = Net::HTTP::{class}.new(uri)\n")),
        None => out.push_str(&format!(
            "request = Net::HTTPGenericRequest.new({}, true, true, uri)\n",
            literal(method)
        )),
    }
    for (name, value) in sent_headers(req) {
        out.push_str(&format!(
            "request[{}] = {}\n",
            literal(name),
            literal(value)
        ));
    }
    match req.body() {
        Body::None => {}
        Body::Text(text) => out.push_str(&format!("request.body = {}\n", literal(text))),
        Body::FormUrlEncoded(pairs) => out.push_str(&format!(
            "request.body = {}\n",
            literal(&form_payload(pairs))
        )),
        Body::File { path, .. } => out.push_str(&format!("request.body = {}\n", binread(path))),
        Body::Multipart(parts) => {
            out.push_str(&format!(
                "request[\"Content-Type\"] = \"multipart/form-data; boundary={BOUNDARY}\"\n"
            ));
            // Each text run is made binary, so joining it with a file's bytes cannot fail.
            out.push_str("request.body = [\n");
            for segment in multipart_segments(parts) {
                let item = match segment {
                    Segment::Text(text) => format!("{}.b", literal(&text)),
                    Segment::File(path) => binread(path),
                };
                out.push_str(&format!("  {item},\n"));
            }
            out.push_str("].join\n");
        }
    }
    out.push_str(
        "\nresponse = Net::HTTP.start(uri.hostname, uri.port, use_ssl: uri.scheme == \"https\") do |http|\n\
         \x20 http.request(request)\nend\nputs response.code\nputs response.body\n",
    );
    out
}
