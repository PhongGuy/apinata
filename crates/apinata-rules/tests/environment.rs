//! The environment rule: what `check` refuses in `environments/*.yml`, ported from Potluck's
//! in-tree tests and judged by Apinata's reader.

use apinata_rules::RULE_SET;
use potluck_kind::{Blob, Change, Role};

use crate::common::{ENV, fixture, hit, unparsable, verdict};

#[test]
fn rule_names_are_the_contract_names() {
    assert_eq!(
        verdict(ENV, &fixture("bad", "env-secret-valued.yml")),
        Some(("secret_value", ENV.to_owned()))
    );
    assert_eq!(
        verdict(ENV, &fixture("bad", "env-not-yaml.yml")),
        Some(("env_unparsable", ENV.to_owned()))
    );
}

#[test]
fn the_rule_follows_the_path() {
    let bad = fixture("bad", "env-secret-valued.yml");
    assert_eq!(
        verdict("environments/dev.yml", &bad),
        hit("environments/dev.yml")
    );
    for p in [
        "api/environments/dev.yml",
        "Environments/dev.yml",
        "environments/dev.yaml",
        "notes/dev.yml",
        "-",
        "/",
    ] {
        assert_eq!(verdict(p, &bad), None, "{p}");
    }
}

#[test]
fn check_ignores_baseline_and_role() {
    let p = "environments/dev.yml";
    let bad = fixture("bad", "env-secret-valued.yml");
    let history = RULE_SET.check(&Change::history(p, Blob::Bytes(&bad), Role::Admin));
    let tip = RULE_SET.check(&Change::tip(
        p,
        Some(Blob::TooLarge),
        Some(Blob::Bytes(&bad)),
        Role::Read,
    ));
    assert_eq!(history, tip);
    assert_eq!(verdict(p, &bad), hit(p));
}

#[test]
fn valued_secrets_are_refused_in_every_shape() {
    for name in [
        "env-secret-valued.yml",
        "env-secret-typed-value.yml",
        "env-secret-nested.yml",
    ] {
        assert_eq!(verdict(ENV, &fixture("bad", name)), hit(ENV), "{name}");
    }
}

#[test]
fn an_empty_string_value_is_still_a_value() {
    let bytes = b"variables:\n  - secret: true\n    name: k\n    value: ''\n";
    assert_eq!(verdict(ENV, bytes), hit(ENV));
}

#[test]
fn unvalued_false_and_null_pass() {
    for name in [
        "env-secret-unvalued.yml",
        "env-secret-false.yml",
        "env-secret-null-value.yml",
    ] {
        assert_eq!(verdict(ENV, &fixture("good", name)), None, "{name}");
    }
}

#[test]
fn an_empty_file_is_unparsable() {
    assert_eq!(verdict(ENV, b""), unparsable(ENV));
    assert_eq!(verdict(ENV, b"\n  \n"), unparsable(ENV));
}

