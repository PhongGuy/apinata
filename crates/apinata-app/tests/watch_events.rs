//! Measurement, not a check: run with --ignored --nocapture and copy the output into the note.

use apinata_core::{Collection, ItemPath};
use notify_debouncer_full::notify::RecommendedWatcher;
use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";

type Batches = Vec<Vec<(String, Vec<PathBuf>)>>;

fn watcher(
    root: &Path,
    dirs: &[&str],
) -> (
    Debouncer<RecommendedWatcher, RecommendedCache>,
    Receiver<DebounceEventResult>,
) {
    let (tx, rx) = channel();
    let mut d = new_debouncer(Duration::from_millis(250), None, tx).unwrap();
    for dir in dirs {
        d.watch(root.join(dir), RecursiveMode::NonRecursive)
            .unwrap();
    }
    (d, rx)
}

// Batches until the watcher has been quiet for 800 ms.
fn collect(rx: &Receiver<DebounceEventResult>) -> Batches {
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_timeout(Duration::from_millis(800)) {
        match r {
            Ok(evs) => out.push(
                evs.iter()
                    .map(|e| (format!("{:?}", e.event.kind), e.event.paths.clone()))
                    .collect(),
            ),
            Err(errs) => out.push(
                errs.iter()
                    .map(|e| (format!("ERR {e}"), e.paths.clone()))
                    .collect(),
            ),
        }
    }
    out
}

fn show(title: &str, root: &Path, batches: &Batches) -> String {
    let mut s = format!("{title}\n");
    for (i, b) in batches.iter().enumerate() {
        s.push_str(&format!("  batch {i}\n"));
        for (kind, paths) in b {
            let rel: Vec<_> = paths
                .iter()
                .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
                .collect();
            s.push_str(&format!("    {kind} {rel:?}\n"));
        }
    }
    print!("{s}");
    s
}

fn fresh() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

#[test]
#[ignore]
fn measure_watch_events() {
    let mut runs = Vec::new();
    for run in 0..5 {
        let (_dir, root) = fresh();
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        for f in ["a/x.yml", "a/b/y.yml", "a/b/folder.yml", "a/b/c/z.yml"] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let (_d, rx) = watcher(&root, &["", "a", "a/b", "a/b/c"]);
        std::fs::remove_dir_all(root.join("a")).unwrap();
        let shown = show(
            &format!("recursive delete, run {run}"),
            &root,
            &collect(&rx),
        );
        // Compare the batches only; the first line carries the run number.
        runs.push(
            shown
                .split_once('\n')
                .map(|(_, rest)| rest.to_owned())
                .unwrap_or_default(),
        );
    }
    assert!(
        runs.windows(2).all(|w| w[0] == w[1]),
        "recursive delete runs differ"
    );
    println!("recursive delete runs identical: true");

    let (_dir, root) = fresh();
    let (_d, rx) = watcher(&root, &[""]);
    std::fs::create_dir(root.join("n")).unwrap();
    std::fs::write(root.join("n/r.yml"), "r").unwrap();
    show(
        "new directory with a file, root watched only",
        &root,
        &collect(&rx),
    );

    let (_dir, root) = fresh();
    std::fs::create_dir(root.join("a")).unwrap();
    let (_d, rx) = watcher(&root, &["", "a"]);
    std::fs::rename(root.join("a"), root.join("b")).unwrap();
    show("directory rename", &root, &collect(&rx));

    let (_dir, root) = fresh();
    std::fs::create_dir(root.join("d")).unwrap();
    std::fs::write(root.join("d/f.yml"), "1").unwrap();
    let (tx, rx) = channel();
    let mut d = new_debouncer(Duration::from_millis(250), None, tx).unwrap();
    d.watch(&root, RecursiveMode::NonRecursive).unwrap();
    d.watch(root.join("d"), RecursiveMode::NonRecursive)
        .unwrap();
    std::fs::remove_dir_all(root.join("d")).unwrap();
    std::fs::create_dir(root.join("d")).unwrap();
    std::fs::write(root.join("d/f.yml"), "2").unwrap();
    show("directory replaced within one window", &root, &collect(&rx));
    std::fs::write(root.join("d/f.yml"), "3").unwrap();
    show("write after replacement, old watch", &root, &collect(&rx));
    d.watch(root.join("d"), RecursiveMode::NonRecursive)
        .unwrap();
    std::fs::write(root.join("d/f.yml"), "4").unwrap();
    show("write after a second watch call", &root, &collect(&rx));

    let (_dir, root) = fresh();
    std::fs::write(root.join("opencollection.yml"), ROOT).unwrap();
    std::fs::create_dir(root.join("f")).unwrap();
    let a = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/0\n";
    std::fs::write(root.join("f/a.yml"), a).unwrap();
    let mut c = Collection::open(&root).unwrap();
    let (_d, rx) = watcher(&root, &["", "f"]);
    let r = c.request_mut(&ItemPath::new("f/a.yml").unwrap()).unwrap();
    r.set_method("POST");
    r.save().unwrap();
    let echo = collect(&rx);
    show("core save echo", &root, &echo);
    let mut paths: Vec<PathBuf> = echo
        .iter()
        .flatten()
        .filter(|(k, _)| !k.starts_with("Access"))
        .flat_map(|(_, p)| p.clone())
        .collect();
    paths.sort();
    paths.dedup();
    assert!(!paths.is_empty(), "the save produced no event");
    for p in &paths {
        println!(
            "reload_path({}) -> {:?}",
            p.strip_prefix(&root).unwrap().display(),
            c.reload_path(p)
        );
    }
}
