//! A named pipe in a collection is never opened: opening one waits for a writer that never comes.

use crate::common;
use crate::script_support;
use crate::tree_support::ip;

use apinata_core::watch::seams::gap_paths;
use apinata_core::{Collection, ContextOptions, DropTarget, SaveError, TreeError, VarContext};
use common::{request, write_tree};
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const ITEM: &str =
    "info:\n  name: t\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/\"\n";

fn mkfifo(path: &Path) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    let made = Command::new("mkfifo").arg(path).status().expect("mkfifo");
    assert!(made.success(), "mkfifo failed");
}

// A collection with a pipe at `rel`.
fn with_pipe(rel: &str) -> (TempDir, PathBuf) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("d/a.yml", ITEM)]);
    let pipe = dir.path().join(rel);
    mkfifo(&pipe);
    (dir, pipe)
}

// Runs `job` on its own thread under a time limit. A job stuck on `pipe` fails the test, and a
// writer is opened apart so the stuck thread ends; that open never returns for a job not on it.
fn finishes<T: Send + 'static>(pipe: &Path, job: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, got) = channel();
    thread::spawn(move || {
        let _ = done.send(job());
    });
    got.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| {
            let released = pipe.to_path_buf();
            thread::spawn(move || drop(fs::OpenOptions::new().write(true).open(released)));
            panic!("blocked on the pipe at {}", pipe.display());
        })
}

fn is_pipe(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_fifo())
}

// A collection whose request `t.yml` has an unsaved edit, with a pipe put over its file.
fn dirty_under_a_pipe() -> (TempDir, Collection, PathBuf) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("t.yml", ITEM)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let request = c.request_mut(&ip("t.yml")).expect("a request");
    request.set_name("draft");
    let pipe = dir.path().join("t.yml");
    fs::remove_file(&pipe).unwrap();
    mkfifo(&pipe);
    (dir, c, pipe)
}

fn opens(root: &Path, pipe: &Path) -> Collection {
    let root = root.to_path_buf();
    finishes(pipe, move || Collection::open(root).expect("opens"))
}

// `reload_path` on the pipe, as the watcher sends each entry of a directory it starts watching.
fn reloads(mut c: Collection, pipe: &Path) -> (Collection, usize) {
    let path = pipe.to_path_buf();
    finishes(pipe, move || {
        let changes = c.reload_path(&path).len();
        (c, changes)
    })
}

#[test]
fn a_new_watch_reports_a_pipe_among_the_entries() {
    let (dir, pipe) = with_pipe("t.yml");
    assert!(gap_paths(&[dir.path().to_path_buf()]).contains(&pipe));
}

#[test]
fn an_open_leaves_a_pipe_item_out() {
    let (dir, pipe) = with_pipe("t.yml");
    let c = opens(dir.path(), &pipe);
    assert!(c.tree().requests().is_empty());
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn a_reload_passes_over_a_pipe_item() {
    let (dir, pipe) = with_pipe("t.yml");
    let c = opens(dir.path(), &pipe);
    let (c, changes) = reloads(c, &pipe);
    assert_eq!(changes, 0);
    assert!(c.tree().requests().is_empty());
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn a_reload_passes_over_a_pipe_environment() {
    let (dir, pipe) = with_pipe("environments/e.yml");
    let c = opens(dir.path(), &pipe);
    let (c, changes) = reloads(c, &pipe);
    assert_eq!(changes, 0);
    assert!(c.environments().is_empty());
}

#[test]
fn a_reload_passes_over_a_pipe_folder_file() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("d/a.yml", ITEM)]);
    let pipe = dir.path().join("d/folder.yml");
    let c = Collection::open(dir.path()).expect("opens");
    mkfifo(&pipe);
    let (c, changes) = reloads(c, &pipe);
    assert_eq!(changes, 0);
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn an_open_passes_over_a_pipe_folder_file() {
    let (dir, pipe) = with_pipe("d/folder.yml");
    let c = opens(dir.path(), &pipe);
    assert_eq!(c.tree().folders().len(), 1);
    assert_eq!(c.tree().folders()[0].requests().len(), 1);
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn a_pipe_dotenv_reads_as_absent() {
    let (dir, pipe) = with_pipe(".env");
    let c = opens(dir.path(), &pipe);
    let (c, changes) = reloads(c, &pipe);
    assert_eq!(changes, 0);
    finishes(&pipe, move || {
        VarContext::new(&c, ContextOptions::new()).expect("context");
    });
}

#[test]
fn a_reload_ignores_a_pipe_over_the_root_file() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let c = Collection::open(dir.path()).expect("opens");
    let pipe = dir.path().join("opencollection.yml");
    fs::remove_file(&pipe).unwrap();
    mkfifo(&pipe);
    let (c, changes) = reloads(c, &pipe);
    assert_eq!(changes, 0);
    assert_eq!(c.name(), "t");
}

