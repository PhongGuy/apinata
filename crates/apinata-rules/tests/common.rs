//! Helpers shared by the rule tests: the vendored fixtures and a check's verdict.

use std::path::{Path, PathBuf};

use apinata_rules::RULE_SET;
use potluck_kind::{Blob, Change, Code, Finding, Role};

/// The environment path most tests check under.
pub const ENV: &str = "environments/x.yml";

/// A rule name and the path it was reported at.
pub type Verdict = Option<(&'static str, String)>;

/// The vendored fixture directory, `good` or `bad`.
pub fn fixture_dir(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir)
}

/// A vendored fixture's bytes.
pub fn fixture(dir: &str, name: &str) -> Vec<u8> {
    let path = fixture_dir(dir).join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The one finding `check` gives for `bytes` at `path`, if any; never more than one.
pub fn finding(path: &str, bytes: &[u8]) -> Option<Finding> {
    let mut found = RULE_SET.check(&Change::history(path, Blob::Bytes(bytes), Role::Write));
    assert!(found.len() <= 1, "{found:?}");
    found.pop()
}

/// The finding's rule and path, its code asserted, its detail left out.
pub fn verdict(path: &str, bytes: &[u8]) -> Verdict {
    finding(path, bytes).map(|f| {
        assert_eq!(f.code(), Code::SecretValue, "{f:?}");
        (f.rule(), f.path().to_owned())
    })
}

/// A valued secret at `path`.
pub fn hit(path: &str) -> Verdict {
    Some(("secret_value", path.to_owned()))
}

/// A file Apinata's reader refuses, at `path`.
pub fn unparsable(path: &str) -> Verdict {
    Some(("env_unparsable", path.to_owned()))
}
