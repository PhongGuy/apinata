use crate::support;

use apinata_app::kv_table::KvRow;
use apinata_app::request_editor::Section;
use apinata_app::url_unit::{Side, UrlParamsUnit};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Focusable};
use gpui_kit::{SharedString, TestAppContext};
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)])
}

fn params(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
) -> Vec<(String, String)> {
    let table = ed
        .read_with(cx, |e, _| e.table(Section::Params).cloned())
        .unwrap();
    table.read_with(cx, |t, cx| {
        t.rows(cx).into_iter().map(|r| (r.name, r.value)).collect()
    })
}

fn pair(n: &str, v: &str) -> (String, String) {
    (n.into(), v.into())
}

#[test]
fn the_unit_locks_the_other_side_while_one_is_in_flight() {
    let mut u = UrlParamsUnit::default();
    assert!(!u.locked(Side::Params, false));
    assert!(u.edit(Side::Url, false));
    assert!(u.edit(Side::Url, true));
    assert!(u.locked(Side::Params, true));
    assert!(!u.locked(Side::Url, true));
    assert!(!u.edit(Side::Params, true));
    assert!(u.locked(Side::Params, true));
    assert!(!u.locked(Side::Params, false));
    assert!(u.edit(Side::Params, false));
    assert!(u.locked(Side::Url, true));
    assert!(!u.edit(Side::Url, true));
}

#[gpui_kit::test]
fn the_editor_shows_the_snapshot_and_url_typing_derives_params(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert_eq!(
        ed.read_with(cx, |e, cx| e.method(cx)),
        Some(SharedString::from("POST"))
    );
    assert_eq!(params(cx, &ed), [pair("a", "1")]);
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("breadcrumb").is_some())
            .unwrap()
    );
    type_at_end(cx, win, None, "url", "&b=2");
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=1&b=2");
    assert_eq!(params(cx, &ed), [pair("a", "1"), pair("b", "2")]);
}

#[gpui_kit::test]
fn the_caret_stays_put_when_the_echo_arrives(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, None, "url", "&v");
    cx.update_window(win, |_, w, cx| {
        w.press("left", cx);
        w.input("Z", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/0?a=1&Zv");
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=1&Zv");
    let caret = ed.read_with(cx, |e, cx| e.url_caret(cx));
    assert_eq!(caret, "http://x/0?a=1&Z".len());
}

#[gpui_kit::test]
fn an_outside_change_reloads_the_url_and_params(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/9?z=1"), cx)
    });
    cx.run_until_parked();
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/9?z=1");
    assert_eq!(params(cx, &ed), [pair("z", "1")]);
}

fn rows_with(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
    value: &str,
) -> Vec<KvRow> {
    let table = ed
        .read_with(cx, |e, _| e.table(Section::Params).cloned())
        .unwrap();
    let mut rows = table.read_with(cx, |t, cx| t.rows(cx));
    rows[0].value = value.into();
    rows
}

#[gpui_kit::test]
fn a_params_edit_is_refused_while_a_url_edit_is_in_flight(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, None, "url", "&b=2");
    assert!(ed.read_with(cx, |e, cx| e.locked(Side::Params, cx)));
    let table = ed
        .read_with(cx, |e, _| e.table(Section::Params).cloned())
        .unwrap();
    assert!(table.read_with(cx, |t, _| t.locked()));
    let rows = rows_with(cx, &ed, "9");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.edit_rows(Section::Params, rows, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=1&b=2");
    assert!(!ed.read_with(cx, |e, cx| e.locked(Side::Params, cx)));
    assert_eq!(params(cx, &ed), [pair("a", "1"), pair("b", "2")]);
    type_at_end(cx, win, Some("params"), ("kv-value", 0usize), "9");
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=19&b=2");
}

#[gpui_kit::test]
fn a_url_edit_is_refused_while_a_params_edit_is_in_flight(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, Some("params"), ("kv-value", 0usize), "9");
    assert!(ed.read_with(cx, |e, cx| e.locked(Side::Url, cx)));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.edit_url("http://x/other", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=19");
    assert!(!ed.read_with(cx, |e, cx| e.locked(Side::Url, cx)));
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/0?a=19");
    type_at_end(cx, win, None, "url", "&c=3");
    cx.run_until_parked();
    assert_eq!(params(cx, &ed), [pair("a", "19"), pair("c", "3")]);
}

#[gpui_kit::test]
fn an_ack_for_an_earlier_keystroke_keeps_the_later_one(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, None, "url", "&c");
    cx.run_until_parked();
    let before = params(cx, &ed);
    cx.update_window(win, |_, w, cx| {
        w.press("left", cx);
        w.input("b", cx)
    })
    .unwrap();
    // The newest snapshot acknowledges "&c" while "b" is still in flight.
    ws.update(cx, |_, cx| cx.notify());
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=1&c");
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/0?a=1&bc");
    assert_eq!(
        ed.read_with(cx, |e, cx| e.url_caret(cx)),
        "http://x/0?a=1&b".len()
    );
    assert_eq!(params(cx, &ed), before);
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=1&bc");
}

