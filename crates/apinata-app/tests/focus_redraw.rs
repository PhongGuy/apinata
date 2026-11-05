//! Focus moved while a frame draws is drawn in the next one: a flipped eye keeps its ring, a
//! closed tab or collection hands a live focus on, and keys reach it without another redraw.

use crate::support;

use apinata_app::kv_table::KvTable;
use apinata_app::response_pane::ResponseTab;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, ParentElement as _, TestAppContext,
};
use support::*;

const SECRET: &str = "s3cr3t-value-123";

// Whether the last drawn frame holds `id`, read without drawing another.
fn drawn(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    let id = id.into();
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

// The label `id` carries in the last drawn frame.
fn drawn_label(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, _| {
        w.try_find(id)
            .and_then(|e| e.label().map(str::to_owned))
            .unwrap_or_default()
    })
    .unwrap()
}

fn activate(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
}

// Presses `key` on the focused eye as the platform does and lets the frames it asked for draw.
fn press_drawn(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    platform_press(cx, win, key);
    next_frame(cx, win);
    next_frame(cx, win);
}

// Flips the eye `eye` by `key`, then back by Space, checking after each that the drawn frame
// rings the same eye and shows the other face.
fn eye_round_trip(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    eye: gpui_kit::ElementId,
    faces: (gpui_kit::ElementId, gpui_kit::ElementId),
    key: &str,
) {
    activate(cx, win);
    tab_to(cx, win, eye.clone());
    press_drawn(cx, win, key);
    assert!(drawn(cx, win, faces.1.clone()), "{key} flips {eye} to hide");
    assert!(
        ring_on(cx, win, eye.clone()),
        "{eye} keeps its ring after {key}"
    );
    press_drawn(cx, win, "space");
    assert!(drawn(cx, win, faces.0), "a second Space flips {eye} back");
    assert!(
        ring_on(cx, win, eye.clone()),
        "{eye} keeps its ring after Space"
    );
}

#[gpui_kit::test]
fn enter_on_a_response_header_eye_keeps_its_ring(cx: &mut TestAppContext) {
    use crate::response_redaction::{row_of, shown};
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "x-echo");
    eye_round_trip(
        cx,
        fx.win,
        ("header-eye", row).into(),
        (("header-reveal", row).into(), ("header-hide", row).into()),
        "enter",
    );
}

#[gpui_kit::test]
fn space_on_a_response_header_eye_shows_the_value(cx: &mut TestAppContext) {
    use crate::response_redaction::{row_of, shown};
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "x-echo");
    activate(cx, fx.win);
    tab_to(cx, fx.win, ("header-eye", row));
    press_drawn(cx, fx.win, "space");
    assert_eq!(drawn_label(cx, fx.win, ("header-value", row)), SECRET);
    assert!(ring_on(cx, fx.win, ("header-eye", row)));
}

#[gpui_kit::test]
fn enter_on_a_cookie_eye_keeps_its_ring(cx: &mut TestAppContext) {
    use crate::response_redaction::{row_of, shown};
    let fx = shown(cx, ResponseTab::Cookies);
    let row = row_of(cx, &fx, "cookie-name", "sess");
    eye_round_trip(
        cx,
        fx.win,
        ("cookie-eye", row).into(),
        (("cookie-reveal", row).into(), ("cookie-hide", row).into()),
        "enter",
    );
}

#[gpui_kit::test]
fn space_on_the_response_body_eye_keeps_its_ring(cx: &mut TestAppContext) {
    let fx = crate::response_redaction::shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    eye_round_trip(
        cx,
        fx.win,
        "body-eye".into(),
        ("body-reveal".into(), "body-hide".into()),
        "space",
    );
}

#[gpui_kit::test]
fn enter_on_the_hop_line_eye_keeps_its_ring(cx: &mut TestAppContext) {
    let fx = crate::timeline_redaction::shown(cx);
    eye_round_trip(
        cx,
        fx.win,
        "hop-line-eye".into(),
        ("hop-line-reveal".into(), "hop-line-hide".into()),
        "enter",
    );
    press_drawn(cx, fx.win, "space");
    assert!(drawn_label(cx, fx.win, "hop-line").contains(SECRET));
}

