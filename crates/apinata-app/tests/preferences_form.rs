//! The Preferences dialog's form: visible selection, sections and shared columns.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::{Orientation, Settings, ThemeChoice};
use apinata_app::style::{FORM_INPUT_WIDTH, FORM_STACK_WIDTH, INPUT_HEIGHT, SPACE_3, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, px};
use support::*;

fn settings(cx: &mut TestAppContext) -> Settings {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone())
}

fn set(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn marks(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    group: &'static str,
    n: usize,
) -> Vec<Option<bool>> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0..n)
            .map(|ix| w.within(group).find(ix).checked())
            .collect()
    })
    .unwrap()
}

fn pick(cx: &mut TestAppContext, win: AnyWindowHandle, group: &'static str, ix: usize) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.within(group).click(ix, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_current_choice_is_marked_and_a_click_moves_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| {
        p.theme = ThemeChoice::Light;
        p.response_orientation = Orientation::Horizontal;
    });
    open_prefs(cx, &s);
    assert_eq!(
        marks(cx, s.win, "theme", 3),
        [Some(false), Some(true), Some(false)]
    );
    assert_eq!(
        marks(cx, s.win, "orientation", 2),
        [Some(false), Some(true)]
    );
    pick(cx, s.win, "theme", 0);
    pick(cx, s.win, "orientation", 0);
    let got = settings(cx);
    assert_eq!(
        (got.theme, got.response_orientation),
        (ThemeChoice::System, Orientation::Vertical)
    );
    assert_eq!(
        marks(cx, s.win, "theme", 3),
        [Some(true), Some(false), Some(false)]
    );
    assert_eq!(
        marks(cx, s.win, "orientation", 2),
        [Some(true), Some(false)]
    );
}

#[cfg(target_os = "linux")]
#[gpui_kit::test]
fn the_title_bar_choice_is_marked(cx: &mut TestAppContext) {
    use apinata_app::settings::LinuxTitleBar;
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert_eq!(
        marks(cx, s.win, "title-bar-mode", 2),
        [Some(true), Some(false)]
    );
    pick(cx, s.win, "title-bar-mode", 1);
    assert_eq!(settings(cx).linux_title_bar, LinuxTitleBar::System);
    assert_eq!(
        marks(cx, s.win, "title-bar-mode", 2),
        [Some(false), Some(true)]
    );
}

#[gpui_kit::test]
fn rows_sit_in_their_sections_in_two_shared_columns(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let groups: [(&'static str, &[&'static str]); 3] = [
            (
                "prefs-appearance",
                &[
                    "pref-theme",
                    "pref-orientation",
                    "pref-zoom",
                    "pref-response-font",
                ],
            ),
            (
                "prefs-network",
                &[
                    "pref-verify-tls",
                    "pref-body-cap",
                    "pref-verify-tls-restart",
                ],
            ),
            ("prefs-updates", &["pref-auto-check", "pref-check-now"]),
        ];
        for (section, rows) in groups {
            for row in rows {
                assert!(
                    w.within(section).try_find(*row).is_some(),
                    "{row} in {section}"
                );
            }
        }
        let title_bar = w
            .within("prefs-appearance")
            .try_find("pref-title-bar")
            .is_some();
        assert_eq!(title_bar, cfg!(target_os = "linux"));
        let tops: Vec<_> = ["prefs-appearance", "prefs-network", "prefs-updates"]
            .iter()
            .map(|id| w.find(*id).bounds().top())
            .collect();
        assert!(tops[0] < tops[1] && tops[1] < tops[2], "{tops:?}");
        let rows = [
            "pref-theme",
            "pref-orientation",
            "pref-zoom",
            "pref-response-font",
            "pref-verify-tls",
            "pref-body-cap",
            "pref-auto-check",
            "pref-check-now",
        ];
        let cols: Vec<_> = rows
            .iter()
            .map(|r| {
                let label = w.within(*r).find("form-label").bounds();
                let control = w.within(*r).find("form-control").bounds();
                let overlap = label.top().max(control.top()) < label.bottom().min(control.bottom());
                assert!(overlap, "{r} wrapped onto two lines");
                (label.left(), control.left())
            })
            .collect();
        assert!(cols.windows(2).all(|p| p[0] == p[1]), "{cols:?}");
        assert_eq!(
            w.find("body-cap").bounds().size.width,
            at_zoom(FORM_INPUT_WIDTH, 1.0)
        );
        assert_eq!(
            w.find("zoom").bounds().size.width,
            at_zoom(FORM_INPUT_WIDTH, 1.0)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_title_outranks_the_headings_and_choices_sit_on_a_track(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, _| {
        let title = w.find("prefs-title").bounds();
        let heading = w.find("prefs-appearance-heading").bounds();
        assert!(
            title.size.height > heading.size.height,
            "{title:?} vs {heading:?}"
        );
        assert!(
            heading.top() - title.bottom() >= at_zoom(SPACE_3, 1.0),
            "{title:?} then {heading:?}"
        );
        let track = w.find("theme").bounds();
        let first = w.within("theme").find(0usize).bounds();
        let last = w.within("theme").find(2usize).bounds();
        assert!(first.left() - track.left() >= px(2.) && first.top() - track.top() >= px(2.));
        assert!(track.right() - last.right() >= px(2.) && track.bottom() - last.bottom() >= px(2.));
        // The track hugs its segments: no slack beyond the padding and a border.
        assert!(
            track.right() - last.right() <= px(5.),
            "track {track:?} runs past its last segment {last:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn check_automatically_is_disabled_while_updates_are_unconfigured(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = settings(cx).check_for_updates;
    open_prefs(cx, &s);
    assert!(
        present(cx, s.win, "updates-disabled"),
        "the test build has no update source"
    );
    // The Updates section sits below the fold of the Network rows.
    prefs_click(cx, &s, "check-updates");
    cx.run_until_parked();
    assert_eq!(settings(cx).check_for_updates, before);
}

// A stored "on" must not draw a tick on a build that cannot check; the setting stays as saved.
#[gpui_kit::test]
fn check_automatically_draws_unticked_while_updates_are_unconfigured(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| p.check_for_updates = true);
    open_prefs(cx, &s);
    prefs_reveal(cx, &s, "check-updates");
    let state = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let b = w.find("check-updates");
            (b.checked(), b.disabled())
        })
        .unwrap()
    };
    assert_eq!(state(cx), (Some(false), Some(true)));
    // The disabled box swallows the click, so it neither ticks nor writes.
    prefs_click(cx, &s, "check-updates");
    cx.run_until_parked();
    assert_eq!(state(cx), (Some(false), Some(true)));
    assert!(settings(cx).check_for_updates, "the stored setting is kept");
}

