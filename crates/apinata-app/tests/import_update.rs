//! Updating an open collection from the Import dialog: the dialog only sends commands to the
//! collection's actor, with a fake trash and the spec served from a local wiremock.

use crate::support::fake_trash::FakeTrash;
use crate::support::*;

use apinata_app::import_link::{self, ImportLinkDialog, Mode, Phase};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, parse_collections, write_collection,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::path::PathBuf;
use std::sync::Arc;
use wiremock::ResponseTemplate;

pub(crate) const V1: &[u8] =
    include_bytes!("../../apinata-core/tests/fixtures/openapi/update-v1.json");
pub(crate) const V2: &[u8] =
    include_bytes!("../../apinata-core/tests/fixtures/openapi/update-v2.json");
const TWO: &[u8] =
    include_bytes!("../../apinata-core/tests/fixtures/insomnia/kong/multiple-workspaces.yaml");

pub(crate) struct Rig {
    pub(crate) _files: tempfile::TempDir,
    pub(crate) parent: tempfile::TempDir,
    pub(crate) win: AnyWindowHandle,
    pub(crate) shell: Entity<Shell>,
    pub(crate) bin: Arc<FakeTrash>,
}

pub(crate) fn rig(cx: &mut TestAppContext) -> Rig {
    crate::support::init(cx);
    http_ready(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let bin = Arc::new(FakeTrash::new());
    ws.update(cx, |w, _| w.set_trash(bin.clone()));
    let (win, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    Rig {
        _files: files,
        parent: tempfile::tempdir().unwrap(),
        win,
        shell,
        bin,
    }
}

impl Rig {
    /// Writes a collection imported from the v1 spec and opens it.
    pub(crate) fn open_v1(&self, cx: &mut TestAppContext) -> PathBuf {
        self.open_spec(cx, V1)
    }

    /// Writes a collection imported from `spec` and opens it.
    pub(crate) fn open_spec(&self, cx: &mut TestAppContext, spec: &[u8]) -> PathBuf {
        let mut all = parse_collections(
            Some(CollectionFormat::OpenApi),
            spec,
            &CollectionOptions::default(),
        )
        .unwrap();
        let b = all.remove(0);
        let (c, _) = write_collection(
            self.parent.path(),
            &b.collection,
            &b.environments,
            &MemoryStore::new(),
        )
        .unwrap();
        self.open(cx, c.root().to_path_buf())
    }

    pub(crate) fn open(&self, cx: &mut TestAppContext, root: PathBuf) -> PathBuf {
        let ws = self.shell.read_with(cx, |s, _| s.workspace().clone());
        ws.update(cx, |w, cx| w.open(root.clone(), cx));
        cx.run_until_parked();
        root
    }

    pub(crate) fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        cx.update_window(self.win, |_, w, cx| w.click(id, cx))
            .unwrap();
        frame(cx, self.win);
    }

    /// Opens the dialog in Update mode with the spec at `link`.
    pub(crate) fn dialog(&self, cx: &mut TestAppContext, link: &str) -> Entity<ImportLinkDialog> {
        switcher_click(cx, self.win, "switcher-import-collection");
        frame(cx, self.win);
        let view = cx.read(import_link::current).expect("dialog open");
        let (v, link) = (view.clone(), link.to_owned());
        cx.update_window(self.win, |_, w, cx| {
            v.update(cx, |d, cx| d.set_link(&link, w, cx))
        })
        .unwrap();
        self.click(cx, "import-mode-update");
        cx.run_until_parked();
        frame(cx, self.win);
        view
    }
}

pub(crate) fn phase(cx: &mut TestAppContext, d: &Entity<ImportLinkDialog>) -> Phase {
    d.read_with(cx, |d, _| d.phase().clone())
}

/// The primary action as Enter runs it, which does not look at the button's disabled state.
fn submit(cx: &mut TestAppContext, r: &Rig, d: &Entity<ImportLinkDialog>) {
    let d = d.clone();
    cx.update_window(r.win, |_, w, cx| d.update(cx, |d, cx| d.submit(w, cx)))
        .unwrap();
}

pub(crate) fn serve(cx: &mut TestAppContext, body: &[u8]) -> wiremock::MockServer {
    mock_server(cx, ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
}

pub(crate) fn preview(
    cx: &mut TestAppContext,
    d: &Entity<ImportLinkDialog>,
) -> apinata_core::import::UpdateSummary {
    wait_for(cx, "the preview", |cx| {
        matches!(phase(cx, d), Phase::Preview { .. } | Phase::Failed(_))
    });
    match phase(cx, d) {
        Phase::Preview { summary, .. } => summary.0,
        other => panic!("not a preview: {other:?}"),
    }
}

#[gpui_kit::test]
fn the_preview_lists_added_changed_and_orphaned(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(d.read_with(cx, |d, _| d.mode()), Mode::Update);
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    assert!(!s.added.is_empty() && !s.changed.is_empty() && !s.orphaned.is_empty());
    assert!(
        root.join("legacy/removeLegacy.yml").exists(),
        "a preview writes nothing"
    );
}

#[gpui_kit::test]
fn prune_off_keeps_the_orphan(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    assert!(root.join("legacy/removeLegacy.yml").exists());
    assert_eq!(r.bin.len(), 0);
    assert!(
        root.join("people/listUsers.yml").exists(),
        "the added request landed"
    );
}

#[gpui_kit::test]
fn prune_on_moves_the_orphan_to_the_trash(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-prune");
    wait_for(cx, "the re-planned preview", |cx| {
        matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
    });
    let s = preview(cx, &d);
    assert_eq!(s.pruned.len(), 1, "the plan counts the removal");
    assert!(root.join("legacy/removeLegacy.yml").exists());
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    assert!(!root.join("legacy/removeLegacy.yml").exists());
    assert_eq!(r.bin.len(), 1);
    r.click(cx, "import-update-close");
    assert!(!present(cx, r.win, "import-link-dialog"));
}

#[gpui_kit::test]
fn an_unsaved_edit_blocks_the_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("people/getUser.yml", "http://edited/"), cx)
    });
    cx.run_until_parked();
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    submit(cx, &r, &d);
    cx.run_until_parked();
    let text = d.read_with(cx, |d, _| d.error().unwrap_or_default().to_owned());
    assert!(
        text.contains("Save or discard your unsaved edits"),
        "{text}"
    );
    assert!(matches!(phase(cx, &d), Phase::Failed(_)));
    assert!(root.join("legacy/removeLegacy.yml").exists());
}

