//! Secrets with no file: of a deleted or renamed environment and of a moved collection. They can
//! be seen and removed by name, and a file that exists is never treated as missing.

use apinata_core::{
    Collection, EnvError, EnvWarning, GlobalEnvironments, MemoryStore, MovedRoot, OrphanReport,
    SecretError, SecretKey, SecretStore,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: live\n    secret: true\n  - name: off\n    secret: true\n    enabled: false\n";

fn tree() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join("environments")).unwrap();
    fs::write(dir.path().join("environments/dev.yml"), DEV).unwrap();
    dir
}

fn seed(store: &MemoryStore, c: &Collection, env: &str, names: &[&str]) {
    for name in names {
        store.set(&c.secret_key(env, name), "v").unwrap();
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn held(store: &MemoryStore, c: &Collection, env: &str, name: &str) -> bool {
    store.get(&c.secret_key(env, name)).unwrap().is_some()
}

fn rows(report: &OrphanReport) -> Vec<(&str, &str, bool)> {
    report
        .orphans
        .iter()
        .map(|o| (o.environment.as_str(), o.variable.as_str(), o.has_file))
        .collect()
}

/// A path under a live temp dir that does not exist, and a store with `names` under it.
fn gone_root(dir: &TempDir, names: &[(&str, &str)]) -> (PathBuf, MemoryStore) {
    let root = dir.path().join("old-collection");
    let store = MemoryStore::new();
    for (env, name) in names {
        store
            .set(&SecretKey::new(root.clone(), *env, *name), "v")
            .unwrap();
    }
    (root, store)
}

/// Lists and deletes like its inner store; reading a value is a failure of the test.
struct NeverRead(MemoryStore);

impl SecretStore for NeverRead {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        panic!("no value may be read")
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.0.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(key)
    }
    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        self.0.list(collection)
    }
}

/// Lists like its inner store, twice over, and refuses to delete `locked`.
struct Awkward {
    inner: MemoryStore,
    locked: &'static str,
}

impl SecretStore for Awkward {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.inner.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        if key.variable == self.locked {
            return Err(SecretError::Unavailable {
                message: "locked".into(),
            });
        }
        self.inner.delete(key)
    }
    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let mut keys = self.inner.list(collection)?;
        keys.extend(self.inner.list(collection)?);
        Ok(keys)
    }
}

/// A store with the default `list`, which fails.
struct NoList;

impl SecretStore for NoList {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        panic!("no value may be read")
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        panic!("nothing may be deleted when the list failed")
    }
}

/// Lists as the keychain does, by the text of the root, which a `PathBuf` compare would hide.
struct TextRoots(MemoryStore);

impl SecretStore for TextRoots {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.0.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.0.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(key)
    }
    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let mut keys = self.0.list(collection)?;
        keys.retain(|key| key.collection.as_os_str() == collection.as_os_str());
        Ok(keys)
    }
}

#[test]
fn a_deleted_environment_can_be_seen_and_removed() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "gone", &["b", "a"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert_eq!(rows(&report), [("gone", "a", false), ("gone", "b", false)]);
    assert!(report.unreadable.is_empty());
    assert_eq!(c.fileless_secrets("gone", &store).unwrap(), ["a", "b"]);
    let warnings = c
        .remove_fileless_secrets("gone", &strings(&["a"]), &store)
        .unwrap();
    assert!(warnings.is_empty());
    assert!(!held(&store, &c, "gone", "a") && held(&store, &c, "gone", "b"));
}

#[test]
fn a_rename_outside_the_app_leaves_the_old_name_fileless_and_the_new_one_fine() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live"]);
    fs::rename(
        dir.path().join("environments/dev.yml"),
        dir.path().join("environments/staging.yml"),
    )
    .unwrap();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "staging", &["live"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert_eq!(rows(&report), [("dev", "live", false)]);
    assert!(c.orphaned_secrets("staging", &store).unwrap().is_empty());
}

#[test]
fn a_file_missing_since_the_load_counts_from_the_fresh_read() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live"]);
    fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    assert_eq!(c.fileless_secrets("dev", &store).unwrap(), ["live"]);
}

