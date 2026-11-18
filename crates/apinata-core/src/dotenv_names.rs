//! The names a literal secret takes when it moves from a collection file into the
//! collection's `.env`, where the file then refers to it as `{{process.env.NAME}}`.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// The base `.env` name of the collection's proxy password.
pub const PROXY_PASSWORD_DOTENV: &str = "APINATA_PROXY_PASSWORD";
/// The base `.env` name of an OAuth 2.0 client secret.
pub const OAUTH2_CLIENT_SECRET_DOTENV: &str = "APINATA_OAUTH2_CLIENT_SECRET";
/// The base `.env` name of an OAuth 2.0 resource-owner password.
pub const OAUTH2_PASSWORD_DOTENV: &str = "APINATA_OAUTH2_PASSWORD";

/// The base `.env` name for the passphrase of client certificate row `row`, counted from zero.
pub fn passphrase_dotenv_base(row: usize) -> String {
    format!("APINATA_CERT_PASSPHRASE_{}", row + 1)
}

/// Whether `value` holds a `{{...}}` reference, so it is written as is rather than moved to
/// `.env`. A stray `{{` with no `}}` after it is a literal secret.
pub fn is_template(value: &str) -> bool {
    value
        .find("{{")
        .is_some_and(|open| value[open + 2..].contains("}}"))
}

/// The `.env` name a value refers to when it is exactly `{{process.env.NAME}}` and NAME is
/// a portable environment variable name: a letter or `_`, then letters, digits or `_`.
pub fn dotenv_reference_name(value: &str) -> Option<&str> {
    let name = value.strip_prefix("{{process.env.")?.strip_suffix("}}")?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let portable = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    portable.then_some(name)
}

/// `base` when it is not in `taken`, else the first of `base_2`, `base_3`, ... that is not.
pub fn free_dotenv_name(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    let mut n = 2u64;
    loop {
        let name = format!("{base}_{n}");
        if !taken.contains(&name) {
            return name;
        }
        n += 1;
    }
}

/// Whether git tracks `<root>/.env`, so a secret written there would be committed. The
/// variables a git hook may set are removed so the question goes to the repository around
/// `root`; any failure to ask reads as untracked.
pub fn dotenv_tracked_by_git(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", "--", ".env"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The secrets an edit moves to `.env`, as (name, value), with the names chosen so far.
/// Its `Debug` shows only the names, since the values are secrets.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DotenvMoves {
    known: BTreeSet<String>,
    all: bool,
    taken: BTreeSet<String>,
    referenced: BTreeSet<String>,
    planned: Vec<(String, String)>,
}

impl DotenvMoves {
    /// No moves yet. Literals in `known` (already in the file) stay unless `all`; names in
    /// `taken` are never chosen, and names in `referenced`, which edited values still use,
    /// are never reused for another field.
    pub fn new(
        known: BTreeSet<String>,
        all: bool,
        taken: BTreeSet<String>,
        referenced: BTreeSet<String>,
    ) -> DotenvMoves {
        DotenvMoves {
            known,
            all,
            taken,
            referenced,
            planned: Vec::new(),
        }
    }

    /// Replace a literal `value` with a `.env` reference when it moves: under the name the
    /// field's `previous` value referred to while no edited value still does, else the first
    /// free name from `base`. Empty values and templates stay as they are.
    pub fn plan(&mut self, value: &mut String, previous: &str, base: &str) {
        let literal = !value.is_empty() && !is_template(value);
        if !literal || (!self.all && self.known.contains(value.as_str())) {
            return;
        }
        let name = match dotenv_reference_name(previous) {
            Some(name) if !self.referenced.contains(name) => name.to_owned(),
            _ => free_dotenv_name(base, &self.taken),
        };
        self.taken.insert(name.clone());
        let reference = format!("{{{{process.env.{name}}}}}");
        self.planned
            .push((name, std::mem::replace(value, reference)));
    }

    /// The moves planned so far, as (name, value), in planning order.
    pub fn planned(&self) -> &[(String, String)] {
        &self.planned
    }

    /// The moves planned, as (name, value), in planning order.
    pub fn into_planned(self) -> Vec<(String, String)> {
        self.planned
    }
}

