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
    let (s, _col) = jar(cx, vec![CookieInfo::new("a.test", "session", "v")]);
    assert_eq!(label(cx, s.win, ("jar-value", 0usize)), NAME_MASK);
    assert_eq!(label(cx, s.win, ("jar-flags", 0usize)), "This host only");
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
