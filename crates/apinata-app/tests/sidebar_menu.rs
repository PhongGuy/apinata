use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::commands::registry;
use apinata_app::sidebar::menu::NEW_FOLDER_NAME;
use apinata_app::sidebar::menu::{MenuEntry, clone_name, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::sidebar::{DeleteSelected, NewFolder, SidebarEvent};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

fn choose(
    cx: &mut TestAppContext,
    h: &SidebarHarness,
    path: &str,
    kind: NodeKind,
    entry: MenuEntry,
) {
    assert!(menu_entries(kind).contains(&entry), "entry offered");
    cx.update_window(h.win, |_, w, cx| w.right_click(row(path), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        let ix = menu_index(w, entry.label());
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn type_and_enter(cx: &mut TestAppContext, h: &SidebarHarness, text: &str) {
    cx.update_window(h.win, |_, w, cx| {
        w.input(text, cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn renaming(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<(std::path::PathBuf, ItemPath)> {
    h.view.read_with(cx, |s, _| s.renaming())
}

#[test]
fn entries_follow_the_row_kind() {
    use MenuEntry as E;
    assert_eq!(
        menu_entries(NodeKind::Collection),
        [
            E::NewRequest,
            E::NewGraphqlRequest,
            E::NewFolder,
            E::PasteItem,
            E::SortByName,
            E::RunCollection,
            E::CollectionSettings,
            E::ExportPostman,
            E::ExportPostmanKeepScripts,
            E::ExportOpenApi,
            E::ExportHtml,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::CollapseAll,
            E::ExpandAll,
            E::Close
        ]
    );
    assert_eq!(
        menu_entries(NodeKind::Folder),
        [
            E::NewRequest,
            E::NewGraphqlRequest,
            E::NewFolder,
            E::CloneItem,
            E::CopyItem,
            E::CutItem,
            E::PasteItem,
            E::SortByName,
            E::RunFolder,
            E::Settings,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::Rename,
            E::Delete
        ]
    );
    assert_eq!(
        menu_entries(NodeKind::Request),
        [
            E::CloneItem,
            E::CopyItem,
            E::CutItem,
            E::PasteItem,
            E::CopyAsCurl,
            E::GenerateCode,
            E::ConvertToGraphql,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::Rename,
            E::Delete
        ]
    );
    assert_eq!(
        menu_entries(NodeKind::Opaque),
        [
            E::CloneItem,
            E::CopyItem,
            E::CutItem,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::Delete
        ]
    );
    assert_eq!(menu_entries(NodeKind::Example), [E::Rename, E::Delete]);
    assert!(menu_entries(NodeKind::Invalid).is_empty());
    assert_eq!(
        (
            E::CloneItem.label(),
            E::NewRequest.label(),
            E::NewFolder.label()
        ),
        ("Duplicate", "New request", "New folder")
    );
    for kind in [NodeKind::Collection, NodeKind::Folder, NodeKind::Request] {
        for e in menu_entries(kind) {
            // Postman, OpenAPI, HTML and GraphQL are product or format names, so they keep their
            // capitals.
            let rest: String = e
                .label()
                .replace("Postman", "postman")
                .replace("OpenAPI", "openapi")
                .replace("HTML", "html")
                .replace("GraphQL", "graphql")
                .chars()
                .skip(1)
                .collect();
            assert!(
                !rest.chars().any(char::is_uppercase),
                "{} is sentence case",
                e.label()
            );
        }
    }
    assert_eq!(clone_name("Health"), "Health copy");
}

#[gpui_kit::test]
fn settings_on_a_folder_asks_for_its_settings_tab(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::Settings);
    let expected = apinata_app::sidebar::SidebarEvent::OpenSettings {
        root: h.root.clone(),
        folder: Some(ip("users")),
    };
    assert_eq!(seen.borrow().last(), Some(&expected));
}

#[gpui_kit::test]
fn clone_copies_the_request_and_selects_the_copy(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(
        cx,
        &h,
        "health.yml",
        NodeKind::Request,
        MenuEntry::CloneItem,
    );
    assert!(h.root.join("Health copy.yml").exists());
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("Health copy.yml")))
    );
}

#[gpui_kit::test]
fn duplicate_copies_a_folder_with_its_children_and_selects_the_copy(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::CloneItem);
    let names = |rel: &str| {
        let mut names: Vec<String> = std::fs::read_dir(h.root.join(rel))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names("Users copy"), names("users"));
    // The copy goes through the writer, so its bytes are laid out anew; its content is the same.
    let load = |rel: &str| apinata_core::RequestFile::load(h.root.join(rel)).unwrap();
    let (copy, source) = (load("Users copy/list.yml"), load("users/list.yml"));
    assert_eq!(
        (copy.name(), copy.method(), copy.url(), copy.seq()),
        (source.name(), source.method(), source.url(), source.seq())
    );
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("Users copy")))
    );
    assert!(shown(cx, h.win, row("Users copy")));
}

#[gpui_kit::test]
fn rename_from_the_menu_focuses_the_inline_input(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(cx, &h, "status.yml", NodeKind::Request, MenuEntry::Rename);
    assert_eq!(renaming(cx, &h), Some((h.root.clone(), ip("status.yml"))));
    type_and_enter(cx, &h, "Ready");
    assert!(h.root.join("Ready.yml").exists());
}

#[gpui_kit::test]
fn new_request_in_a_folder_opens_a_draft_there(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    // An unrelated create first: its Created notice must not start a rename.
    let other = TreeOp::CreateFolder {
        parent: ItemPath::root(),
        name: "x".into(),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(other), cx));
    cx.run_until_parked();
    assert_eq!(renaming(cx, &h), None);
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::NewRequest);
    let tabs = tab_names(cx, &h.ws);
    let [draft] = tabs.as_slice() else {
        panic!("{tabs:?}")
    };
    assert!(ip(draft).is_draft(), "{draft}");
    assert_eq!(ip(draft).parent(), Some(ip("users")));
    assert!(!h.root.join("users/Untitled.yml").exists());
    assert_eq!(renaming(cx, &h), None);
}

