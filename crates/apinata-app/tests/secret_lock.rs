//! The secret lock asks before it loses a stored value, in both environment editors.

use crate::env_orphans::{Listing, click_lock, global_editor_with_ws, seeded};
use crate::support;

use apinata_app::command::{Command, Target};
use apinata_app::env_editor::EnvEditor;
use apinata_app::global_env_editor::GlobalEnvEditor;
use apinata_app::secret_lock::{LockQuestion, question};
use apinata_app::workspace::Workspace;
use apinata_core::{Collection, GlobalEnvironments, SecretKey, SecretStore};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use support::*;

/// `old` is plain and differs from the stored value.
const PLAIN: &str = "variables:\n  - name: host\n    value: a\n  - name: old\n    value: y\n";
/// `old` is plain and empty.
const EMPTY: &str = "variables:\n  - name: host\n    value: a\n  - name: old\n    value: ''\n";
/// `old` is plain and equal to the stored value.
const SAME: &str = "variables:\n  - name: host\n    value: a\n  - name: old\n    value: s3cret\n";
/// `old` is secret.
const SECRET: &str = "variables:\n  - name: host\n    value: a\n  - name: old\n    secret: true\n";

const REPLACE: &str = "confirm-replace";
const MAKE_PLAIN: &str = "confirm-make-plain";
const CANCEL: &str = "confirm-cancel";

#[derive(Clone, Copy)]
enum Kind {
    Collection,
    Global,
}

#[derive(Clone)]
enum Ed {
    Collection(Entity<EnvEditor>),
    Global(Entity<GlobalEnvEditor>),
}

/// One editor over a file whose `old` has the stored value `s3cret`.
struct Fix {
    _dirs: Vec<tempfile::TempDir>,
    ws: Entity<Workspace>,
    outside: Option<(Entity<Workspace>, PathBuf)>,
    win: AnyWindowHandle,
    ed: Ed,
    store: Arc<Listing>,
    key: SecretKey,
    two: SecretKey,
    file: PathBuf,
    other: &'static str,
}

fn fix(cx: &mut TestAppContext, kind: Kind, vars: &str) -> Fix {
    match kind {
        Kind::Collection => {
            let file = format!("name: dev\n{vars}");
            let col = write_tree(&[
                ("opencollection.yml", ROOT),
                ("environments/dev.yml", &file),
                ("environments/prod.yml", "name: prod\n"),
            ]);
            let (store, key) = seeded(&col);
            let (state, ws, root) = open_dir_with(cx, &col, store.clone());
            let shared = ws.clone();
            let (win, ed) = env_editor(cx, &ws, &root);
            cx.run_until_parked();
            let file = col.path().join("environments/dev.yml");
            let two = Collection::open(col.path())
                .unwrap()
                .secret_key("dev", "two");
            Fix {
                outside: Some((ws, root)),
                ws: shared,
                _dirs: vec![col, state],
                win,
                ed: Ed::Collection(ed),
                store,
                key,
                two,
                file,
                other: "prod",
            }
        }
        Kind::Global => {
            let store = Arc::new(Listing::default());
            let g = format!("name: g\n{vars}");
            let (col, globals, state, win, ed, key, ws) =
                global_editor_with_ws(cx, store.clone(), &g);
            let file = globals.path().join("g.yml");
            let two = GlobalEnvironments::open(globals.path())
                .unwrap()
                .secret_key("g", "two");
            Fix {
                outside: None,
                ws,
                _dirs: vec![col, globals, state],
                win,
                ed: Ed::Global(ed),
                store,
                key,
                two,
                file,
                other: "h",
            }
        }
    }
}

impl Fix {
    fn held(&self) -> Option<String> {
        self.store.get(&self.key).unwrap()
    }

    fn bytes(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap()
    }