impl std::fmt::Debug for DotenvMoves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.planned.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("DotenvMoves")
            .field("all", &self.all)
            .field("planned", &names)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn names_count_up_from_the_base_past_taken_ones() {
        assert_eq!(passphrase_dotenv_base(0), "APINATA_CERT_PASSPHRASE_1");
        let taken = set(&["APINATA_PROXY_PASSWORD", "APINATA_PROXY_PASSWORD_2"]);
        assert_eq!(
            free_dotenv_name(PROXY_PASSWORD_DOTENV, &taken),
            "APINATA_PROXY_PASSWORD_3"
        );
        assert_eq!(free_dotenv_name("OTHER", &taken), "OTHER");
    }

    #[test]
    fn a_reference_names_only_a_portable_variable() {
        assert_eq!(
            dotenv_reference_name("{{process.env.MY_PW}}"),
            Some("MY_PW")
        );
        assert_eq!(dotenv_reference_name("{{process.env.MY_PW}}x"), None);
        assert_eq!(dotenv_reference_name("{{pw}}"), None);
        for inner in ["", "tok en", "a/b", "é", "api.key", "my-var_1", "1abc"] {
            let template = format!("{{{{process.env.{inner}}}}}");
            assert_eq!(dotenv_reference_name(&template), None, "{template:?}");
        }
        for inner in ["tok", "API_KEY", "_x1", OAUTH2_CLIENT_SECRET_DOTENV] {
            let template = format!("{{{{process.env.{inner}}}}}");
            assert_eq!(
                dotenv_reference_name(&template),
                Some(inner),
                "{template:?}"
            );
        }
    }

    #[test]
    fn a_template_needs_a_closing_pair_after_the_opening_one() {
        assert!(is_template("{{x}}"));
        assert!(is_template("a{{x}}b"));
        assert!(!is_template("ab{{c"));
        assert!(!is_template("}}{{"));
        assert!(!is_template("plain"));
    }

    #[test]
    fn plan_moves_a_new_literal_under_the_first_free_name() {
        let mut moves = DotenvMoves::new(
            BTreeSet::new(),
            false,
            set(&[PROXY_PASSWORD_DOTENV]),
            BTreeSet::new(),
        );
        let mut value = "hunter2".to_owned();
        moves.plan(&mut value, "", PROXY_PASSWORD_DOTENV);
        assert_eq!(value, "{{process.env.APINATA_PROXY_PASSWORD_2}}");
        assert_eq!(
            moves.planned(),
            [("APINATA_PROXY_PASSWORD_2".to_owned(), "hunter2".to_owned())]
        );
        assert!(!format!("{moves:?}").contains("hunter2"));
    }

    #[test]
    fn plan_reuses_the_previous_name_unless_still_referenced() {
        let previous = "{{process.env.MY_PW}}";
        let mut moves = DotenvMoves::new(BTreeSet::new(), false, set(&["MY_PW"]), BTreeSet::new());
        let mut value = "new".to_owned();
        moves.plan(&mut value, previous, PROXY_PASSWORD_DOTENV);
        assert_eq!(value, "{{process.env.MY_PW}}");

        let mut moves = DotenvMoves::new(BTreeSet::new(), false, set(&["MY_PW"]), set(&["MY_PW"]));
        let mut value = "new".to_owned();
        moves.plan(&mut value, previous, PROXY_PASSWORD_DOTENV);
        assert_eq!(value, "{{process.env.APINATA_PROXY_PASSWORD}}");
    }

    #[test]
    fn plan_keeps_known_literals_templates_and_empty_values_unless_all() {
        let known = set(&["old"]);
        let mut moves = DotenvMoves::new(known.clone(), false, BTreeSet::new(), BTreeSet::new());
        for v in ["old", "{{x}}", ""] {
            let mut value = v.to_owned();
            moves.plan(&mut value, "", PROXY_PASSWORD_DOTENV);
            assert_eq!(value, v);
        }
        assert!(moves.planned().is_empty());

        let mut moves = DotenvMoves::new(known, true, BTreeSet::new(), BTreeSet::new());
        let mut value = "old".to_owned();
        moves.plan(&mut value, "", PROXY_PASSWORD_DOTENV);
        assert_eq!(moves.into_planned().len(), 1);
    }
}
