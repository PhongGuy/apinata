use crate::support;

use apinata_app::palette::{Availability, index_path, palette_items, palette_items_for};
use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::multi::{SelKey, run_count, run_items};
use apinata_app::sidebar::{RunSelected, SidebarEvent};
use apinata_app::snapshot::SnapshotCache;
use apinata_app::tabs::TabKind;
use apinata_core::Collection;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

#[test]
fn run_count_expands_folders_from_the_snapshot_rows() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", "http://x/x")),
        ("f/g/folder.yml", &folder("G")),
        ("f/g/y.yml", &request("Y", "http://x/y")),
        ("a.yml", &request("A", "http://x/a")),
        ("w.yml", SOCKET),
    ]);
    let c = SnapshotCache::new().build(&Collection::open(dir.path()).unwrap());
    let k = |p: &str| SelKey::new(dir.path(), ip(p));
    assert_eq!(run_count(&c.rows, &[k("a.yml")]), 1);
    assert_eq!(run_count(&c.rows, &[k("f")]), 2);
    assert_eq!(run_count(&c.rows, &[k("f"), k("a.yml")]), 3);
    assert_eq!(run_count(&c.rows, &[k("w.yml")]), 0);
}

#[gpui_kit::test]
fn run_selected_emits_the_normalised_items_without_opaque_ones(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    let seen = sidebar_events(cx, &h.view);
    cx.update_window(h.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap(); // expands f
    secondary_click(cx, h.win, row("f/x.yml")); // set: f and its child
    secondary_click(cx, h.win, row("w.yml"));
    secondary_click(cx, h.win, row("a.yml"));
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.run_selected(&RunSelected, w, cx))
    })
    .unwrap();
    let runs: Vec<_> = seen
        .borrow()
        .iter()
        .filter_map(|e| match e {
            SidebarEvent::RunSelection { items, .. } => Some(items.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 1);
    let mut items = runs[0].clone();
    items.sort();
    assert_eq!(
        items,
        [ip("a.yml"), ip("f")],
        "parent wins and the opaque member is stripped"
    );
}

#[gpui_kit::test]
fn run_selected_does_nothing_without_an_active_set(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    let seen = sidebar_events(cx, &h.view);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.run_selected(&RunSelected, w, cx))
    })
    .unwrap();
    assert!(
        !seen
            .borrow()
            .iter()
            .any(|e| matches!(e, SidebarEvent::RunSelection { .. }))
    );
}

#[gpui_kit::test]
fn run_selected_does_nothing_after_the_set_shrinks_to_one_key(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    let seen = sidebar_events(cx, &h.view);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("w.yml")); // two keys
    secondary_click(cx, h.win, row("w.yml")); // deselect: one key left
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.run_selected(&RunSelected, w, cx))
    })
    .unwrap();
    assert!(
        !seen
            .borrow()
            .iter()
            .any(|e| matches!(e, SidebarEvent::RunSelection { .. }))
    );
}

#[gpui_kit::test]
fn the_context_menu_runs_the_selection_in_a_new_tab(cx: &mut TestAppContext) {
    let col = write_tree(
        &multi_files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("a.yml")); // f and a.yml: 1 + 1 requests
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(row("a.yml"), cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    frame(cx, s.win);
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(
        kind,
        Some(TabKind::RunSelection(vec![ip("a.yml"), ip("f")]))
    );
}

// Whether the open palette, filtered to the command, lists "Run selected requests".
fn palette_lists_run_selected(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("Run selected", cx))
        .unwrap();
    frame(cx, s.win);
    let found = open_palette_lists_run_selected(cx, s);
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    found
}

// The same check against a palette that is already open and filtered.
fn open_palette_lists_run_selected(cx: &mut TestAppContext, s: &Started) -> bool {
    // With no tab open the palette also leaves out the tab commands, and the selection decides
    // the sidebar ones; both shift the rows.
    let offers = |cx: &mut TestAppContext, entry| {
        let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
        sidebar.read_with(cx, |sb, cx| sb.selection_offers(entry, cx))
    };
    let (rename, duplicate, show) = (
        offers(cx, MenuEntry::Rename),
        offers(cx, MenuEntry::CloneItem),
        offers(cx, MenuEntry::ShowInFileManager),
    );
    // A request row opens a request tab; a folder or collection row opens a settings tab,
    // which hides the request-only commands.
    let (has_tabs, request_tab) = s.ws.read_with(cx, |w, _| {
        let active = w.tabs().active();
        (active.is_some(), active.is_some_and(|t| t.item().is_some()))
    });
    let items = palette_items_for(Availability {
        has_tabs,
        request_tab,
        // Every request here is a saved HTTP file, so only Convert to GraphQL applies.
        to_graphql: request_tab,
        to_http: false,
        rename,
        duplicate,
        show_in_file_manager: show,
        close_saved: false,
        multi_tab: false,
        not_last: false,
        close_others: false,
        reopen_closed: false,
        stop: false,
        discard: false,
        save_all: false,
        beautify: false,
        beautify_query: false,
        undo_delete: false,
        ..Availability::all()
    });
    let ix = items
        .iter()
        .position(|e| e.label == "Run selected requests")
        .expect("the registry keeps the command");
    cx.update_window(s.win, |_, w, _| {
        w.within("command-palette")
            .try_find(index_path(&items, ix))
            .is_some()
    })
    .unwrap()
}