    fn choose_other(&self, cx: &mut TestAppContext) {
        let other = self.other;
        cx.update_window(self.win, |_, w, cx| match &self.ed {
            Ed::Collection(e) => e.update(cx, |e, cx| e.choose(other, w, cx)),
            Ed::Global(e) => e.update(cx, |e, cx| e.choose(other, w, cx)),
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn save_button(&self) -> &'static str {
        match self.ed {
            Ed::Collection(_) => "env-save",
            Ed::Global(_) => "global-env-save",
        }
    }

    /// Adds a new secret row `name` holding `value` at row `ix`.
    fn draft(&self, cx: &mut TestAppContext, ix: usize, name: &str, value: &str) {
        frame(cx, self.win);
        type_at_end(cx, self.win, None, ("env-name", ix), name);
        cx.run_until_parked();
        click_lock(cx, self.win, ix);
        frame(cx, self.win);
        type_at_end(cx, self.win, None, ("env-value", ix), value);
        cx.run_until_parked();
    }

    /// Saves the file as autosave does, past every prompt.
    fn save_from_outside(&self, cx: &mut TestAppContext) {
        let (ws, root) = self.outside.as_ref().expect("a collection");
        let command = Command::Save(Target::Environment("dev".into()));
        ws.update(cx, |w, cx| w.send(root, command, cx));
        cx.run_until_parked();
        frame(cx, self.win);
    }

    /// A new secret row typed in, then declared behind the table's back, each editor's way.
    fn hold(&self, cx: &mut TestAppContext) {
        self.draft(cx, 1, "old", "typed");
        match self.ed {
            Ed::Collection(_) => self.save_from_outside(cx),
            Ed::Global(_) => {
                std::fs::write(&self.file, format!("name: g\n{SECRET}")).unwrap();
                self.ws.update(cx, |w, cx| w.reload_globals(cx));
                cx.run_until_parked();
                frame(cx, self.win);
            }
        }
    }

    fn enter(&self, cx: &mut TestAppContext) {
        frame(cx, self.win);
        cx.update_window(self.win, |_, w, cx| {
            w.click(("env-value", 1usize), cx);
            w.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn selected(&self, cx: &mut TestAppContext) -> Option<String> {
        match &self.ed {
            Ed::Collection(e) => e.read_with(cx, |e, _| e.selected().map(str::to_owned)),
            Ed::Global(e) => e.read_with(cx, |e, _| e.selected().map(str::to_owned)),
        }
    }

    fn click_save(&self, cx: &mut TestAppContext) {
        frame(cx, self.win);
        cx.update_window(self.win, |_, w, cx| w.click(self.save_button(), cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn typing(&self, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| match &self.ed {
            Ed::Collection(e) => e.read(cx).table().read(cx).has_secret_typing(cx),
            Ed::Global(e) => e.read(cx).table().read(cx).has_secret_typing(cx),
        })
    }

    fn shows_error(&self, cx: &mut TestAppContext) -> bool {
        match &self.ed {
            Ed::Collection(e) => e.read_with(cx, |e, _| e.error().is_some()),
            Ed::Global(_) => present(cx, self.win, "global-env-error"),
        }
    }
}

fn title(cx: &mut TestAppContext) -> Option<String> {
    let win = confirm_window(cx).expect("a prompt");
    cx.update_window(win, |_, w, _| {
        w.find("confirm-dialog").label().map(str::to_owned)
    })
    .unwrap()
}

fn press_on_prompt(cx: &mut TestAppContext, key: &str) {
    let win = confirm_window(cx).expect("a prompt");
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
}

/// Clicks the lock of `old` twice with no pause, so the second lands during the first's check.
fn double_click(cx: &mut TestAppContext, win: AnyWindowHandle) {
    frame(cx, win);
    for _ in 0..2 {
        cx.update_window(win, |_, w, cx| w.click(("env-secret", 1usize), cx))
            .unwrap();
    }
    cx.run_until_parked();
}

// One test body run against each editor.
macro_rules! both {
    ($body:ident, $col:ident, $global:ident) => {
        #[gpui_kit::test]
        fn $col(cx: &mut TestAppContext) {
            $body(cx, Kind::Collection);
        }
        #[gpui_kit::test]
        fn $global(cx: &mut TestAppContext) {
            $body(cx, Kind::Global);
        }
    };
}

#[test]
fn the_rule_asks_only_when_a_stored_value_would_go() {
    use LockQuestion::{Move, Replace};
    let cases = [
        (true, "new", Some("old"), Some(Replace)),
        (true, "same", Some("same"), None),
        (true, "", Some("old"), None),
        (true, "new", Some(""), None),
        (true, "new", None, None),
        (false, "", Some("old"), Some(Move)),
        (false, "typed", Some("old"), Some(Move)),
        (false, "", Some(""), None),
        (false, "", None, None),
    ];
    for (secret, plain, stored, want) in cases {
        assert_eq!(question(secret, plain, stored), want, "{cases:?}");
    }
}

fn replace_asks_and_cancel_keeps(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, PLAIN);
    let before = f.bytes();
    click_lock(cx, f.win, 1);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert_eq!(f.bytes(), before);
}
both!(
    replace_asks_and_cancel_keeps,
    replace_asks_and_cancel_keeps_in_a_collection,
    replace_asks_and_cancel_keeps_in_a_global
);

fn confirming_replaces(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, PLAIN);
    click_lock(cx, f.win, 1);
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("y"));
    assert!(f.bytes().contains("secret: true"), "{}", f.bytes());
}
both!(
    confirming_replaces,
    confirming_replaces_in_a_collection,
    confirming_replaces_in_a_global
);

fn move_asks_and_cancel_keeps(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    let before = f.bytes();
    click_lock(cx, f.win, 1);
    assert_eq!(
        title(cx).as_deref(),
        Some("Move the stored value of \"old\" to the file?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert_eq!(f.bytes(), before);
}
both!(
    move_asks_and_cancel_keeps,
    move_asks_and_cancel_keeps_in_a_collection,
    move_asks_and_cancel_keeps_in_a_global
);

fn confirming_moves(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    click_lock(cx, f.win, 1);
    answer_confirm(cx, MAKE_PLAIN);
    assert_eq!(f.held(), None);
    assert!(f.bytes().contains("s3cret"), "{}", f.bytes());
}
both!(
    confirming_moves,
    confirming_moves_in_a_collection,
    confirming_moves_in_a_global
);

fn enter_and_escape_do_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    let before = f.bytes();
    for key in ["enter", "escape"] {
        click_lock(cx, f.win, 1);
        assert!(confirm_open(cx), "{key}");
        press_on_prompt(cx, key);
        assert!(!confirm_open(cx), "{key} answers");
        assert_eq!(f.held().as_deref(), Some("s3cret"), "{key}");
        assert_eq!(f.bytes(), before, "{key}");
    }
}
both!(
    enter_and_escape_do_nothing,
    enter_and_escape_do_nothing_in_a_collection,
    enter_and_escape_do_nothing_in_a_global
);

fn nothing_stored_asks_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, PLAIN);
    f.store.delete(&f.key).unwrap();
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
    assert_eq!(f.held().as_deref(), Some("y"));
    let f = fix(cx, kind, SECRET);
    f.store.delete(&f.key).unwrap();
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
    assert!(!f.bytes().contains("secret: true"), "{}", f.bytes());
}
both!(
    nothing_stored_asks_nothing,
    nothing_stored_asks_nothing_in_a_collection,
    nothing_stored_asks_nothing_in_a_global
);

fn a_stored_equal_to_the_text_asks_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SAME);
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
    assert!(f.bytes().contains("secret: true"), "{}", f.bytes());
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}
both!(
    a_stored_equal_to_the_text_asks_nothing,
    a_stored_equal_to_the_text_asks_nothing_in_a_collection,
    a_stored_equal_to_the_text_asks_nothing_in_a_global
);

fn empty_text_keeps_the_stored_value_unasked(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, EMPTY);
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
    assert!(f.bytes().contains("secret: true"), "{}", f.bytes());
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}
both!(
    empty_text_keeps_the_stored_value_unasked,
    empty_text_keeps_the_stored_value_unasked_in_a_collection,
    empty_text_keeps_the_stored_value_unasked_in_a_global
);

