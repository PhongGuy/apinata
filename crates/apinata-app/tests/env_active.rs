//! The collection's environment manager marks the environment requests are sent with apart
//! from the row being edited, offers to make the edited one active, activates a new
//! environment only when nothing was chosen, and hides New while a name is being typed.

use crate::support;

use apinata_app::env_editor::{EnvEditor, active_tag_color};
use apinata_app::session::{CollectionSession, Session};
use apinata_app::theme;
use apinata_app::workspace::Workspace;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;

const THEMES: [&str; 4] = [theme::DARK, theme::LIGHT, theme::HC_DARK, theme::HC_LIGHT];

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ])
}

struct Opened {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: PathBuf,
    win: AnyWindowHandle,
    ed: Entity<EnvEditor>,
}

// An editor over dev and prod, opened from a session holding `start`.
fn opened(cx: &mut TestAppContext, start: CollectionSession) -> Opened {
    let col = files();
    let state = tempfile::tempdir().unwrap();
    let root = col.path().to_path_buf();
    let mut s = Session::default();
    s.collections.insert(root.clone(), start);
    s.open.push(root.clone());
    let session = session_at(cx, &state, s);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    let (win, ed) = env_editor(cx, &ws, &root);
    Opened {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    }
}

fn active(cx: &mut TestAppContext, o: &Opened) -> Option<String> {
    o.ws.read_with(cx, |w, cx| w.selected_environment(&o.root, cx))
}

fn pick(cx: &mut TestAppContext, o: &Opened, name: Option<&str>) {
    o.ws.update(cx, |w, cx| {
        w.select_environment(&o.root, name.map(str::to_owned), cx)
    });
    cx.run_until_parked();
}

fn edit(cx: &mut TestAppContext, o: &Opened, name: &str) {
    cx.update_window(o.win, |_, w, cx| {
        o.ed.update(cx, |e, cx| e.select(name, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

// The rows carrying the Active tag, by index.
fn tagged(cx: &mut TestAppContext, o: &Opened) -> Vec<usize> {
    (0..2)
        .filter(|&ix| present(cx, o.win, ("env-item-active", ix)))
        .collect()
}

fn create(cx: &mut TestAppContext, o: &Opened, name: &str) {
    cx.update_window(o.win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_input(o.win, name);
    cx.simulate_keystrokes(o.win, "enter");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_tag_follows_the_title_bar_picker_apart_from_the_selection(cx: &mut TestAppContext) {
    let o = opened(cx, CollectionSession::default());
    assert_eq!(active(cx, &o), None);
    assert!(tagged(cx, &o).is_empty(), "nothing is active yet");
    pick(cx, &o, Some("dev"));
    edit(cx, &o, "prod");
    assert_eq!(tagged(cx, &o), [0], "dev is active while prod is edited");
    pick(cx, &o, Some("prod"));
    assert_eq!(tagged(cx, &o), [1]);
    pick(cx, &o, None);
    assert!(tagged(cx, &o).is_empty());
}

#[gpui_kit::test]
fn the_tag_reads_active(cx: &mut TestAppContext) {
    let o = opened(cx, CollectionSession::default());
    pick(cx, &o, Some("dev"));
    let label = cx
        .update_window(o.win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("env-item-active", 0usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Active"));
}

#[gpui_kit::test]
fn use_this_environment_activates_it_and_then_hides(cx: &mut TestAppContext) {
    let o = opened(cx, CollectionSession::default());
    pick(cx, &o, Some("dev"));
    edit(cx, &o, "dev");
    assert!(
        !present(cx, o.win, "env-use"),
        "the active one offers nothing"
    );
    edit(cx, &o, "prod");
    assert!(present(cx, o.win, "env-use"));
    cx.update_window(o.win, |_, w, cx| w.click("env-use", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &o).as_deref(), Some("prod"));
    assert!(!present(cx, o.win, "env-use"));
    assert_eq!(tagged(cx, &o), [1]);
}

#[gpui_kit::test]
fn a_new_environment_becomes_active_when_none_was_chosen(cx: &mut TestAppContext) {
    let o = opened(cx, CollectionSession::default());
    create(cx, &o, "qa");
    assert_eq!(active(cx, &o).as_deref(), Some("qa"));
}

#[gpui_kit::test]
fn a_new_environment_keeps_an_explicit_no_environment(cx: &mut TestAppContext) {
    let start = CollectionSession {
        no_environment: true,
        ..Default::default()
    };
    let o = opened(cx, start);
    assert_eq!(active(cx, &o), None);
    create(cx, &o, "qa");
    assert_eq!(
        o.ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("qa")
    );
    assert_eq!(active(cx, &o), None);
    assert!(present(cx, o.win, "env-use"), "it can still be made active");
}

#[gpui_kit::test]
fn a_new_environment_leaves_another_active_one(cx: &mut TestAppContext) {
    let start = CollectionSession {
        environment: Some("dev".into()),
        ..Default::default()
    };
    let o = opened(cx, start);
    assert_eq!(active(cx, &o).as_deref(), Some("dev"));
    create(cx, &o, "qa");
    assert_eq!(active(cx, &o).as_deref(), Some("dev"));
}

#[gpui_kit::test]
fn new_hides_while_the_name_field_is_open(cx: &mut TestAppContext) {
    let o = opened(cx, CollectionSession::default());
    assert!(present(cx, o.win, "env-create"));
    cx.update_window(o.win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, o.win, "env-name-input"));
    assert!(
        !present(cx, o.win, "env-create"),
        "one create control at a time"
    );
    cx.simulate_keystrokes(o.win, "escape");
    cx.run_until_parked();
    assert!(!present(cx, o.win, "env-name-input"));
    assert!(present(cx, o.win, "env-create"));
}

// The tag's text reads at 4.5:1 on a plain, hovered and selected row in all four themes.
#[gpui_kit::test]
fn the_tag_is_readable_in_every_theme(cx: &mut TestAppContext) {
    let o = opened(cx, CollectionSession::default());
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(o.win, |_, _, cx| {
            let t = cx.theme();
            let tag = active_tag_color(cx);
            for (row, fill) in [
                ("plain", t.background),
                ("hovered", t.list_hover),
                ("selected", t.list_active),
            ] {
                let seen = contrast(tag, over(fill, t.background));
                assert!(seen >= 4.5, "{name} {row}: {seen}");
            }
        })
        .unwrap();
    }
}

// `top` laid over `under`, as the renderer blends it.
fn over(top: gpui_kit::Hsla, under: gpui_kit::Hsla) -> gpui_kit::Hsla {
    let (t, u) = (top.to_rgb(), under.to_rgb());
    let mix = |a: f32, b: f32| a * top.a + b * (1. - top.a);
    gpui_kit::Rgba {
        r: mix(t.r, u.r),
        g: mix(t.g, u.g),
        b: mix(t.b, u.b),
        a: 1.,
    }
    .into()
}
