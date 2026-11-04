//! A focused text input shows its focus ring at 3:1 against its fill and the page, by pointer
//! as well as by key, in all four themes. Buttons keep their pointer-quiet rings.

use crate::support;

use apinata_app::focus_mode::{Modality, set};
use apinata_app::theme;
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Theme};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Hsla, TestAppContext};
use support::*;

const THEMES: [&str; 4] = [theme::DARK, theme::LIGHT, theme::HC_DARK, theme::HC_LIGHT];

/// The accent ring token of the active mode, read from the theme file rather than the live
/// `ring`, which a pointer press has swapped for the input border.
fn ring_token(t: &Theme) -> Hsla {
    let config = if t.mode.is_dark() {
        &t.dark_theme
    } else {
        &t.light_theme
    };
    Hsla::parse_hex(config.colors.ring.as_deref().unwrap()).unwrap()
}

#[gpui_kit::test]
fn the_ring_token_is_three_to_one_on_the_field_and_the_page(cx: &mut TestAppContext) {
    for name in THEMES {
        use_theme(cx, name);
        let (ring, fill, page) = cx.update(|cx| {
            let t = cx.theme();
            (ring_token(t), t.secondary, t.background)
        });
        assert!(contrast(ring, fill) >= 3.0, "{name}: ring on field fill");
        assert!(contrast(ring, page) >= 3.0, "{name}: ring on page");
    }
}

/// The border colours of quads that outline the `url` input right now.
fn edges(cx: &mut TestAppContext, s: &Started) -> Vec<Hsla> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find("url").bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                q.border_widths.left.0 > 0.
                    && (x - l).abs() < 6. * scale
                    && (y - t).abs() < 6. * scale
                    && (x + q.bounds.size.width.0 - r).abs() < 6. * scale
                    && (y + q.bounds.size.height.0 - bt).abs() < 6. * scale
            })
            .map(|q| q.border_color)
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_focused_input_draws_the_ring_by_pointer_and_by_key(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    for name in THEMES {
        use_theme(cx, name);
        for modality in [Modality::Pointer, Modality::Keyboard] {
            cx.update(|cx| set(modality, cx));
            cx.update_window(s.win, |_, w, cx| w.click("url", cx))
                .unwrap();
            // The click is a press, so it sets Pointer; restore the case under test.
            cx.update(|cx| set(modality, cx));
            frame(cx, s.win);
            let (ring, fill, page) = cx.update(|cx| {
                let t = cx.theme();
                (ring_token(t), t.secondary, t.background)
            });
            let seen = edges(cx, &s);
            assert!(
                seen.contains(&ring),
                "{name} {modality:?}: no ring-coloured edge in {seen:?}"
            );
            assert!(contrast(ring, fill) >= 3.0 && contrast(ring, page) >= 3.0);
        }
    }
}

/// Border colours of quads whose outline sits within 6px of element `id`'s edge.
fn edges_of(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: &'static str,
) -> Vec<Hsla> {
    edges_within(cx, win, id, 6.)
}

/// `edges_of` with the reach `tol` in px: 6 also catches a frame around the input.
fn edges_within(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: &'static str,
    tol: f32,
) -> Vec<Hsla> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                q.border_widths.left.0 > 0.
                    && (x - l).abs() < tol * scale
                    && (y - t).abs() < tol * scale
                    && (x + q.bounds.size.width.0 - r).abs() < tol * scale
                    && (y + q.bounds.size.height.0 - bt).abs() < tol * scale
            })
            .map(|q| q.border_color)
            .collect()
    })
    .unwrap()
}

// The chip input draws no frame of its own (its wrapper does), so a click adds no inner ring.
#[gpui_kit::test]
fn a_frameless_input_gets_no_inner_ring(cx: &mut TestAppContext) {
    use apinata_app::settings_section::SettingsSection;
    use gpui_kit::component::ActiveTheme as _;
    let (win, _host) = host_section(cx, SettingsSection::new);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("tag-text", cx))
        .unwrap();
    cx.update(|cx| set(Modality::Pointer, cx));
    frame(cx, win);
    let ring = cx.update(|cx| ring_token(cx.theme()));
    // Tight reach: the wrapper frame, which does ring, sits 5px outside the text input.
    let seen = edges_within(cx, win, "tag-text", 2.);
    assert!(!seen.contains(&ring), "an inner ring on the chip input");
}

