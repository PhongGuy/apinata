//! Approving, for agent-driven sends, the script texts the app's own editors save.

use std::path::Path;

use apinata_core::{ScriptTrust, script_code, script_hash};
use gpui_kit::App;

use crate::app::AppState;

/// Approves, for the collection at `root`, the hash of `script_code(t)` for every non-empty
/// text in `texts`, in the script trust file `AppState`'s approvals name. Off the UI thread;
/// does nothing without `AppState` or a trust path, and never writes a file that failed to
/// load. Never changes the collection's decision.
pub fn approve_saved(root: &Path, texts: Vec<String>, cx: &mut App) {
    let hashes: Vec<String> = texts
        .iter()
        .map(|t| script_code(t))
        .filter(|code| !code.is_empty())
        .map(script_hash)
        .collect();
    if hashes.is_empty() {
        return;
    }
    let Some(path) = cx
        .try_global::<AppState>()
        .and_then(|s| s.approvals.read(cx).script_trust().map(Path::to_path_buf))
    else {
        return;
    };
    let root = root.to_path_buf();
    cx.background_executor()
        .spawn(async move { record(&path, &root, &hashes) })
        .detach();
}

// Updates under the trust file's lock so entries written elsewhere survive; a store that
// fails to load is never overwritten.
fn record(path: &Path, root: &Path, hashes: &[String]) {
    let _ = ScriptTrust::update(path, |store| store.approve(root, hashes));
}
