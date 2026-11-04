//! The prompt label grammar and where the scanner looks, in the reference's order.

use crate::common;

use apinata_core::{Collection, ContextOptions, VarContext, prompt_label, prompt_labels};
use common::{request, setup, write_tree};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn http(url: &str, under_http: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{under_http}"
    )
}

fn labels_of(files: &[(&str, &str)], rel: &str, options: ContextOptions) -> Vec<String> {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let ctx = VarContext::new(&c, options).expect("context");
    prompt_labels(request(&c, rel), &ctx)
}

#[test]
fn the_label_grammar_follows_the_reference() {
    assert_eq!(prompt_label("?x"), Some("x"));
    assert_eq!(prompt_label("?Order id"), Some("Order id"));
    assert_eq!(prompt_label("?a.b[0]"), Some("a.b[0]"));
    assert_eq!(prompt_label("??"), Some("?"));
    for bad in [
        "?", "? x", "?x ", "?\tx", "?x\u{a0}", "x", "?a{b", "?a}b", "",
    ] {
        assert_eq!(prompt_label(bad), None, "{bad:?}");
    }
}

#[test]
fn only_valid_labels_are_found_in_text() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &http("http://h.test/{{?}}/{{? x}}/{{?x }}/{{?Order id}}", ""),
        ),
    ]);
    assert_eq!(prompt_labels(request(&c, "r.yml"), &ctx), ["Order id"]);
}

#[test]
fn labels_are_deduped_in_first_seen_order() {
    let headers = "  headers:\n    - name: X-B\n      value: \"{{?b}}\"\n";
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &http("http://h.test/{{?a}}/{{?b}}/{{?a}}", headers),
        ),
    ]);
    assert_eq!(prompt_labels(request(&c, "r.yml"), &ctx), ["b", "a"]);
}

#[test]
fn every_scan_place_in_the_reference_order() {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  clientCertificates:\n    - domain: other.test\n      type: pem\n      certificateFilePath: \"{{?nomatch}}\"\n      privateKeyFilePath: k.pem\n    - domain: api.example.com\n      type: pem\n      certificateFilePath: \"{{?certdir}}/c.pem\"\n      privateKeyFilePath: k.pem\n      passphrase: \"{{?pass}}\"\nrequest:\n  variables:\n    - name: shadowed\n      value: \"{{?hidden}}\"\n    - name: coll\n      value: \"{{?collvar}}\"\n  headers:\n    - name: X-C\n      value: \"{{?colhead}}\"\n  auth:\n    type: bearer\n    token: \"{{?tok}}\"\n";
    let dev = "name: dev\nvariables:\n  - name: shadowed\n    value: plain\n  - name: env\n    value: \"{{?envvar}}\"\n";
    let folder = "info:\n  name: api\n  type: folder\nrequest:\n  variables:\n    - name: fold\n      value: \"{{?foldvar}}\"\n";
    let req = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"https://api.example.com/{{?id}}\"\n  headers:\n    - name: X-R\n      value: \"{{?reqhead}}\"\n    - name: X-Off\n      value: \"{{?headoff}}\"\n      disabled: true\n  params:\n    - name: q\n      value: \"{{?paramoff}}\"\n      type: query\n      disabled: true\n  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"{{?form}}\"\n      - name: b\n        value: \"{{?formoff}}\"\n        disabled: true\n  auth: inherit\nruntime:\n  variables:\n    - name: own\n      value: \"{{?reqvar}}\"\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('s', '{{?script}}')\"\ndocs: \"{{?docs}}\"\n";
    let found = labels_of(
        &[
            ("opencollection.yml", root),
            ("environments/dev.yml", dev),
            ("api/folder.yml", folder),
            ("api/r.yml", req),
        ],
        "api/r.yml",
        ContextOptions::new().env("dev"),
    );
    assert_eq!(
        found,
        [
            "certdir", "pass", "collvar", "envvar", "foldvar", "reqvar", "form", "formoff",
            "colhead", "reqhead", "paramoff", "tok", "id",
        ]
    );
}

#[test]
fn the_certificate_matches_on_the_leniently_interpolated_url() {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  clientCertificates:\n    - domain: api.example.com\n      type: pkcs12\n      pkcs12FilePath: c.p12\n      passphrase: \"{{?pass}}\"\nrequest:\n  variables:\n    - name: h\n      value: api.example.com\n";
    let found = labels_of(
        &[
            ("opencollection.yml", root),
            ("r.yml", &http("https://{{h}}/{{nope}}", "")),
        ],
        "r.yml",
        ContextOptions::new(),
    );
    assert_eq!(found, ["pass"]);
}

#[test]
fn runtime_values_are_scanned_and_process_env_is_not() {
    let (_dir, c, _) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/{{process.env.P}}", "")),
    ]);
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().process_env([("P".to_owned(), "{{?proc}}".to_owned())]),
    )
    .expect("context");
    assert!(prompt_labels(request(&c, "r.yml"), &ctx).is_empty());
    ctx.runtime_vars().set_text("cap", "{{?captured}}");
    assert_eq!(prompt_labels(request(&c, "r.yml"), &ctx), ["captured"]);
}
