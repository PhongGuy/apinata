//! With `encodeUrl` on, only the delimiter escapes the params table accounts for pass through;
//! every other `%` in the query is re-encoded content-blind.
//!
//! Reference, at 1301ea17: packages/bruno-common/src/utils/url/index.ts:186-228 parses the query
//! raw with stripFragment false (U:212) and runs encodeURIComponent on every name (U:215) and
//! value (U:219); packages/bruno-cli/src/runner/run-single-request.js:395-396 applies it only
//! when `settings.encodeUrl` is on.

use crate::common;

use apinata_core::{Collection, ContextOptions, Param, ParamKind, RequestFile, VarContext};
use common::{at, write_tree};

const ENCODE: &str = "settings:\n  encodeUrl: true\n";

/// A collection root whose request variables are `vars` as `(name, value)`.
fn root(vars: &[(&str, &str)]) -> String {
    let mut text = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n".to_owned();
    if !vars.is_empty() {
        text.push_str("request:\n  variables:\n");
        for (name, value) in vars {
            text.push_str(&format!(
                "    - name: \"{name}\"\n      value: \"{value}\"\n"
            ));
        }
    }
    text
}

/// A request file with `url` and the hand-written `params` YAML entries under `http:`.
fn file(url: &str, params: &str, top: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{params}{top}")
}

/// One `params` entry as YAML, `disabled` when not `enabled`.
fn entry(name: &str, value: &str, enabled: bool) -> String {
    let off = if enabled {
        ""
    } else {
        "      disabled: true\n"
    };
    format!("    - name: \"{name}\"\n      value: \"{value}\"\n      type: query\n{off}")
}

/// The resolved URL of `text` under a root with `vars`, after `edit` ran on the loaded file.
fn wire(text: &str, vars: &[(&str, &str)], edit: impl FnOnce(&mut RequestFile)) -> String {
    let dir = write_tree(&[("opencollection.yml", &root(vars)), ("r.yml", text)]);
    let mut request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    edit(&mut request);
    request.save().expect("saves");
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    at(&c, &ctx, "r.yml").expect("resolves").url().to_owned()
}

/// The URL `set_params` writes for `params` as query `(name, value)` cells, resolved.
fn cells(params: &[(&str, &str)], vars: &[(&str, &str)]) -> String {
    let params = params
        .iter()
        .map(|(n, v)| Param::new(*n, *v, ParamKind::Query))
        .collect();
    wire(&file("http://x.test/p", "", ENCODE), vars, |r| {
        r.set_params(params);
    })
}

// U:215 and U:219: a typed escape is text, so its `%` is re-encoded.
#[test]
fn a_typed_delimiter_escape_is_re_encoded() {
    for (query, sent) in [
        ("v=a%26b", "v=a%2526b"),
        ("v=a%23b", "v=a%2523b"),
        ("n%3Dx=1", "n%253Dx=1"),
    ] {
        let url = format!("http://x.test/p?{query}");
        let got = wire(&file("http://x.test/p", "", ENCODE), &[], |r| {
            r.set_url(&url)
        });
        assert_eq!(got, format!("http://x.test/p?{sent}"), "{query}");
    }
}

// U:186-228 would split these cells; the table's escapes keep each one value.
#[test]
fn a_table_cell_with_a_delimiter_stays_one_value() {
    assert_eq!(cells(&[("v", "a&b")], &[]), "http://x.test/p?v=a%26b");
    assert_eq!(cells(&[("v", "a#b")], &[]), "http://x.test/p?v=a%23b");
    assert_eq!(cells(&[("n=x", "1")], &[]), "http://x.test/p?n%3Dx=1");
}

// U:219: the typed `%26` in a cell is text; only the `&` the table holds is a delimiter.
#[test]
fn a_mixed_cell_keeps_only_its_delimiter_escape() {
    assert_eq!(
        cells(&[("v", "a&b%26c")], &[]),
        "http://x.test/p?v=a%26b%2526c"
    );
}

// U:186-228: without a params table nothing accounts for an escape.
#[test]
fn a_file_without_params_is_content_blind() {
    let got = wire(&file("http://x.test/p?v=a%26b", "", ENCODE), &[], |_| {});
    assert_eq!(got, "http://x.test/p?v=a%2526b");
}

// U:219: a pair the table does not hold, a disabled entry and a second use of one entry are
// all re-encoded.
#[test]
fn only_a_matching_enabled_table_entry_keeps_an_escape() {
    let params = format!(
        "  params:\n{}{}{}",
        entry("v", "a&b", true),
        entry("w", "x&y", true),
        entry("d", "1&2", false)
    );
    let text = file(
        "http://x.test/p?v=a%26c&w=x%26y&d=1%262&w=x%26y",
        &params,
        ENCODE,
    );
    assert_eq!(
        wire(&text, &[], |_| {}),
        "http://x.test/p?v=a%2526c&w=x%26y&d=1%25262&w=x%2526y"
    );
}

// U:212: stripFragment is false, so text after `#` is query data and is never marked.
#[test]
fn pairs_at_or_past_a_raw_hash_are_content_blind() {
    let params = format!(
        "  params:\n{}{}",
        entry("a", "1", true),
        entry("v", "a&b", true)
    );
    let text = file("http://x.test/p?a=1#&v=a%26b", &params, ENCODE);
    assert_eq!(wire(&text, &[], |_| {}), "http://x.test/p?a=1%23&v=a%2526b");
}

// U:186-228 sees the expanded text; a `?`, `&` or `#` inside a template does not move the
// table's escapes off their pair.
#[test]
fn a_template_with_a_query_delimiter_does_not_drift_marks() {
    for name in ["q?r", "q&r", "q#r"] {
        let template = format!("{{{{{name}}}}}");
        let got = cells(&[("t", &template), ("v", "a&b")], &[(name, "1")]);
        assert_eq!(got, "http://x.test/p?t=1&v=a%26b", "{name}");
    }
}

// U:219: an escape a variable expands to is text; the table's escape next to it stays.
#[test]
fn a_variable_next_to_a_table_escape_keeps_it() {
    assert_eq!(
        cells(&[("v", "{{x}}&b")], &[("x", "1%26")]),
        "http://x.test/p?v=1%2526%26b"
    );
    let got = cells(&[("v", "{{$guid}}&b")], &[]);
    assert!(got.ends_with("%26b"), "{got}");
    assert!(!got.contains("%2526"), "{got}");
}

// run-single-request.js:395-396: with encodeUrl off the URL goes out as written.
#[test]
fn encode_url_off_sends_every_escape_as_written() {
    let off = "settings:\n  encodeUrl: false\n";
    let got = wire(&file("http://x.test/p", "", off), &[], |r| {
        r.set_url("http://x.test/p?v=a%26b&n%3Dx=a%23b");
    });
    assert_eq!(got, "http://x.test/p?v=a%26b&n%3Dx=a%23b");
}