fn a_failed_read_changes_nothing_and_says_so(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    let before = f.bytes();
    f.store.fail_get.store(true, Ordering::SeqCst);
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
    assert!(f.shows_error(cx));
    assert_eq!(f.bytes(), before);
    f.store.fail_get.store(false, Ordering::SeqCst);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    click_lock(cx, f.win, 1);
    assert!(confirm_open(cx), "a failed check holds no later click");
}
both!(
    a_failed_read_changes_nothing_and_says_so,
    a_failed_read_changes_nothing_in_a_collection,
    a_failed_read_changes_nothing_in_a_global
);

fn a_second_click_gives_one_prompt(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    double_click(cx, f.win);
    assert!(confirm_open(cx));
    answer_confirm(cx, CANCEL);
    assert!(!confirm_open(cx), "the second click stacked a prompt");
    click_lock(cx, f.win, 1);
    assert!(confirm_open(cx), "an answered check holds no later click");
}
both!(
    a_second_click_gives_one_prompt,
    a_second_click_gives_one_prompt_in_a_collection,
    a_second_click_gives_one_prompt_in_a_global
);

fn a_moved_selection_changes_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    let before = f.bytes();
    click_lock(cx, f.win, 1);
    f.choose_other(cx);
    answer_confirm(cx, MAKE_PLAIN);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert_eq!(f.bytes(), before);
}
both!(
    a_moved_selection_changes_nothing,
    a_moved_selection_changes_nothing_in_a_collection,
    a_moved_selection_changes_nothing_in_a_global
);

fn an_unanswered_dialog_changes_nothing(cx: &mut TestAppContext, kind: Kind) {
    use gpui_kit::component::WindowExt as _;
    let f = fix(cx, kind, SECRET);
    let before = f.bytes();
    click_lock(cx, f.win, 1);
    let win = confirm_window(cx).expect("a prompt");
    cx.update_window(win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert_eq!(f.bytes(), before);
    if !confirm_open(cx) {
        click_lock(cx, f.win, 1);
        assert!(confirm_open(cx), "a dropped dialog holds no later click");
    }
}
both!(
    an_unanswered_dialog_changes_nothing,
    an_unanswered_dialog_changes_nothing_in_a_collection,
    an_unanswered_dialog_changes_nothing_in_a_global
);

fn cancel_keeps_typing_out_of_the_store(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, SECRET);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(f.win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx), "the typing stays in the input");
}
both!(
    cancel_keeps_typing_out_of_the_store,
    cancel_keeps_typing_out_of_the_store_in_a_collection,
    cancel_keeps_typing_out_of_the_store_in_a_global
);

#[gpui_kit::test]
fn confirming_sends_the_typing_to_the_file_in_a_collection(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, SECRET);
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    answer_confirm(cx, MAKE_PLAIN);
    assert!(f.bytes().contains("typed"), "{}", f.bytes());
    assert!(!f.bytes().contains("s3cret"), "{}", f.bytes());
    assert_eq!(f.held(), None);
}

#[gpui_kit::test]
fn typing_over_nothing_stored_reaches_the_file_unasked(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, SECRET);
    f.store.delete(&f.key).unwrap();
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
    assert!(f.bytes().contains("typed"), "{}", f.bytes());
}

#[gpui_kit::test]
fn confirming_discards_the_typing_and_moves_the_stored_value_in_a_global(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, SECRET);
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    answer_confirm(cx, MAKE_PLAIN);
    assert!(f.bytes().contains("s3cret"), "{}", f.bytes());
    assert!(!f.bytes().contains("typed"), "{}", f.bytes());
    assert_eq!(f.held(), None);
}

#[gpui_kit::test]
fn a_cancelled_global_lock_does_not_save_the_draft(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, PLAIN);
    let before = f.bytes();
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 0usize), "zz");
    type_at_end(cx, f.win, None, ("env-value", 1usize), "zz");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    assert!(confirm_open(cx));
    answer_confirm(cx, CANCEL);
    assert_eq!(f.bytes(), before);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn a_global_lock_compares_the_drafts_text_and_saves_it_once_confirmed(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, SAME);
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 1usize), "2");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    assert!(confirm_open(cx), "the draft differs from what is stored");
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("s3cret2"));
}

#[gpui_kit::test]
fn a_dirtied_collection_during_the_prompt_changes_nothing(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, SECRET);
    let Ed::Collection(ed) = f.ed.clone() else {
        unreachable!()
    };
    click_lock(cx, f.win, 1);
    assert!(confirm_open(cx));
    ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    answer_confirm(cx, MAKE_PLAIN);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn a_dirty_collection_is_not_asked_about_the_lock(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, SECRET);
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    click_lock(cx, f.win, 1);
    assert!(!confirm_open(cx));
}

/// Only `host`, so row 1 is free for a new row named like a stored value.
const HOST: &str = "variables:\n  - name: host\n    value: a\n";

fn save_asks_and_cancel_keeps(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    let before = f.bytes();
    f.draft(cx, 1, "old", "typed");
    f.click_save(cx);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert_eq!(f.bytes(), before);
    assert!(f.typing(cx), "the typing stays in the input");
}
both!(
    save_asks_and_cancel_keeps,
    save_asks_and_cancel_keeps_in_a_collection,
    save_asks_and_cancel_keeps_in_a_global
);

fn save_keys_on_the_prompt_do_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    let before = f.bytes();
    f.draft(cx, 1, "old", "typed");
    for key in ["enter", "escape"] {
        f.click_save(cx);
        assert!(confirm_open(cx), "{key}");
        press_on_prompt(cx, key);
        assert_eq!(f.held().as_deref(), Some("s3cret"), "{key}");
        assert_eq!(f.bytes(), before, "{key}");
        assert!(f.typing(cx), "{key}");
    }
}
both!(
    save_keys_on_the_prompt_do_nothing,
    save_keys_on_the_prompt_do_nothing_in_a_collection,
    save_keys_on_the_prompt_do_nothing_in_a_global
);

