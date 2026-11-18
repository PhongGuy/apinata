//! Moving the proxy password and certificate passphrases into the collection's `.env`, under
//! the names the app gives them, with the file referring to each as `{{process.env.NAME}}`.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, ensure};
use apinata_core::{Collection, CollectionNetwork, CollectionProxy, dotenv_quotable};

/// The base `.env` name of the collection's proxy password.
pub(crate) const PROXY_PASSWORD: &str = "APINATA_PROXY_PASSWORD";

/// The base `.env` name for the passphrase of certificate row `row`, counted from zero.
pub(crate) fn passphrase_base(row: usize) -> String {
    format!("APINATA_CERT_PASSPHRASE_{}", row + 1)
}

/// The secrets an edit moves to `.env`, as (name, value), choosing names as the app does.
pub(crate) struct Moves {
    taken: BTreeSet<String>,
    /// Names the edited values still refer to, which a moved value must not take over.
    referenced: BTreeSet<String>,
    pub(crate) planned: Vec<(String, String)>,
}

impl Moves {
    /// No moves yet for a collection whose settings after the edit are `after`.
    pub(crate) fn new(c: &Collection, after: &CollectionNetwork) -> anyhow::Result<Moves> {
        let referenced: BTreeSet<String> = secrets(after)
            .filter_map(env_name)
            .map(str::to_owned)
            .collect();
        let mut taken = c
            .dotenv_keys()
            .map_err(|e| anyhow::anyhow!("could not read .env: {e}"))?;
        taken.extend(referenced.iter().cloned());
        Ok(Moves {
            taken,
            referenced,
            planned: Vec::new(),
        })
    }

    /// Replace a literal `value` with a `.env` reference: under the name the field's
    /// `previous` value referred to while nothing else still does, else the first free name
    /// from `base`. Empty values and templates stay as they are.
    pub(crate) fn plan(&mut self, value: &mut String, previous: &str, base: &str) {
        if value.is_empty() || is_template(value) {
            return;
        }
        let name = match env_name(previous) {
            Some(name) if !self.referenced.contains(name) => name.to_owned(),
            _ => dotenv_name(base, &self.taken),
        };
        self.taken.insert(name.clone());
        let reference = format!("{{{{process.env.{name}}}}}");
        self.planned
            .push((name, std::mem::replace(value, reference)));
    }

    /// Write the planned values to `.env`, keeping `.env` out of git; Ok holds notes for the
    /// user, which never carry a value. A value `.env` cannot hold, or a `.env` git tracks,
    /// is refused before anything is written.
    pub(crate) fn write(&self, c: &mut Collection) -> anyhow::Result<Vec<String>> {
        let mut notes = Vec::new();
        if self.planned.is_empty() {
            return Ok(notes);
        }
        ensure!(
            self.planned.iter().all(|(_, v)| dotenv_quotable(v)),
            "the secret cannot be stored in .env: it has a line break or all three quote kinds"
        );
        if git_tracks_dotenv(c.root()) {
            bail!(".env is tracked by git, so nothing was saved; untrack .env first");
        }
        if c.ensure_dotenv_ignored()
            .map_err(|e| anyhow::anyhow!("could not update .gitignore: {e}"))?
        {
            notes.push("added .env to .gitignore".to_owned());
        }
        for (name, value) in &self.planned {
            c.set_dotenv(name, value)
                .map_err(|e| anyhow::anyhow!("could not write {name} to .env: {e}"))?;
            notes.push(format!("stored the value in .env as {name}"));
        }
        Ok(notes)
    }
}

/// The proxy password and every certificate passphrase in `network`.
fn secrets(network: &CollectionNetwork) -> impl Iterator<Item = &str> {
    std::iter::once(proxy_password(&network.proxy))
        .chain(network.certificates.iter().map(|c| c.passphrase.as_str()))
}

/// The proxy password, empty when there is none.
pub(crate) fn proxy_password(proxy: &CollectionProxy) -> &str {
    match proxy {
        CollectionProxy::Custom(p) => p.auth.as_ref().map_or("", |a| a.password.as_str()),
        _ => "",
    }
}

/// Whether `value` holds a `{{...}}` reference rather than a literal secret.
pub(crate) fn is_template(value: &str) -> bool {
    value
        .find("{{")
        .is_some_and(|open| value[open + 2..].contains("}}"))
}

/// A secret as it may be shown: a reference as written, a literal never.
pub(crate) fn masked(value: &str) -> &str {
    if is_template(value) { value } else { "hidden" }
}

/// The `.env` name a value refers to when it is exactly `{{process.env.NAME}}` and NAME is
/// a portable environment variable name.
fn env_name(value: &str) -> Option<&str> {
    let name = value.strip_prefix("{{process.env.")?.strip_suffix("}}")?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let portable = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    portable.then_some(name)
}

/// `base` when it is free, else the first of `base_2`, `base_3`, ... not in `taken`.
fn dotenv_name(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    let mut n = 2;
    loop {
        let name = format!("{base}_{n}");
        if !taken.contains(&name) {
            return name;
        }
        n += 1;
    }
}

/// Whether git tracks `<root>/.env`; any failure to ask reads as untracked. Variables a git
/// hook may set are removed so the question goes to the repository around `root`.
fn git_tracks_dotenv(root: &Path) -> bool {
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
