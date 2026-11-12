//! The palette lists every request of the open collections and opens the chosen one; a
//! WebSocket or gRPC request is listed with its kind tag and says it cannot be sent yet.

use crate::support;

use apinata_app::palette::{
    Availability, GoToRequest, index_path, palette_items, palette_items_for, request_entries,
};
use apinata_app::snapshot::Unsupported;
use apinata_app::tab_area::ActivateNextTab;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

const GRPC: &str = "info:\n  name: Greeter\n  type: grpc\n  seq: 3\n";
// A graphql file core cannot read is opaque, but it is not a kind the app cannot send.
const BROKEN_GRAPHQL: &str = "info:\n  name: Query\n  type: graphql\n  seq: 4\n";
const WS_NOTE: &str = "WebSocket requests cannot be sent yet. The file is kept as it is.";
const GRPC_NOTE: &str = "gRPC requests cannot be sent yet. The file is kept as it is.";

fn zoo() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/zebra.yml", &request("Fetch zebra", "http://x/z")),
        ("lion.yml", &request("Fetch lion", "http://x/l")),
        ("ws.yml", SOCKET),
        ("rpc.yml", GRPC),
        ("query.yml", BROKEN_GRAPHQL),
    ])
}

#[gpui_kit::test]
fn entries_name_each_request_with_its_collection_and_folder(cx: &mut TestAppContext) {
    let col = zoo();
    let (_state, ws, root) = open_dir(cx, &col);
    let entries = ws.read_with(cx, request_entries);
    assert!(entries.iter().all(|e| e.root == root));
    let mut got: Vec<(String, String, String, Option<Unsupported>)> = entries
        .into_iter()
        .map(|e| {
            let path = e.path.as_str().to_owned();
            (e.label.clone(), e.detail(), path, e.unsupported)
        })
        .collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    let want = [
        ("Fetch lion", "c", "lion.yml", None),
        ("Fetch zebra", "c / users", "users/zebra.yml", None),
        ("Greeter", "c", "rpc.yml", Some(Unsupported::Grpc)),
        ("Socket", "c", "ws.yml", Some(Unsupported::WebSocket)),
    ]
    .map(|(a, b, c, d)| (a.to_owned(), b.to_owned(), c.to_owned(), d));
    assert_eq!(got, want, "the broken graphql file is not offered");
}

// Opens the palette and types `query`.
fn search(cx: &mut TestAppContext, win: AnyWindowHandle, query: &str) {
    cx.simulate_keystrokes(win, &key("shift-p"));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.input(query, cx))
        .unwrap();
    frame(cx, win);
}

// The kind tag the palette's row for the request at `path` shows, if any.
fn kind_tag(cx: &mut TestAppContext, s: &Started, path: &str) -> Option<String> {
    let r = (s.ws.read_with(cx, request_entries).iter())
        .position(|e| e.path.as_str() == path)
        .unwrap_or_else(|| panic!("{path} is not offered"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(("palette-request-kind", r))
            .map(|e| e.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

// The text of the "cannot be sent yet" toast showing, if any.
fn note(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find("unsupported-note")
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn websocket_and_grpc_rows_show_their_kind_tag(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    let mut tag_of = |query: &str, path: &str| {
        search(cx, s.win, query);
        let tag = kind_tag(cx, &s, path);
        cx.simulate_keystrokes(s.win, "escape");
        frame(cx, s.win);
        tag
    };
    assert_eq!(tag_of("Socket", "ws.yml").as_deref(), Some("WS"));
    assert_eq!(tag_of("Greeter", "rpc.yml").as_deref(), Some("gRPC"));
    assert_eq!(tag_of("lion", "lion.yml"), None, "an http row has no tag");
}

#[gpui_kit::test]
fn a_websocket_row_name_starts_where_an_http_row_name_does(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    let entries = s.ws.read_with(cx, request_entries);
    let ix = |path: &str| (entries.iter().position(|e| e.path.as_str() == path)).unwrap();
    // Each row is measured in a search of its own; its row's height does not move its x.
    let mut name_left = |query: &str, r: usize| {
        search(cx, s.win, query);
        let left = cx
            .update_window(s.win, |_, w, _| {
                w.find(("palette-request-name", r)).bounds().left()
            })
            .unwrap();
        cx.simulate_keystrokes(s.win, "escape");
        frame(cx, s.win);
        left
    };
    let socket = name_left("Socket", ix("ws.yml"));
    let lion = name_left("lion", ix("lion.yml"));
    assert_eq!(socket, lion, "every row's name starts in one column");
}

#[gpui_kit::test]
fn choosing_a_websocket_request_says_it_cannot_be_sent(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    let before = std::fs::read(s.root.join("ws.yml")).unwrap();
    search(cx, s.win, "Socket");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "the palette closed");
    assert_eq!(note(cx, s.win).as_deref(), Some(WS_NOTE));
    assert!(tab_names(cx, &s.ws).is_empty(), "no tab opens");
    assert_eq!(std::fs::read(s.root.join("ws.yml")).unwrap(), before);
}

#[gpui_kit::test]
fn choosing_a_grpc_request_says_it_cannot_be_sent(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    search(cx, s.win, "Greeter");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    assert_eq!(note(cx, s.win).as_deref(), Some(GRPC_NOTE));
    assert!(tab_names(cx, &s.ws).is_empty(), "no tab opens");
}

#[gpui_kit::test]
fn typing_a_request_name_and_enter_opens_it(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    search(cx, s.win, "zebra");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "the palette closed");
    assert_eq!(tab_names(cx, &s.ws), ["users/zebra.yml"]);
    let preview =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.preview));
    assert_eq!(preview, Some(false), "a palette open keeps the tab");
}

// Opens the go-to-request palette with its own key.
fn go_to(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.simulate_keystrokes(win, &key("p"));
    frame(cx, win);
}

#[gpui_kit::test]
fn the_go_to_request_key_lists_only_requests(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    go_to(cx, s.win);
    assert!(present(cx, s.win, "command-palette"));
    let first = index_path(&palette_items(), 0);
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("command-palette");
        assert!(palette.try_find(first).is_none(), "no command rows");
        assert!(
            palette.try_find(("palette-request-name", 0usize)).is_some(),
            "a request row"
        );
    })
    .unwrap();
    cx.update_window(s.win, |_, w, cx| w.input("zebra", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &s.ws), ["users/zebra.yml"]);
}

