//! The breadcrumb splits a request's path into folders and its display name.

use crate::support;

use apinata_app::request_editor::{crumb_tooltip, crumbs};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

#[test]
fn crumbs_split_folders_from_the_display_name() {
    assert_eq!(crumbs(&ip("a.yml"), "a"), (vec![], "a".to_string()));
    assert_eq!(
        crumbs(&ip("users/admin/list.yml"), "List"),
        (
            vec![
                ("users".to_string(), ip("users")),
                ("admin".to_string(), ip("users/admin")),
            ],
            "List".to_string()
        )
    );
    assert_eq!(
        crumbs(&ip("users/untitled-3.draft"), "Untitled request 3").1,
        "Untitled request 3"
    );
}

#[test]
fn the_tooltip_is_the_full_path() {
    assert_eq!(
        crumb_tooltip(&ip("users/admin/list.yml"), "List").as_deref(),
        Some("users/admin/list.yml")
    );
    assert_eq!(
        crumb_tooltip(&ip("users/untitled-3.draft"), "Untitled request 3").as_deref(),
        Some("users/Untitled request 3")
    );
    // A root draft's path is only its name, which the line already shows; a tooltip would
    // repeat it and cover the method selector below.
    assert_eq!(
        crumb_tooltip(&ip("untitled-3.draft"), "Untitled request 3"),
        None
    );
}

#[gpui_kit::test]
fn the_file_name_is_its_own_element(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List", "http://x/")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "users/list.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let line = w.find("breadcrumb").bounds();
        let file = w.find("breadcrumb-file").bounds();
        assert!(line.contains(&file.center()));
        assert!(file.left() > line.left(), "the folder comes first");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_folder_crumb_reveals_and_selects_its_folder(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/admin/folder.yml", &folder("admin")),
        ("users/admin/list.yml", &request("List", "http://x/")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "users/admin/list.yml");
    frame(cx, s.win);
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    assert!(!s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    cx.update_window(s.win, |_, w, cx| w.click(("breadcrumb-folder", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    let selected = s
        .shell
        .read_with(cx, |sh, cx| sh.sidebar().read(cx).selected(cx));
    assert_eq!(selected.map(|(_, p)| p), Some(ip("users")));
}

#[gpui_kit::test]
fn a_folder_crumb_is_at_least_20_px_tall(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List", "http://x/")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "users/list.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let crumb = w.find(("breadcrumb-folder", 0usize)).bounds();
        assert!(crumb.size.height >= gpui_kit::px(20.), "{crumb:?}");
    })
    .unwrap();
}

/// The kind of the active tab after the pane has settled.
fn active_kind(cx: &mut TestAppContext, s: &Started) -> Option<apinata_app::tabs::TabKind> {
    s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()))
}

fn nested() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/admin/folder.yml", &folder("admin")),
        ("users/admin/list.yml", &request("List", "http://x/")),
        ("top.yml", &request("Top", "http://x/")),
    ])
}

#[gpui_kit::test]
fn a_folder_crumb_opens_its_folder_settings(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let s = started(cx, &nested());
    open_tab(cx, &s.ws, &s.root, "users/admin/list.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(("breadcrumb-folder", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(active_kind(cx, &s), Some(TabKind::Folder(ip("users"))));
}

#[gpui_kit::test]
fn the_collection_crumb_opens_collection_settings(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let s = started(cx, &nested());
    open_tab(cx, &s.ws, &s.root, "users/admin/list.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("breadcrumb-collection", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(active_kind(cx, &s), Some(TabKind::Collection));
}

#[gpui_kit::test]
fn a_root_level_request_still_shows_the_collection_crumb(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let s = started(cx, &nested());
    open_tab(cx, &s.ws, &s.root, "top.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("breadcrumb-collection", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(active_kind(cx, &s), Some(TabKind::Collection));
}

#[gpui_kit::test]
fn the_file_crumb_is_not_clickable(cx: &mut TestAppContext) {
    let s = started(cx, &nested());
    open_tab(cx, &s.ws, &s.root, "users/admin/list.yml");
    frame(cx, s.win);
    let before = active_kind(cx, &s);
    let tabs = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
    cx.update_window(s.win, |_, w, cx| w.click("breadcrumb-file", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(active_kind(cx, &s), before);
    assert_eq!(s.ws.read_with(cx, |w, _| w.tabs().entries().len()), tabs);
}

/// The label text of a rendered breadcrumb button.
fn crumb_text(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: (&'static str, usize),
) -> String {
    cx.update_window(win, |_, w, _| w.find(id).label().map(str::to_owned))
        .unwrap()
        .unwrap_or_default()
}

#[gpui_kit::test]
fn a_folder_crumb_shows_the_folders_display_name(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("People")),
        ("users/admin/list.yml", &request("List", "http://x/")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "users/admin/list.yml", false);
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert_eq!(crumb_text(cx, win, ("breadcrumb-folder", 0)), "People");
    // No folder.yml for admin: the directory name stands in.
    assert_eq!(crumb_text(cx, win, ("breadcrumb-folder", 1)), "admin");
}

#[gpui_kit::test]
fn a_deep_path_stays_inside_a_narrow_pane_and_keeps_the_file_whole(cx: &mut TestAppContext) {
    use gpui_kit::{VisualTestContext, px, size};
    let long = "a-rather-long-folder-name";
    let rel = format!("{long}-1/{long}-2/{long}-3/{long}-4/list.yml");
    let mut files = vec![("opencollection.yml".to_string(), ROOT.to_string())];
    for n in 1..=4 {
        let dir = (1..=n)
            .map(|i| format!("{long}-{i}"))
            .collect::<Vec<_>>()
            .join("/");
        files.push((format!("{dir}/folder.yml"), folder(&format!("{long}-{n}"))));
    }
    files.push((rel.clone(), request("List", "http://x/")));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let col = write_tree(&refs);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, &rel, false);
    // The file's natural width, measured while there is room for everything.
    VisualTestContext::from_window(win, cx).simulate_resize(size(px(3000.), px(700.)));
    let (natural, folder_natural) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let folders: Vec<_> = (0..4usize)
                .map(|i| w.find(("breadcrumb-folder", i)).bounds().size.width)
                .collect();
            (w.find("breadcrumb-file").bounds().size.width, folders)
        })
        .unwrap();
    assert!(natural > px(0.));
    VisualTestContext::from_window(win, cx).simulate_resize(size(px(440.), px(700.)));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let pane = w.find("request-column").bounds();
        let line = w.find("breadcrumb").bounds();
        let file = w.find("breadcrumb-file").bounds();
        assert!(line.right() <= pane.right(), "{line:?} vs {pane:?}");
        assert!(file.right() <= pane.right(), "{file:?} vs {pane:?}");
        assert_eq!(file.size.width, natural, "the file name is whole");
        // The nearest folder is whole; the outer ones shrank but keep room for an ellipsis.
        let widths: Vec<_> = (0..4usize)
            .map(|i| w.find(("breadcrumb-folder", i)).bounds().size.width)
            .collect();
        assert_eq!(widths[3], folder_natural[3], "the nearest folder is whole");
        for i in 0..3 {
            assert!(widths[i] < folder_natural[i], "outer folder {i} shrank");
            assert!(
                widths[i] > px(16.),
                "outer folder {i} shows more than padding: {widths:?}"
            );
        }
        let last = w.find(("breadcrumb-folder", 3usize)).bounds();
        assert!(
            last.right() <= file.left(),
            "the nearest folder precedes the file"
        );
    })
    .unwrap();
}
