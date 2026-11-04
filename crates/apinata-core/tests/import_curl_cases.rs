//! curl cases from Bruno's three curl spec files and extra probes, with each difference named.

use apinata_core::import::{CurlError, CurlSyntax, NoteKind, parse_curl};
use apinata_core::{AuthSpec, BodySpec, FileEntry, KeyValue, MultipartField, ParamKind};

type Pairs = &'static [(&'static str, &'static str)];

struct Case {
    id: &'static str,
    input: &'static str,
    differs: Option<&'static str>,
    method: &'static str,
    url: &'static str,
    headers: Pairs,
    query: Pairs,
    body: BodySpec,
    auth: AuthSpec,
    notes: Vec<(&'static str, NoteKind)>,
}

fn case(id: &'static str, input: &'static str, method: &'static str, url: &'static str) -> Case {
    Case {
        id,
        input,
        differs: None,
        method,
        url,
        headers: &[],
        query: &[],
        body: BodySpec::None,
        auth: AuthSpec::Inherit,
        notes: vec![],
    }
}

impl Case {
    fn differs(mut self, why: &'static str) -> Case {
        self.differs = Some(why);
        self
    }
    fn headers(mut self, headers: Pairs) -> Case {
        self.headers = headers;
        self
    }
    fn query(mut self, query: Pairs) -> Case {
        self.query = query;
        self
    }
    fn body(mut self, body: BodySpec) -> Case {
        self.body = body;
        self
    }
    fn auth(mut self, auth: AuthSpec) -> Case {
        self.auth = auth;
        self
    }
    fn note(mut self, source: &'static str, kind: NoteKind) -> Case {
        self.notes.push((source, kind));
        self
    }
}

fn json(text: &str) -> BodySpec {
    BodySpec::Json(text.into())
}
fn text(text: &str) -> BodySpec {
    BodySpec::Text(text.into())
}
fn form(pairs: Pairs) -> BodySpec {
    BodySpec::FormUrlEncoded(pairs.iter().map(|(k, v)| KeyValue::new(*k, *v)).collect())
}
/// A value starting with `@` is a file part.
fn multipart(pairs: Pairs) -> BodySpec {
    BodySpec::Multipart(
        pairs
            .iter()
            .map(|(k, v)| match v.strip_prefix('@') {
                Some(path) => MultipartField::files(*k, vec![path.to_owned()]),
                None => MultipartField::text(*k, *v),
            })
            .collect(),
    )
}
fn file(path: &str, content_type: &str) -> BodySpec {
    let mut entry = FileEntry::new(path);
    entry.content_type = content_type.into();
    entry.selected = true;
    BodySpec::File(vec![entry])
}
fn basic(user: &str, password: &str) -> AuthSpec {
    AuthSpec::Basic {
        username: user.into(),
        password: password.into(),
    }
}

fn check(case: &Case) {
    let parsed = parse_curl(case.input).unwrap_or_else(|e| panic!("{}: {e}", case.id));
    let r = &parsed.request;
    assert_eq!(
        (r.method.as_str(), r.url.as_str()),
        (case.method, case.url),
        "{}",
        case.id
    );
    let headers: Vec<(&str, &str)> = r
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    assert_eq!(headers, case.headers, "{}", case.id);
    assert!(
        r.params.iter().all(|p| p.kind == ParamKind::Query),
        "{}: no path params",
        case.id
    );
    let query: Vec<(&str, &str)> = r
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.value.as_str()))
        .collect();
    assert_eq!(query, case.query, "{}", case.id);
    assert_eq!(r.body, case.body, "{}", case.id);
    assert_eq!(r.auth, case.auth, "{}", case.id);
    let notes: Vec<(&str, NoteKind)> = parsed
        .notes
        .iter()
        .map(|n| (n.source.as_str(), n.kind))
        .collect();
    assert_eq!(notes, case.notes, "{}", case.id);
    assert!(
        parsed
            .notes
            .iter()
            .all(|n| !n.detail.to_lowercase().contains("bruno")),
        "{}: user-facing text",
        case.id
    );
}