#[test]
fn a_pipe_over_a_known_request_reads_as_gone() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("t.yml", ITEM)]);
    let c = Collection::open(dir.path()).expect("opens");
    let pipe = dir.path().join("t.yml");
    fs::remove_file(&pipe).unwrap();
    mkfifo(&pipe);
    let (c, changes) = reloads(c, &pipe);
    assert_eq!(changes, 1);
    assert!(c.tree().requests().is_empty());
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
}

#[test]
fn a_link_to_a_pipe_is_not_followed() {
    let (dir, pipe) = with_pipe("elsewhere/p");
    let link = dir.path().join("t.yml");
    std::os::unix::fs::symlink(&pipe, &link).unwrap();
    let c = Collection::open(dir.path()).expect("opens");
    let (c, changes) = finishes(&pipe, move || {
        let mut c = c;
        let changes = c.reload_path(&link).len();
        (c, changes)
    });
    assert_eq!(changes, 0);
    assert!(c.tree().requests().is_empty());
}

// A link to a regular file is read on reload as it always was.
#[test]
fn a_link_to_a_request_still_reloads() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("other/r", ITEM)]);
    let link = dir.path().join("t.yml");
    std::os::unix::fs::symlink(dir.path().join("other/r"), &link).unwrap();
    let mut c = Collection::open(dir.path()).expect("opens");
    assert!(c.tree().requests().is_empty(), "an open leaves a link out");
    assert_eq!(c.reload_path(&link).len(), 1);
    assert_eq!(request(&c, "t.yml").name(), "t");
}

#[test]
fn a_pipe_dotenv_has_no_keys() {
    let (dir, pipe) = with_pipe(".env");
    let c = opens(dir.path(), &pipe);
    let keys = finishes(&pipe, move || c.dotenv_keys());
    assert!(keys.expect("keys").is_empty());
}

// A rename would swap the pipe for a secrets file that kept the pipe's open mode.
#[test]
fn a_pipe_dotenv_refuses_a_value_and_stays() {
    let (dir, pipe) = with_pipe(".env");
    let mut c = opens(dir.path(), &pipe);
    let set = finishes(&pipe, move || c.set_dotenv("KEY", "value"));
    assert!(set.is_err());
    assert!(is_pipe(&pipe));
}

// Taking the pipe as an empty file would move the wait to the write that follows.
#[test]
fn a_pipe_gitignore_refuses_the_dotenv_line_and_stays() {
    let (dir, pipe) = with_pipe(".gitignore");
    let c = opens(dir.path(), &pipe);
    let ignored = finishes(&pipe, move || c.ensure_dotenv_ignored());
    assert!(ignored.is_err());
    assert!(is_pipe(&pipe));
}

#[test]
fn the_texts_of_a_pipe_over_a_request_have_no_disk_side() {
    let (_dir, c, pipe) = dirty_under_a_pipe();
    let path = pipe.clone();
    let texts = finishes(&pipe, move || c.file_texts(&path)).expect("a document");
    assert_eq!(texts.disk, None);
    assert!(texts.draft.contains("name: draft"), "{}", texts.draft);
}

#[test]
fn a_save_over_a_pipe_conflicts_and_keeps_the_draft_and_the_pipe() {
    let (dir, c, pipe) = dirty_under_a_pipe();
    let (c, saved) = finishes(&pipe, move || {
        let mut c = c;
        let saved = c.request_mut(&ip("t.yml")).expect("a request").save();
        (c, saved)
    });
    assert!(
        matches!(saved, Err(SaveError::Conflict { .. })),
        "{saved:?}"
    );
    assert!(request(&c, "t.yml").is_dirty());
    assert!(is_pipe(&pipe));
    let names: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert_eq!(names.len(), 2, "no temp file stays: {names:?}");
}

