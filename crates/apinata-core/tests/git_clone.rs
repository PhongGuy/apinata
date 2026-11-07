#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_support;

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{CancelFlag, CloneError, CloneProgress, CloneRequest, GitOptions, clone_repo};
#[cfg(target_os = "linux")]
use git_support::{HANG_GIT, wait_gone};
use git_support::{Sandbox, path_with};

fn parent(sb: &Sandbox) -> PathBuf {
    let p = sb.path().join("dest");
    fs::create_dir_all(&p).unwrap();
    p
}

fn run(
    url: &str,
    parent: &Path,
    opts: &GitOptions,
    cancel: &CancelFlag,
) -> (Result<PathBuf, CloneError>, Vec<CloneProgress>) {
    let mut seen = Vec::new();
    let r = clone_repo(
        &CloneRequest::new(url, parent, "col"),
        cancel,
        opts,
        &mut |p| seen.push(p),
    );
    (r, seen)
}

#[cfg(target_os = "linux")]
fn pid(bin: &Path) -> u32 {
    fs::read_to_string(bin.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn a_path_url_clones_a_working_directory() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let parent = parent(&sb);
    let (r, _) = run(
        bare.to_str().unwrap(),
        &parent,
        &sb.options(),
        &CancelFlag::new(),
    );
    assert_eq!(r.unwrap(), parent.join("col"));
    assert!(parent.join("col/opencollection.yml").is_file());
}

#[test]
fn a_file_url_reports_progress() {
    let sb = Sandbox::new();
    let url = format!("file://{}", sb.origin().display());
    let parent = parent(&sb);
    let (r, seen) = run(&url, &parent, &sb.options(), &CancelFlag::new());
    r.unwrap();
    assert!(
        seen.iter().any(|p| p.phase.starts_with("Cloning into")),
        "{seen:?}"
    );
    assert!(seen.iter().all(|p| p.percent.is_none_or(|n| n <= 100)));
}

#[test]
fn a_non_empty_destination_is_refused_and_untouched() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let parent = parent(&sb);
    git_support::write(&parent, "col/keep.txt", "mine\n");
    let (r, _) = run(
        bare.to_str().unwrap(),
        &parent,
        &sb.options(),
        &CancelFlag::new(),
    );
    assert!(matches!(r, Err(CloneError::Exists(p)) if p == parent.join("col")));
    assert_eq!(
        fs::read_to_string(parent.join("col/keep.txt")).unwrap(),
        "mine\n"
    );
}

#[test]
fn an_empty_destination_is_filled() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let parent = parent(&sb);
    fs::create_dir(parent.join("col")).unwrap();
    let (r, _) = run(
        bare.to_str().unwrap(),
        &parent,
        &sb.options(),
        &CancelFlag::new(),
    );
    r.unwrap();
    assert!(parent.join("col/a.txt").is_file());
}

#[test]
fn a_rejected_url_never_starts_git() {
    let sb = Sandbox::new();
    let opts = sb.options().env("PATH", sb.empty_bin());
    let (r, _) = run("ext::sh -c id", &parent(&sb), &opts, &CancelFlag::new());
    assert!(matches!(r, Err(CloneError::InvalidUrl { .. })), "{r:?}");
    let bad_name = CloneRequest::new("/srv/x.git", parent(&sb), "../x");
    let r = clone_repo(&bad_name, &CancelFlag::new(), &opts, &mut |_| {});
    assert!(matches!(r, Err(CloneError::InvalidName { .. })), "{r:?}");
}

#[test]
fn a_missing_git_is_git_missing_and_creates_nothing() {
    let sb = Sandbox::new();
    let opts = sb.options().env("PATH", sb.empty_bin());
    let parent = parent(&sb);
    let (r, _) = run("/srv/x.git", &parent, &opts, &CancelFlag::new());
    assert!(matches!(r, Err(CloneError::GitMissing)), "{r:?}");
    assert!(!parent.join("col").exists());
}

#[test]
fn a_missing_parent_is_an_io_error() {
    let sb = Sandbox::new();
    let (r, _) = run(
        "/srv/x.git",
        &sb.path().join("nope"),
        &sb.options(),
        &CancelFlag::new(),
    );
    assert!(matches!(r, Err(CloneError::Io(_))), "{r:?}");
}

