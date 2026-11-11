//! Writing one key of a collection's `.env`, and ignoring `.env` in git.

use crate::common;

use apinata_core::{Collection, ContextOptions, VarContext, dotenv_quotable, resolve};
use common::{request, write_tree};
use std::fs;
use std::io::ErrorKind;
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const READER: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/\"\n  headers:\n    - name: X-V\n      value: \"{{process.env.K}}\"\n";

fn opened(dotenv: Option<&str>) -> (TempDir, Collection) {
    let mut files = vec![("opencollection.yml", ROOT), ("r.yml", READER)];
    if let Some(text) = dotenv {
        files.push((".env", text));
    }
    let dir = write_tree(&files);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn dotenv(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join(".env")).expect(".env")
}

// What the reader makes of K, with no process environment beneath the file.
fn read_back(c: &Collection) -> String {
    let options = ContextOptions::new().process_env(Vec::<(String, String)>::new());
    let ctx = VarContext::new(c, options).expect("context");
    let r = resolve(request(c, "r.yml"), &ctx).expect("resolves");
    r.headers()[0].1.clone()
}

#[test]
fn values_read_back_exactly() {
    for value in [
        "plain",
        "a b  c",
        "p=ss#word \"x\"",
        "it's",
        "`tick` and 'single'",
        "back\\slash",
    ] {
        let (_dir, mut c) = opened(None);
        c.set_dotenv("K", value).expect("written");
        assert_eq!(read_back(&c), value, "{value:?}");
    }
}

#[test]
fn other_lines_survive_byte_for_byte() {
    let (dir, mut c) = opened(Some(
        "# keep me\r\nA=1\r\nexport K = old # note\r\nB='two'\r\n",
    ));
    c.set_dotenv("K", "new").expect("written");
    assert_eq!(dotenv(&dir), "# keep me\r\nA=1\r\nK='new'\r\nB='two'\r\n");
    assert_eq!(read_back(&c), "new");
}

#[test]
fn a_missing_key_is_appended_with_the_files_line_ending() {
    let (dir, mut c) = opened(Some("A=1"));
    c.set_dotenv("K", "v").expect("written");
    assert_eq!(dotenv(&dir), "A=1\nK='v'\n");
    let (dir, mut c) = opened(Some("A=1\r\n"));
    c.set_dotenv("K", "v").expect("written");
    assert_eq!(dotenv(&dir), "A=1\r\nK='v'\r\n");
}

#[test]
fn the_last_definition_is_rewritten() {
    let (dir, mut c) = opened(Some("K=1\nK=2\n"));
    c.set_dotenv("K", "3").expect("written");
    assert_eq!(dotenv(&dir), "K=1\nK='3'\n");
    assert_eq!(read_back(&c), "3");
}

