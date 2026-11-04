//! `import_request`: parse a command in a supported request format and write it as a new
//! request through core, so the file matches what the CLI and the app write.

use apinata_core::Collection;
use apinata_core::SecretStore;
use apinata_core::import::{RequestFormat, add_request, parse_request, store_request_secrets};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::writes::item_path;

/// `import_request`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportRequestArgs {
    /// The text to import, such as a curl command copied from a browser.
    pub text: String,
    /// `auto` (the default) detects the format; otherwise one of `curl`.
    #[serde(default)]
    pub format: Option<String>,
    /// The folder to create the request in; empty or absent for the collection root.
    #[serde(default)]
    pub parent: String,
    /// The display name; the file name is derived from it.
    pub name: String,
    /// The environment that receives a secret the command carries, such as the key of
    /// `--aws-sigv4` with `--user`; absent to use the collection's only environment. A named
    /// environment that does not exist stores nothing.
    #[serde(default)]
    pub environment: Option<String>,
}

/// The request `import_request` created and what did not map onto it.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ImportedRequestResult {
    /// The new request's collection-relative path.
    pub path: String,
    /// One note per part of the input that was dropped, converted or needs input.
    pub notes: Vec<NoteDoc>,
}

/// One import note.
#[derive(Serialize, JsonSchema)]
pub(crate) struct NoteDoc {
    /// Where in the input the note comes from, such as `curl flag -k`.
    pub source: String,
    /// `dropped`, `converted`, `renamed` or `needs-input`.
    pub kind: String,
    /// What happened to it.
    pub detail: String,
}

/// Parse `a.text` and create it as a request; nothing is sent and no file it names is read.
/// Not retried: `add_request` creates a fresh file and deletes it again when it fails.
pub(crate) fn import_request(
    c: &mut Collection,
    a: &ImportRequestArgs,
    store: &dyn SecretStore,
) -> Result<ImportedRequestResult, String> {
    let format = match a.format.as_deref() {
        None | Some("auto") => None,
        Some(id) => Some(RequestFormat::from_id(id).ok_or_else(|| unknown_format(id))?),
    };
    let parent = item_path(&a.parent)?;
    let mut parsed = parse_request(format, &a.text).map_err(|e| e.to_string())?;
    let path = add_request(c, &parent, &a.name, &parsed.request).map_err(|e| e.to_string())?;
    // A secret the command carries goes to the named environment (nothing is stored if it does
    // not exist), else to the collection's only one, and to the store; otherwise a note says
    // why it was not kept.
    store_request_secrets(
        c,
        &parsed.request,
        a.environment.as_deref(),
        store,
        &mut parsed.notes,
    );
    Ok(ImportedRequestResult {
        path: path.to_string(),
        notes: parsed
            .notes
            .into_iter()
            .map(|n| NoteDoc {
                source: n.source,
                kind: n.kind.id().to_owned(),
                detail: n.detail,
            })
            .collect(),
    })
}

fn unknown_format(id: &str) -> String {
    let ids: Vec<&str> = std::iter::once("auto")
        .chain(RequestFormat::ALL.iter().map(|f| f.id()))
        .collect();
    format!("unknown format {id}; expected {}", ids.join(", "))
}
