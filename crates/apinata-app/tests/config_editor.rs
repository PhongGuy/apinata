use crate::support;

use apinata_app::auth_section::AuthMode;
use apinata_app::command::{Command, Target};
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::kv_table::KvRow;
use apinata_app::style::{PANE_INSET, at_zoom};
use apinata_app::workspace::Workspace;
use apinata_core::{AuthSpec, Collection, EdgeGridSpec, KeyValue};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::Path;
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/a.yml", &request("a", "http://x")),
    ])
}

fn editor(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: ConfigTarget,
) -> (AnyWindowHandle, Entity<ConfigEditor>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, target, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, view)
}

fn row(name: &str, value: &str) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        ..KvRow::default()
    }
}

fn bytes(dir: &tempfile::TempDir, rel: &str) -> Vec<u8> {
    std::fs::read(dir.path().join(rel)).unwrap()
}

#[gpui_kit::test]
fn a_folder_header_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row("X-Trace", "1")], w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let users = ws
        .read_with(cx, |w, cx| w.snapshot(&root, cx))
        .unwrap()
        .content
        .folders[&ip("users")]
        .clone();
    assert!(users.dirty && users.headers.len() == 1);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.within("config-headers").click(("kv-value", 0usize), cx);
        w.press(&key("s"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let f = c.folder_file_mut(&ip("users")).unwrap();
    f.set_headers(vec![KeyValue::new("X-Trace", "1")]).unwrap();
    f.save().unwrap();
    assert_eq!(
        bytes(&col, "users/folder.yml"),
        bytes(&twin, "users/folder.yml")
    );
}

#[gpui_kit::test]
fn collection_auth_offers_no_inherit_and_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    assert!(!auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::Inherit)));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |a, cx| a.set_mode(AuthMode::Bearer, w, cx))
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, "auth-token", "t");
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            apinata_app::command::Command::Save(apinata_app::command::Target::Collection),
            cx,
        )
    });
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    c.file_mut()
        .set_auth(AuthSpec::Bearer { token: "t".into() });
    c.file_mut().save().unwrap();
    assert_eq!(
        bytes(&col, "opencollection.yml"),
        bytes(&twin, "opencollection.yml")
    );
}

#[gpui_kit::test]
fn folder_auth_offers_inherit(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    assert!(auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::Inherit)));
}

#[gpui_kit::test]
fn vars_and_docs_reach_the_draft_and_a_refused_header_shows_its_error(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Vars, vec![row("base", "http://x")], w, cx)
        })
    })
    .unwrap();
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Docs, cx));
    frame(cx, win);
    replace_code(cx, win, Some("config-docs"), "docs-code", "notes");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row("a\nb", "1")], w, cx)
        })
    })
    .unwrap();
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    frame(cx, win);
    let c = ws
        .read_with(cx, |w, cx| w.snapshot(&root, cx))
        .unwrap()
        .content
        .collection
        .clone();
    assert_eq!(
        (c.vars.len(), c.docs.as_str(), c.headers.len()),
        (1, "notes", 0)
    );
    assert!(
        present(cx, win, "kv-error"),
        "a header name with a line break is refused"
    );
}

#[gpui_kit::test]
fn a_refused_config_header_name_marks_its_own_cell(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    let send = |cx: &mut TestAppContext, rows: Vec<KvRow>| {
        cx.update_window(win, |_, w, cx| {
            ed.update(cx, |e, cx| e.edit_rows(ConfigSection::Headers, rows, w, cx))
        })
        .unwrap();
        frame(cx, win);
    };
    send(cx, vec![row("a", "1"), row("b", "2")]);
    send(cx, vec![row("a", "1"), row("b\nc", "2")]);
    assert!(present(cx, win, "kv-error"));
    assert!(present(cx, win, ("kv-name-invalid-icon", 1usize)));
    assert!(!present(cx, win, ("kv-name-invalid-icon", 0usize)));
}

#[gpui_kit::test]
fn settings_pages_are_inset_like_the_other_panes(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    for target in [ConfigTarget::Collection, ConfigTarget::Folder(ip("users"))] {
        let (win, _ed) = editor(cx, &ws, &root, target);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let width = w.viewport_size().width;
            let inset = at_zoom(PANE_INSET, 1.0);
            for id in ["config-title", "config-headers"] {
                let b = w.find(id).bounds();
                assert!(b.left() >= inset, "{id} left: {b:?}");
                assert!(b.right() <= width - inset, "{id} right: {b:?}");
            }
            assert!(w.find("config-title").bounds().top() >= inset);
        })
        .unwrap();
    }
}

