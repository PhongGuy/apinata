//! Bodies, JSON comment stripping and path params.

use crate::common;

use apinata_core::{Body, Part, ResolveError};
use common::{at, pairs, setup};

/// A request file; `under_http` is indented under `http:`.
fn http(url: &str, under_http: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n{under_http}")
}

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: n\n      value:\n        type: number\n        data: \"42\"\n    - name: f\n      value: field\n    - name: q\n      value: a\"b\n    - name: slashes\n      value: http://x//y /* z */\n";
const U: &str = "https://x.test/";

#[test]
fn supported_bodies_render_with_a_default_content_type() {
    let json = http(U, "  body:\n    type: json\n    data: '{\"a\": {{n}}}'\n");
    let typed = http(
        U,
        "  headers:\n    - name: Content-Type\n      value: application/vnd.x+json\n  body:\n    type: json\n    data: '{}'\n",
    );
    let empty = http(U, "  body:\n    type: json\n    data: \"\"\n");
    let text = http(U, "  body:\n    type: text\n    data: \"hi {{f}}\"\n");
    let xml = http(U, "  body:\n    type: xml\n    data: \"<a/>\"\n");
    let form = http(
        U,
        "  body:\n    type: form-urlencoded\n    data:\n      - name: \"{{f}}\"\n        value: \"{{n}}\"\n      - name: gone\n        value: x\n        disabled: true\n      - name: plain\n        value: v\n",
    );
    let none = http(U, "");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("j.yml", &json),
        ("jc.yml", &typed),
        ("je.yml", &empty),
        ("t.yml", &text),
        ("x.yml", &xml),
        ("fu.yml", &form),
        ("none.yml", &none),
    ]);
    let check = |rel: &str, body: Body, headers: &[(&str, &str)]| {
        let r = at(&c, &ctx, rel).expect(rel);
        assert_eq!(r.body(), &body, "{rel}");
        assert_eq!(r.headers().to_vec(), pairs(headers), "{rel}");
    };
    check(
        "j.yml",
        Body::Text("{\"a\": 42}".to_owned()),
        &[("content-type", "application/json")],
    );
    check(
        "jc.yml",
        Body::Text("{}".to_owned()),
        &[("Content-Type", "application/vnd.x+json")],
    );
    check("je.yml", Body::None, &[]);
    check(
        "t.yml",
        Body::Text("hi field".to_owned()),
        &[("content-type", "text/plain")],
    );
    check(
        "x.yml",
        Body::Text("<a/>".to_owned()),
        &[("content-type", "application/xml")],
    );
    check(
        "fu.yml",
        Body::FormUrlEncoded(pairs(&[("field", "42"), ("plain", "v")])),
        &[("content-type", "application/x-www-form-urlencoded")],
    );
    check("none.yml", Body::None, &[]);
}

#[test]
fn json_bodies_are_decommented_before_interpolation() {
    let json = http(
        U,
        "  body:\n    type: json\n    data: |-\n      {\n        // the target\n        \"u\": \"{{slashes}}\", /* inline */ \"k\": \"{{q}}\"\n      }\n",
    );
    let text = http(U, "  body:\n    type: text\n    data: \"keep // this\"\n");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("j.yml", &json),
        ("t.yml", &text),
    ]);
    let body = |rel: &str| at(&c, &ctx, rel).expect(rel).body().clone();
    // Variable values are inserted after decomment and are never JSON-escaped, as in Bruno.
    assert_eq!(
        body("j.yml"),
        Body::Text("{\n  \"u\": \"http://x//y /* z */\",  \"k\": \"a\"b\"\n}".to_owned())
    );
    assert_eq!(body("t.yml"), Body::Text("keep // this".to_owned()));
}

