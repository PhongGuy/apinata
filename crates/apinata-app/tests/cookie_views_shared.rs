//! The response pane's Jar view and the Cookies dialog describe a cookie alike: the same flag
//! words and tooltips, the same dots for a hidden value, copies and a filter that follow the
//! eye, and a way from the view into the dialog filtered to the response's host.

use crate::support;

use apinata_app::cookie_jar::{
    HTTP_ONLY_LABEL, HTTP_ONLY_TIP, SECURE_LABEL, SECURE_TIP, host_search, open_cookie_jar,
};
use apinata_app::executor::{SharedExecutor, shared};
use apinata_app::response_pane::{NAME_MASK, ResponsePane, ResponseTab};
use apinata_core::{CookieInfo, ExecOptions, Executor};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, ElementId, Entity, IntoElement, Render,
    TestAppContext, Window, WindowOptions, div,
};
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

struct Blank;
impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

// A response that set `sid=tok3n9`, HttpOnly and Secure, shown on the Cookies tab's Jar view.
fn jar_view(cx: &mut TestAppContext) -> (Sent, AnyWindowHandle, Entity<ResponsePane>) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200)
            .append_header("set-cookie", "sid=tok3n9; Path=/; HttpOnly; Secure"),
        "",
    );
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    cx.run_until_parked();
    frame(cx, win);
    (s, win, view)
}

