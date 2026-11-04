//! The Go target: a `package main` program sending the request with `net/http` and printing
//! the status and body. The layout is what `gofmt` writes, tabs included.

use crate::execute::form_payload;
use crate::multipart::{Part, PartValue};
use crate::resolve::{Body, ResolvedRequest};

use super::{base_name, omitted_auth, sent_headers};

/// `text` as an interpreted Go string literal. Control characters are escaped so the literal
/// stays on one line; non-ASCII is kept, since Go source is UTF-8, except the byte order mark Go refuses.
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
            // Go rejects a byte order mark anywhere in source, literals included.
            '\u{feff}' => out.push_str("\\ufeff"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A `Content-Disposition` parameter value quoted the way `mime/multipart` quotes its own.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

const CHECK: &str = "\tif err != nil {\n\t\tpanic(err)\n\t}\n";

/// Which packages a multipart body needs beyond `bytes` and `mime/multipart`.
struct Needs {
    textproto: bool,
    os: bool,
}

/// The statements building `body` for a multipart request. An untyped text part goes through
/// `WriteField`; a typed text part or a file part gets its own `CreatePart` header. Names are
/// declared with `:=` on first use and reassigned after.
fn multipart_body(parts: &[Part]) -> (String, Needs) {
    let mut out = String::from("\tbody := &bytes.Buffer{}\n\tform := multipart.NewWriter(body)\n");
    let mut needs = Needs {
        textproto: false,
        os: false,
    };
    for part in parts {
        let filename = match part.value() {
            PartValue::Text(text) if part.content_type().is_none() => {
                out.push_str(&format!(
                    "\tif err := form.WriteField({}, {}); err != nil {{\n\t\tpanic(err)\n\t}}\n",
                    literal(part.name()),
                    literal(text)
                ));
                continue;
            }
            PartValue::Text(_) => String::new(),
            PartValue::File(path) => format!("; filename={}", quoted(&base_name(path))),
        };
        let op = if needs.textproto { "=" } else { ":=" };
        let disposition = format!("form-data; name={}{filename}", quoted(part.name()));
        out.push_str(&format!("\theader {op} make(textproto.MIMEHeader)\n"));
        out.push_str(&format!(
            "\theader.Set(\"Content-Disposition\", {})\n",
            literal(&disposition)
        ));
        if let Some(ct) = part.content_type() {
            out.push_str(&format!(
                "\theader.Set(\"Content-Type\", {})\n",
                literal(ct)
            ));
        }
        out.push_str(&format!(
            "\tpart, err {op} form.CreatePart(header)\n{CHECK}"
        ));
        needs.textproto = true;
        match part.value() {
            PartValue::Text(text) => out.push_str(&format!(
                "\tif _, err := io.WriteString(part, {}); err != nil {{\n\t\tpanic(err)\n\t}}\n",
                literal(text)
            )),
            PartValue::File(path) => {
                let op = if needs.os { "=" } else { ":=" };
                out.push_str(&format!(
                    "\tfile, err {op} os.Open({})\n{CHECK}",
                    literal(&path.to_string_lossy())
                ));
                out.push_str(
                    "\tif _, err := io.Copy(part, file); err != nil {\n\t\tpanic(err)\n\t}\n\tfile.Close()\n",
                );
                needs.os = true;
            }
        }
    }
    out.push_str("\tif err := form.Close(); err != nil {\n\t\tpanic(err)\n\t}\n");
    (out, needs)
}

/// A program sending the request with `http.DefaultClient`. Headers go in with `Header.Add`,
/// which canonicalises name casing.
pub(super) fn net_http(req: &ResolvedRequest) -> String {
    let mut imports = vec!["fmt", "io", "net/http"];
    let mut main = format!("\turl := {}\n", literal(req.url()));
    let multipart = matches!(req.body(), Body::Multipart(_));
    let has_body = match req.body() {
        Body::None => false,
        Body::Text(text) => {
            imports.push("strings");
            main.push_str(&format!("\tbody := strings.NewReader({})\n", literal(text)));
            true
        }
        Body::FormUrlEncoded(pairs) => {
            imports.push("strings");
            main.push_str(&format!(
                "\tbody := strings.NewReader({})\n",
                literal(&form_payload(pairs))
            ));
            true
        }
        Body::Multipart(parts) => {
            let (code, needs) = multipart_body(parts);
            imports.extend(["bytes", "mime/multipart"]);
            if needs.textproto {
                imports.push("net/textproto");
            }
            if needs.os {
                imports.push("os");
            }
            main.push_str(&code);
            true
        }
        Body::File { path, .. } => {
            imports.push("os");
            main.push_str(&format!(
                "\tbody, err := os.Open({})\n{CHECK}\tdefer body.Close()\n",
                literal(&path.to_string_lossy())
            ));
            true
        }
    };
    let body = if has_body { "body" } else { "nil" };
    main.push_str(&format!(
        "\treq, err := http.NewRequest({}, url, {body})\n{CHECK}",
        literal(req.method())
    ));
    for (name, value) in sent_headers(req) {
        main.push_str(&format!(
            "\treq.Header.Add({}, {})\n",
            literal(name),
            literal(value)
        ));
    }
    if multipart {
        main.push_str("\treq.Header.Set(\"Content-Type\", form.FormDataContentType())\n");
    }
    main.push_str(&format!(
        "\n\tres, err := http.DefaultClient.Do(req)\n{CHECK}\tdefer res.Body.Close()\n\
         \tout, err := io.ReadAll(res.Body)\n{CHECK}\tfmt.Println(res.Status)\n\tfmt.Println(string(out))\n"
    ));

    let mut out = String::new();
    if let Some(note) = omitted_auth(req, false) {
        out.push_str(&format!("// {note}\n\n"));
    }
    imports.sort_unstable();
    out.push_str("package main\n\nimport (\n");
    for import in imports {
        out.push_str(&format!("\t\"{import}\"\n"));
    }
    out.push_str(&format!(")\n\nfunc main() {{\n{main}}}\n"));
    out
}
