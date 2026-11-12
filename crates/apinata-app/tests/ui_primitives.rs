//! The presentational primitives: method labels, status classes, and the rendered
//! elements' ids, sizes and dispatch.

use crate::support;

use std::sync::atomic::{AtomicUsize, Ordering};

use apinata_app::focus_mode::Modality;
use apinata_app::style::{
    DOT, FORM_LABEL_WIDTH, INPUT_HEIGHT, METHOD_COLUMN, SHORT_FORM_LABEL_WIDTH, SPACE_2, at_zoom,
};
use apinata_app::ui::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::v_flex;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    Action, AnyElement, AnyWindowHandle, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Render, Styled,
    TestAppContext, TestSupportExt as _, Window, WindowOptions, actions, div, px,
};
use support::*;

actions!(probe, [Ping, Unbound, Shelled]);

struct Host {
    focus: FocusHandle,
    pings: usize,
    build: fn() -> AnyElement,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("host")
            .size_full()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Ping, _, _| this.pings += 1))
            .child((self.build)())
    }
}

fn host(cx: &mut TestAppContext, build: fn() -> AnyElement) -> (AnyWindowHandle, Entity<Host>) {
    support::init(cx);
    cx.update(|cx| cx.bind_keys([KeyBinding::new("ctrl-k", Ping, None)]));
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Host {
                    focus: cx.focus_handle(),
                    pings: 0,
                    build,
                })
            })
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        let focus = view.read(cx).focus.clone();
        focus.focus(w, cx);
    })
    .unwrap();
    frame(cx, win);
    (win, view)
}

fn pings(cx: &mut TestAppContext, view: &Entity<Host>) -> usize {
    cx.update(|cx| view.read(cx).pings)
}

#[test]
fn short_methods() {
    for (m, want) in [
        ("GET", "GET"),
        ("post", "POST"),
        ("PUT", "PUT"),
        ("PATCH", "PTCH"),
        ("DELETE", "DEL"),
        ("options", "OPT"),
        ("HEAD", "HEAD"),
        ("trace", "TRAC"),
        ("x", "X"),
        ("propfind", "PROP"),
        ("query", "QURY"),
    ] {
        assert_eq!(short_method(m).as_ref(), want, "{m}");
        assert_eq!(method_tag(m).label().as_ref(), want);
    }
}

#[test]
fn status_classes_and_text() {
    for (s, c) in [
        (100, StatusClass::Muted),
        (199, StatusClass::Muted),
        (200, StatusClass::Success),
        (299, StatusClass::Success),
        (301, StatusClass::Info),
        (404, StatusClass::Warning),
        (500, StatusClass::Danger),
        (599, StatusClass::Danger),
        (600, StatusClass::Muted),
    ] {
        assert_eq!(status_class(s), c, "{s}");
        assert_eq!(status_pill(s, "R").class(), c);
    }
    assert_eq!(status_pill(200, "OK").text().as_ref(), "200 OK");
    assert_eq!(status_pill(204, "").text().as_ref(), "204");
}

#[test]
fn count_badge_hides_zero() {
    assert!(count_badge(0).is_none());
    assert!(count_badge(1).is_some());
    assert!(count_badge(1200).is_some());
}

