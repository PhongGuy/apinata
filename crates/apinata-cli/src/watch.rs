//! `apinata run --watch`: run, then rerun on every relevant collection edit until Ctrl+C.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use apinata_core::Collection;
use apinata_core::watch::{Change, CollectionWatcher};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::{EXIT_USAGE, RunArgs, run, run_once};

/// Quiet time awaited after a change, on top of the watcher's own debounce.
const QUIET: Duration = Duration::from_millis(300);
/// The exit code when interrupted before any run finished.
const EXIT_INTERRUPTED: u8 = 130;

/// SIGINT, registered before the first run so an early signal is not lost.
struct Interrupt {
    #[cfg(unix)]
    signal: Option<tokio::signal::unix::Signal>,
}

impl Interrupt {
    fn new() -> Interrupt {
        Interrupt {
            #[cfg(unix)]
            signal: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok(),
        }
    }

    async fn recv(&mut self) {
        #[cfg(unix)]
        if let Some(signal) = self.signal.as_mut() {
            signal.recv().await;
            return;
        }
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The collection root around the run's path, opened to learn which directories to watch.
fn open_collection(args: &RunArgs) -> anyhow::Result<Collection> {
    let path = std::fs::canonicalize(args.target())?;
    let root = run::find_root(&path)?;
    Ok(Collection::open(root)?)
}

fn dirs_of(collection: &Collection) -> BTreeSet<PathBuf> {
    collection.watch_dirs().into_iter().collect()
}

/// Files this command writes itself, whose change events must not trigger a rerun.
fn outputs(args: &RunArgs) -> Vec<PathBuf> {
    [
        &args.reporter_json,
        &args.reporter_junit,
        &args.reporter_html,
        &args.output,
        &args.cookie_jar,
    ]
    .into_iter()
    .flatten()
    .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| lexical_abs(p)))
    .collect()
}

/// Absolute without resolving links, for outputs that do not exist yet. The parent is
/// canonicalised so it matches what the watcher reports.
fn lexical_abs(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    match (abs.parent().map(std::fs::canonicalize), abs.file_name()) {
        (Some(Ok(parent)), Some(name)) => parent.join(name),
        _ => abs,
    }
}

/// Whether `path` is the output `out`, something inside it, or a `.tmp*` sibling that an
/// atomic write (the cookie jar's) creates beside it before renaming.
fn is_output_file(path: &Path, out: &Path) -> bool {
    let temp_sibling = path.parent() == out.parent()
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".tmp"));
    path == out || path.starts_with(out) || temp_sibling
}

/// Whether a change should cause a rerun.
fn relevant(change: &Change, collection: &Collection, outputs: &[PathBuf]) -> bool {
    match change {
        Change::Paths(paths) => paths
            .iter()
            .any(|p| !collection.is_ignored(p) && !outputs.iter().any(|o| is_output_file(p, o))),
        _ => true,
    }
}

/// Wait for a relevant change and the quiet window after it. Err carries the watcher's failure.
async fn next_change(
    changes: &mut UnboundedReceiver<Change>,
    collection: &Collection,
    outputs: &[PathBuf],
) -> Result<(), String> {
    loop {
        match changes.recv().await {
            Some(Change::Failed(m)) => return Err(m),
            Some(c) if relevant(&c, collection, outputs) => break,
            Some(_) => {}
            None => return Err("the file watcher stopped".to_owned()),
        }
    }
    // Further edits inside the window join this rerun.
    while let Ok(next) = tokio::time::timeout(QUIET, changes.recv()).await {
        match next {
            Some(Change::Failed(m)) => return Err(m),
            Some(_) => {}
            None => return Err("the file watcher stopped".to_owned()),
        }
    }
    Ok(())
}

fn announce_rerun() {
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        out!("\x1b[2J\x1b[H");
    }
    outln!(
        "--- rerun at {} ---",
        chrono::Local::now().format("%H:%M:%S")
    );
}

/// Run once, then again after each relevant change; returns the exit code for the process.
pub async fn run(args: &RunArgs) -> u8 {
    let mut collection = match open_collection(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            return EXIT_USAGE;
        }
    };
    let (tx, mut changes) = unbounded_channel();
    // The watcher runs on its own thread; hand its batches to this task.
    let watcher = match CollectionWatcher::start(
        collection.root().to_path_buf(),
        dirs_of(&collection),
        move |c| {
            let _ = tx.send(c);
        },
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error: cannot watch the collection: {e}");
            return EXIT_USAGE;
        }
    };
    let mut interrupt = Interrupt::new();
    // Placing the watches first reports every existing entry once. Wait for that baseline so
    // edits made during the first run are caught and the baseline starts no rerun.
    let baseline = tokio::select! {
        b = tokio::time::timeout(Duration::from_secs(10), changes.recv()) => b,
        () = interrupt.recv() => return EXIT_INTERRUPTED,
    };
    match baseline {
        Ok(Some(Change::Failed(m))) => {
            eprintln!("error: cannot watch the collection: {m}");
            return EXIT_USAGE;
        }
        Ok(Some(_)) => {}
        _ => {
            eprintln!("error: the file watcher did not start");
            return EXIT_USAGE;
        }
    }
    let outputs = outputs(args);
    if !outputs.is_empty() {
        eprintln!("note: report files are overwritten on every run");
    }
    eprintln!("watching for changes; press Ctrl+C to stop");
    let mut last: Option<u8> = None;
    loop {
        // An interrupt abandons a run in flight; the last finished run's code stands.
        tokio::select! {
            code = run_once(args) => last = Some(code),
            () = interrupt.recv() => return last.unwrap_or(EXIT_INTERRUPTED),
        }
        // A run may add folders, so refresh the watched set before waiting.
        if let Ok(fresh) = open_collection(args) {
            watcher.set_dirs(dirs_of(&fresh));
            collection = fresh;
        }
        eprintln!("waiting for changes");
        tokio::select! {
            res = next_change(&mut changes, &collection, &outputs) => {
                if let Err(m) = res {
                    eprintln!("error: watching stopped: {m}");
                    return EXIT_USAGE;
                }
            }
            () = interrupt.recv() => return last.unwrap_or(EXIT_INTERRUPTED),
        }
        announce_rerun();
    }
}
