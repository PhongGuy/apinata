//! The cookie jar's add and edit form as drawn: each error under its own field in plain words,
//! an empty value accepted, the title naming the form, Save and Cancel last, and saved rows
//! worded and masked as the Timeline shows them.

use crate::support;

use apinata_app::cookie_jar::open_cookie_jar;
use apinata_app::executor::{SharedExecutor, shared};
use apinata_app::response_pane::NAME_MASK;
use apinata_core::{CookieInfo, ExecOptions, Executor};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Bounds, ElementId, Pixels, TestAppContext};
use std::sync::Arc;
use support::*;

// The started app with the jar open over `cookies`.
fn jar(cx: &mut TestAppContext, cookies: Vec<CookieInfo>) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    for c in cookies {
        exec.cookies().insert(c).unwrap();
    }
    cx.update(|cx| cx.set_global(SharedExecutor(exec)));
    cx.update_window(s.win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, s.win);
    (s, col)
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

fn bounds(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

// The error drawn for `field`: below its input, starting at the input's left edge, and above
// the next row.
fn error_under(cx: &mut TestAppContext, win: AnyWindowHandle, field: &str, next: &str) -> String {
    let input = bounds(cx, win, field.to_owned());
    let error = bounds(cx, win, format!("{field}-error"));
    let below = bounds(cx, win, next.to_owned());
    assert!(
        error.top() >= input.bottom(),
        "{field}: {error:?} under {input:?}"
    );
    assert!(
        error.bottom() <= below.top(),
        "{field}: {error:?} above {below:?}"
    );
    assert!(
        (error.left() - input.left()).abs() < gpui_kit::px(1.),
        "{field}: {error:?} lines up with {input:?}"
    );
    label(cx, win, format!("{field}-error"))
}

#[gpui_kit::test]
fn each_error_sits_under_its_own_field_in_plain_words(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx, Vec::new());
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", "a b");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-expires", "soon");
    click(cx, s.win, "jar-form-save");

    let domain = error_under(cx, s.win, "jar-form-domain", "jar-form-path");
    assert!(domain.starts_with("Invalid cookie domain"), "{domain}");
    let expires = error_under(cx, s.win, "jar-form-expires", "jar-form-secure");
    assert_eq!(expires, "Enter a date and time like 2027-01-01 12:00");
    for id in ["jar-form-path", "jar-form-name", "jar-form-value"] {
        assert!(!present(cx, s.win, format!("{id}-error")), "{id}");
    }
    assert!(
        !present(cx, s.win, "jar-form-error"),
        "nothing is left for a list at the bottom"
    );
    let hint = label(cx, s.win, "jar-form-expires-hint");
    assert!(!hint.contains("RFC"), "{hint}");
}

#[gpui_kit::test]
fn an_empty_value_saves_the_cookie(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx, Vec::new());
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", "a.test");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    click(cx, s.win, "jar-form-save");
    assert!(!present(cx, s.win, "jar-form"), "the form closes on save");
    let listed = cx.update(|cx| shared(cx)).unwrap().cookies().list();
    assert_eq!(
        listed
            .iter()
            .map(|c| (c.name.as_str(), c.value.as_str()))
            .collect::<Vec<_>>(),
        [("sid", "")]
    );
}

#[gpui_kit::test]
fn a_saved_row_says_this_host_only_and_masks_as_the_timeline_does(cx: &mut TestAppContext) {
    let mut flagged = CookieInfo::new("b.test", "all", "v");
    flagged.secure = true;
    flagged.http_only = true;
    let (s, _col) = jar(cx, vec![CookieInfo::new("a.test", "session", "v"), flagged]);
    assert_eq!(label(cx, s.win, ("jar-value", 0usize)), NAME_MASK);
    assert_eq!(label(cx, s.win, ("jar-flags", 0usize)), "This host only");
    // Every flag reads as plain words in one style, and the form's checkboxes match them.
    assert_eq!(
        label(cx, s.win, ("jar-flags", 1usize)),
        "HTTPS only, No script access, This host only"
    );
    click(cx, s.win, "jar-add");
    assert_eq!(label(cx, s.win, "jar-form-secure"), "HTTPS only");
    assert_eq!(label(cx, s.win, "jar-form-http-only"), "No script access");
}