#[gpui_kit::test]
fn dirty_dot_and_method_tag_sizes(cx: &mut TestAppContext) {
    // A flex row lets each child keep its own width instead of stretching to the window.
    let (win, _) = host(cx, || {
        div()
            .flex()
            .child(dirty_dot("dot").test_support())
            .child(div().id("tag").test_support().child(method_tag("DELETE")))
            .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let dot = w.find("dot").bounds();
        assert_eq!(
            (dot.size.width, dot.size.height),
            (at_zoom(DOT, 1.0), at_zoom(DOT, 1.0))
        );
        assert_eq!(
            w.find("tag").bounds().size.width,
            at_zoom(METHOD_COLUMN, 1.0)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn icon_button_dispatches_its_action(cx: &mut TestAppContext) {
    let (win, view) = host(cx, || {
        div()
            .child(icon_button("ping", IconName::Send, "Ping", Some(&Ping)))
            .child(icon_button("plain", IconName::Close, "Close", None))
            .into_any_element()
    });
    cx.update_window(win, |_, w, cx| w.click("ping", cx))
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("plain", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(pings(cx, &view), 1);
}

#[gpui_kit::test]
fn empty_state_shows_actions_with_bound_hints(cx: &mut TestAppContext) {
    let (win, view) = host(cx, || {
        let actions: [(&str, &dyn Action); 2] = [("Ping", &Ping), ("Other", &Unbound)];
        empty_state("empty", "Nothing here", &actions).into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let scope = w.within("empty");
        assert!(scope.try_find(("empty-action", 0usize)).is_some());
        assert!(scope.try_find(("empty-action", 1usize)).is_some());
        assert!(
            scope.try_find(("empty-kbd", 0usize)).is_some(),
            "Ping is bound to ctrl-k"
        );
        assert!(
            scope.try_find(("empty-kbd", 1usize)).is_none(),
            "Unbound has no key"
        );
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| {
        w.within("empty").click(("empty-action", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(pings(cx, &view), 1);
}

static PICKED: AtomicUsize = AtomicUsize::new(usize::MAX);

#[gpui_kit::test]
fn segmented_marks_the_current_option_and_reports_clicks(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        segmented("seg", ["One", "Two", "Three"], 1, |ix, _, _| {
            PICKED.store(*ix, Ordering::SeqCst)
        })
        .into_any_element()
    });
    cx.update_window(win, |_, w, cx| {
        let marks: Vec<_> = (0..3usize)
            .map(|ix| w.within("seg").find(ix).checked())
            .collect();
        assert_eq!(marks, [Some(false), Some(true), Some(false)]);
        w.within("seg").click(2usize, cx);
    })
    .unwrap();
    assert_eq!(PICKED.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn form_rows_share_their_columns(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        v_flex()
            .child(form_heading("form-heading", "Section"))
            .child(form_row(
                "row-a",
                "Short",
                div().id("ctl-a").test_support().w(px(40.)),
            ))
            .child(form_row(
                "row-b",
                "A much longer label",
                div().id("ctl-b").test_support().w(px(80.)),
            ))
            .child(form_note("note", "Helper text"))
            .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let la = w.within("row-a").find("form-label").bounds();
        let lb = w.within("row-b").find("form-label").bounds();
        assert_eq!(la.left(), lb.left());
        assert_eq!(
            (la.size.width, lb.size.width),
            (
                at_zoom(FORM_LABEL_WIDTH, 1.0),
                at_zoom(FORM_LABEL_WIDTH, 1.0)
            )
        );
        let ca = w.within("row-a").find("form-control").bounds().left();
        let cb = w.within("row-b").find("form-control").bounds().left();
        assert_eq!(ca, cb);
        assert_eq!(
            ca,
            la.left() + at_zoom(FORM_LABEL_WIDTH, 1.0) + at_zoom(SPACE_2, 1.0)
        );
        assert_eq!(w.find("ctl-a").bounds().left(), ca);
        assert!(w.find("form-heading").bounds().bottom() <= la.top());
        let note = w.find("note").bounds();
        assert_eq!(note.left(), ca);
        assert!(note.top() >= lb.bottom());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_form_note_is_smaller_than_a_form_label(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        v_flex()
            .child(form_row("row", "Label", div()))
            .child(form_note("note", "Note"))
            .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let label = w.within("row").find("form-label").bounds();
        let note = w.find("note").bounds();
        assert!(
            note.size.height < label.size.height,
            "note {note:?}, label {label:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn empty_state_hints_shell_keys_outside_the_shell(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([KeyBinding::new(
            "ctrl-j",
            Shelled,
            Some(apinata_app::keymap::SHELL_CONTEXT),
        )])
    });
    let (win, _) = host(cx, || {
        let actions: [(&str, &dyn Action); 1] = [("Shelled", &Shelled)];
        empty_state("empty", "Nothing here", &actions).into_any_element()
    });
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        assert!(
            w.within("empty").try_find(("empty-kbd", 0usize)).is_some(),
            "a Shell key shows while focus is outside the Shell"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_selected_segment_is_a_raised_fill_edged_in_neither_ring_nor_brand(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| apinata_app::focus_mode::set(Modality::Keyboard, cx));
    cx.update(|cx| {
        let (ring, brand, raised) = {
            let t = cx.theme();
            (t.ring, t.primary, t.popover)
        };
        let selected = segment_fill(true, cx);
        assert_eq!(selected, raised);
        assert_ne!(selected, ring);
        assert_ne!(selected, brand);
        assert_eq!(segment_fill(false, cx).a, 0.0);
        let edge = segment_edge(true, cx).expect("the selected segment has an edge");
        assert_ne!(edge, ring);
        assert_ne!(edge, brand);
        assert_eq!(segment_edge(false, cx), None);
    });
}

#[gpui_kit::test]
fn a_segment_is_as_big_as_a_plain_ghost_button(cx: &mut TestAppContext) {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    // A bordered segment is 2 px larger each way than the same ghost button without one.
    let (win, _) = host(cx, || {
        v_flex()
            .child(segmented("same", ["Same", "Same"], 0, |_, _, _| {}))
            .child(
                div()
                    .flex()
                    .child(Button::new("plain").ghost().label("Same")),
            )
            .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let plain = w.find("plain").bounds().size;
        let a = w.within("same").find(0usize).bounds().size;
        let b = w.within("same").find(1usize).bounds().size;
        assert_eq!(
            (a, b),
            (plain, plain),
            "selected and unselected carry no border"
        );
    })
    .unwrap();
}

static PICKED_ID: AtomicUsize = AtomicUsize::new(usize::MAX);

#[gpui_kit::test]
fn segments_with_their_own_ids_keep_the_arrow_keys(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        segmented("ided", ["A", "B"], 0, |ix, _, _| {
            PICKED_ID.store(*ix, Ordering::SeqCst)
        })
        .ids(["seg-a", "seg-b"])
        .into_any_element()
    });
    cx.update(|cx| {
        cx.bind_keys([KeyBinding::new(
            "right",
            NextSegment,
            Some(SEGMENTED_CONTEXT),
        )])
    });
    cx.update_window(win, |_, w, cx| {
        assert_eq!(w.find("seg-a").checked(), Some(true));
        assert_eq!(w.find("seg-b").checked(), Some(false));
        w.focus_next(cx);
        w.render_frame(cx);
        assert_eq!(
            w.find("seg-a").focused(),
            Some(true),
            "Tab reaches the first segment"
        );
        w.press("right", cx);
        assert_eq!(
            w.find("seg-b").focused(),
            Some(true),
            "right moves focus by id"
        );
    })
    .unwrap();
    assert_eq!(PICKED_ID.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn field_fills_are_the_raised_fill_and_disabled_differs(cx: &mut TestAppContext) {
    use gpui_kit::component::ThemeMode;
    support::init(cx);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        cx.update(|cx| {
            let t = cx.theme();
            assert_eq!(field_fill(false, cx), t.secondary, "{mode:?}");
            assert_eq!(field_fill(true, cx), t.muted, "{mode:?}");
            assert_ne!(field_fill(true, cx), field_fill(false, cx), "{mode:?}");
        });
    }
}

#[test]
fn every_text_input_sets_its_fill() {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                for (n, line) in lines.iter().enumerate() {
                    if line.contains("Input::new(") {
                        let chain = lines[n..(n + 10).min(lines.len())].join("\n");
                        if !chain.contains("field_fill(") {
                            out.push(format!("{}:{}", p.display(), n + 1));
                        }
                    }
                }
            }
        }
    }
    let mut misses = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut misses,
    );
    assert!(
        misses.is_empty(),
        "inputs without ui::field_fill:\n{}",
        misses.join("\n")
    );
}

#[test]
fn backticks_become_plain_quotes() {
    assert_eq!(
        plain_quotes("variable `token` is not defined"),
        "variable \"token\" is not defined"
    );
    assert_eq!(plain_quotes("no quotes"), "no quotes");
}

// Each common method has its own status colour; the rest stay muted, as before.
#[test]
fn methods_take_their_colour_class() {
    for (m, want) in [
        ("GET", StatusClass::Success),
        ("post", StatusClass::Warning),
        ("PUT", StatusClass::Info),
        ("patch", StatusClass::Info),
        ("DELETE", StatusClass::Danger),
        ("HEAD", StatusClass::Muted),
        ("OPTIONS", StatusClass::Muted),
        ("propfind", StatusClass::Muted),
    ] {
        assert_eq!(method_class(m), want, "{m}");
        assert_eq!(method_tag(m).class(), want, "{m}");
    }
}

// The tag's text colour is the class's theme colour everywhere, a selected row included: the
// selected fill is neutral, so the method keeps its colour there.
#[gpui_kit::test]
fn a_method_tag_takes_its_class_colour(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        let t = cx.theme().clone();
        for (m, want) in [
            ("GET", t.success),
            ("POST", t.warning),
            ("PUT", t.info),
            ("PATCH", t.info),
            ("DELETE", t.danger),
            ("HEAD", t.muted_foreground),
        ] {
            assert_eq!(method_tag(m).color(cx), want, "{m}");
        }
    });
}

thread_local! {
    static TAG_PAINTS: TagPaints = TagPaints::default();
}

// The colour a tag applies in its render is the class colour in both themes, which is what a
// selected tree row shows (the selected fill is neutral). It is read from the render itself,
// so a row style that recoloured the tag text would fail here, not only `color()`.
#[gpui_kit::test]
fn a_rendered_method_tag_paints_its_class_colour_in_both_themes(cx: &mut TestAppContext) {
    use gpui_kit::component::ThemeMode;
    let (win, _) = host(cx, || {
        let paints = TAG_PAINTS.with(|p| p.clone());
        div()
            .flex()
            .children(
                ["GET", "POST", "DELETE", "HEAD"]
                    .map(|m| method_tag(m).record(&paints, m.into()).into_any_element()),
            )
            .into_any_element()
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        TAG_PAINTS.with(|p| p.borrow_mut().clear());
        frame(cx, win);
        let (success, warning, danger, muted, foreground) = cx.update(|cx| {
            let t = cx.theme();
            (
                t.success,
                t.warning,
                t.danger,
                t.muted_foreground,
                t.foreground,
            )
        });
        for (m, want) in [
            ("GET", success),
            ("POST", warning),
            ("DELETE", danger),
            ("HEAD", muted),
        ] {
            let got = TAG_PAINTS.with(|p| p.borrow().get(m).copied());
            assert_eq!(got, Some(want), "{mode:?} {m}");
            assert_ne!(want, foreground, "{mode:?} {m} differs from the row text");
        }
    }
}

fn centre_y(b: Bounds<Pixels>) -> f32 {
    f32::from(b.top()) + f32::from(b.size.height) / 2.
}

#[gpui_kit::test]
fn a_form_label_sits_on_the_first_line_of_its_control(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        v_flex()
            .child(form_row(
                "row-input",
                "Input",
                div()
                    .id("line-input")
                    .test_support()
                    .w(px(80.))
                    .h(INPUT_HEIGHT),
            ))
            .child(form_row(
                "row-error",
                "With an error",
                v_flex()
                    .child(
                        div()
                            .id("line-error")
                            .test_support()
                            .w(px(80.))
                            .h(INPUT_HEIGHT),
                    )
                    .child(
                        div()
                            .id("error-text")
                            .test_support()
                            .h(px(40.))
                            .child("A reason"),
                    ),
            ))
            .child(form_row(
                "row-check",
                "Checkbox",
                div().id("line-check").test_support().size(px(16.)),
            ))
            .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        for (row, line) in [
            ("row-input", "line-input"),
            ("row-error", "line-error"),
            ("row-check", "line-check"),
        ] {
            let label = w.within(row).find("form-label").bounds();
            let first = w.find(line).bounds();
            assert!(
                (centre_y(label) - centre_y(first)).abs() < 0.5,
                "{row}: label {label:?}, first line {first:?}"
            );
        }
        let error = w.find("error-text").bounds();
        let label = w.within("row-error").find("form-label").bounds();
        assert!(
            error.top() >= label.bottom(),
            "the error sits below the label line"
        );
    })
    .unwrap();
}

