//! Focus survives what took its control away: a closing dialog hands it back to the opener or,
//! when the opener died behind it, to a sensible neighbour; a vanished sidebar row hands the
//! selection to the nearest row; the Timeline scrolls a focused eye into view. Every check
//! reads the frame the platform would draw next, with keys sent as the platform sends them.

use crate::support;

use std::time::{Duration, UNIX_EPOCH};

use apinata_app::command::Command;
use apinata_app::focus_mode::{self, Modality};
use apinata_app::last_response::{Limits, ResponseStore};
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::tree::TreeState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext};
use support::*;

fn three_requests_started(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("b.yml", &request("b", "http://127.0.0.1:9/b")),
        ("c.yml", &request("c", "http://127.0.0.1:9/c")),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, s.win);
    (col, s)
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn keyboard(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| focus_mode::is_keyboard(cx))
}

// Presses Tab as the platform does until the drawn frame rings `id`, so the modality is the
// one a keyboard user ends up in.
fn key_to(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    for _ in 0..120 {
        if ring_on(cx, win, id.clone()) {
            return;
        }
        platform_press(cx, win, "tab");
        next_frame(cx, win);
    }
    panic!("Tab never reached {id:?}");
}

// Presses `key` as the platform does and draws the frames it asked for.
fn press_drawn(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    platform_press(cx, win, key);
    next_frame(cx, win);
    next_frame(cx, win);
}

// Lets a closing dialog's animation run out, then draws the frames its focus move asked for.
fn after_close(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    next_frame(cx, win);
    next_frame(cx, win);
}

fn tree_of(cx: &mut TestAppContext, s: &Started) -> Entity<TreeState> {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    sidebar.read_with(cx, |v, _| v.tree().clone())
}

fn selected(cx: &mut TestAppContext, s: &Started) -> (Option<usize>, Option<String>) {
    let tree = tree_of(cx, s);
    tree.read_with(cx, |t, _| {
        (
            t.selected_index(),
            t.selected_item().map(|i| i.id.to_string()),
        )
    })
}

fn select_row(cx: &mut TestAppContext, s: &Started, id: gpui_kit::SharedString) -> usize {
    let tree = tree_of(cx, s);
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&id).expect("a row");
        t.set_selected_index(Some(ix), cx);
        ix
    })
}

// Puts focus on the sidebar tree as a keyboard user left it there, and draws it.
fn keyboard_on_tree(cx: &mut TestAppContext, s: &Started) {
    let tree = tree_of(cx, s);
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    frame(cx, s.win);
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
}

// Opens `rel` and puts focus in its URL input as a keyboard user left it there.
fn keyboard_in_tab(cx: &mut TestAppContext, s: &Started, rel: &str) -> apinata_app::tabs::TabId {
    let id = open_tab(cx, &s.ws, &s.root, rel);
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let focus = area.read_with(cx, |a, cx| {
        a.editor(id)
            .expect("a request editor")
            .read(cx)
            .url_focus(cx)
    });
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    frame(cx, s.win);
    assert!(ring_on(cx, s.win, "url"), "the URL input holds focus");
    id
}

// T1 guards: the kit hands focus back to the control that opened the dialog.

#[gpui_kit::test]
fn escape_on_a_keyboard_opened_dialog_rings_its_opener(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    key_to(cx, s.win, "variables-button");
    press_drawn(cx, s.win, "enter");
    assert!(dialog_open(cx, s.win), "Enter opens the overview");
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(keyboard(cx));
    assert!(
        ring_on(cx, s.win, "variables-button"),
        "the opener shows its ring"
    );
}

#[gpui_kit::test]
fn escape_on_runtime_variables_opened_by_key_rings_the_first_opener(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    key_to(cx, s.win, "variables-button");
    press_drawn(cx, s.win, "enter");
    key_to(cx, s.win, "var-overview-edit-runtime");
    press_drawn(cx, s.win, "enter");
    assert!(
        present(cx, s.win, "runtime-vars-add-name"),
        "the runtime variables dialog is open"
    );
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(
        ring_on(cx, s.win, "variables-button"),
        "the opener shows its ring"
    );
}