struct Probe {
    state: gpui_kit::Entity<gpui_kit::component::input::InputState>,
    disabled: bool,
}

impl gpui_kit::Render for Probe {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::component::input::Input;
        use gpui_kit::{ParentElement as _, Styled as _};
        gpui_kit::div()
            .size_full()
            .p_4()
            .child(apinata_app::ui::padded_input(
                Input::new(&self.state).id("probe").disabled(self.disabled),
                &self.state,
            ))
    }
}

// A focused input shows the ring unless the kit would show none: a disabled one does not.
#[gpui_kit::test]
fn a_disabled_input_gets_no_ring_and_an_enabled_one_does(cx: &mut TestAppContext) {
    use gpui_kit::Focusable as _;
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::component::input::InputState;
    for disabled in [false, true] {
        support::init(cx);
        let (win, _view) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                    let state = cx.new(|cx| InputState::new(w, cx));
                    cx.new(|_| Probe { state, disabled })
                })
            })
            .unwrap();
        frame(cx, win);
        let state = _view.read_with(cx, |p, _| p.state.clone());
        cx.update_window(win, |_, w, cx| {
            let handle = state.read(cx).focus_handle(cx);
            w.focus(&handle, cx);
        })
        .unwrap();
        cx.update(|cx| set(Modality::Pointer, cx));
        frame(cx, win);
        let ring = cx.update(|cx| ring_token(cx.theme()));
        let drawn = edges_of(cx, win, "probe").contains(&ring);
        assert_eq!(drawn, !disabled, "disabled = {disabled}");
    }
}

/// (colour, widest border) of quads outlining element `id`, within 6px of its edge.
fn ring_quads(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> Vec<(Hsla, f32)> {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                q.border_widths.left.0 > 0.
                    && (x - l).abs() < 6. * scale
                    && (y - t).abs() < 6. * scale
                    && (x + q.bounds.size.width.0 - r).abs() < 6. * scale
                    && (y + q.bounds.size.height.0 - bt).abs() < 6. * scale
            })
            .map(|q| (q.border_color, q.border_widths.left.0 / scale))
            .collect()
    })
    .unwrap()
}

fn assert_two_px_accent(seen: &[(Hsla, f32)], ring: Hsla, what: &str) {
    assert!(
        seen.iter().any(|(c, w)| *c == ring && *w >= 2.0),
        "{what}: no 2px accent ring in {seen:?}"
    );
}

#[gpui_kit::test]
fn a_clicked_kv_cell_shows_the_two_pixel_accent_ring(cx: &mut TestAppContext) {
    use apinata_app::kv_table::{KvRow, KvTable};
    use gpui_kit::{ParentElement as _, Styled as _};
    struct H(gpui_kit::Entity<KvTable>);
    impl gpui_kit::Render for H {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div().size_full().child(self.0.clone())
        }
    }
    support::init(cx);
    let (win, _h) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                let t = cx.new(|cx| KvTable::new(w, cx));
                let row = KvRow {
                    name: "a".into(),
                    value: "1".into(),
                    enabled: true,
                    ..KvRow::default()
                };
                t.update(cx, |t, cx| t.load(vec![row], w, cx));
                cx.new(|_| H(t))
            })
        })
        .unwrap();
    frame(cx, win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(win, |_, w, cx| w.click(("kv-value", 0usize), cx))
            .unwrap();
        cx.update(|cx| set(Modality::Pointer, cx));
        frame(cx, win);
        let ring = cx.update(|cx| ring_token(cx.theme()));
        let seen = ring_quads(cx, win, ("kv-value-edge", 0usize));
        assert_two_px_accent(&seen, ring, name);
    }
}

