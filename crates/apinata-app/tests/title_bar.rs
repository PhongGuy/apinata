//! The kit title bar hosts the window's controls in exactly one `title-bar` scope, and the
//! window options suit a client-drawn title bar.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::app::window_options;
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::style::{ENV_CONTROL_MIN, ENV_CONTROL_WIDTH, SPACE_1, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Focusable as _, TestAppContext, WindowOptions, px, size,
};
use support::*;

fn request_close(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.update(cx, |v, cx| v.request_window_close(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

fn shell_with(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
) -> (tempfile::TempDir, AnyWindowHandle) {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (state, win)
}

#[test]
fn window_options_suit_a_client_drawn_title_bar() {
    let opts = window_options(&Session::default(), &[]);
    assert!(opts.app_owns_titlebar_drag);
    let bar = opts.titlebar.expect("titlebar options");
    assert_eq!(bar.title, Some("Apinata".into()));
    assert!(bar.appears_transparent);
    assert!(bar.traffic_light_position.is_some());
}

#[gpui_kit::test]
fn one_title_bar_scope_holds_the_controls(cx: &mut TestAppContext) {
    let col = collection();
    let (_state, win) = shell_with(cx, &col);
    cx.update_window(win, |_, w, _| {
        // `within` panics unless exactly one `title-bar` scope exists.
        let bar = w.within("title-bar");
        for id in [
            "collection-switcher",
            "env-control",
            "env-select",
            "environments",
            "app-menu",
        ] {
            assert!(bar.try_find(id).is_some(), "{id} is in the title bar");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_title_bar_close_prompts_for_unsaved_work(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
    request_close(cx, &s);
    assert!(confirm_open(cx), "the save prompt opened");
    assert_eq!(windows(cx), 1, "the window stays open");
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert_eq!(windows(cx), 1);
}

#[gpui_kit::test]
fn the_title_bar_close_closes_a_clean_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    request_close(cx, &s);
    assert!(!confirm_open(cx));
    assert_eq!(windows(cx), 0);
}

fn title_bar_lacks(cx: &mut TestAppContext, id: &'static str) {
    let col = collection();
    let (_state, win) = shell_with(cx, &col);
    cx.update_window(win, |_, w, _| {
        assert!(
            w.within("title-bar").try_find(id).is_none(),
            "{id} left the title bar"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_collection_left_the_title_bar(cx: &mut TestAppContext) {
    title_bar_lacks(cx, "open-collection");
}

#[gpui_kit::test]
fn import_collection_left_the_title_bar(cx: &mut TestAppContext) {
    title_bar_lacks(cx, "import-collection");
}

#[gpui_kit::test]
fn new_collection_left_the_title_bar(cx: &mut TestAppContext) {
    title_bar_lacks(cx, "new-collection");
}

#[gpui_kit::test]
fn import_environment_left_the_title_bar(cx: &mut TestAppContext) {
    title_bar_lacks(cx, "import-environment");
}

#[gpui_kit::test]
fn cookie_jar_left_the_title_bar(cx: &mut TestAppContext) {
    title_bar_lacks(cx, "cookie-jar");
}

#[gpui_kit::test]
fn the_right_group_is_environment_then_menu(cx: &mut TestAppContext) {
    let col = collection();
    let (_state, win) = shell_with(cx, &col);
    cx.update_window(win, |_, w, _| {
        let bar = w.within("title-bar");
        let env = bar.find("env-control").bounds();
        let globe = bar.find("environments").bounds();
        let menu = bar.find("app-menu").bounds();
        assert!(env.right() <= globe.left() && globe.right() <= menu.left());
    })
    .unwrap();
}

fn title_room(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, zoom: f32) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
    });
    frame(cx, s.win);
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_app_menu_and_its_ring_stay_inside_a_narrow_zoomed_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for (w, h) in [(1000., 700.), (800., 600.)] {
        title_room(cx, &s, w, h, 2.0);
        let (menu, width) = cx
            .update_window(s.win, |_, win, cx| {
                win.render_frame(cx);
                (
                    win.within("title-bar").find("app-menu").bounds(),
                    win.viewport_size().width,
                )
            })
            .unwrap();
        assert!(
            menu.right() <= width - at_zoom(SPACE_1, 2.0),
            "{w}x{h} at 200%: app-menu {menu:?} leaves no ring room in {width:?}"
        );
    }
}

#[gpui_kit::test]
fn the_environment_selects_keep_their_width_at_100_percent(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    title_room(cx, &s, 1280., 800., 1.0);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let bar = w.within("title-bar");
        for id in ["env-control", "global-env-selector"] {
            let b = bar.find(id).bounds();
            assert_eq!(b.size.width, at_zoom(ENV_CONTROL_WIDTH, 1.0), "{id} {b:?}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_focused_environment_select_draws_a_ring(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let env = s.shell.read_with(cx, |sh, _| sh.env_selector().clone());
    cx.update_window(s.win, |_, w, cx| {
        let handle = env.read(cx).select().focus_handle(cx);
        handle.focus(w, cx)
    })
    .unwrap();
    // A programmatic focus move is no key press, so the keyboard ring is set by hand.
    cx.update(|cx| apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx));
    assert!(present(cx, s.win, "env-select-ring"));
    assert!(!present(cx, s.win, "global-env-select-ring"));
    // The global select is the next Tab stop.
    cx.update_window(s.win, |_, w, cx| w.focus_next(cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "env-select-ring"));
    assert!(present(cx, s.win, "global-env-select-ring"));
}

#[gpui_kit::test]
fn icon_only_title_bar_buttons_are_named_like_their_tooltips(cx: &mut TestAppContext) {
    let col = collection();
    let (_state, win) = shell_with(cx, &col);
    cx.update_window(win, |_, w, _| {
        let bar = w.within("title-bar");
        assert_eq!(
            bar.find("environments").label(),
            Some(apinata_app::shell::MANAGE_ENVIRONMENTS_TIP)
        );
        assert_eq!(
            bar.find("app-menu-slot").label(),
            Some(apinata_app::shell::APP_MENU_TIP)
        );
    })
    .unwrap();
    assert_eq!(
        apinata_app::shell::ENV_EDITOR_ICON,
        gpui_kit::assets::IconName::SlidersHorizontal
    );
    assert_eq!(
        apinata_app::shell::MANAGE_ENVIRONMENTS_TIP,
        "Manage environments"
    );
}

#[gpui_kit::test]
fn the_collection_select_has_no_globe_before_it(cx: &mut TestAppContext) {
    let col = collection();
    let (_state, win) = shell_with(cx, &col);
    cx.update_window(win, |_, w, _| {
        let bar = w.within("title-bar");
        let control = bar.find("env-control").bounds();
        let select = bar.find("env-select").bounds();
        assert!(
            (select.left() - control.left()).abs() < px(0.5),
            "{select:?} in {control:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_environment_selects_shrink_but_stay_in_the_bar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for (w, h, zoom) in [(320., 700., 1.0), (800., 600., 2.0)] {
        title_room(cx, &s, w, h, zoom);
        let (switcher, ctl, sel, gctl, gsel, width) = cx
            .update_window(s.win, |_, win, cx| {
                win.render_frame(cx);
                let width = win.viewport_size().width;
                let bar = win.within("title-bar");
                (
                    bar.find("collection-switcher").bounds(),
                    bar.find("env-control").bounds(),
                    bar.find("env-select").bounds(),
                    bar.find("global-env-selector").bounds(),
                    bar.find("global-env-select").bounds(),
                    width,
                )
            })
            .unwrap();
        let at = format!("{w}x{h} at {zoom}");
        assert!(
            switcher.right() <= ctl.left() + px(0.5),
            "{at}: {switcher:?} under {ctl:?}"
        );
        assert!(
            ctl.right() <= gctl.left() + px(0.5),
            "{at}: {ctl:?} {gctl:?}"
        );
        assert!(gctl.right() <= width, "{at}: {gctl:?} past {width:?}");
        for (c, inner) in [(ctl, sel), (gctl, gsel)] {
            assert!(
                c.size.width >= at_zoom(ENV_CONTROL_MIN, zoom) - px(0.5),
                "{at}: {c:?}"
            );
            assert!(
                inner.left() >= c.left() - px(0.5) && inner.right() <= c.right() + px(0.5),
                "{at}: {inner:?} leaves {c:?}"
            );
        }
    }
}

// Every icon button's tooltip shows its shortcut, as History's does, even while focus sits
// outside the Shell; with no focus at all the lookup has no path to search.
#[gpui_kit::test]
fn icon_button_tooltips_find_their_shortcut_without_focus(cx: &mut TestAppContext) {
    use apinata_app::keymap::{OpenEnvironments, ToggleConsole, ToggleOrientation, ToggleSidebar};
    use apinata_app::ui::ICON_TIP_CONTEXT;
    use gpui_kit::component::kbd::Kbd;
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.blur(cx);
        w.render_frame(cx);
        let shown = |a: &dyn gpui_kit::Action| Kbd::binding_for_action(a, ICON_TIP_CONTEXT, w);
        assert!(shown(&OpenEnvironments).is_some(), "Manage environments");
        assert!(shown(&ToggleSidebar).is_some(), "sidebar");
        assert!(shown(&ToggleConsole).is_some(), "console");
        assert!(shown(&ToggleOrientation).is_some(), "orientation");
        let ctx = gpui_kit::KeyContext::parse(ICON_TIP_CONTEXT.unwrap_or_default()).unwrap();
        let env = w
            .highest_precedence_binding_for_action_in_context(&OpenEnvironments, ctx)
            .and_then(|b| b.keystrokes().first().map(|k| k.inner().clone()))
            .expect("bound");
        assert!(
            env.key == "e" && apinata_app::shortcuts::is_secondary(&env.modifiers),
            "{env:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn below_the_select_minimum_the_title_bar_still_fits(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for (w, h, zoom) in [(260., 600., 1.0), (560., 500., 2.0)] {
        title_room(cx, &s, w, h, zoom);
        let (found, width) = cx
            .update_window(s.win, |_, win, cx| {
                win.render_frame(cx);
                let bar = win.within("title-bar");
                let found: Vec<(&str, gpui_kit::Bounds<gpui_kit::Pixels>)> = [
                    "collection-switcher",
                    "env-control",
                    "global-env-selector",
                    "environments",
                    "app-menu",
                ]
                .into_iter()
                .map(|id| (id, bar.find(id).bounds()))
                .collect();
                (found, win.viewport_size().width)
            })
            .unwrap();
        let at = format!("{w}x{h} at {zoom}");
        for pair in found.windows(2) {
            let (a, ab) = &pair[0];
            let (b, bb) = &pair[1];
            assert!(
                ab.right() <= bb.left() + px(0.5),
                "{at}: {a} {ab:?} runs into {b} {bb:?}"
            );
        }
        for (id, b) in &found {
            assert!(
                b.left() >= px(-0.5) && b.right() <= width + px(0.5),
                "{at}: {id} {b:?} outside {width:?}"
            );
        }
    }
}

// `icon_button` itself hands its tooltip the shell context; the lookup test above covers only
// the constant. The status-bar buttons are built with actions on every shell render.
#[gpui_kit::test]
fn icon_button_passes_the_tooltip_context(cx: &mut TestAppContext) {
    use apinata_app::ui::{ICON_TIP_CONTEXT, last_icon_tip_context};
    let col = collection();
    let s = started(cx, &col);
    frame(cx, s.win);
    assert!(ICON_TIP_CONTEXT.is_some(), "the context names a scope");
    assert_eq!(last_icon_tip_context(), Some(ICON_TIP_CONTEXT));
}

// A press that starts on a title-bar control belongs to the control: dragging off it must not
// reach the bar's window move, which the test platform answers with a panic. The controls are
// read from the element tree, so a new one is covered without a list. The empty-spot drag that
// does reach the move is pinned in branch_changes.rs.
#[gpui_kit::test]
fn a_drag_started_on_a_title_bar_control_does_not_panic(cx: &mut TestAppContext) {
    use gpui_kit::{Role, point, px};
    let sb = crate::git_support::Sandbox::new();
    let (_col, s) = crate::bar_rings::one_changed(cx, &sb);
    let win = s.win;
    let controls: Vec<(String, gpui_kit::Point<gpui_kit::Pixels>)> = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let mut found: Vec<_> = gpui_kit::base::test_support::snapshots(w)
                .into_iter()
                .filter(|e| e.visible() && matches!(e.role(), Some(Role::Button | Role::ComboBox)))
                .filter(|e| {
                    e.path()
                        .iter()
                        .any(|p| format!("{p:?}") == "Name(\"title-bar\")")
                })
                .map(|e| {
                    (
                        format!("{:?}", e.path().last().unwrap()),
                        e.bounds().center(),
                    )
                })
                .collect();
            found.sort_by(|a, b| a.0.cmp(&b.0));
            found
        })
        .unwrap();
    // The walk must really reach the bar's controls, branch label included.
    for id in [
        "git-branch",
        "git-changes",
        "environments",
        "app-menu",
        "collection-switcher",
    ] {
        assert!(
            controls.iter().any(|(n, _)| n.contains(id)),
            "{id} missing from {controls:?}"
        );
    }
    for (id, centre) in controls {
        let drag = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cx.update_window(win, |_, w, cx| {
                w.drag(centre, point(centre.x - px(30.), centre.y), cx)
            })
            .unwrap();
        }));
        let message = drag
            .err()
            .and_then(|p| {
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            })
            .unwrap_or_default();
        assert!(message.is_empty(), "{id}: the drag panicked: {message:?}");
    }
}

struct BareBar {
    marked: bool,
}

impl gpui_kit::Render for BareBar {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{
            InteractiveElement as _, IntoElement as _, ParentElement as _,
            StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
        };
        // A bare div with a click handler that never marks the press handled.
        let bare = gpui_kit::div()
            .id("bare")
            .w(px(80.))
            .h(px(20.))
            .on_click(|_, _, _| {});
        let bare = if self.marked {
            apinata_app::ui::no_window_move(bare)
                .test_support()
                .into_any_element()
        } else {
            bare.test_support().into_any_element()
        };
        gpui_kit::component::TitleBar::new().child(bare)
    }
}

fn drag_off_bare(cx: &mut TestAppContext, marked: bool) -> String {
    use gpui_kit::{point, px};
    support::init(cx);
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| BareBar { marked })
            })
        })
        .unwrap();
    frame(cx, win);
    let centre = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("bare").bounds().center()
        })
        .unwrap();
    let drag = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update_window(win, |_, w, cx| {
            w.drag(centre, point(centre.x + px(40.), centre.y), cx)
        })
        .unwrap();
    }));
    drag.err()
        .and_then(|p| {
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        })
        .unwrap_or_default()
}

// Without the marker a bare control carries the window; the marker is what stops it.
#[gpui_kit::test]
fn an_unmarked_bare_control_arms_the_window_move(cx: &mut TestAppContext) {
    assert!(drag_off_bare(cx, false).contains("not implemented"));
}

#[gpui_kit::test]
fn a_marked_bare_control_does_not_arm_the_window_move(cx: &mut TestAppContext) {
    let message = drag_off_bare(cx, true);
    assert!(message.is_empty(), "the drag reached the move: {message:?}");
}
