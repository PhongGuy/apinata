//! A relaunch on kept data shows each request file as it is on disk now, never an older
//! value from the previous run.

use crate::support;

use std::path::PathBuf;
use std::sync::Arc;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::draft_store::Drafts;
use apinata_app::persist::Paths;
use apinata_app::request_editor::Section;
use apinata_app::snapshot::DiskState;
use apinata_core::{MemoryStore, Param, ParamKind, SecretStore};
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use support::*;

const BASE: &str = "http://127.0.0.1:9/a";

// A request file whose URL and params block are written independently, as an outside editor may.
fn file(name: &str, url: &str, z: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  params:\n    - name: z\n      value: \"{z}\"\n      type: query\n"
    )
}

fn z_url(z: &str) -> String {
    format!("{BASE}?z={z}")
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &file("a", &z_url("1"), "1")),
    ])
}

// The first run: the real app over a fresh data dir, with `a.yml` open in a tab.
fn first_run(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    s
}

// Sets z in both the URL and the params table, the pair the editor sends for one change.
fn set_z(cx: &mut TestAppContext, s: &Started, item: &str, z: &str) {
    edit_item(cx, s, item, Edit::Url(z_url(z)));
    let params = vec![Param::new("z", z, ParamKind::Query)];
    edit_item(cx, s, item, Edit::Params(params));
    frame(cx, s.win);
}

// Closes the window the way the user does, discarding unsaved edits if asked, and hands back
// the data dir the run leaves behind.
pub(crate) fn close(cx: &mut TestAppContext, s: Started) -> tempfile::TempDir {
    wait_save(cx);
    // A clean window lets the platform close it, which the test platform leaves to us.
    if VisualTestContext::from_window(s.win, cx).simulate_close() {
        cx.update_window(s.win, |_, w, _| w.remove_window())
            .unwrap();
    } else {
        answer_confirm(cx, "confirm-discard");
    }
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 0, "the window closed");
    s.files
}

// `app::start` again over the data dir a previous run left, as a relaunch with kept data. A real
// quit ends the process and frees the drafts lock; here the first run's store is dropped by hand,
// or the relaunched store would start disabled and restore nothing.
fn relaunch(cx: &mut TestAppContext, files: tempfile::TempDir, root: PathBuf) -> Started {
    relaunch_with(cx, files, root, Arc::new(MemoryStore::new()))
}

// As `relaunch`, over the secret store `store`.
pub(crate) fn relaunch_with(
    cx: &mut TestAppContext,
    files: tempfile::TempDir,
    root: PathBuf,
    store: Arc<dyn SecretStore>,
) -> Started {
    use gpui_kit::test::TestWindowExt as _;
    cx.update(|cx| {
        apinata_app::draft_store::flush_now(None, cx);
        cx.remove_global::<Drafts>();
    });
    cx.run_until_parked();
    let paths = Paths::in_dir(files.path());
    let (win, shell) = cx
        .update(|cx| apinata_app::app::start(&paths, store, Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let ws = shell.read_with(cx, |s, _| s.workspace().clone());
    let enabled = cx.update(|cx| cx.global::<Drafts>().0.read(cx).enabled());
    assert!(enabled, "the relaunched store holds the drafts lock");
    Started {
        files,
        root,
        win,
        shell,
        ws,
    }
}

// What the user sees for the active tab: its item, the URL bar, the params table rows, the
// dirty flag, the disk state and whether the changed-on-disk banner shows.
#[derive(Debug, PartialEq)]
struct Shown {
    item: String,
    url: String,
    params: Vec<(String, String)>,
    dirty: bool,
    disk: DiskState,
    banner: bool,
}

fn shown(cx: &mut TestAppContext, s: &Started) -> Shown {
    frame(cx, s.win);
    let (tab, item) = s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("an active tab");
        (e.id, e.item().expect("a request").as_str().to_owned())
    });
    let ed = editor_of(cx, s, tab);
    let url = ed.read_with(cx, |e, cx| e.url(cx));
    let table = ed
        .read_with(cx, |e, _| e.table(Section::Params).cloned())
        .unwrap();
    let params = table.read_with(cx, |t, cx| {
        t.rows(cx).into_iter().map(|r| (r.name, r.value)).collect()
    });
    let view = view_of(cx, &s.ws, &s.root, &item);
    Shown {
        item,
        url,
        params,
        dirty: view.dirty,
        disk: view.disk,
        banner: present(cx, s.win, "banner-changed"),
    }
}

// Saves `a.yml` through the app after a relaunch: the file must still hold the newer z=6, never
// an older value the relaunch brought back.
fn save_keeps_newer_file(cx: &mut TestAppContext, s: &Started, path: &std::path::Path) {
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    wait_save(cx);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("z=6") && !text.contains("z=3"), "{text}");
}

fn clean(item: &str, url: &str, z: &str) -> Shown {
    Shown {
        item: item.into(),
        url: url.into(),
        params: vec![("z".into(), z.into())],
        dirty: false,
        disk: DiskState::Present,
        banner: false,
    }
}