fn confirming_a_save_replaces(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.draft(cx, 1, "old", "typed");
    f.click_save(cx);
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("typed"));
    assert!(f.bytes().contains("old"), "{}", f.bytes());
}
both!(
    confirming_a_save_replaces,
    confirming_a_save_replaces_in_a_collection,
    confirming_a_save_replaces_in_a_global
);

fn a_save_with_nothing_to_lose_asks_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.draft(cx, 1, "old", "s3cret");
    f.click_save(cx);
    assert!(!confirm_open(cx), "equal to what is stored");
    assert!(f.bytes().contains("old"), "{}", f.bytes());
    let f = fix(cx, kind, HOST);
    f.store.delete(&f.key).unwrap();
    f.draft(cx, 1, "old", "typed");
    f.click_save(cx);
    assert!(!confirm_open(cx), "nothing stored");
    assert_eq!(f.held().as_deref(), Some("typed"));
}
both!(
    a_save_with_nothing_to_lose_asks_nothing,
    a_save_with_nothing_to_lose_asks_nothing_in_a_collection,
    a_save_with_nothing_to_lose_asks_nothing_in_a_global
);

fn two_names_share_one_prompt(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.store.set(&f.two, "second").unwrap();
    f.draft(cx, 1, "old", "typed");
    f.draft(cx, 2, "two", "other");
    f.click_save(cx);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored values of \"old\", \"two\"?")
    );
    answer_confirm(cx, CANCEL);
    assert!(!confirm_open(cx), "one prompt, not a chain");
    assert_eq!(f.store.get(&f.two).unwrap().as_deref(), Some("second"));
    f.click_save(cx);
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("typed"));
    assert_eq!(f.store.get(&f.two).unwrap().as_deref(), Some("other"));
}
both!(
    two_names_share_one_prompt,
    two_names_share_one_prompt_in_a_collection,
    two_names_share_one_prompt_in_a_global
);

fn a_failed_read_saves_nothing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    let before = f.bytes();
    f.draft(cx, 1, "old", "typed");
    f.store.fail_get.store(true, Ordering::SeqCst);
    f.click_save(cx);
    assert!(!confirm_open(cx));
    assert!(f.shows_error(cx));
    assert_eq!(f.bytes(), before);
    f.store.fail_get.store(false, Ordering::SeqCst);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx));
}
both!(
    a_failed_read_saves_nothing,
    a_failed_read_saves_nothing_in_a_collection,
    a_failed_read_saves_nothing_in_a_global
);

fn a_plain_save_reads_no_store(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    frame(cx, f.win);
    type_at_end(cx, f.win, None, ("env-value", 0usize), "z");
    cx.run_until_parked();
    let before = f.store.gets.load(Ordering::SeqCst);
    f.click_save(cx);
    assert_eq!(f.store.gets.load(Ordering::SeqCst), before);
    assert!(f.bytes().contains("az"), "{}", f.bytes());
}
both!(
    a_plain_save_reads_no_store,
    a_plain_save_reads_no_store_in_a_collection,
    a_plain_save_reads_no_store_in_a_global
);

// Escape on the dialog, Save on the close prompt, then the Replace prompt.
fn close_through_save(cx: &mut TestAppContext, win: AnyWindowHandle) {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-save");
}

#[gpui_kit::test]
fn the_close_guard_keeps_the_dialog_after_a_cancel(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    let before = f.bytes();
    f.draft(cx, 1, "old", "typed");
    close_through_save(cx, f.win);
    answer_confirm(cx, CANCEL);
    assert!(present(cx, f.win, "global-env-editor"), "the dialog stays");
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert_eq!(f.bytes(), before);
    assert!(f.typing(cx));
    close_through_save(cx, f.win);
    answer_confirm(cx, REPLACE);
    assert!(!present(cx, f.win, "global-env-editor"), "saved and closed");
    assert_eq!(f.held().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn a_global_lock_asks_before_its_save_replaces_another_row(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    let before = f.bytes();
    f.draft(cx, 1, "old", "typed");
    click_lock(cx, f.win, 0);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.bytes(), before);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn enter_after_an_outside_save_stores_nothing(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.draft(cx, 1, "old", "typed");
    f.save_from_outside(cx);
    assert!(f.bytes().contains("old"), "{}", f.bytes());
    cx.update_window(f.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx), "the typing stays in the input");
}

#[gpui_kit::test]
fn a_save_after_an_outside_save_still_asks(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.draft(cx, 1, "old", "typed");
    f.save_from_outside(cx);
    f.click_save(cx);
    assert!(confirm_open(cx), "the save asks");
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx));
    f.click_save(cx);
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("typed"));
    assert!(!f.typing(cx));
}

#[gpui_kit::test]
fn the_lock_after_an_outside_save_keeps_the_typing_until_answered(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.draft(cx, 1, "old", "typed");
    f.save_from_outside(cx);
    click_lock(cx, f.win, 1);
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx));
    click_lock(cx, f.win, 1);
    answer_confirm(cx, MAKE_PLAIN);
    assert!(f.bytes().contains("typed"), "{}", f.bytes());
    assert_eq!(f.held(), None);
}

#[gpui_kit::test]
fn enter_releases_held_typing_when_nothing_is_stored(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.store.delete(&f.key).unwrap();
    f.hold(cx);
    f.enter(cx);
    assert!(!confirm_open(cx));
    assert_eq!(f.held().as_deref(), Some("typed"));
    assert!(!f.typing(cx));
}

fn enter_asks_over_a_stored_value(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.hold(cx);
    f.enter(cx);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx));
    f.enter(cx);
    answer_confirm(cx, REPLACE);
    f.click_save(cx);
    assert_eq!(f.held().as_deref(), Some("typed"));
    assert!(!f.typing(cx));
}
both!(
    enter_asks_over_a_stored_value,
    enter_asks_over_a_stored_value_in_a_collection,
    enter_asks_over_a_stored_value_in_a_global
);