#[test]
fn a_root_that_is_not_a_mapping_is_unparsable() {
    for bytes in [
        &b"- secret: true\n  value: x\n"[..],
        b"- a\n",
        b"just text\n",
        b"42\n",
    ] {
        assert_eq!(
            verdict(ENV, bytes),
            unparsable(ENV),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn files_apinata_cannot_read_are_refused() {
    for name in [
        "env-two-documents.yml",
        "env-not-yaml.yml",
        "env-duplicate-key-top.yml",
        "env-duplicate-key-variable.yml",
        "env-deep-nesting.yml",
        "env-alias-bomb.yml",
    ] {
        assert_eq!(
            verdict(ENV, &fixture("bad", name)),
            unparsable(ENV),
            "{name}"
        );
    }
    for (name, bytes) in [
        ("unterminated flow sequence", &b"variables: [1, 2"[..]),
        ("invalid utf-8", b"name: \xff\xfe\n"),
        ("unknown alias", b"name: *nowhere\n"),
        ("flow duplicate", b"{a: 1, a: 2}"),
    ] {
        assert_eq!(verdict(ENV, bytes), unparsable(ENV), "{name}");
    }
}

#[test]
fn unparsable_is_only_for_environment_files() {
    let bytes = fixture("bad", "env-not-yaml.yml");
    assert_eq!(verdict("notes/x.yml", &bytes), None);
    assert_eq!(verdict("environments/x.yaml", &bytes), None);
}

#[test]
fn a_leading_bom_is_stripped_before_the_walk() {
    let valued = b"\xef\xbb\xbfvariables:\n  - secret: true\n    name: k\n    value: v\n";
    assert_eq!(verdict(ENV, valued), hit(ENV));
    assert_eq!(verdict(ENV, b"\xef\xbb\xbfname: dev\n"), None);
}

#[test]
fn a_tagged_mapping_cannot_hide_a_valued_secret() {
    let bytes = b"name: dev\nv: !custom {secret: true, value: fixture-tagged}\n";
    assert_eq!(
        verdict(ENV, bytes),
        unparsable(ENV),
        "a local tag is refused"
    );
}

#[test]
fn a_core_tag_reads_as_untagged() {
    assert_eq!(verdict(ENV, b"name: !!str dev\n"), None);
    let valued = b"variables:\n  - secret: !!bool true\n    name: k\n    value: !!str x\n";
    assert_eq!(verdict(ENV, valued), hit(ENV));
}

#[test]
fn aliases_past_a_syntax_the_bound_cannot_follow_are_unparsable() {
    // A tab-led quoted continuation that libyaml accepts and the event bound cannot parse.
    let bytes = b"k: \"a\n\tb\"\na: &s x\nb: *s\n";
    assert_eq!(verdict(ENV, bytes), unparsable(ENV));
}

#[test]
fn the_reader_keeps_every_refusal_of_the_plain_parse() {
    for name in [
        "env-duplicate-key-variable.yml",
        "env-deep-nesting.yml",
        "env-alias-bomb.yml",
    ] {
        assert_eq!(
            verdict(ENV, &fixture("bad", name)),
            unparsable(ENV),
            "{name}"
        );
    }
    assert_eq!(verdict(ENV, b"{a: 1, a: 2}"), unparsable(ENV));
    assert_eq!(
        verdict(
            ENV,
            b"name: dev\nv: !custom {secret: true, value: fixture-tagged}\n"
        ),
        unparsable(ENV),
        "a tagged mapping is refused, not walked"
    );
}

#[test]
fn a_valued_secret_in_a_mapping_key_is_refused() {
    for (name, bytes) in [
        (
            "block complex key",
            &b"variables:\n  - ? {secret: true, value: x}\n    : 1\n"[..],
        ),
        ("flow complex key", b"v: {? {secret: true, value: x} : 1}\n"),
        ("key of a key", b"? {? {secret: true, value: x} : 1}\n: 1\n"),
        (
            "inside a sequence key",
            b"? [{secret: true, value: x}]\n: 1\n",
        ),
        (
            "inside a value of a key",
            b"? {v: {secret: true, value: x}}\n: 1\n",
        ),
        (
            "anchored in a key, aliased as a key",
            b"? &s {secret: true, value: x}\n: 1\nb: {? *s : 2}\n",
        ),
    ] {
        assert_eq!(verdict(ENV, bytes), hit(ENV), "{name}");
    }
    assert_eq!(
        verdict(ENV, b"? !t {secret: true, value: x}\n: 1\n"),
        unparsable(ENV),
        "tagged key"
    );
}

#[test]
fn an_unvalued_secret_in_a_mapping_key_passes() {
    let bytes = b"? {secret: true, value: null}\n: 1\n";
    assert_eq!(verdict(ENV, bytes), None);
}

// Nested complex keys, the secret innermost: `{? {? ... {secret: true, value: x} : 1} : 1}`.
fn nested_keys(depth: usize) -> Vec<u8> {
    let mut s = "{? ".repeat(depth);
    s.push_str("{secret: true, value: x}");
    s.push_str(&" : 1}".repeat(depth));
    s.into_bytes()
}

#[test]
fn key_nesting_is_walked_to_the_parser_depth_limit_and_refused_past_it() {
    assert_eq!(verdict(ENV, &nested_keys(120)), hit(ENV));
    assert_eq!(verdict(ENV, &nested_keys(200)), unparsable(ENV));
}
