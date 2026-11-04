//! Reloading `opencollection.yml`, environments and the root `.env`.

use crate::tree_support;

use apinata_core::{ChangeKind::*, ItemKind};
use std::fs;
use tree_support::{ROOT, one, open, reload, write};

const ENV: &str = "name: dev\nvariables:\n  - name: host\n    value: a\n";

#[test]
fn the_collection_file_reloads_and_its_removal_is_ignored() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(
        &dir,
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: renamed\n",
    );
    let changes = reload(&dir, &mut c, "opencollection.yml");
    assert_eq!(
        changes,
        one("opencollection.yml", Modified, ItemKind::Collection)
    );
    assert_eq!(c.name(), "renamed");
    fs::remove_file(dir.path().join("opencollection.yml")).expect("rm");
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!(c.name(), "renamed");
}

#[test]
fn environments_are_added_modified_and_removed() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let env = |kind| one("environments/dev.yml", kind, ItemKind::Environment);
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    write(&dir, "environments/dev.yml", ENV);
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Added));
    assert_eq!(
        c.environments()
            .iter()
            .map(|e| e.name())
            .collect::<Vec<_>>(),
        ["dev"]
    );
    write(
        &dir,
        "environments/dev.yml",
        &ENV.replace("value: a", "value: b"),
    );
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Modified));
    fs::remove_file(dir.path().join("environments/dev.yml")).expect("rm");
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Removed));
    assert!(c.environments().is_empty());
    fs::create_dir(dir.path().join("environments/deep")).expect("mkdir");
    write(&dir, "environments/deep/x.yml", ENV);
    assert!(reload(&dir, &mut c, "environments/deep/x.yml").is_empty());
}

#[test]
fn the_root_dotenv_is_tracked_by_hash() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let dotenv = |kind| one(".env", kind, ItemKind::DotEnv);
    write(&dir, ".env", "A=1\n");
    assert_eq!(reload(&dir, &mut c, ".env"), dotenv(Added));
    assert!(reload(&dir, &mut c, ".env").is_empty());
    write(&dir, ".env", "A=2\n");
    assert_eq!(reload(&dir, &mut c, ".env"), dotenv(Modified));
    fs::remove_file(dir.path().join(".env")).expect("rm");
    assert_eq!(reload(&dir, &mut c, ".env"), dotenv(Removed));
    fs::create_dir(dir.path().join("f")).expect("mkdir");
    write(&dir, "f/.env", "A=1\n");
    assert!(reload(&dir, &mut c, "f/.env").is_empty());
}

#[test]
fn a_broken_collection_file_keeps_its_content_and_clears_when_restored() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, "opencollection.yml", "info: [");
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!(c.name(), "c");
    assert_eq!(tree_support::errors_for(&dir, &c, "opencollection.yml"), 1);
    write(&dir, "opencollection.yml", ROOT);
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!(tree_support::errors_for(&dir, &c, "opencollection.yml"), 0);
    assert_eq!(c.name(), "c");
}

#[test]
fn a_file_named_only_yml_is_not_an_environment() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    write(&dir, "environments/.yml", ENV);
    assert!(reload(&dir, &mut c, "environments/.yml").is_empty());
    assert!(c.environments().is_empty());
}

#[test]
fn a_dirty_environment_survives_its_file_breaking_until_it_parses_again() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    let env = |kind| one("environments/dev.yml", kind, ItemKind::Environment);
    let path = dir.path().join("environments/dev.yml");
    let errors =
        |c: &apinata_core::Collection| c.load_errors().iter().filter(|e| e.path() == path).count();
    c.environment_mut("dev")
        .expect("dev")
        .set_color(Some("red".into()));
    write(&dir, "environments/dev.yml", "name: [");
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Conflict));
    assert_eq!(errors(&c), 1);
    let dev = c.environment_mut("dev").expect("draft kept");
    assert_eq!(
        (dev.color().as_deref(), dev.is_dirty()),
        (Some("red"), true)
    );
    assert!(matches!(
        dev.save(),
        Err(apinata_core::SaveError::Conflict { .. })
    ));

    write(&dir, "environments/dev.yml", ENV);
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Restored));
    assert_eq!(errors(&c), 0);
    c.environment_mut("dev")
        .expect("dev")
        .save()
        .expect("saves over the restored file");
}

#[cfg(unix)]
#[test]
fn an_unreadable_collection_file_is_a_load_error_and_clears_when_readable() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let path = dir.path().join("opencollection.yml");
    let _g = crate::common::ModeGuard::set(&path, 0o000);
    // A superuser reads through the mode bits, so there is nothing to test.
    if fs::read(&path).is_ok() {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore");
        return;
    }
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!(c.name(), "c");
    assert_eq!(tree_support::errors_for(&dir, &c, "opencollection.yml"), 1);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore");
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!(tree_support::errors_for(&dir, &c, "opencollection.yml"), 0);
}