#[test]
fn a_flag_set_before_the_call_leaves_nothing() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let parent = parent(&sb);
    let cancel = CancelFlag::new();
    cancel.cancel();
    let (r, _) = run(bare.to_str().unwrap(), &parent, &sb.options(), &cancel);
    assert!(matches!(r, Err(CloneError::Cancelled)));
    assert!(!parent.join("col").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn cancel_during_a_clone_kills_the_tree_and_removes_the_directory() {
    let sb = Sandbox::new();
    let bin = sb.fake_git("fake", HANG_GIT);
    let parent = parent(&sb);
    let opts = sb.options().env("PATH", path_with(&bin));
    let cancel = CancelFlag::new();
    let mut seen = Vec::new();
    let r = clone_repo(
        &CloneRequest::new("/srv/x.git", &parent, "col"),
        &cancel,
        &opts,
        &mut |p| {
            if p.percent == Some(10) {
                cancel.cancel();
            }
            seen.push(p);
        },
    );
    assert!(matches!(r, Err(CloneError::Cancelled)), "{r:?}");
    assert!(
        seen.iter()
            .any(|p| p.phase == "Receiving objects" && p.percent == Some(10))
    );
    assert!(!parent.join("col").exists());
    assert!(wait_gone(pid(&bin)));
}

#[cfg(target_os = "linux")]
#[test]
fn cancel_keeps_a_pre_existing_empty_directory_but_empties_it() {
    let sb = Sandbox::new();
    let bin = sb.fake_git("fake", HANG_GIT);
    let parent = parent(&sb);
    fs::create_dir(parent.join("col")).unwrap();
    let opts = sb.options().env("PATH", path_with(&bin));
    let cancel = CancelFlag::new();
    let r = clone_repo(
        &CloneRequest::new("/srv/x.git", &parent, "col"),
        &cancel,
        &opts,
        &mut |_| cancel.cancel(),
    );
    assert!(matches!(r, Err(CloneError::Cancelled)));
    assert_eq!(fs::read_dir(parent.join("col")).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_failed_clone_returns_git_text_and_removes_the_directory() {
    let sb = Sandbox::new();
    let body = "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\nfor last; do :; done\nmkdir -p \"$last\"\necho 'fatal: Authentication failed' >&2\nexit 128\n";
    let bin = sb.fake_git("fake", body);
    let parent = parent(&sb);
    let opts = sb.options().env("PATH", path_with(&bin));
    let (r, _) = run("/srv/x.git", &parent, &opts, &CancelFlag::new());
    assert!(
        matches!(&r, Err(CloneError::Failed { stderr }) if stderr.contains("Authentication failed")),
        "{r:?}"
    );
    assert!(!parent.join("col").exists());
}

#[test]
fn a_name_starting_with_a_dash_is_refused_and_creates_nothing() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let parent = parent(&sb);
    let req = CloneRequest::new(bare.to_str().unwrap(), &parent, "-x");
    let r = clone_repo(&req, &CancelFlag::new(), &sb.options(), &mut |_| {});
    assert!(matches!(r, Err(CloneError::InvalidName { .. })), "{r:?}");
    assert!(!parent.join("-x").exists());
}

#[cfg(unix)]
#[test]
fn credentials_in_the_url_never_reach_errors_or_progress() {
    let sb = Sandbox::new();
    let url = "https://user:s3cret@example.com/x.git";
    let body = format!(
        "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\necho \"Cloning into '{url}'...\" >&2\necho \"fatal: unable to access '{url}': denied\" >&2\nexit 128\n"
    );
    let bin = sb.fake_git("fake", &body);
    let opts = sb.options().env("PATH", path_with(&bin));
    let (r, seen) = run(url, &parent(&sb), &opts, &CancelFlag::new());
    let text = format!("{r:?} {seen:?}");
    assert!(matches!(r, Err(CloneError::Failed { .. })), "{text}");
    assert!(!text.contains("s3cret"), "{text}");
    assert!(text.contains("example.com"), "{text}");
}