// The label of the No Auth line, `None` when it is not drawn.
fn no_auth_text(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("auth-none")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// Collection and folder auth is what their requests inherit, so No Auth there does not
// speak of "this request".
#[gpui_kit::test]
fn no_auth_text_names_the_level_it_applies_to(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let cases = [
        (
            ConfigTarget::Collection,
            "Requests that inherit from the collection send no auth.",
        ),
        (
            ConfigTarget::Folder(ip("users")),
            "Requests that inherit from this folder send no auth.",
        ),
    ];
    for (target, want) in cases {
        let (win, ed) = editor(cx, &ws, &root, target);
        let auth = ed.read_with(cx, |e, _| e.auth().clone());
        ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
        cx.update_window(win, |_, w, cx| {
            auth.update(cx, |a, cx| a.set_mode(AuthMode::None, w, cx))
        })
        .unwrap();
        frame(cx, win);
        assert_eq!(no_auth_text(cx, win).as_deref(), Some(want));
    }
}

#[gpui_kit::test]
fn folder_wsse_auth_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    assert!(auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::Wsse)));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |a, cx| a.set_mode(AuthMode::Wsse, w, cx))
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, "auth-username", "fu");
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Folder(ip("users"))), cx)
    });
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let f = c.folder_file_mut(&ip("users")).unwrap();
    f.set_auth(AuthSpec::Wsse {
        username: "fu".into(),
        password: String::new(),
    });
    f.save().unwrap();
    assert_eq!(
        bytes(&col, "users/folder.yml"),
        bytes(&twin, "users/folder.yml")
    );
}

#[gpui_kit::test]
fn collection_edgegrid_auth_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    assert!(auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::EdgeGrid)));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |a, cx| a.set_mode(AuthMode::EdgeGrid, w, cx))
    })
    .unwrap();
    frame(cx, win);
    for (id, text) in [
        ("auth-eg-access-token", "at"),
        ("auth-eg-client-token", "ct"),
        ("auth-eg-client-secret", "cs"),
    ] {
        type_at_end(cx, win, None, id, text);
        cx.run_until_parked();
    }
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Collection), cx)
    });
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let mut s = EdgeGridSpec::default();
    (s.access_token, s.client_token, s.client_secret) = ("at".into(), "ct".into(), "cs".into());
    c.file_mut().set_auth(AuthSpec::EdgeGrid(s));
    c.file_mut().save().unwrap();
    assert_eq!(
        bytes(&col, "opencollection.yml"),
        bytes(&twin, "opencollection.yml")
    );
}

// Folder and collection auth share the section, so each level refuses a non-number.
fn refuses_a_non_number_max_body_size(cx: &mut TestAppContext, target: ConfigTarget) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, target);
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |a, cx| a.set_mode(AuthMode::EdgeGrid, w, cx))
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, "auth-eg-max-body-size", "12kb");
    cx.run_until_parked();
    assert!(present(cx, win, "auth-eg-max-body-size-error"));
}

#[gpui_kit::test]
fn folder_max_body_size_refuses_a_non_number(cx: &mut TestAppContext) {
    refuses_a_non_number_max_body_size(cx, ConfigTarget::Folder(ip("users")));
}

#[gpui_kit::test]
fn collection_max_body_size_refuses_a_non_number(cx: &mut TestAppContext) {
    refuses_a_non_number_max_body_size(cx, ConfigTarget::Collection);
}

#[gpui_kit::test]
fn a_folder_page_is_headed_folder_settings(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    assert_eq!(ed.read_with(cx, |e, cx| e.title(cx)), "Folder Settings");
}

#[gpui_kit::test]
fn the_collection_page_is_headed_collection_settings(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    assert_eq!(ed.read_with(cx, |e, cx| e.title(cx)), "Collection Settings");
}

