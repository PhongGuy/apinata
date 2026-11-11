//! Moving a typed OAuth 2.0 client secret or resource-owner password to the collection's
//! `.env`, referenced from the request, folder or collection file as `{{process.env.NAME}}`.
//! The draft takes the reference at once; `.env` is written only when the target is saved.

use crate::command::Target;
use crate::network_secrets::{self, Moves, env_name, is_template};
use apinata_core::{AuthSpec, Collection};
use std::collections::BTreeSet;

/// The base `.env` name of an OAuth 2.0 client secret.
pub const CLIENT_SECRET: &str = "APINATA_OAUTH2_CLIENT_SECRET";
/// The base `.env` name of an OAuth 2.0 resource-owner password.
pub const PASSWORD: &str = "APINATA_OAUTH2_PASSWORD";

/// Rewrites a new literal secret in `auth` to a `.env` reference; Ok holds the moved secrets
/// as (name, value), which the caller writes to `.env` when the target is saved, Err a
/// message. `held` names are promised to other unsaved secrets. On Err `auth` is unchanged.
/// A literal the target's file already holds stays unless `all`. Other auth modes pass through.
pub fn apply(
    c: &Collection,
    target: &Target,
    auth: &mut AuthSpec,
    all: bool,
    held: &BTreeSet<String>,
) -> Result<Vec<(String, String)>, String> {
    let AuthSpec::OAuth2(edited) = auth else {
        return Ok(Vec::new());
    };
    let (previous_secret, previous_password) = match current(c, target) {
        Some(AuthSpec::OAuth2(o)) => (o.client_secret, o.password),
        _ => (String::new(), String::new()),
    };
    let known = [&previous_secret, &previous_password]
        .into_iter()
        .filter(|s| !s.is_empty() && !is_template(s))
        .cloned()
        .collect();
    let referenced: BTreeSet<String> = [&edited.client_secret, &edited.password]
        .into_iter()
        .filter_map(|s| env_name(s))
        .map(str::to_owned)
        .collect();
    let mut taken = c
        .dotenv_keys()
        .map_err(|e| format!("Could not read .env: {e}"))?;
    taken.extend(referenced.iter().cloned());
    taken.extend(held.iter().cloned());

    // Plan on a copy, so a refusal leaves the caller's edit as it was.
    let mut planned = edited.clone();
    let mut moves = Moves::new(known, all, taken, referenced);
    moves.plan(&mut planned.client_secret, &previous_secret, CLIENT_SECRET);
    moves.plan(&mut planned.password, &previous_password, PASSWORD);
    network_secrets::check(c, &moves.planned)?;
    *edited = planned;
    Ok(moves.planned)
}

/// The auth the target's draft holds now, or None for a target that is gone.
pub(crate) fn current(c: &Collection, target: &Target) -> Option<AuthSpec> {
    match target {
        Target::Request(path) => c.request(path).map(|r| r.auth()),
        Target::Folder(path) => c.folder(path)?.file().map(|f| f.auth()),
        Target::Collection => Some(c.file().auth()),
        Target::Environment(_) => None,
    }
}
