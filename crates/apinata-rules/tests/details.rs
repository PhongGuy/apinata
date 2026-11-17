//! The detail each finding carries: secret names or a fixed phrase, never a value and never
//! the parser's own words.

use apinata_rules::RULE_SET;
use potluck_kind::{Blob, Change, Role};
use yaml_serde::Value;

use crate::common::{ENV, finding, fixture, fixture_dir};

/// The detail of the one finding for `bytes`, if there is a finding.
fn detail(bytes: &[u8]) -> Option<Option<String>> {
    finding(ENV, bytes).map(|f| f.detail().map(str::to_owned))
}

/// Every phrase an `env_unparsable` detail may be, besides a `line L, column C` position.
const PHRASES: [&str; 8] = [
    "too large to check",
    "not UTF-8",
    "too many values once aliases expand",
    "aliases in a syntax the check cannot follow",
    "not valid YAML",
    "YAML tags are not supported",
    "the root is not a mapping",
    "a merge key that cannot be applied",
];

/// Whether `detail` is `line L, column C` with both numbers 1-based.
fn is_position(detail: &str) -> bool {
    let Some(rest) = detail.strip_prefix("line ") else {
        return false;
    };
    let Some((line, column)) = rest.split_once(", column ") else {
        return false;
    };
    [line, column]
        .iter()
        .all(|n| n.parse::<usize>().is_ok_and(|n| n >= 1))
}

#[test]
fn a_valued_secret_names_each_secret_once_in_document_order() {
    let bytes = b"variables:\n  - {name: b, secret: true, value: x}\n  - {name: a, secret: true, value: y}\n  - {name: b, secret: true, value: z}\n  - {name: 7, secret: true, value: w}\n  - {name: c, secret: true}\n";
    assert_eq!(detail(bytes), Some(Some("b, a".to_owned())));
    assert_eq!(
        detail(&fixture("bad", "env-secret-valued.yml")),
        Some(Some("api_key".to_owned()))
    );
    assert_eq!(
        detail(b"k: &k secret\nvariables:\n  - *k : true\n    value: x\n    name: n\n"),
        Some(Some("n".to_owned()))
    );
}

#[test]
fn a_valued_secret_without_a_string_name_has_no_detail() {
    assert_eq!(detail(b"? {secret: true, value: x}\n: 1\n"), Some(None));
    assert_eq!(
        detail(b"variables:\n  - {name: 7, secret: true, value: x}\n"),
        Some(None)
    );
}

#[test]
fn each_refusal_has_its_fixed_detail() {
    let too_large = RULE_SET.check(&Change::history(ENV, Blob::TooLarge, Role::Write));
    assert_eq!(too_large[0].detail(), Some("too large to check"));
    for (bytes, want) in [
        (&b"name: \xff\xfe\n"[..], "not UTF-8"),
        (
            &fixture("bad", "env-alias-bomb.yml")[..],
            "too many values once aliases expand",
        ),
        (
            b"k: \"a\n\tb\"\na: &s x\nb: *s\n",
            "aliases in a syntax the check cannot follow",
        ),
        (b"variables: [1, 2", "line 2, column 1"),
        (
            b"name: dev\n<<<<<<< HEAD\nx: 1\n=======\nx: 2\n>>>>>>> b\n",
            "line 3, column 1",
        ),
        (b"\xef\xbb\xbfa: b: c\n", "line 1, column 5"),
        (
            &fixture("bad", "env-two-documents.yml")[..],
            "not valid YAML",
        ),
        (
            b"v: !custom {secret: true, value: x}\n",
            "YAML tags are not supported",
        ),
        (b"", "the root is not a mapping"),
        (b"- a\n", "the root is not a mapping"),
        (
            b"variables:\n  - <<: 1\n    name: k\n",
            "a merge key that cannot be applied",
        ),
    ] {
        assert_eq!(
            detail(bytes),
            Some(Some(want.to_owned())),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

/// Every scalar under a `value` key that a plain parse finds, plus the text after each
/// `value:` on a line, so a file the reader refuses is covered too. Texts under three
/// characters are left out, since they occur inside the fixed phrases by chance.
fn values_in(bytes: &[u8]) -> Vec<String> {
    fn walk(node: &Value, under_value: bool, out: &mut Vec<String>) {
        match node {
            Value::Mapping(map) => {
                for (k, v) in map {
                    walk(k, under_value, out);
                    walk(v, under_value || k.as_str() == Some("value"), out);
                }
            }
            Value::Sequence(items) => items.iter().for_each(|i| walk(i, under_value, out)),
            Value::Tagged(tagged) => walk(&tagged.value, under_value, out),
            Value::String(s) if under_value => out.push(s.clone()),
            Value::Number(n) if under_value => out.push(n.to_string()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(mut doc) = yaml_serde::from_slice::<Value>(bytes)
        && doc.apply_merge().is_ok()
    {
        walk(&doc, false, &mut out);
    }
    for line in String::from_utf8_lossy(bytes).lines() {
        if let Some((_, rest)) = line.split_once("value:") {
            out.push(rest.trim().to_owned());
        }
    }
    out.retain(|v| v.len() >= 3);
    out
}

#[test]
fn no_detail_holds_a_value_or_parser_text_from_any_fixture() {
    let mut seen = 0;
    for dir in ["good", "bad"] {
        for entry in std::fs::read_dir(fixture_dir(dir)).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if !name.ends_with(".yml") {
                continue;
            }
            seen += 1;
            let bytes = fixture(dir, &name);
            let Some(found) = finding(ENV, &bytes) else {
                continue;
            };
            let Some(text) = found.detail() else {
                continue;
            };
            for value in values_in(&bytes) {
                assert!(
                    !text.contains(&value),
                    "{dir}/{name}: {text:?} holds {value:?}"
                );
            }
            if found.rule() == "env_unparsable" {
                assert!(
                    PHRASES.contains(&text) || is_position(text),
                    "{dir}/{name}: {text:?}"
                );
            }
        }
    }
    assert_eq!(seen, 15, "the vendored corpus");
}
