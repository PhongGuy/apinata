//! Scratch drafts survive a restart.

use crate::support;

use apinata_app::draft_file;
use apinata_app::persist::{Paths, save_json};
use apinata_core::{DraftSnapshot, ItemPath};
use gpui_kit::TestAppContext;
use support::*;

// A data directory as a previous run left it: settings and one kept scratch draft, no session.
fn data_dir_with_scratch_draft() -> tempfile::TempDir {
    let files = blank_data_dir();
    let paths = Paths::in_dir(files.path());
    let snap = DraftSnapshot::from_json(
        r#"{"info":{"name":"Kept","type":"http"},"http":{"method":"GET","url":"http://127.0.0.1:9/kept"}}"#,
    )
    .unwrap();
    let key = ItemPath::new("untitled-1760600000001.draft").unwrap();
    let file = draft_file::build(
        &key,
        &paths.scratch,
        &ItemPath::new("").unwrap(),
        &snap,
        false,
        &[],
    )
    .unwrap();
    save_json(&paths.drafts.join(draft_file::file_name(&key)), &file).unwrap();
    files
}

#[gpui_kit::test]
fn a_scratch_draft_returns_after_a_restart(cx: &mut TestAppContext) {
    let second = started_blank(cx, data_dir_with_scratch_draft());
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &second.ws);
    assert!(second.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    let view = second
        .ws
        .read_with(cx, |w, cx| w.request(&root, &item, cx))
        .unwrap();
    assert_eq!(view.url, "http://127.0.0.1:9/kept");
}

#[gpui_kit::test]
fn the_saved_session_never_names_the_scratch(cx: &mut TestAppContext) {
    // A real collection makes the session save certain; the scratch then opens beside it.
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (scratch, _) = active_draft(cx, &s.ws);
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&scratch)));
    let session = s.ws.read_with(cx, |w, _| w.session().clone());
    session.update(cx, |p, _| p.flush_now()).unwrap();

    let paths = Paths::in_dir(s.files.path());
    assert_eq!(scratch, paths.scratch);
    let live = session.read_with(cx, |p, _| p.get().clone());
    let file: apinata_app::session::Session = apinata_app::persist::load_json(&paths.session).value;
    for (which, saved) in [("in memory", live), ("on disk", file)] {
        assert!(
            saved.open.contains(&s.root),
            "{which}: the collection is saved: {saved:?}"
        );
        assert!(
            !saved.open.contains(&scratch) && !saved.recent.contains(&scratch),
            "{which}: {saved:?}"
        );
    }
}
