use crate::support;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::draft_file;
use apinata_app::draft_store::Drafts;
use apinata_app::workspace::Workspace;
use apinata_core::{AuthSpec, Collection, ItemPath, MemoryStore, SecretStore};
use gpui_kit::{App, Entity, TestAppContext};
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: token\n";
const FOLDER: &str = "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  variables:\n    - name: token\n      value: plain-folder-value\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("f/folder.yml", FOLDER),
        ("environments/dev.yml", DEV),
        (".env", "DOTENV_KEY=dotenv-value-77\n"),
    ])
}

#[gpui_kit::test]
fn a_typed_draft_is_written_within_the_save_delay_and_an_untouched_one_never(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let s = started(cx, &col);
    let (_idle, idle_key) = new_draft_at(cx, &s, ItemPath::root());
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    cx.run_until_parked();
    assert!(!draft_path(&s, &hk).exists(), "debounced");
    wait_save(cx);
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert_eq!(loaded.drafts.len(), 1, "{:?}", loaded.skipped);
    let file = &loaded.drafts[0];
    assert_eq!(
        (file.key.as_str(), file.root.as_path(), file.parent.as_str()),
        (hk.as_str(), s.root.as_path(), "")
    );
    assert!(
        file.request.contains("http://127.0.0.1:9/second"),
        "{}",
        file.request
    );
    assert!(!draft_path(&s, &idle_key).exists());
}