// Every `src` line containing `pattern`, as "file:line".
fn src_hits(pattern: &str) -> Vec<String> {
    fn walk(dir: &std::path::Path, pattern: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, pattern, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                for (n, line) in text.lines().enumerate() {
                    if line.contains(pattern) {
                        out.push(format!("{}:{}", p.display(), n + 1));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        pattern,
        &mut out,
    );
    out
}

#[test]
fn no_view_uses_the_kit_checkbox() {
    let hits = src_hits("component::checkbox");
    assert!(hits.is_empty(), "use ui::checkbox:\n{}", hits.join("\n"));
}

#[test]
fn every_panel_switch_uses_the_tab_strip() {
    let stray: Vec<_> = src_hits("TabBar::new(")
        .into_iter()
        .filter(|hit| !hit.contains("/ui.rs:") && !hit.contains("/tab_area.rs:"))
        .collect();
    assert!(stray.is_empty(), "use ui::tab_strip:\n{}", stray.join("\n"));
}

#[gpui_kit::test]
fn the_dialog_footer_puts_the_primary_at_the_right_edge(cx: &mut TestAppContext) {
    use gpui_kit::component::button::Button;
    let (win, _) = host(cx, || {
        dialog_footer(
            Some(Button::new("f-cancel").label("Cancel")),
            [Button::new("f-other").label("Other")],
            Button::new("f-ok").label("OK"),
        )
        .hint(div().id("f-hint").test_support().child("Esc: dismiss"))
        .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let footer = w.find("dialog-footer").bounds();
        let [hint, cancel, other, ok] =
            ["f-hint", "f-cancel", "f-other", "f-ok"].map(|id| w.find(id).bounds());
        assert!(hint.right() <= cancel.left());
        assert!(cancel.right() < other.left() && other.right() < ok.left());
        assert_eq!(other.left() - cancel.right(), at_zoom(SPACE_2, 1.0));
        assert_eq!(ok.right(), footer.right());
        assert_eq!(
            ok.top() - footer.top(),
            at_zoom(apinata_app::style::DIALOG_FOOTER_GAP, 1.0)
        );
    })
    .unwrap();
}

#[test]
fn no_info_toast_carries_an_icon() {
    let mut hits = src_hits("Notification::info(");
    hits.extend(src_hits("NotificationType::Info"));
    assert!(hits.is_empty(), "use ui::info_toast:\n{}", hits.join("\n"));
}

fn row_parts(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    row: &'static str,
) -> (Bounds<Pixels>, Bounds<Pixels>) {
    cx.update_window(win, |_, w, _| {
        (
            w.within(row).find("form-label").bounds(),
            w.within(row).find("form-control").bounds(),
        )
    })
    .unwrap()
}

fn line(id: &'static str) -> impl IntoElement {
    div().id(id).test_support().w(px(80.)).h(INPUT_HEIGHT)
}

#[gpui_kit::test]
fn a_narrow_form_row_stacks_its_label_over_the_control(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        v_flex()
            .child(
                div()
                    .w(px(400.))
                    .child(form_row("narrow", "Label", line("ctl-narrow"))),
            )
            .child(
                div()
                    .w(px(700.))
                    .child(form_row("wide", "Label", line("ctl-wide"))),
            )
            .into_any_element()
    });
    frame(cx, win);
    let (label, control) = row_parts(cx, win, "narrow");
    assert!(
        label.bottom() <= control.top(),
        "{label:?} over {control:?}"
    );
    assert_eq!(label.left(), control.left());
    let (label, control) = row_parts(cx, win, "wide");
    assert_eq!(label.top(), control.top());
    assert_eq!(label.size.width, at_zoom(FORM_LABEL_WIDTH, 1.0));
    assert!(label.right() <= control.left());
}

