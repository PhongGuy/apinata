//! The request bar's undefined-variable badge.

use crate::support;

use apinata_app::set_variable::{self, VarTarget};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

const BADGE: &str = "undefined-variables";
const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

fn badge_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(BADGE)
            .map(|e| e.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_undefined_name_shows_the_badge_and_a_runtime_value_clears_it(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{token}}/{{id}}")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert_eq!(
        badge_label(cx, win).as_deref(),
        Some("Undefined variables: id, token")
    );
    cx.update(|cx| set_variable::apply(&ws, &root, "id", "1", &VarTarget::Runtime, cx))
        .unwrap();
    assert_eq!(
        badge_label(cx, win).as_deref(),
        Some("Undefined variable: token")
    );
    cx.update(|cx| set_variable::apply(&ws, &root, "token", "t", &VarTarget::Runtime, cx))
        .unwrap();
    assert_eq!(badge_label(cx, win), None);
}

#[gpui_kit::test]
fn selecting_an_environment_that_defines_the_name_clears_the_badge(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://{{host}}/0")),
        ("environments/dev.yml", DEV),
        // A second environment, so the only one is not selected on open.
        ("environments/prod.yml", "name: prod\nvariables: []\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert_eq!(
        badge_label(cx, win).as_deref(),
        Some("Undefined variable: host")
    );
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    assert_eq!(badge_label(cx, win), None);
}

#[gpui_kit::test]
fn a_name_a_script_sets_shows_no_badge(cx: &mut TestAppContext) {
    let script = "runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        bru.setVar('later', 1)\n";
    let text = request("a", "http://x/{{later}}") + script;
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert_eq!(badge_label(cx, win), None);
}

#[gpui_kit::test]
fn a_request_with_every_name_defined_shows_no_badge(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert_eq!(badge_label(cx, win), None);
}