#[gpui_kit::test]
fn a_dialog_button_close_rings_its_opener(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    keyboard_on_tree(cx, &s);
    select_row(cx, &s, row("a.yml"));
    press_drawn(cx, s.win, "delete");
    assert!(dialog_open(cx, s.win), "Delete asks first");
    key_to(cx, s.win, "confirm-cancel");
    press_drawn(cx, s.win, "enter");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Cancel closes the prompt");
    assert!(s.root.join("a.yml").exists(), "nothing deleted");
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
}

#[gpui_kit::test]
fn an_action_close_rings_its_opener(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    keyboard_on_tree(cx, &s);
    select_row(cx, &s, row("a.yml"));
    press_drawn(cx, s.win, "delete");
    assert!(dialog_open(cx, s.win), "Delete asks first");
    press_drawn(cx, s.win, "enter");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Enter answers the prompt");
    assert!(!s.root.join("a.yml").exists(), "Enter deletes");
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
}

// T2: the opener died behind the dialog.

#[gpui_kit::test]
fn escape_after_the_openers_tab_closed_rings_the_tree(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    let id = keyboard_in_tab(cx, &s, "a.yml");
    press_drawn(cx, s.win, &key("shift-f"));
    assert!(dialog_open(cx, s.win), "the search opens");
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(id);
        })
    });
    cx.run_until_parked();
    next_frame(cx, s.win);
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(keyboard(cx));
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    platform_press(cx, s.win, "enter");
    assert_eq!(
        tab_names(cx, &s.ws).len(),
        1,
        "Enter in the tree opens a request"
    );
}

// As QA 08b-08e: runtime variables opened from a tab's URL input, that tab's file deleted on
// disk behind the dialog while another tab stays open.
#[gpui_kit::test]
fn escape_after_the_openers_file_was_deleted_rings_the_next_tabs_url(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    open_tab(cx, &s.ws, &s.root, "b.yml");
    keyboard_in_tab(cx, &s, "a.yml");
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .dispatch_action(apinata_app::shell::OpenRuntimeVariables);
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(
        present(cx, s.win, "runtime-vars-add-name"),
        "the runtime variables dialog is open"
    );
    delete_on_disk(cx, &s, "a.yml");
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"], "a's tab closed");
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(keyboard(cx));
    assert!(ring_on(cx, s.win, "url"), "b's URL input shows its ring");
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    let url = cx
        .update_window(s.win, |_, w, _| {
            w.try_find("url").and_then(|e| e.value().map(str::to_owned))
        })
        .unwrap()
        .unwrap_or_default();
    assert!(url.contains('x'), "typing reaches it: {url}");
}

// T3: the selected row vanished.

// The selection after a removal sits at the old index clamped to the rows left, or on the row
// just above it when that one shares the removed row's parent.
fn assert_nearest(cx: &mut TestAppContext, s: &Started, old: usize, want: Option<&str>) {
    let (ix, id) = selected(cx, s);
    let rows = tree_of(cx, s).read_with(cx, |t, _| {
        (0..).take_while(|&i| t.entry(i).is_some()).count()
    });
    let ix = ix.unwrap_or_else(|| panic!("a row is selected after the removal"));
    let clamped = old.min(rows - 1);
    assert!(
        ix == clamped || ix + 1 == clamped,
        "selected {ix}, wanted near {clamped}"
    );
    if let Some(want) = want {
        assert_eq!(id.as_deref(), Some(&*row(want)), "the nearest row");
    }
}

#[gpui_kit::test]
fn a_selected_draft_whose_tab_closed_selects_the_nearest_row(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    let (draft, _) = new_draft_at(cx, &s, apinata_core::ItemPath::root());
    frame(cx, s.win);
    let tab = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
    keyboard_on_tree(cx, &s);
    let old = select_row(cx, &s, row(&draft));
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(tab);
        })
    });
    cx.run_until_parked();
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert_nearest(cx, &s, old, None);
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    assert!(keyboard(cx), "the ring is the keyboard one");
    let (at, _) = selected(cx, &s);
    press_drawn(cx, s.win, "down");
    let (moved, _) = selected(cx, &s);
    assert_ne!(moved, at, "Down moves the selection");
}

// Deletes `rel` on disk and reports it as the watcher does.
fn delete_on_disk(cx: &mut TestAppContext, s: &Started, rel: &str) {
    let path = s.root.join(rel);
    std::fs::remove_file(&path).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Reload(vec![path]), cx));
    cx.run_until_parked();
    next_frame(cx, s.win);
    next_frame(cx, s.win);
}

