//! The Java target: a `Main` class sending the request with `java.net.http.HttpClient` and
//! printing the status and body. Multipart bodies are written by hand, since the client has
//! no multipart publisher.

use crate::execute::form_payload;
use crate::resolve::{Body, ResolvedRequest};

use super::{BOUNDARY, Segment, multipart_segments, omitted_auth, sent_headers, strict_url};

/// Headers the client refuses to take from the caller, lower-cased.
const RESTRICTED: [&str; 5] = ["host", "connection", "content-length", "expect", "upgrade"];

/// `text` as a Java string literal. Other controls use the fixed-width `\u00XX` so a following
/// digit cannot join the escape; LF and CR must stay `\n` and `\r`, because javac decodes
/// `\u` escapes before lexing and `\u000a` would end the line inside the literal.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The publisher expression for the body, and whether it reads a file (and so needs `Path`).
fn publisher(req: &ResolvedRequest) -> (String, bool) {
    const P: &str = "HttpRequest.BodyPublishers";
    match req.body() {
        Body::None => (format!("{P}.noBody()"), false),
        Body::Text(text) => (format!("{P}.ofString({})", literal(text)), false),
        Body::FormUrlEncoded(pairs) => (
            format!("{P}.ofString({})", literal(&form_payload(pairs))),
            false,
        ),
        Body::File { path, .. } => (
            format!("{P}.ofFile(Path.of({}))", literal(&path.to_string_lossy())),
            true,
        ),
        Body::Multipart(parts) => {
            let mut reads_file = false;
            let items: Vec<String> = multipart_segments(parts)
                .into_iter()
                .map(|segment| match segment {
                    Segment::Text(text) => format!("{P}.ofString({})", literal(&text)),
                    Segment::File(path) => {
                        reads_file = true;
                        format!("{P}.ofFile(Path.of({}))", literal(&path.to_string_lossy()))
                    }
                })
                .collect();
            let indent = "                ";
            let joined = items.join(&format!(",\n{indent}"));
            (format!("{P}.concat(\n{indent}{joined})"), reads_file)
        }
    }
}

/// A `Main` class sending the request with `HttpClient.newHttpClient()`, or a client following
/// every redirect when the request does. Headers the client sets itself or cannot send unchanged
/// are left out and named in a leading comment.
pub(super) fn http_client(req: &ResolvedRequest) -> String {
    let mut notes = Vec::new();
    if let Some(note) = omitted_auth(req, false) {
        notes.push(note.to_string());
    }
    let (sent, skipped): (Vec<_>, Vec<_>) = sent_headers(req)
        .partition(|(name, _)| !RESTRICTED.contains(&name.to_ascii_lowercase().as_str()));
    // The client writes header bytes as US-ASCII, so any other character would arrive as `?`.
    let (sent, non_ascii): (Vec<_>, Vec<_>) = sent
        .into_iter()
        .partition(|(name, value)| name.is_ascii() && value.is_ascii());
    for (names, note) in [
        (skipped, "Java's HttpClient sets these headers itself: "),
        (
            non_ascii,
            "Java's HttpClient sends header names and values as ASCII only; left out: ",
        ),
    ] {
        if !names.is_empty() {
            // javac decodes `\u` escapes in comments too, so names are escaped as in a literal.
            let names: Vec<String> = names
                .iter()
                .map(|(name, _)| {
                    let quoted = literal(name);
                    quoted[1..quoted.len() - 1].to_string()
                })
                .collect();
            notes.push(format!("{note}{}.", names.join(", ")));
        }
    }

    let (body, reads_file) = publisher(req);
    let mut build = format!(
        "        HttpRequest request = HttpRequest.newBuilder(URI.create({}))\n            .method({}, {body})\n",
        literal(&strict_url(req.url())),
        literal(req.method())
    );
    for (name, value) in sent {
        build.push_str(&format!(
            "            .header({}, {})\n",
            literal(name),
            literal(value)
        ));
    }
    if matches!(req.body(), Body::Multipart(_)) {
        build.push_str(&format!(
            "            .header(\"Content-Type\", \"multipart/form-data; boundary={BOUNDARY}\")\n"
        ));
    }
    build.push_str("            .build();\n");

    let mut out = String::new();
    for note in notes {
        out.push_str(&format!("// {note}\n"));
    }
    out.push_str("import java.net.URI;\nimport java.net.http.HttpClient;\nimport java.net.http.HttpRequest;\nimport java.net.http.HttpResponse;\n");
    if reads_file {
        out.push_str("import java.nio.file.Path;\n");
    }
    // `ALWAYS` also follows https to http, as the app's own send does; `NORMAL` would stop there.
    let client = if req.follow_redirects() {
        "HttpClient.newBuilder()\n            .followRedirects(HttpClient.Redirect.ALWAYS)\n            .build()"
    } else {
        "HttpClient.newHttpClient()"
    };
    out.push_str(&format!(
        "\npublic class Main {{\n    public static void main(String[] args) throws Exception {{\n{build}\
         \x20       HttpResponse<String> response = {client}\n\
         \x20           .send(request, HttpResponse.BodyHandlers.ofString());\n\
         \x20       System.out.println(response.statusCode());\n\
         \x20       System.out.println(response.body());\n    }}\n}}\n"
    ));
    out
}
