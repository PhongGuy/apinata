//! `generate` per target over a fixed matrix of request shapes, pinned by insta snapshots.
//! Every snapshot is reviewed by hand against the output rules and run against the
//! execution check before it is accepted.

use crate::common;

use apinata_core::{ResolvedRequest, Target, UnknownTarget, generate, to_curl};
use common::resolved_one;

const GET_HEADERS: &str = "  headers:\n    - name: X-A\n      value: \"1\"\n    - name: x-a\n      value: \"2\"\n    - name: Cookie\n      value: a=1\n    - name: Cookie\n      value: b=2\n";
const JSON: &str = "  body:\n    type: json\n    data: '{\"a\": 1, \"s\": \"x y\"}'\n";
const FORM: &str = "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: b\n        value: x y&z\n";
const MULTIPART: &str = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: up\n        type: file\n        value: [/data/up.bin]\n        contentType: text/plain\n";
const FILE: &str = "  body:\n    type: file\n    data:\n      - filePath: /data/body.json\n        contentType: application/json\n        selected: true\n";
const BASIC: &str = "  auth:\n    type: basic\n    username: u\n    password: p\n";
const DIGEST: &str = "  auth:\n    type: digest\n    username: alice\n    password: \"p'w\"\n";
const AWS: &str = "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: secret-key\n    sessionToken: \"\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n";
// Quotes, a backslash, `$`, braces, a tab, a newline, non-ASCII and U+2028 in every slot that can hold them.
const HOSTILE: &str = r#"  headers:
    - name: X-H
      value: "it's \"q\" \\ $HOME {x} ü"
  body:
    type: text
    data: "a'b\"c\\d $e {f}\tg\nü€  end"
"#;
// Values a request-item syntax would read as a file, a JSON field or a separator.
const ITEMS: &str = r#"  headers:
    - name: X-E
      value: "=v"
  body:
    type: multipart-form
    data:
      - name: a
        type: text
        value: "@x"
      - name: "k=v"
        type: text
        value: "1"
"#;

// A typed text part, and a byte order mark Go source refuses inside a literal.
const TYPED: &str = "  body:\n    type: multipart-form\n    data:\n      - name: j\n        type: text\n        value: '{\"a\": \"x\u{feff}y\"}'\n        contentType: application/json\n      - name: t\n        type: text\n        value: plain\n";

// A typed text part, a file part, and a name holding a double quote.
const MIXED: &str = "  body:\n    type: multipart-form\n    data:\n      - name: j\n        type: text\n        value: '{\"a\": 1}'\n        contentType: application/json\n      - name: 'q\"x'\n        type: text\n        value: plain\n      - name: up\n        type: file\n        value: [/data/up.bin]\n        contentType: text/plain\n";

fn typed() -> ResolvedRequest {
    resolved_one("POST", "http://x.test/t", TYPED, "")
}

fn mixed() -> ResolvedRequest {
    resolved_one("POST", "http://x.test/x", MIXED, "")
}

/// The request shapes every target is snapshotted over, by case name.
fn matrix() -> Vec<(&'static str, ResolvedRequest)> {
    vec![
        (
            "get",
            resolved_one("GET", "http://x.test/a?q=1&r=two", GET_HEADERS, ""),
        ),
        ("json", resolved_one("POST", "http://x.test/j", JSON, "")),
        ("form", resolved_one("POST", "http://x.test/f", FORM, "")),
        (
            "multipart",
            resolved_one("POST", "http://x.test/m", MULTIPART, ""),
        ),
        ("file", resolved_one("PUT", "http://x.test/b", FILE, "")),
        (
            "basic",
            resolved_one("GET", "http://x.test/basic", BASIC, ""),
        ),
        (
            "digest",
            resolved_one("GET", "http://x.test/digest", DIGEST, ""),
        ),
        ("aws", resolved_one("GET", "http://x.test/aws", AWS, "")),
        (
            "hostile",
            resolved_one("POST", "http://x.test/p?x=a'b$c{d}&y=ü", HOSTILE, ""),
        ),
        ("items", resolved_one("POST", "http://x.test/i", ITEMS, "")),
    ]
}

fn snapshots(target: Target) {
    for (case, req) in matrix() {
        insta::assert_snapshot!(format!("{}-{case}", target.id()), generate(&req, target));
    }
}

