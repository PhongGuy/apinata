//! The collection's Presets tab: the request type and URL new requests start with.

use crate::support;

use apinata_app::command::{Command, Edit, NoticeKind, Target, TreeOp};
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::presets_section::PresetsSection;
use apinata_app::workspace::Workspace;
use apinata_core::{CollectionPresets, ItemPath, RequestKind};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::Path;
use support::*;

const GRAPHQL: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nextensions:\n  bruno:\n    presets:\n      request:\n        type: graphql\n        url: https://g.test\n      defaultEnvironment: Local\n";

type Host = Entity<SectionHost<PresetsSection>>;

fn section(cx: &mut TestAppContext, presets: CollectionPresets) -> (AnyWindowHandle, Host) {
    let (win, host) = host_section(cx, PresetsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&presets, w, cx))
    })
    .unwrap();
    frame(cx, win);
    (win, host)
}

fn last(cx: &mut TestAppContext, host: &Host) -> Option<CollectionPresets> {
    host.read_with(cx, |h, _| {
        h.edits.iter().rev().find_map(|e| match e {
            Edit::Presets(p) => Some(p.clone()),
            _ => None,
        })
    })
}

#[gpui_kit::test]
fn loading_shows_the_presets_without_sending(cx: &mut TestAppContext) {
    let (win, host) = section(cx, CollectionPresets::new("graphql", "https://g.test"));
    assert!(present(cx, win, "presets-url"));
    assert_eq!(host.read_with(cx, |h, _| h.edits.len()), 0);
    let s = host.read_with(cx, |h, _| h.view.clone());
    assert_eq!(s.read_with(cx, |s, cx| s.url(cx)), "https://g.test");
    assert_eq!(s.read_with(cx, |s, _| s.request_type()), "graphql");
}

#[gpui_kit::test]
fn typing_and_picking_send_the_presets(cx: &mut TestAppContext) {
    let (win, host) = section(cx, CollectionPresets::new("http", ""));
    replace_input(cx, win, None, "presets-url", "https://api.test");
    frame(cx, win);
    assert_eq!(
        last(cx, &host),
        Some(CollectionPresets::new("http", "https://api.test"))
    );
    cx.update_window(win, |_, w, cx| w.within("presets-type").click(1usize, cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(
        last(cx, &host),
        Some(CollectionPresets::new("graphql", "https://api.test"))
    );
}

#[gpui_kit::test]
fn a_type_the_app_cannot_create_survives_a_url_edit(cx: &mut TestAppContext) {
    let (win, host) = section(cx, CollectionPresets::new("grpc", ""));
    replace_input(cx, win, None, "presets-url", "https://rpc.test");
    frame(cx, win);
    assert_eq!(
        last(cx, &host),
        Some(CollectionPresets::new("grpc", "https://rpc.test"))
    );
}

fn collection_view_presets(
    cx: &mut TestAppContext,
    model: &Entity<apinata_app::model::CollectionModel>,
) -> CollectionPresets {
    snap(cx, model)
        .content
        .collection
        .presets
        .clone()
        .expect("the collection's presets")
}

#[gpui_kit::test]
fn a_presets_edit_is_dirty_until_saved_and_keeps_the_environment(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", GRAPHQL)]);
    let model = open(cx, &dir);
    assert_eq!(
        collection_view_presets(cx, &model).request_url,
        "https://g.test"
    );
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Collection,
            edit: Edit::Presets(CollectionPresets::new("http", "https://h.test")),
        },
    );
    cx.run_until_parked();
    assert!(snap(cx, &model).content.collection.dirty);
    assert_eq!(collection_view_presets(cx, &model).request_type, "http");
    send(cx, &model, Command::Save(Target::Collection));
    cx.run_until_parked();
    let text = std::fs::read_to_string(dir.path().join("opencollection.yml")).unwrap();
    assert!(
        text.contains("type: http\n        url: https://h.test\n      defaultEnvironment: Local\n"),
        "{text}"
    );
}

fn editor(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: ConfigTarget,
) -> (AnyWindowHandle, Entity<ConfigEditor>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, target, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

#[gpui_kit::test]
fn the_collection_editor_offers_presets_and_saves_them(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    assert!(ed.read_with(cx, |e, _| e.sections().contains(&ConfigSection::Presets)));
    let (_w, folder_ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    assert!(!folder_ed.read_with(cx, |e, _| e.sections().contains(&ConfigSection::Presets)));

    ed.update(cx, |e, cx| e.set_section(ConfigSection::Presets, cx));
    frame(cx, win);
    replace_input(
        cx,
        win,
        Some("config-presets"),
        "presets-url",
        "https://api.test",
    );
    cx.run_until_parked();
    assert!(snap_dirty(cx, &ws, &root));
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Collection), cx)
    });
    cx.run_until_parked();
    let text = std::fs::read_to_string(dir.path().join("opencollection.yml")).unwrap();
    assert!(text.contains("url: https://api.test"), "{text}");
}

fn snap_dirty(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> bool {
    ws_snap(cx, ws, root).content.collection.dirty
}

fn tab_draft(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> ItemPath {
    ws.read_with(cx, |w, _| {
        w.tabs()
            .entries()
            .iter()
            .rev()
            .find_map(|e| e.item().cloned())
            .expect("a draft tab")
    })
}

#[gpui_kit::test]
fn a_new_request_starts_from_the_presets(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", GRAPHQL)]);
    let (_state, ws, root) = open_dir(cx, &dir);
    ws.update(cx, |w, cx| w.new_draft(&root, ItemPath::root(), cx))
        .expect("open");
    cx.run_until_parked();
    let draft = tab_draft(cx, &ws);
    let snapshot = ws_snap(cx, &ws, &root);
    let view = &snapshot.content.requests[&draft];
    assert_eq!(
        (view.kind, view.url.as_str()),
        (RequestKind::Graphql, "https://g.test")
    );
    assert!(!view.dirty);

    // An explicit kind keeps the kind and still takes the URL.
    ws.update(cx, |w, cx| {
        w.new_draft_of_kind(&root, ItemPath::root(), RequestKind::Http, cx)
    })
    .expect("open");
    cx.run_until_parked();
    let draft = tab_draft(cx, &ws);
    let snapshot = ws_snap(cx, &ws, &root);
    let view = &snapshot.content.requests[&draft];
    assert_eq!(
        (view.kind, view.url.as_str()),
        (RequestKind::Http, "https://g.test")
    );
}

#[gpui_kit::test]
fn the_preset_draft_op_answers_created(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", GRAPHQL)]);
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        Command::Tree(TreeOp::CreateDraftFromPresets {
            parent: ItemPath::root(),
        }),
    );
    cx.run_until_parked();
    let got = notices(cx, &model);
    let [NoticeKind::Created(path)] = got.as_slice() else {
        panic!("{got:?}")
    };
    let s = snap(cx, &model);
    assert_eq!(s.content.requests[path].kind, RequestKind::Graphql);
}