#[test]
fn a_file_that_failed_to_parse_is_never_offered() {
    let dir = tree();
    fs::write(dir.path().join("environments/broken.yml"), "a: [\n").unwrap();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    assert!(c.environment("broken").is_none());
    seed(&store, &c, "broken", &["maybe-live"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert!(report.orphans.is_empty(), "{report:?}");
    assert_eq!(report.unreadable, ["broken"]);
    let got = c.fileless_secrets("broken", &store);
    assert!(matches!(got, Err(EnvError::HasFile { .. })), "{got:?}");
    let got = c.remove_fileless_secrets("broken", &strings(&["maybe-live"]), &store);
    assert!(matches!(got, Err(EnvError::HasFile { .. })), "{got:?}");
    assert!(held(&store, &c, "broken", "maybe-live"));
}

#[test]
fn a_malformed_file_is_unreadable_and_a_file_with_no_stored_keys_is_not_read() {
    let dir = tree();
    let envs = dir.path().join("environments");
    fs::write(envs.join("shape.yml"), "name: shape\nvariables: 5\n").unwrap();
    fs::write(envs.join("unused.yml"), "a: [\n").unwrap();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "shape", &["a"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert!(report.orphans.is_empty());
    assert_eq!(report.unreadable, ["shape"]);
}

#[cfg(unix)]
#[test]
fn a_symlinked_file_is_a_file_though_it_is_not_loaded() {
    let dir = tree();
    let envs = dir.path().join("environments");
    std::os::unix::fs::symlink(envs.join("dev.yml"), envs.join("link.yml")).unwrap();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    assert!(c.environment("link").is_none());
    seed(&store, &c, "link", &["a"]);
    let got = c.fileless_secrets("link", &store);
    assert!(matches!(got, Err(EnvError::HasFile { .. })), "{got:?}");
    assert_eq!(c.all_orphaned_secrets(&store).unwrap().unreadable, ["link"]);
}

// A name the file system holds as bytes still loads, under its lossy text.
#[cfg(target_os = "linux")]
#[test]
fn a_file_whose_name_is_not_text_is_still_a_file() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tree();
    let name = std::ffi::OsStr::from_bytes(b"caf\xff.yml");
    fs::write(
        dir.path().join("environments").join(name),
        "name: x\nvariables:\n  - name: live\n    secret: true\n",
    )
    .unwrap();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    let env = "caf\u{fffd}";
    assert!(c.environment(env).is_some());
    seed(&store, &c, env, &["live"]);
    let got = c.fileless_secrets(env, &store);
    assert!(matches!(got, Err(EnvError::HasFile { .. })), "{got:?}");
    assert!(c.all_orphaned_secrets(&store).unwrap().orphans.is_empty());
}

#[test]
fn an_environment_with_a_file_refuses_the_fileless_calls_and_deletes_nothing() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live", "stray"]);
    let got = c.fileless_secrets("dev", &store);
    assert!(matches!(got, Err(EnvError::HasFile { .. })), "{got:?}");
    let got = c.remove_fileless_secrets("dev", &strings(&["stray"]), &store);
    match got {
        Err(EnvError::HasFile { environment }) => assert_eq!(environment, "dev"),
        other => panic!("{other:?}"),
    }
    assert!(held(&store, &c, "dev", "stray"));
}

#[test]
fn a_mistyped_name_refuses_the_whole_removal() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "gone", &["a", "b"]);
    let got = c.remove_fileless_secrets("gone", &strings(&["a", "typo"]), &store);
    match got {
        Err(EnvError::NotOrphaned { variables, .. }) => assert_eq!(variables, ["typo"]),
        other => panic!("{other:?}"),
    }
    assert!(held(&store, &c, "gone", "a") && held(&store, &c, "gone", "b"));
}

#[test]
fn a_failed_delete_is_a_warning_and_the_rest_still_go() {
    let dir = tree();
    let inner = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&inner, &c, "gone", &["a", "b", "c"]);
    let store = Awkward { inner, locked: "b" };
    let warnings = c
        .remove_fileless_secrets("gone", &strings(&["a", "b", "c"]), &store)
        .unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(&warnings[0], EnvWarning::Secret { variable, .. } if variable == "b"));
    assert!(!held(&store.inner, &c, "gone", "a") && !held(&store.inner, &c, "gone", "c"));
    assert!(held(&store.inner, &c, "gone", "b"));
}

#[test]
fn the_report_is_sorted_and_has_each_name_once() {
    let dir = tree();
    let inner = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&inner, &c, "dev", &["z", "stray"]);
    seed(&inner, &c, "another", &["q"]);
    seed(&inner, &c, "gone", &["b", "a"]);
    let store = Awkward { inner, locked: "" };
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert_eq!(
        rows(&report),
        [
            ("another", "q", false),
            ("dev", "stray", true),
            ("dev", "z", true),
            ("gone", "a", false),
            ("gone", "b", false),
        ]
    );
    assert_eq!(c.fileless_secrets("gone", &store).unwrap(), ["a", "b"]);
}