#[test]
fn ids_round_trip_and_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for t in Target::ALL {
        assert_eq!(t.id().parse::<Target>(), Ok(*t));
        assert!(seen.insert(t.id()), "duplicate id {}", t.id());
        assert!(!t.label().is_empty());
    }
}

#[test]
fn an_unknown_id_lists_the_valid_ones_without_echoing_the_input() {
    let err: UnknownTarget = "secret-ish".parse::<Target>().unwrap_err();
    let text = err.to_string();
    assert!(!text.contains("secret-ish"), "{text}");
    for t in Target::ALL {
        assert!(text.contains(t.id()), "{text}");
    }
    assert_eq!("Curl".parse::<Target>(), Err(err));
}

#[test]
fn curl_without_send_auth_is_to_curl() {
    for (case, req) in matrix() {
        if case != "digest" && case != "aws" {
            assert_eq!(generate(&req, Target::Curl), to_curl(&req), "{case}");
        }
    }
}

#[test]
fn curl_digest_adds_the_flags_and_aws_is_named_not_dropped() {
    let m = matrix();
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    assert!(
        generate(digest, Target::Curl).ends_with(" \\\n  --digest \\\n  --user 'alice:p'\\''w'"),
        "{}",
        generate(digest, Target::Curl)
    );
    let aws = &m.iter().find(|(c, _)| *c == "aws").unwrap().1;
    assert!(
        generate(aws, Target::Curl)
            .starts_with("# AWS Signature auth is not included in this snippet.\ncurl ")
    );
}

#[test]
fn no_snippet_holds_a_template() {
    for t in Target::ALL {
        for (case, req) in matrix() {
            assert!(!generate(&req, *t).contains("{{"), "{} {case}", t.id());
        }
    }
}

#[test]
fn httpie_escapes_request_item_separators() {
    let m = matrix();
    let items = &m.iter().find(|(c, _)| *c == "items").unwrap().1;
    let text = generate(items, Target::Httpie);
    for item in [r"'X-E:\=v'", r"'a=\@x'", r"'k\=v=1'"] {
        assert!(text.contains(item), "{item} in {text}");
    }
}

#[test]
fn curl_snapshots() {
    snapshots(Target::Curl);
}

#[test]
fn httpie_snapshots() {
    snapshots(Target::Httpie);
}

#[test]
fn python_requests_snapshots() {
    snapshots(Target::PythonRequests);
}

#[test]
fn python_sends_text_as_utf8_bytes_and_digest_natively() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::PythonRequests);
    assert!(out.contains(".encode('utf-8')"), "{out}");
    assert!(out.contains("\\t") && out.contains("\\n"), "{out}");
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    let out = generate(digest, Target::PythonRequests);
    assert!(
        out.contains("auth=HTTPDigestAuth('alice', 'p\\'w')"),
        "{out}"
    );
    assert!(!out.contains("not included"), "{out}");
}

#[test]
fn js_fetch_snapshots() {
    snapshots(Target::JsFetch);
}

#[test]
fn node_axios_snapshots() {
    snapshots(Target::NodeAxios);
}

#[test]
fn javascript_escapes_line_separators_and_names_digest() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    for t in [Target::JsFetch, Target::NodeAxios] {
        let out = generate(hostile, t);
        assert!(
            out.contains("\\u2028") && !out.contains('\u{2028}'),
            "{out}"
        );
        assert!(!out.contains('`'), "{out}");
    }
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    assert!(
        generate(digest, Target::JsFetch)
            .starts_with("// Digest auth is not included in this snippet.\n")
    );
}

#[test]
fn go_snapshots() {
    snapshots(Target::GoNative);
}

#[test]
fn every_target_is_in_the_picker_in_order() {
    let ids: Vec<&str> = Target::ALL.iter().map(|t| t.id()).collect();
    assert_eq!(
        ids,
        [
            "curl",
            "httpie",
            "python-requests",
            "js-fetch",
            "node-axios",
            "go",
            "powershell",
            "php-curl",
            "rust-reqwest",
            "java-httpclient",
            "csharp-httpclient"
        ]
    );
    let labels: Vec<&str> = Target::ALL.iter().map(|t| t.label()).collect();
    assert_eq!(
        labels,
        [
            "curl",
            "HTTPie",
            "Python (requests)",
            "JavaScript (fetch)",
            "Node.js (axios)",
            "Go (net/http)",
            "PowerShell (Invoke-RestMethod)",
            "PHP (curl)",
            "Rust (reqwest)",
            "Java (HttpClient)",
            "C# (HttpClient)"
        ]
    );
}