#[gpui_kit::test]
fn the_collection_row_creates_in_the_root(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(cx, &h, "", NodeKind::Collection, MenuEntry::NewRequest);
    let tabs = tab_names(cx, &h.ws);
    let [draft] = tabs.as_slice() else {
        panic!("{tabs:?}")
    };
    assert_eq!(ip(draft).parent(), Some(ItemPath::root()));
    choose(cx, &h, "", NodeKind::Collection, MenuEntry::NewFolder);
    assert!(h.root.join("New folder").is_dir());
    assert_eq!(renaming(cx, &h), Some((h.root.clone(), ip("New folder"))));
}

// A row created under a filter it does not match would stay hidden with its rename pending.
#[gpui_kit::test]
fn a_create_under_a_filter_clears_it_and_renames_at_once(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input("health", cx))
        .unwrap();
    choose(cx, &h, "", NodeKind::Collection, MenuEntry::NewFolder);
    assert_eq!(renaming(cx, &h), Some((h.root.clone(), ip("New folder"))));
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "");
}

#[gpui_kit::test]
fn delete_asks_first_for_folders_and_opaque_items(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::Delete);
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(h.root.join("users").exists());
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::Delete);
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(!h.root.join("users").exists());
    assert!(!shown(cx, h.win, row("users")));
    choose(cx, &h, "ws.yml", NodeKind::Opaque, MenuEntry::Delete);
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(!h.root.join("ws.yml").exists());
}

#[gpui_kit::test]
fn close_removes_the_collection(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(cx, &h, "", NodeKind::Collection, MenuEntry::Close);
    assert!(h.ws.read_with(cx, |w, _| w.collections().is_empty()));
    assert!(!shown(cx, h.win, row("")));
}

#[gpui_kit::test]
fn secondary_d_clones_the_selected_request(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("status.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    cx.simulate_keystrokes(h.win, &secondary("d"));
    cx.run_until_parked();
    assert!(h.root.join("Status copy.yml").exists());
}

#[gpui_kit::test]
fn escape_on_the_delete_dialog_keeps_the_folder(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::Delete);
    let win = confirm_window(cx).expect("the delete dialog");
    cx.update_window(win, |_, w, _| {
        assert!(w.try_find("confirm-detail").is_some());
    })
    .unwrap();
    cx.simulate_keystrokes(win, "escape");
    cx.run_until_parked();
    assert!(!confirm_open(cx));
    assert!(h.root.join("users").exists());
}