#[test]
fn a_list_failure_is_an_error_never_an_empty_report() {
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    let got = c.all_orphaned_secrets(&NoList);
    assert!(
        matches!(got, Err(EnvError::SecretListRoot { .. })),
        "{got:?}"
    );
    let text = got.unwrap_err().to_string();
    assert!(!text.contains("environment"), "{text}");
    let got = c.fileless_secrets("gone", &NoList);
    assert!(matches!(got, Err(EnvError::SecretList { .. })), "{got:?}");
    let got = c.remove_fileless_secrets("gone", &strings(&["a"]), &NoList);
    assert!(matches!(got, Err(EnvError::SecretList { .. })), "{got:?}");
    let scratch = TempDir::new().unwrap();
    let moved = MovedRoot::new(scratch.path().join("old")).unwrap();
    let got = moved.all_orphaned_secrets(&NoList);
    assert!(
        matches!(got, Err(EnvError::SecretListRoot { .. })),
        "{got:?}"
    );
}

#[test]
fn a_directory_that_cannot_be_read_is_an_error_not_zero_files() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live"]);
    fs::remove_dir_all(dir.path().join("environments")).unwrap();
    fs::write(dir.path().join("environments"), "not a directory").unwrap();
    assert!(c.all_orphaned_secrets(&store).is_err());
    assert!(c.fileless_secrets("dev", &store).is_err());
    let got = c.remove_fileless_secrets("dev", &strings(&["live"]), &store);
    assert!(got.is_err());
    assert!(held(&store, &c, "dev", "live"));
}

#[test]
fn a_missing_environments_directory_means_no_files() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live"]);
    fs::remove_dir_all(dir.path().join("environments")).unwrap();
    assert_eq!(c.fileless_secrets("dev", &store).unwrap(), ["live"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert_eq!(rows(&report), [("dev", "live", false)]);
}

#[cfg(unix)]
#[test]
fn a_dangling_environments_link_is_an_error_and_nothing_is_lost() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live"]);
    fs::remove_dir_all(dir.path().join("environments")).unwrap();
    let link = dir.path().join("environments");
    std::os::unix::fs::symlink(dir.path().join("unmounted"), &link).unwrap();
    let text = c.all_orphaned_secrets(&store).unwrap_err().to_string();
    assert!(text.contains(&link.display().to_string()), "{text}");
    assert!(c.fileless_secrets("dev", &store).is_err());
    let got = c.remove_fileless_secrets("dev", &strings(&["live"]), &store);
    assert!(got.is_err());
    assert!(held(&store, &c, "dev", "live"));
}

#[test]
fn a_collection_whose_root_has_gone_is_an_error_and_nothing_is_lost() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live"]);
    fs::remove_dir_all(dir.path()).unwrap();
    let text = c.all_orphaned_secrets(&store).unwrap_err().to_string();
    assert!(text.contains(&c.root().display().to_string()), "{text}");
    assert!(c.fileless_secrets("dev", &store).is_err());
    let got = c.remove_fileless_secrets("dev", &strings(&["live"]), &store);
    assert!(got.is_err());
    assert!(held(&store, &c, "dev", "live"));
}

#[test]
fn a_global_directory_never_created_is_an_empty_answer() {
    let dir = TempDir::new().unwrap();
    let globals = GlobalEnvironments::open(dir.path().join("never/created")).unwrap();
    let store = MemoryStore::new();
    let report = globals.all_orphaned_secrets(&store).unwrap();
    assert!(report.orphans.is_empty() && report.unreadable.is_empty());
    store.set(&globals.secret_key("gone", "a"), "v").unwrap();
    assert_eq!(globals.fileless_secrets("gone", &store).unwrap(), ["a"]);
}

#[cfg(unix)]
#[test]
fn a_dangling_link_at_the_global_directory_is_an_error_and_nothing_is_lost() {
    let dir = TempDir::new().unwrap();
    let link = dir.path().join("globals");
    let globals = GlobalEnvironments::open(&link).unwrap();
    std::os::unix::fs::symlink(dir.path().join("unmounted"), &link).unwrap();
    let store = MemoryStore::new();
    let key = globals.secret_key("dev", "live");
    store.set(&key, "v").unwrap();
    assert!(globals.all_orphaned_secrets(&store).is_err());
    assert!(globals.fileless_secrets("dev", &store).is_err());
    let got = globals.remove_fileless_secrets("dev", &strings(&["live"]), &store);
    assert!(got.is_err());
    assert!(store.get(&key).unwrap().is_some());
}