#[gpui_kit::test]
fn enter_on_a_hop_header_eye_keeps_its_ring(cx: &mut TestAppContext) {
    use crate::timeline_redaction::{row_of, shown};
    let fx = shown(cx);
    let row = row_of(cx, &fx, "hop-request-header", "x-auth");
    eye_round_trip(
        cx,
        fx.win,
        ("hop-request-header-eye", row).into(),
        (
            ("hop-request-header-reveal", row).into(),
            ("hop-request-header-hide", row).into(),
        ),
        "enter",
    );
}

#[gpui_kit::test]
fn a_credential_named_header_eye_is_keyboard_reachable(cx: &mut TestAppContext) {
    use crate::timeline_redaction::{row_of, shown_named};
    let fx = shown_named(cx);
    let row = row_of(cx, &fx, "hop-request-header", "x-api-key");
    eye_round_trip(
        cx,
        fx.win,
        ("hop-request-header-eye", row).into(),
        (
            ("hop-request-header-reveal", row).into(),
            ("hop-request-header-hide", row).into(),
        ),
        "enter",
    );
}

fn two_requests_started(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("b.yml", &request("b", "http://127.0.0.1:9/b")),
    ]);
    let s = started(cx, &col);
    activate(cx, s.win);
    (col, s)
}

// Opens `a.yml` and puts focus in its tab, as a click into the page would.
fn focus_in_tab(cx: &mut TestAppContext, s: &Started) -> apinata_app::tabs::TabId {
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let focus = area
        .read_with(cx, |a, cx| a.active_focus(cx))
        .expect("an active tab");
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    s.ws.read_with(cx, |w, _| w.tabs().entries()[0].id)
}

#[gpui_kit::test]
fn a_tab_closed_under_its_focus_rings_the_tree(cx: &mut TestAppContext) {
    let (_col, s) = two_requests_started(cx);
    let id = focus_in_tab(cx, &s);
    // Closed from outside the tab area, as a deleted file or an external close does.
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(id);
        })
    });
    cx.run_until_parked();
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    platform_press(cx, s.win, "enter");
    assert_eq!(
        tab_names(cx, &s.ws),
        ["a.yml"],
        "Enter in the tree opens a request"
    );
}

// A guard: the close key moves focus in its own handler, which schedules the frame.
#[gpui_kit::test]
fn closing_the_last_tab_by_key_rings_the_tree(cx: &mut TestAppContext) {
    let (_col, s) = two_requests_started(cx);
    focus_in_tab(cx, &s);
    platform_press(cx, s.win, &key("w"));
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    platform_press(cx, s.win, "enter");
    assert_eq!(
        tab_names(cx, &s.ws),
        ["a.yml"],
        "Enter in the tree opens a request"
    );
}

// With no collection left, the shell takes the focus its closed tab lost.
#[gpui_kit::test]
fn closing_the_last_collection_under_tab_focus_draws_shell_focus(cx: &mut TestAppContext) {
    let (_col, s) = two_requests_started(cx);
    focus_in_tab(cx, &s);
    s.ws.update(cx, |w, cx| w.close(&s.root, cx));
    cx.run_until_parked();
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(
        s.ws.read_with(cx, |w, cx| w.shows_empty_state(cx)),
        "no collection is open"
    );
    let (focused, drawn_path) = cx
        .update_window(s.win, |_, w, cx| {
            let focused = s.shell.read(cx).focus_handle(cx).is_focused(w);
            (focused, format!("{:?}", w.context_stack()))
        })
        .unwrap();
    assert!(focused, "the shell holds focus");
    assert_eq!(
        drawn_path, "[Root, Shell]",
        "the drawn frame routes keys to the shell"
    );
    platform_press(cx, s.win, &key("shift-p"));
    next_frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(open, "the palette key opens the palette");
}

// A probe: the checkbox takes focus in render, before the frame paints.
#[gpui_kit::test]
fn a_moved_rows_checkbox_draws_its_focus(cx: &mut TestAppContext) {
    use crate::kv_table_move::{abc, focus_name, host};
    let (win, h) = host(cx, abc());
    activate(cx, win);
    focus_name(cx, win, 0);
    cx.update_window(win, |_, w, cx| w.focus_prev(cx)).unwrap();
    frame(cx, win);
    assert!(ring_on(cx, win, ("kv-enabled", 0usize)));
    platform_press(cx, win, "alt-down");
    next_frame(cx, win);
    next_frame(cx, win);
    assert!(
        ring_on(cx, win, ("kv-enabled", 1usize)),
        "the moved box shows its ring"
    );
    platform_press(cx, win, "space");
    let last = h.read_with(cx, |h, _| h.events.last().unwrap().clone());
    let seen: Vec<_> = last.iter().map(|r| (r.name.as_str(), r.enabled)).collect();
    assert_eq!(
        seen,
        [("b", true), ("a", false), ("c", true)],
        "Space toggles it"
    );
}