#[gpui_kit::test]
fn a_url_query_without_a_params_list_fills_the_params_tab(cx: &mut TestAppContext) {
    let file = "info:\n  name: q\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0?a=1&b=2\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("q.yml", file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "q.yml", false);
    assert_eq!(params(cx, &ed), [pair("a", "1"), pair("b", "2")]);
    assert_eq!(
        std::fs::read_to_string(col.path().join("q.yml")).unwrap(),
        file
    );
    type_at_end(cx, win, Some("params"), ("kv-value", 1usize), "3");
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "q.yml").url, "http://x/0?a=1&b=23");
    assert_eq!(params(cx, &ed), [pair("a", "1"), pair("b", "23")]);
}

#[gpui_kit::test]
fn an_ampersand_typed_in_a_param_survives_typing_in_the_url(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, Some("params"), ("kv-value", 0usize), " & 2");
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0?a=1 %26 2");
    type_at_end(cx, win, None, "url", "x");
    cx.run_until_parked();
    assert_eq!(
        view_of(cx, &ws, &root, "a.yml").url,
        "http://x/0?a=1 %26 2x"
    );
    // Reference: a URL edit derives the table with decode=false (slices/collections/index.js:1165,
    // bruno-common url/index.ts:122), so the escape the table wrote shows as URL text.
    assert_eq!(params(cx, &ed), [pair("a", "1 %26 2x")]);
}

// Reference: a URL edit derives the table with decode=false (slices/collections/index.js:1165,
// bruno-common url/index.ts:122).
#[gpui_kit::test]
fn an_escape_typed_in_the_url_shows_as_typed_in_the_params(cx: &mut TestAppContext) {
    let file =
        "info:\n  name: q\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("q.yml", file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "q.yml", false);
    type_at_end(cx, win, None, "url", "?v=a%26b");
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "q.yml").url, "http://x/0?v=a%26b");
    assert_eq!(params(cx, &ed), [pair("v", "a%26b")]);
}

// Leaving bulk edit drops the focused text editor; Save must still reach the request.
#[gpui_kit::test]
fn save_still_works_after_leaving_bulk_edit(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let table = ed
        .read_with(cx, |e, _| e.table(Section::Params).cloned())
        .unwrap();
    type_at_end(cx, win, None, "url", "&b=2");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    let bulk = table.read_with(cx, |t, cx| {
        t.bulk_editor().expect("bulk on").read(cx).focus_handle(cx)
    });
    assert!(
        cx.update_window(win, |_, w, _| bulk.is_focused(w)).unwrap(),
        "turning bulk edit on focuses its text"
    );
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    let focus = ed.read_with(cx, |e, cx| e.focus_handle(cx));
    assert!(
        cx.update_window(win, |_, w, cx| focus.contains_focused(w, cx))
            .unwrap(),
        "focus stays inside the request editor"
    );
    cx.update_window(win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::request_editor::SaveRequest), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let saved = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(saved.contains("http://x/0?a=1&b=2"), "{saved}");
}

#[test]
fn the_method_select_offers_nine_verbs_and_appends_the_files_own() {
    use apinata_app::request_editor::method_items;
    let names =
        |m: &str| -> Vec<String> { method_items(m).iter().map(|s| s.to_string()).collect() };
    let nine = [
        "GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD", "TRACE", "QUERY",
    ];
    assert_eq!(names("GET"), nine);
    assert_eq!(names("QUERY"), nine);
    let mut with = nine.map(String::from).to_vec();
    with.push("PROPFIND".into());
    assert_eq!(names("PROPFIND"), with);
}

// Each verb in the method picker carries the colour the sidebar gives it.
#[gpui_kit::test]
fn the_method_picker_colours_each_verb_like_the_sidebar(cx: &mut TestAppContext) {
    use apinata_app::request_editor::method_rows;
    use apinata_app::ui::method_class;
    use gpui_kit::component::select::SelectItem as _;
    init(cx);
    cx.update(|cx| {
        let rows = method_rows("PROPFIND");
        assert_eq!(rows.len(), 10);
        for row in rows {
            let verb = row.title();
            assert_eq!(row.color(cx), Some(method_class(&verb).color(cx)), "{verb}");
        }
    });
}

fn type_custom(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
    win: gpui_kit::AnyWindowHandle,
    text: &str,
) {
    let input = ed.read_with(cx, |e, _| e.custom_method_input().clone());
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.set_value(text.to_string(), w, cx));
    })
    .unwrap();
}

fn commit(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
    win: gpui_kit::AnyWindowHandle,
) {
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.commit_custom_method(w, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_custom_method_commits_uppercased_and_a_bad_one_shows_an_error(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_custom(cx, &ed, win, "GE T");
    commit(cx, &ed, win);
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "POST");
    assert_eq!(
        ed.read_with(cx, |e, cx| e.method(cx)),
        Some(SharedString::from("POST"))
    );
    type_custom(cx, &ed, win, "propfind");
    commit(cx, &ed, win);
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_none()));
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "PROPFIND");
    assert_eq!(
        ed.read_with(cx, |e, cx| e.method(cx)),
        Some(SharedString::from("PROPFIND"))
    );
    let shown = ed.read_with(cx, |e, cx| e.custom_method_input().read(cx).value());
    assert_eq!(shown.as_ref(), "", "the input follows the draft");
}

