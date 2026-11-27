//! The Swift target: a top-level script sending the request with Foundation's `URLSession`
//! and printing the status and body. It builds on Linux too, where URLSession lives in
//! FoundationNetworking. Multipart bodies are written by hand.

use crate::execute::form_payload;
use crate::resolve::{Body, ResolvedRequest};

use super::{
    BOUNDARY, Segment, digest_credentials, multipart_segments, omitted_auth, sent_headers,
    strict_url,
};

/// Headers URLSession sets itself and ignores from the caller, lower-cased.
const RESERVED: [&str; 3] = ["host", "connection", "content-length"];

/// `text` as a Swift string literal. Every `\` is escaped, so no `\(` interpolation can form;
/// other controls and the line separators use `\u{XX}`.
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
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A file's bytes as `Data`; a read failure stops the script.
fn read(path: &std::path::Path) -> String {
    format!(
        "try! Data(contentsOf: URL(fileURLWithPath: {}))",
        literal(&path.to_string_lossy())
    )
}

/// A script sending the request with a data task and waiting for it. A session delegate is
/// written only when needed: to stop at a redirect, or to answer a digest challenge.
pub(super) fn url_session(req: &ResolvedRequest) -> String {
    let mut notes = Vec::new();
    if let Some(note) = omitted_auth(req, true) {
        notes.push(note.to_string());
    }
    let (sent, reserved): (Vec<_>, Vec<_>) = sent_headers(req)
        .partition(|(name, _)| !RESERVED.contains(&name.to_ascii_lowercase().as_str()));
    if !reserved.is_empty() {
        let names: Vec<String> = reserved
            .iter()
            .map(|(name, _)| name.escape_debug().to_string())
            .collect();
        notes.push(format!(
            "URLSession sets these headers itself: {}.",
            names.join(", ")
        ));
    }

    let mut out = String::new();
    for note in notes {
        out.push_str(&format!("// {note}\n"));
    }
    out.push_str(
        "import Foundation\n#if canImport(FoundationNetworking)\nimport FoundationNetworking\n#endif\n\n",
    );
    out.push_str(&format!(
        "var request = URLRequest(url: URL(string: {})!)\nrequest.httpMethod = {}\n",
        literal(&strict_url(req.url())),
        literal(req.method())
    ));
    for (name, value) in &sent {
        out.push_str(&format!(
            "request.setValue({}, forHTTPHeaderField: {})\n",
            literal(value),
            literal(name)
        ));
    }
    match req.body() {
        Body::None => {}
        Body::Text(text) => out.push_str(&format!(
            "request.httpBody = Data({}.utf8)\n",
            literal(text)
        )),
        Body::FormUrlEncoded(pairs) => out.push_str(&format!(
            "request.httpBody = Data({}.utf8)\n",
            literal(&form_payload(pairs))
        )),
        Body::File { path, .. } => out.push_str(&format!("request.httpBody = {}\n", read(path))),
        Body::Multipart(parts) => {
            out.push_str(&format!(
                "request.setValue(\"multipart/form-data; boundary={BOUNDARY}\", forHTTPHeaderField: \"Content-Type\")\n"
            ));
            out.push_str("var body = Data()\n");
            for segment in multipart_segments(parts) {
                match segment {
                    Segment::Text(text) => {
                        out.push_str(&format!("body.append(Data({}.utf8))\n", literal(&text)))
                    }
                    Segment::File(path) => out.push_str(&format!("body.append({})\n", read(path))),
                }
            }
            out.push_str("request.httpBody = body\n");
        }
    }

    let digest = digest_credentials(req);
    let session = if req.follow_redirects() && digest.is_none() {
        "URLSession.shared".to_string()
    } else {
        out.push_str("\nfinal class Delegate: NSObject, URLSessionTaskDelegate {\n");
        if !req.follow_redirects() {
            out.push_str(
                "    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {\n\
                 \x20       completionHandler(nil)\n    }\n",
            );
        }
        if let Some((user, password)) = digest {
            out.push_str(&format!(
                "    func urlSession(_ session: URLSession, task: URLSessionTask, didReceive challenge: URLAuthenticationChallenge, completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {{\n\
                 \x20       if challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodHTTPDigest && challenge.previousFailureCount == 0 {{\n\
                 \x20           completionHandler(.useCredential, URLCredential(user: {}, password: {}, persistence: .forSession))\n\
                 \x20       }} else {{\n\
                 \x20           completionHandler(.performDefaultHandling, nil)\n\
                 \x20       }}\n    }}\n",
                literal(user),
                literal(password)
            ));
        }
        out.push_str("}\n");
        "URLSession(configuration: .default, delegate: Delegate(), delegateQueue: nil)".to_string()
    };

    out.push_str(&format!(
        "\nlet done = DispatchSemaphore(value: 0)\n\
         {session}.dataTask(with: request) {{ data, response, error in\n\
         \x20   if let response = response as? HTTPURLResponse {{\n\
         \x20       print(response.statusCode)\n\
         \x20       print(String(decoding: data ?? Data(), as: UTF8.self))\n\
         \x20   }} else if let error = error {{\n\
         \x20       print(error)\n\
         \x20   }}\n\
         \x20   done.signal()\n\
         }}.resume()\n\
         done.wait()\n"
    ));
    out
}
