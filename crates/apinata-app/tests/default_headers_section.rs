//! The Default headers group under a request's headers: one row per header core adds, whose
//! toggle writes `settings.omitHeaders` through the normal edit and save path.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::default_headers_section::DefaultHeadersSection;
use apinata_core::KeyValue;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

fn section(
    cx: &mut TestAppContext,
    omitted: &[&str],
    own: &[&str],
) -> (AnyWindowHandle, Entity<SectionHost<DefaultHeadersSection>>) {
    let (win, host) = host_section(cx, DefaultHeadersSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let omitted: Vec<String> = omitted.iter().map(|n| (*n).to_owned()).collect();
    let own: Vec<KeyValue> = own.iter().map(|n| KeyValue::new(*n, "x")).collect();
    cx.update_window(win, |_, _, cx| {
        s.update(cx, |s, cx| {
            s.load(&omitted, cx);
            s.set_own(&own, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    (win, host)
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

fn last(cx: &mut TestAppContext, host: &Entity<SectionHost<DefaultHeadersSection>>) -> Vec<String> {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::OmitHeaders(names)) => names,
        other => panic!("no omit edit: {other:?}"),
    }
}

#[gpui_kit::test]
fn one_row_per_default_with_its_preview_value(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, &[], &[]);
    assert!(found(cx, win, "default-headers"));
    assert!(found(cx, win, "default-header-user-agent"));
    assert!(found(cx, win, "default-header-accept"));
    assert!(found(cx, win, "default-header-user-agent-value"));
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn all_four_defaults_have_a_row(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, &[], &[]);
    assert!(found(cx, win, "default-header-accept-encoding"));
    assert!(found(cx, win, "default-header-connection"));
    assert_eq!(
        label(cx, win, "default-header-connection-value").as_deref(),
        Some("keep-alive")
    );
}

#[gpui_kit::test]
fn an_omitted_accept_says_it_still_sends_the_wildcard(cx: &mut TestAppContext) {
    // The HTTP stack cannot drop Accept, so the row shows what really goes out.
    let (win, _host) = section(cx, &["Accept", "Connection"], &[]);
    assert_eq!(
        label(cx, win, "default-header-accept-value").as_deref(),
        Some("sends */*")
    );
    // Connection really leaves the wire, so its row keeps the default value.
    assert_eq!(
        label(cx, win, "default-header-connection-value").as_deref(),
        Some("keep-alive")
    );
    let (win, _host) = section(cx, &[], &[]);
    assert_eq!(
        label(cx, win, "default-header-accept-value").as_deref(),
        Some("application/json, text/plain, */*")
    );
}

#[gpui_kit::test]
fn toggling_off_omits_and_toggling_on_sends_again(cx: &mut TestAppContext) {
    let (win, host) = section(cx, &[], &[]);
    click(cx, win, "default-header-send-user-agent");
    assert_eq!(last(cx, &host), ["User-Agent"]);
    click(cx, win, "default-header-send-accept");
    assert_eq!(last(cx, &host), ["User-Agent", "Accept"]);
    click(cx, win, "default-header-send-user-agent");
    assert_eq!(last(cx, &host), ["Accept"]);
}

#[gpui_kit::test]
fn names_already_omitted_in_another_case_are_kept_or_removed_by_name(cx: &mut TestAppContext) {
    // A hand-written lowercase name counts as omitted; other names in the list are untouched.
    let (win, host) = section(cx, &["user-agent", "Connection"], &[]);
    click(cx, win, "default-header-send-user-agent");
    assert_eq!(last(cx, &host), ["Connection"]);
}

#[gpui_kit::test]
fn a_header_the_user_set_shows_overridden_and_cannot_be_toggled(cx: &mut TestAppContext) {
    let (win, host) = section(cx, &[], &["user-agent"]);
    assert!(found(cx, win, "default-header-user-agent-overridden"));
    assert!(!found(cx, win, "default-header-accept-overridden"));
    click(cx, win, "default-header-send-user-agent");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn toggling_edits_and_saves_the_file_and_the_snapshot_shows_it(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ]);
    let model = open(cx, &dir);
    let a = Target::Request(ip("a.yml"));
    send(
        cx,
        &model,
        Command::Edit {
            target: a.clone(),
            edit: Edit::OmitHeaders(vec!["User-Agent".into()]),
        },
    );
    cx.run_until_parked();
    let s = snap(cx, &model);
    let view = &s.content.requests[&ip("a.yml")];
    assert_eq!(view.omitted_headers, ["User-Agent"]);
    assert!(view.dirty);
    send(cx, &model, Command::Save(a.clone()));
    cx.run_until_parked();
    let on_disk = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert!(
        on_disk.ends_with("settings:\n  omitHeaders:\n    - User-Agent\n"),
        "{on_disk}"
    );
    // A fresh open reads the saved state back.
    let reopened = open(cx, &dir);
    let s = snap(cx, &reopened);
    assert_eq!(
        s.content.requests[&ip("a.yml")].omitted_headers,
        ["User-Agent"]
    );
}

#[gpui_kit::test]
fn the_request_editor_shows_the_group_under_the_headers_table(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    let text = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n  headers:\n    - name: Accept\n      value: text/csv\nsettings:\n  omitHeaders:\n    - User-Agent\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", text)]);
    let w = watching(cx, &dir, None);
    let (win, view) = editor_for(cx, &w.ws, &w.root, "a.yml", false);
    view.update(cx, |v, cx| v.show(Section::Headers, cx));
    assert!(found(cx, win, "default-headers"));
    // The user's own Accept wins; the omitted User-Agent is shown off.
    assert!(found(cx, win, "default-header-accept-overridden"));
    assert!(!found(cx, win, "default-header-user-agent-overridden"));
    click(cx, win, "default-header-send-user-agent");
    let s = ws_snap(cx, &w.ws, &w.root);
    let a = &s.content.requests[&ip("a.yml")];
    assert!(
        a.omitted_headers.is_empty() && a.dirty,
        "{:?}",
        a.omitted_headers
    );
}

// Names and values draw in the mono font at the small size, like the header table above.
#[gpui_kit::test]
fn default_header_cells_use_the_mono_font(cx: &mut TestAppContext) {
    use apinata_app::default_headers_section::header_cell;
    use gpui_kit::Styled as _;
    use gpui_kit::component::ActiveTheme as _;
    support::init(cx);
    cx.update(|cx| {
        let mut cell = header_cell(gpui_kit::div(), cx);
        let text = &cell.style().text;
        assert_eq!(text.font_family, Some(cx.theme().mono_font_family.clone()));
        assert_eq!(text.font_size, Some(apinata_app::style::TEXT_SMALL.into()));
    });
}