#[gpui_kit::test]
fn enter_and_blur_commit_and_a_method_change_clears_the_error(cx: &mut TestAppContext) {
    use gpui_kit::component::input::InputEvent;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let input = ed.read_with(cx, |e, _| e.custom_method_input().clone());
    type_custom(cx, &ed, win, "bad verb");
    input.update(cx, |_, cx| cx.emit(InputEvent::Blur));
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_method("GET", w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_none()));
    type_custom(cx, &ed, win, "mkcol");
    input.update(cx, |_, cx| {
        cx.emit(InputEvent::PressEnter {
            secondary: false,
            shift: false,
        })
    });
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "MKCOL");
}

#[gpui_kit::test]
fn typing_in_the_custom_input_clears_its_error(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_custom(cx, &ed, win, "bad verb");
    commit(cx, &ed, win);
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    type_custom(cx, &ed, win, "bad verb2");
    // `set_value` is silent; typing emits Change.
    let input = ed.read_with(cx, |e, _| e.custom_method_input().clone());
    input.update(cx, |_, cx| {
        cx.emit(gpui_kit::component::input::InputEvent::Change)
    });
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_none()));
}

#[gpui_kit::test]
fn an_empty_commit_clears_the_error_and_keeps_the_method(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_custom(cx, &ed, win, "bad verb");
    commit(cx, &ed, win);
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    type_custom(cx, &ed, win, "");
    commit(cx, &ed, win);
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_none()));
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "POST");
}

#[gpui_kit::test]
fn picking_from_the_select_empties_the_custom_input(cx: &mut TestAppContext) {
    use gpui_kit::component::select::SelectEvent;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_custom(cx, &ed, win, "propfind");
    let select = ed.read_with(cx, |e, _| e.method_select().clone());
    select.update(cx, |_, cx| {
        cx.emit(SelectEvent::Confirm(Some(SharedString::from("GET"))))
    });
    cx.run_until_parked();
    let shown = ed.read_with(cx, |e, cx| e.custom_method_input().read(cx).value());
    assert_eq!(shown.as_ref(), "");
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "GET");
}

#[gpui_kit::test]
fn the_custom_placeholder_fits_the_input_at_the_default_width(cx: &mut TestAppContext) {
    use apinata_app::request_editor::CUSTOM_METHOD_PLACEHOLDER;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        let theme = gpui_kit::component::ActiveTheme::theme(cx);
        let font = gpui_kit::font(theme.mono_font_family.clone());
        let size = gpui_kit::rems(0.875).to_pixels(w.rem_size());
        let mut run = w.text_style().to_run(CUSTOM_METHOD_PLACEHOLDER.len());
        run.font = font;
        let text = w
            .text_system()
            .shape_line(CUSTOM_METHOD_PLACEHOLDER.into(), size, &[run], None)
            .width;
        // Allowance: 2 rem for the input's horizontal padding and border.
        let room = apinata_app::style::CUSTOM_METHOD.to_pixels(w.rem_size())
            - gpui_kit::rems(2.).to_pixels(w.rem_size());
        assert!(text <= room, "placeholder {text:?} in {room:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_on_an_invalid_custom_method_keeps_the_error_visible(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let focus = ed.read_with(cx, |e, cx| e.custom_method_focus(cx));
    cx.update_window(win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    cx.run_until_parked();
    // Real keystrokes, so the error must outlive every event Enter itself produces.
    cx.simulate_keystrokes(win, "g e space t enter");
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("custom-method-error").is_some())
            .unwrap()
    );
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "POST");
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
}

/// While a params edit is in flight the URL input is locked: read-only in every theme, so it
/// still takes focus; the high-contrast themes keep full-strength text on the muted fill.
#[gpui_kit::test]
fn a_locked_url_is_read_only_in_every_theme(cx: &mut TestAppContext) {
    for (theme, high_contrast) in [
        (apinata_app::theme::HC_DARK, true),
        (apinata_app::theme::HC_LIGHT, true),
        (apinata_app::theme::DARK, false),
        (apinata_app::theme::LIGHT, false),
    ] {
        let col = tree();
        let (_state, ws, root) = open_dir(cx, &col);
        let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
        use_theme(cx, theme);
        type_at_end(cx, win, Some("params"), ("kv-value", 0usize), "9");
        assert!(ed.read_with(cx, |e, cx| e.locked(Side::Url, cx)));
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let url = ed.read_with(cx, |e, _| e.url_input().clone());
        let (readonly, disabled) = url.read_with(cx, |i, _| {
            let p = i.presentation();
            (p.is_readonly(), p.is_disabled())
        });
        assert_eq!((readonly, disabled), (true, false), "{theme}");
        if high_contrast {
            let (text, fill) = cx.update(|cx| {
                use gpui_kit::component::ActiveTheme;
                (cx.theme().foreground, cx.theme().muted)
            });
            assert!(contrast(text, fill) >= 7.0, "{theme}");
        }
    }
}