#[test]
fn go_starts_with_its_package_and_adds_each_sent_header() {
    let m = matrix();
    let get = &m.iter().find(|(c, _)| *c == "get").unwrap().1;
    let out = generate(get, Target::GoNative);
    assert!(out.starts_with("package main\n"), "{out}");
    // resolve folds repeated names (last value wins), so one Cookie header reaches the snippet.
    assert_eq!(out.matches("req.Header.Add(\"Cookie\"").count(), 1, "{out}");
}

#[test]
fn go_writes_a_typed_text_part_and_escapes_a_byte_order_mark() {
    let req = typed();
    let out = generate(&req, Target::GoNative);
    assert!(
        !out.contains('\u{feff}') && out.contains("\\ufeff"),
        "{out}"
    );
    assert!(
        out.contains("header.Set(\"Content-Type\", \"application/json\")"),
        "{out}"
    );
    assert!(out.contains("form.WriteField(\"t\", \"plain\")"), "{out}");
    insta::assert_snapshot!("go-typed", out);
    // curl's snippet of the same request is the execution check's reference.
    insta::assert_snapshot!("curl-typed", generate(&req, Target::Curl));
}

#[test]
fn go_puts_the_omitted_auth_line_apart_from_the_package_clause() {
    let m = matrix();
    let aws = &m.iter().find(|(c, _)| *c == "aws").unwrap().1;
    let out = generate(aws, Target::GoNative);
    assert!(
        out.starts_with("// AWS Signature auth is not included in this snippet.\n\npackage main\n"),
        "{out}"
    );
}

#[test]
fn oauth1_is_named_not_signed_in_every_snippet() {
    let req = resolved_one(
        "GET",
        "https://x.test/o",
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs\n",
        "",
    );
    for t in Target::ALL {
        let code = generate(&req, *t);
        assert!(
            code.contains("OAuth 1.0 auth is not included in this snippet."),
            "{}: {code}",
            t.id()
        );
        assert!(!code.contains("oauth_signature"), "{}", t.id());
    }
}

