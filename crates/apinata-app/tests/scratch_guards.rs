//! Close, Export, Run collection and Collection settings on the scratch pad: one short notice
//! from every entry point, and no effect.

use crate::support;

use apinata_app::shell::{
    CloseCollection, ExportCollection, ExportCollectionHtml, ExportCollectionOpenApi,
    ExportCollectionPostmanKeepScripts,
};
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AppContext as _, Focusable as _, TestAppContext};
use support::*;

// The scratch pad holding a draft, so its tab is active and its row is listed.
fn scratch_open(cx: &mut TestAppContext) -> Started {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    s
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn tab_count(cx: &mut TestAppContext, s: &Started) -> usize {
    s.ws.read_with(cx, |w, _| w.tabs().entries().len())
}

fn dispatch(cx: &mut TestAppContext, s: &Started, action: impl Action) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(action.boxed_clone(), cx)
    })
    .unwrap();
    cx.run_until_parked();
}

// Runs `act` and asserts it raised exactly one notice and changed nothing else.
fn assert_refused(cx: &mut TestAppContext, s: &Started, act: impl FnOnce(&mut TestAppContext)) {
    let (toasts_before, tabs_before) = (toasts(cx, s), tab_count(cx, s));
    act(cx);
    assert_eq!(toasts(cx, s), toasts_before + 1, "one notice");
    assert_eq!(tab_count(cx, s), tabs_before, "no tab opened or closed");
    assert!(!cx.did_prompt_for_new_path(), "no save dialog");
}

#[gpui_kit::test]
fn close_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| dispatch(cx, &s, CloseCollection));
}

#[gpui_kit::test]
fn export_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| dispatch(cx, &s, ExportCollection));
}

#[gpui_kit::test]
fn export_keeping_scripts_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        dispatch(cx, &s, ExportCollectionPostmanKeepScripts)
    });
}

#[gpui_kit::test]
fn export_openapi_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| dispatch(cx, &s, ExportCollectionOpenApi));
}

#[gpui_kit::test]
fn export_html_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| dispatch(cx, &s, ExportCollectionHtml));
}

#[gpui_kit::test]
fn run_collection_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        dispatch(cx, &s, apinata_app::keymap::RunCollection)
    });
}

#[gpui_kit::test]
fn collection_settings_on_a_scratch_tab_shows_a_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        dispatch(cx, &s, apinata_app::keymap::OpenCollectionSettings)
    });
}

#[gpui_kit::test]
fn the_palette_run_collection_shows_the_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        let shell = s.shell.clone();
        cx.update_window(s.win, |_, w, cx| {
            shell.read(cx).focus_handle(cx).focus(w, cx)
        })
        .unwrap();
        cx.simulate_keystrokes(s.win, &key("shift-p"));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| w.input("Run collection", cx))
            .unwrap();
        frame(cx, s.win);
        cx.simulate_keystrokes(s.win, "enter");
        cx.run_until_parked();
    });
}

#[gpui_kit::test]
fn exporting_the_scratch_root_directly_shows_the_notice(cx: &mut TestAppContext) {
    use apinata_app::export::ExportFormat;
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        let (ws, root) = (s.ws.clone(), s.root.clone());
        cx.update_window(s.win, |_, w, cx| {
            apinata_app::export::export_collection(
                &ws,
                &root,
                ExportFormat::Postman {
                    keep_scripts: false,
                },
                w,
                cx,
            )
        })
        .unwrap();
        cx.run_until_parked();
    });
}

#[gpui_kit::test]
fn the_scratch_row_menu_disables_collection_only_entries(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    for entry in [
        MenuEntry::RunCollection,
        MenuEntry::CollectionSettings,
        MenuEntry::ExportPostman,
        MenuEntry::ExportPostmanKeepScripts,
        MenuEntry::ExportOpenApi,
        MenuEntry::ExportHtml,
        MenuEntry::Close,
    ] {
        let (toasts_before, tabs_before) = (toasts(cx, &s), tab_count(cx, &s));
        choose_menu(cx, s.win, "", NodeKind::Collection, entry);
        assert_eq!(tab_count(cx, &s), tabs_before, "{entry:?} opened a tab");
        assert!(!cx.did_prompt_for_new_path(), "{entry:?} prompted");
        assert_eq!(toasts(cx, &s), toasts_before, "{entry:?} is disabled");
    }
}

#[gpui_kit::test]
fn the_scratch_row_play_button_does_nothing(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    let tabs_before = tab_count(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("run-collection", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_count(cx, &s), tabs_before, "no runner tab");
}

#[gpui_kit::test]
fn shift_enter_on_the_scratch_row_shows_the_notice(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        cx.update_window(s.win, |_, w, cx| w.click(row(""), cx))
            .unwrap();
        frame(cx, s.win);
        cx.simulate_keystrokes(s.win, "shift-enter");
        cx.run_until_parked();
    });
}

#[gpui_kit::test]
fn the_breadcrumb_collection_crumb_shows_the_notice_on_a_scratch_tab(cx: &mut TestAppContext) {
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.click("breadcrumb-collection", cx)
        })
        .unwrap();
        cx.run_until_parked();
    });
}

#[gpui_kit::test]
fn running_a_selection_of_scratch_rows_shows_the_notice(cx: &mut TestAppContext) {
    use apinata_app::sidebar::SidebarEvent;
    let s = scratch_open(cx);
    assert_refused(cx, &s, |cx| {
        let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
        let event = SidebarEvent::RunSelection {
            root: s.root.clone(),
            items: vec![ip("a.yml")],
        };
        sidebar.update(cx, |_, cx| cx.emit(event));
        cx.run_until_parked();
    });
}