#[gpui_kit::test]
fn a_form_row_restacks_when_its_window_narrows_and_widens(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        form_row("row", "Label", line("ctl")).into_any_element()
    });
    let stacked = |cx: &mut TestAppContext| {
        frame(cx, win);
        frame(cx, win);
        let (label, control) = row_parts(cx, win, "row");
        label.bottom() <= control.top()
    };
    assert!(!stacked(cx));
    cx.simulate_window_resize(win, gpui_kit::size(px(400.), px(600.)));
    assert!(stacked(cx));
    cx.simulate_window_resize(win, gpui_kit::size(px(900.), px(600.)));
    assert!(!stacked(cx));
}

#[gpui_kit::test]
fn a_long_label_wraps_inside_its_column_from_the_top(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        form_row(
            "long",
            "A label long enough that it has to wrap onto a second line",
            line("ctl-long"),
        )
        .into_any_element()
    });
    let (label, control) = row_parts(cx, win, "long");
    assert_eq!(label.size.width, at_zoom(FORM_LABEL_WIDTH, 1.0));
    assert!(
        label.size.height > at_zoom(INPUT_HEIGHT, 1.0),
        "{label:?} did not wrap"
    );
    assert_eq!(label.top(), control.top());
}

#[gpui_kit::test]
fn a_hint_sits_under_the_control_in_smaller_text(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        form_row("hinted", "Label", line("ctl-hinted"))
            .hint("hinted-hint", "Helper text")
            .into_any_element()
    });
    cx.update_window(win, |_, w, _| {
        let control = w.find("ctl-hinted").bounds();
        let hint = w.find("hinted-hint").bounds();
        let label = w.within("hinted").find("form-label").bounds();
        assert_eq!(hint.left(), control.left());
        assert!(hint.top() >= control.bottom(), "{hint:?} under {control:?}");
        assert!(
            hint.size.height < label.size.height,
            "{hint:?} vs {label:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_label_column_never_stacks(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        div()
            .w(px(300.))
            .child(form_row("short", "Name", line("ctl-short")).label_width(SHORT_FORM_LABEL_WIDTH))
            .into_any_element()
    });
    frame(cx, win);
    let (label, control) = row_parts(cx, win, "short");
    assert_eq!(label.size.width, at_zoom(SHORT_FORM_LABEL_WIDTH, 1.0));
    assert_eq!(label.top(), control.top());
    assert!(label.right() <= control.left());
}

