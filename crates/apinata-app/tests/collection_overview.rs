//! The collection settings' Overview section: what the collection is, and its Runner, Mock
//! and Export actions.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::mock_server::MockState;
use apinata_app::tabs::TabKind;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/a.yml", &request("a", "http://x")),
        ("b.yml", &request("b", "http://y")),
    ])
}

// Opens the collection's settings tab in the started app and draws it.
fn overview(cx: &mut TestAppContext, s: &Started) {
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Collection, cx)
    });
    frame(cx, s.win);
}

// The text the last frame painted inside the element `id`, joined in paint order.
fn text_in(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(win, |_, w, _| {
        let el = gpui_kit::ElementId::Name(id.into());
        gpui_kit::base::test_support::texts(w)
            .into_iter()
            .filter(|t| t.path().contains(&el))
            .map(|t| t.text().to_owned())
            .collect::<Vec<_>>()
            .join("")
    })
    .unwrap()
}

// The accessible label of the element `id`, which reads out the value it shows.
fn label_of(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, _| w.find(id).label().map(str::to_owned))
        .unwrap()
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.within("config-overview").click(id, cx))
        .unwrap();
    frame(cx, win);
}

fn folder_editor(cx: &mut TestAppContext, s: &Started) -> (AnyWindowHandle, Entity<ConfigEditor>) {
    support::init(cx);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Folder(ip("users")), w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

fn active_kind(cx: &mut TestAppContext, s: &Started) -> Option<TabKind> {
    s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()))
}

#[gpui_kit::test]
fn collection_settings_open_on_the_overview_first(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    overview(cx, &s);
    assert!(present(cx, s.win, "config-overview"));
    let (_, folder) = folder_editor(cx, &s);
    assert!(!folder.read_with(cx, |e, _| e.sections().contains(&ConfigSection::Overview)));
    assert_eq!(
        folder.read_with(cx, |e, _| e.section()),
        ConfigSection::Headers
    );
}

#[gpui_kit::test]
fn the_overview_shows_the_name_path_and_request_count(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    overview(cx, &s);
    let label = |cx: &mut TestAppContext, id| label_of(cx, s.win, id);
    assert_eq!(label(cx, "config-overview-name").as_deref(), Some("c"));
    let path = s.root.display().to_string();
    assert_eq!(label(cx, "config-overview-path"), Some(path));
    assert_eq!(
        label(cx, "config-overview-requests").as_deref(),
        Some("2 requests")
    );
}

#[gpui_kit::test]
fn the_overview_previews_the_collection_docs(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    overview(cx, &s);
    assert!(present(cx, s.win, "config-overview-docs-empty"));
    s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Edit {
                target: Target::Collection,
                edit: Edit::Docs("Talks to the users API.".into()),
            },
            cx,
        )
    });
    frame(cx, s.win);
    assert!(!present(cx, s.win, "config-overview-docs-empty"));
    assert!(text_in(cx, s.win, "config-overview-docs").contains("Talks to the users API."));
}

#[gpui_kit::test]
fn the_overview_runner_action_opens_the_collection_runner(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    overview(cx, &s);
    click(cx, s.win, "config-overview-run");
    assert_eq!(active_kind(cx, &s), Some(TabKind::Runner(ItemPath::root())));
}

#[gpui_kit::test]
fn the_overview_mock_action_starts_the_mock_server(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
    let col = tree();
    let s = started(cx, &col);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    s.ws.update(cx, |w, _| w.set_mock_port(port));
    overview(cx, &s);
    click(cx, s.win, "config-overview-mock");
    let root = s.root.clone();
    assert_ne!(
        s.ws.read_with(cx, |w, _| w.mock_state(&root)),
        MockState::Off
    );
    s.ws.update(cx, |w, cx| w.set_mock(&root, false, cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_overview_export_action_asks_where_to_save(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    overview(cx, &s);
    click(cx, s.win, "config-overview-export");
    click_menu(cx, s.win, "Postman");
    assert!(cx.did_prompt_for_new_path());
}
