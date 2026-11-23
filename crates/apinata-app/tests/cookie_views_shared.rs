//! The response pane's Jar view and the Cookies dialog describe a cookie alike: the same flag
//! words and tooltips, the same dots for a hidden value, and a way from the view into the
//! dialog filtered to the response's host.

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

#[gpui_kit::test]
fn the_jar_view_masks_with_dots_and_copies_the_real_value(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    let value = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("cookie-value", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(value.as_deref(), Some(NAME_MASK));
    cx.update_window(win, |_, w, cx| w.click(("cookie-copy", 0usize), cx))
        .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("tok3n9"));
    cx.update_window(win, |_, w, cx| w.click("cookies-copy", cx))
        .unwrap();
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("sid=tok3n9; Domain=127.0.0.1; Path=/; HttpOnly; Secure")
    );
}

#[gpui_kit::test]
fn the_jar_filter_reads_the_real_value_never_the_mask(cx: &mut TestAppContext) {
    let (_s, win, _view) = jar_view(cx);
    assert_eq!(rows_shown(cx, win), 1);
    for mask in ["redacted", "•"] {
        replace_input(cx, win, None, "cookies-filter", mask);
        frame(cx, win);
        assert_eq!(rows_shown(cx, win), 0, "{mask:?} matched the mask");
    }
    replace_input(cx, win, None, "cookies-filter", "tok3n");
    frame(cx, win);
    assert_eq!(rows_shown(cx, win), 1);
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
