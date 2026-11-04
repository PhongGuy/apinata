//! The script gate for MCP sends, decided from the trust file and inbox before anything runs,
//! and the shapes in which script tests and console lines reach the agent.

use std::path::{Path, PathBuf};

use apinata_core::{
    ApprovalInbox, Filing, RequestResult, ScriptDecision, ScriptOptions, ScriptTrust, Verdict,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const NEEDS_APPROVAL: &str = "scripts need approval in the Apinata app; retry after the user \
                              approves. Nothing was sent; set skip_scripts to send without them.";
const BLOCKED: &str = "scripts are turned off for this collection in the Apinata app; nothing \
                       was sent. Set skip_scripts to send without them.";

/// One script test, or one entry standing for a script that failed or could not run.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct TestDoc {
    pub phase: String,
    pub name: String,
    /// `pass`, `fail` or `skipped`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub script_error: bool,
}

/// One line a script logged.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ConsoleDoc {
    pub phase: String,
    pub level: String,
    pub text: String,
}

fn not_filed(why: &str) -> String {
    format!(
        "scripts need approval but the request could not be filed ({why}); the user can run \
         `apinata trust scripts`. Nothing was sent."
    )
}

/// The options a send may run under, or the refusal to return. `texts` are the hashes of the
/// texts the send would evaluate; anything not approved files one inbox entry for `request`.
/// Decided from files only.
pub(crate) fn gate(
    texts: &[String],
    skip: bool,
    root: &Path,
    trust: Option<&Path>,
    inbox: Option<&ApprovalInbox>,
    request: &str,
) -> Result<ScriptOptions, String> {
    if skip {
        return Ok(ScriptOptions::new());
    }
    // An unreadable or missing trust path never runs a script; it is not refiled either.
    let store = match trust.map(ScriptTrust::load) {
        Some(Ok(store)) => store,
        Some(Err(e)) if !texts.is_empty() => {
            return Err(format!(
                "the script trust file could not be read ({e}); nothing was sent"
            ));
        }
        _ => ScriptTrust::empty(PathBuf::new()),
    };
    let approved = store.approved(root);
    // With nothing listed, a blocked collection still runs no script, even one that appears
    // only after interpolation; other collections run under the approved set, failing closed.
    if texts.is_empty() {
        if store.decision(root) == Some(ScriptDecision::Blocked) {
            return Ok(ScriptOptions::new());
        }
        return Ok(ScriptOptions::new().enabled(true).only_approved(approved));
    }
    let missing: Vec<String> = texts
        .iter()
        .filter(|h| !approved.contains(*h))
        .cloned()
        .collect();
    match store.decision(root) {
        Some(ScriptDecision::Blocked) => return Err(BLOCKED.to_owned()),
        Some(ScriptDecision::Allowed) if missing.is_empty() => {
            return Ok(ScriptOptions::new().enabled(true).only_approved(approved));
        }
        _ => {}
    }
    // An undecided collection whose texts are all approved still asks, so the user decides.
    let ask = if missing.is_empty() {
        texts.to_vec()
    } else {
        missing
    };
    match inbox.map(|i| i.request_scripts(&store, root, request, &ask)) {
        Some(Ok(Filing::Denied)) => Err(BLOCKED.to_owned()),
        Some(Ok(_)) => Err(NEEDS_APPROVAL.to_owned()),
        Some(Err(e)) => Err(not_filed(&e.to_string())),
        None => Err(not_filed("no data directory")),
    }
}

/// `r`'s script tests in the tool's shape; a fail carries its detail, a skip its reason.
pub(crate) fn test_docs(r: &RequestResult) -> Vec<TestDoc> {
    r.tests
        .iter()
        .map(|t| {
            let (status, message) = match &t.verdict {
                Verdict::Pass => ("pass", None),
                Verdict::Fail { detail, .. } => ("fail", detail.clone()),
                Verdict::Skipped { reason, .. } => ("skipped", Some(reason.clone())),
                _ => ("skipped", None),
            };
            TestDoc {
                phase: t.phase.as_str().to_owned(),
                name: t.name.clone(),
                status: status.to_owned(),
                message,
                script_error: t.script_error,
            }
        })
        .collect()
}

/// `r`'s console lines in the tool's shape.
pub(crate) fn console_docs(r: &RequestResult) -> Vec<ConsoleDoc> {
    r.redacted_console()
        .into_iter()
        .map(|c| ConsoleDoc {
            phase: c.phase.as_str().to_owned(),
            level: c.level.as_str().to_owned(),
            text: c.text,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_collection_with_nothing_to_approve_keeps_scripts_off() {
        let (root, state) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let path = state.path().join("script-trust.json");
        let mut trust = ScriptTrust::load(&path).unwrap();
        trust.set_decision(root.path(), Some(ScriptDecision::Blocked));
        trust.save().unwrap();
        let got = gate(&[], false, root.path(), Some(&path), None, "r.yml");
        assert_eq!(got, Ok(ScriptOptions::new()));
    }
}