#[gpui_kit::test]
fn a_selected_file_deleted_on_disk_selects_the_next_row(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    keyboard_on_tree(cx, &s);
    let old = select_row(cx, &s, row("b.yml"));
    frame(cx, s.win);
    delete_on_disk(cx, &s, "b.yml");
    assert_nearest(cx, &s, old, Some("c.yml"));
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    assert!(keyboard(cx), "the ring is the keyboard one");
    press_drawn(cx, s.win, "up");
    assert_eq!(
        selected(cx, &s).1.as_deref(),
        Some(&*row("a.yml")),
        "Up moves from it"
    );
}

#[gpui_kit::test]
fn a_selected_last_file_deleted_on_disk_selects_the_one_above(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    keyboard_on_tree(cx, &s);
    let old = select_row(cx, &s, row("c.yml"));
    frame(cx, s.win);
    delete_on_disk(cx, &s, "c.yml");
    assert_nearest(cx, &s, old, Some("b.yml"));
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    assert!(keyboard(cx), "the ring is the keyboard one");
}

#[gpui_kit::test]
fn a_focused_tabs_file_deleted_on_disk_rings_the_nearest_row(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    keyboard_in_tab(cx, &s, "b.yml");
    let old = selected(cx, &s).0.expect("the active request's row");
    assert_eq!(selected(cx, &s).1.as_deref(), Some(&*row("b.yml")));
    delete_on_disk(cx, &s, "b.yml");
    assert!(tab_names(cx, &s.ws).is_empty(), "the tab closed");
    assert!(
        ring_on(cx, s.win, "sidebar-tree"),
        "the tree shows its ring"
    );
    assert!(keyboard(cx), "the ring is the keyboard one");
    assert_nearest(cx, &s, old, Some("c.yml"));
    press_drawn(cx, s.win, "up");
    assert_eq!(
        selected(cx, &s).1.as_deref(),
        Some(&*row("a.yml")),
        "Up moves from it"
    );
}

#[gpui_kit::test]
fn closing_a_draft_from_its_menu_selects_the_nearest_row(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    let (draft, _) = new_draft_at(cx, &s, apinata_core::ItemPath::root());
    frame(cx, s.win);
    let old = select_row(cx, &s, row(&draft));
    choose_menu(cx, s.win, &draft, NodeKind::Draft, MenuEntry::CloseDraft);
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty(), "the draft closed");
    assert_nearest(cx, &s, old, None);
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
    assert!(!keyboard(cx), "a pointer close leaves the plain border");
}

// T4: Tab into the Timeline scrolls the focused eye into view.

#[gpui_kit::test]
fn tab_to_the_timeline_request_body_eye_scrolls_it_into_view(cx: &mut TestAppContext) {
    let fx = crate::timeline_redaction::shown(cx);
    gpui_kit::VisualTestContext::from_window(fx.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(700.), gpui_kit::px(220.)));
    frame(cx, fx.win);
    cx.update_window(fx.win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, fx.win);
    // A pane window starts with nothing focused, where Tab has no key path to act on.
    cx.update_window(fx.win, |_, w, cx| w.focus_next(cx))
        .unwrap();
    frame(cx, fx.win);
    key_to(cx, fx.win, "hop-request-body-eye");
    next_frame(cx, fx.win);
    next_frame(cx, fx.win);
    let (eye, view) = cx
        .update_window(fx.win, |_, w, _| {
            (
                w.find("hop-request-body-eye").bounds(),
                w.find("response-timeline").bounds(),
            )
        })
        .unwrap();
    assert!(
        eye.top() >= view.top() && eye.bottom() <= view.bottom(),
        "the eye {eye:?} lies inside the timeline {view:?}"
    );
}

// T5: a restored body that was saved scrubbed says why it cannot be revealed.

const SECRET: &str = "s3cr3t-value-123";

fn restored_body(cx: &mut TestAppContext, text: &str, secrets: &[String]) -> AnyWindowHandle {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body(text)),
        Vec::new(),
    );
    let saved_at = UNIX_EPOCH + Duration::from_secs(1_760_000_000);
    store
        .save_redacted(
            &root,
            &ip("a.yml"),
            &result,
            secrets,
            saved_at,
            Limits::default(),
        )
        .unwrap();
    cx.update(|cx| apinata_app::last_response::install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.restored().is_some()));
    frame(cx, win);
    std::mem::forget((col, state));
    win
}