// With the stored value off, an enabled click would write true, so this click is observable.
#[gpui_kit::test]
fn a_click_on_the_unconfigured_check_automatically_box_changes_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| p.check_for_updates = false);
    open_prefs(cx, &s);
    prefs_click(cx, &s, "check-updates");
    cx.run_until_parked();
    let drawn = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("check-updates").checked()
        })
        .unwrap();
    assert_eq!(drawn, Some(false));
    assert!(!settings(cx).check_for_updates, "the disabled click wrote");
}

// The note explaining the disabled checkbox belongs right under it, not under "Manual check".
#[gpui_kit::test]
fn the_unconfigured_updates_note_sits_directly_under_its_checkbox(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    prefs_reveal(cx, &s, "pref-check-now");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let auto = w.find("pref-auto-check").bounds();
        let note = w.find("updates-disabled").bounds();
        let manual = w.find("pref-check-now").bounds();
        assert!(
            note.top() >= auto.bottom() && note.bottom() <= manual.top(),
            "note {note:?} must lie between {auto:?} and {manual:?}"
        );
    })
    .unwrap();
}

// Clicks Preferences input `id`, replaces its text and leaves with `then` (a key, or none).
fn type_then(cx: &mut TestAppContext, s: &Started, id: &str, text: &str, then: &str) {
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    prefs_click(cx, s, id);
    cx.update_window(s.win, |_, w, cx| {
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input(text, cx);
        if !then.is_empty() {
            w.press(then, cx);
        }
    })
    .unwrap();
    // A focus change reaches blur listeners when the next frame is drawn.
    frame(cx, s.win);
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    use gpui_kit::component::WindowExt as _;
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn body_cap_saves_on_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    type_then(cx, &s, "body-cap", "5", "tab");
    assert_eq!(settings(cx).max_body_bytes, Some(5 * 1024 * 1024));
}

#[gpui_kit::test]
fn an_invalid_port_shows_an_inline_error_and_is_not_saved(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| {
        p.proxy.mode = apinata_app::settings::ProxyMode::Manual;
        p.proxy.port = "3128".into();
    });
    let view = support::open_prefs(cx, &s);
    type_then(cx, &s, "global-proxy-port", "abc", "tab");
    frame(cx, s.win);
    assert!(present(cx, s.win, "global-proxy-port-error"));
    assert_eq!(settings(cx).proxy.port, "3128");
    assert_eq!(
        view.read_with(cx, |v, cx| v.input_text("global-proxy-port", cx)),
        Some("abc".into())
    );
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(settings(cx).proxy.port, "3128");
    assert_eq!(toasts(cx, &s), before + 1);
    let unsaved = view.read_with(cx, |v, _| v.unsaved());
    assert_eq!(unsaved.len(), 1, "{unsaved:?}");
    assert!(unsaved[0].starts_with("Not saved: Port: "), "{unsaved:?}");
    // Escape reaches the same close hook as the close button.
    let view = support::open_prefs(cx, &s);
    type_then(cx, &s, "global-proxy-port", "abc", "escape");
    assert_eq!(toasts(cx, &s), before + 2);
    assert_eq!(view.read_with(cx, |v, _| v.unsaved().len()), 1);
    assert_eq!(settings(cx).proxy.port, "3128");
}

