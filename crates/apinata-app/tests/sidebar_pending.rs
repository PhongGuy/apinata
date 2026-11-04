//! The sidebar's tracked creates: one the collection can no longer answer never claims a
//! later row, so a reopened or acknowledged collection's creates do not focus or rename.

use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::sidebar::NewFolder;
use apinata_app::sidebar::menu::NEW_FOLDER_NAME;
use apinata_app::workspace::WorkspaceEvent;
use apinata_core::ItemPath;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

// Creates a folder named `name` at the root from the workspace, not the sidebar.
fn create_elsewhere(cx: &mut TestAppContext, h: &SidebarHarness, name: &str) {
    let op = TreeOp::CreateFolder {
        parent: ItemPath::root(),
        name: name.into(),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(op), cx));
    cx.run_until_parked();
}

fn renaming(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<(std::path::PathBuf, ItemPath)> {
    h.view.read_with(cx, |s, _| s.renaming())
}

// A New Folder in flight when its collection closes is never answered; after a reopen, whose
// command ids start over, another view's creates must not take its place.
#[gpui_kit::test]
fn a_create_lost_to_a_close_does_not_claim_a_create_after_reopening(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.new_folder(&NewFolder, w, cx));
        h.ws.update(cx, |ws, cx| ws.close(&h.root, cx));
    })
    .unwrap();
    cx.run_until_parked();
    h.ws.update(cx, |w, cx| w.open(h.root.clone(), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    for name in ["one", "two", "three", "four"] {
        create_elsewhere(cx, &h, name);
        frame(cx, h.win);
        assert_eq!(renaming(cx, &h), None, "after creating {name}");
    }
}

// A New Folder sent to a reopened collection still loading is answered after its first
// snapshot, so it still renames.
#[gpui_kit::test]
fn a_new_folder_sent_right_after_reopening_still_renames(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    h.ws.update(cx, |w, cx| w.close(&h.root, cx));
    cx.run_until_parked();
    cx.update_window(h.win, |_, w, cx| {
        h.ws.update(cx, |ws, cx| ws.open(h.root.clone(), cx));
        h.view.update(cx, |s, cx| s.new_folder(&NewFolder, w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join(NEW_FOLDER_NAME).is_dir());
    assert_eq!(
        renaming(cx, &h),
        Some((h.root.clone(), ip(NEW_FOLDER_NAME)))
    );
}

// Once a snapshot acknowledges a tracked create, a Created for that id arriving later is not
// this view's to focus.
#[gpui_kit::test]
fn an_acknowledged_create_does_not_claim_a_later_notice(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.new_folder(&NewFolder, w, cx));
        h.ws.update(cx, |ws, cx| {
            let generation = ws.collections()[0].generation();
            cx.emit(WorkspaceEvent::Delivered {
                root: h.root.clone(),
                acked: u64::MAX,
                generation,
            })
        });
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join(NEW_FOLDER_NAME).is_dir());
    assert_eq!(renaming(cx, &h), None);
}

// A close and reopen inside the update that sent the New Folder leaves the root open when the
// sidebar next looks, so only the instance it was sent to can tell the stale create apart.
#[gpui_kit::test]
fn a_create_lost_to_a_close_and_reopen_in_one_update_does_not_claim_a_later_create(
    cx: &mut TestAppContext,
) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.new_folder(&NewFolder, w, cx));
        h.ws.update(cx, |ws, cx| {
            ws.close(&h.root, cx);
            ws.open(h.root.clone(), cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    for name in ["one", "two", "three", "four", "five", "six"] {
        create_elsewhere(cx, &h, name);
        frame(cx, h.win);
        assert_eq!(renaming(cx, &h), None, "after creating {name}");
    }
}

// Every file under `dir`, relative and sorted, so a test can tell nothing moved.
fn files_under(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(at) = todo.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                let rel = path.strip_prefix(dir).unwrap();
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

fn clipboard_len(cx: &mut TestAppContext, h: &SidebarHarness) -> usize {
    h.view.read_with(cx, |s, _| s.clipboard_len())
}

// A cut move sent to an instance closed and reopened in the same update is never answered,
// and the cut is bound to that instance, so it leaves the clipboard; the reopened instance's
// paste after the reopen moves nothing.
#[gpui_kit::test]
fn a_cut_move_lost_to_a_close_and_reopen_in_one_update_drops_the_cut(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.cut_selection(w, cx));
        h.view
            .update(cx, |s, cx| s.paste(&h.root, ip("users"), w, cx));
        h.ws.update(cx, |ws, cx| {
            ws.close(&h.root, cx);
            ws.open(h.root.clone(), cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert_eq!(clipboard_len(cx, &h), 0);
    let before = files_under(&h.root);
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.paste(&h.root, ip("users"), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert_eq!(files_under(&h.root), before);
}

// Sends a create to the open instance, then reopens the root and sends a New Folder from the
// sidebar to the new instance at the moment the old one's snapshot lands: the workspace
// notifies before it emits, so the old instance's Notices and Delivered reach the sidebar
// only after the reopen. `pad` sends filler creates first so the New Folder's id equals the
// old create's id.
fn reopen_as_the_old_instance_answers(
    cx: &mut TestAppContext,
    h: &SidebarHarness,
    pad: bool,
) -> gpui_kit::Subscription {
    for name in ["one", "two", "three"] {
        create_elsewhere(cx, h, name);
    }
    let op = TreeOp::CreateFolder {
        parent: ItemPath::root(),
        name: "old".into(),
    };
    let old =
        h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(op), cx));
    let old = old.unwrap();
    let (root, win, view) = (h.root.clone(), h.win, h.view.clone());
    let done = std::cell::Cell::new(false);
    cx.update(|cx| {
        cx.observe(&h.ws, move |ws, cx| {
            let acked = ws.read(cx).snapshot(&root, cx).map_or(0, |s| s.acked);
            if done.get() || acked < old {
                return;
            }
            done.set(true);
            ws.update(cx, |ws, cx| {
                ws.close(&root, cx);
                ws.open(root.clone(), cx);
                let mut n = 0;
                while pad && n + 1 < old {
                    let op = TreeOp::CreateFolder {
                        parent: ItemPath::root(),
                        name: format!("pad{n}"),
                    };
                    n = ws.send(&root, Command::Tree(op), cx).unwrap();
                }
                assert!(
                    !pad || n + 1 == old,
                    "the New Folder reuses the old id {old}"
                );
            });
            win.update(cx, |_, w, cx| {
                view.update(cx, |s, cx| s.new_folder(&NewFolder, w, cx))
            })
            .unwrap();
        })
    })
}

// The old instance's Delivered acknowledges ids the reopened instance reuses, so judged by
// the instance open when it arrives it would drop the New Folder sent to the new one.
#[gpui_kit::test]
fn a_delivered_of_the_old_instance_keeps_the_reopened_instances_create(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let _reopen = reopen_as_the_old_instance_answers(cx, &h, false);
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join("old").is_dir());
    assert!(h.root.join(NEW_FOLDER_NAME).is_dir());
    assert_eq!(
        renaming(cx, &h),
        Some((h.root.clone(), ip(NEW_FOLDER_NAME)))
    );
}

// The old instance's Created carries the same id as the New Folder sent to the reopened
// instance; it must not take that create's focus and rename.
#[gpui_kit::test]
fn notices_of_the_old_instance_do_not_match_the_reopened_instances_create(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let _reopen = reopen_as_the_old_instance_answers(cx, &h, true);
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join(NEW_FOLDER_NAME).is_dir());
    assert_eq!(
        renaming(cx, &h),
        Some((h.root.clone(), ip(NEW_FOLDER_NAME)))
    );
}

// Emits `notices` for `root` as the instance of `generation` would.
fn emit_notices(
    cx: &mut gpui_kit::App,
    h: &SidebarHarness,
    generation: u64,
    notices: Vec<apinata_app::command::Notice>,
) {
    h.ws.update(cx, |_, cx| {
        cx.emit(WorkspaceEvent::Notices {
            root: h.root.clone(),
            notices,
            generation,
        })
    });
}

fn generation_of(cx: &mut TestAppContext, h: &SidebarHarness) -> u64 {
    h.ws.read_with(cx, |w, _| w.collections()[0].generation())
}

// A batch belongs to the instance it was sent to: after a reopen in one update, the old
// instance's answer for an id the new batch reuses neither finishes the new batch nor keeps
// the old one, which can never be answered now.
#[gpui_kit::test]
fn batches_of_a_reopened_root_answer_only_to_their_own_instance(cx: &mut TestAppContext) {
    use apinata_app::command::{Notice, NoticeKind};
    use apinata_app::sidebar::batch::{Batch, Verb};
    let h = open_sidebar(cx);
    let old = generation_of(cx, &h);
    cx.update_window(h.win, |_, _, cx| {
        let sent = || vec![(1, ip("health.yml"))];
        h.view.update(cx, |s, _| {
            s.push_batch(Batch::new(Verb::Delete, h.root.clone(), old, sent()))
        });
        h.ws.update(cx, |ws, cx| {
            ws.close(&h.root, cx);
            ws.open(h.root.clone(), cx);
        });
        let new = h.ws.read(cx).collections()[0].generation();
        h.view.update(cx, |s, _| {
            s.push_batch(Batch::new(Verb::Delete, h.root.clone(), new, sent()))
        });
        let answer = Notice {
            command: 1,
            kind: NoticeKind::Deleted(ip("health.yml")),
        };
        emit_notices(cx, &h, old, vec![answer]);
    })
    .unwrap();
    assert_eq!(
        h.view.read_with(cx, |s, _| s.batch_roots()),
        std::slice::from_ref(&h.root),
        "only the new instance's batch is left, still waiting"
    );
}

// A Moved notice is only sent once the move is on disk, so the reopened instance shows the
// moved folder too: a notice that arrives after the reopen still carries the expansion and
// the selection to the new path.
#[gpui_kit::test]
fn a_move_answered_after_a_reopen_keeps_the_folder_expanded_and_selected(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    frame(cx, h.win);
    assert!(
        h.view
            .read_with(cx, |s, _| s.expanded_paths(&h.root))
            .contains(&ip("users"))
    );
    let old = generation_of(cx, &h);
    // The workspace notifies before it emits notices, so reopening as the rename's answer
    // lands hands the sidebar its Moved only after the root is open as another instance.
    let root = h.root.clone();
    let before =
        h.ws.read_with(cx, |w, cx| w.snapshot(&root, cx).unwrap().acked);
    let done = std::cell::Cell::new(false);
    let _reopen = cx.update(|cx| {
        cx.observe(&h.ws, move |ws, cx| {
            let acked = ws.read(cx).snapshot(&root, cx).map_or(0, |s| s.acked);
            if done.get() || acked <= before {
                return;
            }
            done.set(true);
            ws.update(cx, |ws, cx| {
                ws.close(&root, cx);
                ws.open(root.clone(), cx);
            });
        })
    });
    let (view, root) = (h.view.clone(), h.root.clone());
    cx.update_window(h.win, |_, w, cx| {
        view.update(cx, |s, cx| s.start_rename(&root, &ip("users"), w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.input("people", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join("people/list.yml").exists());
    assert_ne!(generation_of(cx, &h), old, "the root reopened");
    let expanded = h.view.read_with(cx, |s, _| s.expanded_paths(&h.root));
    assert!(expanded.contains(&ip("people")), "{expanded:?}");
    assert!(!expanded.contains(&ip("users")), "{expanded:?}");
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("people")))
    );
}

// An example delete of the closed instance shifts indices of rows it no longer shows, so the
// reopened view's rename of a later example goes on.
#[gpui_kit::test]
fn an_example_delete_of_the_old_instance_changes_nothing(cx: &mut TestAppContext) {
    use apinata_app::command::{Notice, NoticeKind};
    use apinata_app::sidebar::RenameSelected;
    use gpui_kit::test::TestWindowExt as _;
    const THREE: &str = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: a\n    response:\n      status: 200\n  - name: b\n    response:\n      status: 201\n  - name: c\n    response:\n      status: 202\n";
    let h = open_sidebar_with(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_owned()),
            ("users.yml", THREE.to_owned()),
        ],
    );
    let old = generation_of(cx, &h);
    h.ws.update(cx, |ws, cx| {
        ws.close(&h.root, cx);
        ws.open(h.root.clone(), cx);
    });
    cx.run_until_parked();
    frame(cx, h.win);
    let toggle = apinata_app::sidebar::marker_id("examples-toggle", &row("users.yml"));
    cx.update_window(h.win, |_, w, cx| w.click(toggle, cx))
        .unwrap();
    frame(cx, h.win);
    let last = apinata_app::sidebar::model::example_id(0, &ip("users.yml"), 2);
    cx.update_window(h.win, |_, w, cx| w.click(last, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.rename_selected(&RenameSelected, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    assert_eq!(renaming(cx, &h), Some((h.root.clone(), ip("users.yml"))));
    cx.update_window(h.win, |_, _, cx| {
        let deleted = Notice {
            command: 1,
            kind: NoticeKind::ExampleDeleted {
                item: ip("users.yml"),
                index: 0,
            },
        };
        emit_notices(cx, &h, old, vec![deleted]);
    })
    .unwrap();
    frame(cx, h.win);
    assert_eq!(renaming(cx, &h), Some((h.root.clone(), ip("users.yml"))));
}
