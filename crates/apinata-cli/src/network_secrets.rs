//! Moving the proxy password and certificate passphrases into the collection's `.env`, with
//! names and planning from core, so a secret lands under the same name the app gives it.

use std::collections::BTreeSet;

use anyhow::{bail, ensure};
use apinata_core::{
    Collection, CollectionNetwork, CollectionProxy, DotenvMoves, dotenv_quotable,
    dotenv_reference_name, dotenv_tracked_by_git, is_template,
};

/// An empty plan for an edit from `before` to `after`. As in the app, literals `before`
/// already holds stay unless `all`, and names `after` still refers to are not reused.
pub(crate) fn moves(
    c: &Collection,
    before: &CollectionNetwork,
    after: &CollectionNetwork,
    all: bool,
) -> anyhow::Result<DotenvMoves> {
    let known = secrets(before)
        .filter(|s| !s.is_empty() && !is_template(s))
        .map(str::to_owned)
        .collect();
    let referenced: BTreeSet<String> = secrets(after)
        .filter_map(dotenv_reference_name)
        .map(str::to_owned)
        .collect();
    let mut taken = c
        .dotenv_keys()
        .map_err(|e| anyhow::anyhow!("could not read .env: {e}"))?;
    taken.extend(referenced.iter().cloned());
    Ok(DotenvMoves::new(known, all, taken, referenced))
}

/// Write the planned values to `.env`, keeping `.env` out of git; Ok holds notes that never
/// carry a value. A value `.env` cannot hold, or a `.env` git tracks, is refused first.
pub(crate) fn write(c: &mut Collection, moves: &DotenvMoves) -> anyhow::Result<Vec<String>> {
    let planned = moves.planned();
    let mut notes = Vec::new();
    if planned.is_empty() {
        return Ok(notes);
    }
    ensure!(
        planned.iter().all(|(_, v)| dotenv_quotable(v)),
        "the secret cannot be stored in .env: it has a line break or all three quote kinds"
    );
    if dotenv_tracked_by_git(c.root()) {
        bail!(".env is tracked by git, so nothing was saved; untrack .env first");
    }
    if c.ensure_dotenv_ignored()
        .map_err(|e| anyhow::anyhow!("could not update .gitignore: {e}"))?
    {
        notes.push("added .env to .gitignore".to_owned());
    }
    for (name, value) in planned {
        c.set_dotenv(name, value)
            .map_err(|e| anyhow::anyhow!("could not write {name} to .env: {e}"))?;
        notes.push(format!("stored the value in .env as {name}"));
    }
    Ok(notes)
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

/// A secret as it may be shown: a reference as written, a literal never.
pub(crate) fn masked(value: &str) -> &str {
    if is_template(value) { value } else { "hidden" }
}
