//! The trusted-hosts settings: this collection's lists from the per-user trust file.

use crate::support;

use apinata_app::config_editor::ConfigSection;
use apinata_app::tabs::TabKind;
use apinata_app::trusted_hosts::TrustedHostsView;
use apinata_core::{ApprovalInbox, TrustStore};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext};
use std::fs;
use std::path::PathBuf;
use support::*;
use tempfile::TempDir;

struct Fx {
    state: TempDir,
    coll: TempDir,
}

impl Fx {
    fn new() -> Fx {
        Fx {
            state: TempDir::new().unwrap(),
            coll: TempDir::new().unwrap(),
        }
    }
    fn trust_file(&self) -> PathBuf {
        self.state.path().join("trusted-hosts.json")
    }
    fn store(&self) -> TrustStore {
        TrustStore::load(self.trust_file()).unwrap()
    }
}

fn view(cx: &mut TestAppContext, fx: &Fx) -> (AnyWindowHandle, Entity<TrustedHostsView>) {
    support::init(cx);
    let (file, root) = (fx.trust_file(), fx.coll.path().to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TrustedHostsView::new(file, root, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

fn lists(cx: &mut TestAppContext, v: &Entity<TrustedHostsView>) -> (Vec<String>, Vec<String>) {
    v.read_with(cx, |v, _| (v.trusted().to_vec(), v.denied().to_vec()))
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn add(cx: &mut TestAppContext, win: AnyWindowHandle, host: &str) {
    replace_input(cx, win, None, "hosts-add", host);
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn shows_this_collections_lists_only(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let other = TempDir::new().unwrap();
    let mut s = TrustStore::empty(fx.trust_file());
    s.trust(fx.coll.path(), "a.example").unwrap();
    s.trust(fx.coll.path(), "b.example").unwrap();
    s.deny(fx.coll.path(), "evil.example").unwrap();
    s.trust(other.path(), "elsewhere.example").unwrap();
    s.save().unwrap();
    let (win, v) = view(cx, &fx);
    assert_eq!(
        lists(cx, &v),
        (
            vec!["a.example".to_string(), "b.example".into()],
            vec!["evil.example".to_string()]
        )
    );
    assert!(present(cx, win, ("trusted-host", 1usize)));
    assert!(present(cx, win, ("denied-host", 0usize)));
    assert!(!present(cx, win, ("trusted-host", 2usize)));
}

#[gpui_kit::test]
fn a_typed_host_is_trusted_on_enter(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, v) = view(cx, &fx);
    add(cx, win, "API.Example.com.");
    assert!(fx.store().is_trusted(fx.coll.path(), "api.example.com"));
    assert_eq!(lists(cx, &v).0, ["api.example.com"]);
}

#[gpui_kit::test]
fn remove_takes_a_host_off_its_list(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let mut s = TrustStore::empty(fx.trust_file());
    s.trust(fx.coll.path(), "a.example").unwrap();
    s.deny(fx.coll.path(), "evil.example").unwrap();
    s.save().unwrap();
    let (win, v) = view(cx, &fx);
    click(cx, win, ("trusted-remove", 0usize));
    click(cx, win, ("denied-remove", 0usize));
    assert!(fx.store().trusted(fx.coll.path()).is_empty());
    assert!(fx.store().denied(fx.coll.path()).is_empty());
    assert_eq!(lists(cx, &v), (Vec::<String>::new(), Vec::<String>::new()));
}

#[gpui_kit::test]
fn an_invalid_host_is_refused_without_writing(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, v) = view(cx, &fx);
    add(cx, win, "exa mple");
    assert!(present(cx, win, "hosts-error"));
    assert!(v.read_with(cx, |v, _| v.error().is_some()));
    assert!(!fx.trust_file().exists());
}

#[gpui_kit::test]
fn a_malformed_trust_file_is_shown_and_never_overwritten(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fs::write(fx.trust_file(), "{ broken").unwrap();
    let (win, v) = view(cx, &fx);
    assert!(v.read_with(cx, |v, _| v.error().is_some()));
    add(cx, win, "a.example");
    assert_eq!(fs::read_to_string(fx.trust_file()).unwrap(), "{ broken");
}

#[gpui_kit::test]
fn collection_settings_offer_trusted_hosts_and_folder_settings_do_not(cx: &mut TestAppContext) {
    use ConfigSection::*;
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/a.yml", &request("a", "http://x")),
    ]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Collection, cx)
        })
        .unwrap();
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    assert_eq!(
        ed.read_with(cx, |e, _| e.sections()),
        [
            Headers, Auth, Vars, Script, Tests, Docs, Hosts, Network, Presets, Scripts
        ]
    );
    let ix = ed.read_with(cx, |e, _| e.sections().iter().position(|x| *x == Hosts));
    click(cx, s.win, ("config-section", ix.unwrap()));
    assert!(present(cx, s.win, "config-hosts"));
    let hosts = ed
        .read_with(cx, |e, _| e.hosts().cloned())
        .expect("a hosts view");

    // A decision in the modal shows up in the open settings.
    let store = TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    ApprovalInbox::new(s.files.path().join("approvals"))
        .request(&store, &s.root, "api.example.com", "a.yml")
        .unwrap();
    let approvals = cx.update(|cx| cx.global::<apinata_app::app::AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    frame(cx, s.win);
    click(cx, s.win, "approval-trust");
    assert_eq!(
        hosts.read_with(cx, |h, _| h.trusted().to_vec()),
        ["api.example.com"]
    );

    let folder_tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Folder(ip("users")), cx)
        })
        .unwrap();
    frame(cx, s.win);
    let fed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(folder_tab).cloned()
        })
        .unwrap();
    assert_eq!(
        fed.read_with(cx, |e, _| e.sections()),
        [Headers, Auth, Vars, Script, Tests, Docs, Tags]
    );
    assert!(fed.read_with(cx, |e, _| e.hosts().is_none()));
}