// A mouse click lands on the dialog's overlay and closes the palette, so the selection
// changes the way a file-watch update would: through the sidebar, with the dialog still up.
fn toggle_row_under_palette(cx: &mut TestAppContext, s: &Started, path: &str) {
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    cx.update_window(s.win, |_, w, cx| {
        sidebar.update(cx, |sb, cx| sb.modified_click(&row(path), true, w, cx))
    })
    .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn an_open_palette_follows_the_selection_as_it_changes(cx: &mut TestAppContext) {
    let col = write_tree(
        &multi_files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("Run selected", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(
        !open_palette_lists_run_selected(cx, &s),
        "one row hides the command"
    );
    toggle_row_under_palette(cx, &s, "b.yml");
    assert!(
        open_palette_lists_run_selected(cx, &s),
        "the open palette lists it once several rows are selected"
    );
    toggle_row_under_palette(cx, &s, "b.yml");
    assert!(
        !open_palette_lists_run_selected(cx, &s),
        "and drops it when the selection collapses"
    );
}

#[gpui_kit::test]
fn the_palette_lists_and_runs_the_selection_only_with_several_rows(cx: &mut TestAppContext) {
    let col = write_tree(
        &multi_files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == "Run selected requests")
        .expect("listed");
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    assert!(
        !palette_lists_run_selected(cx, &s),
        "one row hides the command"
    );
    secondary_click(cx, s.win, row("b.yml"));
    assert!(
        palette_lists_run_selected(cx, &s),
        "a multi-selection lists it"
    );
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(
        kind,
        Some(TabKind::RunSelection(vec![ip("a.yml"), ip("b.yml")]))
    );
}

#[test]
fn run_items_strips_the_root_and_opaque_rows_and_keeps_sidebar_order() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", "http://x/x")),
        ("a.yml", &request("A", "http://x/a")),
        ("w.yml", SOCKET),
    ]);
    let c = SnapshotCache::new().build(&Collection::open(dir.path()).unwrap());
    let k = |p: &str| SelKey::new(dir.path(), ip(p));
    let root = SelKey::new(dir.path(), apinata_core::ItemPath::root());
    let keys = [k("w.yml"), k("a.yml"), root.clone(), k("f")];
    assert_eq!(run_items(&c.rows, &keys), [ip("f"), ip("a.yml")]);
    assert!(run_items(&c.rows, &[k("w.yml")]).is_empty());
    assert!(
        run_items(&c.rows, &[root]).is_empty(),
        "the root never plans everything"
    );
}

#[gpui_kit::test]
fn a_started_selection_run_hits_the_server_in_sidebar_order(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let uri = server.uri();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", &format!("{uri}/x"))),
        ("a.yml", &request("A", &format!("{uri}/a"))),
        ("b.yml", &request("B", &format!("{uri}/b"))),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    // Given out of order; the run follows the tree, and the unselected b.yml stays out.
    let m = cx.new(|_| RunnerModel::for_items(ws, root, vec![ip("a.yml"), ip("f")]));
    m.update(cx, |m, cx| assert!(m.start(cx)));
    wait_real(cx, |cx| {
        m.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    assert_eq!(served(cx, &server), ["/x", "/a"]);
}

// With one row selected the command is offered for a folder or the collection, which it runs,
// and stays hidden for a request or an opaque item, which it would not.
fn one_row_lists_run_selected(cx: &mut TestAppContext, path: &str) -> bool {
    let col = write_tree(
        &multi_files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    frame(cx, s.win);
    palette_lists_run_selected(cx, &s)
}

#[gpui_kit::test]
fn the_palette_lists_run_selected_for_one_folder_row(cx: &mut TestAppContext) {
    assert!(one_row_lists_run_selected(cx, "f"));
}

#[gpui_kit::test]
fn the_palette_lists_run_selected_for_the_collection_row(cx: &mut TestAppContext) {
    assert!(one_row_lists_run_selected(cx, ""));
}

#[gpui_kit::test]
fn the_palette_hides_run_selected_for_one_request_row(cx: &mut TestAppContext) {
    assert!(!one_row_lists_run_selected(cx, "a.yml"));
}

#[gpui_kit::test]
fn the_palette_hides_run_selected_for_one_opaque_row(cx: &mut TestAppContext) {
    assert!(!one_row_lists_run_selected(cx, "w.yml"));
}
