//! An inheriting request names the auth it inherits and the level it comes from.

use crate::support;

use apinata_app::request_editor::Section;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const INHERITS: &str = "  auth: inherit\n";

/// The Auth section's inherit text for `rel` in `col`, read from its accessible label.
fn inherit_text(cx: &mut TestAppContext, col: &tempfile::TempDir, rel: &str) -> Option<String> {
    let (_state, ws, root) = open_dir(cx, col);
    let (win, _view) = editor_for(cx, &ws, &root, rel, false);
    shown_text(cx, win)
}

/// The inherit text of the editor in `win`, after opening its Auth section.
fn shown_text(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("auth-inherit").label().map(str::to_owned)
    })
    .unwrap()
}

#[gpui_kit::test]
fn inherit_names_the_folder_or_collection_auth(cx: &mut TestAppContext) {
    let root = format!("{ROOT}request:\n  auth:\n    type: bearer\n    token: t\n");
    let users = format!(
        "{}request:\n  auth:\n    type: basic\n    username: u\n    password: p\n",
        folder("users")
    );
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("u/folder.yml", &users),
        (
            "u/list.yml",
            &format!("{}{INHERITS}", request("list", "http://x/u")),
        ),
        (
            "top.yml",
            &format!("{}{INHERITS}", request("top", "http://x/t")),
        ),
    ]);
    assert_eq!(
        inherit_text(cx, &col, "u/list.yml").as_deref(),
        Some("Inherits Basic Auth from folder \"users\".")
    );
    assert_eq!(
        inherit_text(cx, &col, "top.yml").as_deref(),
        Some("Inherits Bearer Token from the collection.")
    );
}

#[gpui_kit::test]
fn inherit_from_a_collection_without_auth_says_no_auth(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &format!("{}{INHERITS}", request("a", "http://x/a")),
        ),
    ]);
    assert_eq!(
        inherit_text(cx, &col, "a.yml").as_deref(),
        Some("Inherits No Auth from the collection.")
    );
}

#[gpui_kit::test]
fn inherit_skips_an_inheriting_folder_and_names_the_outer_one(cx: &mut TestAppContext) {
    let users = format!(
        "{}request:\n  auth:\n    type: apikey\n    key: k\n    value: v\n",
        folder("users")
    );
    let admins = format!("{}request:\n{INHERITS}", folder("admins"));
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("u/folder.yml", &users),
        ("u/a/folder.yml", &admins),
        (
            "u/a/list.yml",
            &format!("{}{INHERITS}", request("list", "http://x/a")),
        ),
    ]);
    assert_eq!(
        inherit_text(cx, &col, "u/a/list.yml").as_deref(),
        Some("Inherits API Key from folder \"users\".")
    );
}

#[gpui_kit::test]
fn inherit_text_follows_an_in_app_edit_of_the_folder_or_collection(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, Edit, Target};
    use apinata_core::AuthSpec;
    let users = format!(
        "{}request:\n  auth:\n    type: basic\n    username: u\n    password: p\n",
        folder("users")
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("u/folder.yml", &users),
        (
            "u/list.yml",
            &format!("{}{INHERITS}", request("list", "http://x/u")),
        ),
        (
            "top.yml",
            &format!("{}{INHERITS}", request("top", "http://x/t")),
        ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (u_win, _u) = editor_for(cx, &ws, &root, "u/list.yml", false);
    let (top_win, _top) = editor_for(cx, &ws, &root, "top.yml", false);
    let bearer = || Edit::Auth(AuthSpec::Bearer { token: "t".into() });
    let folder = Target::Folder(ip("u"));
    let send = |cx: &mut TestAppContext, command: Command| {
        ws.update(cx, |w, cx| w.send(&root, command, cx));
        cx.run_until_parked();
    };

    send(
        cx,
        Command::Edit {
            target: folder.clone(),
            edit: bearer(),
        },
    );
    assert_eq!(
        shown_text(cx, u_win).as_deref(),
        Some("Inherits Bearer Token from folder \"users\".")
    );
    send(
        cx,
        Command::Edit {
            target: folder.clone(),
            edit: Edit::Name("people".into()),
        },
    );
    assert_eq!(
        shown_text(cx, u_win).as_deref(),
        Some("Inherits Bearer Token from folder \"people\".")
    );
    send(cx, Command::Revert(folder));
    assert_eq!(
        shown_text(cx, u_win).as_deref(),
        Some("Inherits Basic Auth from folder \"users\".")
    );
    send(
        cx,
        Command::Edit {
            target: Target::Collection,
            edit: bearer(),
        },
    );
    assert_eq!(
        shown_text(cx, top_win).as_deref(),
        Some("Inherits Bearer Token from the collection.")
    );
}

#[gpui_kit::test]
fn inherit_names_ntlm_at_folder_and_collection_level(cx: &mut TestAppContext) {
    let ntlm = "  auth:\n    type: ntlm\n    username: u\n    password: p\n    domain: CORP\n";
    let users = format!("{}request:\n{ntlm}", folder("users"));
    let root = format!("{ROOT}request:\n{ntlm}");
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("u/folder.yml", &users),
        (
            "u/list.yml",
            &format!("{}{INHERITS}", request("list", "http://x/u")),
        ),
        (
            "top.yml",
            &format!("{}{INHERITS}", request("top", "http://x/t")),
        ),
    ]);
    assert_eq!(
        inherit_text(cx, &col, "u/list.yml").as_deref(),
        Some("Inherits NTLM from folder \"users\".")
    );
    assert_eq!(
        inherit_text(cx, &col, "top.yml").as_deref(),
        Some("Inherits NTLM from the collection.")
    );
}