// Opens `kind`'s settings on `section` with the dock open in a short window, then checks that
// the section body ends above the dock and that the wheel brings `last` into it.
fn reaches_its_last_control(
    cx: &mut TestAppContext,
    kind: apinata_app::tabs::TabKind,
    section: ConfigSection,
    last: &'static str,
) {
    use apinata_app::keymap::ToggleConsole;
    use gpui_kit::{InputEvent as _, ScrollDelta, ScrollEvent, VisualTestContext, point, px, size};
    let col = tree();
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_settings(&s.root, kind, cx))
            .unwrap();
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    ed.update(cx, |e, cx| e.set_section(section, cx));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(360.)));
    frame(cx, s.win);
    let centre = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let dock = w.find("console-dock").bounds();
            let body = w.try_find("config-body").map(|e| e.bounds());
            assert!(
                body.is_some_and(|b| b.bottom() <= dock.top()),
                "{section:?}: body {body:?} past the dock {dock:?}"
            );
            body.unwrap().center()
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_event(
            ScrollEvent {
                position: centre,
                delta: ScrollDelta::Lines(point(0., -1000.)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let body = w.find("config-body").bounds();
        let control = w.find(last).bounds();
        assert!(
            control.bottom() <= body.bottom() && control.top() >= body.top(),
            "{section:?}: {last} {control:?} outside the body {body:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_collection_settings_tab_scrolls_to_the_last_network_control(cx: &mut TestAppContext) {
    http_ready(cx);
    reaches_its_last_control(
        cx,
        apinata_app::tabs::TabKind::Collection,
        ConfigSection::Network,
        "host-add",
    );
}

#[gpui_kit::test]
fn a_short_folder_settings_tab_scrolls_to_its_post_response_vars(cx: &mut TestAppContext) {
    http_ready(cx);
    reaches_its_last_control(
        cx,
        apinata_app::tabs::TabKind::Folder(apinata_core::ItemPath::new("users").unwrap()),
        ConfigSection::Vars,
        "config-post-response-vars",
    );
}

#[gpui_kit::test]
fn folder_ntlm_auth_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    assert!(auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::Ntlm)));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |a, cx| a.set_mode(AuthMode::Ntlm, w, cx))
    })
    .unwrap();
    frame(cx, win);
    for (id, text) in [
        ("auth-username", "fu"),
        ("auth-password", "fp"),
        ("auth-domain", "CORP"),
    ] {
        type_at_end(cx, win, None, id, text);
        cx.run_until_parked();
    }
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Folder(ip("users"))), cx)
    });
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let f = c.folder_file_mut(&ip("users")).unwrap();
    f.set_auth(AuthSpec::Ntlm {
        username: "fu".into(),
        password: "fp".into(),
        domain: "CORP".into(),
    });
    f.save().unwrap();
    assert_eq!(
        bytes(&col, "users/folder.yml"),
        bytes(&twin, "users/folder.yml")
    );
}

#[gpui_kit::test]
fn collection_ntlm_auth_offers_no_inherit_and_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    assert!(auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::Ntlm)));
    assert!(!auth.read_with(cx, |a, _| a.modes().contains(&AuthMode::Inherit)));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |a, cx| a.set_mode(AuthMode::Ntlm, w, cx))
    })
    .unwrap();
    frame(cx, win);
    for (id, text) in [
        ("auth-username", "cu"),
        ("auth-password", "{{pw}}"),
        ("auth-domain", "D"),
    ] {
        type_at_end(cx, win, None, id, text);
        cx.run_until_parked();
    }
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Collection), cx)
    });
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    c.file_mut().set_auth(AuthSpec::Ntlm {
        username: "cu".into(),
        password: "{{pw}}".into(),
        domain: "D".into(),
    });
    c.file_mut().save().unwrap();
    assert_eq!(
        bytes(&col, "opencollection.yml"),
        bytes(&twin, "opencollection.yml")
    );
}

#[gpui_kit::test]
fn a_folders_tags_are_typed_saved_and_shown_again_on_reload(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Tags, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.within("config-tags").click("tag-text", cx);
        w.input(" smoke ,v2,smoke", cx);
        w.press("enter", cx);
    })
    .unwrap();
    frame(cx, win);
    let tags = ed.read_with(cx, |e, cx| e.tags().unwrap().read(cx).tags().to_vec());
    assert_eq!(tags, ["smoke", "v2"]);
    cx.update_window(win, |_, w, cx| {
        w.within("config-tags").click("tag-text", cx);
        w.press(&key("s"), cx);
    })
    .unwrap();
    frame(cx, win);

    // The same edit through core writes the same bytes.
    let mut c = Collection::open(twin.path()).unwrap();
    let f = c.folder_file_mut(&ip("users")).unwrap();
    f.set_tags(vec!["smoke".into(), "v2".into()]);
    f.save().unwrap();
    assert_eq!(
        bytes(&col, "users/folder.yml"),
        bytes(&twin, "users/folder.yml")
    );

    // A fresh editor over the saved file shows the chips.
    let (_state2, ws2, root2) = open_dir(cx, &col);
    let (win2, ed2) = editor(cx, &ws2, &root2, ConfigTarget::Folder(ip("users")));
    frame(cx, win2);
    let shown = ed2.read_with(cx, |e, cx| e.tags().unwrap().read(cx).tags().to_vec());
    assert_eq!(shown, ["smoke", "v2"]);
}

#[gpui_kit::test]
fn the_collection_settings_have_no_tags_section(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    assert!(ed.read_with(cx, |e, _| e.tags().is_none()));
    assert!(!ed.read_with(cx, |e, _| e.sections().contains(&ConfigSection::Tags)));
}