#[gpui_kit::test]
fn a_clicked_env_cell_shows_the_two_pixel_accent_ring(cx: &mut TestAppContext) {
    use apinata_app::env_table::{EnvRow, EnvVarTable};
    use gpui_kit::{ParentElement as _, Styled as _};
    struct H(gpui_kit::Entity<EnvVarTable>);
    impl gpui_kit::Render for H {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div().size_full().child(self.0.clone())
        }
    }
    support::init(cx);
    let (win, _h) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                let t = cx.new(|cx| EnvVarTable::new(w, cx));
                let row = EnvRow {
                    name: "a".into(),
                    value: "1".into(),
                    enabled: true,
                    secret: false,
                    source: Some(0),
                    unsaved: false,
                };
                t.update(cx, |t, cx| t.load(vec![row], w, cx));
                cx.new(|_| H(t))
            })
        })
        .unwrap();
    frame(cx, win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
            .unwrap();
        cx.update(|cx| set(Modality::Pointer, cx));
        frame(cx, win);
        let ring = cx.update(|cx| ring_token(cx.theme()));
        let seen = ring_quads(cx, win, ("env-name-edge", 0usize));
        assert_two_px_accent(&seen, ring, name);
    }
}

#[gpui_kit::test]
fn the_tag_input_frame_shows_the_accent_ring_while_focus_is_within(cx: &mut TestAppContext) {
    use apinata_app::settings_section::SettingsSection;
    let (win, _host) = host_section(cx, SettingsSection::new);
    frame(cx, win);
    let ring = cx.update(|cx| ring_token(cx.theme()));
    assert!(
        !ring_quads(cx, win, "tag-input")
            .iter()
            .any(|(c, _)| *c == ring),
        "ringed before focus"
    );
    cx.update_window(win, |_, w, cx| w.click("tag-text", cx))
        .unwrap();
    cx.update(|cx| set(Modality::Pointer, cx));
    frame(cx, win);
    let seen = ring_quads(cx, win, "tag-input");
    assert_two_px_accent(&seen, ring, "tag frame");
    let b = cx
        .update_window(win, |_, w, _| w.find("tag-input").bounds())
        .unwrap();
    frame(cx, win);
    let after = cx
        .update_window(win, |_, w, _| w.find("tag-input").bounds())
        .unwrap();
    assert_eq!(b, after, "the frame moved");
}

/// Hosts one kv row and one env row, returning their windows.
fn resting_hosts(
    cx: &mut TestAppContext,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::AnyWindowHandle) {
    use apinata_app::env_table::{EnvRow, EnvVarTable};
    use apinata_app::kv_table::{KvRow, KvTable};
    use gpui_kit::{ParentElement as _, Styled as _};
    struct H<T: 'static>(gpui_kit::Entity<T>);
    impl<T: 'static + gpui_kit::Render> gpui_kit::Render for H<T> {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div().size_full().child(self.0.clone())
        }
    }
    support::init(cx);
    let (kv, _h) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                let t = cx.new(|cx| KvTable::new(w, cx));
                let row = KvRow {
                    name: "a".into(),
                    value: "1".into(),
                    enabled: true,
                    ..KvRow::default()
                };
                t.update(cx, |t, cx| t.load(vec![row], w, cx));
                cx.new(|_| H(t))
            })
        })
        .unwrap();
    let (env, _h2) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                let t = cx.new(|cx| EnvVarTable::new(w, cx));
                let row = EnvRow {
                    name: "a".into(),
                    value: "1".into(),
                    enabled: true,
                    secret: false,
                    source: Some(0),
                    unsaved: false,
                };
                t.update(cx, |t, cx| t.load(vec![row], w, cx));
                cx.new(|_| H(t))
            })
        })
        .unwrap();
    frame(cx, kv);
    frame(cx, env);
    (kv, env)
}

#[gpui_kit::test]
fn the_input_border_is_three_to_one_on_the_page_and_differs_from_the_ring(cx: &mut TestAppContext) {
    for name in THEMES {
        use_theme(cx, name);
        let (input, ring, page) = cx.update(|cx| {
            let t = cx.theme();
            (t.input, ring_token(t), t.background)
        });
        assert!(contrast(input, page) >= 3.0, "{name}: input on page");
        assert_ne!(input, ring, "{name}: resting edge equals the ring");
    }
}

