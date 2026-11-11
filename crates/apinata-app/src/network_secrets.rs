//! Naming and recognising the collection's network secrets, which live in `.env` and are
//! referenced from the collection file as `{{process.env.NAME}}`.

use crate::command::Edit;
use apinata_core::{Collection, CollectionNetwork, CollectionProxy, dotenv_quotable};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// The base `.env` name of the collection's proxy password.
pub const PROXY_PASSWORD: &str = "APINATA_PROXY_PASSWORD";
/// The refusal when git tracks `.env`.
pub const TRACKED: &str = ".env is tracked by git, so nothing was saved; untrack .env first";
/// The warning when `.env` was added to `.gitignore`.
pub const IGNORED: &str = "Added .env to .gitignore";

/// The refusal when a secret cannot be quoted for `.env`; it names no value.
pub const UNQUOTABLE: &str =
    "the secret cannot be stored in .env: it has a line break or all three quote kinds";

/// Applies a network edit to the collection draft; Ok holds the moved secrets as (name,
/// value) for `.env`, which the caller writes when the collection is saved. `held` names are
/// already promised to other unsaved secrets. On Err the draft is unchanged and nothing is
/// written. Other edits are a bug here.
pub fn apply(
    c: &mut Collection,
    edit: Edit,
    held: &BTreeSet<String>,
) -> Result<Vec<(String, String)>, String> {
    if let Edit::HostOverrides(rows) = &edit {
        c.file_mut().set_host_overrides(rows);
        // A bad row is shown on the row itself, so the edit carries no warning.
        return Ok(Vec::new());
    }
    let current = c.file().network();
    let mut taken = c
        .dotenv_keys()
        .map_err(|e| format!("Could not read .env: {e}"))?;
    taken.extend(held.iter().cloned());
    let all = matches!(edit, Edit::MoveNetworkSecrets);
    let mut moves = Moves::new(literals(&current), all, taken, BTreeSet::new());

    // Plan every move against the edited values first, so a refusal writes nothing.
    let (mut proxy, mut certificates) = match edit {
        Edit::Proxy(proxy) => (Some(proxy), None),
        Edit::ClientCertificates(certificates) => (None, Some(certificates)),
        Edit::MoveNetworkSecrets => (
            Some(current.proxy.clone()),
            Some(current.certificates.clone()),
        ),
        _ => unreachable!("only network edits reach network_secrets::apply"),
    };
    // The values the file will hold after the edit: the edited part, the rest unchanged.
    let kept_proxy = proxy.as_ref().unwrap_or(&current.proxy);
    let kept_certs = certificates.as_deref().unwrap_or(&current.certificates);
    moves.referenced = std::iter::once(proxy_password(kept_proxy))
        .chain(kept_certs.iter().map(|c| c.passphrase.as_str()))
        .filter_map(env_name)
        .map(str::to_owned)
        .collect();
    moves.taken.extend(moves.referenced.iter().cloned());
    if let Some(CollectionProxy::Custom(p)) = &mut proxy
        && let Some(auth) = &mut p.auth
    {
        moves.plan(
            &mut auth.password,
            proxy_password(&current.proxy),
            PROXY_PASSWORD,
        );
    }
    for (row, cert) in certificates.iter_mut().flatten().enumerate() {
        let previous = current
            .certificates
            .get(row)
            .map_or("", |c| c.passphrase.as_str());
        moves.plan(&mut cert.passphrase, previous, &passphrase_base(row));
    }

    // Only a move touches `.env`; a "Move to .env" with nothing to move leaves the draft alone.
    if moves.planned.is_empty() && moves.all {
        return Ok(Vec::new());
    }
    check(c, &moves.planned)?;
    if let Some(proxy) = &proxy {
        c.file_mut().set_proxy(proxy);
    }
    if let Some(certificates) = &certificates {
        c.file_mut().set_client_certificates(certificates);
    }
    Ok(moves.planned)
}

/// Refuses `planned` moves that `.env` could not take: a value that cannot be quoted, or a
/// `.env` that git tracks. Nothing planned passes.
pub(crate) fn check(c: &Collection, planned: &[(String, String)]) -> Result<(), String> {
    if planned.is_empty() {
        return Ok(());
    }
    if !planned.iter().all(|(_, v)| dotenv_quotable(v)) {
        return Err(UNQUOTABLE.to_owned());
    }
    if git_tracks_dotenv(c.root()) {
        return Err(TRACKED.to_owned());
    }
    Ok(())
}