// The full palette keeps its own key and offers Go to request with its key beside it.
#[gpui_kit::test]
fn the_command_palette_offers_go_to_request(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    search(cx, s.win, "go to request");
    // Rows are placed among the shown commands, and the test app has no update source, so
    // Check for updates is hidden; key hints keep their place in the full list.
    let shown = palette_items_for(Availability {
        updates: false,
        ..Availability::all()
    });
    let at = |items: &[_]| {
        (items.iter())
            .position(|e: &apinata_app::palette::PaletteEntry| e.action.partial_eq(&GoToRequest))
            .expect("listed")
    };
    let (row, ix) = (at(&shown), at(&palette_items()));
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("command-palette");
        assert!(palette.try_find(index_path(&shown, row)).is_some());
        let hint = palette.find(("palette-kbd", ix));
        assert_eq!(hint.label(), Some(secondary("p").as_str()));
    })
    .unwrap();
}

// Whether the open palette shows the first registry command's row.
fn lists_commands(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    let first = index_path(&palette_items(), 0);
    cx.update_window(win, |_, w, _| {
        w.within("command-palette").try_find(first).is_some()
    })
    .unwrap()
}

fn close(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.simulate_keystrokes(win, "escape");
    frame(cx, win);
}

#[gpui_kit::test]
fn each_key_opens_its_own_palette_after_the_other(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    go_to(cx, s.win);
    assert!(
        !lists_commands(cx, s.win),
        "Go to request lists no commands"
    );
    close(cx, s.win);
    search(cx, s.win, "");
    assert!(lists_commands(cx, s.win), "the command palette lists them");
    close(cx, s.win);
    go_to(cx, s.win);
    assert!(
        !lists_commands(cx, s.win),
        "and Go to request again does not"
    );
    close(cx, s.win);
    search(cx, s.win, "");
    assert!(
        lists_commands(cx, s.win),
        "nor does it stop the next palette"
    );
}

// The last chord bound is the one the hint shows, so Next tab reads the browser key.
#[gpui_kit::test]
fn the_next_tab_row_shows_ctrl_tab(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    // Next tab is listed only with more than one tab open.
    open_tab(cx, &s.ws, &s.root, "lion.yml");
    open_tab(cx, &s.ws, &s.root, "users/zebra.yml");
    frame(cx, s.win);
    search(cx, s.win, "next tab");
    let ix = (palette_items().iter())
        .position(|e| e.action.partial_eq(&ActivateNextTab))
        .expect("listed");
    cx.update_window(s.win, |_, w, _| {
        let hint = w.within("command-palette").find(("palette-kbd", ix));
        assert_eq!(hint.label(), Some("ctrl-tab"));
    })
    .unwrap();
}
