//! `CloneJob`: a clone on the background executor, local bare repositories only.

use crate::git_support::{Sandbox, path_with, wait_gone};
use crate::support::{sleeper_pid, stalling_git, wait_for};

use std::fs;
use std::path::PathBuf;

use apinata_app::clone_job::{CloneEvent, CloneJob};
use apinata_core::{CloneError, CloneRequest};
use gpui_kit::TestAppContext;

fn parent(sb: &Sandbox) -> PathBuf {
    let p = sb.path().join("dest");
    fs::create_dir_all(&p).unwrap();
    p
}

fn finished(seen: &[CloneEvent]) -> bool {
    seen.iter().any(|e| matches!(e, CloneEvent::Finished(_)))
}

fn ten_percent(seen: &[CloneEvent]) -> bool {
    seen.iter()
        .any(|e| matches!(e, CloneEvent::Progress(p) if p.percent == Some(10)))
}

/// Collects events into `seen` until `until` holds.
fn pump(
    cx: &mut TestAppContext,
    job: &CloneJob,
    seen: &mut Vec<CloneEvent>,
    what: &str,
    until: fn(&[CloneEvent]) -> bool,
) {
    let rx = job.events().clone();
    wait_for(cx, what, |_| {
        while let Ok(e) = rx.try_recv() {
            seen.push(e);
        }
        until(seen)
    });
}

fn start(
    cx: &mut TestAppContext,
    url: &str,
    parent: &PathBuf,
    opts: apinata_core::GitOptions,
) -> CloneJob {
    let req = CloneRequest::new(url, parent, "col");
    cx.update(|cx| CloneJob::start(req, opts, cx))
}

#[gpui_kit::test]
fn a_local_clone_reports_progress_and_the_new_folder(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let url = format!("file://{}", sb.origin().display());
    let parent = parent(&sb);
    let job = start(cx, &url, &parent, sb.options());
    let mut seen = Vec::new();
    pump(cx, &job, &mut seen, "the clone", finished);
    assert!(seen.iter().any(|e| matches!(e, CloneEvent::Progress(_))));
    match seen.pop() {
        Some(CloneEvent::Finished(Ok(path))) => {
            assert_eq!(path, parent.join("col"));
            assert!(path.join("opencollection.yml").is_file());
        }
        other => panic!("expected a finished clone, got {:?}", other.is_some()),
    }
}

#[gpui_kit::test]
fn cancel_kills_git_and_removes_the_folder(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = stalling_git(&sb, "clone");
    let parent = parent(&sb);
    let job = start(
        cx,
        "/srv/x.git",
        &parent,
        sb.options().env("PATH", path_with(&bin)),
    );
    let mut seen = Vec::new();
    pump(cx, &job, &mut seen, "10% progress", ten_percent);
    assert!(
        parent.join("col").exists(),
        "the stalled clone made its folder"
    );
    job.cancel();
    pump(cx, &job, &mut seen, "the cancel", finished);
    assert!(matches!(
        seen.last(),
        Some(CloneEvent::Finished(Err(CloneError::Cancelled)))
    ));
    assert!(!parent.join("col").exists());
    assert!(wait_gone(sleeper_pid(&bin)));
}

#[gpui_kit::test]
fn dropping_the_job_cancels_the_clone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = stalling_git(&sb, "clone");
    let parent = parent(&sb);
    let job = start(
        cx,
        "/srv/x.git",
        &parent,
        sb.options().env("PATH", path_with(&bin)),
    );
    let mut seen = Vec::new();
    pump(cx, &job, &mut seen, "10% progress", ten_percent);
    drop(job);
    wait_for(cx, "the folder to go", |_| !parent.join("col").exists());
    assert!(wait_gone(sleeper_pid(&bin)));
}

#[gpui_kit::test]
fn a_failed_clone_finishes_with_gits_text_and_leaves_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let body = "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\nfor last; do :; done\nmkdir -p \"$last\"\necho 'fatal: Authentication failed' >&2\nexit 128\n";
    let bin = sb.fake_git("failing", body);
    let parent = parent(&sb);
    let job = start(
        cx,
        "/srv/x.git",
        &parent,
        sb.options().env("PATH", path_with(&bin)),
    );
    let mut seen = Vec::new();
    pump(cx, &job, &mut seen, "the failure", finished);
    match seen.pop() {
        Some(CloneEvent::Finished(Err(CloneError::Failed { stderr }))) => {
            assert!(stderr.contains("Authentication failed"), "{stderr}")
        }
        _ => panic!("expected CloneError::Failed"),
    }
    assert!(!parent.join("col").exists());
}