#[gpui_kit::test]
fn a_restored_scrubbed_body_shows_a_disabled_eye(cx: &mut TestAppContext) {
    let win = restored_body(cx, &format!("token={SECRET}"), &[SECRET.to_owned()]);
    let label = cx
        .update_window(win, |_, w, _| {
            w.try_find("body-eye")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        label.as_deref(),
        Some(apinata_app::body_view::SCRUBBED_TIP),
        "the eye is shown and says why"
    );
    assert!(present(cx, win, "body-reveal"), "it shows its masked face");
    // The locked eye reads as disabled and takes no focus, so a full lap of Tab never rests on it.
    let disabled = cx
        .update_window(win, |_, w, _| {
            w.try_find("body-eye").and_then(|e| e.disabled())
        })
        .unwrap();
    assert_eq!(disabled, Some(true), "the locked eye reads as disabled");
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    frame(cx, win);
    let first = cx.update_window(win, |_, w, cx| w.focused(cx)).unwrap();
    for _ in 0..200 {
        assert!(!ring_on(cx, win, "body-eye"), "Tab reached the locked eye");
        platform_press(cx, win, "tab");
        next_frame(cx, win);
        if cx.update_window(win, |_, w, cx| w.focused(cx)).unwrap() == first {
            break;
        }
    }
    assert!(present(cx, win, "body-reveal"), "nothing was revealed");
}

#[gpui_kit::test]
fn a_restored_plain_body_has_no_eye(cx: &mut TestAppContext) {
    let win = restored_body(cx, "hello", &[SECRET.to_owned()]);
    assert!(!present(cx, win, "body-eye"));
}

#[test]
fn an_entry_without_masked_loads_unmasked() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("r"));
    let root = std::path::Path::new("/c");
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body(format!("token={SECRET}"))),
        Vec::new(),
    );
    store
        .save_redacted(
            root,
            &ip("a.yml"),
            &result,
            &[SECRET.to_owned()],
            UNIX_EPOCH,
            Limits::default(),
        )
        .unwrap();
    assert!(store.load(root, &ip("a.yml")).unwrap().body_masked);
    // Drop the field from the kept entry, as a file written before it existed has none.
    for file in std::fs::read_dir(store.dir()).unwrap() {
        let path = file.unwrap().path();
        if path.file_name().is_some_and(|n| n != "index.json")
            && path.extension().is_some_and(|e| e == "json")
        {
            let mut entry: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(entry.as_object_mut().unwrap().remove("masked").is_some());
            std::fs::write(&path, entry.to_string()).unwrap();
        }
    }
    let got = store
        .load(root, &ip("a.yml"))
        .expect("an older entry still loads");
    assert!(!got.body_masked);
}

// T2 (i562): focus handed back by the fallback leaves the caret at the URL's end, so a typed
// key extends the URL rather than landing before it.

fn drawn_url(cx: &mut TestAppContext, win: AnyWindowHandle) -> String {
    cx.update_window(win, |_, w, _| {
        w.try_find("url").and_then(|e| e.value().map(str::to_owned))
    })
    .unwrap()
    .unwrap_or_default()
}

// T6: a dialog opened from the title-bar app menu closes back into the active tab, whose
// opener (the menu) is gone by then. As QA B2a-B2c.

// Opens `rel` and clicks into its URL input as a mouse user does.
fn pointer_in_tab(cx: &mut TestAppContext, s: &Started, rel: &str) {
    open_tab(cx, &s.ws, &s.root, rel);
    frame(cx, s.win);
    platform_click(cx, s.win, "url");
    next_frame(cx, s.win);
    assert!(ring_on(cx, s.win, "url"), "the URL input holds focus");
    assert!(!keyboard(cx), "a click leaves pointer mode");
}

// Left-clicks, as the platform delivers it, the centre of `id` inside the drawn `scope`.
fn click_within(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    scope: &'static str,
    id: impl Into<ElementId>,
) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        let at = w.within(scope).find(id).bounds().center();
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    next_frame(cx, win);
    next_frame(cx, win);
}

