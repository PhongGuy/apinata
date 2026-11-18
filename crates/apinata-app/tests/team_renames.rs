//! The sync dialog's rename rows: what is open, what Move all takes, and when the range clears.

use TargetState::{Free, Same, Taken};
use apinata_app::team_renames::{RenameRows, row_text};
use apinata_core::{PullRange, SecretName, SecretRename, SecretRenames, TargetState};

fn name(s: &str) -> SecretName {
    let (e, v) = s.split_once('/').unwrap();
    SecretName::new(e, v)
}

fn rename(from: &str, to: &[(&str, TargetState)], sure: bool) -> SecretRename {
    SecretRename::new(
        name(from),
        to.iter().map(|(t, s)| (name(t), *s)).collect(),
        sure,
    )
}

fn rows(renames: Vec<SecretRename>, held_back: usize) -> RenameRows {
    RenameRows::new(
        PullRange::new("a", "b"),
        SecretRenames::new(renames, held_back),
    )
}

#[test]
fn rows_settle_only_once_every_row_is_answered() {
    let mut r = rows(
        vec![rename("dev/token", &[("staging/token", Free)], true)],
        0,
    );
    assert!(!r.settle_now());
    r.answer(&name("dev/token"));
    assert!(r.open_rows().is_empty());
    assert!(r.settle_now());
}

#[test]
fn a_held_back_rename_keeps_the_range_even_with_no_rows() {
    assert!(!rows(vec![], 1).settle_now());
    assert!(rows(vec![], 0).settle_now());
}

#[test]
fn a_failed_ask_never_settles() {
    let mut r = rows(vec![], 0);
    r.fail("the secret store is locked".into());
    assert!(!r.settle_now());
}

#[test]
fn a_good_ask_after_a_failed_one_clears_the_failure_and_row_errors() {
    let mut r = rows(
        vec![rename("dev/token", &[("staging/token", Free)], true)],
        0,
    );
    r.set_error(&name("dev/token"), "set refused".into());
    r.fail("the secret store is locked".into());
    assert_eq!(r.open_rows().len(), 1, "a failed ask keeps the rows");
    r.refresh(PullRange::new("a", "b"), SecretRenames::new(vec![], 0));
    assert_eq!(r.failed(), None);
    assert_eq!(r.error(&name("dev/token")), None);
    assert!(r.settle_now());
}

#[test]
fn a_value_being_typed_again_keeps_the_range_until_finished() {
    let mut r = rows(
        vec![rename("dev/token", &[("staging/token", Free)], true)],
        0,
    );
    r.reenter(name("dev/token"), name("staging/token"));
    assert!(r.open_rows().is_empty());
    assert!(!r.settle_now());
    r.finished(&name("dev/token"));
    assert!(r.settle_now());
}

#[test]
fn a_fresh_ask_keeps_answers_and_typing_marks() {
    let a = || rename("dev/a", &[("staging/a", Taken)], true);
    let b = || rename("dev/b", &[("staging/b", Free)], true);
    let mut r = rows(vec![a(), b()], 0);
    r.answer(&name("dev/a"));
    r.reenter(name("dev/b"), name("staging/b"));
    r.refresh(
        PullRange::new("a", "c"),
        SecretRenames::new(vec![a(), b()], 0),
    );
    assert_eq!(r.range(), &PullRange::new("a", "c"));
    assert!(r.open_rows().is_empty());
    assert_eq!(r.reentering(), [(name("dev/b"), name("staging/b"))]);
}

#[test]
fn move_all_takes_only_sure_rows_with_a_free_or_same_target() {
    let r = rows(
        vec![
            rename("dev/a", &[("staging/a", Free)], true),
            rename("dev/b", &[("staging/b", Same)], true),
            rename("dev/c", &[("staging/c", Taken)], true),
            rename("dev/d", &[("staging/e", Free)], false),
            rename("prod/t", &[("aaa/t", Free), ("production/t", Free)], false),
        ],
        0,
    );
    assert_eq!(
        r.movable_sure(),
        [
            (name("dev/a"), name("staging/a")),
            (name("dev/b"), name("staging/b"))
        ]
    );
}

#[test]
fn a_row_reads_old_to_new_and_says_when_it_is_a_guess() {
    let sure = rename("dev/host_key", &[("staging/host_key", Free)], true);
    let guess = rename("dev/token", &[("staging/api_token", Free)], false);
    assert_eq!(row_text(&sure, 0), "dev / host_key → staging / host_key");
    assert_eq!(
        row_text(&guess, 0),
        "dev / token → staging / api_token (renamed? check before moving)"
    );
}

#[test]
fn a_closed_editor_checks_each_typed_value_once_and_an_unsaved_one_reopens_its_row() {
    let mut r = rows(
        vec![rename("dev/token", &[("staging/token", Free)], true)],
        0,
    );
    r.reenter(name("dev/token"), name("staging/token"));
    assert_eq!(
        r.editor_closed(),
        [(name("dev/token"), name("staging/token"))]
    );
    assert_eq!(
        r.editor_closed(),
        [],
        "a check already sent is not repeated"
    );
    assert!(r.open_rows().is_empty(), "hidden until the check answers");
    r.returned(&name("dev/token"));
    assert_eq!(r.open_rows().len(), 1);
    assert!(r.reentering().is_empty());
    assert!(!r.settle_now());
}

#[test]
fn typing_again_after_a_return_checks_the_value_again() {
    let mut r = rows(
        vec![rename("dev/token", &[("staging/token", Free)], true)],
        0,
    );
    r.reenter(name("dev/token"), name("staging/token"));
    r.editor_closed();
    r.returned(&name("dev/token"));
    r.reenter(name("dev/token"), name("staging/token"));
    assert_eq!(r.editor_closed().len(), 1);
    r.finished(&name("dev/token"));
    assert!(r.settle_now());
}

#[test]
fn a_secret_moved_outside_the_app_drops_its_row_without_an_error() {
    use crate::git_support::{Sandbox, write};
    use apinata_core::{Collection, MemoryStore, SecretStore};
    let env = |name: &str, var: &str| {
        format!("name: {name}\nvariables:\n  - name: {var}\n    secret: true\n")
    };
    let sb = Sandbox::new();
    let col = sb.repo("col");
    write(
        &col,
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    );
    write(&col, "environments/dev.yml", &env("dev", "token"));
    sb.commit_all(&col, "base");
    let head = || sb.git(&col, &["rev-parse", "HEAD"]).trim().to_owned();
    let base = head();
    std::fs::remove_file(col.join("environments/dev.yml")).unwrap();
    write(&col, "environments/staging.yml", &env("staging", "token"));
    sb.commit_all(&col, "rename");
    let range = PullRange::new(base, head());
    let c = Collection::open(&col).unwrap();
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), "v").unwrap();
    let scan = || {
        c.secret_renames(&range, None, &store, &sb.options())
            .unwrap()
    };
    let mut r = RenameRows::new(range.clone(), scan());
    assert_eq!(r.open_rows().len(), 1);
    // The CLI moves it while the dialog is open; the next ask finds nothing left to move.
    c.move_renamed_secret(&name("dev/token"), &name("staging/token"), &store)
        .unwrap();
    r.refresh(range.clone(), scan());
    assert!(r.open_rows().is_empty());
    assert_eq!(r.error(&name("dev/token")), None);
    assert_eq!(r.failed(), None);
    assert!(r.settle_now());
    assert!(r.movable_sure().is_empty());
}
