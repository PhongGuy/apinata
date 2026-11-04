//! Saved examples read as Bruno's parser reads them, and converted from and to responses.

use apinata_core::{
    BodySpec, Collection, ExampleBody, ExampleBodyKind, ExampleResponse, ItemPath, ParamKind,
    Response,
};
use std::fs;
use tempfile::TempDir;

const STORED: &str = r#"info:
  name: a
  type: http
  seq: 1
http:
  method: GET
  url: https://x.test
examples:
  - name: created
    description: a new user
    request:
      url: https://{{host}}/users/:id
      method: POST
      headers:
        - name: Authorization
          value: Bearer {{token}}
        - name: X-Debug
          value: "1"
          disabled: true
      params:
        - name: id
          value: "{{userId}}"
          type: path
      body:
        type: json
        data: '{"a": 1}'
    response:
      status: 201
      statusText: Created
      headers:
        - name: Content-Type
          value: application/json
      body:
        type: json
        data: '{"id": 7}'
  - plain text entry
  - name: 42
    description:
      content: "  "
    response:
      status: "404"
      body:
        type: weird
  - {}
"#;

fn examples(text: &str) -> Vec<apinata_core::Example> {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: \"1.0.0\"\ninfo:\n  name: c\n",
    )
    .unwrap();
    fs::write(dir.path().join("a.yml"), text).unwrap();
    let c = Collection::open(dir.path()).unwrap();
    c.request(&ItemPath::new("a.yml").unwrap())
        .unwrap()
        .examples()
}

#[test]
fn examples_read_as_bruno_parses_them_one_per_entry() {
    let ex = examples(STORED);
    assert_eq!(
        ex.len(),
        4,
        "one example per entry, so indexes match the file"
    );
    let first = &ex[0];
    assert_eq!(first.name, "created");
    assert_eq!(first.description.as_deref(), Some("a new user"));
    let req = first.request.as_ref().expect("a request");
    assert_eq!(
        (req.method.as_str(), req.url.as_str()),
        ("POST", "https://{{host}}/users/:id")
    );
    assert_eq!(req.headers.len(), 2);
    assert_eq!(req.headers[0].value, "Bearer {{token}}");
    assert!(!req.headers[1].enabled);
    assert_eq!(
        (req.params[0].kind, req.params[0].value.as_str()),
        (ParamKind::Path, "{{userId}}")
    );
    assert!(req.body == BodySpec::Json("{\"a\": 1}".into()));
    let res = first.response.as_ref().expect("a response");
    assert_eq!(res.status, Some(201));
    assert_eq!(res.status_text.as_deref(), Some("Created"));
    assert_eq!(
        res.headers,
        [("Content-Type".to_owned(), "application/json".to_owned())]
    );
    assert_eq!(
        res.body,
        Some(ExampleBody::new(ExampleBodyKind::Json, "{\"id\": 7}"))
    );
    // A non-mapping entry keeps its slot with Bruno's default name.
    assert_eq!(ex[1].name, "Untitled Example");
    assert!(ex[1].request.is_none() && ex[1].response.is_none());
    // A numeric name reads as text, a blank description object as none, a string status as
    // a number, and an unknown body type as text with empty data.
    assert_eq!(ex[2].name, "42");
    assert_eq!(ex[2].description, None);
    let res = ex[2].response.as_ref().expect("a response");
    assert_eq!((res.status, res.status_text.as_deref()), (Some(404), None));
    assert_eq!(res.body, Some(ExampleBody::new(ExampleBodyKind::Text, "")));
    assert_eq!(ex[3].name, "Untitled Example");
}

#[test]
fn no_examples_key_or_a_non_list_reads_as_none() {
    let plain = "info:\n  name: a\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n";
    assert!(examples(plain).is_empty());
    assert!(examples(&format!("{plain}examples: nope\n")).is_empty());
}

#[test]
fn body_kinds_follow_the_content_type() {
    use ExampleBodyKind as K;
    let cases = [
        ("application/json; charset=utf-8", K::Json),
        ("application/problem+json", K::Text),
        ("text/xml", K::Xml),
        ("application/xml", K::Xml),
        ("text/html; charset=utf-8", K::Html),
        ("image/png", K::Binary),
        ("image/svg+xml", K::Text),
        ("application/pdf", K::Binary),
        ("application/octet-stream", K::Binary),
        ("AUDIO/MPEG", K::Binary),
        ("text/plain", K::Text),
        ("", K::Text),
    ];
    for (content_type, kind) in cases {
        assert_eq!(
            ExampleBodyKind::from_content_type(content_type),
            kind,
            "{content_type}"
        );
    }
    assert_eq!(K::Binary.as_str(), "binary");
}

#[test]
fn a_response_becomes_an_example_response() {
    let json = Response::new(200)
        .with_status_text("OK")
        .with_header("Content-Type", "application/json")
        .with_text_body("{\"a\":1}");
    let ex = ExampleResponse::from_response(&json);
    assert_eq!(
        (ex.status, ex.status_text.as_deref()),
        (Some(200), Some("OK"))
    );
    assert_eq!(
        ex.headers,
        [("content-type".to_owned(), "application/json".to_owned())]
    );
    assert_eq!(
        ex.body,
        Some(ExampleBody::new(ExampleBodyKind::Json, "{\n  \"a\": 1\n}"))
    );

    let png = Response::new(200)
        .with_header("content-type", "image/png")
        .with_body(vec![0x89, b'P', b'N', b'G'], "");
    let ex = ExampleResponse::from_response(&png);
    assert_eq!(ex.status_text, None);
    assert_eq!(
        ex.body,
        Some(ExampleBody::new(ExampleBodyKind::Binary, "iVBORw=="))
    );

    let not_utf8 = Response::new(500)
        .with_header("content-type", "text/plain")
        .with_body(vec![0xff, 0xfe], "\u{fffd}\u{fffd}");
    let ex = ExampleResponse::from_response(&not_utf8);
    assert_eq!(
        ex.body,
        Some(ExampleBody::new(ExampleBodyKind::Binary, "//4="))
    );

    let empty = ExampleResponse::from_response(&Response::new(204));
    assert_eq!(
        empty.body,
        Some(ExampleBody::new(ExampleBodyKind::Text, ""))
    );
}

#[test]
fn a_json_body_that_does_not_parse_stays_as_received() {
    let broken = Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body("{oops");
    let ex = ExampleResponse::from_response(&broken);
    assert_eq!(
        ex.body,
        Some(ExampleBody::new(ExampleBodyKind::Json, "{oops"))
    );
}

#[test]
fn an_example_response_shows_as_a_response() {
    let mut ex = ExampleResponse::new(201);
    ex.status_text = Some("Created".into());
    ex.headers = vec![("Content-Type".into(), "image/png".into())];
    ex.body = Some(ExampleBody::new(ExampleBodyKind::Binary, "iVBORw=="));
    let r = ex.to_response();
    assert_eq!((r.status(), r.status_text()), (201, "Created"));
    assert_eq!(
        r.headers(),
        [("content-type".to_owned(), "image/png".to_owned())]
    );
    assert_eq!(r.body(), [0x89, b'P', b'N', b'G']);

    ex.body = Some(ExampleBody::new(ExampleBodyKind::Json, "{}"));
    let r = ex.to_response();
    assert_eq!((r.body(), r.text()), (&b"{}"[..], "{}"));

    ex.body = Some(ExampleBody::new(ExampleBodyKind::Binary, "not base64!"));
    assert_eq!(ex.to_response().text(), "not base64!");

    ex.body = None;
    assert!(ex.to_response().body().is_empty());
}
