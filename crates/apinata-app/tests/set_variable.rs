//! "Set as variable...": the name prefill, the targets, the writes and the dialog.

use crate::support;

use apinata_app::body_node::{node_copies, prefill_name};
use apinata_app::command::Command;
use apinata_app::set_variable::{self, SetVariableForm, VarTarget};
use apinata_app::workspace::Workspace;
use apinata_core::{VarScope, VariableSet};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, Window, WindowOptions,
    div,
};
use std::path::Path;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: key\n";
const JSON: &str = r#"{"user":{"name":"ann","tags":["a","b"]},"n":1.50}"#;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://{{token}}/0")),
        ("environments/dev.yml", DEV),
    ])
}

fn selected(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) {
    ws.update(cx, |w, cx| {
        w.select_environment(root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
}

fn apply(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    name: &str,
    target: VarTarget,
) -> Result<(), String> {
    let result = cx.update(|cx| set_variable::apply(ws, root, name, "abc", &target, cx));
    cx.run_until_parked();
    result
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

fn resolved(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    name: &str,
) -> Option<(VarScope, Option<String>)> {
    let (reply, answer) = async_channel::bounded(1);
    let command = Command::Variables {
        item: ip("a.yml"),
        reply,
    };
    ws.update(cx, |w, cx| w.send(root, command, cx));
    cx.run_until_parked();
    let set: VariableSet = answer.try_recv().expect("a reply").expect("built");
    set.get(name)
        .map(|v| (v.scope(), v.value().map(str::to_owned)))
}

#[test]
fn the_name_is_prefilled_from_the_nodes_key() {
    let at = |needle: &str| {
        let node = apinata_core::json_node_at(JSON, JSON.find(needle).unwrap()).unwrap();
        prefill_name(node.path.as_deref())
    };
    assert_eq!(at("ann"), "name");
    assert_eq!(at("\"user\""), "user");
    assert_eq!(at("[\"a"), "tags");
    assert_eq!(at("\"b\""), "tags");
    assert_eq!(prefill_name(Some("res.body")), "");
    assert_eq!(prefill_name(None), "");
    assert_eq!(prefill_name(Some("res.body[\"a b\"]")), "a_b");
}

#[test]
fn the_node_menu_model_offers_set_as_variable_last() {
    let copies = node_copies(JSON, JSON.find("ann").unwrap());
    let labels: Vec<_> = copies.iter().map(|c| c.label).collect();
    assert_eq!(labels, ["Copy value", "Copy path", "Set as variable..."]);
    let set = &copies[2];
    assert_eq!(set.text, "ann");
    assert_eq!(set.variable.as_deref(), Some("name"));
    assert!(copies[..2].iter().all(|c| c.variable.is_none()));
}

#[test]
fn the_targets_never_include_a_secret_and_lead_with_the_active_environment() {
    let with = set_variable::targets(Some("dev"));
    assert_eq!(
        with,
        [
            VarTarget::Environment("dev".into()),
            VarTarget::Collection,
            VarTarget::Runtime
        ]
    );
    assert_eq!(
        set_variable::targets(None),
        [VarTarget::Collection, VarTarget::Runtime]
    );
}

#[gpui_kit::test]
fn the_environment_target_writes_the_active_environment_file(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    apply(
        cx,
        &ws,
        &root,
        "token",
        VarTarget::Environment("dev".into()),
    )
    .unwrap();
    wait_for(cx, "the save", |_| {
        read(&root, "environments/dev.yml").contains("token")
    });
    let disk = read(&root, "environments/dev.yml");
    assert!(
        disk.contains("name: token") && disk.contains("value: abc"),
        "{disk}"
    );
    assert!(
        disk.contains("name: host"),
        "the other variables stay: {disk}"
    );
    assert_eq!(
        resolved(cx, &ws, &root, "token"),
        Some((VarScope::Environment, Some("abc".into())))
    );
}

#[gpui_kit::test]
fn an_existing_environment_variable_is_replaced_not_duplicated(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    apply(cx, &ws, &root, "host", VarTarget::Environment("dev".into())).unwrap();
    wait_for(cx, "the save", |_| {
        read(&root, "environments/dev.yml").contains("abc")
    });
    assert_eq!(
        read(&root, "environments/dev.yml")
            .matches("name: host")
            .count(),
        1
    );
}

#[gpui_kit::test]
fn a_secret_of_the_environment_is_never_overwritten(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    let before = read(&root, "environments/dev.yml");
    let result = apply(cx, &ws, &root, "key", VarTarget::Environment("dev".into()));
    assert!(result.is_err());
    cx.run_until_parked();
    assert_eq!(read(&root, "environments/dev.yml"), before);
}

#[gpui_kit::test]
fn the_collection_target_writes_the_collection_file(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    apply(cx, &ws, &root, "token", VarTarget::Collection).unwrap();
    wait_for(cx, "the save", |_| {
        read(&root, "opencollection.yml").contains("token")
    });
    let disk = read(&root, "opencollection.yml");
    assert!(
        disk.contains("name: token") && disk.contains("value: abc"),
        "{disk}"
    );
    assert_eq!(
        resolved(cx, &ws, &root, "token"),
        Some((VarScope::Collection, Some("abc".into())))
    );
}

#[gpui_kit::test]
fn the_runtime_target_writes_no_file_and_resolves_on_the_next_send(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    let files = ["opencollection.yml", "environments/dev.yml", "a.yml"].map(|f| read(&root, f));
    apply(cx, &ws, &root, "token", VarTarget::Runtime).unwrap();
    assert_eq!(
        resolved(cx, &ws, &root, "token"),
        Some((VarScope::Runtime, Some("abc".into())))
    );
    let after = ["opencollection.yml", "environments/dev.yml", "a.yml"].map(|f| read(&root, f));
    assert_eq!(files, after);
    let snap = ws_snap(cx, &ws, &root);
    assert!(!snap.content.collection.dirty);
}

#[gpui_kit::test]
fn a_blank_or_malformed_name_is_refused(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    assert!(apply(cx, &ws, &root, "  ", VarTarget::Runtime).is_err());
    assert!(apply(cx, &ws, &root, "a b", VarTarget::Runtime).is_err());
}

#[gpui_kit::test]
fn names_outside_the_shared_variable_rule_are_refused(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    for name in ["{{tok}}", "a}}", "caf\u{e9}"] {
        assert!(
            apply(cx, &ws, &root, name, VarTarget::Runtime).is_err(),
            "{name}"
        );
    }
}

struct Blank;
impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn dialog(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
) -> (gpui_kit::AnyWindowHandle, Entity<SetVariableForm>) {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let form = cx
        .update_window(win, |_, w, cx| {
            set_variable::open(&ws, &root, "name", "ann", w, cx)
        })
        .unwrap();
    frame(cx, win);
    (win, form)
}

#[gpui_kit::test]
fn the_dialog_shows_the_prefilled_name_and_the_value_and_enter_confirms(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    let (win, form) = dialog(cx, &ws, &root);
    assert!(present(cx, win, "set-variable-dialog"));
    assert!(present(cx, win, "set-variable-value"));
    let answer = form.read_with(cx, |f, cx| f.answer(cx)).expect("a name");
    assert_eq!(answer.name, "name");
    assert_eq!(answer.target, VarTarget::Environment("dev".into()));
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    wait_for(cx, "the save", |_| {
        read(&root, "environments/dev.yml").contains("name: name")
    });
    frame(cx, win);
    assert!(!present(cx, win, "set-variable-dialog"));
}

#[gpui_kit::test]
fn escape_cancels_without_writing(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    let before = read(&root, "environments/dev.yml");
    let (win, _form) = dialog(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, win);
    assert!(!present(cx, win, "set-variable-dialog"));
    assert_eq!(read(&root, "environments/dev.yml"), before);
    assert!(
        !cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
}

// Enter in the value field confirms (above), but on a focused Cancel it presses Cancel.
#[gpui_kit::test]
fn enter_on_the_focused_cancel_button_cancels_without_writing(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    let before = read(&root, "environments/dev.yml");
    let (win, _form) = dialog(cx, &ws, &root);
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, "set-variable-cancel");
    frame(cx, win);
    assert!(!present(cx, win, "set-variable-dialog"));
    assert_eq!(read(&root, "environments/dev.yml"), before);
}

#[gpui_kit::test]
fn a_dialog_without_an_environment_defaults_to_the_collection(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, form) = dialog(cx, &ws, &root);
    let answer = form.read_with(cx, |f, cx| f.answer(cx)).expect("a name");
    assert_eq!(answer.target, VarTarget::Collection);
}

#[gpui_kit::test]
fn the_response_pane_opens_the_dialog_when_the_body_asks(cx: &mut TestAppContext) {
    use apinata_app::body_view::SetVariable;
    use apinata_app::response_pane::ResponsePane;
    http_ready(cx);
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let body = pane.read_with(cx, |p, _| p.body().clone());
    body.update(cx, |_, cx| {
        cx.emit(SetVariable {
            name: "name".into(),
            value: "ann".into(),
            sensitive: false,
        })
    });
    frame(cx, win);
    assert!(present(cx, win, "set-variable-dialog"));
}

#[gpui_kit::test]
fn enter_on_the_focused_target_select_opens_it_without_writing(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    selected(cx, &ws, &root);
    let before = read(&root, "environments/dev.yml");
    let (win, _form) = dialog(cx, &ws, &root);
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, "set-variable-target");
    frame(cx, win);
    assert!(
        present(cx, win, "set-variable-dialog"),
        "the dialog stays open"
    );
    let open = cx
        .update_window(win, |_, w, _| w.find("set-variable-target").expanded())
        .unwrap();
    assert_eq!(open, Some(true), "Enter opens the targets");
    assert_eq!(read(&root, "environments/dev.yml"), before);
}
