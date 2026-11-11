//! Writes a resolved request as a snippet for another tool or language. Generation is pure:
//! no I/O, and every value is a literal in the target's syntax that evaluates to the resolved
//! value. Transport settings are not written.

mod csharp;
mod go;
mod java;
mod javascript;
mod php;
mod powershell;
mod python;
mod rust;
mod shell;

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use crate::multipart::{Part, PartValue, file_content_type};
use crate::resolve::{Body, ResolvedRequest, SendAuth};

/// A language or tool `generate` writes. `ALL` is the picker's order.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A POSIX shell `curl` command.
    Curl,
    /// A POSIX shell HTTPie (`http`) command.
    Httpie,
    /// A Python script using `requests`.
    PythonRequests,
    /// A Node 18+ ES module using the built-in `fetch`.
    JsFetch,
    /// A Node ES module using `axios`.
    NodeAxios,
    /// A Go `package main` program using `net/http`.
    GoNative,
    /// A PowerShell 7 script using `Invoke-RestMethod`.
    PowerShellRestMethod,
    /// A PHP script using the curl extension.
    PhpCurl,
    /// A Rust program using `reqwest::blocking`.
    RustReqwest,
    /// A Java 11+ `Main` class using `java.net.http.HttpClient`.
    JavaHttpClient,
    /// A C# top-level program using `System.Net.Http.HttpClient`.
    CSharpHttpClient,
}

impl Target {
    /// Every target, in the order a picker lists them.
    pub const ALL: &'static [Target] = &[
        Target::Curl,
        Target::Httpie,
        Target::PythonRequests,
        Target::JsFetch,
        Target::NodeAxios,
        Target::GoNative,
        Target::PowerShellRestMethod,
        Target::PhpCurl,
        Target::RustReqwest,
        Target::JavaHttpClient,
        Target::CSharpHttpClient,
    ];

    /// The stable id the CLI accepts, such as `curl`.
    pub fn id(self) -> &'static str {
        match self {
            Target::Curl => "curl",
            Target::Httpie => "httpie",
            Target::PythonRequests => "python-requests",
            Target::JsFetch => "js-fetch",
            Target::NodeAxios => "node-axios",
            Target::GoNative => "go",
            Target::PowerShellRestMethod => "powershell",
            Target::PhpCurl => "php-curl",
            Target::RustReqwest => "rust-reqwest",
            Target::JavaHttpClient => "java-httpclient",
            Target::CSharpHttpClient => "csharp-httpclient",
        }
    }

    /// The name shown to people, such as `HTTPie`.
    pub fn label(self) -> &'static str {
        match self {
            Target::Curl => "curl",
            Target::Httpie => "HTTPie",
            Target::PythonRequests => "Python (requests)",
            Target::JsFetch => "JavaScript (fetch)",
            Target::NodeAxios => "Node.js (axios)",
            Target::GoNative => "Go (net/http)",
            Target::PowerShellRestMethod => "PowerShell (Invoke-RestMethod)",
            Target::PhpCurl => "PHP (curl)",
            Target::RustReqwest => "Rust (reqwest)",
            Target::JavaHttpClient => "Java (HttpClient)",
            Target::CSharpHttpClient => "C# (HttpClient)",
        }
    }
}

impl FromStr for Target {
    type Err = UnknownTarget;

    /// The target whose id is exactly `s`.
    fn from_str(s: &str) -> Result<Target, UnknownTarget> {
        Target::ALL
            .iter()
            .copied()
            .find(|t| t.id() == s)
            .ok_or(UnknownTarget)
    }
}

/// A target id that names no target. Its message lists the valid ids and never echoes the
/// rejected input, which may be anything the caller typed.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownTarget;

impl fmt::Display for UnknownTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids: Vec<&str> = Target::ALL.iter().map(|t| t.id()).collect();
        write!(f, "unknown target; expected one of: {}", ids.join(", "))
    }
}

impl std::error::Error for UnknownTarget {}

/// The request as a `target` snippet. Secret values are included as resolved; check
/// `ResolvedRequest::sensitive_values` before showing it. Auth the target cannot express is
/// named in a leading comment, never dropped silently.
pub fn generate(req: &ResolvedRequest, target: Target) -> String {
    match target {
        Target::Curl => shell::curl(req),
        Target::Httpie => shell::httpie(req),
        Target::PythonRequests => python::requests(req),
        Target::JsFetch => javascript::fetch(req),
        Target::NodeAxios => javascript::axios(req),
        Target::GoNative => go::net_http(req),
        Target::PowerShellRestMethod => powershell::rest_method(req),
        Target::PhpCurl => php::curl(req),
        Target::RustReqwest => rust::reqwest(req),
        Target::JavaHttpClient => java::http_client(req),
        Target::CSharpHttpClient => csharp::http_client(req),
    }
}