// The person chose to replace what the disk holds; the rename never opens the pipe.
#[test]
fn an_overwrite_replaces_a_pipe_with_the_draft() {
    let (_dir, mut c, pipe) = dirty_under_a_pipe();
    let path = pipe.clone();
    finishes(&pipe, move || c.save_overwrite(&path)).expect("overwrites");
    assert!(!is_pipe(&pipe));
    let text = fs::read_to_string(&pipe).unwrap();
    assert!(text.contains("name: draft"), "{text}");
}

#[test]
fn a_move_conflicts_with_a_pipe_over_an_item() {
    let first = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: u\n";
    let second = "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: u\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", first),
        ("b.yml", second),
    ]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let pipe = dir.path().join("a.yml");
    fs::remove_file(&pipe).unwrap();
    mkfifo(&pipe);
    let moved = finishes(&pipe, move || {
        c.move_item(&ip("b.yml"), DropTarget::Before(ip("a.yml")))
    });
    assert!(
        matches!(moved, Err(TreeError::Conflict { .. })),
        "{moved:?}"
    );
    assert!(is_pipe(&pipe));
    assert_eq!(
        fs::read_to_string(dir.path().join("b.yml")).unwrap(),
        second
    );
}

#[test]
fn a_pipe_at_the_temp_path_never_stops_a_save() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("t.yml", ITEM)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    c.request_mut(&ip("t.yml"))
        .expect("a request")
        .set_name("draft");
    let pipe = dir.path().join(".t.yml.apinata-tmp");
    mkfifo(&pipe);
    finishes(&pipe, move || {
        c.request_mut(&ip("t.yml")).expect("a request").save()
    })
    .expect("saves");
    let saved = fs::read_to_string(dir.path().join("t.yml")).expect("the file");
    assert!(saved.contains("name: draft"), "{saved}");
    assert!(!pipe.exists(), "the temp path was left behind");
}

// A request in `dir` posting `under_http` to a port nothing answers, with a pipe at `rel`.
fn request_over_a_pipe(under_http: &str, rel: &str) -> (TempDir, PathBuf) {
    let head =
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: http://127.0.0.1:9/\n";
    let text = format!("{head}{under_http}");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let pipe = dir.path().join(rel);
    mkfifo(&pipe);
    (dir, pipe)
}

// Resolves and sends `r.yml`, giving back what went wrong.
fn send_failure(root: &Path, pipe: &Path) -> String {
    let root = root.to_path_buf();
    finishes(pipe, move || {
        let c = Collection::open(&root).expect("opens");
        let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let request = common::at(&c, &ctx, "r.yml").map_err(|e| e.to_string())?;
        let sent = runtime.block_on(common::executor().send(&request));
        sent.map(|_| ()).map_err(|e| e.to_string())
    })
    .expect_err("a pipe is not a file to send")
}

#[test]
fn a_pipe_as_an_oauth1_key_file_fails_the_send() {
    let auth = "  auth:\n    type: oauth1\n    consumerKey: ck\n    signatureMethod: RSA-SHA1\n    privateKey:\n      type: file\n      value: k.pem\n    placement: header\n";
    let (dir, pipe) = request_over_a_pipe(auth, "k.pem");
    let failure = send_failure(dir.path(), &pipe);
    assert!(failure.contains("private key file"), "{failure}");
    assert!(failure.contains("not a regular file"), "{failure}");
}

#[test]
fn a_pipe_as_a_script_module_fails_the_require() {
    let code = "try { require(\"./lib.js\") } catch (e) { console.log(e.message) }";
    let scripts = script_support::script("before-request", code);
    let text = script_support::scripted("R", "http://127.0.0.1:9/r", &scripts, "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let pipe = dir.path().join("lib.js");
    mkfifo(&pipe);
    let root = dir.path().to_path_buf();
    let logged = finishes(&pipe, move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");
        let result = runtime.block_on(script_support::send(&root, "r.yml", script_support::on()));
        script_support::console(&result)
    });
    assert_eq!(logged, ["log:Error loading local module ./lib.js"]);
}
