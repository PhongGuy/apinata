//! The title bar's two environment pickers read as controls: each names its scope in a
//! tooltip and an icon, fills on hover, and lists its actions apart from its environments.

use crate::support;

use apinata_app::env_selector::{
    ACTIONS_GROUP, ENV_SELECT_TIP, ENVIRONMENTS_GROUP, IMPORT_ENVIRONMENT, MANAGE_ENVIRONMENTS,
    NO_ENVIRONMENT,
};
use apinata_app::global_env_selector::{
    GLOBAL_ENV_SELECT_TIP, GLOBALS_GROUP, NO_GLOBAL_ENVIRONMENT, manage_global_environments,
};
use apinata_app::shell::Shell;
use apinata_app::tabs::TabKind;
use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
use apinata_app::workspace::Workspace;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, SharedString, TestAppContext, WindowOptions,
};
use support::*;

struct Bar {
    _dirs: [tempfile::TempDir; 3],
    win: AnyWindowHandle,
    ws: Entity<Workspace>,
    shell: Entity<Shell>,
}

// A shell over a collection with two environments and a global directory with two globals.
fn bar(cx: &mut TestAppContext) -> Bar {
    bar_with(cx, &[("g.yml", "name: g\n"), ("h.yml", "name: h\n")])
}

// As `bar`, with `globals` as the global directory's files.
fn bar_with(cx: &mut TestAppContext, globals: &[(&str, &str)]) -> Bar {
    support::init(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let globals = write_tree(globals);
    let (state, ws, _root) = open_dir(cx, &col);
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    Bar {
        _dirs: [col, globals, state],
        win,
        ws,
        shell,
    }
}

fn tip_after_hover(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    target: &'static str,
    tip: &'static str,
) -> Option<String> {
    cx.update_window(win, |_, w, cx| w.within("title-bar").hover(target, cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(tip).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// Hovering either select names which environment it picks, so the two read apart.
#[gpui_kit::test]
fn each_picker_names_its_scope_in_a_tooltip(cx: &mut TestAppContext) {
    let b = bar(cx);
    assert_eq!(
        tip_after_hover(cx, b.win, "env-select", "env-control-tooltip").as_deref(),
        Some(ENV_SELECT_TIP)
    );
    assert_eq!(ENV_SELECT_TIP, "Collection environment");
    assert_eq!(
        tip_after_hover(
            cx,
            b.win,
            "global-env-select",
            "global-env-selector-tooltip"
        )
        .as_deref(),
        Some(GLOBAL_ENV_SELECT_TIP)
    );
    assert_eq!(GLOBAL_ENV_SELECT_TIP, "Global environment");
    cx.update_window(b.win, |_, w, _| {
        assert_eq!(w.find("env-select").label(), Some(ENV_SELECT_TIP));
        assert_eq!(
            w.find("global-env-select").label(),
            Some(GLOBAL_ENV_SELECT_TIP)
        );
    })
    .unwrap();
}

// Each trigger leads with its scope icon, drawn in the trigger's text colour, which holds 3:1
// against the title bar whether it shows a pick or the muted "none".
#[gpui_kit::test]
fn each_picker_shows_a_scope_icon_at_three_to_one(cx: &mut TestAppContext) {
    let b = bar(cx);
    cx.update_window(b.win, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.within("env-select").try_find("env-scope-icon").is_some());
        assert!(
            (w.within("global-env-select"))
                .try_find("global-env-scope-icon")
                .is_some()
        );
    })
    .unwrap();
    b.ws.update(cx, |w, cx| {
        let root = w.current_root().unwrap();
        w.select_environment(&root, Some("dev".into()), cx);
    });
    frame(cx, b.win);
    cx.update_window(b.win, |_, w, _| {
        assert!(w.within("env-select").try_find("env-scope-icon").is_some());
    })
    .unwrap();
    for theme in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        use_theme(cx, theme);
        let (muted, fg, title) = cx.update(|cx| {
            let t = cx.theme();
            (t.muted_foreground, t.foreground, t.title_bar)
        });
        for (name, c) in [("muted", muted), ("foreground", fg)] {
            let seen = contrast(c, title);
            assert!(
                seen >= 3.0,
                "{theme} {name} icon on the title bar = {seen:.2}"
            );
        }
    }
}

// Under the pointer each picker fills as the bar's ghost buttons do, a fill that differs from
// the bar in every theme.
#[gpui_kit::test]
fn hovering_a_picker_fills_it_in_every_theme(cx: &mut TestAppContext) {
    let b = bar(cx);
    for theme in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        use_theme(cx, theme);
        let (fill, title) =
            cx.update(|cx| (apinata_app::ui::ghost_hover_fill(cx), cx.theme().title_bar));
        assert_ne!(
            over(fill, title),
            title,
            "{theme}: the hover fill is the bar's own colour"
        );
        for control in ["env-control", "global-env-selector"] {
            let fills = cx
                .update_window(b.win, |_, w, cx| {
                    w.within("title-bar").hover(control, cx);
                    w.render_frame(cx);
                    let scale = w.scale_factor();
                    let r = w.find(control).bounds();
                    let (l, t) = (r.left().as_f32() * scale, r.top().as_f32() * scale);
                    w.painted_quads()
                        .into_iter()
                        .filter(|q| {
                            (q.bounds.origin.x.0 - l).abs() < 1.
                                && (q.bounds.origin.y.0 - t).abs() < 1.
                        })
                        .filter_map(|q| q.background.as_solid())
                        .collect::<Vec<_>>()
                })
                .unwrap();
            assert!(
                fills.contains(&fill),
                "{theme} {control}: painted {fills:?}, wanted {fill:?}"
            );
        }
    }
}

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