/// The headers to send: the resolved ones, less a multipart `Content-Type` when the body is
/// multipart, since its boundary is stale and the library writes its own.
pub(crate) fn sent_headers(req: &ResolvedRequest) -> impl Iterator<Item = &(String, String)> {
    let multipart = matches!(req.body(), Body::Multipart(_));
    req.headers().iter().filter(move |(name, value)| {
        !(multipart
            && name.eq_ignore_ascii_case("content-type")
            && value.starts_with("multipart/form-data"))
    })
}

/// The sentence naming send-time auth the snippet leaves out: SigV4, OAuth 1.0, WSSE and
/// EdgeGrid always, and digest unless the target writes digest itself (`natively_digest`).
pub(crate) fn omitted_auth(req: &ResolvedRequest, natively_digest: bool) -> Option<&'static str> {
    match req.send_auth()? {
        SendAuth::AwsV4(_) => Some("AWS Signature auth is not included in this snippet."),
        SendAuth::OAuth1(_) => Some("OAuth 1.0 auth is not included in this snippet."),
        SendAuth::Wsse { .. } => Some("WSSE auth is not included in this snippet."),
        SendAuth::Ntlm { .. } => Some("NTLM auth is not included in this snippet."),
        SendAuth::EdgeGrid(_) => Some("Akamai EdgeGrid auth is not included in this snippet."),
        SendAuth::Digest { .. } if !natively_digest => {
            Some("Digest auth is not included in this snippet.")
        }
        SendAuth::Digest { .. } => None,
    }
}

/// The digest credentials when the request answers a digest challenge.
pub(crate) fn digest_credentials(req: &ResolvedRequest) -> Option<(&str, &str)> {
    match req.send_auth()? {
        SendAuth::Digest { username, password } => Some((username, password)),
        SendAuth::AwsV4(_)
        | SendAuth::OAuth1(_)
        | SendAuth::Wsse { .. }
        | SendAuth::Ntlm { .. }
        | SendAuth::EdgeGrid(_) => None,
    }
}

/// The file name a multipart file part is sent with, as curl does.
pub(crate) fn base_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The boundary of every hand-built multipart body. A file whose content holds a line
/// `--` plus this boundary would end its part early; that is accepted.
pub(crate) const BOUNDARY: &str = "apinata-boundary-5f0c2e9a7d4b";

/// One run of a hand-built multipart body: literal text, or the bytes of a file read at run time.
pub(crate) enum Segment<'a> {
    Text(String),
    File(&'a Path),
}

/// The body as segments, adjacent text merged: per part `--B\r\nContent-Disposition: form-data;
/// name="N"[; filename="F"]\r\n[Content-Type: T\r\n]\r\n`, the value, `\r\n`; then `--B--\r\n`.
pub(crate) fn multipart_segments(parts: &[Part]) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut text = String::new();
    for part in parts {
        text.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"",
            disposition_escape(part.name())
        ));
        if let PartValue::File(path) = part.value() {
            text.push_str(&format!(
                "; filename=\"{}\"",
                disposition_escape(&base_name(path))
            ));
        }
        text.push_str("\r\n");
        if let Some(ct) = part_type(part) {
            text.push_str(&format!("Content-Type: {ct}\r\n"));
        }
        text.push_str("\r\n");
        match part.value() {
            PartValue::Text(value) => text.push_str(value),
            PartValue::File(path) => {
                segments.push(Segment::Text(std::mem::take(&mut text)));
                segments.push(Segment::File(path));
            }
        }
        text.push_str("\r\n");
    }
    text.push_str(&format!("--{BOUNDARY}--\r\n"));
    segments.push(Segment::Text(text));
    segments
}

/// A name or file name made safe inside a quoted Content-Disposition parameter, as browsers
/// write it: `"`, CR and LF become `%22`, `%0D` and `%0A`.
pub(crate) fn disposition_escape(name: &str) -> String {
    name.replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// The content type a part is sent with: its own, else for a file part the one the executor
/// looks up from its extension, as curl does. An untyped text part has none.
pub(crate) fn part_type(part: &Part) -> Option<&str> {
    match part.value() {
        PartValue::File(path) => Some(part.content_type().unwrap_or(file_content_type(path))),
        PartValue::Text(_) => part.content_type(),
    }
}

/// Whether some text part carries its own content type.
pub(crate) fn has_typed_text(parts: &[Part]) -> bool {
    parts
        .iter()
        .any(|p| matches!(p.value(), PartValue::Text(_)) && p.content_type().is_some())
}