#[test]
fn unsupported_bodies_fail_before_any_interpolation() {
    let kinds = [("mp.yml", "msgpack"), ("gq.yml", "graphql")];
    let mut files: Vec<(String, String)> = kinds
        .iter()
        .map(|(rel, kind)| {
            (
                (*rel).to_owned(),
                http(
                    "https://x.test/{{missing}}",
                    &format!("  body:\n    type: {kind}\n    data: []\n"),
                ),
            )
        })
        .collect();
    files.push((
        "var.yml".to_owned(),
        http(
            U,
            "  body:\n    - title: a\n      body:\n        type: json\n        data: '{}'\n",
        ),
    ));
    files.push(("opencollection.yml".to_owned(), ROOT.to_owned()));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, c, ctx) = setup(&refs);
    for (rel, expected) in kinds.into_iter().chain([("var.yml", "variants")]) {
        match at(&c, &ctx, rel) {
            Err(ResolveError::UnsupportedBody { kind }) => assert_eq!(kind, expected, "{rel}"),
            other => panic!("{rel}: expected UnsupportedBody, got {other:?}"),
        }
    }
}

#[test]
fn path_params_fill_whole_segments_and_query_params_stay_in_the_url() {
    let params = http(
        "https://x.test:8080/a:b/users/:id/posts/:post?q=1",
        "  params:\n    - name: id\n      value: \"{{n}}\"\n      type: path\n    - name: post\n      value: \"9\"\n      type: path\n      disabled: true\n    - name: q\n      value: \"1\"\n      type: query\n    - name: extra\n      value: \"2\"\n      type: query\n",
    );
    let blank = http(
        "https://x.test/:id/:other",
        "  params:\n    - name: id\n      value: \"  \"\n      type: path\n    - name: other\n      value: o\n      type: path\n",
    );
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("p.yml", &params),
        ("b.yml", &blank),
    ]);
    assert_eq!(
        at(&c, &ctx, "p.yml").expect("resolves").url(),
        "https://x.test:8080/a:b/users/42/posts/:post?q=1"
    );
    assert_eq!(
        at(&c, &ctx, "b.yml").expect("resolves").url(),
        "https://x.test/:id/o"
    );
}

#[test]
fn a_url_query_that_disagrees_with_the_params_is_sent_as_the_url_has_it() {
    let r = http(
        "https://x.test/?z=6",
        "  params:\n    - name: z\n      value: \"3\"\n      type: query\n",
    );
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    assert_eq!(
        at(&c, &ctx, "r.yml").expect("resolves").url(),
        "https://x.test/?z=6"
    );
}

#[test]
fn path_params_that_interpolate_to_blank_stay_in_the_url() {
    let root = format!("{ROOT}    - name: empty\n      value: \"\"\n");
    let r = http(
        "https://x.test/:id/:n",
        "  params:\n    - name: id\n      value: \"{{empty}}\"\n      type: path\n    - name: n\n      value: \"{{n}}\"\n      type: path\n",
    );
    let (_dir, c, ctx) = setup(&[("opencollection.yml", &root), ("r.yml", &r)]);
    assert_eq!(
        at(&c, &ctx, "r.yml").expect("resolves").url(),
        "https://x.test/:id/42"
    );
}

#[test]
fn debug_output_shows_body_shape_and_redacts_url_secrets() {
    let text = Body::Text("s3cret".to_owned());
    assert_eq!(format!("{text:?}"), "Text { bytes: 6 }");
    let form = Body::FormUrlEncoded(pairs(&[("user", "s3cret")]));
    assert_eq!(format!("{form:?}"), "FormUrlEncoded { fields: [\"user\"] }");
    assert_eq!(format!("{:?}", Body::None), "None");

    let r = http(
        "https://me:s3cret@x.test/p?token=s3cret&s3cret&k=v=s3cret#top",
        "",
    );
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    let debug = format!("{:?}", at(&c, &ctx, "r.yml").expect("resolves"));
    assert!(
        !debug.contains("s3cret") && !debug.contains("me:"),
        "{debug}"
    );
    assert!(
        debug.contains(
            "\"https://<redacted>@x.test/p?token=<redacted>&<redacted>&k=<redacted>#top\""
        ),
        "{debug}"
    );
}

