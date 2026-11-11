//! Items that show the same name get the shortest part of their place that tells them apart.

use crate::support::{self, *};

use std::path::Path;

use apinata_app::name_hints::{Shown, collection_hints, collection_tags, hints};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};

fn shown(name: &str, place: &[&str], file: Option<String>) -> Shown {
    Shown {
        name: name.to_owned(),
        place: place.iter().map(|s| s.to_string()).collect(),
        file,
    }
}

fn hint_strings(items: &[Shown]) -> Vec<Option<String>> {
    hints(items)
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[test]
fn unique_names_get_no_hint() {
    let items = [
        shown("Get user", &["Demo", "Users"], some("get.yml")),
        shown("List users", &["Demo", "Users"], some("list.yml")),
    ];
    assert_eq!(hint_strings(&items), [None::<String>, None]);
}

#[test]
fn shared_names_get_the_shortest_folder_path_that_differs() {
    let items = [
        shown("Get user", &["Demo", "Users"], some("Get user.yml")),
        shown(
            "Get user",
            &["Demo", "Admin", "Users"],
            some("Get user.yml"),
        ),
        shown("Get user", &["Demo", "GraphQL"], some("Get user.yml")),
    ];
    assert_eq!(
        hint_strings(&items),
        [some("Users"), some("Admin/Users"), some("GraphQL")]
    );
}

#[test]
fn an_item_at_the_top_of_its_collection_shows_the_collection() {
    let items = [
        shown("Get user", &["Demo"], some("a.yml")),
        shown("Get user", &["Demo", "Users"], some("b.yml")),
    ];
    assert_eq!(hint_strings(&items), [some("Demo"), some("Users")]);
}

#[test]
fn items_in_two_collections_show_the_collection_where_the_folders_match() {
    let items = [
        shown("Get user", &["Demo", "Users"], some("a.yml")),
        shown("Get user", &["Shop", "Users"], some("a.yml")),
    ];
    assert_eq!(
        hint_strings(&items),
        [some("Demo/Users"), some("Shop/Users")]
    );
}

#[test]
fn siblings_sharing_a_name_show_their_file_names() {
    let items = [
        shown("echo", &["Demo"], some("one.yml")),
        shown("echo", &["Demo"], some("two.yml")),
    ];
    assert_eq!(hint_strings(&items), [some("one.yml"), some("two.yml")]);
}

#[test]
fn same_named_collections_show_the_part_of_their_folder_that_differs() {
    let a = Path::new("/home/dan/work/shop");
    let b = Path::new("/home/dan/play/shop");
    let c = Path::new("/home/dan/demo");
    let got = collection_hints(&[(a, "Shop"), (b, "Shop"), (c, "Demo")]);
    assert_eq!(
        got,
        [
            Some("work/shop".to_owned()),
            Some("play/shop".to_owned()),
            None
        ]
    );
    let tags = collection_tags(&[(a, "Shop"), (b, "Shop"), (c, "Demo")]);
    assert_eq!(
        tags[a], "work/shop",
        "a shared name is replaced by its hint"
    );
    assert_eq!(tags[c], "Demo");
}

#[gpui_kit::test]
fn the_title_bar_and_switcher_tell_two_shop_collections_apart(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let roots = [tmp.path().join("work/shop"), tmp.path().join("play/shop")];
    for dir in &roots {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: Shop\n",
        )
        .unwrap();
    }
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    for root in &roots {
        ws.update(cx, |w, cx| w.open(root.clone(), cx));
    }
    cx.run_until_parked();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert!(
        present(cx, win, "collection-switcher-hint"),
        "the title bar names where the current Shop lives"
    );
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, win);
    for ix in [0usize, 1] {
        assert!(present(cx, win, ("switch-open-hint", ix)), "row {ix}");
    }
}
