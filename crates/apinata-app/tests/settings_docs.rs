use crate::support;

use apinata_app::command::Edit;
use apinata_app::settings_section::{
    DIGITS_NOTE, DocsMode, DocsSection, SettingsSection, TIMEOUT_ZERO_NOTE, TimeoutMode,
};
use apinata_app::style::{FORM_INPUT_WIDTH, at_zoom};
use apinata_core::{RequestSettings, Timeout};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use support::*;

fn settings(
    cx: &mut TestAppContext,
    base: RequestSettings,
) -> (
    AnyWindowHandle,
    Entity<SectionHost<SettingsSection>>,
    Entity<SettingsSection>,
) {
    let (win, host) = host_section(cx, SettingsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&base, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host, s)
}

fn last(cx: &mut TestAppContext, host: &Entity<SectionHost<SettingsSection>>) -> RequestSettings {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Settings(s)) => s,
        _ => panic!("no settings edit"),
    }
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn found(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn toggles_send_the_settings(cx: &mut TestAppContext) {
    let (win, host, _s) = settings(cx, RequestSettings::new());
    let mut want = RequestSettings::new();
    click(cx, win, "set-follow-redirects");
    want.follow_redirects = false;
    assert_eq!(last(cx, &host), want);
    click(cx, win, "set-encode-url");
    want.encode_url = false;
    assert_eq!(last(cx, &host), want);
    click(cx, win, "set-forward-auth");
    want.forward_authorization_header = false;
    assert_eq!(last(cx, &host), want);
}

#[gpui_kit::test]
fn max_redirects_take_digits_only(cx: &mut TestAppContext) {
    let (win, host, _s) = settings(cx, RequestSettings::new());
    replace_input(cx, win, None, "set-max-redirects", "2");
    cx.run_until_parked();
    assert_eq!(last(cx, &host).max_redirects, 2);
    let n = host.read_with(cx, |h, _| h.edits.len());
    type_at_end(cx, win, None, "set-max-redirects", "x");
    cx.run_until_parked();
    assert_eq!(host.read_with(cx, |h, _| h.edits.len()), n);
    assert!(found(cx, win, "set-max-redirects-error"));
    replace_input(cx, win, None, "set-max-redirects", "");
    cx.run_until_parked();
    assert_eq!(last(cx, &host).max_redirects, 0);
    assert!(!found(cx, win, "set-max-redirects-error"));
}

// With following off the limit means nothing, so its field is disabled and says why on
// hover; the number itself is kept for when following comes back on.
#[gpui_kit::test]
fn max_redirects_wait_while_following_is_off(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.max_redirects = 7;
    let (win, host, s) = settings(cx, base);
    let input = s.read_with(cx, |s, _| s.max_redirects_input().clone());
    let disabled = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        input.read_with(cx, |i, _| i.presentation().is_disabled())
    };
    assert!(!disabled(cx));
    click(cx, win, "set-follow-redirects");
    assert!(disabled(cx));
    assert_eq!(last(cx, &host).max_redirects, 7);
    cx.update_window(win, |_, w, cx| w.hover("set-max-redirects", cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    let tip = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("set-max-redirects-tooltip")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        tip.as_deref(),
        Some(apinata_app::settings_section::MAX_REDIRECTS_OFF_TIP)
    );
    click(cx, win, "set-follow-redirects");
    assert!(!disabled(cx));
    assert_eq!(input.read_with(cx, |i, _| i.value().to_string()), "7");
    assert_eq!(last(cx, &host).max_redirects, 7);
}

fn mode(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    s: &Entity<SettingsSection>,
    mode: TimeoutMode,
) {
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_timeout_mode(mode, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn sent(cx: &mut TestAppContext, host: &Entity<SectionHost<SettingsSection>>) -> usize {
    host.read_with(cx, |h, _| h.edits.len())
}

// Never time out is a choice of its own, so Custom always holds a limit above 0: picking Custom
// with no number yet sends nothing until one is typed, and a typed 0 is refused.
#[gpui_kit::test]
fn timeout_is_default_no_limit_or_custom(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Inherit;
    let (win, host, s) = settings(cx, base);
    assert_eq!(
        s.read_with(cx, |s, _| s.timeout_mode()),
        TimeoutMode::Inherit
    );
    assert!(!found(cx, win, "set-timeout"));
    mode(cx, win, &s, TimeoutMode::Unlimited);
    assert_eq!(last(cx, &host).timeout, Timeout::Unlimited);
    assert!(!found(cx, win, "set-timeout"));
    let before = sent(cx, &host);
    mode(cx, win, &s, TimeoutMode::Custom);
    assert_eq!(
        sent(cx, &host),
        before,
        "Custom with no number sends nothing"
    );
    assert_eq!(
        s.read_with(cx, |s, _| s.timeout_mode()),
        TimeoutMode::Custom
    );
    assert!(found(cx, win, "set-timeout"));
    let shown = s.read_with(cx, |s, cx| s.timeout_input().read(cx).value().to_string());
    assert_eq!(shown, "");
    replace_input(cx, win, None, "set-timeout", "5000");
    cx.run_until_parked();
    assert_eq!(last(cx, &host).timeout, Timeout::Millis(5000));
    replace_input(cx, win, None, "set-timeout", "0");
    cx.run_until_parked();
    assert_eq!(last(cx, &host).timeout, Timeout::Millis(5000));
    assert!(found(cx, win, "set-timeout-error"));
    mode(cx, win, &s, TimeoutMode::Unlimited);
    assert_eq!(last(cx, &host).timeout, Timeout::Unlimited);
    mode(cx, win, &s, TimeoutMode::Custom);
    assert_eq!(last(cx, &host).timeout, Timeout::Millis(5000));
    mode(cx, win, &s, TimeoutMode::Inherit);
    assert_eq!(last(cx, &host).timeout, Timeout::Inherit);
}

// A refused number is reported under its own field in the error style Preferences uses: the
// danger icon and, while focused, the danger ring on the input, then the reason below it.
fn refused_under_its_field(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    (row, input, note): (&'static str, &'static str, &'static str),
) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let error = w
            .within(row)
            .find(gpui_kit::SharedString::from(format!("{input}-error")));
        assert_eq!(error.label(), Some(note));
        let error = error.bounds();
        w.within(row).find(gpui_kit::SharedString::from(format!(
            "{input}-invalid-icon"
        )));
        w.within(row)
            .find(gpui_kit::SharedString::from(format!("{input}-danger-ring")));
        let field = w.find(input).bounds();
        assert!(error.top() >= field.bottom(), "{error:?} under {field:?}");
        let tags = w.find("set-row-tags").bounds();
        assert!(error.bottom() <= tags.top(), "{error:?} above {tags:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_zero_timeout_is_refused_under_the_timeout_field(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Millis(5000);
    let (win, _host, _s) = settings(cx, base);
    replace_input(cx, win, None, "set-timeout", "0");
    cx.run_until_parked();
    refused_under_its_field(
        cx,
        win,
        ("set-row-timeout", "set-timeout", TIMEOUT_ZERO_NOTE),
    );
    assert!(!found(cx, win, "set-max-redirects-error"));
}

#[gpui_kit::test]
fn a_non_digit_max_redirects_is_refused_under_its_field(cx: &mut TestAppContext) {
    let (win, _host, _s) = settings(cx, RequestSettings::new());
    type_at_end(cx, win, None, "set-max-redirects", "x");
    cx.run_until_parked();
    refused_under_its_field(
        cx,
        win,
        ("set-row-max-redirects", "set-max-redirects", DIGITS_NOTE),
    );
    assert!(!found(cx, win, "set-timeout-error"));
}

// Each timeout choice says under the row where its limit comes from, so Default and Never
// time out read apart even when the Preferences default is no limit.
#[gpui_kit::test]
fn each_timeout_choice_says_where_its_limit_comes_from(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Inherit;
    let (win, _host, s) = settings(cx, base);
    for m in [
        TimeoutMode::Inherit,
        TimeoutMode::Unlimited,
        TimeoutMode::Custom,
    ] {
        mode(cx, win, &s, m);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let hint = w.within("set-row-timeout").find("set-timeout-hint");
            assert_eq!(hint.label(), Some(m.hint()), "{m:?}");
        })
        .unwrap();
    }
}

// A file's `timeout: 0` reads as Never time out, not as a custom timeout of 0.
#[gpui_kit::test]
fn a_zero_timeout_shows_as_no_limit(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Unlimited;
    let (win, _host, s) = settings(cx, base);
    assert_eq!(
        s.read_with(cx, |s, _| s.timeout_mode()),
        TimeoutMode::Unlimited
    );
    assert_eq!(
        s.read_with(cx, |s, cx| s.timeout_shown(cx)).as_deref(),
        Some("Never time out")
    );
    assert!(!found(cx, win, "set-timeout"));
}

#[gpui_kit::test]
fn docs_edit_as_markdown(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, DocsSection::new);
    let d = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| d.update(cx, |d, cx| d.load("# Old", w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let code = d.read_with(cx, |d, _| d.code().clone());
    assert_eq!(
        code.read_with(cx, |c, _| (
            c.value().to_string(),
            c.language_name().to_string()
        )),
        ("# Old".into(), "markdown".into())
    );
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    pick(cx, win, 1);
    replace_code(cx, win, None, "docs-code", "# Title");
    cx.update_window(win, |_, w, cx| {
        w.press("enter", cx);
        w.input("line two", cx)
    })
    .unwrap();
    cx.run_until_parked();
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Docs(text)) => assert_eq!(text, "# Title\nline two"),
        _ => panic!("no docs edit"),
    }
}

#[gpui_kit::test]
fn the_settings_form_shares_two_columns(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Millis(5000);
    let (win, _host, _s) = settings(cx, base);
    let rows = [
        "set-row-encode-url",
        "set-row-follow-redirects",
        "set-row-forward-auth",
        "set-row-max-redirects",
        "set-row-timeout",
    ];
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let cols: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    w.within(*r).find("form-label").bounds().left(),
                    w.within(*r).find("form-control").bounds().left(),
                )
            })
            .collect();
        assert!(cols.windows(2).all(|p| p[0] == p[1]), "{cols:?}");
        assert!(
            w.within("set-row-encode-url")
                .try_find("set-encode-url")
                .is_some()
        );
        assert!(
            w.within("set-row-timeout")
                .try_find("set-timeout")
                .is_some()
        );
        assert_eq!(
            w.find("set-max-redirects").bounds().size.width,
            at_zoom(FORM_INPUT_WIDTH, 1.0)
        );
        assert_eq!(
            w.find("set-timeout").bounds().size.width,
            at_zoom(FORM_INPUT_WIDTH, 1.0)
        );
        assert_eq!(
            w.find("tag-input").bounds().size.width,
            at_zoom(FORM_INPUT_WIDTH, 1.0)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_docs_editor_fills_its_slot(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, DocsSection::new);
    let d = host.read_with(cx, |h, _| h.view.clone());
    let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
    cx.update_window(win, |_, w, cx| d.update(cx, |d, cx| d.load(&text, w, cx)))
        .unwrap();
    cx.run_until_parked();
    pick(cx, win, 1);
    let code = d.read_with(cx, |d, _| d.code().clone());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let slot = w.find("docs-code").bounds();
        let input = code.read(cx).input_bounds();
        assert!(
            input.size.height >= slot.size.height - gpui_kit::px(20.),
            "editor {input:?} short of its slot {slot:?}"
        );
        assert!(
            input.bottom() <= slot.bottom() + gpui_kit::px(0.5),
            "{input:?} past {slot:?}"
        );
    })
    .unwrap();
}