// A vars edit sent after a refused header edit's snapshot redrew, but before its notices
// arrived, leaves the refusal's error under the headers in the next frame.
#[gpui_kit::test]
fn an_edit_sent_before_a_refusal_is_delivered_keeps_it(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    frame(cx, win);
    let acked = move |ws: &Entity<Workspace>, cx: &gpui_kit::App| {
        ws.read(cx).snapshot(&root, cx).map_or(0, |s| s.acked)
    };
    let before = ws.read_with(cx, |_, cx| acked(&ws, cx));
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let (flag, gap_ed) = (fired.clone(), ed.clone());
    let _gap = cx.update(|cx| {
        cx.observe(&ws, move |ws, cx| {
            if !flag.get() && acked(&ws, cx) > before {
                flag.set(true);
                win.update(cx, |_, w, cx| {
                    gap_ed.update(cx, |e, cx| {
                        e.edit_rows(ConfigSection::Vars, vec![row("base", "1")], w, cx)
                    })
                })
                .unwrap();
            }
        })
    });
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row("a\nb", "1")], w, cx)
        })
    })
    .unwrap();
    frame(cx, win);
    assert!(fired.get(), "the vars edit went out in the gap");
    assert!(
        present(cx, win, "kv-error"),
        "the refused header's error is shown"
    );
}

#[gpui_kit::test]
fn save_shows_while_the_settings_are_unsaved_and_saves_them(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    assert!(!present(cx, win, "config-save-button"));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row("X-Trace", "1")], w, cx)
        })
    })
    .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "config-save-button"));
    cx.update_window(win, |_, w, cx| w.click("config-save-button", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    let saved = String::from_utf8(bytes(&col, "users/folder.yml")).unwrap();
    assert!(saved.contains("X-Trace"), "{saved}");
    assert!(!present(cx, win, "config-save-button"));
}

#[gpui_kit::test]
fn the_folder_tags_note_starts_where_the_tag_input_does(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Tags, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let input = w.find("tag-input").bounds();
        let note = w.find("config-tags-note").bounds();
        assert_eq!(note.left(), input.left(), "{note:?} {input:?}");
        assert!(note.top() >= input.bottom());
        let label = w.within("config-row-tags").find("form-label").bounds();
        assert!(label.right() <= input.left(), "{label:?} {input:?}");
    })
    .unwrap();
}

// The Inherit line of the folder's Auth section, read from its accessible label.
fn folder_inherit_text(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("auth-inherit").label().map(str::to_owned)
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_inheriting_folder_names_the_auth_in_effect(cx: &mut TestAppContext) {
    let users = format!("{}request:\n  auth: inherit\n", folder("Users"));
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &users),
        ("users/a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    frame(cx, win);
    assert_eq!(
        folder_inherit_text(cx, win).as_deref(),
        Some("Inherits No Auth from the collection.")
    );
    // An unsaved change to the collection's auth shows at once.
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Collection,
                edit: apinata_app::command::Edit::Auth(AuthSpec::Bearer { token: "t".into() }),
            },
            cx,
        )
    });
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(
        folder_inherit_text(cx, win).as_deref(),
        Some("Inherits Bearer Token from the collection.")
    );
}

#[gpui_kit::test]
fn the_post_response_placeholder_reads_as_an_example(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    let (_, value) = ed.read_with(cx, |e, cx| e.post_response_table().read(cx).placeholders());
    assert!(value.starts_with("e.g. "), "{value}");
}

#[gpui_kit::test]
fn an_inheriting_folder_names_the_nearest_folder_with_auth(cx: &mut TestAppContext) {
    let root = format!("{ROOT}request:\n  auth:\n    type: bearer\n    token: t\n");
    let users = format!(
        "{}request:\n  auth:\n    type: basic\n    username: u\n    password: p\n",
        folder("Users")
    );
    let admin = format!("{}request:\n  auth: inherit\n", folder("Admin"));
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("users/folder.yml", &users),
        ("users/admin/folder.yml", &admin),
        ("users/admin/a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users/admin")));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
    frame(cx, win);
    assert_eq!(
        folder_inherit_text(cx, win).as_deref(),
        Some("Inherits Basic Auth from folder \"Users\".")
    );
}

#[gpui_kit::test]
fn autosave_shows_no_save_on_the_settings_page(cx: &mut TestAppContext) {
    let col = tree();
    let s = started_with(cx, &col, |p| {
        let mut settings = test_settings();
        settings.autosave = true;
        apinata_app::persist::save_json(&p.settings, &settings).unwrap();
    });
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, apinata_app::tabs::TabKind::Collection, cx)
        })
        .unwrap();
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row("X-Trace", "1")], w, cx)
        })
    })
    .unwrap();
    frame(cx, s.win);
    let dirty = s.ws.read_with(cx, |w, cx| {
        w.snapshot(&s.root, cx)
            .is_some_and(|s| s.content.collection.dirty)
    });
    assert!(dirty, "the edit is still unsaved");
    assert!(!present(cx, s.win, "config-save"));
    assert!(!present(cx, s.win, "config-save-button"));
}
