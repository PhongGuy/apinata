//! `parse_curl`: the request line, headers, auth, settings and the notes for what does not map.

use apinata_core::import::{CurlError, CurlSyntax, NoteKind, ParsedCurl, parse_curl};
use apinata_core::{AuthSpec, ParamKind, RequestSettings, Timeout};

fn parse(command: &str) -> ParsedCurl {
    parse_curl(command).unwrap_or_else(|e| panic!("{command}: {e}"))
}
fn h(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}
fn n(source: &str, kind: NoteKind) -> (String, NoteKind) {
    (source.to_owned(), kind)
}
fn headers(parsed: &ParsedCurl) -> Vec<(String, String)> {
    parsed
        .request
        .headers
        .iter()
        .map(|x| h(&x.name, &x.value))
        .collect()
}
fn notes(parsed: &ParsedCurl) -> Vec<(String, NoteKind)> {
    parsed.notes.iter().map(|x| n(&x.source, x.kind)).collect()
}

#[test]
fn method_follows_x_then_body_then_head() {
    let method = |c: &str| parse(c).request.method;
    assert_eq!(method("curl https://x.test"), "GET");
    assert_eq!(method("curl -X delete https://x.test"), "DELETE");
    assert_eq!(method("curl -I https://x.test"), "HEAD");
    assert_eq!(method("curl -d a=1 https://x.test"), "POST");
    assert_eq!(method("curl -I -d a=1 https://x.test"), "POST");
    assert_eq!(method("curl -F a=1 https://x.test"), "POST");
    assert_eq!(method("curl -X PUT -F a=1 https://x.test"), "PUT");
    assert_eq!(method("curl -G -d a=1 https://x.test"), "GET");
}

#[test]
fn the_url_keeps_templates_and_gains_https_only_without_a_scheme() {
    let url = |c: &str| parse(c).request.url;
    assert_eq!(
        url("curl echo.usebruno.com/a?b=1"),
        "https://echo.usebruno.com/a?b=1"
    );
    assert_eq!(url("/usr/bin/curl http://x.test"), "http://x.test");
    assert_eq!(
        url("curl '{{baseUrl}}/users/{{id}}'"),
        "{{baseUrl}}/users/{{id}}"
    );
    assert_eq!(url("curl --url https://a.test"), "https://a.test");
    assert_eq!(
        url("curl \"https://x.test/$PART?t=${T}\""),
        "https://x.test/$PART?t=${T}"
    );
}

#[test]
fn params_come_from_the_url_query_then_its_path() {
    let parsed = parse("curl 'https://x.test/users/:id?page=1&flag&q={{q}}'");
    let params: Vec<(&str, &str, ParamKind)> = parsed
        .request
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.value.as_str(), p.kind))
        .collect();
    assert_eq!(
        params,
        [
            ("page", "1", ParamKind::Query),
            ("flag", "", ParamKind::Query),
            ("q", "{{q}}", ParamKind::Query),
            ("id", "", ParamKind::Path),
        ]
    );
}

#[test]
fn headers_keep_order_duplicates_and_shell_text() {
    let parsed = parse(
        r#"curl https://x.test -H 'X-A: 1' -H 'X-A: 2' -H "X-T: ${TOKEN}" -H 'Accept:application/json' -H 'X-Empty;' -A 'Agent 1' -b 'a=1' -b 'b=2' --compressed"#,
    );
    assert_eq!(
        headers(&parsed),
        [
            h("X-A", "1"),
            h("X-A", "2"),
            h("X-T", "${TOKEN}"),
            h("Accept", "application/json"),
            h("X-Empty", ""),
            h("User-Agent", "Agent 1"),
            h("Cookie", "a=1; b=2"),
            h("Accept-Encoding", "deflate, gzip"),
        ]
    );
    assert!(parsed.notes.is_empty());
}

#[test]
fn headers_curl_would_not_send_are_noted() {
    let parsed = parse(
        "curl https://x.test -H 'Host:' -H @headers.txt -H junk -b cookies.txt --compressed -H 'accept-encoding: br'",
    );
    assert_eq!(headers(&parsed), [h("accept-encoding", "br")]);
    let dropped = NoteKind::Dropped;
    assert_eq!(
        notes(&parsed),
        [
            n("curl flag -H", dropped),
            n("curl flag -H", dropped),
            n("curl flag -H", dropped),
            n("curl flag -b", dropped)
        ]
    );
}

#[test]
fn user_splits_on_the_first_colon() {
    assert_eq!(
        parse("curl -u 'a:b:c' https://x.test").request.auth,
        AuthSpec::Basic {
            username: "a".into(),
            password: "b:c".into()
        }
    );
    let parsed = parse("curl --user alice https://x.test");
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Basic {
            username: "alice".into(),
            password: String::new()
        }
    );
    assert_eq!(
        notes(&parsed),
        [n("curl flag --user", NoteKind::NeedsInput)]
    );
    assert_eq!(parse("curl https://x.test").request.auth, AuthSpec::Inherit);
}

fn ntlm(username: &str, password: &str, domain: &str) -> AuthSpec {
    AuthSpec::Ntlm {
        username: username.into(),
        password: password.into(),
        domain: domain.into(),
    }
}