fn names(items: &[&str]) -> Vec<SharedString> {
    items.iter().map(|s| SharedString::from(*s)).collect()
}

// The environments form one group and the actions a second one after it.
#[gpui_kit::test]
fn actions_sit_in_their_own_group_after_the_environments(cx: &mut TestAppContext) {
    let b = bar(cx);
    let env = b.shell.read_with(cx, |s, _| s.env_selector().clone());
    assert_eq!(
        env.read_with(cx, |s, _| s.groups()),
        vec![
            (
                SharedString::from(ENVIRONMENTS_GROUP),
                names(&[NO_ENVIRONMENT, "dev", "prod"])
            ),
            (
                SharedString::from(ACTIONS_GROUP),
                names(&[MANAGE_ENVIRONMENTS, IMPORT_ENVIRONMENT])
            ),
        ]
    );
    let global = b
        .shell
        .read_with(cx, |s, _| s.global_env_selector().clone());
    assert_eq!(
        global.read_with(cx, |s, _| s.groups()),
        vec![
            (
                SharedString::from(GLOBALS_GROUP),
                names(&[NO_GLOBAL_ENVIRONMENT, "g", "h"])
            ),
            (
                SharedString::from(ACTIONS_GROUP),
                names(&[manage_global_environments()])
            ),
        ]
    );
}

// The open list draws a group header between the last environment and the first action, so
// that gap is wider than the one between two environments.
#[gpui_kit::test]
fn the_open_list_sets_the_actions_apart(cx: &mut TestAppContext) {
    let b = bar(cx);
    for (select, first, last, action) in [
        ("env-select", "env-none", "env-option-prod", "env-manage"),
        (
            "global-env-select",
            "global-env-none",
            "global-env-option-h",
            "global-env-manage",
        ),
    ] {
        cx.update_window(b.win, |_, w, cx| w.within("title-bar").click(select, cx))
            .unwrap();
        frame(cx, b.win);
        let (step, gap) = cx
            .update_window(b.win, |_, w, _| {
                let top = |id: &'static str| w.find(id).bounds().top().as_f32();
                let second = if select == "env-select" {
                    "env-option-dev"
                } else {
                    "global-env-option-g"
                };
                (top(second) - top(first), top(action) - top(last))
            })
            .unwrap();
        assert!(
            gap > step + 8.,
            "{select}: the action is {gap} below the last environment, rows step {step}"
        );
        cx.simulate_keystrokes(b.win, "escape");
        frame(cx, b.win);
    }
}

// A click on an action row still runs it, and a click on an environment row still picks it.
#[gpui_kit::test]
fn clicking_grouped_rows_runs_actions_and_picks_environments(cx: &mut TestAppContext) {
    let b = bar(cx);
    let root = b.ws.read_with(cx, |w, _| w.current_root().unwrap());
    let win = b.win;
    let pick = |cx: &mut TestAppContext, select: &'static str, row: &'static str| {
        cx.update_window(win, |_, w, cx| w.within("title-bar").click(select, cx))
            .unwrap();
        frame(cx, win);
        cx.update_window(win, |_, w, cx| w.click(row, cx)).unwrap();
        cx.run_until_parked();
        frame(cx, win);
    };
    pick(cx, "env-select", "env-option-prod");
    assert_eq!(
        b.ws.read_with(cx, |w, cx| w.selected_environment(&root, cx))
            .as_deref(),
        Some("prod")
    );
    pick(cx, "env-select", "env-manage");
    assert_eq!(
        b.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone())),
        Some(TabKind::Environments)
    );
    assert_eq!(
        b.ws.read_with(cx, |w, cx| w.selected_environment(&root, cx))
            .as_deref(),
        Some("prod"),
        "managing keeps the pick"
    );
    pick(cx, "global-env-select", "global-env-option-h");
    assert_eq!(
        b.ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("h")
    );
    pick(cx, "global-env-select", "global-env-manage");
    assert!(present(cx, b.win, "global-env-editor"));
    assert_eq!(
        b.ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("h"),
        "managing keeps the pick"
    );
}