#[gpui_kit::test]
fn a_form_note_follows_stacked_rows(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        v_flex()
            .child(
                v_flex()
                    .w(px(400.))
                    .child(form_row("narrow-row", "Label", line("ctl-n")))
                    .child(form_note("narrow-note", "Note")),
            )
            .child(
                v_flex()
                    .w(px(700.))
                    .child(form_row("wide-row", "Label", line("ctl-w")))
                    .child(form_note("wide-note", "Note")),
            )
            .into_any_element()
    });
    frame(cx, win);
    let (narrow_label, _) = row_parts(cx, win, "narrow-row");
    let (_, wide_control) = row_parts(cx, win, "wide-row");
    cx.update_window(win, |_, w, _| {
        assert_eq!(w.find("narrow-note").bounds().left(), narrow_label.left());
        assert_eq!(w.find("wide-note").bounds().left(), wide_control.left());
    })
    .unwrap();
}

// A stacked row without a label leaves no blank band above its control.
#[gpui_kit::test]
fn a_stacked_row_without_a_label_starts_at_its_control(cx: &mut TestAppContext) {
    let (win, _) = host(cx, || {
        v_flex()
            .child(
                div()
                    .w(px(400.))
                    .child(form_row("bare-narrow", "", line("ctl-bare-narrow"))),
            )
            .child(
                div()
                    .w(px(700.))
                    .child(form_row("bare-wide", "", line("ctl-bare-wide"))),
            )
            .into_any_element()
    });
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let row = w.find("bare-narrow").bounds();
        let control = w.within("bare-narrow").find("form-control").bounds();
        assert_eq!(control.top(), row.top(), "{control:?} in {row:?}");
        // Side by side, the empty label still holds the label column.
        let label = w.within("bare-wide").find("form-label").bounds();
        assert_eq!(label.size.width, at_zoom(FORM_LABEL_WIDTH, 1.0));
    })
    .unwrap();
}

