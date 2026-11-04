//! The per-user script trust file: decisions and approved hashes per canonical collection root.

use apinata_core::{ScriptDecision, ScriptTrust, TrustError, script_hash};
use std::fs;
use tempfile::TempDir;

#[test]
fn hashes_are_lowercase_sha256_hex_of_the_exact_text() {
    assert_eq!(
        script_hash(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        script_hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_ne!(script_hash("a"), script_hash("a\n"));
}

#[test]
fn decisions_and_approvals_round_trip_by_canonical_root() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    fs::create_dir(coll.path().join("sub")).unwrap();
    let file = state.path().join("script-trust.json");
    let mut trust = ScriptTrust::empty(&file);
    assert_eq!(trust.decision(coll.path()), None);
    let hash = script_hash("bru.setVar('a', 1)");
    trust.set_decision(&coll.path().join("sub/.."), Some(ScriptDecision::Allowed));
    trust.approve(coll.path(), [hash.clone()]);
    trust.save().unwrap();

    let back = ScriptTrust::load(&file).unwrap();
    assert_eq!(back.decision(coll.path()), Some(ScriptDecision::Allowed));
    assert!(back.is_approved(coll.path(), &hash));
    assert!(!back.is_approved(coll.path(), &script_hash("other")));
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    let key = fs::canonicalize(coll.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(json["version"], 1);
    assert_eq!(json["collections"][&key]["scripts"], "allowed");
    assert_eq!(json["collections"][&key]["approved"][0], hash.as_str());
}

#[test]
fn a_missing_file_is_undecided_and_a_bad_one_is_an_error() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let file = state.path().join("script-trust.json");
    assert_eq!(
        ScriptTrust::load(&file).unwrap().decision(coll.path()),
        None
    );
    fs::write(&file, "{\"version\": 2, \"collections\": {}}").unwrap();
    assert!(matches!(
        ScriptTrust::load(&file),
        Err(TrustError::Malformed { .. })
    ));
    fs::write(&file, "not json").unwrap();
    assert!(matches!(
        ScriptTrust::load(&file),
        Err(TrustError::Malformed { .. })
    ));
}

#[test]
fn unknown_decisions_read_as_undecided_and_bad_hashes_are_dropped() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let key = fs::canonicalize(coll.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let good = script_hash("x");
    let mut entry = serde_json::Map::new();
    entry.insert("scripts".into(), "sometimes".into());
    entry.insert(
        "approved".into(),
        serde_json::json!([good, "ZZ", good.to_uppercase()]),
    );
    let mut collections = serde_json::Map::new();
    collections.insert(key, serde_json::Value::Object(entry));
    let text = serde_json::json!({ "version": 1, "collections": collections });
    let file = state.path().join("t.json");
    fs::write(&file, text.to_string()).unwrap();
    let trust = ScriptTrust::load(&file).unwrap();
    assert_eq!(trust.decision(coll.path()), None);
    assert_eq!(
        trust.approved(coll.path()).into_iter().collect::<Vec<_>>(),
        [good]
    );
}

#[test]
fn blocking_keeps_approvals() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let file = state.path().join("t.json");
    let mut trust = ScriptTrust::empty(&file);
    trust.approve(coll.path(), [script_hash("a")]);
    trust.set_decision(coll.path(), Some(ScriptDecision::Blocked));
    trust.save().unwrap();
    let back = ScriptTrust::load(&file).unwrap();
    assert_eq!(back.decision(coll.path()), Some(ScriptDecision::Blocked));
    assert!(back.is_approved(coll.path(), &script_hash("a")));
}

// Writers that load, change and save at the same moment must not drop one another's changes.
// The sleep inside each change holds every writer between its load and its save, so without
// the lock all of them read the same empty file and only the last save survives.
#[test]
fn concurrent_updates_keep_every_change() {
    use std::sync::{Arc, Barrier};
    use std::time::Duration;
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let file = state.path().join("script-trust.json");
    let writers = 8;
    let barrier = Arc::new(Barrier::new(writers));
    let handles: Vec<_> = (0..writers)
        .map(|n| {
            let (file, root, barrier) = (file.clone(), coll.path().to_path_buf(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                ScriptTrust::update(&file, |t| {
                    std::thread::sleep(Duration::from_millis(20));
                    t.approve(&root, [script_hash(&format!("script {n}"))]);
                })
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    let back = ScriptTrust::load(&file).unwrap();
    assert_eq!(back.approved(coll.path()).len(), writers);
}

#[test]
fn an_update_saves_its_change_and_never_overwrites_a_bad_file() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let file = state.path().join("script-trust.json");
    let hash = script_hash("let a = 1");
    let got = ScriptTrust::update(&file, |t| {
        t.set_decision(coll.path(), Some(ScriptDecision::Allowed));
        t.approve(coll.path(), [hash.clone()]);
        7
    })
    .unwrap();
    assert_eq!(got, 7);
    let back = ScriptTrust::load(&file).unwrap();
    assert_eq!(back.decision(coll.path()), Some(ScriptDecision::Allowed));
    assert!(back.is_approved(coll.path(), &hash));
    fs::write(&file, "not json").unwrap();
    let mut ran = false;
    let failed = ScriptTrust::update(&file, |_| ran = true);
    assert!(
        matches!(failed, Err(TrustError::Malformed { .. })),
        "{failed:?}"
    );
    assert!(!ran, "a file that failed to load is never changed");
    assert_eq!(fs::read_to_string(&file).unwrap(), "not json");
}

// A first run has no config directory yet: the update creates it and leaves the lock file
// beside the trust file.
#[test]
fn an_update_creates_a_missing_directory_and_locks_beside_the_file() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let file = state.path().join("fresh").join("script-trust.json");
    ScriptTrust::update(&file, |t| {
        t.set_decision(coll.path(), Some(ScriptDecision::Blocked))
    })
    .unwrap();
    assert!(file.is_file());
    assert!(state.path().join("fresh/script-trust.json.lock").is_file());
}

// A parent directory that cannot be created, or a directory the user cannot write, is an
// error naming the lock file, never a panic.
#[test]
fn an_update_reports_an_unusable_directory_as_an_error() {
    let state = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    fs::write(state.path().join("blocker"), "a file, not a directory").unwrap();
    let file = state.path().join("blocker/sub/script-trust.json");
    let failed = ScriptTrust::update(&file, |t| t.approve(coll.path(), [script_hash("x")]));
    assert!(matches!(failed, Err(TrustError::Io { .. })), "{failed:?}");

    // Root writes through any permission bit, so the read-only case only means something
    // for an ordinary user.
    #[cfg(unix)]
    {
        let readonly = state.path().join("readonly");
        fs::create_dir(&readonly).unwrap();
        let _g = crate::common::ModeGuard::set(&readonly, 0o555);
        let probe = readonly.join(".probe");
        if fs::write(&probe, "").is_ok() {
            let _ = fs::remove_file(&probe);
            eprintln!("skipping the read-only case: this user writes through permissions");
        } else {
            let file = readonly.join("script-trust.json");
            let failed = ScriptTrust::update(&file, |t| t.approve(coll.path(), [script_hash("x")]));
            assert!(
                matches!(&failed, Err(TrustError::Io { path, .. }) if path == &readonly.join("script-trust.json.lock")),
                "{failed:?}"
            );
            assert!(!file.exists());
        }
    }
}
