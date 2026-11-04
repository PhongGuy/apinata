//! A snapshot that predates a section edit must not overwrite what the user typed:
//! each case types, lets it ack, types again, then refreshes against the older snapshot.

use crate::support;

use apinata_app::auth_section::AuthField;
use apinata_app::request_editor::{RequestEditor, Section};
use apinata_core::{AuthSpec, BodySpec};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0\"\n  body:\n    type: json\n    data: \"{}\"\n  auth:\n    type: bearer\n    token: t\n";

fn open_section(cx: &mut TestAppContext, win: AnyWindowHandle, s: Section) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", s as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
}

fn body_code(cx: &mut TestAppContext, ed: &Entity<RequestEditor>) -> (String, usize) {
    let code = ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    code.read_with(cx, |c, _| (c.value().to_string(), c.cursor()))
}

#[gpui_kit::test]
fn a_stale_snapshot_keeps_body_typing(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_s, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    open_section(cx, win, Section::Body);
    replace_code(cx, win, Some("body"), "body-code", "ab");
    cx.run_until_parked();
    assert_eq!(
        view_of(cx, &ws, &root, "a.yml").body,
        BodySpec::Json("ab".into())
    );

    // "cd" is in flight while the workspace refreshes against the snapshot holding "ab".
    cx.update_window(win, |_, w, cx| w.input("cd", cx)).unwrap();
    let before = body_code(cx, &ed);
    ws.update(cx, |_, cx| cx.notify());
    assert_eq!(
        body_code(cx, &ed),
        before,
        "older snapshot overwrote the body"
    );
    cx.run_until_parked();
    assert_eq!(body_code(cx, &ed), before);
    assert_eq!(
        view_of(cx, &ws, &root, "a.yml").body,
        BodySpec::Json(before.0.clone())
    );
}

#[gpui_kit::test]
fn a_stale_snapshot_keeps_auth_and_docs_typing(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_s, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);

    open_section(cx, win, Section::Auth);
    type_at_end(cx, win, Some("auth"), "auth-token", "1");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.input("2", cx)).unwrap();
    ws.update(cx, |_, cx| cx.notify());
    let tok = ed.read_with(cx, |e, cx| {
        e.auth().read(cx).input(AuthField::Token).clone()
    });
    assert_eq!(tok.read_with(cx, |i, _| i.value().to_string()), "t12");
    cx.run_until_parked();
    assert_eq!(
        view_of(cx, &ws, &root, "a.yml").auth,
        AuthSpec::Bearer {
            token: "t12".into()
        }
    );

    open_section(cx, win, Section::Docs);
    replace_code(cx, win, Some("docs"), "docs-code", "no");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.input("tes", cx))
        .unwrap();
    ws.update(cx, |_, cx| cx.notify());
    let docs = ed.read_with(cx, |e, cx| e.docs().read(cx).code().clone());
    assert_eq!(docs.read_with(cx, |c, _| c.value().to_string()), "notes");
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").docs, "notes");
}