// A render records every header and body bar icon, so recording an unchanged glyph again
// must not allocate. The probe's path is too long to sit inline in a string.
#[test]
fn recording_an_unchanged_glyph_does_not_allocate() {
    clear_recorded_glyphs();
    glyph("glyph-probe", IconName::RotateCcwClock);
    let (_, allocated) = crate::counting(|| glyph("glyph-probe", IconName::RotateCcwClock));
    assert_eq!(allocated.count, 0, "{allocated:?}");
    let recorded = recorded_glyphs();
    let (_, path) = recorded
        .iter()
        .find(|(id, _)| *id == "glyph-probe")
        .expect("the probe is recorded");
    assert_eq!(*path, IconName::RotateCcwClock.path());
    clear_recorded_glyphs();
}

// A short dropdown leaves the kit's scrolling off, so it draws no scrollbar; one that may
// outgrow the kit's height cap (half the window, at most 450 px at a 16 px rem) scrolls.
#[test]
fn only_a_long_dropdown_scrolls() {
    let rem = gpui_kit::px(16.);
    assert!(!menu_may_overflow(2, gpui_kit::px(800.), rem));
    assert!(!menu_may_overflow(8, gpui_kit::px(800.), rem));
    assert!(menu_may_overflow(40, gpui_kit::px(800.), rem));
    assert!(
        menu_may_overflow(8, gpui_kit::px(300.), rem),
        "a short window"
    );
    assert!(
        menu_may_overflow(8, gpui_kit::px(800.), gpui_kit::px(40.)),
        "a large zoom"
    );
}
