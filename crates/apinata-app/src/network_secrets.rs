//! Naming and recognising the collection's network secrets, which live in `.env` and are
//! referenced from the collection file as `{{process.env.NAME}}`.

use crate::command::Edit;
use apinata_core::{
    Collection, CollectionNetwork, CollectionProxy, DotenvMoves as Moves, dotenv_quotable,
};
use std::collections::BTreeSet;

// The naming rule lives in core, so the CLI names a moved secret exactly as the app does.
pub use apinata_core::{
    PROXY_PASSWORD_DOTENV as PROXY_PASSWORD, dotenv_reference_name as env_name,
    dotenv_tracked_by_git as git_tracks_dotenv, free_dotenv_name as dotenv_name, is_template,
    passphrase_dotenv_base as passphrase_base,
};
/// The refusal when git tracks `.env`.
pub const TRACKED: &str =
    ".env is tracked by git, so nothing was saved; untrack it with git rm --cached .env first";
/// The warning when `.env` was added to `.gitignore`.
pub const IGNORED: &str = "Added .env to .gitignore";

/// The notice when a collection opens with a `.env` no git rule ignored.
pub const IGNORED_OPEN: &str = "Added .env to .gitignore, since it holds secret values";

/// The notice when a collection opens with a `.env` git tracks.
pub const TRACKED_OPEN: &str = ".env is tracked by git, so its secret values are committed; \
     untrack it with git rm --cached .env and commit";

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
    let referenced: BTreeSet<String> = std::iter::once(proxy_password(kept_proxy))
        .chain(kept_certs.iter().map(|c| c.passphrase.as_str()))
        .filter_map(env_name)
        .map(str::to_owned)
        .collect();
    taken.extend(referenced.iter().cloned());
    let mut moves = Moves::new(literals(&current), all, taken, referenced);
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
    if moves.planned().is_empty() && all {
        return Ok(Vec::new());
    }
    check(c, moves.planned())?;
    if let Some(proxy) = &proxy {
        c.file_mut().set_proxy(proxy);
    }
    if let Some(certificates) = &certificates {
        c.file_mut().set_client_certificates(certificates);
    }
    Ok(moves.into_planned())
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

/// The `.env` names the secrets of `target`'s draft refer to: its auth credentials, and for
/// the collection also the proxy password and certificate passphrases.
pub(crate) fn referenced_names(
    c: &Collection,
    target: &crate::command::Target,
) -> BTreeSet<String> {
    let mut values = Vec::new();
    if let Some(auth) = crate::auth_secrets::current(c, target) {
        let fields = apinata_core::import::auth_secret_fields(&auth);
        values.extend(fields.into_iter().map(|(_, text)| text));
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