// A probe: locking a table in bulk mode moves focus to the table in render.
#[gpui_kit::test]
fn locking_a_bulk_table_draws_focus_on_the_table(cx: &mut TestAppContext) {
    use crate::kv_table_move::{abc, host};
    let (win, h) = host(cx, abc());
    activate(cx, win);
    let t: Entity<KvTable> = h.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    let bulk = t
        .read_with(cx, |t, _| t.bulk_editor().cloned())
        .expect("bulk mode");
    cx.update_window(win, |_, w, cx| bulk.focus_handle(cx).focus(w, cx))
        .unwrap();
    frame(cx, win);
    t.update(cx, |t, cx| t.set_locked(true, cx));
    cx.run_until_parked();
    next_frame(cx, win);
    next_frame(cx, win);
    let (focused, keyed) = cx
        .update_window(win, |_, w, cx| {
            let focused = t.read(cx).focus_handle(cx).is_focused(w);
            let keyed = w
                .context_stack()
                .iter()
                .any(|c| c.contains(apinata_app::kv_grid::STRUCTURED_ROWS_CONTEXT));
            (focused, keyed)
        })
        .unwrap();
    assert!(focused, "the table holds focus");
    assert!(keyed, "the drawn frame routes keys to the table");
}

// A probe: a finished preview puts focus on the dialog in render.
#[gpui_kit::test]
fn a_finished_import_preview_draws_focus_on_the_dialog(cx: &mut TestAppContext) {
    use crate::import_update::{V2, phase, preview, rig, serve};
    use apinata_app::import_link::Phase;
    let r = rig(cx);
    r.open_v1(cx);
    activate(cx, r.win);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    next_frame(cx, r.win);
    next_frame(cx, r.win);
    assert!(
        ring_on(cx, r.win, "import-link-dialog"),
        "the dialog holds focus"
    );
    platform_press(cx, r.win, "enter");
    wait_for(cx, "the update", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
}

// Focus lost behind an open dialog goes into the dialog, so its Escape still arrives.
#[gpui_kit::test]
fn a_background_focus_removed_under_a_dialog_lands_in_the_dialog(cx: &mut TestAppContext) {
    let (_col, s) = two_requests_started(cx);
    let id = focus_in_tab(cx, &s);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let focus = area
        .read_with(cx, |a, cx| a.active_focus(cx))
        .expect("an active tab");
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(id);
        })
    });
    cx.run_until_parked();
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    let (open, path, trapped) = cx
        .update_window(s.win, |_, w, cx| {
            let trapped = gpui_kit::base::active_focus_trap(w, cx).is_some();
            (
                w.has_active_dialog(cx),
                format!("{:?}", w.context_stack()),
                trapped,
            )
        })
        .unwrap();
    assert!(open, "the dialog stays open");
    assert!(
        !path.contains("Sidebar"),
        "no background stop holds focus: {path}"
    );
    assert!(trapped, "focus sits inside the dialog: {path}");
    platform_press(cx, s.win, "escape");
    next_frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "Escape closes the dialog");
}

// Focuses the active tab's editor behind the open dialogs and closes that tab, then lets the
// frames it asked for draw.
fn lose_background_focus(cx: &mut TestAppContext, s: &Started, id: apinata_app::tabs::TabId) {
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let focus = area
        .read_with(cx, |a, cx| a.active_focus(cx))
        .expect("an active tab");
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(id);
        })
    });
    cx.run_until_parked();
    next_frame(cx, s.win);
    next_frame(cx, s.win);
}

#[gpui_kit::test]
fn a_background_focus_lost_under_stacked_dialogs_lands_in_the_top_one(cx: &mut TestAppContext) {
    let (_col, s) = two_requests_started(cx);
    let id = focus_in_tab(cx, &s);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.open_dialog(cx, |d, _, _| {
            d.close_button(false)
                .child(gpui_kit::component::button::Button::new("top-ok").label("OK"))
        })
    })
    .unwrap();
    frame(cx, s.win);
    lose_background_focus(cx, &s, id);
    assert!(
        ring_on(cx, s.win, "top-ok"),
        "the top dialog's stop holds focus"
    );
    assert!(
        !ring_on(cx, s.win, "search-input"),
        "the covered dialog does not"
    );
}