#[gpui_kit::test]
fn a_collection_without_a_baseline_cannot_be_chosen(cx: &mut TestAppContext) {
    let r = rig(cx);
    let plain = collection();
    r.open(cx, plain.path().to_path_buf());
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let targets = d.read_with(cx, |d, _| d.targets());
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].baseline, Some(false));
    assert_eq!(
        targets[0].reason().as_deref(),
        Some("Not imported from a spec, so it cannot be updated")
    );
    r.click(cx, "import-update-target-0");
    assert_eq!(d.read_with(cx, |d, _| d.selected()), None);
    submit(cx, &r, &d);
    cx.run_until_parked();
    assert!(
        matches!(phase(cx, &d), Phase::Failed(_)),
        "nothing to update"
    );
}

#[gpui_kit::test]
fn a_spec_with_several_collections_is_rejected(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, TWO);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, &d), Phase::Failed(_))
    });
    let text = d.read_with(cx, |d, _| d.error().unwrap_or_default().to_owned());
    assert!(
        text.contains("This spec holds several collections; update needs one"),
        "{text}"
    );
}

pub(crate) fn applied(cx: &mut TestAppContext, d: &Entity<ImportLinkDialog>) {
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, d), Phase::Done(_))
    });
}

#[gpui_kit::test]
fn the_mode_cannot_change_while_applying_or_after(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    // Click Update and switch mode before the actor has answered.
    cx.update_window(r.win, |_, w, cx| w.click("import-update-apply", cx))
        .unwrap();
    d.update(cx, |d, cx| d.set_mode(Mode::New, cx));
    assert_eq!(d.read_with(cx, |d, _| d.mode()), Mode::Update);
    applied(cx, &d);
    d.update(cx, |d, cx| d.set_mode(Mode::New, cx));
    assert!(matches!(phase(cx, &d), Phase::Done(_)));
    assert!(root.join("people/listUsers.yml").exists());
}

#[gpui_kit::test]
fn the_preview_stays_while_a_toggle_plans_again(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    cx.update_window(r.win, |_, w, cx| w.click("import-update-prune", cx))
        .unwrap();
    let p = phase(cx, &d);
    assert!(matches!(p, Phase::Preview { .. }), "no flicker: {p:?}");
    assert!(present(cx, r.win, "import-update-prune"));
    wait_for(cx, "the new plan", |cx| {
        matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
    });
}

#[gpui_kit::test]
fn changed_fields_read_as_words(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    let rows = d.read_with(cx, |d, _| d.changed_rows());
    let (_, fields) = rows.first().expect("a changed row");
    assert!(
        !fields.contains("Target") && !fields.contains("Headers"),
        "{fields}"
    );
    assert!(!s.changed.is_empty());
}

#[gpui_kit::test]
fn an_edit_after_the_preview_blocks_apply(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("people/getUser.yml", "http://edited/"), cx)
    });
    cx.run_until_parked();
    frame(cx, r.win);
    let why = d.read_with(cx, |d, cx| d.blocked(cx)).expect("blocked");
    assert!(why.contains("Save or discard your unsaved edits"), "{why}");
    assert!(present(cx, r.win, "import-update-blocked"));
    r.click(cx, "import-update-apply");
    d.update(cx, |d, cx| d.submit_for_test(cx));
    cx.run_until_parked();
    assert!(matches!(phase(cx, &d), Phase::Preview { .. }));
    assert!(
        !root.join("people/listUsers.yml").exists(),
        "nothing was written"
    );
}