fn close_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn unedited_fields_write_nothing_on_blur_or_close(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| {
        p.max_body_bytes = Some(1_000_000);
        p.zoom = 1.234;
        p.response_font_offset = 0.3;
    });
    open_prefs(cx, &s);
    for id in ["zoom", "response-font-size", "body-cap"] {
        prefs_click(cx, &s, id);
        cx.update_window(s.win, |_, w, cx| w.press("tab", cx))
            .unwrap();
        frame(cx, s.win);
    }
    close_prefs(cx, &s);
    let got = settings(cx);
    assert_eq!(
        (got.max_body_bytes, got.zoom, got.response_font_offset),
        (Some(1_000_000), 1.234, 0.3)
    );
}

#[gpui_kit::test]
fn a_setting_changed_elsewhere_survives_the_close(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| {
        p.proxy.mode = apinata_app::settings::ProxyMode::Manual
    });
    open_prefs(cx, &s);
    set(cx, |p| {
        p.proxy.host = "elsewhere.local".into();
        p.history_limit = 7;
    });
    close_prefs(cx, &s);
    let got = settings(cx);
    assert_eq!(
        (got.proxy.host.as_str(), got.history_limit),
        ("elsewhere.local", 7)
    );
}

#[gpui_kit::test]
fn a_field_error_wraps_inside_the_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| {
        p.proxy.mode = apinata_app::settings::ProxyMode::Manual
    });
    open_prefs(cx, &s);
    type_then(cx, &s, "global-proxy-port", "abc", "tab");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let dialog = w.find("preferences").bounds();
        let error = w.find("global-proxy-port-error").bounds();
        assert!(
            error.right() <= dialog.right() && error.left() >= dialog.left(),
            "{error:?} outside {dialog:?}"
        );
    })
    .unwrap();
}

// Wheels the body far past its end, so the last row sits at the bottom of the scroll range.
fn scroll_prefs_to_end(cx: &mut TestAppContext, s: &Started) {
    use gpui_kit::{ScrollDelta, point};
    for _ in 0..40 {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.scroll(
                "preferences",
                ScrollDelta::Pixels(point(px(0.), px(-400.))),
                cx,
            )
        })
        .unwrap();
    }
    frame(cx, s.win);
}

// At a narrow window the long notes must wrap onto several lines (taller than one line) and
// still lie wholly inside the rows' viewport once revealed.
#[gpui_kit::test]
fn long_notes_wrap_and_scroll_wholly_into_view(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(520.), px(700.)));
    // The proxy help shows only in System mode.
    set(cx, |p| {
        p.proxy.mode = apinata_app::settings::ProxyMode::System
    });
    let mut one_line = None;
    for id in [
        "updates-disabled",
        "prefs-proxy-note",
        "prefs-keyboard-note",
    ] {
        open_prefs(cx, &s);
        if id == "prefs-keyboard-note" {
            scroll_prefs_to_end(cx, &s);
        } else {
            prefs_reveal(cx, &s, id);
        }
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let rows = w.find("preferences-rows").bounds();
            let note = w.find(id).bounds();
            assert!(
                note.top() >= rows.top()
                    && note.bottom() <= rows.bottom()
                    && note.right() <= rows.right()
                    && rows.right() <= px(520.),
                "{id}: note {note:?} rows {rows:?}"
            );
            match one_line {
                None => one_line = Some(note.size.height),
                Some(line) => assert!(
                    note.size.height > line * 1.5,
                    "{id} does not wrap: {note:?} against one line {line:?}"
                ),
            }
        })
        .unwrap();
        close_prefs(cx, &s);
    }
}

// Every Preferences form row; rows the current settings hide are skipped.
const FORM_ROWS: [&str; 28] = [
    "pref-theme",
    "pref-orientation",
    "pref-title-bar",
    "pref-zoom",
    "pref-response-font",
    "pref-verify-tls",
    "pref-body-cap",
    "pref-http-version",
    "pref-proxy-mode",
    "pref-proxy-protocol",
    "pref-proxy-host",
    "pref-proxy-port",
    "pref-proxy-username",
    "pref-proxy-password",
    "pref-proxy-bypass",
    "pref-proxy-pac",
    "pref-ca-file",
    "pref-keep-default-cas",
    "pref-store-cookies",
    "pref-send-cookies",
    "pref-run-scripts",
    "pref-script-timeout",
    "pref-history-limit",
    "pref-remember-responses",
    "pref-clear-responses",
    "pref-autosave",
    "pref-auto-check",
    "pref-check-now",
];

