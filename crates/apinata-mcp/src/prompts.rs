//! MCP prompts: canned requests that name a target and the tools the model should use on it.
//!
//! A prompt carries only the target's path, checked against the loaded collection, never any file
//! contents; the model reads those through the tools, which scrub them.

use apinata_core::{Collection, ItemPath};
use rmcp::model::{GetPromptResult, Prompt, PromptArgument, PromptMessage, Role};
use serde_json::{Map, Value};

const WRITE_TESTS: &str = "write_tests";
const DOCUMENT_FOLDER: &str = "document_folder";
const EXPLAIN_FAILURE: &str = "explain_failure";

/// Tells the model what to do when a send needs the user's approval in the app.
fn approval(tool: &str) -> String {
    format!(
        "Sending to a host that is not local needs the user's approval in the app: if {tool} \
         reports that, tell the user and wait."
    )
}

fn path_argument(description: &str, required: bool) -> Vec<PromptArgument> {
    vec![
        PromptArgument::new("path")
            .with_description(description)
            .with_required(required),
    ]
}

/// The prompts this server offers.
pub(crate) fn list() -> Vec<Prompt> {
    vec![
        Prompt::new(
            WRITE_TESTS,
            Some("Write assertions for one request"),
            Some(path_argument("The request, such as users/get.yml", true)),
        ),
        Prompt::new(
            DOCUMENT_FOLDER,
            Some("Write documentation for a folder and its requests"),
            Some(path_argument(
                "The folder; empty for the whole collection",
                false,
            )),
        ),
        Prompt::new(
            EXPLAIN_FAILURE,
            Some("Find out why a request or folder fails"),
            Some(path_argument("A request or folder", true)),
        ),
    ]
}

/// The prompt `name` for `arguments`, checked against `collection`. `Err` is the message of an
/// invalid-params error: an unknown prompt, a missing or non-string path, or a path that names
/// nothing of the right kind.
pub(crate) fn get(
    collection: &Collection,
    name: &str,
    arguments: Option<&Map<String, Value>>,
) -> Result<GetPromptResult, String> {
    let raw = match arguments.and_then(|a| a.get("path")) {
        None => None,
        Some(Value::String(path)) => Some(path.as_str()),
        Some(_) => return Err("`path` must be a string".to_owned()),
    };
    let text = match name {
        WRITE_TESTS => {
            let path = request_path(collection, raw)?;
            let approval = approval("send_request");
            format!(
                "Write assertions for the request `{path}`. Read it with read_request and the \
                 shared settings of its folder with read_folder, send it with send_request to \
                 see a real response, then add assertions that check the status, the headers \
                 and the body with update_request. Do not change anything else. {approval}"
            )
        }
        DOCUMENT_FOLDER => {
            let path = folder_path(collection, raw.unwrap_or(""))?;
            let target = if path.is_empty() {
                "the whole collection".to_owned()
            } else {
                format!("the folder `{path}`")
            };
            format!(
                "Document {target}. Find its requests with list_items, read the folder with \
                 read_folder and each request with read_request, then write what they do into \
                 the docs of the folder with update_folder and of each request with \
                 update_request. Do not change anything but docs."
            )
        }
        EXPLAIN_FAILURE => {
            let raw = raw.filter(|p| !p.is_empty());
            match request_path(collection, raw) {
                Ok(path) => {
                    let approval = approval("send_request");
                    format!(
                        "Explain why the request `{path}` fails. Read it with read_request, read the \
                     environment it uses with read_environment, and send it with send_request \
                     to see the response. Explain the cause; change nothing unless asked. {approval}"
                    )
                }
                Err(_) => {
                    let path = folder_path(collection, raw.ok_or("`path` is required")?)?;
                    let approval = approval("run_folder");
                    format!(
                        "Explain why requests in the folder `{path}` fail. Read the folder with \
                         read_folder, a failing request with read_request and the environment \
                         with read_environment, and run the folder with run_folder. Explain the \
                         cause; change nothing unless asked. {approval}"
                    )
                }
            }
        }
        other => return Err(format!("no prompt named {other}")),
    };
    Ok(GetPromptResult::new(vec![PromptMessage::new_text(
        Role::User,
        text,
    )]))
}

/// Refuses a path that could break out of the quoted name in the prompt text: a control
/// character or a backtick, which a hostile file name could use to add instructions.
fn plain(raw: &str) -> Result<(), String> {
    if raw.chars().any(|c| c.is_control() || c == '`') {
        return Err("`path` holds a control character or a backtick".to_owned());
    }
    Ok(())
}

fn request_path(collection: &Collection, raw: Option<&str>) -> Result<String, String> {
    let raw = raw.ok_or("`path` is required")?;
    plain(raw)?;
    let known = ItemPath::new(raw).is_some_and(|p| collection.request(&p).is_some());
    if known {
        Ok(raw.to_owned())
    } else {
        Err(format!("no request at {raw}"))
    }
}

/// A folder path, with one trailing slash tolerated and dropped. Empty means the collection root.
fn folder_path(collection: &Collection, raw: &str) -> Result<String, String> {
    plain(raw)?;
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    // A bare "/" or a doubled slash is malformed, not a spelling of the root or a folder.
    if (trimmed.is_empty() && !raw.is_empty()) || trimmed.ends_with('/') {
        return Err(format!("no folder at {raw}"));
    }
    let known =
        ItemPath::new(trimmed).is_some_and(|p| p.is_root() || collection.folder(&p).is_some());
    if known {
        Ok(trimmed.to_owned())
    } else {
        Err(format!("no folder at {raw}"))
    }
}