#[test]
fn ntlm_takes_the_domain_from_the_user() {
    for (command, want) in [
        (
            r"curl --ntlm -u 'CORP\bob:pw' https://x.test",
            ntlm("bob", "pw", "CORP"),
        ),
        (
            "curl --ntlm -u bob:pw https://x.test",
            ntlm("bob", "pw", ""),
        ),
        (
            r"curl -u 'CORP\bob:a\b:c' --ntlm https://x.test",
            ntlm("bob", r"a\b:c", "CORP"),
        ),
        (
            r"curl --ntlm -u 'a\b\c:pw' https://x.test",
            ntlm(r"b\c", "pw", "a"),
        ),
        (
            r"curl --ntlm -u 'bob:C:\temp' https://x.test",
            ntlm("bob", r"C:\temp", ""),
        ),
    ] {
        let parsed = parse(command);
        assert_eq!(parsed.request.auth, want, "{command}");
        assert!(parsed.notes.is_empty(), "{command}: {:?}", parsed.notes);
    }
}

#[test]
fn ntlm_without_a_password_asks_for_one_and_without_a_user_carries_nothing() {
    let parsed = parse(r"curl --ntlm --user 'CORP\bob' https://x.test");
    assert_eq!(parsed.request.auth, ntlm("bob", "", "CORP"));
    assert_eq!(
        notes(&parsed),
        [n("curl flag --user", NoteKind::NeedsInput)]
    );
    let parsed = parse("curl --ntlm https://x.test");
    assert_eq!(parsed.request.auth, AuthSpec::Inherit);
    assert_eq!(notes(&parsed), [n("curl flag --ntlm", NoteKind::Dropped)]);
}

#[test]
fn an_unknown_letter_in_a_cluster_ends_the_word() {
    // `-W` is no curl 8.22.0 option, so curl rejects the word; nothing after it is read.
    let parsed = parse("curl -Wsecret https://x.test");
    assert_eq!(notes(&parsed), [n("curl flag -W", NoteKind::Dropped)]);
    assert!(
        parsed
            .notes
            .iter()
            .all(|x| !x.source.contains("secret") && !x.detail.contains("secret"))
    );
    let parsed = parse("curl -WHfoo:bar https://x.test");
    assert!(headers(&parsed).is_empty());
    assert_eq!(notes(&parsed), [n("curl flag -W", NoteKind::Dropped)]);
}

#[test]
fn unknown_flags_and_shell_text_are_noted_without_values() {
    let parsed = parse("curl -sSL -o out.json --proxy-user u:s3cret -k https://x.test/a | jq .");
    assert_eq!(parsed.request.url, "https://x.test/a");
    let d = NoteKind::Dropped;
    assert_eq!(
        notes(&parsed),
        [
            n("curl flag -s", d),
            n("curl flag -S", d),
            n("curl flag -o", d),
            n("curl flag --proxy-user", d),
            n("curl flag -k", d),
            n("shell", d),
        ]
    );
    assert!(
        parsed
            .notes
            .iter()
            .all(|note| !note.detail.contains("s3cret"))
    );
    // A misspelled option with an attached value keeps the value out of the note too.
    let parsed = parse("curl --oauth2-baerer=TOKEN123 https://x.test");
    assert_eq!(notes(&parsed), [n("curl flag --oauth2-baerer", d)]);
    assert!(
        parsed
            .notes
            .iter()
            .all(|note| !note.source.contains("TOKEN123") && !note.detail.contains("TOKEN123"))
    );
}

#[test]
fn extra_urls_are_noted() {
    let parsed = parse("curl https://a.test --url https://b.test\ncurl https://c.test");
    assert_eq!(parsed.request.url, "https://a.test");
    let d = NoteKind::Dropped;
    assert_eq!(
        notes(&parsed),
        [n("curl url", d), n("curl url", d), n("curl url", d)]
    );
}

#[test]
fn settings_turn_encoding_off_only_where_it_would_change_the_url() {
    let mut expected = RequestSettings::new();
    expected.forward_authorization_header = false;
    // curl does not follow redirects without -L.
    expected.follow_redirects = false;
    // curl's own limits are dropped, so the request takes the default timeout.
    expected.timeout = Timeout::Inherit;
    // Encoding leaves this URL as written, so the request needs no `encodeUrl: false`.
    assert_eq!(
        parse("curl https://x.test/a/b?q=1&r=two").request.settings,
        Some(expected)
    );
    // Encoding would turn the query's `%20` into `%2520`, so the URL is sent as written.
    expected.encode_url = false;
    assert_eq!(
        parse("curl 'https://x.test/a?q=a%20b'").request.settings,
        Some(expected)
    );
    // A variable's value is unknown and may already be encoded, so encoding stays off.
    assert_eq!(
        parse("curl https://{{host}}/a").request.settings,
        Some(expected)
    );
}

#[test]
fn input_that_is_not_one_curl_command_is_an_error() {
    let not_curl = CurlError::Syntax {
        offset: 0,
        form: CurlSyntax::NotCurl,
    };
    assert_eq!(
        parse_curl("wget https://x.test").expect_err("not curl"),
        not_curl
    );
    assert_eq!(parse_curl("  ").expect_err("empty"), not_curl);
    assert_eq!(
        parse_curl("curl -H 'a: b'").expect_err("no url"),
        CurlError::MissingUrl
    );
    assert_eq!(
        parse_curl("curl ''").expect_err("empty url"),
        CurlError::MissingUrl
    );
    assert_eq!(
        parse_curl("curl -h https://x.test").expect_err("help takes a subject"),
        CurlError::MissingUrl
    );
    assert_eq!(
        parse_curl("curl https://x.test -H").expect_err("no value"),
        CurlError::MissingValue {
            flag: "-H".into(),
            offset: 20
        }
    );
    assert!(matches!(
        parse_curl("curl 'x"),
        Err(CurlError::Syntax {
            form: CurlSyntax::UnclosedQuote,
            ..
        })
    ));
}