// The dialog alone over a jar holding one Secure, HttpOnly cookie.
fn dialog(cx: &mut TestAppContext) -> AnyWindowHandle {
    support::init(cx);
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    let mut c = CookieInfo::new("a.test", "sid", "v");
    c.secure = true;
    c.http_only = true;
    exec.cookies().insert(c).unwrap();
    cx.update(|cx| cx.set_global(SharedExecutor(exec)));
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    cx.update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    win
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

// The tooltip `tip_id` shown after hovering `id`, if any.
fn tip(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
    tip_id: &'static str,
) -> Option<String> {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(id, cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(tip_id)
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn rows_shown(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    (0..3usize)
        .filter(|ix| present(cx, win, ("cookie", *ix)))
        .count()
}

#[test]
fn the_flags_go_by_the_words_developers_look_for() {
    assert_eq!(HTTP_ONLY_LABEL, "HttpOnly");
    assert_eq!(SECURE_LABEL, "Secure");
    assert!(!HTTP_ONLY_TIP.is_empty() && !SECURE_TIP.is_empty());
}

#[gpui_kit::test]
fn the_jar_view_headings_use_the_dialog_flag_words_and_tips(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    assert_eq!(
        label(cx, win, ("cookies-head-cell", 5usize)),
        HTTP_ONLY_LABEL
    );
    assert_eq!(label(cx, win, ("cookies-head-cell", 6usize)), SECURE_LABEL);
    assert_eq!(
        tip(cx, win, ("cookies-head-cell", 5usize), "kv-head-tip").as_deref(),
        Some(HTTP_ONLY_TIP)
    );
}

#[gpui_kit::test]
fn the_dialog_rows_and_form_use_the_same_flag_words_and_tips(cx: &mut TestAppContext) {
    let win = dialog(cx);
    assert_eq!(
        label(cx, win, ("jar-flags", 0usize)),
        format!("{SECURE_LABEL}, {HTTP_ONLY_LABEL}, This host only")
    );
    assert_eq!(
        tip(cx, win, ("jar-flag-secure", 0usize), "jar-flag-tip").as_deref(),
        Some(SECURE_TIP)
    );
    cx.update_window(win, |_, w, cx| w.click("jar-add", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(label(cx, win, "jar-form-secure"), SECURE_LABEL);
    assert_eq!(label(cx, win, "jar-form-http-only"), HTTP_ONLY_LABEL);
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn set_rows_shown(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    (0..3usize)
        .filter(|ix| present(cx, win, ("set-cookie", *ix)))
        .count()
}

#[gpui_kit::test]
fn the_jar_view_masks_with_dots_and_its_copies_follow_the_eye(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    assert_eq!(label(cx, win, ("cookie-value", 0usize)), NAME_MASK);
    click(cx, win, ("cookie-copy", 0usize));
    assert_eq!(clipboard(cx).as_deref(), Some(NAME_MASK));
    click(cx, win, "cookies-copy");
    assert_eq!(
        clipboard(cx),
        Some(format!(
            "sid={NAME_MASK}; Domain=127.0.0.1; Path=/; HttpOnly; Secure"
        ))
    );
    click(cx, win, ("cookie-reveal", 0usize));
    click(cx, win, ("cookie-copy", 0usize));
    assert_eq!(clipboard(cx).as_deref(), Some("tok3n9"));
}

#[gpui_kit::test]
fn copy_all_with_every_value_hidden_puts_no_secret_on_the_clipboard(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    click(cx, win, "cookies-copy");
    let jar = clipboard(cx).unwrap_or_default();
    assert!(jar.contains(NAME_MASK) && !jar.contains("tok3n9"), "{jar}");
    click(cx, win, "cookies-view-response");
    click(cx, win, "cookies-copy");
    let response = clipboard(cx).unwrap_or_default();
    assert!(
        response.contains(NAME_MASK) && !response.contains("tok3n9"),
        "{response}"
    );
}

#[gpui_kit::test]
fn the_jar_filter_matches_a_value_only_once_revealed(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    assert_eq!(rows_shown(cx, win), 1);
    for query in ["redacted", "•", "tok3n"] {
        replace_input(cx, win, None, "cookies-filter", query);
        frame(cx, win);
        assert_eq!(rows_shown(cx, win), 0, "{query:?} matched a hidden value");
    }
    // The path is matched like the name and domain.
    replace_input(cx, win, None, "cookies-filter", "/");
    frame(cx, win);
    assert_eq!(rows_shown(cx, win), 1, "the path did not match");
    replace_input(cx, win, None, "cookies-filter", "");
    frame(cx, win);
    click(cx, win, ("cookie-reveal", 0usize));
    replace_input(cx, win, None, "cookies-filter", "tok3n");
    frame(cx, win);
    assert_eq!(rows_shown(cx, win), 1, "a revealed value did not match");
}

#[gpui_kit::test]
fn the_response_filter_matches_a_value_only_once_revealed(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    click(cx, win, "cookies-view-response");
    assert_eq!(set_rows_shown(cx, win), 1);
    assert_eq!(label(cx, win, ("set-cookie-value", 0usize)), NAME_MASK);
    replace_input(cx, win, None, "cookies-filter", "tok3n");
    frame(cx, win);
    assert_eq!(set_rows_shown(cx, win), 0, "a hidden value matched");
    replace_input(cx, win, None, "cookies-filter", "");
    frame(cx, win);
    click(cx, win, ("set-cookie-reveal", 0usize));
    replace_input(cx, win, None, "cookies-filter", "tok3n");
    frame(cx, win);
    assert_eq!(set_rows_shown(cx, win), 1, "a revealed value did not match");
}

#[gpui_kit::test]
fn manage_cookies_opens_the_dialog_filtered_to_the_host(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    // A cookie for another host shows that the dialog's list is filtered, not just small.
    let exec = cx.update(|cx| shared(cx).unwrap());
    exec.cookies()
        .insert(CookieInfo::new("other.test", "elsewhere", "1"))
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("cookies-manage", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    let search = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("jar-search").value().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(search.as_deref(), Some("127.0.0.1"));
    let names = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (0..3usize)
                .filter(|ix| w.try_find(("jar-cookie", *ix)).is_some())
                .count()
        })
        .unwrap();
    assert_eq!(names, 1, "only the response host's cookie is listed");
}

#[test]
fn the_host_search_keeps_every_cookie_the_jar_view_lists() {
    // A domain-wide cookie for the parent domain applies to the host, so the search is the
    // shortest listed domain, which every other listed domain contains.
    assert_eq!(
        host_search("api.example.test", &["api.example.test", ".example.test"]),
        "example.test"
    );
    assert_eq!(host_search("api.example.test", &[]), "api.example.test");
}
