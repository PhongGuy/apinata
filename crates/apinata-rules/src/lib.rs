//! Apinata's push rules for team workspaces, as a potluck-kind rule set: which files of an
//! Apinata workspace a push may not carry, the collection format it declares, and the files a
//! new workspace starts with.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use potluck_kind::{Change, Code, Finding, RuleSet, SeedFile};

/// The rule set Potluck registers for kind `apinata`.
pub const RULE_SET: &dyn RuleSet = &Apinata;

/// The rule an environment file breaks when it holds a valued secret.
#[expect(
    dead_code,
    reason = "the environment walk that reports it is not wired in yet"
)]
const SECRET_VALUE: &str = "secret_value";
/// The rule an environment file breaks when it cannot be read and so cannot be vouched for.
const ENV_UNPARSABLE: &str = "env_unparsable";

#[derive(Debug)]
struct Apinata;

impl RuleSet for Apinata {
    fn kind(&self) -> &'static str {
        "apinata"
    }

    // The literal, not `RULES_ABI`: built inside Potluck, that constant is Potluck's own, so only
    // a literal lets its runtime version check catch a mismatch.
    fn abi(&self) -> u32 {
        1
    }

    fn formats(&self) -> &'static [&'static str] {
        &["1.0.0"]
    }

    fn format_file(&self) -> &'static str {
        "opencollection.yml"
    }

    fn detect_format(&self, format_file: &[u8]) -> Option<String> {
        detect_format(format_file)
    }

    fn path_rule(&self, _path: &str) -> Option<Finding> {
        None
    }

    fn reads(&self, path: &str) -> bool {
        is_environment_file(path)
    }

    fn check(&self, change: &Change<'_>) -> Vec<Finding> {
        let path = change.path();
        if !is_environment_file(path) {
            return Vec::new();
        }
        // Fail closed: a blob the hook did not read, or bytes not yet judged, cannot be vouched
        // for. A deleted file has no new side and passes.
        match change.new_blob() {
            None => Vec::new(),
            Some(_) => vec![Finding::new(Code::SecretValue, ENV_UNPARSABLE, path)],
        }
    }

    fn seed(&self) -> Vec<SeedFile> {
        vec![
            SeedFile::new(".gitignore", b".env\n"),
            SeedFile::new(
                "opencollection.yml",
                include_bytes!("../seed/opencollection.yml"),
            ),
        ]
    }
}

/// A top-level environment file: `environments/<name>.yml` with no further directory.
fn is_environment_file(path: &str) -> bool {
    path.strip_prefix("environments/")
        .is_some_and(|name| !name.contains('/') && name.ends_with(".yml"))
}

/// The format version `opencollection.yml` declares: the first top-level `opencollection:`
/// line's value, with a trailing ` #` comment and one pair of matching quotes removed.
/// `None` for bytes that are not UTF-8, no such line, or an empty value.
fn detect_format(bytes: &[u8]) -> Option<String> {
    // A line scan, not a YAML parse: the version must be readable even when the rest of the
    // file is not, and only a key at column 0 is top level.
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let line = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .find_map(|l| l.strip_prefix("opencollection:"))?;
    let value = line.split_once(" #").map_or(line, |(v, _)| v).trim();
    let value = ['"', '\'']
        .iter()
        .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
        .unwrap_or(value);
    (!value.is_empty()).then(|| value.to_owned())
}