// Clicks the app menu, then its row for `action`.
fn menu_pick(cx: &mut TestAppContext, s: &Started, action: &dyn gpui_kit::Action) {
    platform_click(cx, s.win, "app-menu");
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    let ix = apinata_app::app_menu::app_menu_index(action).expect("in the app menu");
    click_within(cx, s.win, "popup-menu", ix);
    assert!(dialog_open(cx, s.win), "the menu row opens its dialog");
}

// The drawn frame holds focus in the URL input and a typed key lands there.
fn assert_url_takes_keys(cx: &mut TestAppContext, s: &Started) {
    assert!(ring_on(cx, s.win, "url"), "the URL input holds focus");
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    let url = drawn_text(cx, s.win, "url");
    assert!(url.contains('x'), "typing reaches it: {url}");
}

fn menu_dialog_closed_by_its_x_returns_to_the_url(
    cx: &mut TestAppContext,
    action: &dyn gpui_kit::Action,
) {
    let (_col, s) = three_requests_started(cx);
    pointer_in_tab(cx, &s, "a.yml");
    menu_pick(cx, &s, action);
    platform_click(cx, s.win, "close");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "the x closes it");
    assert!(!keyboard(cx), "pointer mode stays, so no ring");
    assert_url_takes_keys(cx, &s);
}

#[gpui_kit::test]
fn runtime_variables_from_the_menu_closed_by_its_x_returns_to_the_url(cx: &mut TestAppContext) {
    menu_dialog_closed_by_its_x_returns_to_the_url(cx, &apinata_app::shell::OpenRuntimeVariables);
}

#[gpui_kit::test]
fn preferences_from_the_menu_closed_by_its_x_returns_to_the_url(cx: &mut TestAppContext) {
    menu_dialog_closed_by_its_x_returns_to_the_url(cx, &apinata_app::preferences::OpenPreferences);
}

#[gpui_kit::test]
fn the_cookie_jar_from_the_menu_closed_by_its_x_returns_to_the_url(cx: &mut TestAppContext) {
    menu_dialog_closed_by_its_x_returns_to_the_url(cx, &apinata_app::shell::OpenCookies);
}

#[gpui_kit::test]
fn the_palette_from_the_menu_closed_by_escape_returns_to_the_url(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    pointer_in_tab(cx, &s, "a.yml");
    menu_pick(cx, &s, &apinata_app::palette::OpenCommandPalette);
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert_url_takes_keys(cx, &s);
}

// The global environment picker's popup closes as its row opens the editor, but the picker
// itself stays drawn and is the opener the editor hands focus back to.
#[gpui_kit::test]
fn global_environments_from_the_picker_closed_by_its_x_return_to_the_picker(
    cx: &mut TestAppContext,
) {
    let (_col, s) = three_requests_started(cx);
    pointer_in_tab(cx, &s, "a.yml");
    platform_click(cx, s.win, "global-env-selector");
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    click_within(cx, s.win, "global-env-selector", "global-env-manage");
    assert!(
        present(cx, s.win, "global-env-editor"),
        "the manage row opens the editor"
    );
    platform_click(cx, s.win, "close");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "the x closes it");
    assert!(!keyboard(cx), "pointer mode stays, so no ring");
    assert!(
        ring_on(cx, s.win, "global-env-select"),
        "the picker holds focus"
    );
}

// The arrow steps from the menu button to the row of `action`: arrows skip greyed rows, and
// these collections sit in no repository, so its repository rows are greyed.
fn enabled_menu_row(action: &dyn gpui_kit::Action) -> usize {
    let off = apinata_app::app_menu::app_menu_disabled(false, false);
    let items = apinata_app::app_menu::app_menu_items();
    let ix = (items.iter())
        .position(|(_, a)| a.partial_eq(action))
        .expect("in the app menu");
    items[..ix].iter().filter(|(l, _)| !off.contains(l)).count()
}

