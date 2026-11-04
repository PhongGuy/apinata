//! The one-line stderr warning for a request whose URL query and params block disagree. The
//! URL's query is what goes out, so the run goes ahead; each request is named once per process.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use apinata_core::{ItemPath, RunPlan};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Records the global `--quiet` flag.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// The warning line for each planned request whose two sides disagree, by item path.
pub fn lines(plan: &RunPlan) -> HashMap<ItemPath, String> {
    plan.items()
        .zip(plan.requests())
        .filter_map(|(item, request)| {
            let names = request.query_mismatch()?.names.join(", ");
            let line = format!(
                "warning: {item}: URL query and params disagree on {names}; the URL's query is sent"
            );
            Some((item.clone(), line))
        })
        .collect()
}

/// Prints `item`'s line from `lines` as the request starts, unless `--quiet` or printed before.
pub fn starting(lines: &HashMap<ItemPath, String>, item: &ItemPath) {
    static PRINTED: LazyLock<Mutex<HashSet<ItemPath>>> = LazyLock::new(Mutex::default);
    let Some(line) = lines.get(item) else {
        return;
    };
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    let mut printed = PRINTED.lock().unwrap_or_else(PoisonError::into_inner);
    if printed.insert(item.clone()) {
        eprintln!("{line}");
    }
}