#[test]
fn curl_writes_a_part_name_holding_an_equals_sign_through_its_own_disposition() {
    let m = matrix();
    let items = &m.iter().find(|(c, _)| *c == "items").unwrap().1;
    let out = generate(items, Target::Curl);
    assert!(
        out.contains(r#"--form 'x=1;headers="Content-Disposition: form-data; name=\"k=v\""'"#),
        "{out}"
    );
    assert!(!out.contains("'k=v=1'"), "{out}");
    assert!(out.contains(r#"--form 'a="@x"'"#), "{out}");

    let file = resolved_one(
        "POST",
        "http://x.test/e",
        "  body:\n    type: multipart-form\n    data:\n      - name: f=1\n        type: file\n        value: [/data/up.bin]\n        contentType: text/plain\n",
        "",
    );
    let out = generate(&file, Target::Curl);
    assert!(
        out.contains(r#"--form 'x=@/data/up.bin;type=text/plain;headers="Content-Disposition: form-data; name=\"f=1\"; filename=\"up.bin\""'"#),
        "{out}"
    );
}

#[test]
fn javascript_keeps_a_typed_text_part_by_writing_the_body() {
    for t in [Target::JsFetch, Target::NodeAxios] {
        let out = generate(&mixed(), t);
        assert!(!out.contains("new FormData()"), "{out}");
        assert!(out.contains("Buffer.concat(["), "{out}");
        assert!(
            out.contains(
                r#"Content-Disposition: form-data; name="j"\r\nContent-Type: application/json\r\n"#
            ),
            "{out}"
        );
        assert!(out.contains("name=\"q%22x\""), "{out}");
        assert!(out.contains("fs.readFileSync('/data/up.bin')"), "{out}");
        assert!(
            out.contains(
                "'Content-Type': 'multipart/form-data; boundary=apinata-boundary-5f0c2e9a7d4b'"
            ),
            "{out}"
        );
        insta::assert_snapshot!(format!("{}-mixed", t.id()), out);
        insta::assert_snapshot!(format!("{}-typed", t.id()), generate(&typed(), t));
    }
}

#[test]
fn javascript_without_a_typed_text_part_keeps_form_data() {
    let m = matrix();
    let multipart = &m.iter().find(|(c, _)| *c == "multipart").unwrap().1;
    for t in [Target::JsFetch, Target::NodeAxios] {
        let out = generate(multipart, t);
        assert!(out.contains("new FormData()"), "{out}");
        assert!(!out.contains("Buffer.concat"), "{out}");
    }
}

#[test]
fn curl_percent_encodes_a_quote_in_a_part_name_holding_an_equals_sign() {
    let req = resolved_one(
        "POST",
        "http://x.test/e",
        "  body:\n    type: multipart-form\n    data:\n      - name: 'a\"=b'\n        type: text\n        value: \"1\"\n",
        "",
    );
    let out = generate(&req, Target::Curl);
    assert!(out.contains(r#"name=\"a%22=b\""#), "{out}");
}

fn toolchain(tool: &str) -> bool {
    if std::env::var_os("APINATA_CODEGEN_TOOLCHAINS").is_none() {
        return false;
    }
    let found = std::process::Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("{tool} is not installed; check skipped");
    }
    found
}

/// Writes each matrix case plus `typed` and `mixed` for `target` to `file_name` in a fresh
/// directory and runs `check` on it; a failing check names the case and the tool output.
fn toolchain_check(
    target: Target,
    file_name: &str,
    check: impl Fn(&std::path::Path) -> std::process::Output,
) {
    let mut cases = matrix();
    cases.push(("typed", typed()));
    cases.push(("mixed", mixed()));
    for (case, req) in cases {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(file_name);
        std::fs::write(&file, generate(&req, target)).unwrap();
        let out = check(&file);
        assert!(
            out.status.success(),
            "{} {case}: {}{}",
            target.id(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn java_snapshots() {
    snapshots(Target::JavaHttpClient);
    insta::assert_snapshot!(
        "java-httpclient-typed",
        generate(&typed(), Target::JavaHttpClient)
    );
    insta::assert_snapshot!(
        "java-httpclient-mixed",
        generate(&mixed(), Target::JavaHttpClient)
    );
}

#[test]
fn java_escapes_without_unicode_line_breaks_and_names_skipped_headers() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::JavaHttpClient);
    assert!(out.contains("\\t") && out.contains("\\n"), "{out}");
    assert!(
        !out.contains("\\u000a") && !out.contains("\\u000A"),
        "{out}"
    );
    assert!(out.contains("public class Main"), "{out}");
    // A control character followed by a digit must keep its own fixed-width escape.
    let ctl = resolved_one(
        "POST",
        "http://x.test/c",
        "  body:\n    type: text\n    data: \"\\x017\"\n",
        "",
    );
    let out = generate(&ctl, Target::JavaHttpClient);
    assert!(out.contains("\"\\u00017\""), "{out}");
    let host = resolved_one(
        "GET",
        "http://x.test/h",
        "  headers:\n    - name: Host\n      value: y.test\n    - name: X-A\n      value: \"1\"\n",
        "",
    );
    let out = generate(&host, Target::JavaHttpClient);
    assert!(
        out.starts_with("// Java's HttpClient sets these headers itself: Host.\n"),
        "{out}"
    );
    assert!(!out.contains(".header(\"Host\""), "{out}");
    assert!(out.contains(".header(\"X-A\", \"1\")"), "{out}");
    let m = matrix();
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    assert!(
        generate(digest, Target::JavaHttpClient)
            .starts_with("// Digest auth is not included in this snippet.\n")
    );
}

#[test]
fn java_compiles_with_javac() {
    if !toolchain("javac") {
        return;
    }
    toolchain_check(Target::JavaHttpClient, "Main.java", |file| {
        std::process::Command::new("javac")
            .arg("-encoding")
            .arg("UTF-8")
            .arg("-d")
            .arg(file.parent().unwrap().join("out"))
            .arg(file)
            .output()
            .unwrap()
    });
}

#[test]
fn java_percent_encodes_what_java_net_uri_refuses_in_a_url() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::JavaHttpClient);
    assert!(
        out.contains(r#"URI.create("http://x.test/p?x=a%27b$c%7Bd%7D&y=%C3%BC")"#),
        "{out}"
    );
    let req = resolved_one("GET", "http://[::1]:8/a|b?q=[1]^%zz#f#g", "", "");
    let out = generate(&req, Target::JavaHttpClient);
    assert!(
        out.contains(r#"URI.create("http://[::1]:8/a%7Cb?q=%5B1%5D%5E%25zz#f%23g")"#),
        "{out}"
    );
}

#[test]
fn java_names_a_non_ascii_header_instead_of_sending_it_mangled() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::JavaHttpClient);
    assert!(
        out.starts_with(
            "// Java's HttpClient sends header names and values as ASCII only; left out: X-H.\n"
        ),
        "{out}"
    );
    assert!(!out.contains(".header(\"X-H\""), "{out}");
    assert!(
        out.contains(".header(\"content-type\", \"text/plain\")"),
        "{out}"
    );
}

#[test]
fn java_note_cannot_end_its_comment_through_a_unicode_escape() {
    let req = resolved_one(
        "GET",
        "http://x.test/n",
        "  headers:\n    - name: 'X-\\u000aB'\n      value: ü\n",
        "",
    );
    let out = generate(&req, Target::JavaHttpClient);
    assert!(
        out.starts_with(
            "// Java's HttpClient sends header names and values as ASCII only; left out: X-\\\\u000aB.\n"
        ),
        "{out}"
    );
}

#[test]
fn csharp_snapshots() {
    snapshots(Target::CSharpHttpClient);
    insta::assert_snapshot!(
        "csharp-httpclient-typed",
        generate(&typed(), Target::CSharpHttpClient)
    );
    insta::assert_snapshot!(
        "csharp-httpclient-mixed",
        generate(&mixed(), Target::CSharpHttpClient)
    );
}

#[test]
fn csharp_escapes_line_separators_and_writes_digest_natively() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::CSharpHttpClient);
    assert!(
        out.contains("\\u2028") && !out.contains('\u{2028}'),
        "{out}"
    );
    assert!(out.contains("using System.Net.Http;"), "{out}");
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    let out = generate(digest, Target::CSharpHttpClient);
    assert!(
        out.contains("new NetworkCredential(\"alice\", \"p'w\")"),
        "{out}"
    );
    assert!(!out.contains("not included"), "{out}");
    let get = resolved_one(
        "GET",
        "http://x.test/c",
        "  headers:\n    - name: Content-Type\n      value: text/plain\n",
        "",
    );
    let out = generate(&get, Target::CSharpHttpClient);
    assert!(
        out.starts_with("// HttpClient sends content headers only with a body: Content-Type.\n"),
        "{out}"
    );
}

/// Roslyn's `csc.dll` from the last listed SDK and the assemblies of one installed reference pack
/// (whichever sorts last as a string; any pack compiles these snippets).
fn csharp_compiler() -> Option<(std::path::PathBuf, Vec<std::path::PathBuf>)> {
    let sdks = std::process::Command::new("dotnet")
        .arg("--list-sdks")
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&sdks.stdout)
        .lines()
        .last()?
        .to_owned();
    let (version, rest) = line.split_once(' ')?;
    let sdk_root =
        std::path::PathBuf::from(rest.trim().trim_start_matches('[').trim_end_matches(']'));
    let csc = sdk_root.join(version).join("Roslyn/bincore/csc.dll");
    let packs = sdk_root.parent()?.join("packs/Microsoft.NETCore.App.Ref");
    let newest = |dir: &std::path::Path| {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        entries.pop()
    };
    let refs_dir = newest(&newest(&packs)?.join("ref"))?;
    let refs = std::fs::read_dir(refs_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dll"))
        .collect();
    Some((csc, refs))
}

#[test]
fn csharp_compiles_with_csc() {
    if !toolchain("dotnet") {
        return;
    }
    let (csc, refs) = csharp_compiler().expect("an SDK with Roslyn and a reference pack");
    toolchain_check(Target::CSharpHttpClient, "Program.cs", |file| {
        let dir = file.parent().unwrap();
        let mut cmd = std::process::Command::new("dotnet");
        cmd.env("DOTNET_CLI_HOME", dir)
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .arg(&csc)
            .arg("-nologo")
            .arg("-t:exe")
            .arg(format!("-out:{}", dir.join("p.dll").display()));
        for r in &refs {
            cmd.arg(format!("-r:{}", r.display()));
        }
        cmd.arg(file).output().unwrap()
    });
}

#[test]
fn csharp_sends_a_non_ascii_header_value_as_utf8() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::CSharpHttpClient);
    assert!(
        out.contains("new HttpClient(new SocketsHttpHandler { RequestHeaderEncodingSelector = (_, _) => Encoding.UTF8 });"),
        "{out}"
    );
    assert!(out.contains("using System.Text;"), "{out}");
    let get = &m.iter().find(|(c, _)| *c == "get").unwrap().1;
    let out = generate(get, Target::CSharpHttpClient);
    assert!(out.contains("var client = new HttpClient();"), "{out}");
    let digest = resolved_one(
        "GET",
        "http://x.test/d",
        &format!("  headers:\n    - name: X-U\n      value: ü\n{DIGEST}"),
        "",
    );
    let out = generate(&digest, Target::CSharpHttpClient);
    assert!(
        out.contains("new SocketsHttpHandler { Credentials = new NetworkCredential(\"alice\", \"p'w\"), RequestHeaderEncodingSelector = (_, _) => Encoding.UTF8 }"),
        "{out}"
    );
}

#[test]
fn csharp_percent_encodes_a_quote_in_a_part_name() {
    // MultipartFormDataContent throws on a `"` inside a name.
    let out = generate(&mixed(), Target::CSharpHttpClient);
    assert!(out.contains("form.Add(part2, \"q%22x\");"), "{out}");
}

#[test]
fn csharp_writes_a_non_ascii_part_name_as_utf8() {
    // MultipartFormDataContent would RFC 2047-encode the name; the disposition is written by hand.
    let req = resolved_one(
        "POST",
        "http://x.test/u",
        "  body:\n    type: multipart-form\n    data:\n      - name: ü\n        type: text\n        value: v\n      - name: f\n        type: file\n        value: [/data/é.bin]\n",
        "",
    );
    let out = generate(&req, Target::CSharpHttpClient);
    assert!(
        out.contains("form.HeaderEncodingSelector = (_, _) => Encoding.UTF8;\n"),
        "{out}"
    );
    assert!(
        out.contains("part1.Headers.TryAddWithoutValidation(\"Content-Disposition\", \"form-data; name=\\\"ü\\\"\");\nform.Add(part1);\n"),
        "{out}"
    );
    assert!(
        out.contains("\"form-data; name=\\\"f\\\"; filename=\\\"é.bin\\\"\");\nform.Add(part2);\n"),
        "{out}"
    );
    assert!(out.contains("using System.Text;"), "{out}");
}

#[test]
fn powershell_snapshots() {
    snapshots(Target::PowerShellRestMethod);
    insta::assert_snapshot!(
        "powershell-typed",
        generate(&typed(), Target::PowerShellRestMethod)
    );
    insta::assert_snapshot!(
        "powershell-mixed",
        generate(&mixed(), Target::PowerShellRestMethod)
    );
}

#[test]
fn powershell_leaves_nothing_live_in_a_string() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::PowerShellRestMethod);
    // A value holding a control character is double-quoted, so `$` and the tab must be escaped.
    assert!(
        out.contains("`$e") && out.contains("`t") && out.contains("`n"),
        "{out}"
    );
    assert!(out.contains("#Requires -Version 7"), "{out}");
    // The typographic quote sits in the body: Invoke-RestMethod cannot send a non-ASCII header
    // value, so a header holding one is named in a note instead (asserted below).
    let smart = resolved_one(
        "POST",
        "http://x.test/s",
        "  headers:\n    - name: X-S\n      value: \"it\u{2019}s $x\"\n  body:\n    type: text\n    data: \"it\u{2019}s $x\"\n",
        "",
    );
    let out = generate(&smart, Target::PowerShellRestMethod);
    assert!(out.contains("'it\u{2019}\u{2019}s $x'"), "{out}");
    assert!(
        out.starts_with(
            "# Invoke-RestMethod sends header names and values as ASCII only; left out: X-S.\n"
        ),
        "{out}"
    );
    let custom = resolved_one("PURGE", "http://x.test/p", "", "");
    assert!(generate(&custom, Target::PowerShellRestMethod).contains("-CustomMethod 'PURGE'"));
}

