//! Palette commands that would do nothing are left out: the Availability flags for Close
//! other tabs, Reopen closed tab, Stop, Discard changes, Save all, Beautify, the sidebar
//! selection commands, the tab-switching commands and the collection-scoped commands.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::palette::{
    Availability, PaletteGroup, collection_scoped_ids, index_path, palette_items_for,
};
use apinata_app::tabs::TabKind;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

fn label(id: &str) -> &'static str {
    registry().into_iter().find(|c| c.id == id).expect(id).label
}

fn listed(a: Availability, id: &str) -> bool {
    let l = label(id);
    palette_items_for(a).iter().any(|e| e.label == l)
}

// One row per flag: how to clear it in an Availability, and the commands it governs.
type Clear = fn(&mut Availability);
const FLAGS: [(&str, Clear, &[&str]); 21] = [
    (
        "close_others",
        |a| a.close_others = false,
        &["tabs.close_others"],
    ),
    (
        "reopen_closed",
        |a| a.reopen_closed = false,
        &["tabs.reopen"],
    ),
    ("stop", |a| a.stop = false, &["request.stop"]),
    ("discard", |a| a.discard = false, &["request.discard"]),
    ("save_all", |a| a.save_all = false, &["request.save_all"]),
    (
        "beautify",
        |a| a.beautify = false,
        &["request.beautify_body", "request.minify_body"],
    ),
    (
        "beautify_query",
        |a| a.beautify_query = false,
        &["request.beautify_query"],
    ),
    (
        "has_tabs",
        |a| a.has_tabs = false,
        &["tabs.close", "request.save", "request.undo", "request.redo"],
    ),
    (
        "request_tab",
        |a| a.request_tab = false,
        &[
            "request.send",
            "request.copy_as_curl",
            "request.generate_code",
            "request.save_as_example",
            "request.export_har",
            "request.compare_response",
            "request.copy_json_value",
            "request.copy_json_path",
            "request.set_json_variable",
            "request.assert_json_value",
        ],
    ),
    (
        "rename_idle",
        |a| a.rename_idle = false,
        &["request.rename"],
    ),
    ("close_all", |a| a.close_all = false, &["tabs.close_all"]),
    ("pin", |a| a.pin = false, &["tabs.pin"]),
    ("unpin", |a| a.unpin = false, &["tabs.unpin"]),
    ("rename", |a| a.rename = false, &["request.rename"]),
    ("duplicate", |a| a.duplicate = false, &["request.duplicate"]),
    (
        "show_in_file_manager",
        |a| a.show_in_file_manager = false,
        &["request.show_in_file_manager"],
    ),
    (
        "close_saved",
        |a| a.close_saved = false,
        &["tabs.close_saved"],
    ),
    (
        "multi_tab",
        |a| a.multi_tab = false,
        &["tabs.next", "tabs.previous"],
    ),
    ("not_last", |a| a.not_last = false, &["tabs.last"]),
    (
        "undo_delete",
        |a| a.undo_delete = false,
        &["request.undo_delete"],
    ),
    (
        "collection_open",
        |a| a.collection_open = false,
        &[
            "collection.settings",
            "collection.run",
            "collection.collapse_all",
        ],
    ),
];

#[test]
fn each_flag_hides_its_commands_and_nothing_else() {
    for (name, clear, ids) in FLAGS {
        let mut a = Availability::all();
        clear(&mut a);
        for id in ids {
            assert!(!listed(a, id), "{name} hides {id}");
            assert!(
                listed(Availability::all(), id),
                "{id} is listed when available"
            );
        }
        let all = palette_items_for(Availability::all()).len();
        assert!(palette_items_for(a).len() < all, "{name} hides something");
        for keep in [
            "request.undo",
            "request.redo",
            "request.save",
            "collection.switch_branch",
        ] {
            if name == "has_tabs" && keep != "collection.switch_branch" {
                continue;
            }
            assert!(listed(a, keep), "{keep} stays listed with {name} cleared");
        }
    }
}

#[test]
fn the_collection_flag_hides_exactly_the_scoped_commands() {
    let a = Availability {
        collection_open: false,
        ..Availability::all()
    };
    let hidden: Vec<&str> = collection_scoped_ids();
    let palette: Vec<_> = registry().into_iter().filter(|c| c.in_palette).collect();
    for id in &hidden {
        let cmd = palette.iter().find(|c| c.id == *id);
        let cmd = cmd.unwrap_or_else(|| panic!("{id} is a palette command"));
        assert!(!listed(a, id), "{id} is hidden");
        assert!(listed(Availability::all(), cmd.id));
    }
    let shown: Vec<_> = palette_items_for(a).iter().map(|e| e.label).collect();
    let all = palette_items_for(Availability::all());
    assert_eq!(all.len() - shown.len(), hidden.len());
}