fn select(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    cx.update_window(h.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn delete_selected_asks_then_deletes(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    select(cx, &h, "status.yml");
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.delete_selected(&DeleteSelected, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(!h.root.join("status.yml").exists());
}

#[gpui_kit::test]
fn new_folder_goes_beside_the_selected_request(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    // The folder starts collapsed; opening the request's tab makes the sidebar follow it.
    open_tab(cx, &h.ws, &h.root, "users/list.yml");
    frame(cx, h.win);
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("users/list.yml")))
    );
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.new_folder(&NewFolder, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(h.root.join("users").join(NEW_FOLDER_NAME).is_dir());
    assert_eq!(
        renaming(cx, &h),
        Some((h.root.clone(), ip(&format!("users/{NEW_FOLDER_NAME}"))))
    );
}

#[gpui_kit::test]
fn new_graphql_request_opens_a_graphql_draft(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    choose(
        cx,
        &h,
        "",
        NodeKind::Collection,
        MenuEntry::NewGraphqlRequest,
    );
    let tabs = tab_names(cx, &h.ws);
    let [draft] = tabs.as_slice() else {
        panic!("{tabs:?}")
    };
    assert_eq!(ip(draft).parent(), Some(ItemPath::root()));
    let view = view_of(cx, &h.ws, &h.root, draft);
    assert_eq!(view.kind, apinata_core::RequestKind::Graphql);
    assert!(view.draft);
    assert_eq!(view.method, "POST");
    assert_eq!(
        view.body,
        apinata_core::BodySpec::Graphql {
            query: String::new(),
            variables: String::new()
        }
    );
    assert_eq!(MenuEntry::NewGraphqlRequest.label(), "New GraphQL request");
}

#[test]
fn tree_entries_take_the_registry_names() {
    use MenuEntry as E;
    let named = |id: &str| registry().into_iter().find(|c| c.id == id).expect(id).label;
    for (entry, id) in [
        (E::NewRequest, "collection.new_request"),
        (E::NewFolder, "collection.new_folder"),
        (E::CopyAsCurl, "request.copy_as_curl"),
        (E::GenerateCode, "request.generate_code"),
        (E::RunCollection, "collection.run"),
        (E::RunFolder, "collection.run_folder"),
        (E::CollectionSettings, "collection.settings"),
        (E::ExportPostman, "collection.export"),
        (
            E::ExportPostmanKeepScripts,
            "collection.export_keep_scripts",
        ),
        (E::ExportOpenApi, "collection.export_openapi"),
        (E::ExportHtml, "collection.export_html"),
    ] {
        assert_eq!(entry.label(), named(id), "{entry:?}");
    }
    assert_eq!(
        (
            E::RunCollection.label(),
            E::RunFolder.label(),
            E::CollectionSettings.label(),
            E::Settings.label(),
        ),
        (
            "Run collection",
            "Run folder",
            "Collection settings",
            "Settings"
        )
    );
    for kind in [
        NodeKind::Collection,
        NodeKind::Folder,
        NodeKind::Request,
        NodeKind::Opaque,
        NodeKind::Example,
    ] {
        for e in menu_entries(kind) {
            assert!(
                !e.label().ends_with("...") && !e.label().ends_with('…'),
                "{e:?}"
            );
        }
    }
}

#[gpui_kit::test]
fn collection_settings_on_the_collection_row_asks_for_its_settings_tab(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    choose(
        cx,
        &h,
        "",
        NodeKind::Collection,
        MenuEntry::CollectionSettings,
    );
    let expected = SidebarEvent::OpenSettings {
        root: h.root.clone(),
        folder: None,
    };
    assert_eq!(seen.borrow().last(), Some(&expected));
}

#[test]
fn a_draft_row_offers_save_and_close() {
    use MenuEntry as E;
    assert_eq!(menu_entries(NodeKind::Draft), [E::SaveDraft, E::CloseDraft]);
    assert_eq!(
        (E::SaveDraft.label(), E::CloseDraft.label()),
        ("Save", "Close")
    );
    assert!(menu_entries(NodeKind::UnsavedGroup).is_empty());
}
