//! The one-line stderr notice when the network settings a send uses differ from what the last
//! run saw: an edited collection, or a folder, request or run variable that sends a request
//! elsewhere. Core judges each send and asks; the command line answers by printing the line and
//! going ahead, so a run never blocks.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use apinata_core::{NetworkAck, NetworkAnswer, NetworkCheck, NetworkQuestion, QuestionKind};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Records the global `--quiet` flag.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// The CLI's own record, apart from the app's, so each notices a change independently.
/// Debug builds honour `APINATA_TEST_NETWORK_ACK` so tests never touch the data directory.
fn ack_file() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("APINATA_TEST_NETWORK_ACK") {
        return Some(PathBuf::from(path));
    }
    NetworkAck::default_cli_path()
}

/// The line to print for `question`.
fn line(question: &NetworkQuestion) -> String {
    match question.kind() {
        QuestionKind::FirstSeen => format!(
            "note: this collection sets network options: {}",
            question.summary()
        ),
        _ => format!(
            "warning: collection network settings changed since the last run: {}",
            question.summary()
        ),
    }
}

/// The check for a run's executor: each question prints its line once per process unless
/// `--quiet`, and the send goes ahead with what it showed recorded as seen. `None` without a
/// data directory.
pub fn check() -> Option<NetworkCheck> {
    static PRINTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);
    let file = ack_file()?;
    Some(NetworkCheck::new(file, |question: NetworkQuestion| {
        if !QUIET.load(Ordering::Relaxed) {
            let line = line(&question);
            let mut printed = PRINTED.lock().unwrap_or_else(PoisonError::into_inner);
            if printed.insert(line.clone()) {
                eprintln!("{line}");
            }
        }
        async { NetworkAnswer::Send }
    }))
}