#[test]
fn an_absent_file_is_created_private_and_a_reload_sees_no_change() {
    let (dir, mut c) = opened(None);
    c.set_dotenv("K", "v").expect("written");
    assert_eq!(dotenv(&dir), "K='v'\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join(".env"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(c.reload_path(&dir.path().join(".env")).is_empty());
    c.set_dotenv("K", "w").expect("rewritten");
    assert!(c.reload_path(&dir.path().join(".env")).is_empty());
}

#[test]
fn unwritable_input_is_refused_and_nothing_is_written() {
    let (dir, mut c) = opened(None);
    for (name, value) in [
        ("BAD NAME", "v"),
        ("K", "a\nb"),
        ("K", "a\rb"),
        ("K", "'`\""),
    ] {
        let err = c.set_dotenv(name, value).expect_err(name);
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{name:?} {value:?}");
    }
    assert!(!dir.path().join(".env").exists());
}

#[test]
fn keys_are_listed_from_the_file() {
    let (_dir, c) = opened(None);
    assert!(c.dotenv_keys().expect("keys").is_empty());
    let (_dir, c) = opened(Some("A=1\nexport B=2\n# C=3\n"));
    let keys: Vec<String> = c.dotenv_keys().expect("keys").into_iter().collect();
    assert_eq!(keys, ["A", "B"]);
}

#[test]
fn gitignore_gains_one_dotenv_line() {
    let (dir, c) = opened(None);
    let ignore = dir.path().join(".gitignore");
    assert!(c.ensure_dotenv_ignored().expect("written"));
    assert_eq!(fs::read_to_string(&ignore).unwrap(), ".env\n");
    assert!(!c.ensure_dotenv_ignored().expect("read"));
    assert_eq!(fs::read_to_string(&ignore).unwrap(), ".env\n");
    for (before, after, appended) in [
        ("/.env\n", "/.env\n", false),
        ("node_modules", "node_modules\n.env\n", true),
        ("x\r\n", "x\r\n.env\r\n", true),
    ] {
        fs::write(&ignore, before).unwrap();
        assert_eq!(
            c.ensure_dotenv_ignored().expect("ok"),
            appended,
            "{before:?}"
        );
        assert_eq!(fs::read_to_string(&ignore).unwrap(), after);
    }
}

#[test]
fn quotable_agrees_with_set_dotenv() {
    for value in [
        "plain",
        "it's",
        "`tick` and 'single'",
        "'`x",
        "a\nb",
        "a\rb",
        "'`\"",
        "'`\\n",
    ] {
        let (_dir, mut c) = opened(None);
        assert_eq!(
            dotenv_quotable(value),
            c.set_dotenv("K", value).is_ok(),
            "{value:?}"
        );
    }
}

#[test]
fn names_outside_the_shared_variable_rule_are_refused() {
    let (dir, mut c) = opened(None);
    for name in ["", "{{tok}}", "a}}", "caf\u{e9}"] {
        let err = c.set_dotenv(name, "v").expect_err(name);
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{name:?}");
    }
    assert!(!dir.path().join(".env").exists());
}

#[test]
fn a_restore_puts_both_files_back_and_a_reload_sees_no_change() {
    let (dir, mut c) = opened(Some("A=1\n"));
    let backup = c.backup_dotenv().expect("backed up");
    c.set_dotenv("K", "v").expect("written");
    assert!(c.ensure_dotenv_ignored().expect("ignored"));
    c.restore_dotenv(&backup).expect("restored");
    assert_eq!(dotenv(&dir), "A=1\n");
    assert!(!dir.path().join(".gitignore").exists());
    assert!(c.reload_path(&dir.path().join(".env")).is_empty());

    let (dir, mut c) = opened(None);
    let backup = c.backup_dotenv().expect("backed up");
    c.set_dotenv("K", "v").expect("written");
    c.restore_dotenv(&backup).expect("restored");
    assert!(!dir.path().join(".env").exists());
    assert!(c.reload_path(&dir.path().join(".env")).is_empty());
}

#[test]
fn a_backup_debug_shows_no_value() {
    let (_dir, c) = opened(Some("K=hunter2-s3cret\n"));
    let backup = c.backup_dotenv().expect("backed up");
    let shown = format!("{backup:?}");
    // Neither as text nor as the list of bytes a derived Debug would print.
    let bytes = format!("{:?}", "hunter2".as_bytes());
    let bytes = bytes.trim_matches(['[', ']']);
    assert!(
        !shown.contains("hunter2") && !shown.contains(bytes),
        "{shown}"
    );
    assert!(shown.contains("DotenvBackup"), "{shown}");
}

#[test]
fn an_undo_keeps_files_changed_since_the_write() {
    let (dir, mut c) = opened(Some("A=1\n"));
    let backup = c.backup_dotenv().expect("backed up");
    c.set_dotenv("K", "v").expect("written");
    let written = c.backup_dotenv().expect("backed up");
    fs::write(dir.path().join(".env"), "A=1\nK=v\nB=elsewhere\n").expect("changed");
    assert!(!c.undo_dotenv(&backup, &written).expect("checked"));
    assert_eq!(dotenv(&dir), "A=1\nK=v\nB=elsewhere\n");

    // Unchanged since the write, the undo puts the old file back.
    let (dir, mut c) = opened(Some("A=1\n"));
    let backup = c.backup_dotenv().expect("backed up");
    c.set_dotenv("K", "v").expect("written");
    let written = c.backup_dotenv().expect("backed up");
    assert!(c.undo_dotenv(&backup, &written).expect("undone"));
    assert_eq!(dotenv(&dir), "A=1\n");
}
