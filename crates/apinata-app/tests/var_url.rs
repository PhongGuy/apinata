use crate::support;

use std::ops::Range;

use apinata_app::command::{Command, EnvOp};
use gpui_kit::AppContext as _;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Entity, InputEvent as _, Modifiers, MouseMoveEvent, TestAppContext,
};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n";
const DEV: &str = "name: dev\nvariables:\n  - name: nope\n    value: here\n";
const HOST: Range<usize> = 8..16;
const NOPE: Range<usize> = 17..25;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &request("a", "https://{{host}}/{{nope}}")),
    ])
}

fn hover_tip(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    input: &Entity<InputState>,
    range: Range<usize>,
) -> Option<String> {
    let at = input
        .read_with(cx, |s, _| s.range_to_bounds(&range))
        .expect("laid out")
        .center();
    cx.update_window(win, |_, w, cx| {
        let event = MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        };
        w.dispatch_event(event.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("var-tooltip")
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_url_bar_explains_each_placeholder_and_follows_the_environment(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    clear_environment(cx, &ws, &root);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    let url = ed.read_with(cx, |e, _| e.url_input().clone());
    assert_eq!(
        hover_tip(cx, win, &url, HOST).as_deref(),
        Some("Collection · c.test")
    );
    assert_eq!(
        hover_tip(cx, win, &url, NOPE).as_deref(),
        Some("Undefined variable")
    );
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    wait_for(cx, "the environment's variables", |cx| {
        ed.read_with(cx, |e, _| {
            e.variables().is_some_and(|v| v.get("nope").is_some())
        })
    });
    assert_eq!(
        hover_tip(cx, win, &url, NOPE).as_deref(),
        Some("Environment · here")
    );
}

const SECRET_DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: key\n";

#[gpui_kit::test]
fn storing_a_secret_value_defines_its_placeholder(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("environments/dev.yml", SECRET_DEV),
        ("a.yml", &request("a", "https://{{key}}")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    let url = ed.read_with(cx, |e, _| e.url_input().clone());
    assert_eq!(
        hover_tip(cx, win, &url, 8..15).as_deref(),
        Some("Undefined variable")
    );
    let store = Command::Env(EnvOp::SetSecretValue {
        env: "dev".into(),
        var: "key".into(),
        value: "s3cret".into(),
    });
    ws.update(cx, |w, cx| w.send(&root, store, cx));
    wait_for(cx, "the stored secret", |cx| {
        ed.read_with(cx, |e, _| {
            e.variables().is_some_and(|v| v.get("key").is_some())
        })
    });
    assert_eq!(
        hover_tip(cx, win, &url, 8..15).as_deref(),
        Some("Environment · ••••••")
    );
}

#[gpui_kit::test]
fn a_failing_context_drops_the_variable_set(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("missing".into()), cx)
    });
    wait_for(cx, "the set to drop", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_none())
    });
}