#[gpui_kit::test]
fn a_resting_kv_or_env_cell_has_an_edge_in_light_themes_only(cx: &mut TestAppContext) {
    let (kv, env) = resting_hosts(cx);
    let cells: [(_, (&'static str, usize)); 2] =
        [(kv, ("kv-value-edge", 0)), (env, ("env-name-edge", 0))];
    for name in THEMES {
        use_theme(cx, name);
        let light = cx.update(|cx| !cx.theme().mode.is_dark());
        let input = cx.update(|cx| cx.theme().input);
        for (win, id) in cells {
            frame(cx, win);
            let shown = cx
                .update_window(win, |_, w, _| w.find(id).visible())
                .unwrap();
            assert_eq!(shown, light, "{name} {id:?}: resting edge visible");
            if light {
                let seen = ring_quads(cx, win, id);
                assert!(
                    seen.iter().any(|(c, _)| *c == input),
                    "{name} {id:?}: no input-coloured edge in {seen:?}"
                );
            }
        }
    }
}

/// The real Changes dialog over a git collection with one change, ready to commit.
fn changes_dialog(
    cx: &mut TestAppContext,
) -> (crate::git_support::Sandbox, tempfile::TempDir, Started) {
    let sb = crate::git_support::Sandbox::new();
    let col = crate::h::changed(&sb);
    let s = crate::h::git_started(cx, &sb, &col);
    crate::h::open_on(cx, &s, col.path());
    (sb, col, s)
}

/// Whether a quad outlining the commit message carries the accent ring token.
fn message_ringed(cx: &mut TestAppContext, s: &Started) -> bool {
    let ring = cx.update(|cx| ring_token(cx.theme()));
    ring_quads(cx, s.win, "changes-message")
        .iter()
        .any(|(c, _)| *c == ring)
}

/// Asserts the commit message ring: 2px accent by pointer, the kit's own accent ring by key.
fn assert_message_ring(cx: &mut TestAppContext, s: &Started, modality: Modality, what: &str) {
    let ring = cx.update(|cx| ring_token(cx.theme()));
    let seen = ring_quads(cx, s.win, "changes-message");
    match modality {
        Modality::Pointer => assert_two_px_accent(&seen, ring, what),
        Modality::Keyboard => assert!(seen.iter().any(|(c, _)| *c == ring), "{what}: {seen:?}"),
    }
}

/// Clicks the commit message, then restores `modality`, since the click sets Pointer.
fn focus_message(cx: &mut TestAppContext, s: &Started, modality: Modality) {
    crate::h::click(cx, s, "changes-message");
    cx.update(|cx| set(modality, cx));
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_focused_commit_message_draws_the_ring_by_pointer_and_by_key(cx: &mut TestAppContext) {
    let (_sb, _col, s) = changes_dialog(cx);
    for name in THEMES {
        use_theme(cx, name);
        for modality in [Modality::Pointer, Modality::Keyboard] {
            focus_message(cx, &s, modality);
            assert_message_ring(cx, &s, modality, &format!("{name} {modality:?}: no ring"));
        }
    }
}

#[gpui_kit::test]
fn an_unfocused_commit_message_draws_no_ring(cx: &mut TestAppContext) {
    let (_sb, _col, s) = changes_dialog(cx);
    for name in THEMES {
        use_theme(cx, name);
        for modality in [Modality::Pointer, Modality::Keyboard] {
            focus_message(cx, &s, modality);
            // A blur inside the open dialog comes back into it, so Tab moves focus off.
            cx.update_window(s.win, |_, w, cx| w.focus_next(cx))
                .unwrap();
            frame(cx, s.win);
            assert!(!message_ringed(cx, &s), "{name} {modality:?}: ringed");
        }
    }
}

// A busy dialog locks the message read-only in every theme, so it keeps focus and the ring.
#[gpui_kit::test]
fn a_locked_commit_message_keeps_the_ring_in_every_theme(cx: &mut TestAppContext) {
    for name in THEMES {
        for modality in [Modality::Pointer, Modality::Keyboard] {
            let (_sb, _col, s) = changes_dialog(cx);
            use_theme(cx, name);
            focus_message(cx, &s, modality);
            crate::h::view(cx).update(cx, |v, cx| {
                v.toggle(0, cx);
                assert!(v.busy());
            });
            // `ring_quads` draws a bare frame, so the work cannot finish before it is read.
            assert_message_ring(cx, &s, modality, &format!("{name} {modality:?}: busy"));
            crate::h::idle(cx, &s);
        }
    }
}
