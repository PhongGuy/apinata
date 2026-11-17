//! Aliases, merge keys and the expansion bound: an alias or a merge cannot carry a value past
//! the walk, and a merge yaml_serde cannot apply is refused.

use crate::common::{ENV, fixture, hit, unparsable, verdict};

fn wide(aliases: usize) -> Vec<u8> {
    let mut s = String::from("a: &a [");
    s.push_str(&vec!["x"; 4000].join(", "));
    s.push_str("]\nb: [");
    s.push_str(&vec!["*a"; aliases].join(", "));
    s.push_str("]\n");
    s.into_bytes()
}

#[test]
fn the_real_bound_refuses_a_wide_alias_fan_out() {
    // 501 copies of a 4,002-event anchor pass 2,000,000 events.
    assert_eq!(verdict(ENV, &wide(501)), unparsable(ENV));
}

#[test]
fn an_alias_or_a_merge_cannot_hide_a_valued_secret() {
    let alias = fixture("bad", "env-alias-secret-value.yml");
    let merge = fixture("bad", "env-merge-secret-value.yml");
    for (name, bytes) in [
        ("env-alias-secret-value.yml", &alias[..]),
        ("env-merge-secret-value.yml", &merge[..]),
        (
            "aliased whole variable",
            b"name: dev\nvariables:\n  - &v\n    secret: true\n    name: a\n    value: x\n  - *v\n",
        ),
        (
            "merge list, value from the second mapping",
            b"a: &a {secret: true}\nb: &b {value: x}\nvariables:\n  - <<: [*a, *b]\n    name: k\n",
        ),
        (
            "merge brings secret, explicit value",
            b"a: &a {secret: true}\nvariables:\n  - <<: *a\n    name: k\n    value: x\n",
        ),
    ] {
        assert_eq!(verdict(ENV, bytes), hit(ENV), "{name}");
    }
}

#[test]
fn a_merge_without_a_value_passes() {
    let bytes = fixture("good", "env-merge-secret-unvalued.yml");
    assert_eq!(verdict(ENV, &bytes), None);
}

#[test]
fn an_explicit_null_value_beats_a_merged_value() {
    // The anchor alone carries no `secret`, the variable alone no value.
    let bytes =
        b"a: &a {value: x}\nvariables:\n  - <<: *a\n    secret: true\n    name: k\n    value:\n";
    assert_eq!(verdict(ENV, bytes), None);
}

#[test]
fn a_merge_under_a_tag_is_unparsable() {
    let bytes = b"a: &a {value: x}\nv: !t {<<: *a, secret: true}\n";
    assert_eq!(verdict(ENV, bytes), unparsable(ENV));
}

#[test]
fn a_merge_yaml_serde_cannot_apply_is_unparsable() {
    for (name, bytes) in [
        ("scalar merge", &b"variables:\n  - <<: 1\n    name: k\n"[..]),
        ("tagged merge", b"v: {<<: !t {k: 1}}\n"),
        (
            "scalar in merge list",
            b"a: &a {secret: true}\nv: {<<: [*a, 1]}\n",
        ),
    ] {
        assert_eq!(verdict(ENV, bytes), unparsable(ENV), "{name}");
    }
}

#[test]
fn a_plain_merge_key_is_not_a_variable_key() {
    // The merge key vanishes; a mapping with only `<<` and `name` has no value.
    let bytes = b"a: &a {name: base}\nvariables:\n  - <<: *a\n    secret: true\n";
    assert_eq!(verdict(ENV, bytes), None);
}

#[test]
fn an_alias_used_as_a_mapping_key_is_read_as_its_value() {
    assert_eq!(verdict(ENV, b"a: &a [1, 2, 3]\n? *a\n: v\n"), None);
    // An anchored `secret` scalar spelled as an alias key is still the `secret` key.
    let valued = b"k: &k secret\nvariables:\n  - *k : true\n    value: x\n    name: n\n";
    assert_eq!(verdict(ENV, valued), hit(ENV));
    let unvalued = b"k: &k secret\nvariables:\n  - *k : true\n    name: n\n";
    assert_eq!(verdict(ENV, unvalued), None);
}

#[test]
fn a_chained_merge_cannot_hide_a_valued_secret() {
    for (name, bytes) in [
        (
            "plain chain",
            &b"base: &b {value: x}\nmid: &m {<<: *b}\nvariables:\n  - <<: *m\n    secret: true\n    name: k\n"[..],
        ),
        (
            "chain through merge lists",
            b"base: &b {value: x}\nmid: &m {<<: [*b]}\nvariables:\n  - <<: [*m]\n    secret: true\n    name: k\n",
        ),
    ] {
        assert_eq!(verdict(ENV, bytes), hit(ENV), "{name}");
    }
}

#[test]
fn an_alias_bomb_expanding_only_into_keys_is_unparsable() {
    // Ten levels of ten-fold aliases, every anchor inside a key: 10^10 nodes unexpanded.
    let mut s = String::from("? &l0 [x, x, x, x, x, x, x, x, x, x]\n: 0\n");
    for i in 1..10 {
        let prev = format!("*l{}", i - 1);
        s.push_str(&format!("? &l{i} [{}]\n: {i}\n", vec![prev; 10].join(", ")));
    }
    assert_eq!(verdict(ENV, s.as_bytes()), unparsable(ENV));
}

// One 4000-scalar anchor, then `entries` distinct keys `[*a, i]` each expanding it, then a
// valued secret as the last key.
fn key_fan_out(entries: usize) -> Vec<u8> {
    let mut s = String::from("? &a [");
    s.push_str(&vec!["x"; 4000].join(", "));
    s.push_str("]\n: 0\n");
    for i in 0..entries {
        s.push_str(&format!("? [*a, {i}]\n: {i}\n"));
    }
    s.push_str("? {secret: true, value: x}\n: end\n");
    s.into_bytes()
}

#[test]
fn a_key_alias_fan_out_is_walked_to_its_last_key_under_the_real_bound() {
    // Each entry is 4,006 events: 498 entries are the most the bound accepts.
    assert_eq!(verdict(ENV, &key_fan_out(497)), hit(ENV));
    assert_eq!(verdict(ENV, &key_fan_out(501)), unparsable(ENV));
}

#[test]
fn a_merge_inside_a_mapping_key_is_unparsable() {
    // yaml_serde's apply_merge folds values only, so a `<<` inside a key is a merge it
    // cannot apply, valued or not.
    for (name, bytes) in [
        (
            "merge brings the value into a key",
            &b"a: &a {value: x}\nv: {? {<<: *a, secret: true} : 1}\n"[..],
        ),
        (
            "benign merge in a key",
            b"a: &a {name: n}\nv: {? {<<: *a} : 1}\n",
        ),
        (
            "merge in a key of a key",
            b"a: &a {name: n}\n? {? {<<: *a} : 1}\n: 1\n",
        ),
        (
            "merge in a key under a tag",
            b"a: &a {name: n}\nv: !t {? [{<<: *a}] : 1}\n",
        ),
    ] {
        assert_eq!(verdict(ENV, bytes), unparsable(ENV), "{name}");
    }
}
