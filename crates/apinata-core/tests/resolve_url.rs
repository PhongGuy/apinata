//! Bruno's `encodeUrl` pass followed by WHATWG parsing, and redacted invalid URLs.

use crate::common;

use apinata_core::{ResolveError, Warning};
use common::{at, setup};

/// A request file with `url` double-quoted and `top` after `http:` at the top level.
fn http(url: &str, top: &str) -> String {
    let quoted = url.replace('\\', "\\\\").replace('"', "\\\"");
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{quoted}\"\n{top}")
}

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const ENCODE: &str = "settings:\n  encodeUrl: true\n";

/// Measured on 2026-10-09 with Node 24: `new URL(encodeUrl(input)).href`, `encodeUrl` from
/// `packages/bruno-common/src/utils/url/index.ts` at Bruno 1301ea17.
const MEASURED: &[(&str, &str)] = &[
    ("http://x.test/a b", "http://x.test/a%20b"),
    ("http://x.test/a%20b", "http://x.test/a%20b"),
    ("http://x.test/a%2fb", "http://x.test/a%2Fb"),
    ("http://x.test/%zz", "http://x.test/%25zz"),
    ("http://x.test/100%", "http://x.test/100%25"),
    ("http://x.test/café", "http://x.test/caf%C3%A9"),
    ("http://x.test/😀", "http://x.test/%F0%9F%98%80"),
    ("http://x.test/{a}|^`", "http://x.test/%7Ba%7D%7C%5E%60"),
    ("http://x.test/a!b'c(d)*~-_.", "http://x.test/a!b'c(d)*~-_."),
    (
        "http://x.test/a:b@c;d,e=f+g$h&i",
        "http://x.test/a%3Ab%40c%3Bd%2Ce%3Df%2Bg%24h%26i",
    ),
    ("http://x.test/p?q=a b", "http://x.test/p?q=a%20b"),
    ("http://x.test/p?q=a%20b", "http://x.test/p?q=a%2520b"),
    ("http://x.test/p?q=a+b", "http://x.test/p?q=a%2Bb"),
    ("http://x.test/p?q={x}|^", "http://x.test/p?q=%7Bx%7D%7C%5E"),
    ("http://x.test/p?q=café", "http://x.test/p?q=caf%C3%A9"),
    ("http://x.test/p?a=1&b=2&c", "http://x.test/p?a=1&b=2&c"),
    ("http://x.test/p?a=1&&b=", "http://x.test/p?a=1&b="),
    ("http://x.test/p?=v&k=x=y", "http://x.test/p?k=x%3Dy"),
    ("http://x.test/p?q=1#frag", "http://x.test/p?q=1%23frag"),
    ("http://x.test/p#frag", "http://x.test/p%23frag"),
    ("http://x.test/p#a b", "http://x.test/p%23a%20b"),
    ("http://x.test/p?q=a/b?c", "http://x.test/p?q=a%2Fb%3Fc"),
    ("http://x.test/p?", "http://x.test/p?"),
    ("http://x.test", "http://x.test/"),
    ("http://x.test/", "http://x.test/"),
    (
        "https://User:P%40ss@X.test:8080/a b?q=1",
        "https://User:P%40ss@x.test:8080/a%20b?q=1",
    ),
    ("http://[::1]:3000/a b", "http://[::1]:3000/a%20b"),
    ("http://x.test/a/../b/./c", "http://x.test/b/c"),
    ("http://x.test//a//b/", "http://x.test//a//b/"),
    ("http://x.test/a\\b", "http://x.test/a%5Cb"),
    ("http://x.test/p?q=%", "http://x.test/p?q=%25"),
    ("http://xn--caf-dma.test/a", "http://xn--caf-dma.test/a"),
    ("http://x.test/%C3%28", "http://x.test/%25C3("),
    ("http://x.test/%41%e9%zz", "http://x.test/A%25e9%25zz"),
    (
        "http://x.test/p?%C3%A9=%41",
        "http://x.test/p?%25C3%25A9=%2541",
    ),
    ("http://x.test/p?q=1&q=2", "http://x.test/p?q=1&q=2"),
];

/// Deliberate divergences from Bruno: padding at the ends of the URL is trimmed (issue 432), where
/// Bruno's `encodeUrl` would send a trailing space as `%20`. Each URL comes with the params table
/// `set_params` wrote it from, which is what lets its delimiter escapes pass through.
/// An input URL, the params table cells it was written from, and the URL sent.
type Trimmed = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

const TRIMMED: &[Trimmed] = &[
    (
        "http://x.test/p? q = v ",
        &[],
        "http://x.test/p?%20q%20=%20v",
    ),
    (
        "http://x.test/p?a=1%262 ",
        &[("a", "1&2 ")],
        "http://x.test/p?a=1%262",
    ),
    (
        "http://x.test/p?a=1%262  §",
        &[("a", "1&2  §")],
        "http://x.test/p?a=1%262",
    ),
    (
        "  http://x.test/p?a=1%262&b=%23x ",
        &[("a", "1&2"), ("b", "#x ")],
        "http://x.test/p?a=1%262&b=%23x",
    ),
    (
        "http://x.test/p?a=1%262 &b=1",
        &[("a", "1&2 "), ("b", "1")],
        "http://x.test/p?a=1%262%20&b=1",
    ),
];