#[test]
fn powershell_writes_a_non_ascii_part_name_as_utf8_bytes() {
    // Part headers are written as Latin-1, so the disposition carries the UTF-8 bytes as chars.
    let req = resolved_one(
        "POST",
        "http://x.test/u",
        "  body:\n    type: multipart-form\n    data:\n      - name: \u{fc}\n        type: text\n        value: v\n",
        "",
    );
    let out = generate(&req, Target::PowerShellRestMethod);
    assert!(
        out.contains("[void]$part1.Headers.TryAddWithoutValidation('Content-Disposition', [System.Text.Encoding]::Latin1.GetString([System.Text.Encoding]::UTF8.GetBytes('form-data; name=\"\u{fc}\"')))\n$body.Add($part1)\n"),
        "{out}"
    );
}

#[test]
fn powershell_parses_with_pwsh() {
    if !toolchain("pwsh") {
        return;
    }
    toolchain_check(Target::PowerShellRestMethod, "snippet.ps1", |file| {
        let check = file.parent().unwrap().join("check.ps1");
        std::fs::write(
            &check,
            "$errors = $null\n[void][System.Management.Automation.Language.Parser]::ParseFile($args[0], [ref]$null, [ref]$errors)\nif ($errors) { $errors; exit 1 }\n",
        )
        .unwrap();
        std::process::Command::new("pwsh")
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(&check)
            .arg(file)
            .output()
            .unwrap()
    });
}