#[gpui_kit::test]
fn a_saved_value_changed_outside_shows_the_new_value(cx: &mut TestAppContext) {
    let col = tree();
    let s = first_run(cx, &col);
    set_z(cx, &s, "a.yml", "3");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    cx.run_until_parked();
    assert_eq!(shown(cx, &s), clean("a.yml", &z_url("3"), "3"), "saved");
    let root = s.root.clone();
    let files = close(cx, s);
    let path = col.path().join("a.yml");
    std::fs::write(&path, file("a", &z_url("6"), "6")).unwrap();
    let s = relaunch(cx, files, root);
    assert_eq!(shown(cx, &s), clean("a.yml", &z_url("6"), "6"));
    save_keeps_newer_file(cx, &s, &path);
}

#[gpui_kit::test]
fn an_edit_discarded_at_close_never_returns_over_a_newer_file(cx: &mut TestAppContext) {
    let col = tree();
    let s = first_run(cx, &col);
    set_z(cx, &s, "a.yml", "3");
    assert!(shown(cx, &s).dirty, "an unsaved edit");
    let root = s.root.clone();
    let files = close(cx, s);
    let path = col.path().join("a.yml");
    assert!(std::fs::read_to_string(&path).unwrap().contains("z=1"));
    std::fs::write(&path, file("a", &z_url("6"), "6")).unwrap();
    let s = relaunch(cx, files, root);
    assert_eq!(shown(cx, &s), clean("a.yml", &z_url("6"), "6"));
    save_keeps_newer_file(cx, &s, &path);
}

#[gpui_kit::test]
fn a_changed_on_disk_banner_at_close_leaves_only_the_file(cx: &mut TestAppContext) {
    let col = tree();
    let s = first_run(cx, &col);
    set_z(cx, &s, "a.yml", "3");
    let path = col.path().join("a.yml");
    std::fs::write(&path, file("a", &z_url("6"), "6")).unwrap();
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Reload(vec![path.clone()]), cx)
    });
    let before = shown(cx, &s);
    assert_eq!(
        (before.dirty, before.disk, before.banner),
        (true, DiskState::Changed, true),
        "the draft z=3 sits over a newer file"
    );
    let root = s.root.clone();
    let files = close(cx, s);
    let s = relaunch(cx, files, root);
    assert_eq!(shown(cx, &s), clean("a.yml", &z_url("6"), "6"));
    save_keeps_newer_file(cx, &s, &path);
}

// The params table is filled from the file's params block, not by parsing the URL, so a file
// whose two disagree shows both exactly as stored.
#[gpui_kit::test]
fn a_url_changed_alone_outside_shows_the_file_as_stored(cx: &mut TestAppContext) {
    let col = tree();
    let s = first_run(cx, &col);
    let root = s.root.clone();
    let files = close(cx, s);
    std::fs::write(col.path().join("a.yml"), file("a", &z_url("6"), "3")).unwrap();
    let s = relaunch(cx, files, root);
    assert_eq!(shown(cx, &s), clean("a.yml", &z_url("6"), "3"));
}

#[gpui_kit::test]
fn a_saved_draft_comes_back_as_its_file_only(cx: &mut TestAppContext) {
    let col = tree();
    let s = first_run(cx, &col);
    let (draft, _) = new_draft_at(cx, &s, apinata_core::ItemPath::root());
    set_z(cx, &s, &draft, "3");
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    let handle = s.shell.read_with(cx, gpui_kit::Focusable::focus_handle);
    VisualTestContext::from_window(s.win, cx).update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.input("Saved", cx);
        w.press("enter", cx);
    })
    .unwrap();
    let saved = col.path().join("Saved.yml");
    wait_for(cx, "the save", |_| saved.exists());
    assert_eq!(shown(cx, &s), clean("Saved.yml", &z_url("3"), "3"));
    let root = s.root.clone();
    let files = close(cx, s);
    let drafts = Paths::in_dir(files.path()).drafts;
    assert!(
        apinata_app::draft_file::load_dir(&drafts).drafts.is_empty(),
        "no draft file is kept for a saved draft"
    );
    std::fs::write(&saved, file("Saved", &z_url("6"), "6")).unwrap();
    let s = relaunch(cx, files, root);
    let items = tab_names(cx, &s.ws);
    assert!(
        items.iter().all(|i| !ip(i).is_draft()) && items.contains(&"Saved.yml".to_owned()),
        "{items:?}"
    );
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("Saved.yml"), false, cx))
        .expect("the saved request");
    assert_eq!(shown(cx, &s), clean("Saved.yml", &z_url("6"), "6"));
}

// The control for the relaunch helper: a kept untitled draft does come back through it.
#[gpui_kit::test]
fn a_kept_draft_comes_back_through_the_same_relaunch(cx: &mut TestAppContext) {
    let col = tree();
    let s = first_run(cx, &col);
    let (draft, _) = new_draft_at(cx, &s, apinata_core::ItemPath::root());
    set_z(cx, &s, &draft, "3");
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    let root = s.root.clone();
    let files = close(cx, s);
    let s = relaunch(cx, files, root);
    let back = shown(cx, &s);
    assert!(ip(&back.item).is_draft(), "{back:?}");
    assert_eq!(
        (back.url, back.params),
        (z_url("3"), vec![("z".into(), "3".into())])
    );
}