#[gpui_kit::test]
fn input_that_is_not_a_bare_host_is_refused_without_writing(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, v) = view(cx, &fx);
    for input in [
        "good.example@evil.example",
        "evil.example#.good.example",
        "evil.example/.good",
        "ex\tample.com",
        "http://good.example",
    ] {
        add(cx, win, input);
        assert!(v.read_with(cx, |v, _| v.error().is_some()), "{input:?}");
        assert!(!fx.trust_file().exists(), "{input:?} wrote the trust file");
    }
}

#[gpui_kit::test]
fn plain_and_uppercase_hosts_are_trusted(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, v) = view(cx, &fx);
    add(cx, win, "plain.example");
    add(cx, win, "UPPER.Example");
    add(cx, win, "[::1]");
    assert_eq!(lists(cx, &v).0, ["plain.example", "upper.example", "[::1]"]);
    assert!(v.read_with(cx, |v, _| v.error().is_none()));
}

#[gpui_kit::test]
fn a_user_at_host_refusal_never_names_the_host_after_the_at(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, v) = view(cx, &fx);
    add(cx, win, "good.example@evil.example");
    let error = v
        .read_with(cx, |v, _| v.error().map(str::to_owned))
        .unwrap();
    assert!(!error.contains("evil.example"), "{error}");
    assert!(error.contains("bare host name"), "{error}");
}

// One mouse click with the platform's `count` at the centre of element `id`.
fn mouse_click(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: (&'static str, usize),
    count: usize,
) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let at = cx
        .update_window(win, |_, w, _| w.find(id).bounds().center())
        .unwrap();
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: count,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: count,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, win);
    }
}

#[gpui_kit::test]
fn a_double_click_on_remove_removes_only_one_host(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let mut s = TrustStore::empty(fx.trust_file());
    s.trust(fx.coll.path(), "a.example").unwrap();
    s.trust(fx.coll.path(), "b.example").unwrap();
    s.save().unwrap();
    let (win, v) = view(cx, &fx);
    mouse_click(cx, win, ("trusted-remove", 0), 1);
    mouse_click(cx, win, ("trusted-remove", 0), 2);
    assert_eq!(lists(cx, &v).0, ["b.example"], "the repeat click removed b");
    assert!(fx.store().is_trusted(fx.coll.path(), "b.example"));
}

#[gpui_kit::test]
fn empty_lists_say_so(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, _v) = view(cx, &fx);
    assert!(present(cx, win, "trusted-empty"));
    assert!(present(cx, win, "denied-empty"));
    add(cx, win, "a.example");
    assert!(!present(cx, win, "trusted-empty"));
    assert!(present(cx, win, "denied-empty"));
}

#[gpui_kit::test]
fn the_add_button_trusts_the_typed_host(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, v) = view(cx, &fx);
    replace_input(cx, win, None, "hosts-add", "api.example");
    click(cx, win, "hosts-add-button");
    assert_eq!(lists(cx, &v).0, ["api.example"]);
    assert!(fx.store().is_trusted(fx.coll.path(), "api.example"));
}

#[gpui_kit::test]
fn the_lists_say_they_apply_only_to_the_mcp_server(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, _v) = view(cx, &fx);
    let note = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("hosts-note").label().map(str::to_owned)
        })
        .unwrap()
        .unwrap_or_default();
    assert!(note.contains("MCP server"), "{note}");
    assert!(note.contains("from the app"), "{note}");
}

#[gpui_kit::test]
fn trust_host_sits_left_of_its_input(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let (win, _v) = view(cx, &fx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let label = w.within("hosts-add-row").find("form-label").bounds();
        let input = w.find("hosts-add").bounds();
        assert!(label.right() <= input.left(), "{label:?} {input:?}");
        assert!(
            (f32::from(label.top()) - f32::from(input.top())).abs() < 0.5,
            "{label:?} {input:?}"
        );
    })
    .unwrap();
}