#[gpui_kit::test]
fn enter_on_held_typing_stores_on_a_global_save_with_nothing_stored(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    f.store.delete(&f.key).unwrap();
    f.hold(cx);
    f.enter(cx);
    assert!(!confirm_open(cx));
    f.click_save(cx);
    assert!(!confirm_open(cx));
    assert_eq!(f.held().as_deref(), Some("typed"));
}

fn a_failed_read_on_enter_keeps_the_typing(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.hold(cx);
    f.store.fail_get.store(true, Ordering::SeqCst);
    f.enter(cx);
    assert!(!confirm_open(cx));
    assert!(f.shows_error(cx));
    assert!(f.typing(cx));
    f.store.fail_get.store(false, Ordering::SeqCst);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}
both!(
    a_failed_read_on_enter_keeps_the_typing,
    a_failed_read_on_enter_keeps_the_typing_in_a_collection,
    a_failed_read_on_enter_keeps_the_typing_in_a_global
);

#[gpui_kit::test]
fn a_switch_releases_held_typing_when_nothing_is_stored(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.store.delete(&f.key).unwrap();
    f.hold(cx);
    f.choose_other(cx);
    assert!(!confirm_open(cx));
    assert_eq!(f.selected(cx).as_deref(), Some("prod"));
    assert_eq!(f.held().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn a_switch_asks_over_a_stored_value(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.hold(cx);
    f.choose_other(cx);
    assert!(confirm_open(cx));
    answer_confirm(cx, CANCEL);
    assert_eq!(
        f.selected(cx).as_deref(),
        Some("dev"),
        "the selection stays"
    );
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx));
    f.choose_other(cx);
    answer_confirm(cx, REPLACE);
    assert_eq!(f.selected(cx).as_deref(), Some("prod"));
    assert_eq!(f.held().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn a_failed_read_stops_a_switch(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.hold(cx);
    f.store.fail_get.store(true, Ordering::SeqCst);
    f.choose_other(cx);
    assert!(!confirm_open(cx));
    assert_eq!(f.selected(cx).as_deref(), Some("dev"));
    assert!(f.shows_error(cx));
    assert!(f.typing(cx));
}

// A click on another environment asks through the close guard, whose Save checks the store.
fn switch_in_the_global_list(cx: &mut TestAppContext, f: &Fix) {
    frame(cx, f.win);
    cx.update_window(f.win, |_, w, cx| w.click(("global-env-item", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_global_switch_keeps_held_typing_until_answered(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    f.hold(cx);
    switch_in_the_global_list(cx, &f);
    answer_confirm(cx, "confirm-save");
    answer_confirm(cx, CANCEL);
    assert_eq!(f.selected(cx).as_deref(), Some("g"));
    assert!(f.typing(cx));
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    switch_in_the_global_list(cx, &f);
    answer_confirm(cx, "confirm-save");
    answer_confirm(cx, REPLACE);
    assert_eq!(f.selected(cx).as_deref(), Some("h"));
    assert_eq!(f.held().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn a_global_switch_stores_held_typing_with_nothing_stored(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    f.store.delete(&f.key).unwrap();
    f.hold(cx);
    switch_in_the_global_list(cx, &f);
    answer_confirm(cx, "confirm-save");
    assert_eq!(f.selected(cx).as_deref(), Some("h"));
    assert_eq!(f.held().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn the_global_close_guard_keeps_held_typing_after_a_cancel(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    f.hold(cx);
    close_through_save(cx, f.win);
    answer_confirm(cx, CANCEL);
    assert!(present(cx, f.win, "global-env-editor"), "the dialog stays");
    assert!(f.typing(cx));
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    f.store.fail_get.store(true, Ordering::SeqCst);
    close_through_save(cx, f.win);
    assert!(
        present(cx, f.win, "global-env-editor"),
        "a failed read stops it"
    );
    assert!(f.shows_error(cx));
    f.store.fail_get.store(false, Ordering::SeqCst);
    close_through_save(cx, f.win);
    answer_confirm(cx, REPLACE);
    assert!(!present(cx, f.win, "global-env-editor"));
    assert_eq!(f.held().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn undo_and_redo_keep_held_typing_out_of_harms_way(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    let Ed::Collection(ed) = f.ed.clone() else {
        unreachable!()
    };
    f.hold(cx);
    let step = |cx: &mut TestAppContext, undo: bool| {
        cx.update_window(f.win, |_, w, cx| {
            ed.update(cx, |e, cx| match undo {
                true => e.undo_edit(w, cx),
                false => e.redo_edit(w, cx),
            })
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, true);
    assert!(confirm_open(cx), "the undo would take the typing with it");
    answer_confirm(cx, CANCEL);
    assert!(f.typing(cx));
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    step(cx, false);
    if confirm_open(cx) {
        answer_confirm(cx, CANCEL);
    }
    assert!(f.typing(cx));
    assert_eq!(f.held().as_deref(), Some("s3cret"));
}

impl Fix {
    fn empty_the_row(&self, cx: &mut TestAppContext) {
        replace_input(cx, self.win, None, ("env-value", 1usize), "");
        cx.run_until_parked();
    }

    fn holds_typing(&self, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| match &self.ed {
            Ed::Collection(e) => e.read(cx).table().read(cx).holds_typing(cx),
            Ed::Global(e) => e.read(cx).table().read(cx).holds_typing(cx),
        })
    }
}

fn retyping_an_emptied_row_still_asks(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.hold(cx);
    f.empty_the_row(cx);
    assert!(!f.typing(cx));
    type_at_end(cx, f.win, None, ("env-value", 1usize), "fresh");
    cx.update_window(f.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    f.click_save(cx);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, CANCEL);
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.typing(cx));
    f.click_save(cx);
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("fresh"));
}
both!(
    retyping_an_emptied_row_still_asks,
    retyping_an_emptied_row_still_asks_in_a_collection,
    retyping_an_emptied_row_still_asks_in_a_global
);

fn an_emptied_row_leaves_the_stored_value(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.hold(cx);
    f.empty_the_row(cx);
    assert!(!f.holds_typing(cx), "nothing is typed");
    f.enter(cx);
    assert!(!confirm_open(cx));
    f.click_save(cx);
    assert!(!confirm_open(cx));
    assert_eq!(f.held().as_deref(), Some("s3cret"));
    assert!(f.bytes().contains("old"), "{}", f.bytes());
}
both!(
    an_emptied_row_leaves_the_stored_value,
    an_emptied_row_leaves_the_stored_value_in_a_collection,
    an_emptied_row_leaves_the_stored_value_in_a_global
);

// `old` is held with nothing stored; a draft row `two` sits over a stored value that only a Save
// would touch.
fn enter_leaves_a_draft_row_out_of_its_prompt(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.store.delete(&f.key).unwrap();
    f.store.set(&f.two, "second").unwrap();
    f.hold(cx);
    f.draft(cx, 2, "two", "other");
    f.enter(cx);
    assert!(!confirm_open(cx), "this Enter replaces nothing");
    assert_eq!(f.store.get(&f.two).unwrap().as_deref(), Some("second"));
    assert!(!f.holds_typing(cx), "the held typing went");
}
both!(
    enter_leaves_a_draft_row_out_of_its_prompt,
    enter_leaves_a_draft_row_out_of_its_prompt_in_a_collection,
    enter_leaves_a_draft_row_out_of_its_prompt_in_a_global
);

fn enter_names_only_the_row_it_releases(cx: &mut TestAppContext, kind: Kind) {
    let f = fix(cx, kind, HOST);
    f.store.set(&f.two, "second").unwrap();
    f.hold(cx);
    f.draft(cx, 2, "two", "other");
    f.enter(cx);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"old\"?")
    );
    answer_confirm(cx, REPLACE);
    assert_eq!(f.store.get(&f.two).unwrap().as_deref(), Some("second"));
    f.click_save(cx);
    assert_eq!(
        title(cx).as_deref(),
        Some("Replace the stored value of \"two\"?"),
        "the draft row is asked about at its own Save"
    );
    answer_confirm(cx, REPLACE);
    assert_eq!(f.held().as_deref(), Some("typed"));
    assert_eq!(f.store.get(&f.two).unwrap().as_deref(), Some("other"));
}
both!(
    enter_names_only_the_row_it_releases,
    enter_names_only_the_row_it_releases_in_a_collection,
    enter_names_only_the_row_it_releases_in_a_global
);

#[gpui_kit::test]
fn a_switch_leaves_a_draft_row_out_of_its_prompt(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Collection, HOST);
    f.store.delete(&f.key).unwrap();
    f.store.set(&f.two, "second").unwrap();
    f.hold(cx);
    f.draft(cx, 2, "two", "other");
    f.choose_other(cx);
    assert!(!confirm_open(cx), "this switch replaces nothing");
    assert_eq!(f.store.get(&f.two).unwrap().as_deref(), Some("second"));
}

// The toggle drops the row's typing and moves the stored value, so nothing is replaced.
#[gpui_kit::test]
fn a_global_lock_does_not_name_the_row_it_toggles(cx: &mut TestAppContext) {
    let f = fix(cx, Kind::Global, HOST);
    f.hold(cx);
    click_lock(cx, f.win, 1);
    answer_confirm(cx, MAKE_PLAIN);
    assert!(!confirm_open(cx), "one question");
    assert!(f.bytes().contains("s3cret"), "{}", f.bytes());
    assert!(!f.bytes().contains("typed"), "{}", f.bytes());
    assert_eq!(f.held(), None);
}

mod held {
    use super::*;
    use apinata_app::tab_area::CloseTab;
    use gpui_kit::VisualTestContext;

    fn windows(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| cx.windows().len())
    }

    // The running app with a row declared by a save from outside, holding its typing.
    fn app(
        cx: &mut TestAppContext,
        stored: bool,
    ) -> (tempfile::TempDir, Started, Arc<Listing>, SecretKey) {
        let dev = format!("name: dev\n{HOST}");
        let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", &dev)]);
        let (store, key) = seeded(&col);
        if !stored {
            store.delete(&key).unwrap();
        }
        let s = started_with_store(cx, &col, store.clone());
        cx.update_window(s.win, |_, w, cx| {
            w.within("title-bar").click("environments", cx)
        })
        .unwrap();
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-name", 1usize), "old");
        cx.run_until_parked();
        click_lock(cx, s.win, 1);
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-value", 1usize), "typed");
        cx.run_until_parked();
        let root = s.root.clone();
        let command = Command::Save(Target::Environment("dev".into()));
        s.ws.update(cx, |w, cx| w.send(&root, command, cx));
        cx.run_until_parked();
        frame(cx, s.win);
        (col, s, store, key)
    }

    fn close_tab(cx: &mut TestAppContext, s: &Started) {
        cx.dispatch_action(s.win, CloseTab);
        cx.run_until_parked();
    }

    fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
        let closed = VisualTestContext::from_window(s.win, cx).simulate_close();
        cx.run_until_parked();
        closed
    }

    #[gpui_kit::test]
    fn the_tab_shows_held_typing_as_unsaved(cx: &mut TestAppContext) {
        let (_col, s, _store, _key) = app(cx, false);
        let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
        let id =
            s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().clone());
        assert!(cx.update(|cx| area.read(cx).holds_typing(&id, cx)));
    }

    #[gpui_kit::test]
    fn a_tab_close_releases_held_typing_when_nothing_is_stored(cx: &mut TestAppContext) {
        let (_col, s, store, key) = app(cx, false);
        close_tab(cx, &s);
        assert!(!confirm_open(cx));
        assert!(!present(cx, s.win, "env-editor"), "the tab went");
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
    }

    #[gpui_kit::test]
    fn a_tab_close_leaves_a_draft_row_out_of_its_prompt(cx: &mut TestAppContext) {
        let (col, s, store, _key) = app(cx, false);
        let two = Collection::open(col.path())
            .unwrap()
            .secret_key("dev", "two");
        store.set(&two, "second").unwrap();
        type_at_end(cx, s.win, None, ("env-name", 2usize), "two");
        cx.run_until_parked();
        click_lock(cx, s.win, 2);
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-value", 2usize), "other");
        cx.run_until_parked();
        close_tab(cx, &s);
        assert_ne!(
            title(cx).as_deref(),
            Some("Replace the stored value of \"two\"?"),
            "closing asks about the draft's Save first"
        );
        assert_eq!(store.get(&two).unwrap().as_deref(), Some("second"));
    }

    #[gpui_kit::test]
    fn a_tab_close_asks_over_a_stored_value(cx: &mut TestAppContext) {
        let (_col, s, store, key) = app(cx, true);
        close_tab(cx, &s);
        assert!(confirm_open(cx));
        answer_confirm(cx, CANCEL);
        assert!(present(cx, s.win, "env-editor"), "the tab stays");
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        close_tab(cx, &s);
        answer_confirm(cx, REPLACE);
        cx.run_until_parked();
        assert!(!confirm_open(cx), "no second prompt");
        assert!(!present(cx, s.win, "env-editor"));
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
    }

    #[gpui_kit::test]
    fn a_failed_read_stops_a_tab_close(cx: &mut TestAppContext) {
        let (_col, s, store, key) = app(cx, true);
        store.fail_get.store(true, Ordering::SeqCst);
        close_tab(cx, &s);
        assert!(!confirm_open(cx));
        assert!(present(cx, s.win, "env-editor"));
        assert!(present(cx, s.win, "env-error"));
        store.fail_get.store(false, Ordering::SeqCst);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
    }

    #[gpui_kit::test]
    fn a_window_close_releases_held_typing_when_nothing_is_stored(cx: &mut TestAppContext) {
        let (_col, s, store, key) = app(cx, false);
        assert!(!close_window(cx, &s), "the check runs first");
        assert!(!confirm_open(cx));
        assert_eq!(windows(cx), 0);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
    }

    #[gpui_kit::test]
    fn a_window_close_asks_over_a_stored_value(cx: &mut TestAppContext) {
        let (_col, s, store, key) = app(cx, true);
        assert!(!close_window(cx, &s));
        assert!(confirm_open(cx));
        answer_confirm(cx, CANCEL);
        assert_eq!(windows(cx), 1, "the window stays");
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert!(!close_window(cx, &s));
        answer_confirm(cx, REPLACE);
        cx.run_until_parked();
        assert_eq!(windows(cx), 0);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
    }

    #[gpui_kit::test]
    fn a_failed_read_stops_a_window_close(cx: &mut TestAppContext) {
        let (_col, s, store, key) = app(cx, true);
        store.fail_get.store(true, Ordering::SeqCst);
        assert!(!close_window(cx, &s));
        assert_eq!(windows(cx), 1);
        assert!(present(cx, s.win, "env-error"));
        store.fail_get.store(false, Ordering::SeqCst);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
    }
}

mod tabs {
    use super::*;
    use apinata_app::keymap::SaveActive;
    use apinata_app::tab_area::{CloseTab, SaveAll};
    use gpui_kit::Action;

    // The running app on the environments tab, with row `old` drafted over a stored value.
    fn app(cx: &mut TestAppContext) -> (tempfile::TempDir, Started, Arc<Listing>, SecretKey) {
        let dev = format!("name: dev\n{HOST}");
        let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", &dev)]);
        let (store, key) = seeded(&col);
        let s = started_with_store(cx, &col, store.clone());
        cx.update_window(s.win, |_, w, cx| {
            w.within("title-bar").click("environments", cx)
        })
        .unwrap();
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-name", 1usize), "old");
        cx.run_until_parked();
        click_lock(cx, s.win, 1);
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-value", 1usize), "typed");
        cx.run_until_parked();
        (col, s, store, key)
    }

    fn file(col: &tempfile::TempDir) -> String {
        std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap()
    }

    fn cancel_then_confirm(cx: &mut TestAppContext, act: impl Action + Clone) {
        let (col, s, store, key) = app(cx);
        let before = file(&col);
        cx.dispatch_action(s.win, act.clone());
        cx.run_until_parked();
        assert!(confirm_open(cx), "the save asks");
        answer_confirm(cx, CANCEL);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(file(&col), before);
        cx.dispatch_action(s.win, act);
        cx.run_until_parked();
        answer_confirm(cx, REPLACE);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
        assert!(file(&col).contains("old"), "{}", file(&col));
    }

    #[gpui_kit::test]
    fn save_active_asks(cx: &mut TestAppContext) {
        cancel_then_confirm(cx, SaveActive);
    }

    #[gpui_kit::test]
    fn save_all_asks(cx: &mut TestAppContext) {
        cancel_then_confirm(cx, SaveAll);
    }

    #[gpui_kit::test]
    fn closing_a_tab_through_save_asks(cx: &mut TestAppContext) {
        let (col, s, store, key) = app(cx);
        let before = file(&col);
        cx.dispatch_action(s.win, CloseTab);
        cx.run_until_parked();
        answer_confirm(cx, "confirm-save");
        assert!(confirm_open(cx), "the save asks");
        answer_confirm(cx, CANCEL);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(file(&col), before);
        assert!(present(cx, s.win, "env-editor"), "the tab stays");
        cx.dispatch_action(s.win, CloseTab);
        cx.run_until_parked();
        answer_confirm(cx, "confirm-save");
        answer_confirm(cx, REPLACE);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
    }

    fn windows(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| cx.windows().len())
    }

    fn typing(cx: &mut TestAppContext, s: &Started) -> bool {
        let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
        let id = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
        cx.update(|cx| {
            let editor = area.read(cx).env_editor(id).unwrap().read(cx);
            editor.table().read(cx).has_secret_typing(cx)
        })
    }

    fn close_window_through_save_all(cx: &mut TestAppContext, s: &Started) {
        assert!(!gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_close());
        cx.run_until_parked();
        assert_eq!(title(cx).as_deref(), Some("Save changes before closing?"));
        answer_confirm(cx, "confirm-save");
    }

    #[gpui_kit::test]
    fn closing_the_window_through_save_all_asks(cx: &mut TestAppContext) {
        let (col, s, store, key) = app(cx);
        let before = file(&col);
        close_window_through_save_all(cx, &s);
        assert_eq!(
            title(cx).as_deref(),
            Some("Replace the stored value of \"old\"?"),
            "the save asks"
        );
        answer_confirm(cx, CANCEL);
        assert_eq!(windows(cx), 1, "the window stays");
        assert!(present(cx, s.win, "env-editor"), "the tab stays");
        assert!(typing(cx, &s), "the typing stays");
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(file(&col), before);
        close_window_through_save_all(cx, &s);
        answer_confirm(cx, REPLACE);
        cx.run_until_parked();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
        assert!(file(&col).contains("old"), "{}", file(&col));
        assert_eq!(windows(cx), 0, "the window closed");
    }

    #[gpui_kit::test]
    fn closing_the_collection_through_save_all_asks(cx: &mut TestAppContext) {
        use apinata_app::shell::CloseCollection;
        let (col, s, store, key) = app(cx);
        let before = file(&col);
        let close = |cx: &mut TestAppContext| {
            cx.dispatch_action(s.win, CloseCollection);
            cx.run_until_parked();
            answer_confirm(cx, "confirm-save");
        };
        close(cx);
        assert_eq!(
            title(cx).as_deref(),
            Some("Replace the stored value of \"old\"?")
        );
        answer_confirm(cx, CANCEL);
        assert_eq!(s.ws.read_with(cx, |w, _| w.collections().len()), 1);
        assert!(typing(cx, &s), "the typing stays");
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(file(&col), before);
        close(cx);
        answer_confirm(cx, REPLACE);
        cx.run_until_parked();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
        assert!(file(&col).contains("old"), "{}", file(&col));
        assert!(s.ws.read_with(cx, |w, _| w.collections().is_empty()));
    }

    // The window's Save asks about the stored value before any save; a draft whose name is
    // refused afterwards still keeps the window open.
    #[gpui_kit::test]
    fn the_windows_save_on_closing_asks_before_a_draft_dialog(cx: &mut TestAppContext) {
        let dev = format!("name: dev\n{HOST}");
        let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", &dev)]);
        let (store, key) = seeded(&col);
        let held = std::cell::RefCell::new(None);
        let s = started_on(
            cx,
            &col,
            |paths| *held.borrow_mut() = apinata_app::draft_file::lock(&paths.drafts),
            store.clone(),
        );
        let _lock = held.into_inner().expect("the drafts lock is free");
        cx.update_window(s.win, |_, w, cx| {
            w.within("title-bar").click("environments", cx)
        })
        .unwrap();
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-name", 1usize), "old");
        cx.run_until_parked();
        click_lock(cx, s.win, 1);
        frame(cx, s.win);
        type_at_end(cx, s.win, None, ("env-value", 1usize), "typed");
        cx.run_until_parked();

        let (draft, _) = new_draft_at(cx, &s, apinata_core::ItemPath::root());
        let edit = Command::Edit {
            target: Target::Request(ip(&draft)),
            edit: apinata_app::command::Edit::Url("http://x/typed".into()),
        };
        let root = s.root.clone();
        s.ws.update(cx, |w, cx| w.send(&root, edit, cx));
        cx.run_until_parked();

        assert!(!gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_close());
        answer_confirm(cx, "confirm-save");
        assert_eq!(
            title(cx).as_deref(),
            Some("Replace the stored value of \"old\"?")
        );
        answer_confirm(cx, REPLACE);
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(windows(cx), 1, "the refused draft keeps the window");
        assert!(
            file(&col).contains("old"),
            "the window's save wrote the row"
        );
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
    }

    // Compare on the changed-file banner, then its Overwrite.
    fn overwrite_from_compare(cx: &mut TestAppContext, win: AnyWindowHandle) {
        frame(cx, win);
        cx.update_window(win, |_, w, cx| w.click("banner-compare", cx))
            .unwrap();
        frame(cx, win);
        cx.update_window(win, |_, w, cx| w.click("compare-overwrite", cx))
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn compares_overwrite_asks(cx: &mut TestAppContext) {
        let (col, s, store, key) = app(cx);
        let outside = "name: dev\nvariables:\n  - name: host\n    value: outside\n";
        std::fs::write(col.path().join("environments/dev.yml"), outside).unwrap();
        let ws = s.ws.clone();
        ws.update(cx, |w, cx| {
            w.send(col.path(), apinata_app::command::Command::ReloadAll, cx)
        });
        overwrite_from_compare(cx, s.win);
        assert!(confirm_open(cx), "the write asks");
        answer_confirm(cx, CANCEL);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(file(&col), outside);
        // The prompt comes again only while the typing is still there.
        overwrite_from_compare(cx, s.win);
        answer_confirm(cx, REPLACE);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
        assert!(file(&col).contains("old"), "{}", file(&col));
    }

    #[gpui_kit::test]
    fn a_banner_write_asks(cx: &mut TestAppContext) {
        let (col, s, store, key) = app(cx);
        std::fs::write(
            col.path().join("environments/dev.yml"),
            "name: dev\nvariables:\n  - name: host\n    value: outside\n",
        )
        .unwrap();
        let ws = s.ws.clone();
        ws.update(cx, |w, cx| {
            w.send(col.path(), apinata_app::command::Command::ReloadAll, cx)
        });
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| w.click("banner-overwrite", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(confirm_open(cx), "the write asks");
        answer_confirm(cx, CANCEL);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
    }
}