#[gpui_kit::test]
fn a_double_click_on_update_applies_once(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-prune");
    wait_for(cx, "the new plan", |cx| {
        matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
    });
    for _ in 0..2 {
        cx.update_window(r.win, |_, w, cx| w.click("import-update-apply", cx))
            .unwrap();
    }
    applied(cx, &d);
    assert_eq!(r.bin.len(), 1);
    assert!(root.join("people/listUsers.yml").exists());
}

#[gpui_kit::test]
fn enter_applies_in_the_preview_and_closes_when_done(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    cx.update_window(r.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    applied(cx, &d);
    assert!(root.join("people/listUsers.yml").exists());
    cx.update_window(r.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, r.win);
    assert!(!present(cx, r.win, "import-link-dialog"));
}

#[gpui_kit::test]
fn the_preview_groups_items_in_labelled_sections(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    for id in [
        "import-update-body",
        "import-update-section-added",
        "import-update-section-changed",
        "import-update-section-missing",
        "import-update-section-unchanged",
        "import-update-prune-note",
    ] {
        assert!(present(cx, r.win, id), "{id} is shown");
    }
    assert!(
        !present(cx, r.win, "import-update-section-edited"),
        "an empty section is left out"
    );
    assert!(!present(cx, r.win, "import-update-section-removed"));
}

#[gpui_kit::test]
fn the_result_shows_counts_and_what_was_removed(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-prune");
    wait_for(cx, "the re-planned preview", |cx| {
        matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
    });
    r.click(cx, "import-update-apply");
    applied(cx, &d);
    frame(cx, r.win);
    for id in [
        "import-update-counts",
        "import-update-section-added",
        "import-update-section-removed",
        "import-update-close",
    ] {
        assert!(present(cx, r.win, id), "{id} is shown");
    }
    assert!(!present(cx, r.win, "import-update-prune"));
}

#[gpui_kit::test]
fn a_disabled_collection_shows_its_reason(cx: &mut TestAppContext) {
    let r = rig(cx);
    let plain = collection();
    r.open(cx, plain.path().to_path_buf());
    let server = serve(cx, V2);
    r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert!(present(cx, r.win, "import-update-reason-0"));
}

#[gpui_kit::test]
fn the_prune_checkbox_starts_off_and_a_disabled_row_stays_unselected(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let plain = collection();
    r.open(cx, plain.path().to_path_buf());
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    // The active collection is preselected; the row without a baseline cannot take its place.
    assert_eq!(d.read_with(cx, |d, _| d.selected()), Some(0));
    r.click(cx, "import-update-target-1");
    assert_eq!(d.read_with(cx, |d, _| d.selected()), Some(0));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    assert!(!d.read_with(cx, |d, _| d.prune()));
    assert!(matches!(phase(cx, &d), Phase::Preview { prune: false, .. }));
}

/// Runs the dialog update to v2 on the open v1 collection and waits for the result.
fn update_to_v2(cx: &mut TestAppContext, r: &Rig, prune: bool) -> wiremock::MockServer {
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    if prune {
        r.click(cx, "import-update-prune");
        wait_for(cx, "the re-planned preview", |cx| {
            matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
        });
        preview(cx, &d);
    }
    r.click(cx, "import-update-apply");
    applied(cx, &d);
    cx.run_until_parked();
    server
}

#[gpui_kit::test]
fn an_open_tab_shows_the_updated_request_without_a_reload(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    let item = ip("people/getUser.yml");
    let tab = ws
        .update(cx, |w, cx| w.open_item(&root, item.clone(), false, cx))
        .expect("a request tab");
    let before = ws.read_with(cx, |w, cx| w.request(&root, &item, cx).unwrap());
    assert!(before.params.iter().all(|p| p.name != "include"));
    let _server = update_to_v2(cx, &r, false);
    let after = ws.read_with(cx, |w, cx| w.request(&root, &item, cx).unwrap());
    assert!(
        after.params.iter().any(|p| p.name == "include"),
        "the open tab's request carries v2: {:?}",
        after.params
    );
    assert!(
        ws.read_with(cx, |w, _| w.tabs().get(tab).is_some()),
        "tab stays"
    );
}

#[gpui_kit::test]
fn a_tab_on_a_pruned_request_closes_like_an_external_delete(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    let legacy = ip("legacy/removeLegacy.yml");
    let tab = ws
        .update(cx, |w, cx| w.open_item(&root, legacy.clone(), false, cx))
        .expect("a request tab");
    let _server = update_to_v2(cx, &r, true);
    assert!(!root.join("legacy/removeLegacy.yml").exists());
    assert_eq!(r.bin.len(), 1);
    assert!(
        ws.read_with(cx, |w, cx| w.request(&root, &legacy, cx).is_none()),
        "the pruned request left the snapshot"
    );
    assert!(
        ws.read_with(cx, |w, _| w.tabs().get(tab).is_none()),
        "its tab closed, as for a file deleted outside the app"
    );
}

#[gpui_kit::test]
fn a_broken_trash_refuses_removal_and_shows_why(cx: &mut TestAppContext) {
    let r = rig(cx);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    ws.update(cx, |w, _| w.set_trash(Arc::new(FakeTrash::broken())));
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-prune");
    wait_for(cx, "the refusal", |cx| {
        matches!(phase(cx, &d), Phase::Failed(_))
    });
    let text = d.read_with(cx, |d, _| d.error().unwrap_or_default().to_owned());
    assert!(text.contains("The trash is not available"), "{text}");
    assert!(!d.read_with(cx, |d, _| d.prune()), "removal is off again");
    assert!(root.join("legacy/removeLegacy.yml").exists());
    assert!(!root.join("people/listUsers.yml").exists());
}

#[gpui_kit::test]
fn switching_mode_turns_removal_off(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-prune");
    wait_for(cx, "prune plan", |cx| {
        matches!(phase(cx, &d), Phase::Preview { prune: true, .. })
    });
    d.update(cx, |d, cx| {
        d.set_mode(Mode::New, cx);
        d.set_mode(Mode::Update, cx);
    });
    assert!(!d.read_with(cx, |d, _| d.prune()));
}

#[gpui_kit::test]
fn cancel_is_off_while_applying(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    cx.update_window(r.win, |_, w, cx| w.click("import-update-apply", cx))
        .unwrap();
    assert_eq!(phase(cx, &d), Phase::Applying);
    cx.update_window(r.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    assert!(present(cx, r.win, "import-link-dialog"), "still open");
    applied(cx, &d);
}

#[gpui_kit::test]
fn a_closed_collection_is_named_at_apply(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    d.update(cx, |d, cx| d.submit_for_test(cx));
    let text = d.read_with(cx, |d, _| d.error().unwrap_or_default().to_owned());
    assert!(text.contains("That collection is no longer open"), "{text}");
}

#[gpui_kit::test]
fn the_active_collection_with_a_baseline_is_preselected(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(d.read_with(cx, |d, _| d.selected()), Some(0));
    assert_eq!(d.read_with(cx, |d, cx| d.primary_blocked(cx)), None);
}

#[gpui_kit::test]
fn an_active_collection_without_a_baseline_is_not_preselected(cx: &mut TestAppContext) {
    let r = rig(cx);
    // With no tab open the first collection is the active one.
    let plain = collection();
    r.open(cx, plain.path().to_path_buf());
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let targets = d.read_with(cx, |d, _| d.targets());
    assert_eq!(targets[1].baseline, Some(true));
    assert_eq!(d.read_with(cx, |d, _| d.selected()), None);
}

#[gpui_kit::test]
fn unsaved_edits_disable_the_primary_button(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("people/getUser.yml", "http://edited/"), cx)
    });
    cx.run_until_parked();
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let why = d.read_with(cx, |d, cx| d.primary_blocked(cx));
    assert_eq!(why.as_deref(), Some(import_link::UNSAVED));
    // A click on the disabled button starts nothing.
    r.click(cx, "import-link-import");
    cx.run_until_parked();
    assert_eq!(phase(cx, &d), Phase::Idle);
}

#[gpui_kit::test]
fn no_chosen_collection_disables_the_primary_button(cx: &mut TestAppContext) {
    let r = rig(cx);
    let plain = collection();
    r.open(cx, plain.path().to_path_buf());
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let why = d.read_with(cx, |d, cx| d.primary_blocked(cx));
    assert_eq!(why.as_deref(), Some("Choose a collection to update."));
    r.click(cx, "import-link-import");
    cx.run_until_parked();
    assert_eq!(phase(cx, &d), Phase::Idle);
}

#[gpui_kit::test]
fn the_result_lists_the_unchanged_items_under_their_heading(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    assert!(!s.kept.is_empty(), "the fixture keeps a request");
    r.click(cx, "import-update-apply");
    applied(cx, &d);
    frame(cx, r.win);
    let section = cx
        .update_window(r.win, |_, w, _| {
            w.find("import-update-section-unchanged")
                .bounds()
                .size
                .height
        })
        .unwrap();
    // A lone heading line is under 20px; each listed item adds a line below it.
    assert!(
        section > gpui_kit::px(30.),
        "the unchanged section lists its items under the heading: {section:?}"
    );
}
