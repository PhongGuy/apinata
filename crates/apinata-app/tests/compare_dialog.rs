//! The Compare action on the changed-on-disk banner: a read-only diff of disk against draft,
//! with the banner's own Reload and Overwrite, in all three editors.

use crate::support;

use apinata_app::command::{Command, Edit, EnvOp, Target};
use apinata_app::compare_dialog::CompareDialog;
use apinata_app::config_editor::{ConfigEditor, ConfigTarget};
use apinata_app::workspace::Workspace;
use apinata_core::{Collection, MemoryStore, SecretStore};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use std::sync::Arc;
use support::*;

const SECRET: &str = "s3cr3t-value-9f2";
const ENV0: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn env_text(host: &str) -> String {
    format!(
        "name: dev\nvariables:\n  - name: host\n    value: {host}\n  - secret: true\n    name: token\n"
    )
}

type Getter = Box<dyn Fn(&mut TestAppContext) -> Option<Entity<CompareDialog>>>;

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: PathBuf,
    win: AnyWindowHandle,
    path: PathBuf,
    target: Target,
    mine: fn() -> Command,
    // Marker text only the draft holds, and the markers of two disk versions.
    draft_mark: &'static str,
    disk: [String; 2],
    compare: Getter,
}

fn request_rig(cx: &mut TestAppContext) -> Rig {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    Rig {
        path: col.path().join("a.yml"),
        target: Target::Request(ip("a.yml")),
        mine: || edit_url("a.yml", "http://x/mine"),
        draft_mark: "http://x/mine",
        disk: [
            request("a", "http://x/disk1"),
            request("a", "http://x/disk2"),
        ],
        compare: Box::new(move |cx| ed.read_with(cx, |e, _| e.compare())),
        _col: col,
        _state: state,
        ws,
        root,
        win,
    }
}

fn folder_rig(cx: &mut TestAppContext) -> Rig {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (w2, r2) = (ws.clone(), root.clone());
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(w2, r2, ConfigTarget::Folder(ip("users")), w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    Rig {
        path: col.path().join("users/folder.yml"),
        target: Target::Folder(ip("users")),
        mine: || Command::Edit {
            target: Target::Folder(ip("users")),
            edit: Edit::Docs("mine-docs".into()),
        },
        draft_mark: "mine-docs",
        disk: [folder("Disk1"), folder("Disk2")],
        compare: Box::new(move |cx| ed.read_with(cx, |e, _| e.compare())),
        _col: col,
        _state: state,
        ws,
        root,
        win,
    }
}

fn env_rig(cx: &mut TestAppContext) -> Rig {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV0)]);
    let store = Arc::new(MemoryStore::new());
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "token");
    store.set(&key, SECRET).unwrap();
    let (state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    Rig {
        path: col.path().join("environments/dev.yml"),
        target: Target::Environment("dev".into()),
        mine: || Command::Edit {
            target: Target::Environment("dev".into()),
            edit: Edit::Color(Some("#ff0000".into())),
        },
        draft_mark: "#ff0000",
        disk: [env_text("disk1"), env_text("disk2")],
        compare: Box::new(move |cx| ed.read_with(cx, |e, _| e.compare())),
        _col: col,
        _state: state,
        ws,
        root,
        win,
    }
}

impl Rig {
    // The draft holds an edit and the file then changes under it.
    fn changed(&self, cx: &mut TestAppContext, version: usize) {
        self.ws
            .update(cx, |w, cx| w.send(&self.root, (self.mine)(), cx));
        cx.run_until_parked();
        self.external(cx, version);
    }

    fn external(&self, cx: &mut TestAppContext, version: usize) {
        std::fs::write(&self.path, &self.disk[version]).unwrap();
        let path = self.path.clone();
        self.ws.update(cx, |w, cx| {
            w.send(&self.root, Command::Reload(vec![path]), cx)
        });
        cx.run_until_parked();
        frame(cx, self.win);
    }

    fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        let win = compare_window(cx).unwrap_or(self.win);
        cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
        cx.run_until_parked();
        frame(cx, self.win);
    }

    fn open_compare(&self, cx: &mut TestAppContext) -> Entity<CompareDialog> {
        self.click(cx, "banner-compare");
        assert!(compare_window(cx).is_some(), "the dialog is open");
        (self.compare)(cx).expect("the editor holds the dialog")
    }

    fn dirty(&self, cx: &mut TestAppContext) -> bool {
        let s = self
            .ws
            .read_with(cx, |w, cx| w.snapshot(&self.root, cx))
            .unwrap();
        match &self.target {
            Target::Request(p) => s.content.requests[p].dirty,
            Target::Folder(p) => s.content.folders[p].dirty,
            Target::Environment(n) => s
                .content
                .environments
                .iter()
                .any(|e| &e.name == n && e.dirty),
            Target::Collection => s.content.collection.dirty,
        }
    }
}

fn compare_window(cx: &mut TestAppContext) -> Option<AnyWindowHandle> {
    cx.run_until_parked();
    cx.windows().into_iter().find(|&win| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("compare-dialog").is_some()
        })
        .unwrap_or(false)
    })
}

macro_rules! each_editor {
    ($($name:ident,)*) => {
        mod request_editor {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) {
                let r = request_rig(cx);
                super::$name(cx, &r);
            })*
        }
        mod folder_editor {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) {
                let r = folder_rig(cx);
                super::$name(cx, &r);
            })*
        }
        mod env_editor {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) {
                let r = env_rig(cx);
                super::$name(cx, &r);
            })*
        }
    };
}

