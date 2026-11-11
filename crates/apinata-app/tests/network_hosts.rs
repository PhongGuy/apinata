//! The Network tab's host override rows, their edit through the collection model, and the
//! timeline line a send through one shows.

use crate::support;

use apinata_app::command::{Command, Edit, NoticeKind, Target};
use apinata_app::network_section::{HostField, NetworkSection};
use apinata_app::response_pane::ResponsePane;
use apinata_core::{CollectionNetwork, HostOverride};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, WindowOptions,
};
use std::path::PathBuf;
use support::*;

type Host = Entity<SectionHost<NetworkSection>>;

fn section(cx: &mut TestAppContext, rows: Vec<HostOverride>) -> (AnyWindowHandle, Host) {
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let mut network = CollectionNetwork::default();
    network.host_overrides = rows;
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    (win, host)
}

fn sent(cx: &mut TestAppContext, host: &Host) -> Option<Vec<HostOverride>> {
    host.read_with(cx, |h, _| {
        h.edits.iter().rev().find_map(|e| match e {
            Edit::HostOverrides(rows) => Some(rows.clone()),
            _ => None,
        })
    })
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn loaded_rows_show_their_host_and_address(cx: &mut TestAppContext) {
    let (_win, host) = section(cx, vec![HostOverride::new("fake.test", "127.0.0.1:8080")]);
    let view = host.read_with(cx, |h, _| h.view.clone());
    let (name, address) = view.read_with(cx, |s, cx| {
        let text = |f| s.host_input(0, f).map(|i| i.read(cx).value().to_string());
        (text(HostField::Host), text(HostField::Address))
    });
    assert_eq!(name.as_deref(), Some("fake.test"));
    assert_eq!(address.as_deref(), Some("127.0.0.1:8080"));
}

#[gpui_kit::test]
fn add_then_type_a_host_and_an_address(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![]);
    click(cx, win, "host-add");
    assert_eq!(sent(cx, &host), None, "an empty row is a draft");
    type_at_end(cx, win, None, ("host-name", 0usize), "fake.test");
    cx.run_until_parked();
    assert_eq!(sent(cx, &host), None, "a row without an address is a draft");
    type_at_end(cx, win, None, ("host-address", 0usize), "127.0.0.1");
    cx.run_until_parked();
    assert_eq!(
        sent(cx, &host),
        Some(vec![HostOverride::new("fake.test", "127.0.0.1")])
    );
}

#[gpui_kit::test]
fn enabled_and_remove(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![HostOverride::new("fake.test", "127.0.0.1")]);
    click(cx, win, ("host-enabled", 0usize));
    let mut off = HostOverride::new("fake.test", "127.0.0.1");
    off.enabled = false;
    assert_eq!(sent(cx, &host), Some(vec![off]));
    click(cx, win, ("host-remove", 0usize));
    assert_eq!(sent(cx, &host), Some(vec![]));
}

#[gpui_kit::test]
fn the_edit_writes_a_bad_row_without_a_warning(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    let rows = vec![
        HostOverride::new("fake.test", "127.0.0.1"),
        HostOverride::new("bad.test", "nope"),
    ];
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Collection,
            edit: Edit::HostOverrides(rows.clone()),
        },
    );
    cx.run_until_parked();
    let got = snap(cx, &model)
        .content
        .collection
        .network
        .clone()
        .expect("network")
        .host_overrides;
    assert_eq!(got, rows);
    let mentions_bad = notices(cx, &model).into_iter().any(|n| match n {
        NoticeKind::Warnings(w) => w.iter().any(|l| l.contains("bad.test")),
        _ => false,
    });
    assert!(!mentions_bad);
    send(cx, &model, Command::Save(Target::Collection));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(dir.path().join("opencollection.yml")).unwrap();
    assert!(
        saved.contains("hostOverrides") && saved.contains("host: fake.test"),
        "{saved}"
    );
}

fn shown(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize)) -> bool {
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

#[gpui_kit::test]
fn an_added_empty_row_shows_no_error(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![]);
    click(cx, win, "host-add");
    assert!(!shown(cx, win, ("host-error", 0)));
    let notice = host.read_with(cx, |h, cx| h.view.read(cx).notice().map(str::to_owned));
    assert_eq!(notice, None);
}