#[test]
fn php_snapshots() {
    snapshots(Target::PhpCurl);
    // `curl_close` is deprecated in PHP 8.5 and a no-op since 8.0.
    for (case, req) in matrix() {
        assert!(
            !generate(&req, Target::PhpCurl).contains("curl_close"),
            "{case}"
        );
    }
    insta::assert_snapshot!("php-curl-typed", generate(&typed(), Target::PhpCurl));
    insta::assert_snapshot!("php-curl-mixed", generate(&mixed(), Target::PhpCurl));
}

#[test]
fn php_escapes_dollars_writes_empty_headers_and_digest_natively() {
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::PhpCurl);
    assert!(out.starts_with("<?php\n"), "{out}");
    assert!(out.contains("\\$e") && out.contains("\\$HOME"), "{out}");
    let empty = resolved_one(
        "GET",
        "http://x.test/e",
        "  headers:\n    - name: X-Empty\n      value: \"\"\n",
        "",
    );
    assert!(generate(&empty, Target::PhpCurl).contains("\"X-Empty;\""));
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    let out = generate(digest, Target::PhpCurl);
    assert!(out.contains("CURLAUTH_DIGEST"), "{out}");
    assert!(!out.contains("not included"), "{out}");
    let repeated = resolved_one(
        "POST",
        "http://x.test/r",
        "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: a\n        type: text\n        value: \"2\"\n",
        "",
    );
    let out = generate(&repeated, Target::PhpCurl);
    assert!(
        out.contains("boundary=apinata-boundary-5f0c2e9a7d4b"),
        "{out}"
    );
}

