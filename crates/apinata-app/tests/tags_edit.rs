//! Tags set through the edit path the tag input uses: they normalise, save, reload, and
//! filter a run.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::runner_model::RunnerModel;
use apinata_core::Collection;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/one.yml", &request("one", "http://127.0.0.1:9/one")),
        ("other.yml", &request("other", "http://127.0.0.1:9/other")),
    ])
}

fn tags(list: &[&str]) -> Edit {
    Edit::Tags(list.iter().map(|t| (*t).to_owned()).collect())
}

fn edit_and_save(cx: &mut TestAppContext, dir: &tempfile::TempDir, target: Target, edit: Edit) {
    let model = open(cx, dir);
    send(
        cx,
        &model,
        Command::Edit {
            target: target.clone(),
            edit,
        },
    );
    cx.run_until_parked();
    send(cx, &model, Command::Save(target));
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), []);
}

#[gpui_kit::test]
fn request_tags_save_normalised_and_reload(cx: &mut TestAppContext) {
    let dir = tree();
    let target = Target::Request(ip("other.yml"));
    edit_and_save(cx, &dir, target, tags(&[" smoke ", "", "v2", "smoke"]));
    let c = Collection::open(dir.path()).unwrap();
    assert_eq!(c.request(&ip("other.yml")).unwrap().tags(), ["smoke", "v2"]);
    // The snapshot a reopened app shows carries them.
    let model = open(cx, &dir);
    let view = snap(cx, &model).content.requests[&ip("other.yml")].clone();
    assert_eq!(view.tags, ["smoke", "v2"]);
}

#[gpui_kit::test]
fn clearing_request_tags_removes_the_key(cx: &mut TestAppContext) {
    let dir = tree();
    let target = Target::Request(ip("other.yml"));
    edit_and_save(cx, &dir, target.clone(), tags(&["a"]));
    edit_and_save(cx, &dir, target, tags(&[]));
    let text = std::fs::read_to_string(dir.path().join("other.yml")).unwrap();
    assert!(!text.contains("tags"), "{text}");
}

#[gpui_kit::test]
fn folder_tags_save_normalised_and_reload(cx: &mut TestAppContext) {
    let dir = tree();
    let target = Target::Folder(ip("api"));
    edit_and_save(cx, &dir, target, tags(&["  smoke", "smoke", "v2 "]));
    let text = std::fs::read_to_string(dir.path().join("api/folder.yml")).unwrap();
    assert!(text.contains("tags:\n    - smoke\n    - v2\n"), "{text}");
    let model = open(cx, &dir);
    let view = snap(cx, &model).content.folders[&ip("api")].clone();
    assert_eq!(view.tags, ["smoke", "v2"]);
    assert!(!view.dirty);
}

#[gpui_kit::test]
fn a_folder_tag_edit_marks_the_folder_dirty_until_saved(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let target = Target::Folder(ip("api"));
    send(
        cx,
        &model,
        Command::Edit {
            target,
            edit: tags(&["x"]),
        },
    );
    cx.run_until_parked();
    let view = snap(cx, &model).content.folders[&ip("api")].clone();
    assert!(view.dirty);
    assert_eq!(view.tags, ["x"]);
}

#[gpui_kit::test]
fn the_collection_refuses_tags(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Collection,
            edit: tags(&["x"]),
        },
    );
    cx.run_until_parked();
    assert_eq!(notices(cx, &model).len(), 1);
}

#[gpui_kit::test]
fn a_tag_set_through_the_edit_path_filters_a_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let dir = tree();
    let (_state, ws, root) = open_dir(cx, &dir);
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Request(ip("other.yml")),
                edit: tags(&[" smoke "]),
            },
            cx,
        )
    });
    cx.run_until_parked();
    // The runner previews the saved plan, so the edit is saved first.
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Request(ip("other.yml"))), cx)
    });
    cx.run_until_parked();
    let (w, r) = (ws.clone(), root.clone());
    let m = cx.new(|_| RunnerModel::new(w, r, ip("")));
    m.update(cx, |m, cx| {
        m.set_tags("smoke".into(), cx);
        m.refresh_preview(cx)
    });
    wait_real(cx, |cx| m.read_with(cx, |m, _| m.preview().len() == 1));
    let paths: Vec<String> = m.read_with(cx, |m, _| {
        m.preview()
            .iter()
            .map(|r| r.path.as_str().to_owned())
            .collect()
    });
    assert_eq!(paths, ["other.yml"]);
}