#[gpui_kit::test]
fn a_bad_address_is_flagged_on_its_row_after_blur_only(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![]);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    click(cx, win, "host-add");
    type_at_end(cx, win, None, ("host-name", 0usize), "bad.test");
    frame(cx, win);
    type_at_end(cx, win, None, ("host-address", 0usize), "nope");
    frame(cx, win);
    cx.run_until_parked();
    frame(cx, win);
    assert!(!shown(cx, win, ("host-error", 0)), "flagged while typing");
    // Moving to the row's other input is not leaving the row.
    click(cx, win, ("host-name", 0usize));
    frame(cx, win);
    assert!(!shown(cx, win, ("host-error", 0)), "flagged inside the row");
    // Adding a row moves focus to the new one, which leaves the first.
    click(cx, win, "host-add");
    frame(cx, win);
    assert!(shown(cx, win, ("host-error", 0)));
    assert!(!shown(cx, win, ("host-error", 1)));
    let reason = HostOverride::new("bad.test", "nope")
        .check()
        .err()
        .map(|e| e.to_string())
        .unwrap();
    let text = cx
        .update_window(win, |_, w, _| {
            w.find(("host-error", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert!(text.is_some_and(|t| t.contains(&reason)), "{reason}");
    let notice = host.read_with(cx, |h, cx| h.view.read(cx).notice().map(str::to_owned));
    assert_eq!(notice, None);
}

#[gpui_kit::test]
fn a_disabled_bad_loaded_row_gets_a_muted_note(cx: &mut TestAppContext) {
    let mut row = HostOverride::new("bad.test", "nope");
    row.enabled = false;
    let (win, _host) = section(cx, vec![row]);
    assert!(shown(cx, win, ("host-muted", 0)));
    assert!(!shown(cx, win, ("host-error", 0)));
}

#[gpui_kit::test]
fn a_loaded_enabled_bad_row_is_flagged_at_once(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, vec![HostOverride::new("bad.test", "nope")]);
    assert!(shown(cx, win, ("host-error", 0)));
    assert!(!shown(cx, win, ("host-muted", 0)));
}

#[gpui_kit::test]
fn a_send_through_an_override_shows_it_on_the_timeline(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let port = server.address().port();
    let root = format!(
        "{ROOT}extensions:\n  apinata:\n    hostOverrides:\n      - host: fake.test\n        address: 127.0.0.1\n"
    );
    let a = request("a", &format!("http://fake.test:{port}/ok"));
    let col = write_tree(&[("opencollection.yml", &root), ("a.yml", &a)]);
    let (_state, ws, root_path) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root_path, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let m = model.clone();
    let (_win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let lines = pane.read_with(cx, |p, cx| p.hop_override_lines(cx));
    assert_eq!(lines, ["Host override: fake.test -> 127.0.0.1"]);
}

#[gpui_kit::test]
fn leaving_an_untouched_row_flags_nothing(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, vec![]);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    click(cx, win, "host-add");
    frame(cx, win);
    click(cx, win, "host-add");
    frame(cx, win);
    assert!(!shown(cx, win, ("host-error", 0)));
    assert!(!shown(cx, win, ("host-error", 1)));
}

#[gpui_kit::test]
fn a_fresh_disabled_row_gets_its_note_only_after_an_edit_and_leave(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, vec![]);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    click(cx, win, "host-add");
    click(cx, win, ("host-enabled", 0usize));
    frame(cx, win);
    assert!(!shown(cx, win, ("host-muted", 0)));
    type_at_end(cx, win, None, ("host-name", 0usize), "bad.test");
    type_at_end(cx, win, None, ("host-address", 0usize), "nope");
    frame(cx, win);
    assert!(!shown(cx, win, ("host-muted", 0)));
    click(cx, win, "host-add");
    frame(cx, win);
    assert!(shown(cx, win, ("host-muted", 0)));
    assert!(!shown(cx, win, ("host-error", 0)));
}

fn left(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: (&'static str, usize),
) -> gpui_kit::Pixels {
    cx.update_window(win, |_, w, _| w.find(id).bounds().left())
        .unwrap()
}

#[gpui_kit::test]
fn host_notes_start_at_the_host_inputs_left_edge(cx: &mut TestAppContext) {
    let mut off = HostOverride::new("off.test", "nope");
    off.enabled = false;
    let (win, _host) = section(cx, vec![HostOverride::new("bad.test", "nope"), off]);
    let input = left(cx, win, ("host-name", 0));
    assert_eq!(left(cx, win, ("host-error", 0)), input, "error note");
    let input = left(cx, win, ("host-name", 1));
    assert_eq!(left(cx, win, ("host-muted", 1)), input, "muted note");
}

#[gpui_kit::test]
fn a_focused_invalid_input_shows_a_danger_ring_and_a_valid_one_does_not(cx: &mut TestAppContext) {
    let (win, _host) = section(
        cx,
        vec![
            HostOverride::new("bad.test", "nope"),
            HostOverride::new("ok.test", "127.0.0.1"),
        ],
    );
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    assert!(!shown(cx, win, ("host-danger-ring", 0)), "unfocused");
    click(cx, win, ("host-name", 0usize));
    assert!(
        shown(cx, win, ("host-danger-ring", 0)),
        "focused invalid host"
    );
    click(cx, win, ("host-address", 0usize));
    assert!(
        shown(cx, win, ("address-danger-ring", 0)),
        "focused invalid address"
    );
    assert!(
        !shown(cx, win, ("host-danger-ring", 0)),
        "focus moved off the host"
    );
    click(cx, win, ("host-name", 1usize));
    assert!(
        !shown(cx, win, ("host-danger-ring", 1)),
        "focused valid host"
    );
    assert!(!shown(cx, win, ("address-danger-ring", 1)));
}

// The pointer accent ring on a focused input never covers the danger ring of an invalid one:
// the danger quad is the last one painted at the frame.
#[gpui_kit::test]
fn the_danger_ring_is_painted_over_the_accent_ring(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let (win, _host) = section(cx, vec![HostOverride::new("bad.test", "nope")]);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    click(cx, win, ("host-name", 0usize));
    let danger = cx.update(|cx| cx.theme().danger);
    let last = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let scale = w.scale_factor();
            let b = w.find(("host-danger-ring", 0usize)).bounds();
            let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
            w.painted_quads()
                .into_iter()
                .filter(|q| {
                    q.border_widths.left.0 > 0.
                        && (q.bounds.origin.x.0 - l).abs() < 6. * scale
                        && (q.bounds.origin.y.0 - t).abs() < 6. * scale
                })
                .map(|q| q.border_color)
                .next_back()
        })
        .unwrap();
    assert_eq!(last, Some(danger));
}