#[test]
fn php_lints_with_php() {
    if !toolchain("php") {
        return;
    }
    toolchain_check(Target::PhpCurl, "snippet.php", |file| {
        std::process::Command::new("php")
            .arg("-l")
            .arg(file)
            .output()
            .unwrap()
    });
}

#[test]
fn rust_snapshots() {
    snapshots(Target::RustReqwest);
    insta::assert_snapshot!(
        "rust-reqwest-typed",
        generate(&typed(), Target::RustReqwest)
    );
    insta::assert_snapshot!(
        "rust-reqwest-mixed",
        generate(&mixed(), Target::RustReqwest)
    );
}

#[test]
fn rust_names_its_features_and_types_text_parts() {
    let out = generate(&mixed(), Target::RustReqwest);
    assert!(
        out.contains("features = [\"blocking\", \"multipart\"]"),
        "{out}"
    );
    assert!(out.contains(".mime_str(\"application/json\")?"), "{out}");
    let m = matrix();
    let hostile = &m.iter().find(|(c, _)| *c == "hostile").unwrap().1;
    let out = generate(hostile, Target::RustReqwest);
    assert!(out.contains("HeaderValue::from_bytes("), "{out}");
    let digest = &m.iter().find(|(c, _)| *c == "digest").unwrap().1;
    assert!(
        generate(digest, Target::RustReqwest)
            .starts_with("// Digest auth is not included in this snippet.\n")
    );
}