#[gpui_kit::test]
fn an_expiry_reopens_as_it_was_entered(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx, Vec::new());
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", "a.test");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-expires", "2099-01-01 12:00");
    click(cx, s.win, "jar-form-save");
    click(cx, s.win, ("jar-edit", 0usize));
    assert_eq!(
        drawn_text(cx, s.win, "jar-form-expires"),
        "2099-01-01 12:00"
    );
}

#[gpui_kit::test]
fn the_title_names_the_form_and_save_and_cancel_come_last(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx, vec![CookieInfo::new("a.test", "k", "v")]);
    assert_eq!(label(cx, s.win, "jar-title"), "Cookies");

    click(cx, s.win, "jar-add");
    assert_eq!(label(cx, s.win, "jar-title"), "Add cookie");
    // The form stands alone, so nothing from the list sits below its buttons.
    for id in [
        ElementId::from("jar-search"),
        ("jar-cookie", 0usize).into(),
        "jar-footer".into(),
    ] {
        assert!(!present(cx, s.win, id.clone()), "{id:?}");
    }
    let save = bounds(cx, s.win, "jar-form-save");
    let cancel = bounds(cx, s.win, "jar-form-cancel");
    for id in ["jar-form-expires", "jar-form-secure", "jar-form-http-only"] {
        let above = bounds(cx, s.win, id);
        assert!(
            save.top() >= above.bottom(),
            "{id}: {above:?} over {save:?}"
        );
        assert!(cancel.top() >= above.bottom(), "{id}");
    }

    click(cx, s.win, "jar-form-cancel");
    assert_eq!(label(cx, s.win, "jar-title"), "Cookies");
    click(cx, s.win, ("jar-edit", 0usize));
    assert_eq!(label(cx, s.win, "jar-title"), "Edit cookie");
}

