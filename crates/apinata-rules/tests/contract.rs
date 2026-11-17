//! The rule set's fixed answers: kind, contract version, formats, which paths it reads, and
//! the seed's files.

use apinata_rules::RULE_SET;
use potluck_kind::{Blob, Change, Code, RULES_ABI, Role};

// What Apinata's writer produces for `Collection::create(dir, "Workspace")`.
const SEED_COLLECTION: &[u8] = b"opencollection: 1.0.0\n\ninfo:\n  name: Workspace\nbundled: false\nextensions:\n  bruno:\n    ignore:\n      - node_modules\n      - .git\n";

// Paths `reads` refuses: nested, other directories, other cases and extensions.
const NOT_READ: [&str; 14] = [
    "Environments/dev.yml",
    "sub/environments/dev.yml",
    "collections/api/environments/dev.yml",
    "environments/dev.yaml",
    "environments/dev.YML",
    "environments/nested/dev.yml",
    "my-environments/dev.yml",
    "dev.yml",
    "environments.yml",
    "environments/dev.yml.bak",
    "/environments/dev.yml",
    "-",
    "/",
    "",
];

/// The code, rule name and path of each finding for a change, details left out.
fn verdicts(change: &Change<'_>) -> Vec<(Code, &'static str, String)> {
    RULE_SET
        .check(change)
        .iter()
        .map(|f| (f.code(), f.rule(), f.path().to_owned()))
        .collect()
}

#[test]
fn apinata_states_its_contract() {
    assert_eq!(
        (
            RULE_SET.kind(),
            RULE_SET.abi(),
            RULE_SET.formats(),
            RULE_SET.format_file()
        ),
        ("apinata", 1, &["1.0.0"] as &[&str], "opencollection.yml")
    );
}

#[test]
fn the_abi_literal_is_the_pinned_contract_version() {
    assert_eq!(RULE_SET.abi(), RULES_ABI);
}

#[test]
fn the_format_detector_reads_only_the_top_level_version() {
    for (bytes, want) in [
        (&b"opencollection: 1.0.0\n"[..], Some("1.0.0")),
        (b"opencollection: \"1.0.0\"\n", Some("1.0.0")),
        (b"opencollection: '1.0.0'\n", Some("1.0.0")),
        (
            b"\xef\xbb\xbfopencollection: 1.0.0\r\ninfo: {}\r\n",
            Some("1.0.0"),
        ),
        (
            b"info:\n  name: x\nopencollection: 2.0.0 # newer\n",
            Some("2.0.0"),
        ),
        (b"opencollection: 1.0.0\n{{ not yaml", Some("1.0.0")),
        (b"info:\n  opencollection: 1.0.0\n", None),
        (b"opencollection:\n", None),
        (b"opencollection: ''\n", None),
        (b"name: x\n", None),
        (b"", None),
        (b"opencollection: \xff\n", None),
    ] {
        assert_eq!(
            RULE_SET.detect_format(bytes).as_deref(),
            want,
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn only_root_environment_yml_files_are_read() {
    for p in [
        "environments/dev.yml",
        "environments/Dev.yml",
        "environments/a-b_c.yml",
    ] {
        assert!(RULE_SET.reads(p), "{p}");
    }
    for p in NOT_READ {
        assert!(!RULE_SET.reads(p), "{p}");
    }
}

#[test]
fn no_path_is_judged_by_its_name() {
    for p in ["environments/dev.yml", ".env", "notes/x.yml", ""] {
        assert_eq!(RULE_SET.path_rule(p), None, "{p}");
    }
}

#[test]
fn a_path_it_does_not_read_gives_no_finding() {
    for p in NOT_READ {
        for blob in [Blob::Bytes(b"secret: true\nvalue: x\n"), Blob::TooLarge] {
            assert!(
                verdicts(&Change::history(p, blob, Role::Write)).is_empty(),
                "{p}"
            );
        }
    }
}

#[test]
fn a_deleted_environment_file_passes() {
    let p = "environments/dev.yml";
    let old = Some(Blob::Bytes(b"secret: true\nvalue: x\n" as &[u8]));
    assert!(verdicts(&Change::tip(p, old, None, Role::Write)).is_empty());
    assert!(verdicts(&Change::tip(p, Some(Blob::TooLarge), None, Role::Admin)).is_empty());
}

#[test]
fn an_environment_file_too_large_to_read_is_unparsable() {
    let p = "environments/big.yml";
    let want = vec![(Code::SecretValue, "env_unparsable", p.to_owned())];
    assert_eq!(
        verdicts(&Change::history(p, Blob::TooLarge, Role::Write)),
        want
    );
    assert_eq!(
        verdicts(&Change::tip(p, None, Some(Blob::TooLarge), Role::Read)),
        want
    );
}

#[test]
fn the_seed_is_the_gitignore_and_the_collection_byte_for_byte() {
    let files = RULE_SET.seed();
    let paths: Vec<&str> = files.iter().map(|f| f.path()).collect();
    assert_eq!(paths, [".gitignore", "opencollection.yml"]);
    assert_eq!(files[0].bytes(), b".env\n");
    assert_eq!(files[1].bytes(), SEED_COLLECTION);
    assert_eq!(files[1].bytes().len(), 129);
}

#[test]
fn the_seed_detects_to_a_listed_format() {
    let seed = RULE_SET.seed();
    let file = seed
        .iter()
        .find(|f| f.path() == RULE_SET.format_file())
        .unwrap();
    assert_eq!(
        RULE_SET.detect_format(file.bytes()).as_deref(),
        Some("1.0.0")
    );
    assert!(RULE_SET.formats().contains(&"1.0.0"));
}