#[test]
fn a_stored_name_with_a_path_in_it_never_reaches_the_file_system() {
    let dir = tree();
    // A join of the name into a path would land on this file and call the name taken.
    let outside = dir.path().join("x.yml");
    fs::write(&outside, "name: x\n").unwrap();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "../x", &["a"]);
    assert_eq!(c.fileless_secrets("../x", &store).unwrap(), ["a"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert_eq!(rows(&report), [("../x", "a", false)]);
    c.remove_fileless_secrets("../x", &strings(&["a"]), &store)
        .unwrap();
    assert!(outside.is_file());
    assert!(!held(&store, &c, "../x", "a"));
}

#[test]
fn a_live_declaration_stays_out_even_when_disabled() {
    let dir = tree();
    let store = MemoryStore::new();
    let c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "dev", &["live", "off", "stray"]);
    let report = c.all_orphaned_secrets(&store).unwrap();
    assert_eq!(rows(&report), [("dev", "stray", true)]);
}

#[test]
fn no_value_is_ever_read() {
    let dir = tree();
    let store = NeverRead(MemoryStore::new());
    let c = Collection::open(dir.path()).unwrap();
    seed(&store.0, &c, "gone", &["a"]);
    seed(&store.0, &c, "dev", &["stray"]);
    c.all_orphaned_secrets(&store).unwrap();
    c.fileless_secrets("gone", &store).unwrap();
    c.remove_fileless_secrets("gone", &strings(&["a"]), &store)
        .unwrap();
    let scratch = TempDir::new().unwrap();
    let (root, inner) = gone_root(&scratch, &[("dev", "a")]);
    let store = NeverRead(inner);
    let moved = MovedRoot::new(root).unwrap();
    moved.all_orphaned_secrets(&store).unwrap();
    moved.fileless_secrets("dev", &store).unwrap();
    moved
        .remove_fileless_secrets("dev", &strings(&["a"]), &store)
        .unwrap();
}

#[test]
fn a_reload_and_a_save_leave_a_fileless_value_in_place() {
    let dir = tree();
    let store = MemoryStore::new();
    let mut c = Collection::open(dir.path()).unwrap();
    seed(&store, &c, "gone", &["a"]);
    c.reload();
    c.set_variable_text("dev", "b", "1").unwrap();
    assert!(held(&store, &c, "gone", "a"));
}

#[test]
fn global_environments_work_the_same() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("g.yml"),
        "name: g\nvariables:\n  - name: live\n    secret: true\n",
    )
    .unwrap();
    fs::write(dir.path().join("broken.yml"), "a: [\n").unwrap();
    let globals = GlobalEnvironments::open(dir.path()).unwrap();
    let store = MemoryStore::new();
    for (env, name) in [
        ("g", "live"),
        ("g", "stray"),
        ("gone", "a"),
        ("broken", "b"),
    ] {
        store.set(&globals.secret_key(env, name), "v").unwrap();
    }
    let report = globals.all_orphaned_secrets(&store).unwrap();
    assert_eq!(rows(&report), [("g", "stray", true), ("gone", "a", false)]);
    assert_eq!(report.unreadable, ["broken"]);
    assert_eq!(globals.fileless_secrets("gone", &store).unwrap(), ["a"]);
    let got = globals.fileless_secrets("g", &store);
    assert!(matches!(got, Err(EnvError::HasFile { .. })), "{got:?}");
    globals
        .remove_fileless_secrets("gone", &strings(&["a"]), &store)
        .unwrap();
    assert_eq!(store.get(&globals.secret_key("gone", "a")).unwrap(), None);
}

