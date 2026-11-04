//! The read tools' bodies: core reads mapped to documents, with errors that name the path.

use apinata_core::{Collection, ItemPath};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::dto::{self, ConfigDoc, EnvironmentDoc, RequestDoc};

/// Arguments naming one item. Unknown fields are refused, since `delete_item` takes these too and
/// an invented flag such as `recursive: false` must not be silently ignored.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathArgs {
    /// Collection-relative path, such as `users/get.yml`.
    pub path: String,
}

/// Arguments naming one environment; unknown fields are refused, as for [`PathArgs`].
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameArgs {
    /// The environment's name, such as `dev`.
    pub name: String,
}

/// The HTTP or GraphQL request file at `raw` as a document. Another opaque item is refused by
/// kind, so the agent learns which tools can still act on it.
pub(crate) fn request(c: &Collection, raw: &str) -> Result<RequestDoc, String> {
    let path = ItemPath::new(raw).ok_or_else(|| format!("invalid item path: {raw}"))?;
    if let Some(request) = c.request(&path) {
        return dto::request_doc(request);
    }
    match c.opaque_item(&path) {
        Some(other) => Err(format!(
            "{raw} is a {} request, which MCP tools can list, move, rename and delete only",
            other.kind()
        )),
        None => Err(format!("no request at {raw}")),
    }
}

/// The shared sections of the collection (`raw` empty) or of the folder at `raw`. A folder
/// without `folder.yml` reads as empty; one whose `folder.yml` failed to load reports that error.
pub(crate) fn folder(c: &Collection, raw: &str) -> Result<ConfigDoc, String> {
    let path = ItemPath::new(raw).ok_or_else(|| format!("invalid item path: {raw}"))?;
    if path.is_root() {
        let f = c.file();
        return dto::config_doc(f.headers(), f.auth(), f.vars(), f.docs());
    }
    let folder = c
        .folder(&path)
        .ok_or_else(|| format!("no folder at {raw}"))?;
    if let Some(f) = folder.file() {
        return dto::config_doc(f.headers(), f.auth(), f.vars(), f.docs());
    }
    // Core keeps no broken flag on a folder, so a load error for its file is the only signal.
    let file = folder.path().join("folder.yml");
    match c.load_errors().iter().find(|e| e.path() == file) {
        Some(error) => Err(error.to_string()),
        None => Ok(ConfigDoc::default()),
    }
}

/// The variables of the environment `name`, secrets without their values.
pub(crate) fn environment(c: &Collection, name: &str) -> Result<EnvironmentDoc, String> {
    let env = c
        .environment(name)
        .ok_or_else(|| c.environment_not_found(name).to_string())?;
    Ok(EnvironmentDoc {
        name: env.name().to_owned(),
        variables: env
            .variables()
            .iter()
            .map(dto::env_var)
            .collect::<Result<_, _>>()?,
    })
}