const FORM_TYPE: Pairs = &[("Content-Type", "application/x-www-form-urlencoded")];
const NOT_A_FORM: &str =
    "Bruno makes one form field named after the whole text; curl sends it as is";
const NO_MULTIPART_HEADER: &str = "Bruno adds a multipart Content-Type header without a boundary";

fn bruno_cases() -> Vec<Case> {
    use NoteKind::{Converted, Dropped, NeedsInput};
    vec![
        case("get", "curl https://api.example.com/users", "GET", "https://api.example.com/users"),
        case("header-no-space", r#"curl --header "Content-Type:application/json" https://api.example.com"#, "GET", "https://api.example.com")
            .headers(&[("Content-Type", "application/json")]),
        case("json-data", r#"curl -d '{"name": "John", "age": 30}' https://api.example.com/users"#, "POST", "https://api.example.com/users")
            .headers(FORM_TYPE).body(text(r#"{"name": "John", "age": 30}"#)).note("curl flag -d", Converted).differs(NOT_A_FORM),
        case("multiple-data", "curl -d \"name=John\" \\\n     -d \"age=30\" \\\n     https://api.example.com/users", "POST", "https://api.example.com/users")
            .body(form(&[("name", "John"), ("age", "30")])),
        case("binary-file", r#"curl --data-binary "@/path/to/file" https://api.example.com/upload"#, "POST", "https://api.example.com/upload")
            .body(file("/path/to/file", "")).differs("without a Content-Type Bruno turns the file reference into form fields"),
        // index.spec.js: "should treat --data-binary with inline JSON as a JSON body".
        case("binary-inline-json", r#"curl -H "Content-Type: application/json; charset=UTF-8" --data-binary "{\"pageUri\":\"/mobile-phones-store\"}" "https://1.rome.api.flipkart.net/4/page/fetch""#, "POST", "https://1.rome.api.flipkart.net/4/page/fetch")
            .headers(&[("Content-Type", "application/json; charset=UTF-8")]).body(json("{\n  \"pageUri\": \"/mobile-phones-store\"\n}")),
        case("binary-file-typed", r#"curl -H "Content-Type: application/octet-stream" --data-binary "@/path/to/payload.json" "https://example.com/upload""#, "POST", "https://example.com/upload")
            .headers(&[("Content-Type", "application/octet-stream")]).body(file("/path/to/payload.json", "application/octet-stream")),
        case("user-only", r#"curl --user "username" https://api.example.com"#, "GET", "https://api.example.com")
            .auth(basic("username", "")).note("curl flag --user", NeedsInput).differs("Bruno adds no note; curl would prompt for the password"),
        case("digest", r#"curl --digest -u "myuser:mypass" https://api.example.com/digest"#, "GET", "https://api.example.com/digest")
            .auth(AuthSpec::Digest { username: "myuser".into(), password: "mypass".into() }),
        case("ntlm-domain", r#"curl --ntlm --user "domain\\username:password" https://api.example.com/ntlm"#, "GET", "https://api.example.com/ntlm")
            .auth(AuthSpec::Ntlm { username: "username".into(), password: "password".into(), domain: "domain".into() })
            .differs("Bruno keeps domain\\username whole as the user name and has no domain field; core splits the domain off"),
        case("form-text", "curl -F \"name=John\" \\\n     -F \"age=30\" \\\n     https://api.example.com/users", "POST", "https://api.example.com/users")
            .body(multipart(&[("name", "John"), ("age", "30")])).differs(NO_MULTIPART_HEADER),
        case("form-file", r#"curl --form "file=@/path/to/file.txt" https://api.example.com/upload"#, "POST", "https://api.example.com/upload")
            .body(multipart(&[("file", "@/path/to/file.txt")])).differs(NO_MULTIPART_HEADER),
        case("cookies", r#"curl -b "session=abc123" -b "user=john" https://api.example.com"#, "GET", "https://api.example.com")
            .headers(&[("Cookie", "session=abc123; user=john")]),
        case("quote-pattern", r#"curl -d '{"name": "John'\''s data"}' https://api.example.com"#, "POST", "https://api.example.com")
            .headers(FORM_TYPE).body(text(r#"{"name": "John's data"}"#)).note("curl flag -d", Converted).differs(NOT_A_FORM),
        case("query", "curl https://api.example.com/users?page=1&limit=10&sort=asc", "GET", "https://api.example.com/users?page=1&limit=10&sort=asc")
            .query(&[("page", "1"), ("limit", "10"), ("sort", "asc")]),
        case("no-protocol", "curl echo.usebruno.com", "GET", "https://echo.usebruno.com"),
        case("json-flag", r#"curl --json '{"name": "John Doe", "email": "john@example.com"}' https://api.example.com/users"#, "POST", "https://api.example.com/users")
            .headers(&[("Content-Type", "application/json")]).body(json("{\n  \"name\": \"John Doe\",\n  \"email\": \"john@example.com\"\n}")),
        case("insecure", "curl -k https://api.example.com", "GET", "https://api.example.com")
            .note("curl flag -k", Dropped).differs("Bruno drops -k silently"),
        case("get-data", r#"curl -G -d "name=John" -d "age=30" https://api.example.com/users"#, "GET", "https://api.example.com/users?name=John&age=30")
            .query(&[("name", "John"), ("age", "30")]),
        case("get-urlencode", "curl -G --data-urlencode \"name=John Doe\" \\\n  --data-urlencode \"email=john@example.com\" \\\n  --data-urlencode \"hello\" \\\n  https://api.example.com/users?test=urlquery&hello", "GET", "https://api.example.com/users?test=urlquery&hello&name=John+Doe&email=john%40example.com&hello")
            .query(&[("test", "urlquery"), ("hello", ""), ("name", "John+Doe"), ("email", "john%40example.com"), ("hello", "")])
            .differs("Bruno drops the repeated `hello`, encodes the space as `%20` and leaves `@` unencoded; curl gives `+` and `%40`"),
        // curl-to-json.spec.js: "should accept escaped curl string", after JavaScript's own unescaping.
        case("ansi-c-cookie", "curl https://www.usebruno.com\n    -H $'cookie: val_1=\\'\\'; val_2=\\^373:0\\^373:0; val_3=hello'", "GET", "https://www.usebruno.com")
            .headers(&[("cookie", r"val_1=''; val_2=\^373:0\^373:0; val_3=hello")]),
        case("custom-json-type", "curl 'https://api.example.com/test'\n    -H 'content-type: application/x.custom+json;version=1'\n    --data-raw '{\"test\":\"data\"}'", "POST", "https://api.example.com/test")
            .headers(&[("content-type", "application/x.custom+json;version=1")]).body(json("{\n  \"test\": \"data\"\n}")),
    ]
}

#[test]
fn every_bruno_case_maps_as_stated() {
    for case in bruno_cases() {
        check(&case);
    }
}

fn probes() -> Vec<Case> {
    use NoteKind::Dropped;
    vec![
        case("probe-templates", r#"curl '{{baseUrl}}/users/{{id}}?q={{q}}' -H 'Authorization: Bearer {{token}}' -H 'Content-Type: application/json' -d '{"id": {{id}}}'"#, "POST", "{{baseUrl}}/users/{{id}}?q={{q}}")
            .headers(&[("Authorization", "Bearer {{token}}"), ("Content-Type", "application/json")]).query(&[("q", "{{q}}")])
            .body(json("{\n  \"id\": {{id}}\n}")).differs("Bruno percent-encodes templates and finds no URL in `{{baseUrl}}/users`"),
        case("probe-shell-vars", r#"curl "https://api.example.com/$PART" -H "X-Token: ${TOKEN}" -H 'X-Raw: $RAW'"#, "GET", "https://api.example.com/$PART")
            .headers(&[("X-Token", "${TOKEN}"), ("X-Raw", "$RAW")]).differs("Bruno empties `$VARS`"),
        case("probe-user-colons", "curl -u 'a:b:c' https://api.example.com", "GET", "https://api.example.com")
            .auth(basic("a", "b:c")).differs("Bruno cuts the password at the second colon"),
        case("probe-unknown-flags", "curl -sSL --max-time 10 -o out.json https://api.example.com/x", "GET", "https://api.example.com/x")
            .note("curl flag -s", Dropped).note("curl flag -S", Dropped)
            .note("curl flag -o", Dropped)
            .differs("Bruno skips unknown flags silently and takes `out.json` as the URL"),
        case("probe-chrome-bash", "curl 'https://api.example.com/v1/items?limit=10' \\\n  -H 'accept: application/json' \\\n  -H 'authorization: Bearer {{token}}' \\\n  -H 'content-type: application/json' \\\n  --data-raw $'{\"name\":\"it\\'s\",\"n\":1e3}' \\\n  --compressed", "POST", "https://api.example.com/v1/items?limit=10")
            .headers(&[("accept", "application/json"), ("authorization", "Bearer {{token}}"), ("content-type", "application/json"), ("Accept-Encoding", "deflate, gzip")])
            .query(&[("limit", "10")]).body(json("{\n  \"name\": \"it's\",\n  \"n\": 1e3\n}")),
        case("probe-duplicate-headers", "curl -H 'X-A: 1' -H 'X-A: 2' https://api.example.com", "GET", "https://api.example.com")
            .headers(&[("X-A", "1"), ("X-A", "2")]).differs("Bruno keeps one header per name"),
        case("probe-form-keeps-method", "curl -X PUT -F 'a=1' https://api.example.com", "PUT", "https://api.example.com")
            .body(multipart(&[("a", "1")])).differs("Bruno's -F resets the method to POST"),
        case("probe-unknown-content-type", "curl -H 'Content-Type: application/octet-stream' -d 'raw' https://api.example.com", "POST", "https://api.example.com")
            .headers(&[("Content-Type", "application/octet-stream")]).body(text("raw")).differs("Bruno leaves the body empty"),
        case("probe-pipe", "curl https://api.example.com/x | jq .", "GET", "https://api.example.com/x")
            .note("shell", Dropped).differs("Bruno ignores the pipe silently"),
        case("probe-multipart-data", "curl -H 'Content-Type: multipart/form-data' -d 'a=1' https://api.example.com", "POST", "https://api.example.com")
            .headers(&[("Content-Type", "multipart/form-data")]).body(text("a=1"))
            .differs("Bruno stores the raw data as the multipart part list; curl sends the text as is"),
        case("probe-invalid-json", r#"curl --json '{"a": 1,}' https://api.example.com"#, "POST", "https://api.example.com")
            .headers(&[("Content-Type", "application/json")]).body(json(r#"{"a": 1,}"#))
            .differs("Bruno's tolerant formatter re-indents text that is not JSON; it is kept as written"),
    ]
}

#[test]
fn every_probe_maps_as_stated() {
    for case in probes() {
        check(&case);
    }
}

#[test]
fn every_difference_has_a_reason_and_ids_are_unique() {
    let all: Vec<Case> = bruno_cases().into_iter().chain(probes()).collect();
    let mut ids: Vec<&str> = all.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all.len());
    assert!(
        all.iter()
            .filter_map(|c| c.differs)
            .all(|why| !why.trim().is_empty())
    );
    assert!(
        probes()
            .iter()
            .all(|c| c.differs.is_some() || c.id == "probe-chrome-bash")
    );
}

#[test]
fn shell_forms_bruno_guesses_at_are_errors() {
    let form = |c: &str| match parse_curl(c) {
        Err(CurlError::Syntax { form, .. }) => form,
        other => panic!("{c}: {other:?}"),
    };
    // parse-curl.spec.js "should handle complex escaped quotes": Bruno rewrites `\'` inside single quotes.
    assert_eq!(
        form(r#"curl -d '{"message": "Don\'t stop believing"}' https://api.example.com"#),
        CurlSyntax::UnclosedQuote
    );
    assert_eq!(
        form("curl ^\"https://api.example.com/^\" ^\n  -H ^\"accept: */*^\""),
        CurlSyntax::Cmd
    );
    assert_eq!(
        form("curl https://api.example.com `\n  -H 'accept: */*'"),
        CurlSyntax::PowerShell
    );
}