#[test]
fn urls_are_finished_like_bruno_and_whatwg() {
    let root = format!("{ROOT}    - name: host\n      value: x.test\n");
    let files = [
        ("opencollection.yml", root),
        ("space.yml", http("x.test/a b?q=a b", "")),
        ("port.yml", http("localhost:8080/api", "")),
        ("upper.yml", http("HTTPS://X.test", "")),
        ("var.yml", http("{{host}}/p", "")),
        ("bad.yml", http("http://exa mple.com/", "")),
    ];
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (*a, b.as_str())).collect();
    let (_dir, c, ctx) = setup(&refs);
    let url = |rel: &str| at(&c, &ctx, rel).expect(rel).url().to_owned();
    assert_eq!(url("space.yml"), "http://x.test/a%20b?q=a%20b");
    assert_eq!(url("port.yml"), "http://localhost:8080/api");
    assert_eq!(url("upper.yml"), "https://x.test/");
    assert_eq!(url("var.yml"), "http://x.test/p");
    match at(&c, &ctx, "bad.yml") {
        Err(ResolveError::InvalidUrl { url }) => assert_eq!(url, "http://exa mple.com/"),
        other => panic!("expected InvalidUrl, got {other:?}"),
    }
}

#[test]
fn text_and_xml_bodies_without_data_send_an_empty_body() {
    let text = http(U, "  body:\n    type: text\n");
    let xml = http(U, "  body:\n    type: xml\n    data: \"\"\n");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("t.yml", &text),
        ("x.yml", &xml),
    ]);
    for (rel, content_type) in [("t.yml", "text/plain"), ("x.yml", "application/xml")] {
        let r = at(&c, &ctx, rel).expect(rel);
        // Measured with axios 1.18.0: Bruno's `''` and a missing body both go out as an empty
        // body with `Content-Length: 0` on POST.
        assert_eq!(r.body(), &Body::Text(String::new()), "{rel}");
        // Bruno still sets the default content type for these modes.
        assert_eq!(
            r.headers().to_vec(),
            pairs(&[("content-type", content_type)]),
            "{rel}"
        );
    }
}

#[test]
fn multipart_bodies_interpolate_values_and_paths_but_not_names() {
    let mp = http(
        U,
        concat!(
            "  body:\n    type: multipart-form\n    data:\n",
            "      - name: \"{{f}}\"\n        type: text\n        value: \"{{n}}\"\n",
            "      - name: list\n        type: text\n        value: [a, \"{{f}}\"]\n",
            "      - name: gone\n        type: text\n        value: x\n        disabled: true\n",
            "      - name: typed\n        type: text\n        value: v\n        contentType: application/json\n",
            "      - name: up\n        type: file\n        value: [\" files/a.txt \", \"{{f}}/b.json\", /abs/c.bin]\n        contentType: image/png\n",
            "      - name: odd\n        type: other\n        value: x\n",
        ),
    );
    let typed = http(
        U,
        "  headers:\n    - name: Content-Type\n      value: multipart/mixed\n  body:\n    type: multipart-form\n    data: []\n",
    );
    let (dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("mp.yml", &mp),
        ("mt.yml", &typed),
    ]);
    let r = at(&c, &ctx, "mp.yml").expect("resolves");
    let root = dir.path();
    // A list text value is one part, comma-joined, as the filestore's ensureString makes it.
    let expected = vec![
        Part::text("{{f}}", "42"),
        Part::text("list", "a,field"),
        Part::text("typed", "v").with_content_type("application/json"),
        Part::file("up", root.join("files/a.txt")).with_content_type("image/png"),
        Part::file("up", root.join("field/b.json")).with_content_type("image/png"),
        Part::file("up", "/abs/c.bin").with_content_type("image/png"),
    ];
    assert_eq!(r.body(), &Body::Multipart(expected));
    assert_eq!(
        r.headers().to_vec(),
        pairs(&[("content-type", "multipart/form-data")])
    );
    let shown = format!("{r:?}");
    assert!(
        !shown.contains("a,field") && !shown.contains("b.json"),
        "{shown}"
    );
    let r = at(&c, &ctx, "mt.yml").expect("resolves");
    assert_eq!(r.body(), &Body::Multipart(Vec::new()));
    assert_eq!(
        r.headers().to_vec(),
        pairs(&[("Content-Type", "multipart/mixed")])
    );
}