/// Writes `planned` to `.env` after `check`, keeping `.env` out of git; Ok holds warnings.
/// Nothing planned writes nothing; only an I/O failure partway may leave earlier writes.
pub(crate) fn write(
    c: &mut Collection,
    planned: &[(String, String)],
) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    if planned.is_empty() {
        return Ok(warnings);
    }
    check(c, planned)?;
    let ignored = c
        .ensure_dotenv_ignored()
        .map_err(|e| format!("Could not update .gitignore: {e}"))?;
    if ignored {
        warnings.push(IGNORED.to_owned());
    }
    for (name, value) in planned {
        c.set_dotenv(name, value)
            .map_err(|e| format!("Could not write {name} to .env: {e}"))?;
    }
    Ok(warnings)
}

/// The secrets an edit moves to `.env`, as (name, value), with the names already in use.
pub(crate) struct Moves {
    /// Literal secrets already in the collection file, which stay unless `all`.
    known: BTreeSet<String>,
    all: bool,
    taken: BTreeSet<String>,
    /// Names the edited values still refer to, which a moved value must not take over.
    referenced: BTreeSet<String>,
    pub(crate) planned: Vec<(String, String)>,
}

impl Moves {
    /// No moves yet. `known` literals stay unless `all`; `taken` names are never chosen, and
    /// `referenced` names, which edited values still use, are never reused for a field.
    pub(crate) fn new(
        known: BTreeSet<String>,
        all: bool,
        taken: BTreeSet<String>,
        referenced: BTreeSet<String>,
    ) -> Moves {
        Moves {
            known,
            all,
            taken,
            referenced,
            planned: Vec::new(),
        }
    }

    /// Replace a literal `value` with a `.env` reference when it moves: under the name the
    /// field's `previous` value referred to while no edited value still does, else the first
    /// free name from `base`.
    pub(crate) fn plan(&mut self, value: &mut String, previous: &str, base: &str) {
        let literal = !value.is_empty() && !is_template(value);
        if !literal || (!self.all && self.known.contains(value.as_str())) {
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
}

/// The literal passwords and passphrases in `network`.
fn literals(network: &CollectionNetwork) -> BTreeSet<String> {
    let proxy = std::iter::once(proxy_password(&network.proxy));
    let passphrases = network.certificates.iter().map(|c| c.passphrase.as_str());
    proxy
        .chain(passphrases)
        .filter(|s| !s.is_empty() && !is_template(s))
        .map(str::to_owned)
        .collect()
}

/// The proxy password, empty when there is none.
fn proxy_password(proxy: &CollectionProxy) -> &str {
    match proxy {
        CollectionProxy::Custom(p) => p.auth.as_ref().map_or("", |a| a.password.as_str()),
        _ => "",
    }
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
pub fn env_name(value: &str) -> Option<&str> {
    let name = value.strip_prefix("{{process.env.")?.strip_suffix("}}")?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let portable = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    portable.then_some(name)
}

/// The `.env` names the secrets of `target`'s draft refer to: its OAuth 2.0 client secret
/// and password, and for the collection also the proxy password and certificate passphrases.
pub(crate) fn referenced_names(
    c: &Collection,
    target: &crate::command::Target,
) -> BTreeSet<String> {
    let mut values = Vec::new();
    if let Some(apinata_core::AuthSpec::OAuth2(o)) = crate::auth_secrets::current(c, target) {
        values.extend([o.client_secret, o.password]);
    }
    if matches!(target, crate::command::Target::Collection) {
        let network = c.file().network();
        values.push(proxy_password(&network.proxy).to_owned());
        values.extend(network.certificates.into_iter().map(|c| c.passphrase));
    }
    values
        .iter()
        .filter_map(|v| env_name(v))
        .map(str::to_owned)
        .collect()
}

/// The base `.env` name for the passphrase of certificate row `row`, counted from zero.
pub fn passphrase_base(row: usize) -> String {
    format!("APINATA_CERT_PASSPHRASE_{}", row + 1)
}

/// `base` when it is free, else the first of `base_2`, `base_3`, ... not in `taken`.
// The candidate sequence never ends, so `find` returns only once a name is free.
#[allow(clippy::expect_used)]
pub fn dotenv_name(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !taken.contains(name))
        .expect("an unbounded range finds a free name")
}

/// Whether the network settings hold a literal secret: a non-empty proxy password or
/// passphrase that is not a template.
pub fn plain_secrets(network: &CollectionNetwork) -> bool {
    let plain = |s: &str| !s.is_empty() && !is_template(s);
    let proxy = match &network.proxy {
        CollectionProxy::Custom(p) => p.auth.as_ref().is_some_and(|a| plain(&a.password)),
        _ => false,
    };
    proxy || network.certificates.iter().any(|c| plain(&c.passphrase))
}

/// Whether git tracks `<root>/.env`. The variables a git hook may set are removed so the
/// check always asks the repository around `root`; any failure to ask reads as untracked.
pub fn git_tracks_dotenv(root: &Path) -> bool {
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