/// A one-part form and literals far past rustfmt's widths, which the fixed layout still writes.
fn rust_layout_cases() -> Vec<ResolvedRequest> {
    let long = "x".repeat(120);
    vec![
        resolved_one(
            "POST",
            "http://x.test/o",
            "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: b\n",
            "",
        ),
        resolved_one(
            "POST",
            &format!("http://x.test/{long}"),
            &format!(
                "  headers:\n    - name: X-U\n      value: \"ü{long}\"\n  body:\n    type: text\n    data: \"{long}\"\n"
            ),
            "",
        ),
        resolved_one(
            "POST",
            "http://x.test/l",
            &format!(
                "  body:\n    type: multipart-form\n    data:\n      - name: j\n        type: text\n        value: \"{long}\"\n        contentType: application/json\n      - name: up\n        type: file\n        value: [/data/{long}.bin]\n"
            ),
            "",
        ),
        resolved_one(
            "PUT",
            "http://x.test/f",
            &format!(
                "  body:\n    type: file\n    data:\n      - filePath: /data/{long}.json\n        contentType: application/json\n        selected: true\n"
            ),
            "",
        ),
    ]
}

#[test]
fn rust_writes_each_link_on_its_own_line_with_inline_arguments() {
    let cases = rust_layout_cases();
    let out = generate(&cases[0], Target::RustReqwest);
    assert!(
        out.contains("    let form = Form::new()\n        .percent_encode_noop()\n        .text(\"a\", \"b\");\n"),
        "{out}"
    );
    let long = "x".repeat(120);
    let out = generate(&cases[1], Target::RustReqwest);
    assert!(
        out.contains(&format!("        .body(\"{long}\")\n")),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "        .request(reqwest::Method::POST, \"http://x.test/{long}\")\n"
        )),
        "{out}"
    );
}

#[test]
fn rust_parses_with_rustfmt() {
    if !toolchain("rustfmt") {
        return;
    }
    let parse = |file: &std::path::Path| {
        std::process::Command::new("rustfmt")
            .args(["--edition", "2024", "--emit", "stdout"])
            .arg(file)
            .output()
            .unwrap()
    };
    toolchain_check(Target::RustReqwest, "main.rs", parse);
    for (i, req) in rust_layout_cases().iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, generate(req, Target::RustReqwest)).unwrap();
        let out = parse(&file);
        assert!(
            out.status.success(),
            "layout case {i}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn an_untyped_file_part_gets_the_type_its_extension_looks_up_in_every_target() {
    // The typed text part makes PHP, fetch and axios hand-build the body.
    let req = resolved_one(
        "POST",
        "http://x.test/u",
        "  body:\n    type: multipart-form\n    data:\n      - name: j\n        type: text\n        value: '{}'\n        contentType: application/json\n      - name: up\n        type: file\n        value: [/data/up.png]\n",
        "",
    );
    for t in [
        Target::JavaHttpClient,
        Target::CSharpHttpClient,
        Target::PowerShellRestMethod,
        Target::RustReqwest,
        Target::PhpCurl,
        Target::JsFetch,
        Target::NodeAxios,
    ] {
        let out = generate(&req, t);
        assert!(out.contains("image/png"), "{}: {out}", t.id());
    }
}

#[test]
fn a_non_ascii_header_name_is_named_in_the_same_note_shape() {
    let req = resolved_one(
        "GET",
        "http://x.test/n",
        "  headers:\n    - name: X-\u{fc}\n      value: v\n",
        "",
    );
    assert!(generate(&req, Target::JavaHttpClient).starts_with(
        "// Java's HttpClient sends header names and values as ASCII only; left out: X-\u{fc}.\n"
    ));
    assert!(generate(&req, Target::PowerShellRestMethod).starts_with(
        "# Invoke-RestMethod sends header names and values as ASCII only; left out: X-\u{fc}.\n"
    ));
}

#[test]
fn rust_imports_part_only_when_a_part_needs_it() {
    let m = matrix();
    let items = &m.iter().find(|(c, _)| *c == "items").unwrap().1;
    let out = generate(items, Target::RustReqwest);
    assert!(
        out.contains("use reqwest::blocking::multipart::Form;\n") && !out.contains("Part"),
        "{out}"
    );
    let multipart = &m.iter().find(|(c, _)| *c == "multipart").unwrap().1;
    assert!(
        generate(multipart, Target::RustReqwest)
            .contains("use reqwest::blocking::multipart::{Form, Part};\n")
    );
}