#[test]
fn multipart_file_rows_with_falsy_or_null_values_yield_no_parts() {
    // Bruno reads `value || []`, so a falsy scalar is no files; a null list item is skipped.
    let mp = http(
        U,
        concat!(
            "  body:\n    type: multipart-form\n    data:\n",
            "      - name: empty\n        type: file\n        value: \"\"\n",
            "      - name: no\n        type: file\n        value: false\n",
            "      - name: zero\n        type: file\n        value: 0\n",
            "      - name: none\n        type: file\n        value: null\n",
            "      - name: holes\n        type: file\n        value: [null, a.txt]\n",
        ),
    );
    let (dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("mp.yml", &mp)]);
    let r = at(&c, &ctx, "mp.yml").expect("resolves");
    let expected = vec![Part::file("holes", dir.path().join("a.txt"))];
    assert_eq!(r.body(), &Body::Multipart(expected));
}

#[test]
fn file_bodies_send_the_selected_entry_and_its_type_replaces_the_header() {
    let entries = |rows: &str| format!("  body:\n    type: file\n    data:\n{rows}");
    let user = "  headers:\n    - name: Content-Type\n      value: text/csv\n";
    let selected = "      - filePath: other.bin\n        contentType: x/other\n        selected: false\n      - filePath: \"{{f}}.json\"\n        contentType: \"application/{{f}}\"\n        selected: true\n";
    let empty_type =
        "      - filePath: /abs/b.json\n        contentType: \"\"\n        selected: true\n";
    let unselected =
        "      - filePath: b.json\n        contentType: x/y\n        selected: false\n";
    let no_path = "      - filePath: \"\"\n        contentType: x/y\n        selected: true\n";
    let (dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("sel.yml", &http(U, &format!("{user}{}", entries(selected)))),
        ("empty.yml", &http(U, &entries(empty_type))),
        ("none.yml", &http(U, &entries(unselected))),
        (
            "user.yml",
            &http(U, &format!("{user}{}", entries(unselected))),
        ),
        ("nopath.yml", &http(U, &entries(no_path))),
    ]);
    let check = |rel: &str, body: Body, headers: &[(&str, &str)]| {
        let r = at(&c, &ctx, rel).expect(rel);
        assert_eq!(r.body(), &body, "{rel}");
        assert_eq!(r.headers().to_vec(), pairs(headers), "{rel}");
    };
    // The path is read before interpolation in bruno-cli, so `{{f}}` stays literal; the type is a
    // header value and interpolates.
    check(
        "sel.yml",
        Body::File {
            path: dir.path().join("{{f}}.json"),
            content_type: "application/field".to_owned(),
        },
        &[("Content-Type", "application/field")],
    );
    check(
        "empty.yml",
        Body::File {
            path: "/abs/b.json".into(),
            content_type: String::new(),
        },
        &[("content-type", "")],
    );
    let shown = format!("{:?}", at(&c, &ctx, "sel.yml").expect("resolves"));
    assert!(!shown.contains("{{f}}.json"), "{shown}");
    check(
        "none.yml",
        Body::None,
        &[("content-type", "application/octet-stream")],
    );
    check("user.yml", Body::None, &[("Content-Type", "text/csv")]);
    check("nopath.yml", Body::None, &[("content-type", "x/y")]);
}
