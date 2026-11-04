//! The C# target: a top-level program sending the request with `System.Net.Http.HttpClient`
//! and printing the status code and body.

use crate::execute::form_payload;
use crate::multipart::{Part, PartValue};
use crate::resolve::{Body, ResolvedRequest};

use super::{
    base_name, digest_credentials, disposition_escape, omitted_auth, part_type, sent_headers,
};

/// Headers .NET keeps on `HttpContent` rather than on the request, lower-cased.
pub(super) const CONTENT_HEADERS: [&str; 11] = [
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-location",
    "content-md5",
    "content-range",
    "content-disposition",
    "expires",
    "last-modified",
    "allow",
];

/// `text` as a C# regular string literal. Controls use the fixed-width `\uXXXX` (`\x` would
/// swallow a following hex digit), and so do U+0085, U+2028 and U+2029, which end a line in C#.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => out.push_str("\\0"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20
                || matches!(c, '\u{7f}' | '\u{85}' | '\u{2028}' | '\u{2029}') =>
            {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The statements that set `request.Content`, whether they read a file, and whether they use
/// `System.Text.Encoding`.
fn content(req: &ResolvedRequest) -> Option<(String, bool, bool)> {
    let bytes = |text: &str| {
        format!(
            "request.Content = new ByteArrayContent(Encoding.UTF8.GetBytes({}));\n",
            literal(text)
        )
    };
    match req.body() {
        Body::None => None,
        Body::Text(text) => Some((bytes(text), false, true)),
        Body::FormUrlEncoded(pairs) => Some((bytes(&form_payload(pairs)), false, true)),
        Body::File { path, .. } => Some((
            format!(
                "request.Content = new StreamContent(File.OpenRead({}));\n",
                literal(&path.to_string_lossy())
            ),
            true,
            false,
        )),
        Body::Multipart(parts) => {
            let mut reads_file = false;
            // `Add` would RFC 2047-encode a non-ASCII name or filename, so such a part gets its
            // disposition written by hand and the form writes part headers as UTF-8.
            let raw_disposition = |part: &Part| {
                !part.name().is_ascii()
                    || matches!(part.value(), PartValue::File(p) if !base_name(p).is_ascii())
            };
            let utf8 = parts.iter().any(raw_disposition);
            let mut out = String::from("var form = new MultipartFormDataContent();\n");
            if utf8 {
                out.push_str("form.HeaderEncodingSelector = (_, _) => Encoding.UTF8;\n");
            }
            for (i, part) in parts.iter().enumerate() {
                let var = format!("part{}", i + 1);
                // An untyped text part drops StringContent's default type.
                let (make, file_name) = match part.value() {
                    PartValue::Text(value) => {
                        (format!("new StringContent({})", literal(value)), None)
                    }
                    PartValue::File(path) => {
                        reads_file = true;
                        (
                            format!(
                                "new StreamContent(File.OpenRead({}))",
                                literal(&path.to_string_lossy())
                            ),
                            Some(base_name(path)),
                        )
                    }
                };
                let ctype = part_type(part).map_or("null".to_string(), |t| {
                    format!("MediaTypeHeaderValue.Parse({})", literal(t))
                });
                out.push_str(&format!(
                    "var {var} = {make};\n{var}.Headers.ContentType = {ctype};\n"
                ));
                // `Add` throws on a `"` inside a name, so names are escaped as curl writes them.
                let name = disposition_escape(part.name());
                let file_name = file_name.map(|f| disposition_escape(&f));
                if raw_disposition(part) {
                    let mut value = format!("form-data; name=\"{name}\"");
                    if let Some(f) = file_name {
                        value.push_str(&format!("; filename=\"{f}\""));
                    }
                    out.push_str(&format!(
                        "{var}.Headers.TryAddWithoutValidation(\"Content-Disposition\", {});\nform.Add({var});\n",
                        literal(&value)
                    ));
                } else if let Some(f) = file_name {
                    out.push_str(&format!(
                        "form.Add({var}, {}, {});\n",
                        literal(&name),
                        literal(&f)
                    ));
                } else {
                    out.push_str(&format!("form.Add({var}, {});\n", literal(&name)));
                }
            }
            out.push_str("request.Content = form;\n");
            Some((out, reads_file, utf8))
        }
    }
}

/// A top-level program sending the request through `HttpClient`, with digest credentials on
/// its handler. Content headers go on the body; with no body they are named in a leading comment.
pub(super) fn http_client(req: &ResolvedRequest) -> String {
    let mut notes = Vec::new();
    if let Some(note) = omitted_auth(req, true) {
        notes.push(note.to_string());
    }
    let body = content(req);
    let (on_content, on_request): (Vec<_>, Vec<_>) = sent_headers(req)
        .partition(|(name, _)| CONTENT_HEADERS.contains(&name.to_ascii_lowercase().as_str()));
    if body.is_none() && !on_content.is_empty() {
        // Names are escaped as in a literal, since U+2028 would end a `//` comment.
        let names: Vec<String> = on_content
            .iter()
            .map(|(name, _)| {
                let quoted = literal(name);
                quoted[1..quoted.len() - 1].to_string()
            })
            .collect();
        notes.push(format!(
            "HttpClient sends content headers only with a body: {}.",
            names.join(", ")
        ));
    }

    // The handler refuses a non-ASCII header value unless told an encoding; UTF-8 sends the
    // value's bytes as curl does.
    let utf8_headers = req
        .headers()
        .iter()
        .any(|(name, value)| !name.is_ascii() || !value.is_ascii());
    let digest = digest_credentials(req);
    let credentials = digest.map(|(user, password)| {
        format!(
            "Credentials = new NetworkCredential({}, {})",
            literal(user),
            literal(password)
        )
    });
    let mut code = match (credentials, utf8_headers) {
        (None, false) => "var client = new HttpClient();\n".to_string(),
        (Some(c), false) => {
            format!("var client = new HttpClient(new HttpClientHandler {{ {c} }});\n")
        }
        (c, true) => {
            let c = c.map_or(String::new(), |c| format!("{c}, "));
            format!(
                "var client = new HttpClient(new SocketsHttpHandler {{ {c}RequestHeaderEncodingSelector = (_, _) => Encoding.UTF8 }});\n"
            )
        }
    };
    code.push_str(&format!(
        "var request = new HttpRequestMessage(new HttpMethod({}), {});\n",
        literal(req.method()),
        literal(req.url())
    ));
    for (name, value) in on_request {
        code.push_str(&format!(
            "request.Headers.TryAddWithoutValidation({}, {});\n",
            literal(name),
            literal(value)
        ));
    }
    let (mut reads_file, mut encodes) = (false, utf8_headers);
    if let Some((set, file, uses_encoding)) = &body {
        reads_file = *file;
        encodes |= *uses_encoding;
        code.push_str(set);
        for (name, value) in on_content {
            code.push_str(&format!(
                "request.Content.Headers.TryAddWithoutValidation({}, {});\n",
                literal(name),
                literal(value)
            ));
        }
    }

    let multipart = matches!(req.body(), Body::Multipart(_));
    let mut out = String::new();
    for note in notes {
        out.push_str(&format!("// {note}\n"));
    }
    out.push_str("using System;\n");
    if reads_file {
        out.push_str("using System.IO;\n");
    }
    if digest.is_some() {
        out.push_str("using System.Net;\n");
    }
    out.push_str("using System.Net.Http;\n");
    if multipart {
        out.push_str("using System.Net.Http.Headers;\n");
    }
    if encodes {
        out.push_str("using System.Text;\n");
    }
    out.push_str(&format!(
        "\n{code}\nvar response = await client.SendAsync(request);\n\
         Console.WriteLine((int)response.StatusCode);\n\
         Console.WriteLine(await response.Content.ReadAsStringAsync());\n"
    ));
    out
}
