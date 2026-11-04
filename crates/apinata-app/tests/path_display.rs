//! The shared path display: home as "~", middle elision, and its three call sites.

use crate::support::{self, *};

use std::path::{Path, PathBuf};

use apinata_app::clone_dialog;
use apinata_app::path_display::{display, tilde};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, WindowOptions};

// One unit per character, so widths are exact in the pure tests.
fn chars(s: &str) -> f32 {
    s.chars().count() as f32
}

#[test]
fn home_is_abbreviated_to_a_tilde() {
    let home = Path::new("/home/dan");
    assert_eq!(
        tilde(Path::new("/home/dan/work/shop"), Some(home)),
        "~/work/shop"
    );
    assert_eq!(tilde(Path::new("/home/dan"), Some(home)), "~");
    assert_eq!(
        tilde(Path::new("/home/daniel/x"), Some(home)),
        "/home/daniel/x"
    );
    assert_eq!(tilde(Path::new("/srv/shop"), None), "/srv/shop");
}

#[test]
fn a_path_that_fits_is_shown_whole() {
    let home = Path::new("/home/dan");
    let shown = display(Path::new("/home/dan/work/shop"), Some(home), 40., &chars);
    assert_eq!(shown, "~/work/shop");
}

#[test]
fn the_middle_is_elided_and_the_last_two_components_stay() {
    let home = Path::new("/home/dan");
    let path = Path::new("/home/dan/work/deep/deeper/client-b/shop");
    let shown = display(path, Some(home), 24., &chars);
    assert_eq!(shown, "~/work/…/client-b/shop");
    assert!(chars(&shown) <= 24.);
}

#[test]
fn the_tail_is_kept_when_only_it_fits() {
    let path = Path::new("/srv/aaaaaaaaaa/bbbbbbbbbb/client-b/shop");
    let shown = display(path, None, 17., &chars);
    assert_eq!(shown, "/…/client-b/shop");
}

#[test]
fn a_long_final_component_is_never_cut_while_it_fits() {
    let path = Path::new("/srv/aaaaaaaaaa/a-rather-long-collection-name");
    let shown = display(path, None, 30., &chars);
    assert!(shown.ends_with("a-rather-long-collection-name"), "{shown}");
    assert!(chars(&shown) <= 30.);
}

#[test]
fn a_final_component_wider_than_the_room_is_cut_at_its_end() {
    let path = Path::new("/srv/x/an-extremely-long-collection-folder-name");
    let shown = display(path, None, 12., &chars);
    assert_eq!(chars(&shown), 12.);
    assert!(shown.starts_with("an-extremel"), "{shown}");
    assert!(shown.ends_with('…'));
}

fn deep_collection(root: &Path, parent: &str, leaf: &str) -> PathBuf {
    let dir = root.join(parent).join("client").join(leaf);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: Shop\n",
    )
    .unwrap();
    dir
}

const LONG: &str = "an-unreasonably-long-parent-folder-name-for-the-sake-of-width";

fn shell(
    cx: &mut TestAppContext,
    open: Option<PathBuf>,
    recent: Vec<PathBuf>,
    home: PathBuf,
) -> AnyWindowHandle {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(
        cx,
        &state,
        Session {
            recent,
            ..Session::default()
        },
    );
    let ws = workspace(cx, &session);
    if let Some(root) = open {
        ws.update(cx, |w, cx| w.open(root, cx));
    }
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    cx.update(|cx| view.update(cx, |s, _| s.set_home(Some(home))));
    frame(cx, win);
    win
}

fn value(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: (&'static str, usize),
) -> Option<String> {
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.value().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn two_shop_collections_show_different_switcher_text(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let a = deep_collection(tmp.path(), LONG, "shop-a");
    let b = deep_collection(tmp.path(), LONG, "shop-b");
    let win = shell(cx, Some(a), vec![b], tmp.path().to_path_buf());
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, win);
    let open = value(cx, win, ("switcher-open-path", 0)).expect("open path text");
    let recent = value(cx, win, ("recent-path", 0)).expect("recent path text");
    assert_ne!(open, recent);
    assert!(open.starts_with("~/") && open.contains('…'), "{open}");
    assert!(open.ends_with("client/shop-a"), "{open}");
    assert!(recent.ends_with("client/shop-b"), "{recent}");
}

#[gpui_kit::test]
fn the_empty_state_recents_use_the_helper(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let b = deep_collection(tmp.path(), LONG, "shop-b");
    let win = shell(cx, None, vec![b.clone()], tmp.path().to_path_buf());
    let shown = value(cx, win, ("recent-path", 0)).expect("recent path text");
    assert!(shown.starts_with("~/") && shown.contains('…'), "{shown}");
    assert!(shown.ends_with("client/shop-b"), "{shown}");
    let full = cx
        .update_window(win, |_, w, _| {
            w.find(("recent-path", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(full, Some(b.display().to_string()));
}

#[gpui_kit::test]
fn the_clone_dialog_folder_uses_the_helper(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join(LONG).join("client").join("shop-b");
    std::fs::create_dir_all(&parent).unwrap();
    let col = collection();
    let s = started(cx, &col);
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| clone_dialog::open(&ws, w, cx))
        .unwrap();
    let home = tmp.path().to_path_buf();
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_home(Some(home));
            v.set_parent(parent.clone(), cx);
        })
    });
    frame(cx, s.win);
    let (shown, full) = cx
        .update_window(s.win, |_, w, _| {
            let e = w.find("clone-parent");
            (e.value().map(str::to_owned), e.label().map(str::to_owned))
        })
        .unwrap();
    let shown = shown.expect("folder text");
    assert!(shown.starts_with("~/") && shown.contains('…'), "{shown}");
    assert!(shown.ends_with("client/shop-b"), "{shown}");
    assert_eq!(full, Some(parent.display().to_string()));
}