// The custom timeout names its unit right beside the field.
#[gpui_kit::test]
fn a_custom_timeout_names_its_unit_beside_the_field(cx: &mut TestAppContext) {
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Millis(5000);
    let (win, _host, _s) = settings(cx, base);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let input = w.find("set-timeout").bounds();
        let unit = w.within("set-row-timeout").find("set-timeout-unit");
        assert_eq!(unit.label(), Some("ms"));
        let unit = unit.bounds();
        assert!(unit.left() >= input.right(), "{unit:?} after {input:?}");
        assert!(unit.top() >= input.top() && unit.bottom() <= input.bottom());
    })
    .unwrap();
}

fn docs(
    cx: &mut TestAppContext,
    text: &str,
) -> (
    AnyWindowHandle,
    Entity<SectionHost<DocsSection>>,
    Entity<DocsSection>,
) {
    let (win, host) = host_section(cx, DocsSection::new);
    let d = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| d.update(cx, |d, cx| d.load(text, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host, d)
}

fn pick(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.within("docs-mode").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn written_docs_open_rendered(cx: &mut TestAppContext) {
    let (win, _host, d) = docs(cx, "# Title\n\n- one\n- two");
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Preview);
    assert!(present(cx, win, "docs-preview"));
    assert!(!present(cx, win, "docs-code"));
    assert!(!present(cx, win, "docs-empty"));
}

#[gpui_kit::test]
fn empty_docs_open_for_editing(cx: &mut TestAppContext) {
    let (win, _host, d) = docs(cx, "  \n");
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Edit);
    assert!(present(cx, win, "docs-code"));
    assert!(!present(cx, win, "docs-preview"));
}