// The keyboard path: Tab to the app menu, arrow down to its Runtime variables row, Enter, then
// Escape. The menu button held focus before the menu opened, so it gets the ring back.
#[gpui_kit::test]
fn runtime_variables_from_the_menu_by_key_ring_the_menu_button_on_escape(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    keyboard_in_tab(cx, &s, "a.yml");
    key_to(cx, s.win, "app-menu");
    press_drawn(cx, s.win, "enter");
    let row = enabled_menu_row(&apinata_app::shell::OpenRuntimeVariables);
    for _ in 0..=row {
        press_drawn(cx, s.win, "down");
    }
    press_drawn(cx, s.win, "enter");
    assert!(
        present(cx, s.win, "runtime-vars-add-name"),
        "Enter on the row opens runtime variables"
    );
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(keyboard(cx), "keyboard mode stays, so the ring shows");
    assert!(
        ring_on(cx, s.win, "app-menu"),
        "the menu button shows its ring"
    );
}

// T7: the tree held focus when a title-bar control opened the dialog, so closing it goes back
// to the tree, which is still drawn.

// Opens `rel` in a tab, then puts focus on the sidebar tree in pointer mode.
fn pointer_on_tree(cx: &mut TestAppContext, s: &Started, rel: &str) {
    open_tab(cx, &s.ws, &s.root, rel);
    frame(cx, s.win);
    let tree = tree_of(cx, s);
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    frame(cx, s.win);
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
    assert!(!keyboard(cx));
}

#[gpui_kit::test]
fn the_variables_button_dialog_closed_by_its_x_returns_to_the_tree(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    pointer_on_tree(cx, &s, "a.yml");
    platform_click(cx, s.win, "variables-button");
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(dialog_open(cx, s.win), "the button opens the overview");
    platform_click(cx, s.win, "close");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "the x closes it");
    assert!(!keyboard(cx), "pointer mode stays, so no ring");
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
}

#[gpui_kit::test]
fn runtime_variables_from_the_menu_closed_by_its_x_returns_to_the_tree(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    pointer_on_tree(cx, &s, "a.yml");
    menu_pick(cx, &s, &apinata_app::shell::OpenRuntimeVariables);
    platform_click(cx, s.win, "close");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "the x closes it");
    assert!(!keyboard(cx), "pointer mode stays, so no ring");
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
}

// The keyboard path from the tree: Tab to the variables button, Enter, then Escape.
#[gpui_kit::test]
fn the_variables_button_by_key_from_the_tree_rings_it_on_escape(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    keyboard_on_tree(cx, &s);
    key_to(cx, s.win, "variables-button");
    press_drawn(cx, s.win, "enter");
    assert!(dialog_open(cx, s.win), "Enter opens the overview");
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(keyboard(cx));
    assert!(
        ring_on(cx, s.win, "variables-button"),
        "the opener, still drawn, shows its ring"
    );
}

// Tab from the tree to the app menu: the menu button, not the tree, held focus before the
// menu, so the ring goes back to it.
#[gpui_kit::test]
fn runtime_variables_from_the_menu_by_key_from_the_tree_ring_the_menu_button(
    cx: &mut TestAppContext,
) {
    let (_col, s) = three_requests_started(cx);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    keyboard_on_tree(cx, &s);
    key_to(cx, s.win, "app-menu");
    press_drawn(cx, s.win, "enter");
    let row = enabled_menu_row(&apinata_app::shell::OpenRuntimeVariables);
    for _ in 0..=row {
        press_drawn(cx, s.win, "down");
    }
    press_drawn(cx, s.win, "enter");
    assert!(present(cx, s.win, "runtime-vars-add-name"));
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Escape closes it");
    assert!(keyboard(cx));
    assert!(
        ring_on(cx, s.win, "app-menu"),
        "the menu button shows its ring"
    );
}

// The branch label's dialog, opened by a click with the tree focused.
#[gpui_kit::test]
fn the_branch_dialog_closed_by_mouse_returns_to_the_tree(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let sb = crate::git_support::Sandbox::new();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("b.yml", &request("b", "http://127.0.0.1:9/b")),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "init");
    crate::git_support::write(col.path(), "a.yml", &request("a", "http://127.0.0.1:9/z"));
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.set_git(Some(sb.options()), cx));
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    wait_for(cx, "the changed count", |cx| {
        frame(cx, s.win);
        drawn_label(cx, s.win, "git-label").contains('1')
    });
    pointer_on_tree(cx, &s, "b.yml");
    platform_click(cx, s.win, "git-branch");
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert!(dialog_open(cx, s.win), "the label opens the branch dialog");
    assert!(
        cx.update(|cx| apinata_app::branch_dialog::current(cx))
            .is_some()
    );
    platform_click(cx, s.win, "branch-close");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Close closes it");
    assert!(!keyboard(cx), "pointer mode stays, so no ring");
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
}

