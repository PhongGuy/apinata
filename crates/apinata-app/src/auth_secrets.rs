//! Moving a typed OAuth 2.0 client secret or resource-owner password to the collection's
//! `.env`, referenced from the request, folder or collection file as `{{process.env.NAME}}`.

use crate::command::Target;
use crate::network_secrets::{self, Moves, env_name, is_template};
use apinata_core::{AuthSpec, Collection};
use std::collections::BTreeSet;

/// The base `.env` name of an OAuth 2.0 client secret.
pub const CLIENT_SECRET: &str = "APINATA_OAUTH2_CLIENT_SECRET";
/// The base `.env` name of an OAuth 2.0 resource-owner password.
pub const PASSWORD: &str = "APINATA_OAUTH2_PASSWORD";

/// Rewrites a new literal secret in `auth` to a `.env` reference and writes it to `.env`; Ok
/// holds warnings, Err a message. On Err nothing is written and `auth` is unchanged. A literal
/// the target's file already holds stays unless `all`. Other auth modes pass through.
pub fn apply(
    c: &mut Collection,
    target: &Target,
    auth: &mut AuthSpec,
    all: bool,
) -> Result<Vec<String>, String> {
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

    // Plan on a copy, so a refusal leaves the caller's edit as it was.
    let mut planned = edited.clone();
    let mut moves = Moves::new(known, all, taken, referenced);
    moves.plan(&mut planned.client_secret, &previous_secret, CLIENT_SECRET);
    moves.plan(&mut planned.password, &previous_password, PASSWORD);
    let warnings = network_secrets::commit(c, &moves)?;
    *edited = planned;
    Ok(warnings)
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