// The group headers read as labels, not rows: their own ids and names, shorter than a row,
// and a rule above the actions; their muted text holds the theme's text floor on the popup.
#[gpui_kit::test]
fn group_headers_read_as_labels(cx: &mut TestAppContext) {
    let b = bar(cx);
    for (select, prefix, first, second, titles) in [
        (
            "env-select",
            "env-group",
            "env-none",
            "env-option-dev",
            [ENVIRONMENTS_GROUP, ACTIONS_GROUP],
        ),
        (
            "global-env-select",
            "global-env-group",
            "global-env-none",
            "global-env-option-g",
            [GLOBALS_GROUP, ACTIONS_GROUP],
        ),
    ] {
        cx.update_window(b.win, |_, w, cx| w.within("title-bar").click(select, cx))
            .unwrap();
        frame(cx, b.win);
        let border = cx.update(|cx| cx.theme().border);
        cx.update_window(b.win, |_, w, cx| {
            w.render_frame(cx);
            for (ix, title) in titles.iter().enumerate() {
                assert_eq!(w.find((prefix, ix)).label(), Some(*title), "{select} {ix}");
            }
            let top = |id: &'static str| w.find(id).bounds().top().as_f32();
            let step = top(second) - top(first);
            let header = w.find((prefix, 1usize)).bounds();
            let tall = header.size.height.as_f32();
            assert!(
                tall < step,
                "{select}: a header {tall} tall is no shorter than a row {step}"
            );
            let scale = w.scale_factor();
            let (l, t) = (
                header.left().as_f32() * scale,
                header.top().as_f32() * scale,
            );
            let rule = w.painted_quads().into_iter().any(|q| {
                q.border_widths.top.0 > 0.
                    && q.border_color == border
                    && (q.bounds.origin.x.0 - l).abs() < 1.
                    && (q.bounds.origin.y.0 - t).abs() < 1.
            });
            assert!(rule, "{select}: no rule above the actions");
        })
        .unwrap();
        cx.simulate_keystrokes(b.win, "escape");
        frame(cx, b.win);
    }
    for theme in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        use_theme(cx, theme);
        let (muted, popup) = cx.update(|cx| (cx.theme().muted_foreground, cx.theme().popover));
        let floor = if theme == HC_LIGHT || theme == HC_DARK {
            7.0
        } else {
            4.5
        };
        let seen = contrast(muted, popup);
        assert!(
            seen >= floor,
            "{theme}: header text on the popup = {seen:.2}"
        );
    }
}

// With globals to pick from and none picked, the picker names the choice the way the
// collection picker does, rather than reading as if none exist.
#[gpui_kit::test]
fn with_none_picked_the_global_picker_says_no_global_environment(cx: &mut TestAppContext) {
    let b = bar(cx);
    assert_eq!(NO_GLOBAL_ENVIRONMENT, "No global environment");
    let global = b
        .shell
        .read_with(cx, |s, _| s.global_env_selector().clone());
    assert_eq!(
        global.read_with(cx, |s, _| s.shown()).as_deref(),
        Some(NO_GLOBAL_ENVIRONMENT)
    );
}

// A name cut short in the open list keeps a gap before the check mark beside it.
#[gpui_kit::test]
fn a_cut_name_keeps_a_gap_before_the_check(cx: &mut TestAppContext) {
    let long = "shared-staging-credentials-for-the-payments-team";
    let (path, file) = (format!("{long}.yml"), format!("name: {long}\n"));
    let b = bar_with(cx, &[(&path, &file)]);
    b.ws.update(cx, |w, cx| w.select_picker_item(long, cx));
    frame(cx, b.win);
    cx.update_window(b.win, |_, w, cx| {
        w.within("title-bar").click("global-env-select", cx)
    })
    .unwrap();
    frame(cx, b.win);
    let option: &'static str = format!("global-env-option-{long}").leak();
    let (name, row) = cx
        .update_window(b.win, |_, w, _| {
            let name = (w.within(option).find("global-env-row-name").bounds())
                .right()
                .as_f32();
            (name, w.find(option).bounds().right().as_f32())
        })
        .unwrap();
    assert!(
        row - name >= 4.,
        "the cut name ends {} before the check",
        row - name
    );
}

// The tip names the picked environment after the scope, so a name the trigger cuts short
// can still be read in full.
#[gpui_kit::test]
fn the_tip_names_the_picked_environment(cx: &mut TestAppContext) {
    let b = bar(cx);
    b.ws.update(cx, |w, cx| {
        let root = w.current_root().unwrap();
        w.select_environment(&root, Some("prod".into()), cx);
        w.select_picker_item("h", cx);
    });
    frame(cx, b.win);
    assert_eq!(
        tip_after_hover(cx, b.win, "env-select", "env-control-tooltip").as_deref(),
        Some("Collection environment: prod")
    );
    assert_eq!(
        tip_after_hover(
            cx,
            b.win,
            "global-env-select",
            "global-env-selector-tooltip"
        )
        .as_deref(),
        Some("Global environment: h")
    );
}
