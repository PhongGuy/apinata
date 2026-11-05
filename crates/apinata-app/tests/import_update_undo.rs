//! Undoing the requests an Import dialog update removed: one Undo, from the toast or Ctrl+Z,
//! puts every one back as it was.

use crate::import_update::{Rig, applied, phase, preview, rig, serve};
use crate::sidebar_trash::{click_undo, ctrl_z, expire_toasts, has};
use crate::support::*;

use apinata_app::import_link::Phase;
use apinata_core::import::UpdateSummary;
use gpui_kit::TestAppContext;
use std::path::{Path, PathBuf};

/// Three requests in one folder, so removing two of them renumbers the one left.
const THREE: &[u8] = br#"{"openapi":"3.0.3","info":{"title":"Shop","version":"1"},"paths":{
 "/users/{id}":{"get":{"tags":["people"],"operationId":"getUser","responses":{"200":{"description":"ok"}}}},
 "/legacy/a":{"get":{"tags":["legacy"],"operationId":"legacyA","responses":{"200":{"description":"ok"}}}},
 "/legacy/b":{"get":{"tags":["legacy"],"operationId":"legacyB","responses":{"200":{"description":"ok"}}}},
 "/legacy/c":{"get":{"tags":["legacy"],"operationId":"legacyC","responses":{"200":{"description":"ok"}}}}}}"#;

/// `THREE` without the first and last legacy operations.
const ONE: &[u8] = br#"{"openapi":"3.0.3","info":{"title":"Shop","version":"2"},"paths":{
 "/users/{id}":{"get":{"tags":["people"],"operationId":"getUser","responses":{"200":{"description":"ok"}}}},
 "/legacy/b":{"get":{"tags":["legacy"],"operationId":"legacyB","responses":{"200":{"description":"ok"}}}}}}"#;

const LEGACY: [&str; 3] = [
    "legacy/legacyA.yml",
    "legacy/legacyB.yml",
    "legacy/legacyC.yml",
];

/// The bytes of each legacy request.
fn files(root: &Path) -> Vec<Vec<u8>> {
    LEGACY
        .iter()
        .map(|f| std::fs::read(root.join(f)).unwrap())
        .collect()
}

/// Each legacy request's seq on disk, checked against the open collection's.
fn seqs(cx: &mut TestAppContext, r: &Rig, root: &Path) -> Vec<u64> {
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    let snap = ws.read_with(cx, |w, cx| w.snapshot(root, cx)).unwrap();
    LEGACY
        .iter()
        .map(|file| {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let line = text.lines().find(|l| l.trim_start().starts_with("seq:"));
            let on_disk: u64 = line.unwrap().trim()[4..].trim().parse().unwrap();
            assert_eq!(
                snap.content.requests[&ip(file)].seq,
                Some(on_disk),
                "{file}"
            );
            on_disk
        })
        .collect()
}

/// Updates the open `THREE` collection to `ONE` with removal on, closes the dialog and returns
/// the result it showed.
fn prune_two(cx: &mut TestAppContext, r: &Rig, root: &Path) -> UpdateSummary {
    let server = serve(cx, ONE);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-prune");
    wait_for(cx, "the re-planned preview", |cx| {
        matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
    });
    preview(cx, &d);
    r.click(cx, "import-update-apply");
    applied(cx, &d);
    let Phase::Done(summary) = phase(cx, &d) else {
        unreachable!("applied waits for the result")
    };
    r.click(cx, "import-update-close");
    cx.run_until_parked();
    frame(cx, r.win);
    assert!(!root.join(LEGACY[0]).exists() && !root.join(LEGACY[2]).exists());
    assert_eq!(r.bin.len(), 2);
    summary.0
}

fn opened(cx: &mut TestAppContext) -> (Rig, PathBuf, Vec<Vec<u8>>, Vec<u64>) {
    let r = rig(cx);
    let root = r.open_spec(cx, THREE);
    let before = files(&root);
    let numbers = seqs(cx, &r, &root);
    (r, root, before, numbers)
}

// The sidebar inside the rig's shell, as the delete tests drive it.
fn harness(cx: &mut TestAppContext, r: &Rig, root: PathBuf) -> SidebarHarness {
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    let view = r.shell.read_with(cx, |s, _| s.sidebar().clone());
    SidebarHarness {
        col: tempfile::tempdir().unwrap(),
        state: tempfile::tempdir().unwrap(),
        root,
        win: r.win,
        ws,
        view,
    }
}

#[gpui_kit::test]
fn the_toasts_undo_restores_every_pruned_request_as_it_was(cx: &mut TestAppContext) {
    let (r, root, before, numbers) = opened(cx);
    let summary = prune_two(cx, &r, &root);
    assert_eq!(summary.pruned.len(), 2);
    assert!(
        summary.notes.iter().all(|n| !n.detail.contains("trash")),
        "an undoable removal needs no note: {:?}",
        summary.notes
    );
    let h = harness(cx, &r, root.clone());
    assert!(has(cx, &h, "undo-delete"), "the removal offers Undo");
    click_undo(cx, &h);
    cx.run_until_parked();
    assert_eq!(files(&root), before, "every file is back byte for byte");
    assert_eq!(seqs(cx, &r, &root), numbers);
    assert_eq!(r.bin.len(), 0);
}

#[gpui_kit::test]
fn ctrl_z_restores_every_pruned_request_after_the_toast_expired(cx: &mut TestAppContext) {
    let (r, root, before, numbers) = opened(cx);
    cx.update(apinata_app::keymap::init);
    prune_two(cx, &r, &root);
    let h = harness(cx, &r, root.clone());
    expire_toasts(cx, &h);
    focus_tree(cx, &h);
    ctrl_z(cx, &h);
    cx.run_until_parked();
    assert_eq!(files(&root), before);
    assert_eq!(seqs(cx, &r, &root), numbers);
    assert_eq!(r.bin.len(), 0);
    assert!(!has(cx, &h, "toast-error") && !has(cx, &h, "toast-warning"));
}
