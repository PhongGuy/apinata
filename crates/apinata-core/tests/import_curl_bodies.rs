//! `parse_curl` bodies: Bruno's Content-Type decision, curl's data rules and the JSON layout.

use apinata_core::import::{NoteKind, ParsedCurl, parse_curl};
use apinata_core::{BodySpec, FileEntry, KeyValue, MultipartField, ParamKind};

fn parse(command: &str) -> ParsedCurl {
    parse_curl(command).unwrap_or_else(|e| panic!("{command}: {e}"))
}
fn body(command: &str) -> BodySpec {
    parse(command).request.body
}
fn form(pairs: &[(&str, &str)]) -> BodySpec {
    BodySpec::FormUrlEncoded(pairs.iter().map(|(k, v)| KeyValue::new(*k, *v)).collect())
}
fn headers(parsed: &ParsedCurl) -> Vec<(&str, &str)> {
    parsed
        .request
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect()
}
fn notes(parsed: &ParsedCurl) -> Vec<(&str, NoteKind)> {
    parsed
        .notes
        .iter()
        .map(|n| (n.source.as_str(), n.kind))
        .collect()
}

#[test]
fn content_type_picks_the_mode() {
    let with = |ct: &str, data: &str| {
        body(&format!(
            "curl https://x.test -H 'Content-Type: {ct}' --data-raw '{data}'"
        ))
    };
    assert_eq!(
        with("application/json; charset=UTF-8", r#"{"a":1}"#),
        BodySpec::Json("{\n  \"a\": 1\n}".into())
    );
    assert_eq!(
        with("application/vnd.api+json", "[]"),
        BodySpec::Json("[]".into())
    );
    assert_eq!(with("text/xml", "<a/>"), BodySpec::Xml("<a/>".into()));
    assert_eq!(
        with("application/soap+xml", "<a/>"),
        BodySpec::Xml("<a/>".into())
    );
    assert_eq!(
        with("application/xml-dtd", "<a/>"),
        BodySpec::Xml("<a/>".into()),
        "any type containing xml"
    );
    assert_eq!(
        with("application/x-ndjson", "{}\n{}"),
        BodySpec::Text("{}\n{}".into())
    );
    assert_eq!(with("text/plain", "hi"), BodySpec::Text("hi".into()));
    assert_eq!(
        with("application/octet-stream", "raw"),
        BodySpec::Text("raw".into())
    );
    assert_eq!(
        with("application/x-www-form-urlencoded", "a=1&b=x%20y+z"),
        form(&[("a", "1"), ("b", "x y z")])
    );
}

#[test]
fn data_without_a_content_type_is_a_form_when_every_pair_has_a_name() {
    let parsed = parse("curl -d name=John -d 'age=30' -d 'tag={{t}}' https://x.test");
    assert_eq!(
        parsed.request.body,
        form(&[("name", "John"), ("age", "30"), ("tag", "{{t}}")])
    );
    assert!(parsed.notes.is_empty());
    let parsed = parse(r#"curl -d '{"name": "John"}' https://x.test"#);
    assert_eq!(
        parsed.request.body,
        BodySpec::Text(r#"{"name": "John"}"#.into())
    );
    assert_eq!(
        headers(&parsed),
        [("Content-Type", "application/x-www-form-urlencoded")]
    );
    assert_eq!(notes(&parsed), [("curl flag -d", NoteKind::Converted)]);
    let detail = parsed.notes[0].detail.as_str();
    assert_eq!(
        detail,
        "the data is not name=value pairs, so the body is kept as raw text and the Content-Type header is set to application/x-www-form-urlencoded, as curl does; change the header if the server expects another type"
    );
    assert!(!detail.to_lowercase().contains("bruno"));
}

#[test]
fn get_moves_the_data_into_the_query() {
    let parsed = parse("curl -G -d 'name=John' -d age=30 https://x.test/users");
    assert_eq!(parsed.request.url, "https://x.test/users?name=John&age=30");
    assert_eq!(parsed.request.body, BodySpec::None);
    let params: Vec<(&str, &str, ParamKind)> = parsed
        .request
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.value.as_str(), p.kind))
        .collect();
    assert_eq!(
        params,
        [
            ("name", "John", ParamKind::Query),
            ("age", "30", ParamKind::Query)
        ]
    );
    assert_eq!(
        parse("curl -G -d b=2 'https://x.test/?a=1'").request.url,
        "https://x.test/?a=1&b=2"
    );
    assert_eq!(
        parse("curl -G -d b=2 'https://x.test/?'").request.url,
        "https://x.test/?b=2"
    );
}

#[test]
fn data_urlencode_encodes_like_curl() {
    let parsed = parse(
        "curl -G --data-urlencode 'name=John Doe' --data-urlencode '=a&b' --data-urlencode 'email=j@x.io' --data-urlencode hello 'https://x.test/u?t=1&hello'",
    );
    assert_eq!(
        parsed.request.url,
        "https://x.test/u?t=1&hello&name=John+Doe&a%26b&email=j%40x.io&hello"
    );
    assert_eq!(
        body("curl --data-urlencode 'q=a b&c' https://x.test"),
        form(&[("q", "a b&c")])
    );
    let parsed = parse("curl --data-urlencode a=1 --data-urlencode 'b@file.txt' https://x.test");
    assert_eq!(parsed.request.body, form(&[("a", "1")]));
    assert_eq!(
        notes(&parsed),
        [("curl flag --data-urlencode", NoteKind::NeedsInput)]
    );
    // Checked against curl 8.22.0: a space is `+`, and `@` names a file only before any `=`.
    let parsed = parse(
        "curl -G --data-urlencode 'a=~-._ x' --data-urlencode 'q=a b&c!*' --data-urlencode 'b@f=x' --data-urlencode '=a b+' https://x.test/",
    );
    assert_eq!(
        parsed.request.url,
        "https://x.test/?a=~-._+x&q=a+b%26c%21%2A&b@f=x&a+b%2B"
    );
    assert!(parsed.notes.is_empty());
    assert_eq!(
        body("curl --data-urlencode 'b@f=x y' https://x.test"),
        form(&[("b@f", "x y")])
    );
}

#[test]
fn an_at_file_is_a_file_body_with_the_content_type() {
    let entry = |path: &str, ct: &str| {
        let mut e = FileEntry::new(path);
        e.content_type = ct.into();
        e.selected = true;
        e
    };
    assert_eq!(
        body(
            "curl -H 'Content-Type: application/octet-stream' --data-binary @/tmp/p.bin https://x.test"
        ),
        BodySpec::File(vec![entry("/tmp/p.bin", "application/octet-stream")])
    );
    assert_eq!(
        body("curl -d @body.json https://x.test"),
        BodySpec::File(vec![entry("body.json", "")])
    );
    assert_eq!(
        body("curl --data-raw @literal https://x.test"),
        BodySpec::Text("@literal".into())
    );
    let parsed = parse("curl -d a=1 -d @more.txt https://x.test");
    assert_eq!(parsed.request.body, form(&[("a", "1")]));
    assert_eq!(notes(&parsed), [("curl flag -d", NoteKind::NeedsInput)]);
}

#[test]
fn form_fields_become_multipart_parts() {
    let parsed = parse(
        "curl -F name=John -F 'img=@a.png;type=image/png' -F 'meta=<meta.json' -F 'doc=@d.txt;filename=x.txt' -F nofield -F 'e=f;token123' -d b=2 https://x.test",
    );
    let mut img = MultipartField::files("img", vec!["a.png".into()]);
    img.content_type = "image/png".into();
    assert_eq!(
        parsed.request.body,
        BodySpec::Multipart(vec![
            MultipartField::text("name", "John"),
            img,
            MultipartField::files("meta", vec!["meta.json".into()]),
            MultipartField::files("doc", vec!["d.txt".into()]),
            MultipartField::text("e", "f"),
        ])
    );
    assert!(
        headers(&parsed).is_empty(),
        "the sender sets the multipart Content-Type"
    );
    assert_eq!(
        notes(&parsed),
        [
            ("curl flag -F", NoteKind::Converted),
            ("curl flag -F", NoteKind::Dropped),
            ("curl flag -F", NoteKind::Dropped),
            ("curl flag -F", NoteKind::Dropped),
            ("curl flag -d", NoteKind::Dropped),
        ]
    );
    for note in &parsed.notes {
        assert!(
            !note.detail.to_lowercase().contains("bruno"),
            "{}",
            note.detail
        );
        assert!(!note.detail.contains("token123"), "{}", note.detail);
    }
    // curl's quoting: the quotes go, `\"` unescapes, `;` inside stays; an unclosed quote is plain text.
    assert_eq!(
        body(r#"curl -F 'f=@"a;b"' https://x.test"#),
        BodySpec::Multipart(vec![MultipartField::files("f", vec!["a;b".into()])])
    );
    let mut typed = MultipartField::text("c", "x\"y;z");
    typed.content_type = "t/x".into();
    assert_eq!(
        body(r#"curl -F 'c="x\"y;z";type=t/x' -F 'u="ab' https://x.test"#),
        BodySpec::Multipart(vec![typed, MultipartField::text("u", "\"ab")])
    );
}

#[test]
fn json_flag_sets_the_content_type_unless_given() {
    let parsed = parse(r#"curl --json '{"a":1}' https://x.test"#);
    assert_eq!(headers(&parsed), [("Content-Type", "application/json")]);
    assert_eq!(parsed.request.method, "POST");
    let parsed =
        parse(r#"curl -H 'content-type: application/vnd.x+json' --json '{"a":1}' https://x.test"#);
    assert_eq!(
        headers(&parsed),
        [("content-type", "application/vnd.x+json")]
    );
}

#[test]
fn json_is_laid_out_with_two_spaces_keeping_keys_numbers_and_templates() {
    let json = |data: &str| body(&format!("curl --json '{data}' https://x.test"));
    let expected = r#"{
  "b": 1e3,
  "a": [
    1.50,
    {},
    []
  ],
  "t": {{num}},
  "s": "x\"y {{v}}"
}"#;
    assert_eq!(
        json(r#"{"b": 1e3, "a": [1.50, {}, []], "t": {{num}}, "s": "x\"y {{v}}"}"#),
        BodySpec::Json(expected.into())
    );
    assert_eq!(json("42"), BodySpec::Json("42".into()));
    assert_eq!(
        json(r#"{"a": 1,}"#),
        BodySpec::Json(r#"{"a": 1,}"#.into()),
        "invalid JSON is kept as written"
    );
}

#[test]
fn json_layout_keeps_duplicate_keys_and_exponent_numbers() {
    let parsed = body(
        r#"curl https://x.test -H 'Content-Type: application/json' --data-raw '{"a":1e3,"a":2}'"#,
    );
    assert_eq!(
        parsed,
        BodySpec::Json("{\n  \"a\": 1e3,\n  \"a\": 2\n}".into())
    );
}