#[gpui_kit::test]
fn an_incomplete_row_says_it_is_not_saved_until_both_fields_are_filled(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, vec![]);
    click(cx, win, "host-add");
    assert!(shown(cx, win, ("host-incomplete", 0)));
    type_at_end(cx, win, None, ("host-name", 0usize), "fake.test");
    type_at_end(cx, win, None, ("host-address", 0usize), "127.0.0.1");
    frame(cx, win);
    assert!(!shown(cx, win, ("host-incomplete", 0)));
}

#[gpui_kit::test]
fn clearing_a_saved_rows_host_holds_the_row_back(cx: &mut TestAppContext) {
    let (win, host) = section(
        cx,
        vec![
            HostOverride::new("a.test", "127.0.0.1"),
            HostOverride::new("b.test", "127.0.0.2"),
        ],
    );
    replace_input(cx, win, None, ("host-name", 0usize), "");
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(
        sent(cx, &host),
        Some(vec![HostOverride::new("b.test", "127.0.0.2")])
    );
    assert!(shown(cx, win, ("host-incomplete", 0)));
    assert!(!shown(cx, win, ("host-error", 0)));
    type_at_end(cx, win, None, ("host-name", 0usize), "c.test");
    cx.run_until_parked();
    assert_eq!(
        sent(cx, &host),
        Some(vec![
            HostOverride::new("c.test", "127.0.0.1"),
            HostOverride::new("b.test", "127.0.0.2"),
        ])
    );
}

#[gpui_kit::test]
fn removing_or_toggling_a_draft_row_sends_nothing(cx: &mut TestAppContext) {
    let (win, host) = section(cx, vec![]);
    click(cx, win, "host-add");
    click(cx, win, ("host-enabled", 0usize));
    assert_eq!(sent(cx, &host), None, "toggling a draft");
    click(cx, win, ("host-remove", 0usize));
    assert_eq!(sent(cx, &host), None, "removing a draft");
}

#[gpui_kit::test]
fn an_echo_of_the_saved_rows_keeps_a_draft_in_place(cx: &mut TestAppContext) {
    let b = HostOverride::new("b.test", "127.0.0.2");
    let (win, host) = section(
        cx,
        vec![HostOverride::new("a.test", "127.0.0.1"), b.clone()],
    );
    replace_input(cx, win, None, ("host-name", 0usize), "");
    cx.run_until_parked();
    assert_eq!(sent(cx, &host), Some(vec![b.clone()]));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let mut network = CollectionNetwork::default();
    network.host_overrides = vec![b];
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let text = |cx: &mut TestAppContext, row, f| {
        s.read_with(cx, |s, cx| {
            s.host_input(row, f).map(|i| i.read(cx).value().to_string())
        })
    };
    assert_eq!(text(cx, 0, HostField::Host).as_deref(), Some(""));
    assert_eq!(
        text(cx, 0, HostField::Address).as_deref(),
        Some("127.0.0.1")
    );
    assert_eq!(text(cx, 1, HostField::Host).as_deref(), Some("b.test"));
    assert_eq!(text(cx, 2, HostField::Host), None);
    assert!(shown(cx, win, ("host-incomplete", 0)));
}
