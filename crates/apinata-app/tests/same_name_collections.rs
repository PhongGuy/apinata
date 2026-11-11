//! Two open collections that share a name stay apart in the window: their runner and
//! environment tabs and the breadcrumb carry the folder hint, and the sidebar root keeps its
//! name whole while the hint gives way.

use crate::support::{self, *};

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::sidebar::{marker_id, model::node_id};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Bounds, Entity, Pixels, TestAppContext};

struct Twins {
    _tmp: tempfile::TempDir,
    _state: tempfile::TempDir,
    roots: [std::path::PathBuf; 2],
    ws: Entity<apinata_app::workspace::Workspace>,
    win: AnyWindowHandle,
}

// The folders `demo` and `demo-twin`, both named "Apinata Demo", open in one shell.
fn twins(cx: &mut TestAppContext, sidebar_width: Option<f32>) -> Twins {
    let tmp = tempfile::tempdir().unwrap();
    let roots = [tmp.path().join("demo"), tmp.path().join("demo-twin")];
    for dir in &roots {
        std::fs::create_dir_all(dir.join("users")).unwrap();
        std::fs::write(
            dir.join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: Apinata Demo\n",
        )
        .unwrap();
        std::fs::write(dir.join("users/folder.yml"), folder("Users")).unwrap();
        std::fs::write(dir.join("users/get.yml"), request("Get user", "http://x/")).unwrap();
    }
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let start = Session {
        sidebar_width,
        ..Session::default()
    };
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    for root in &roots {
        ws.update(cx, |w, cx| w.open(root.clone(), cx));
    }
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    Twins {
        _tmp: tmp,
        _state: state,
        roots,
        ws,
        win,
    }
}

fn bounds(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

#[gpui_kit::test]
fn runner_and_environment_tabs_of_twins_show_their_folders(cx: &mut TestAppContext) {
    let t = twins(cx, None);
    for root in &t.roots {
        t.ws.update(cx, |w, cx| w.open_runner(root, ItemPath::root(), cx))
            .expect("a runner tab");
    }
    for root in &t.roots {
        t.ws.update(cx, |w, cx| {
            w.open_settings(root, apinata_app::tabs::TabKind::Environments, cx)
        })
        .expect("an environments tab");
    }
    frame(cx, t.win);
    // Each pair's hints read "demo" and "demo-twin", so the second is the wider one.
    for (a, b) in [(0usize, 1usize), (2, 3)] {
        assert!(present(cx, t.win, ("tab-detail", a)), "tab {a} has a hint");
        assert!(present(cx, t.win, ("tab-detail", b)), "tab {b} has a hint");
        let (da, db) = (
            bounds(cx, t.win, ("tab-detail", a)),
            bounds(cx, t.win, ("tab-detail", b)),
        );
        assert!(db.size.width > da.size.width, "{da:?} {db:?}");
    }
}

#[gpui_kit::test]
fn a_lone_runner_tab_of_a_twin_still_names_its_folder(cx: &mut TestAppContext) {
    let t = twins(cx, None);
    t.ws.update(cx, |w, cx| w.open_runner(&t.roots[1], ItemPath::root(), cx))
        .expect("a runner tab");
    t.ws.update(cx, |w, cx| {
        w.open_settings(&t.roots[0], apinata_app::tabs::TabKind::Environments, cx)
    })
    .expect("an environments tab");
    frame(cx, t.win);
    for ix in [0usize, 1] {
        assert!(
            present(cx, t.win, ("tab-detail", ix)),
            "tab {ix} has a hint"
        );
    }
}

#[gpui_kit::test]
fn the_breadcrumb_of_a_twin_names_its_folder(cx: &mut TestAppContext) {
    let t = twins(cx, None);
    let mut widths = Vec::new();
    for root in &t.roots {
        open_tab(cx, &t.ws, root, "users/get.yml");
        frame(cx, t.win);
        let name = bounds(cx, t.win, "breadcrumb-collection");
        assert!(
            present(cx, t.win, "breadcrumb-collection-hint"),
            "the crumb shows where its collection lives"
        );
        let hint = bounds(cx, t.win, "breadcrumb-collection-hint");
        let folder = bounds(cx, t.win, ("breadcrumb-folder", 0usize));
        assert!(hint.left() >= name.right(), "the hint follows the name");
        assert!(hint.right() <= folder.left(), "before the first folder");
        widths.push(hint.size.width);
    }
    assert!(widths[1] > widths[0], "demo-twin reads longer: {widths:?}");
}

#[gpui_kit::test]
fn a_breadcrumb_of_a_lone_name_has_no_hint(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/get.yml", &request("Get user", "http://x/")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "users/get.yml");
    frame(cx, s.win);
    assert!(present(cx, s.win, "breadcrumb-collection"));
    assert!(!present(cx, s.win, "breadcrumb-collection-hint"));
}

// The sidebar's twin roots: each row's name box, its hint and its New request button.
fn root_row(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) -> [Bounds<Pixels>; 3] {
    let node = node_id(ix, &ItemPath::root());
    [
        bounds(cx, win, ("collection", ix)),
        bounds(cx, win, marker_id("hint", &node)),
        bounds(cx, win, ("new-request", ix)),
    ]
}

#[gpui_kit::test]
fn a_twin_root_keeps_its_name_and_the_hint_gives_way(cx: &mut TestAppContext) {
    for width in [None, Some(200.)] {
        let t = twins(cx, width);
        let gap = cx
            .update_window(t.win, |_, w, _| {
                apinata_app::style::SPACE_1.to_pixels(w.rem_size())
            })
            .unwrap();
        let [name0, hint0, new0] = root_row(cx, t.win, 0);
        let [name1, hint1, new1] = root_row(cx, t.win, 1);
        assert_eq!(
            name0.size.width, name1.size.width,
            "the same name gets the same room at {width:?}"
        );
        assert!(hint0.left() >= name0.right(), "{hint0:?} {name0:?}");
        for (hint, new) in [(hint0, new0), (hint1, new1)] {
            assert!(
                hint.right() + gap <= new.left(),
                "a gap before the icons at {width:?}: {hint:?} {new:?}"
            );
        }
    }
}