#[test]
fn import_curl_stays_listed_without_a_collection() {
    let a = Availability {
        collection_open: false,
        ..Availability::all()
    };
    assert!(listed(a, "collection.import_curl"));
    assert!(!collection_scoped_ids().contains(&"collection.import_curl"));
}

// Opens the palette, filters it to `label` and counts the rows left. The filter matches
// substrings, so "Pin tab" also keeps "Unpin tab".
fn palette_rows(cx: &mut TestAppContext, s: &Started, label: &str) -> usize {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    frame(cx, s.win);
    let found = cx
        .update_window(s.win, |_, w, _| {
            gpui_kit::base::test_support::snapshots(w)
                .iter()
                .filter(|n| n.role() == Some(gpui_kit::Role::ListBoxOption))
                .count()
        })
        .unwrap();
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    found
}

fn palette_lists(cx: &mut TestAppContext, s: &Started, label: &str) -> bool {
    palette_rows(cx, s, label) > 0
}

fn three() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("c.yml", &request("c", "http://x/c")),
    ])
}

#[gpui_kit::test]
fn close_others_needs_another_unpinned_tab(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, label("tabs.close_others")));
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, label("tabs.close_others")));
    // A pinned neighbour is left alone by the command, so it is not something to close.
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(a, true)));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(b)));
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, label("tabs.close_others")));
}

#[gpui_kit::test]
fn reopen_needs_a_closed_tab(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, label("tabs.reopen")));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(a)));
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, label("tabs.reopen")));
}

#[gpui_kit::test]
fn discard_and_save_all_need_an_edit(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    for id in ["request.discard", "request.save_all"] {
        assert!(!palette_lists(cx, &s, label(id)), "{id} hidden when clean");
    }
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
    for id in ["request.discard", "request.save_all"] {
        assert!(palette_lists(cx, &s, label(id)), "{id} listed when dirty");
    }
    // Save all counts edits on any tab, Discard only the active one's.
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(b)));
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, label("request.save_all")));
    assert!(!palette_lists(cx, &s, label("request.discard")));
}

#[gpui_kit::test]
fn stop_is_listed_only_while_a_response_streams(cx: &mut TestAppContext) {
    use crate::support::sse_server::{Script, event};
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("one")]));
    let col = sse_collection(&server);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, label("request.stop")));
    let ed = s
        .shell
        .read_with(cx, |sh, cx| {
            let id = sh.workspace().read(cx).tabs().active().unwrap().id;
            sh.tab_area().read(cx).editor(id).cloned()
        })
        .expect("an editor");
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, label("request.stop")));
}

#[test]
fn the_collection_flag_hides_environment_and_sidebar_commands_that_need_one() {
    let a = Availability {
        collection_open: false,
        ..Availability::all()
    };
    for id in [
        "environment.open_selector",
        "environment.import",
        "environment.export",
        "environment.export_all",
        "request.rename",
        "request.duplicate",
        "request.show_in_file_manager",
    ] {
        assert!(!listed(a, id), "{id} is hidden without a collection");
    }
    // These act without a collection, so they stay.
    for id in [
        "environment.variables_overview",
        "view.filter_requests",
        "environment.manage_globals",
    ] {
        assert!(listed(a, id), "{id} stays listed without a collection");
    }
}

#[gpui_kit::test]
fn close_and_pin_commands_follow_the_open_tabs(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    frame(cx, s.win);
    let close = [label("tabs.close"), label("tabs.close_all")];
    let (pin, unpin) = (label("tabs.pin"), label("tabs.unpin"));
    // "Pin tab" lists its own row only when the rows it matches outnumber "Unpin tab"'s.
    let pin_listed =
        |cx: &mut TestAppContext| palette_rows(cx, &s, pin) > palette_rows(cx, &s, unpin);
    for l in close.into_iter().chain([unpin]) {
        assert!(!palette_lists(cx, &s, l), "{l} hidden with no tab");
    }
    assert!(!pin_listed(cx), "no tab, no Pin");
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    for l in close {
        assert!(palette_lists(cx, &s, l), "{l} listed with a tab");
    }
    assert!(pin_listed(cx));
    assert!(!palette_lists(cx, &s, unpin));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(a, true)));
    frame(cx, s.win);
    assert!(!pin_listed(cx));
    assert!(palette_lists(cx, &s, unpin));
    // Close all tabs leaves pinned tabs open, so with only pinned tabs it has nothing to do.
    assert!(palette_lists(cx, &s, close[0]));
    assert!(!palette_lists(cx, &s, close[1]));
    // One unpinned tab among pinned ones is enough for Close all tabs to act.
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, close[1]));
}