#[gpui_kit::test]
fn the_file_holds_no_secret_even_shadowed_or_with_an_undefined_variable(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let c = Collection::open(col.path()).unwrap();
    store
        .set(&c.secret_key("dev", "token"), "env-token-value-1")
        .unwrap();
    let s = started_with_store(cx, &col, store);
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let (item, hk) = new_draft_at(cx, &s, ip("f"));
    let url = "http://127.0.0.1:9/{{undefined_var}}/env-token-value-1/dotenv-value-77";
    edit_item(cx, &s, &item, Edit::Url(url.into()));
    edit_item(
        cx,
        &s,
        &item,
        Edit::Auth(AuthSpec::Bearer {
            token: "literal-bearer-555".into(),
        }),
    );
    wait_save(cx);
    let text = std::fs::read_to_string(draft_path(&s, &hk)).expect("written");
    for secret in ["env-token-value-1", "dotenv-value-77", "literal-bearer-555"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert!(loaded.drafts[0].credentials_blanked);
    assert!(loaded.drafts[0].request.contains("{{undefined_var}}"));
}

#[gpui_kit::test]
fn typing_a_literal_token_after_the_first_write_marks_the_file_blanked(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    let bearer = |token: &str| {
        Edit::Auth(AuthSpec::Bearer {
            token: token.into(),
        })
    };
    edit_item(cx, &s, &item, bearer(""));
    wait_save(cx);
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert!(
        !loaded.drafts[0].credentials_blanked,
        "nothing to blank yet"
    );
    edit_item(cx, &s, &item, bearer("literal-bearer-777"));
    cx.run_until_parked();
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    let text = std::fs::read_to_string(draft_path(&s, &hk)).expect("written");
    assert!(!text.contains("literal-bearer-777"), "leaked: {text}");
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert!(loaded.drafts[0].credentials_blanked, "{text}");
}

#[gpui_kit::test]
fn a_root_whose_context_fails_settles_and_retries_only_on_a_change(cx: &mut TestAppContext) {
    let col = two_requests();
    std::fs::create_dir(col.path().join(".env")).unwrap();
    let s = started(cx, &col);
    let store = cx.update(|cx| cx.global::<Drafts>().0.clone());
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save(cx);
    let fetches = store.read_with(cx, |d, _| d.fetches());
    assert!(fetches >= 1);
    assert!(!draft_path(&s, &hk).exists(), "no list, no file");
    edit_item(
        cx,
        &s,
        "a.yml",
        Edit::Url("http://127.0.0.1:9/saved".into()),
    );
    wait_save(cx);
    assert_eq!(
        store.read_with(cx, |d, _| d.fetches()),
        fetches,
        "no retry without a change"
    );
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/y".into()));
    wait_save(cx);
    assert_eq!(
        store.read_with(cx, |d, _| d.fetches()),
        fetches + 1,
        "a changed draft retries once"
    );
}

#[gpui_kit::test]
fn a_second_instance_writes_nothing(cx: &mut TestAppContext) {
    let col = tree();
    let held = RefCell::new(None);
    let s = started_with(cx, &col, |paths| {
        *held.borrow_mut() = draft_file::lock(&paths.drafts)
    });
    assert!(held.borrow().is_some());
    let (item, _) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save(cx);
    let names: Vec<_> = std::fs::read_dir(s.files.path().join("drafts"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(".lock")]);
}

// A store with the dev environment's token set, the collection open, and a written draft built
// with the list of the context before any environment is selected.
fn written_draft(cx: &mut TestAppContext, col: &tempfile::TempDir) -> (Started, String, ItemPath) {
    let store = Arc::new(MemoryStore::new());
    let c = Collection::open(col.path()).unwrap();
    store
        .set(&c.secret_key("dev", "token"), "env-token-value-1")
        .unwrap();
    let s = started_with_store(cx, col, store);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/first".into()));
    wait_save(cx);
    assert!(draft_path(&s, &hk).exists());
    (s, item, hk)
}

fn generation(ws: &Entity<Workspace>, root: &Path, cx: &App) -> u64 {
    ws.read(cx).snapshot(root, cx).unwrap().context_generation
}

#[gpui_kit::test]
fn a_quit_before_the_new_contexts_list_arrives_writes_no_secret(cx: &mut TestAppContext) {
    let col = tree();
    let (s, item, hk) = written_draft(cx, &col);
    let before = cx.update(|cx| generation(&s.ws, &s.root, cx));
    // The moment the switched context's snapshot lands, the store has asked for its list and
    // the window closes, so the only list the store holds is the one from before the switch.
    let seen = Rc::new(RefCell::new(None));
    let (root, file, out) = (s.root.clone(), draft_path(&s, &hk), seen.clone());
    let listed = Target::Request(ip(&item));
    cx.update(|cx| {
        cx.observe(&s.ws, move |ws, cx| {
            if out.borrow().is_none() && generation(&ws, &root, cx) != before {
                apinata_app::draft_store::flush_now(None, cx);
                let unsaved = ws.read(cx).unsaved(None, cx);
                let prompts = unsaved.iter().any(|u| u.targets.contains(&listed));
                *out.borrow_mut() = Some((std::fs::read_to_string(&file).ok(), prompts));
            }
        })
        .detach()
    });
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    edit_item(
        cx,
        &s,
        &item,
        Edit::Url("http://127.0.0.1:9/env-token-value-1".into()),
    );
    cx.run_until_parked();
    let (text, prompts) = seen.borrow().clone().expect("the switch landed");
    let text = text.unwrap_or_default();
    assert!(
        !text.contains("env-token-value-1"),
        "leaked at quit: {text}"
    );
    assert!(prompts, "a draft not written at quit is unkept and prompts");
}

#[gpui_kit::test]
fn a_secret_pasted_after_a_switch_while_a_list_is_in_flight_is_scrubbed(cx: &mut TestAppContext) {
    let col = tree();
    let (s, item, hk) = written_draft(cx, &col);
    let drafts = cx.update(|cx| cx.global::<Drafts>().0.clone());
    let asked = drafts.read_with(cx, |d, _| d.fetches());
    // Right after the store asks for the list of the context before the switch, the
    // environment changes and its secret is pasted, all before that list arrives.
    let (root, target, done) = (s.root.clone(), ip(&item), Rc::new(RefCell::new(false)));
    let flag = done.clone();
    cx.update(|cx| {
        cx.observe(&s.ws, move |ws, cx| {
            if !*flag.borrow() && drafts.read(cx).fetches() > asked {
                *flag.borrow_mut() = true;
                let url = Edit::Url("http://127.0.0.1:9/env-token-value-1".into());
                ws.update(cx, |w, cx| {
                    w.select_environment(&root, Some("dev".into()), cx);
                    let edit = Command::Edit {
                        target: Target::Request(target.clone()),
                        edit: url,
                    };
                    w.send(&root, edit, cx);
                });
            }
        })
        .detach()
    });
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/second".into()));
    wait_save(cx);
    assert!(
        *done.borrow(),
        "the switch was sent while the list was in flight"
    );
    let text = std::fs::read_to_string(draft_path(&s, &hk)).expect("written");
    assert!(text.contains("[redacted]"), "{text}");
    assert!(!text.contains("env-token-value-1"), "leaked: {text}");
}

#[gpui_kit::test]
fn a_draft_is_kept_only_as_the_credentials_flag_it_was_written_with(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(
        cx,
        &s,
        &item,
        Edit::Auth(AuthSpec::Bearer {
            token: String::new(),
        }),
    );
    wait_save(cx);
    // A literal credential typed now leaves the blanked snapshot as it is, but the file would
    // have to say credentials_blanked, so the written file no longer keeps the draft.
    let (as_written, with_literal) = s.ws.read_with(cx, |w, cx| {
        let view = &w.snapshot(&s.root, cx).expect("snapshot").content.requests[&ip(&item)];
        let snap = view.snapshot.as_ref().expect("a draft has a snapshot");
        (
            apinata_app::draft_store::is_kept(&hk, snap, false, cx),
            apinata_app::draft_store::is_kept(&hk, snap, true, cx),
        )
    });
    assert!(as_written, "the file matches the draft as it is");
    assert!(!with_literal, "the file lacks the blanked flag");
}
