//! `save_example`: the last response `send_request` got for a request, scrubbed of every
//! secret known at send time, appended to the request file's examples.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use apinata_core::export::{har, sent_exchange};
use apinata_core::{Collection, ExampleResponse, ItemPath, NamedSecret, Response, TreeError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one completed send left to save: the response as received and the secrets the request
/// knew then, or why they could not be found.
pub(crate) struct LastSend {
    pub response: Response,
    pub secrets: Result<Vec<NamedSecret>, String>,
    /// When the send ended, which a HAR export dates the exchange by.
    pub ended: SystemTime,
}

/// The last completed send per request, shared by every clone of the server.
pub(crate) type LastSends = Arc<Mutex<HashMap<ItemPath, LastSend>>>;

/// `save_example`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveExampleArgs {
    /// Collection-relative path of the request, such as `users/get.yml`.
    pub path: String,
    /// The example's name; it must not be blank or taken by another example of the request.
    pub name: String,
    /// An optional description.
    pub description: Option<String>,
}

/// What `save_example` reports.
#[derive(Serialize, JsonSchema)]
pub(crate) struct SavedExample {
    pub path: String,
    pub name: String,
    /// The example's position in the request's `examples`.
    pub index: usize,
    /// True when a binary body held a secret and was left out; the description says so.
    pub body_withheld: bool,
}

/// The scrubbed example for `args.path`'s last send, or why there is none to save.
pub(crate) fn prepare(
    sends: &LastSends,
    args: &SaveExampleArgs,
) -> Result<(ItemPath, ExampleResponse, String, bool), String> {
    let path =
        ItemPath::new(&args.path).ok_or_else(|| format!("invalid item path: {}", args.path))?;
    let sends = sends.lock().unwrap_or_else(|e| e.into_inner());
    let last = sends.get(&path).ok_or_else(|| {
        format!(
            "no response for {} this session; send the request first with send_request",
            args.path
        )
    })?;
    if !ExampleResponse::savable(&last.response) {
        return Err(format!(
            "the last response of {} was cut short or is 5 MiB or more, so it cannot be saved",
            args.path
        ));
    }
    let secrets = last.secrets.as_deref().map_err(|reason| {
        format!("could not check the response for secret values: {reason}; nothing was written")
    })?;
    let mut example = ExampleResponse::from_response(&last.response);
    let scrubbed = example.scrub(Some(secrets));
    let description = scrubbed.description(args.description.as_deref().unwrap_or("").trim());
    Ok((path, example, description, scrubbed.withheld))
}

/// Append `example` to the request at `path` under `name`, trimmed and checked by core,
/// refusing a blank name or one already taken. Returns the index and the name as saved.
pub(crate) fn save(
    c: &mut Collection,
    path: &ItemPath,
    name: &str,
    description: &str,
    example: &ExampleResponse,
) -> Result<(usize, String), String> {
    let name = c.new_example_name(path, name).map_err(|e| match e {
        TreeError::NotFound { .. } => format!("no HTTP request at {path}"),
        TreeError::InvalidName { .. } => "the example name must not be blank".to_owned(),
        other => other.to_string(),
    })?;
    let index = c
        .add_example(path, &name, description, example)
        .map_err(|e| e.to_string())?;
    Ok((index, name))
}

/// `export_har`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportHarArgs {
    /// Collection-relative path of the request, such as `users/get.yml`.
    pub path: String,
}

/// What `export_har` reports.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ExportedHar {
    pub path: String,
    /// The HAR 1.2 document, one entry, as the app's export writes it.
    pub har: String,
}

/// `args.path`'s last send as HAR, written by core's one path for a sent exchange, or why
/// there is none to write.
pub(crate) fn export_har(sends: &LastSends, args: ExportHarArgs) -> Result<ExportedHar, String> {
    let path =
        ItemPath::new(&args.path).ok_or_else(|| format!("invalid item path: {}", args.path))?;
    let sends = sends.lock().unwrap_or_else(|e| e.into_inner());
    let last = sends.get(&path).ok_or_else(|| {
        format!(
            "no response for {} this session; send the request first with send_request",
            args.path
        )
    })?;
    let secrets = last.secrets.as_deref().map_err(|reason| {
        format!("could not check the exchange for secret values: {reason}; nothing was exported")
    })?;
    let exchange = sent_exchange(&last.response, last.ended, secrets)
        .map_err(|e| format!("{e}; nothing was exported"))?;
    Ok(ExportedHar {
        path: args.path,
        har: har(&exchange),
    })
}

/// `scrub_example_userinfo`'s arguments.
#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScrubArgs {
    /// Save the scrubbed examples; without it the call only lists them.
    #[serde(default)]
    pub write: bool,
}

/// What `scrub_example_userinfo` found, or with `write` changed.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ScrubbedUserinfo {
    /// Whether any request file was saved.
    pub written: bool,
    /// Each saved example whose URL held a username or password.
    pub changes: Vec<ScrubChange>,
}

/// One saved example whose URL held userinfo.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ScrubChange {
    /// Collection-relative path of the request file.
    pub item: String,
    /// The example's name.
    pub example: String,
    /// The URL without its userinfo.
    pub url: String,
}

/// Scrub every saved example's URL userinfo through core, saving only with `write`.
pub(crate) fn scrub(c: &mut Collection, write: bool) -> Result<ScrubbedUserinfo, String> {
    let report = apinata_core::scrub_example_userinfo(c, write).map_err(|e| e.to_string())?;
    Ok(ScrubbedUserinfo {
        written: report.written(),
        changes: report
            .changes()
            .iter()
            .map(|ch| ScrubChange {
                item: ch.item().to_string(),
                example: ch.example().to_owned(),
                url: ch.url().to_owned(),
            })
            .collect(),
    })
}