#[gpui_kit::test]
fn the_toggle_switches_views_and_the_preview_shows_unsaved_text(cx: &mut TestAppContext) {
    let (win, host, d) = docs(cx, "# Old");
    pick(cx, win, 1);
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Edit);
    assert!(present(cx, win, "docs-code"));
    replace_code(cx, win, None, "docs-code", "# New");
    cx.run_until_parked();
    pick(cx, win, 0);
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Preview);
    assert!(present(cx, win, "docs-preview"));
    assert_eq!(d.read_with(cx, |d, cx| d.preview_text(cx)), "# New");
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Docs(text)) => assert_eq!(text, "# New"),
        _ => panic!("no docs edit"),
    }
}

#[gpui_kit::test]
fn a_later_load_keeps_the_mode(cx: &mut TestAppContext) {
    let (win, _host, d) = docs(cx, "");
    cx.update_window(win, |_, w, cx| {
        d.update(cx, |d, cx| d.load("# Now written", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Edit);

    let (win, _host, d) = docs(cx, "# Written");
    cx.update_window(win, |_, w, cx| d.update(cx, |d, cx| d.load("", w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Preview);
    assert!(present(cx, win, "docs-empty"));
}

#[gpui_kit::test]
fn an_echo_of_typing_keeps_the_editor_open(cx: &mut TestAppContext) {
    let (win, _host, d) = docs(cx, "");
    replace_code(cx, win, None, "docs-code", "# Typed");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        d.update(cx, |d, cx| d.load("# Typed", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Edit);
    assert!(present(cx, win, "docs-code"));
}

#[gpui_kit::test]
fn choosing_edit_focuses_the_editor(cx: &mut TestAppContext) {
    let (win, _host, d) = docs(cx, "# Written");
    pick(cx, win, 1);
    let code = d.read_with(cx, |d, _| d.code().clone());
    cx.update_window(win, |_, w, cx| assert!(code.focus_handle(cx).is_focused(w)))
        .unwrap();
}

#[gpui_kit::test]
fn a_mode_chosen_before_the_first_load_is_kept(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, DocsSection::new);
    let d = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        d.update(cx, |d, cx| {
            d.set_mode(DocsMode::Edit, w, cx);
            d.load("# Written", w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(d.read_with(cx, |d, _| d.mode()), DocsMode::Edit);
}

#[gpui_kit::test]
fn the_settings_form_stacks_at_the_minimum_window(cx: &mut TestAppContext) {
    use apinata_app::session::{MIN_HEIGHT, MIN_WIDTH};
    let mut base = RequestSettings::new();
    base.timeout = Timeout::Millis(5000);
    let (win, _host, _s) = settings(cx, base);
    cx.simulate_window_resize(
        win,
        gpui_kit::size(gpui_kit::px(MIN_WIDTH), gpui_kit::px(MIN_HEIGHT.max(600.))),
    );
    frame(cx, win);
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        for r in [
            "set-row-encode-url",
            "set-row-follow-redirects",
            "set-row-forward-auth",
            "set-row-max-redirects",
            "set-row-timeout",
        ] {
            let label = w.within(r).find("form-label").bounds();
            let control = w.within(r).find("form-control").bounds();
            assert!(
                label.bottom() <= control.top(),
                "{r}: {label:?} over {control:?}"
            );
            assert_eq!(label.left(), control.left(), "{r}");
        }
    })
    .unwrap();
}

// With no tags yet, the Tags field is as tall as the inputs above it.
#[gpui_kit::test]
fn an_empty_tags_field_is_as_tall_as_the_other_inputs(cx: &mut TestAppContext) {
    let (win, _host, _s) = settings(cx, RequestSettings::new());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let tags = w.find("tag-input").bounds().size.height;
        let redirects = w.find("set-max-redirects").bounds().size.height;
        assert_eq!(tags, redirects);
    })
    .unwrap();
}