/// `params` as the YAML entries of a request file's `http:` block.
fn query_params(params: &[(&str, &str)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let mut text = "  params:\n".to_owned();
    for (name, value) in params {
        text.push_str(&format!(
            "    - name: \"{name}\"\n      value: \"{value}\"\n      type: query\n"
        ));
    }
    text
}

#[test]
fn trimmed_padding_keeps_delimiter_escapes_with_encode_url_on() {
    let files: Vec<(String, String)> = TRIMMED
        .iter()
        .enumerate()
        .map(|(i, (url, params, _))| {
            let top = format!("{}{ENCODE}", query_params(params));
            (format!("r{i}.yml"), http(url, &top).replace('§', "\\n"))
        })
        .chain([("opencollection.yml".to_owned(), ROOT.to_owned())])
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, c, ctx) = setup(&refs);
    for (i, (input, _, expected)) in TRIMMED.iter().enumerate() {
        let r = at(&c, &ctx, &format!("r{i}.yml")).expect(input);
        assert_eq!(r.url(), *expected, "{input:?}");
        assert!(!r.url().contains("%2526"), "{input:?}");
    }
}

#[test]
fn encode_url_matches_bruno_then_whatwg() {
    let files: Vec<(String, String)> = MEASURED
        .iter()
        .enumerate()
        .map(|(i, (url, _))| (format!("r{i}.yml"), http(url, ENCODE)))
        .chain([("opencollection.yml".to_owned(), ROOT.to_owned())])
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, c, ctx) = setup(&refs);
    for (i, (input, expected)) in MEASURED.iter().enumerate() {
        let r = at(&c, &ctx, &format!("r{i}.yml")).expect(input);
        assert_eq!(r.url(), *expected, "{input}");
    }
}

#[test]
fn encoding_needs_a_settings_block_and_is_on_by_default_there() {
    let url = "http://x.test/a b?q=a%20b|^";
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("none.yml", &http(url, "")),
        ("default.yml", &http(url, "settings:\n  timeout: 0\n")),
        ("off.yml", &http(url, "settings:\n  encodeUrl: false\n")),
    ]);
    let r = |rel: &str| at(&c, &ctx, rel).expect(rel);
    // Without settings Bruno skips `encodeUrl`, and `false` is WHATWG only with a warning.
    assert_eq!(r("none.yml").url(), "http://x.test/a%20b?q=a%20b|^");
    assert_eq!(
        r("default.yml").url(),
        "http://x.test/a%20b?q=a%2520b%7C%5E"
    );
    assert_eq!(r("off.yml").url(), "http://x.test/a%20b?q=a%20b|^");
    assert_eq!(r("off.yml").warnings(), [Warning::EncodeUrlIgnored]);
}

#[test]
fn invalid_urls_are_reported_redacted() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        (
            "bad.yml",
            &http("http://me:s3cret@exa mple.com/?t=s3cret", ""),
        ),
    ]);
    match at(&c, &ctx, "bad.yml") {
        Err(e @ ResolveError::InvalidUrl { .. }) => {
            let shown = format!("{e} {e:?}");
            assert!(!shown.contains("s3cret"), "{shown}");
            assert!(
                shown.contains("http://<redacted>@exa mple.com/?t=<redacted>"),
                "{shown}"
            );
        }
        other => panic!("expected InvalidUrl, got {other:?}"),
    }
}

// Reference: the URL goes out as typed with encodeUrl off (interpolate-vars.js:187,240), and
// encodeUrl re-encodes every `%` in the query (bruno-common url/index.ts:186-228).
#[test]
fn typed_query_escapes_on_the_wire_with_encode_url_off_and_on() {
    const WIRE: &[(&str, &str)] = &[
        ("v=a%26b", "v=a%2526b"),
        ("v=a%23b", "v=a%2523b"),
        ("v=a%3Db", "v=a%253Db"),
        ("v=a%20b", "v=a%2520b"),
        ("v=a+b", "v=a%2Bb"),
        ("n%3Dx=1", "n%253Dx=1"),
    ];
    let files: Vec<(String, String)> = WIRE
        .iter()
        .enumerate()
        .flat_map(|(i, (q, _))| {
            let url = format!("http://x.test/p?{q}");
            [
                (format!("none{i}.yml"), http(&url, "")),
                (
                    format!("off{i}.yml"),
                    http(&url, "settings:\n  encodeUrl: false\n"),
                ),
                (format!("on{i}.yml"), http(&url, ENCODE)),
            ]
        })
        .chain([("opencollection.yml".to_owned(), ROOT.to_owned())])
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, c, ctx) = setup(&refs);
    for (i, (q, on)) in WIRE.iter().enumerate() {
        let raw = format!("http://x.test/p?{q}");
        for off in ["none", "off"] {
            let r = at(&c, &ctx, &format!("{off}{i}.yml")).expect(q);
            assert_eq!(r.url(), raw, "{off} {q}");
        }
        // With no params table, nothing marks an escape as the editor's, so every `%` is
        // re-encoded as the reference does.
        let r = at(&c, &ctx, &format!("on{i}.yml")).expect(q);
        assert_eq!(r.url(), format!("http://x.test/p?{on}"), "on {q}");
    }
}
