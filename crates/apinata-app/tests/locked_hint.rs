//! A focused locked input shows a lock hint, stays reachable by Tab, and still copies.

use crate::support;

use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
use apinata_app::ui;
use gpui_kit::Focusable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, WindowOptions, div, open_window,
};
use support::*;

struct Host {
    before: Entity<InputState>,
    locked: Entity<InputState>,
    after: Entity<InputState>,
    lock: bool,
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Input::new(&self.before))
            .child(ui::table_input(&self.locked, self.lock, window, cx))
            .child(Input::new(&self.after))
    }
}

fn host(cx: &mut TestAppContext, theme: &str, lock: bool) -> (AnyWindowHandle, Entity<Host>) {
    support::init(cx);
    use_theme(cx, theme);
    let (win, view) = cx
        .update(|cx| {
            open_window(WindowOptions::default(), cx, |w, cx| {
                let before = cx.new(|cx| InputState::new(w, cx).default_value("before"));
                let locked = cx.new(|cx| InputState::new(w, cx).default_value("locked-value"));
                let after = cx.new(|cx| InputState::new(w, cx).default_value("after"));
                cx.new(|_| Host {
                    before,
                    locked,
                    after,
                    lock,
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (win, view)
}

fn frame(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn focus(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    pick: impl Fn(&Host) -> Entity<InputState>,
    view: &Entity<Host>,
) {
    let state = view.read_with(cx, |h, _| pick(h));
    cx.update_window(win, |_, w, cx| {
        let handle = state.read(cx).focus_handle(cx);
        w.focus(&handle, cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn hint_shown(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<Host>) -> bool {
    let id = view.read_with(cx, |h, _| h.locked.entity_id());
    cx.update_window(win, |_, w, _| w.try_find(("lock-hint", id)).is_some())
        .unwrap()
}

fn has_focus(cx: &mut TestAppContext, win: AnyWindowHandle, state: &Entity<InputState>) -> bool {
    cx.update_window(win, |_, w, cx| {
        state.read(cx).focus_handle(cx).is_focused(w)
    })
    .unwrap()
}

fn hint_follows_focus(cx: &mut TestAppContext, themes: &[&str]) {
    for theme in themes {
        let (win, view) = host(cx, theme, true);
        assert!(!hint_shown(cx, win, &view), "{theme}: hint before focus");
        focus(cx, win, |h| h.locked.clone(), &view);
        assert!(hint_shown(cx, win, &view), "{theme}: no hint while focused");
        focus(cx, win, |h| h.after.clone(), &view);
        assert!(!hint_shown(cx, win, &view), "{theme}: hint after blur");
    }
}

#[gpui_kit::test]
fn a_focused_locked_field_shows_the_lock_hint_in_high_contrast(cx: &mut TestAppContext) {
    hint_follows_focus(cx, &[HC_DARK, HC_LIGHT]);
}

#[gpui_kit::test]
fn a_focused_locked_field_shows_the_lock_hint_in_the_standard_themes(cx: &mut TestAppContext) {
    hint_follows_focus(cx, &[DARK, LIGHT]);
}

#[gpui_kit::test]
fn a_focused_editable_field_shows_no_lock_hint(cx: &mut TestAppContext) {
    for theme in [HC_DARK, DARK] {
        let (win, view) = host(cx, theme, false);
        focus(cx, win, |h| h.locked.clone(), &view);
        assert!(!hint_shown(cx, win, &view), "{theme}");
    }
}

#[gpui_kit::test]
fn typing_into_a_focused_locked_field_changes_nothing(cx: &mut TestAppContext) {
    for theme in [HC_DARK, DARK] {
        let (win, view) = host(cx, theme, true);
        focus(cx, win, |h| h.locked.clone(), &view);
        cx.update_window(win, |_, w, cx| {
            w.input("zz", cx);
            w.press("backspace", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        assert_eq!(
            locked.read_with(cx, |s, _| s.value()),
            "locked-value",
            "{theme}"
        );
    }
}

#[gpui_kit::test]
fn tab_reaches_a_locked_field_and_leaves_it(cx: &mut TestAppContext) {
    for theme in [HC_DARK, DARK] {
        let (win, view) = host(cx, theme, true);
        focus(cx, win, |h| h.before.clone(), &view);
        let (locked, after) = view.read_with(cx, |h, _| (h.locked.clone(), h.after.clone()));
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
        assert!(
            has_focus(cx, win, &locked),
            "{theme}: Tab skips the locked field"
        );
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
        assert!(
            has_focus(cx, win, &after),
            "{theme}: Tab stays on the locked field"
        );
    }
}

#[gpui_kit::test]
fn a_locked_value_still_copies(cx: &mut TestAppContext) {
    for theme in [HC_DARK, DARK] {
        let (win, view) = host(cx, theme, true);
        focus(cx, win, |h| h.locked.clone(), &view);
        cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("old".into())));
        cx.update_window(win, |_, w, cx| {
            w.press("ctrl-a", cx);
            w.press("ctrl-c", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
        assert_eq!(copied.as_deref(), Some("locked-value"), "{theme}");
    }
}

/// The lock icon is drawn at full strength on the fill the locked field really draws
/// ([`ui::field_fill`]), so that pair is what must reach 3:1.
#[gpui_kit::test]
fn the_lock_hint_keeps_a_legible_contrast(cx: &mut TestAppContext) {
    for theme in [HC_DARK, HC_LIGHT, DARK, LIGHT] {
        let (win, view) = host(cx, theme, true);
        focus(cx, win, |h| h.locked.clone(), &view);
        let (icon, fill) = cx.update(|cx| (ui::lock_hint_color(cx), ui::field_fill(true, cx)));
        assert!(
            drawn_fill(cx, win, &view, fill),
            "{theme}: the locked field is not drawn on the locked fill"
        );
        let r = contrast(fill.blend(icon), fill);
        assert!(r >= 3.0, "{theme}: lock icon {r:.2}:1 on the locked fill");
    }
}

/// Whether the locked input paints a solid quad of `fill` within its bounds.
fn drawn_fill(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<Host>,
    fill: gpui_kit::Hsla,
) -> bool {
    let id = view.read_with(cx, |h, _| h.locked.entity_id());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(("input", id)).bounds();
        w.painted_quads().into_iter().any(|q| {
            q.background.as_solid() == Some(fill)
                && (q.bounds.origin.x.0 / scale - f32::from(b.left())).abs() < 2.
                && (q.bounds.origin.y.0 / scale - f32::from(b.top())).abs() < 2.
        })
    })
    .unwrap()
}

/// Whether a bordered quad in the theme's ring colour is painted around the locked input.
fn ring_shown(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<Host>) -> bool {
    use gpui_kit::component::ActiveTheme as _;
    let id = view.read_with(cx, |h, _| h.locked.entity_id());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(("input", id)).bounds();
        let ring = cx.theme().ring;
        w.painted_quads().into_iter().any(|q| {
            q.border_widths.left.0 > 0.
                && q.border_color == ring
                && (q.bounds.origin.x.0 / scale - f32::from(b.left())).abs() < 3.
                && (q.bounds.origin.y.0 / scale - f32::from(b.top())).abs() < 3.
        })
    })
    .unwrap()
}

fn click_locked(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<Host>) {
    let id = view.read_with(cx, |h, _| h.locked.entity_id());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("input", id), cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

const ALL: [&str; 4] = [DARK, LIGHT, HC_DARK, HC_LIGHT];

#[gpui_kit::test]
fn a_click_focuses_a_locked_field_in_every_theme(cx: &mut TestAppContext) {
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        click_locked(cx, win, &view);
        assert!(has_focus(cx, win, &locked), "{theme}: click did not focus");
    }
}

#[gpui_kit::test]
fn a_focused_locked_field_shows_the_focus_ring_in_every_theme(cx: &mut TestAppContext) {
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        // The accent ring is for keyboard focus, so ask for it; Pointer swaps in the border.
        cx.update(|cx| {
            apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx)
        });
        frame(cx, win);
        assert!(!ring_shown(cx, win, &view), "{theme}: ring before focus");
        focus(cx, win, |h| h.locked.clone(), &view);
        assert!(ring_shown(cx, win, &view), "{theme}: no ring while focused");
    }
}

#[gpui_kit::test]
fn ctrl_a_selects_the_whole_locked_value_after_a_click(cx: &mut TestAppContext) {
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        click_locked(cx, win, &view);
        cx.update_window(win, |_, w, cx| w.press("ctrl-a", cx))
            .unwrap();
        cx.run_until_parked();
        let range = locked.read_with(cx, |s, _| s.selected_range());
        assert_eq!(range, 0.."locked-value".len(), "{theme}");
    }
}

#[gpui_kit::test]
fn typing_after_a_click_changes_nothing_in_every_theme(cx: &mut TestAppContext) {
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        click_locked(cx, win, &view);
        cx.update_window(win, |_, w, cx| {
            w.input("zz", cx);
            w.press("backspace", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        assert_eq!(
            locked.read_with(cx, |s, _| s.value()),
            "locked-value",
            "{theme}"
        );
    }
}

/// The text colour is the one thing no painted-quad probe can see, so the contract is pinned
/// on the helper `lock_input` applies: the muted foreground in every theme (full strength in
/// the high-contrast pair, dimmed by the kit's own alpha elsewhere), untouched when editable.
#[gpui_kit::test]
fn locked_text_is_muted_in_every_theme(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    for theme in ALL {
        let _ = host(cx, theme, true);
        let (locked, editable, dim) = cx.update(|cx| {
            (
                ui::locked_text_color(true, cx),
                ui::locked_text_color(false, cx),
                cx.theme().muted_foreground,
            )
        });
        assert_eq!(locked, Some(dim), "{theme}: locked text");
        assert_eq!(editable, None, "{theme}: editable text stays foreground");
    }
}

/// `lock_input` must actually hand the dim colour to the `Input`, not only compute it: probe
/// the colour the returned input carries.
#[gpui_kit::test]
fn lock_input_gives_the_input_the_dim_text_colour(cx: &mut TestAppContext) {
    use gpui_kit::Styled as _;
    use gpui_kit::component::ActiveTheme as _;
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        let carried = |cx: &mut TestAppContext, lock: bool| {
            cx.update_window(win, |_, w, cx| {
                let mut input = ui::lock_input(Input::new(&locked), &locked, lock, w, cx);
                input.style().text.color
            })
            .unwrap()
        };
        let dim = cx.update(|cx| cx.theme().muted_foreground);
        assert_eq!(carried(cx, true), Some(dim), "{theme}: locked");
        assert_eq!(carried(cx, false), None, "{theme}: editable");
    }
}

/// A selection exists only while the input is focused, so a focused locked input draws the
/// normal foreground and never muted text on the selection; it dims again once unfocused.
#[gpui_kit::test]
fn a_focused_locked_input_draws_foreground_text_and_an_unfocused_one_muted(
    cx: &mut TestAppContext,
) {
    use gpui_kit::Styled as _;
    use gpui_kit::component::ActiveTheme as _;
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        let carried = |cx: &mut TestAppContext| {
            cx.update_window(win, |_, w, cx| {
                let mut input = ui::lock_input(Input::new(&locked), &locked, true, w, cx);
                input.style().text.color
            })
            .unwrap()
        };
        let (dim, fg) = cx.update(|cx| (cx.theme().muted_foreground, cx.theme().foreground));
        assert_eq!(carried(cx), Some(dim), "{theme}: unfocused");
        focus(cx, win, |h| h.locked.clone(), &view);
        assert_eq!(carried(cx), Some(fg), "{theme}: focused");
        focus(cx, win, |h| h.after.clone(), &view);
        assert_eq!(carried(cx), Some(dim), "{theme}: blurred again");
    }
}

/// A locked input and a locked textarea keep the kit's own edge in every theme (never a
/// dashed one); the cue is their muted text.
#[gpui_kit::test]
fn locked_fields_keep_the_kit_edge_in_every_theme(cx: &mut TestAppContext) {
    use gpui_kit::Styled as _;
    use gpui_kit::component::input::{Textarea, TextareaState};
    for theme in ALL {
        let (win, view) = host(cx, theme, true);
        let locked = view.read_with(cx, |h, _| h.locked.clone());
        let area = cx
            .update_window(win, |_, w, cx| cx.new(|cx| TextareaState::new(w, cx)))
            .unwrap();
        let styles = cx
            .update_window(win, |_, w, cx| {
                let one = |lock: bool| {
                    let mut i = ui::lock_input(Input::new(&locked), &locked, lock, w, cx);
                    let mut r = ui::readonly_input(Input::new(&locked), &locked, lock, w, cx);
                    let mut t = ui::lock_textarea(Textarea::new(&area), &area, lock, w, cx);
                    (
                        i.style().border_style,
                        r.style().border_style,
                        t.style().border_style,
                    )
                };
                (one(true), one(false))
            })
            .unwrap();
        assert_eq!(styles.0, (None, None, None), "{theme}: locked");
        assert_eq!(styles.1, (None, None, None), "{theme}: editable");
    }
}

/// A window holding only a textarea, locked through `lock_textarea`.
struct AreaHost {
    area: Entity<gpui_kit::component::input::TextareaState>,
}

impl Render for AreaHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::input::Textarea;
        div().size_full().child(ui::lock_textarea(
            Textarea::new(&self.area),
            &self.area,
            true,
            window,
            cx,
        ))
    }
}

/// In every theme a locked textarea draws muted text while unfocused and the full foreground
/// while focused, like a locked input.
#[gpui_kit::test]
fn a_locked_textarea_draws_muted_text_until_focused_in_every_theme(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::component::input::{Textarea, TextareaState};
    support::init(cx);
    for theme in ALL {
        use_theme(cx, theme);
        let (win, view) = cx
            .update(|cx| {
                open_window(WindowOptions::default(), cx, |w, cx| {
                    let area = cx.new(|cx| TextareaState::new(w, cx));
                    cx.new(|_| AreaHost { area })
                })
            })
            .unwrap();
        cx.run_until_parked();
        frame(cx, win);
        let area = view.read_with(cx, |h, _| h.area.clone());
        let carried = |cx: &mut TestAppContext, lock: bool| {
            cx.update_window(win, |_, w, cx| {
                let mut t = ui::lock_textarea(Textarea::new(&area), &area, lock, w, cx);
                t.style().text.color
            })
            .unwrap()
        };
        let (dim, fg) = cx.update(|cx| (cx.theme().muted_foreground, cx.theme().foreground));
        assert_eq!(carried(cx, false), None, "{theme}: editable");
        assert_eq!(carried(cx, true), Some(dim), "{theme}: unfocused");
        cx.update_window(win, |_, w, cx| {
            let handle = area.read(cx).focus_handle(cx);
            w.focus(&handle, cx);
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, win);
        let focused = cx
            .update_window(win, |_, w, cx| area.read(cx).focus_handle(cx).is_focused(w))
            .unwrap();
        assert!(focused, "{theme}: took focus");
        assert_eq!(carried(cx, true), Some(fg), "{theme}: focused");
    }
}
