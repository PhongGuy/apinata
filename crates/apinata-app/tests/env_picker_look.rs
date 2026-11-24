//! The title bar's environment picker reads as a control: it names itself in a tooltip and an
//! icon, fills on hover, and sets its two sections and its actions apart in the open menu.

use crate::support;

use apinata_app::env_picker::{
    ACTIONS_GROUP, COLLECTION_GROUP, ENV_SELECT_TIP, GLOBAL_GROUP, NO_GLOBAL_ENVIRONMENT,
};
use apinata_app::shell::Shell;
use apinata_app::tabs::TabKind;
use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
use apinata_app::workspace::Workspace;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
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

// Hovers the picker from elsewhere, so a tip shown before is built afresh.
fn tip_after_hover(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| w.within("title-bar").hover("app-menu", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").hover("env-select", cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find("env-control-tooltip")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn open(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("env-select", cx)
    })
    .unwrap();
    frame(cx, win);
}

// The tip names the picker, then both picks in full, since the button may cut them short.
#[gpui_kit::test]
fn the_tip_names_the_picker_and_both_picks(cx: &mut TestAppContext) {
    let b = bar(cx);
    assert_eq!(tip_after_hover(cx, b.win).as_deref(), Some(ENV_SELECT_TIP));
    assert_eq!(ENV_SELECT_TIP, "Environment");
    b.ws.update(cx, |w, cx| {
        let root = w.current_root().unwrap();
        w.select_environment(&root, Some("prod".into()), cx);
        w.select_picker_item("h", cx);
    });
    frame(cx, b.win);
    assert_eq!(
        tip_after_hover(cx, b.win).as_deref(),
        Some("Environment: prod · h")
    );
    cx.update_window(b.win, |_, w, _| {
        assert_eq!(w.find("env-select").label(), Some("Environment: prod · h"));
    })
    .unwrap();
}

// The button leads with its scope icon, drawn in the button's text colour, which holds 3:1
// against the title bar whether it shows a pick or the muted "No environment".
#[gpui_kit::test]
fn the_picker_shows_a_scope_icon_at_three_to_one(cx: &mut TestAppContext) {
    let b = bar(cx);
    cx.update_window(b.win, |_, w, cx| {
        w.render_frame(cx);
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

// Under the pointer the picker fills as the bar's ghost buttons do, a fill that differs from
// the bar in every theme.
#[gpui_kit::test]
fn hovering_the_picker_fills_it_in_every_theme(cx: &mut TestAppContext) {
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
        let fills = cx
            .update_window(b.win, |_, w, cx| {
                w.within("title-bar").hover("env-select", cx);
                w.render_frame(cx);
                let scale = w.scale_factor();
                let r = w.find("env-select").bounds();
                let (l, t) = (r.left().as_f32() * scale, r.top().as_f32() * scale);
                crate::support::whole_quads(w)
                    .into_iter()
                    .filter(|q| {
                        (q.bounds.origin.x.0 - l).abs() < 1. && (q.bounds.origin.y.0 - t).abs() < 1.
                    })
                    .filter_map(|q| q.background.as_solid())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert!(
            fills.contains(&fill),
            "{theme}: painted {fills:?}, wanted {fill:?}"
        );
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

// The open menu draws a header and a rule between sections, so the gap from the last
// collection environment to the first global row, and from the last global to the first
// action, is wider than the one between two rows.
#[gpui_kit::test]
fn the_open_menu_sets_its_sections_apart(cx: &mut TestAppContext) {
    let b = bar(cx);
    open(cx, b.win);
    cx.update_window(b.win, |_, w, cx| {
        w.render_frame(cx);
        let top = |id: &'static str| w.find(id).bounds().top().as_f32();
        let step = top("env-option-dev") - top("env-none");
        let to_globals = top("global-env-none") - top("env-option-prod");
        let to_actions = top("env-manage") - top("global-env-option-h");
        assert!(to_globals > step + 8., "{to_globals} vs {step}");
        assert!(to_actions > step + 8., "{to_actions} vs {step}");
        let menu = w.within("popup-menu");
        for title in [COLLECTION_GROUP, GLOBAL_GROUP, ACTIONS_GROUP] {
            assert!(menu.try_find_by_label(title).is_some(), "{title} header");
        }
    })
    .unwrap();
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

// Each section's manage action opens its editor and keeps both picks.
#[gpui_kit::test]
fn the_manage_actions_open_their_editors_and_keep_the_picks(cx: &mut TestAppContext) {
    let b = bar(cx);
    let root = b.ws.read_with(cx, |w, _| w.current_root().unwrap());
    let win = b.win;
    let pick = |cx: &mut TestAppContext, row: &'static str| {
        open(cx, win);
        cx.update_window(win, |_, w, cx| w.within("popup-menu").click(row, cx))
            .unwrap();
        cx.run_until_parked();
        frame(cx, win);
    };
    pick(cx, "env-option-prod");
    pick(cx, "global-env-option-h");
    pick(cx, "env-manage");
    assert_eq!(
        b.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone())),
        Some(TabKind::Environments)
    );
    pick(cx, "global-env-manage");
    assert!(present(cx, b.win, "global-env-editor"));
    assert_eq!(
        b.ws.read_with(cx, |w, cx| w.selected_environment(&root, cx))
            .as_deref(),
        Some("prod"),
        "managing keeps the collection pick"
    );
    assert_eq!(
        b.ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("h"),
        "managing keeps the global pick"
    );
}

// With globals to pick from and none picked, the global section names the choice rather than
// reading as if none exist, and the button names only the collection's pick.
#[gpui_kit::test]
fn with_no_global_picked_the_menu_says_no_global_environment(cx: &mut TestAppContext) {
    let b = bar(cx);
    assert_eq!(NO_GLOBAL_ENVIRONMENT, "No global environment");
    let picker = b.shell.read_with(cx, |s, _| s.env_picker().clone());
    assert_eq!(
        picker.read_with(cx, |s, cx| s.checked(cx).1),
        NO_GLOBAL_ENVIRONMENT
    );
    assert_eq!(picker.read_with(cx, |s, cx| s.label(cx)), "No environment");
}

// A name cut short in the open menu stays inside its row.
#[gpui_kit::test]
fn a_cut_name_stays_inside_its_row(cx: &mut TestAppContext) {
    let long = "shared-staging-credentials-for-the-payments-team";
    let (path, file) = (format!("{long}.yml"), format!("name: {long}\n"));
    let b = bar_with(cx, &[(&path, &file)]);
    b.ws.update(cx, |w, cx| w.select_picker_item(long, cx));
    frame(cx, b.win);
    open(cx, b.win);
    let option: &'static str = format!("global-env-option-{long}").leak();
    let (name, row) = cx
        .update_window(b.win, |_, w, _| {
            let name = (w.within(option).find("global-env-row-name").bounds())
                .right()
                .as_f32();
            (name, w.find("popup-menu").bounds().right().as_f32())
        })
        .unwrap();
    assert!(
        name <= row,
        "the cut name ends {} past the menu",
        name - row
    );
}