fn drawn_label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
    .unwrap_or_default()
}

#[gpui_kit::test]
fn escape_after_the_openers_file_was_deleted_types_at_the_next_urls_end(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    open_tab(cx, &s.ws, &s.root, "b.yml");
    keyboard_in_tab(cx, &s, "a.yml");
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .dispatch_action(apinata_app::shell::OpenRuntimeVariables);
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    delete_on_disk(cx, &s, "a.yml");
    platform_press(cx, s.win, "escape");
    after_close(cx, s.win);
    assert!(ring_on(cx, s.win, "url"), "b's URL input holds focus");
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    assert_eq!(drawn_url(cx, s.win), "http://127.0.0.1:9/bx");
}

#[gpui_kit::test]
fn closing_the_focused_tab_types_at_the_next_urls_end(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    open_tab(cx, &s.ws, &s.root, "b.yml");
    keyboard_in_tab(cx, &s, "a.yml");
    platform_press(cx, s.win, &secondary("w"));
    next_frame(cx, s.win);
    next_frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"], "a's tab closed");
    assert!(ring_on(cx, s.win, "url"), "b's URL input holds focus");
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    assert_eq!(drawn_url(cx, s.win), "http://127.0.0.1:9/bx");
}

// T8: the shell takes focus on any press on a bare spot of it; focus stays where it was.

// Left-clicks the status bar's left end, a spot no control covers.
fn click_bare_status_bar(cx: &mut TestAppContext, win: AnyWindowHandle) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent, point, px};
    cx.update_window(win, |_, w, cx| {
        let bar = w.find("status-bar").bounds();
        let at = point(bar.left() + px(2.), bar.center().y);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    next_frame(cx, win);
    next_frame(cx, win);
}

#[gpui_kit::test]
fn a_bare_status_bar_click_keeps_the_tree_focused(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    pointer_on_tree(cx, &s, "a.yml");
    click_bare_status_bar(cx, s.win);
    assert!(ring_on(cx, s.win, "sidebar-tree"), "the tree holds focus");
}

#[gpui_kit::test]
fn a_bare_status_bar_click_keeps_the_url_focused(cx: &mut TestAppContext) {
    let (_col, s) = three_requests_started(cx);
    pointer_in_tab(cx, &s, "a.yml");
    click_bare_status_bar(cx, s.win);
    assert_url_takes_keys(cx, &s);
}

// The window started inactive and focus reached the URL before the first active frame, so
// the shell's first focus arrives only once the menu dialog closes.
#[gpui_kit::test]
fn a_menu_dialog_after_an_inactive_start_returns_to_the_url(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
    ]);
    let s = started(cx, &col);
    let id = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let url = area.read_with(cx, |a, cx| {
        a.editor(id).expect("an editor").read(cx).url_focus(cx)
    });
    cx.update_window(s.win, |_, w, cx| url.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, s.win);
    menu_pick(cx, &s, &apinata_app::shell::OpenRuntimeVariables);
    platform_click(cx, s.win, "close");
    after_close(cx, s.win);
    assert!(!dialog_open(cx, s.win), "the x closes it");
    assert_url_takes_keys(cx, &s);
}

// A menu removed while open never announces its close; the next use of the record unfreezes
// it, so recording cannot stay off for the rest of the session.
#[gpui_kit::test]
fn a_used_stand_in_records_again_after_a_freeze_that_never_ended(cx: &mut TestAppContext) {
    use apinata_app::stand_in::StandIn;
    let (_col, s) = three_requests_started(cx);
    let shell = s.shell.read_with(cx, gpui_kit::Focusable::focus_handle);
    let held = cx.update(|cx| cx.focus_handle());
    let stand_in = StandIn::default();
    stand_in.freeze(true);
    assert!(stand_in.take().is_none(), "nothing recorded yet");
    cx.update_window(s.win, |_, w, cx| {
        held.focus(w, cx);
        stand_in.hold(&shell, w, cx);
    })
    .unwrap();
    assert_eq!(stand_in.take(), Some(held), "recording works again");
}