fn click_row(cx: &mut TestAppContext, s: &Started, rel: &str) {
    cx.update_window(s.win, |_, w, cx| w.click(row(rel), cx))
        .unwrap();
    frame(cx, s.win);
}

fn sidebar_commands() -> [&'static str; 3] {
    [
        label("request.rename"),
        label("request.duplicate"),
        label("request.show_in_file_manager"),
    ]
}

fn with_folder() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("x", "http://x/x")),
    ])
}

#[gpui_kit::test]
fn sidebar_commands_follow_the_selection(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = started(cx, &col);
    frame(cx, s.win);
    let [rename, duplicate, show] = sidebar_commands();
    for l in [rename, duplicate, show] {
        assert!(!palette_lists(cx, &s, l), "{l} hidden with no selection");
    }
    click_row(cx, &s, "a.yml");
    for l in [rename, duplicate, show] {
        assert!(palette_lists(cx, &s, l), "{l} listed for one request");
    }
    secondary_click(cx, s.win, row("b.yml"));
    frame(cx, s.win);
    for l in [rename, duplicate, show] {
        assert!(!palette_lists(cx, &s, l), "{l} hidden for several rows");
    }
    // A folder renames, duplicates with everything in it, and shows.
    click_row(cx, &s, "f");
    for l in [rename, duplicate, show] {
        assert!(palette_lists(cx, &s, l), "{l} listed for a folder");
    }
}

#[gpui_kit::test]
fn show_in_file_manager_is_hidden_on_a_draft(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, ip(""), cx));
    cx.run_until_parked();
    frame(cx, s.win);
    let [rename, duplicate, show] = sidebar_commands();
    for l in [rename, duplicate, show] {
        assert!(!palette_lists(cx, &s, l), "{l} hidden on a draft row");
    }
}

#[gpui_kit::test]
fn rename_is_listed_on_an_example_but_show_and_duplicate_are_not(cx: &mut TestAppContext) {
    let users = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: ok\n    response:\n      status: 200\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("users.yml", users)]);
    let s = started(cx, &col);
    frame(cx, s.win);
    let toggle = apinata_app::sidebar::marker_id("examples-toggle", &row("users.yml"));
    cx.update_window(s.win, |_, w, cx| w.click(toggle, cx))
        .unwrap();
    frame(cx, s.win);
    let example = apinata_app::sidebar::model::example_id(0, &ip("users.yml"), 0);
    cx.update_window(s.win, |_, w, cx| w.click(example, cx))
        .unwrap();
    frame(cx, s.win);
    let [rename, duplicate, show] = sidebar_commands();
    assert!(palette_lists(cx, &s, rename));
    assert!(!palette_lists(cx, &s, duplicate));
    assert!(!palette_lists(cx, &s, show));
}

#[gpui_kit::test]
fn tab_commands_follow_the_open_tabs_and_their_state(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    frame(cx, s.win);
    let (saved, next, prev, last) = (
        label("tabs.close_saved"),
        label("tabs.next"),
        label("tabs.previous"),
        label("tabs.last"),
    );
    for l in [saved, next, prev, last] {
        assert!(!palette_lists(cx, &s, l), "{l} hidden with no tab");
    }
    // One clean tab can be closed as saved, but there is nowhere to switch to.
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, saved));
    for l in [next, prev, last] {
        assert!(!palette_lists(cx, &s, l), "{l} hidden with one tab");
    }
    // A pinned tab is left open by Close saved tabs.
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(a, true)));
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, saved), "pinned-only");
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(a, false)));
    // So is a tab with unsaved edits.
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, saved), "dirty-only");
    // Two tabs: switching works; the newest is active and last, so Last tab has nothing to do.
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, saved), "b is clean");
    assert!(palette_lists(cx, &s, next) && palette_lists(cx, &s, prev));
    assert!(!palette_lists(cx, &s, last), "active tab is the last");
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(a)));
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, last));
    let _ = b;
}

// With no tab open, every command of the Tabs group is hidden.
fn no_tabs() -> Availability {
    Availability {
        close_to_right: false,
        reveal: false,
        reopen_closed: false,
        close_others: false,
        has_tabs: false,
        request_tab: false,
        close_all: false,
        pin: false,
        unpin: false,
        close_saved: false,
        multi_tab: false,
        not_last: false,
        ..Availability::all()
    }
}