// A jar long enough to scroll in a short window, scrolled well down its list.
fn scrolled_jar(cx: &mut TestAppContext) -> (Started, tempfile::TempDir) {
    let many = (0..40)
        .map(|i| CookieInfo::new("a.test", format!("c{i:02}"), "v"))
        .collect();
    let (s, col) = jar(cx, many);
    cx.simulate_window_resize(
        s.win,
        gpui_kit::size(gpui_kit::px(900.), gpui_kit::px(400.)),
    );
    frame(cx, s.win);
    let top = bounds(cx, s.win, ("jar-cookie", 0usize));
    cx.update_window(s.win, |_, w, cx| {
        w.scroll(
            "cookie-jar-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-600.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
    assert!(
        bounds(cx, s.win, ("jar-cookie", 0usize)).top() < top.top(),
        "the list scrolled"
    );
    (s, col)
}

// The first row past the top third of the scrolled list that shows whole inside it.
fn row_in_view(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    let view = bounds(cx, win, "cookie-jar-scroll");
    (0..40)
        .find(|&ix| {
            let row = bounds(cx, win, ("jar-cookie", ix));
            row.top() >= view.top() + view.size.height / 3. && row.bottom() <= view.bottom()
        })
        .expect("a row in view")
}

// Edits the row in view, closes the form with `close`, and checks the row has not moved.
fn closing_the_form_keeps_the_row_in_place(cx: &mut TestAppContext, close: &'static str) {
    let (s, _col) = scrolled_jar(cx);
    let ix = row_in_view(cx, s.win);
    let before = bounds(cx, s.win, ("jar-cookie", ix));
    click(cx, s.win, ("jar-edit", ix));
    assert!(present(cx, s.win, "jar-form"));
    click(cx, s.win, close);
    assert!(!present(cx, s.win, "jar-form"));
    assert_eq!(bounds(cx, s.win, ("jar-cookie", ix)), before, "{close}");
}

#[gpui_kit::test]
fn cancelling_an_edit_keeps_the_list_where_it_was(cx: &mut TestAppContext) {
    closing_the_form_keeps_the_row_in_place(cx, "jar-form-cancel");
}

#[gpui_kit::test]
fn saving_an_edit_keeps_the_list_where_it_was(cx: &mut TestAppContext) {
    closing_the_form_keeps_the_row_in_place(cx, "jar-form-save");
}

#[gpui_kit::test]
fn empty_domain_and_name_say_what_to_enter(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx, Vec::new());
    click(cx, s.win, "jar-add");
    click(cx, s.win, "jar-form-save");
    assert_eq!(
        error_under(cx, s.win, "jar-form-domain", "jar-form-path"),
        "Enter a domain"
    );
    assert_eq!(
        error_under(cx, s.win, "jar-form-name", "jar-form-value"),
        "Enter a name"
    );
}

#[gpui_kit::test]
fn delete_stays_clear_of_the_scrollbar(cx: &mut TestAppContext) {
    let (s, _col) = scrolled_jar(cx);
    let ix = row_in_view(cx, s.win);
    let list = bounds(cx, s.win, "cookie-jar-scroll");
    let delete = bounds(cx, s.win, ("jar-delete", ix));
    assert!(
        delete.right() <= list.right() - apinata_app::style::SCROLLBAR_GUTTER,
        "{delete:?} under the scrollbar of {list:?}"
    );
}

#[gpui_kit::test]
fn row_columns_line_up_whatever_their_text(cx: &mut TestAppContext) {
    let mut long = CookieInfo::new("a.test", "a_much_longer_name", "");
    long.path = "/deep/path".into();
    long.expires = Some(std::time::SystemTime::now() + std::time::Duration::from_secs(86_400));
    let (s, _col) = jar(cx, vec![CookieInfo::new("a.test", "s", "v"), long]);
    for id in ["jar-value", "jar-flags"] {
        let lefts: Vec<_> = (0..2usize)
            .map(|ix| bounds(cx, s.win, (id, ix)).left())
            .collect();
        assert!(
            (lefts[0] - lefts[1]).abs() < gpui_kit::px(1.),
            "{id}: {lefts:?}"
        );
    }
}

#[gpui_kit::test]
fn search_and_add_stay_above_the_scrolled_list(cx: &mut TestAppContext) {
    let (s, _col) = scrolled_jar(cx);
    let title = bounds(cx, s.win, "jar-title");
    let list = bounds(cx, s.win, "cookie-jar-scroll");
    for id in ["jar-add", "jar-search"] {
        let fixed = bounds(cx, s.win, id);
        assert!(
            fixed.top() >= title.bottom(),
            "{id}: {fixed:?} under {title:?}"
        );
        assert!(
            fixed.bottom() <= list.top(),
            "{id}: {fixed:?} above {list:?}"
        );
    }
}

#[gpui_kit::test]
fn path_and_expiry_sit_at_the_top_of_wrapped_flags(cx: &mut TestAppContext) {
    let mut flagged = CookieInfo::new("a.test", "sid", "v");
    flagged.secure = true;
    flagged.http_only = true;
    let (s, _col) = jar(cx, vec![flagged]);
    cx.simulate_window_resize(
        s.win,
        gpui_kit::size(gpui_kit::px(560.), gpui_kit::px(600.)),
    );
    frame(cx, s.win);
    let flags = bounds(cx, s.win, ("jar-flags", 0usize));
    let path = bounds(cx, s.win, ("jar-path", 0usize));
    assert!(
        flags.size.height > path.size.height * 1.5,
        "the flags wrap: {flags:?} beside {path:?}"
    );
    for id in ["jar-path", "jar-expiry"] {
        let column = bounds(cx, s.win, (id, 0usize));
        assert!(
            (column.top() - flags.top()).abs() < gpui_kit::px(1.),
            "{id}: {column:?} beside {flags:?}"
        );
    }
}