// At the smallest window the app keeps, every Preferences row puts its label above its
// control, inside the dialog.
#[gpui_kit::test]
fn every_preferences_row_stacks_inside_the_dialog_at_the_minimum_window(cx: &mut TestAppContext) {
    use apinata_app::session::{MIN_HEIGHT, MIN_WIDTH};
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| {
        p.proxy.mode = apinata_app::settings::ProxyMode::Manual
    });
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(MIN_WIDTH), px(MIN_HEIGHT.max(700.))));
    open_prefs(cx, &s);
    frame(cx, s.win);
    let checked = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let dialog = w.find("preferences").bounds();
            let rows = w.find("preferences-rows").bounds();
            // The kit shrinks a dialog to the window, so the rows column is under the threshold.
            assert!(
                rows.size.width < at_zoom(FORM_STACK_WIDTH, 1.0),
                "rows column {rows:?} is not narrow"
            );
            let mut checked = 0;
            for row in FORM_ROWS {
                if w.try_find(row).is_none() {
                    continue;
                }
                let label = w.within(row).find("form-label").bounds();
                let control = w.within(row).find("form-control").bounds();
                assert!(
                    label.bottom() <= control.top() + px(0.5),
                    "{row}: {label:?} not above {control:?}"
                );
                assert!(
                    label.right() <= dialog.right() && control.right() <= dialog.right(),
                    "{row} leaves {dialog:?}"
                );
                checked += 1;
            }
            checked
        })
        .unwrap();
    assert!(checked >= 20, "only {checked} rows found");
}

#[gpui_kit::test]
fn a_long_preferences_label_wraps_in_its_column(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let label = w
            .within("pref-keep-default-cas")
            .find("form-label")
            .bounds();
        let control = w
            .within("pref-keep-default-cas")
            .find("form-control")
            .bounds();
        assert!(
            label.size.height > at_zoom(INPUT_HEIGHT, 1.0),
            "{label:?} is one line"
        );
        assert!(label.right() <= control.left());
        assert_eq!(label.top(), control.top());
    })
    .unwrap();
}

/// The text the last frame painted inside element `id`, in paint order.
fn drawn_text_in(w: &gpui_kit::Window, id: &str) -> Vec<String> {
    let id = gpui_kit::ElementId::Name(id.to_string().into());
    gpui_kit::base::test_support::texts(w)
        .into_iter()
        .filter(|t| t.path().contains(&id))
        .map(|t| t.text().to_string())
        .collect()
}

// The response size limit says what it does in plain words. The row label is plain text the
// window does not report, so the search, which matches the label as drawn, reads it; the old
// name still finds the row.
#[gpui_kit::test]
fn the_response_size_limit_reads_in_plain_words(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    for query in [
        "Largest response to keep (MB, blank for no limit)",
        "body cap",
    ] {
        replace_input(cx, s.win, None, "prefs-search", query);
        frame(cx, s.win);
        assert!(present(cx, s.win, "pref-body-cap"), "{query}");
        assert!(!present(cx, s.win, "pref-verify-tls"), "{query}");
    }
}

// Each network row that waits for the next start says so beside its own control; the rows
// that apply at once do not.
#[gpui_kit::test]
fn network_rows_that_wait_for_a_restart_say_so_beside_the_control(cx: &mut TestAppContext) {
    const RESTART: &str = "Takes effect after restart";
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let rows = [
        ("pref-verify-tls", true),
        ("pref-body-cap", true),
        ("pref-request-timeout", false),
        ("pref-http-version", true),
        ("pref-proxy-mode", true),
        ("pref-ca-file", true),
        ("pref-keep-default-cas", true),
        ("pref-store-cookies", false),
        ("pref-send-cookies", false),
    ];
    for (row, waits) in rows {
        prefs_reveal(cx, &s, row);
        let drawn = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                drawn_text_in(w, row)
            })
            .unwrap();
        assert_eq!(
            drawn.iter().any(|t| t == RESTART),
            waits,
            "{row}: {drawn:?}"
        );
    }
}

// The layout choices say how the request and response sit, not an axis.
#[gpui_kit::test]
fn the_layout_choices_read_stacked_and_side_by_side(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let labels: Vec<Option<String>> = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (0..2usize)
                .map(|ix| w.within("orientation").find(ix).label().map(str::to_owned))
                .collect()
        })
        .unwrap();
    assert_eq!(
        labels,
        [Some("Stacked".to_owned()), Some("Side by side".to_owned())]
    );
}