each_editor! {
    compare_shows_only_for_a_changed_file,
    opening_it_changes_nothing,
    the_diff_holds_disk_and_draft_text,
    reload_from_the_dialog_takes_the_disk,
    overwrite_from_the_dialog_writes_the_draft,
    close_leaves_everything_as_it_was,
    a_second_external_change_refreshes_the_diff,
}

fn compare_shows_only_for_a_changed_file(cx: &mut TestAppContext, r: &Rig) {
    assert!(!present(cx, r.win, "banner-compare"), "clean");
    r.changed(cx, 0);
    assert!(present(cx, r.win, "banner-changed"));
    assert!(present(cx, r.win, "banner-compare"));
    // A deleted file has a banner of its own, with no Compare.
    std::fs::remove_file(&r.path).unwrap();
    let path = r.path.clone();
    r.ws.update(cx, |w, cx| w.send(&r.root, Command::Reload(vec![path]), cx));
    cx.run_until_parked();
    frame(cx, r.win);
    assert!(present(cx, r.win, "banner-deleted"));
    assert!(!present(cx, r.win, "banner-compare"));
}

fn opening_it_changes_nothing(cx: &mut TestAppContext, r: &Rig) {
    r.changed(cx, 0);
    let before = std::fs::read_to_string(&r.path).unwrap();
    assert!(r.dirty(cx));
    r.open_compare(cx);
    assert_eq!(std::fs::read_to_string(&r.path).unwrap(), before);
    assert!(r.dirty(cx), "the draft is kept");
    assert!(present(cx, r.win, "banner-changed"));
}

fn the_diff_holds_disk_and_draft_text(cx: &mut TestAppContext, r: &Rig) {
    r.changed(cx, 0);
    let d = r.open_compare(cx);
    let (disk, draft) = d.read_with(cx, |d, _| (d.disk().to_owned(), d.draft().to_owned()));
    assert_eq!(disk, r.disk[0]);
    assert!(draft.contains(r.draft_mark), "{draft}");
    assert!(!disk.contains(r.draft_mark));
    let rows = d.read_with(cx, |d, _| d.rows().to_vec());
    let shown: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert!(shown.iter().any(|t| t.contains(r.draft_mark)), "{shown:?}");
    let win = compare_window(cx).unwrap();
    assert!(present(cx, win, ("diff-line", 0usize)));
}

fn reload_from_the_dialog_takes_the_disk(cx: &mut TestAppContext, r: &Rig) {
    r.changed(cx, 0);
    let before = std::fs::read_to_string(&r.path).unwrap();
    r.open_compare(cx);
    r.click(cx, "compare-reload");
    assert!(compare_window(cx).is_none(), "the dialog closed");
    assert!(!present(cx, r.win, "banner-changed"));
    assert!(!r.dirty(cx));
    assert_eq!(std::fs::read_to_string(&r.path).unwrap(), before);
}

fn overwrite_from_the_dialog_writes_the_draft(cx: &mut TestAppContext, r: &Rig) {
    r.changed(cx, 0);
    r.open_compare(cx);
    r.click(cx, "compare-overwrite");
    assert!(compare_window(cx).is_none(), "the dialog closed");
    assert!(!present(cx, r.win, "banner-changed"));
    let disk = std::fs::read_to_string(&r.path).unwrap();
    assert!(disk.contains(r.draft_mark), "{disk}");
}

fn close_leaves_everything_as_it_was(cx: &mut TestAppContext, r: &Rig) {
    r.changed(cx, 0);
    r.open_compare(cx);
    r.click(cx, "compare-close");
    assert!(compare_window(cx).is_none());
    assert!(present(cx, r.win, "banner-changed"));
    assert!(r.dirty(cx));
    assert_eq!(std::fs::read_to_string(&r.path).unwrap(), r.disk[0]);
}

fn a_second_external_change_refreshes_the_diff(cx: &mut TestAppContext, r: &Rig) {
    r.changed(cx, 0);
    let d = r.open_compare(cx);
    r.external(cx, 1);
    assert!(compare_window(cx).is_some(), "still open");
    let disk = d.read_with(cx, |d, _| d.disk().to_owned());
    assert_eq!(disk, r.disk[1]);
}

#[gpui_kit::test]
fn a_secret_value_never_appears_in_the_diff(cx: &mut TestAppContext) {
    let r = env_rig(cx);
    // The value is in the store; the file and the draft name the variable only.
    r.ws.update(cx, |w, cx| {
        w.send(
            &r.root,
            Command::Env(EnvOp::SetSecretValue {
                env: "dev".into(),
                var: "token".into(),
                value: SECRET.into(),
            }),
            cx,
        )
    });
    cx.run_until_parked();
    r.changed(cx, 0);
    let d = r.open_compare(cx);
    let (disk, draft, rows) = d.read_with(cx, |d, _| {
        (d.disk().to_owned(), d.draft().to_owned(), d.rows().to_vec())
    });
    assert!(draft.contains("token"), "{draft}");
    for text in [&disk, &draft] {
        assert!(!text.contains(SECRET), "{text}");
    }
    assert!(rows.iter().all(|r| !r.text.contains(SECRET)));
}