// The kit numbers a group's section even when it is empty, so the paths the palette confirms
// must too: a later group's command and the Requests rows still land on their own rows.
#[gpui_kit::test]
fn a_fully_hidden_group_keeps_later_sections_stable(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("alpha.yml", &request("alpha", "http://x/alpha")),
    ]);
    let s = started(cx, &col);
    frame(cx, s.win);
    let items = palette_items_for(no_tabs());
    assert!(
        items.iter().all(|e| e.group != PaletteGroup::Tabs),
        "the Tabs group is empty"
    );
    let ix = items
        .iter()
        .position(|e| e.label == label("view.toggle_sidebar"))
        .expect("listed");
    let path = index_path(&items, ix);
    let after_tabs = PaletteGroup::ALL
        .iter()
        .position(|g| *g == items[ix].group)
        .expect("in ALL")
        > PaletteGroup::ALL
            .iter()
            .position(|g| *g == PaletteGroup::Tabs)
            .expect("in ALL");
    assert!(after_tabs, "Toggle sidebar sits after the empty group");
    assert_eq!(
        path.section,
        PaletteGroup::ALL
            .iter()
            .position(|g| *g == items[ix].group)
            .unwrap()
    );

    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("toggle sidebar", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("command-palette").click(path, cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));

    // The Requests group is the section after every registry group, hidden or not.
    assert!(tab_names(cx, &s.ws).is_empty());
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("alpha", cx))
        .unwrap();
    frame(cx, s.win);
    let requests = gpui_kit::component::IndexPath::new(0).section(PaletteGroup::ALL.len());
    cx.update_window(s.win, |_, w, cx| {
        w.within("command-palette").click(requests, cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &s.ws), ["alpha.yml"]);
}

// The request-tab commands act only on a request tab; Save, Undo and Redo also act on the
// environment and settings tabs, so only having no tab at all hides them.
const REQUEST_ONLY: [&str; 10] = [
    "request.send",
    "request.copy_as_curl",
    "request.generate_code",
    "request.save_as_example",
    "request.export_har",
    "request.compare_response",
    "request.copy_json_value",
    "request.copy_json_path",
    "request.set_json_variable",
    "request.assert_json_value",
];

fn request_only_listed(cx: &mut TestAppContext, s: &Started) -> Vec<bool> {
    REQUEST_ONLY
        .iter()
        .map(|id| palette_lists(cx, s, label(id)))
        .collect()
}

#[gpui_kit::test]
fn request_commands_follow_the_active_tab_kind(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    frame(cx, s.win);
    let (undo, redo) = (label("request.undo"), label("request.redo"));
    // No tab: nothing to send, save or undo.
    assert!(request_only_listed(cx, &s).iter().all(|l| !l), "no tabs");
    assert!(!palette_lists(cx, &s, undo) && !palette_lists(cx, &s, redo));
    let none_save = palette_rows(cx, &s, "Save");
    assert_eq!(none_save, 0, "Save hidden with no tab");
    // An Environments tab: request-only commands stay hidden, the shared ones return.
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Environments, cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(request_only_listed(cx, &s).iter().all(|l| !l), "env tab");
    assert!(palette_lists(cx, &s, undo) && palette_lists(cx, &s, redo));
    assert!(palette_rows(cx, &s, "Save") > none_save, "Save is back");
    // A request tab lists everything.
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(
        request_only_listed(cx, &s).iter().all(|l| *l),
        "request tab"
    );
    // So does an unsaved draft.
    new_draft_at(cx, &s, ip(""));
    frame(cx, s.win);
    assert!(request_only_listed(cx, &s).iter().all(|l| *l), "draft tab");
}

#[gpui_kit::test]
fn rename_is_hidden_while_a_rename_is_under_way(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = started(cx, &col);
    frame(cx, s.win);
    click_row(cx, &s, "a.yml");
    let rename = label("request.rename");
    assert!(palette_lists(cx, &s, rename), "listed before");
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    cx.update_window(s.win, |_, w, cx| {
        sidebar.update(cx, |sb, cx| sb.start_rename(&s.root, &ip("a.yml"), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    assert!(sidebar.read_with(cx, |sb, _| sb.renaming().is_some()));
    assert!(!palette_lists(cx, &s, rename), "hidden during the rename");
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    click_row(cx, &s, "a.yml");
    assert!(palette_lists(cx, &s, rename), "listed after");
}

// With no tab the Request group is gone, so Enter must still run the row it highlights in
// the sections after it.
#[gpui_kit::test]
fn enter_runs_the_right_row_after_a_hidden_group(cx: &mut TestAppContext) {
    let col = three();
    let s = started(cx, &col);
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("toggle sidebar", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
}

#[test]
fn the_demo_command_is_listed_with_or_without_a_collection() {
    for open in [true, false] {
        let a = Availability {
            collection_open: open,
            ..Availability::all()
        };
        assert!(listed(a, "collection.open_demo"), "collection_open={open}");
    }
    assert!(!collection_scoped_ids().contains(&"collection.open_demo"));
}