#[test]
fn a_moved_root_lists_every_key_with_no_file_and_removes_by_name() {
    let dir = TempDir::new().unwrap();
    let (root, store) = gone_root(&dir, &[("dev", "b"), ("dev", "a"), ("prod", "c")]);
    let moved = MovedRoot::new(root.clone()).unwrap();
    let report = moved.all_orphaned_secrets(&store).unwrap();
    assert_eq!(
        rows(&report),
        [
            ("dev", "a", false),
            ("dev", "b", false),
            ("prod", "c", false)
        ]
    );
    assert!(report.unreadable.is_empty());
    assert_eq!(moved.fileless_secrets("dev", &store).unwrap(), ["a", "b"]);
    let warnings = moved
        .remove_fileless_secrets("dev", &strings(&["a"]), &store)
        .unwrap();
    assert!(warnings.is_empty());
    assert_eq!(store.get(&SecretKey::new(&root, "dev", "a")).unwrap(), None);
    assert!(
        store
            .get(&SecretKey::new(&root, "dev", "b"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_moved_root_refuses_a_mistyped_name_and_deletes_nothing() {
    let dir = TempDir::new().unwrap();
    let (root, store) = gone_root(&dir, &[("dev", "a")]);
    let moved = MovedRoot::new(root.clone()).unwrap();
    let got = moved.remove_fileless_secrets("dev", &strings(&["a", "typo"]), &store);
    assert!(matches!(got, Err(EnvError::NotOrphaned { .. })), "{got:?}");
    assert!(
        store
            .get(&SecretKey::new(&root, "dev", "a"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_relative_old_root_is_refused() {
    for path in ["old/collection", "", "."] {
        let got = MovedRoot::new(path);
        assert!(
            matches!(got, Err(EnvError::OldRootRelative { .. })),
            "{path}: {got:?}"
        );
    }
}

#[test]
fn an_old_root_where_anything_exists_is_refused() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a-file");
    fs::write(&file, "x").unwrap();
    for path in [dir.path().to_path_buf(), file.clone()] {
        let got = MovedRoot::new(path);
        assert!(
            matches!(got, Err(EnvError::OldRootExists { .. })),
            "{got:?}"
        );
    }
    // Something under a file answers neither yes nor no, so it is refused as unknown.
    let got = MovedRoot::new(file.join("below"));
    assert!(
        matches!(got, Err(EnvError::OldRootUnknown { .. })),
        "{got:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_dangling_link_at_the_old_root_is_something_there() {
    let dir = TempDir::new().unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();
    let got = MovedRoot::new(link);
    assert!(
        matches!(got, Err(EnvError::OldRootExists { .. })),
        "{got:?}"
    );
}

#[test]
fn an_old_root_given_with_a_trailing_slash_or_a_dot_finds_the_keys_stored_without() {
    let dir = TempDir::new().unwrap();
    let (root, store) = gone_root(&dir, &[("dev", "a")]);
    let store = TextRoots(store);
    let mut dotted = dir.path().as_os_str().to_owned();
    dotted.push("/./old-collection");
    for given in [root.join(""), PathBuf::from(dotted)] {
        let moved = MovedRoot::new(given.clone()).unwrap();
        let names = moved.fileless_secrets("dev", &store).unwrap();
        assert_eq!(names, ["a"], "{}", given.display());
    }
}

#[cfg(unix)]
#[test]
fn a_dangling_link_given_with_a_trailing_slash_is_something_there() {
    let dir = TempDir::new().unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();
    let got = MovedRoot::new(link.join(""));
    assert!(
        matches!(got, Err(EnvError::OldRootExists { .. })),
        "{got:?}"
    );
}

#[test]
fn a_root_that_comes_back_is_refused_at_every_call() {
    let dir = TempDir::new().unwrap();
    let (root, store) = gone_root(&dir, &[("dev", "a")]);
    let moved = MovedRoot::new(root.clone()).unwrap();
    fs::create_dir(&root).unwrap();
    let exists = |e: &EnvError| matches!(e, EnvError::OldRootExists { .. });
    assert!(exists(&moved.all_orphaned_secrets(&store).unwrap_err()));
    assert!(exists(&moved.fileless_secrets("dev", &store).unwrap_err()));
    let got = moved.remove_fileless_secrets("dev", &strings(&["a"]), &store);
    assert!(exists(&got.unwrap_err()));
    assert!(
        store
            .get(&SecretKey::new(&root, "dev", "a"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn other_roots_keys_and_per_install_keys_stay_out() {
    let dir = TempDir::new().unwrap();
    let (root, store) = gone_root(&dir, &[("dev", "a")]);
    let other = dir.path().join("elsewhere");
    let kept = [
        SecretKey::new(&other, "dev", "a"),
        SecretKey::new("", "", "oauth2-token-key"),
        SecretKey::new("", "app", "proxy-password"),
    ];
    for key in &kept {
        store.set(key, "v").unwrap();
    }
    let moved = MovedRoot::new(root).unwrap();
    assert_eq!(rows(&moved.all_orphaned_secrets(&store).unwrap()).len(), 1);
    moved
        .remove_fileless_secrets("dev", &strings(&["a"]), &store)
        .unwrap();
    for key in &kept {
        assert!(store.get(key).unwrap().is_some(), "{key:?}");
    }
    assert!(
        moved
            .all_orphaned_secrets(&store)
            .unwrap()
            .orphans
            .is_empty()
    );
}
