//! Script approvals in the inbox: version 2 entries the host reader skips.

use apinata_core::{ApprovalInbox, Filing, ScriptDecision, ScriptTrust, script_hash};
use std::fs;
use tempfile::TempDir;

struct Fx {
    _tmp: TempDir,
    coll: TempDir,
    inbox: ApprovalInbox,
    trust: ScriptTrust,
}

fn fx() -> Fx {
    let tmp = TempDir::new().unwrap();
    Fx {
        inbox: ApprovalInbox::new(tmp.path().join("approvals")),
        trust: ScriptTrust::empty(tmp.path().join("script-trust.json")),
        coll: TempDir::new().unwrap(),
        _tmp: tmp,
    }
}

#[test]
fn a_script_request_is_a_version_2_file_the_host_reader_skips() {
    let f = fx();
    let hashes = vec![script_hash("a"), script_hash("b")];
    let filed = f
        .inbox
        .request_scripts(&f.trust, f.coll.path(), "users/get.yml", &hashes)
        .unwrap();
    let Filing::Written(path) = filed else {
        panic!("{filed:?}")
    };
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 2);
    assert_eq!(json["kind"], "scripts");
    assert_eq!(json["request"], "users/get.yml");
    assert_eq!(json["hashes"], serde_json::json!(hashes));
    let root = fs::canonicalize(f.coll.path()).unwrap();
    assert_eq!(json["collection"], root.to_string_lossy().as_ref());
    assert!(f.inbox.pending().unwrap().is_empty());
    let scripts = f.inbox.pending_scripts().unwrap();
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].collection, root);
    assert_eq!(scripts[0].hashes, hashes);
    assert_eq!(scripts[0].request, "users/get.yml");
    assert_eq!(scripts[0].file, path);
}

#[test]
fn a_covered_request_is_pending_and_a_blocked_collection_files_nothing() {
    let mut f = fx();
    let (a, b) = (script_hash("a"), script_hash("b"));
    f.inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &[a.clone(), b.clone()])
        .unwrap();
    let again = f
        .inbox
        .request_scripts(&f.trust, f.coll.path(), "other.yml", &[b]);
    assert_eq!(again.unwrap(), Filing::Pending);
    let new = f
        .inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &[script_hash("c")]);
    assert!(matches!(new.unwrap(), Filing::Written(_)));
    f.trust
        .set_decision(f.coll.path(), Some(ScriptDecision::Blocked));
    let blocked = f
        .inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &[script_hash("d")]);
    assert_eq!(blocked.unwrap(), Filing::Denied);
    assert_eq!(f.inbox.pending_scripts().unwrap().len(), 2);
    // Once the user dismisses the entry, the same hashes are filed again.
    let first = f.inbox.pending_scripts().unwrap()[0].clone();
    assert!(f.inbox.remove_scripts(&first).unwrap());
    f.trust.set_decision(f.coll.path(), None);
    let refiled = f
        .inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &[a]);
    assert!(matches!(refiled.unwrap(), Filing::Written(_)));
}

#[test]
fn forged_script_entries_are_skipped_and_removal_deletes_one_file() {
    let f = fx();
    let dir = f.inbox.dir().to_path_buf();
    fs::create_dir_all(&dir).unwrap();
    let coll = f.coll.path().to_string_lossy().into_owned();
    let entry = |kind: &str, hashes: serde_json::Value| {
        serde_json::json!({"version": 2, "kind": kind, "collection": coll, "request": "r.yml",
            "hashes": hashes, "requested_at": "2026-10-11T00:00:00Z"})
        .to_string()
    };
    fs::write(
        dir.join("1-1.json"),
        entry("scripts", serde_json::json!(["nothex"])),
    )
    .unwrap();
    fs::write(
        dir.join("1-2.json"),
        entry("other", serde_json::json!([script_hash("a")])),
    )
    .unwrap();
    fs::write(
        dir.join("1-3.json"),
        entry("scripts", serde_json::json!([])),
    )
    .unwrap();
    assert!(f.inbox.pending_scripts().unwrap().is_empty());

    f.inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &[script_hash("a")])
        .unwrap();
    let pending = f.inbox.pending_scripts().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(f.inbox.remove_scripts(&pending[0]).unwrap());
    assert!(f.inbox.pending_scripts().unwrap().is_empty());
    assert!(!f.inbox.remove_scripts(&pending[0]).unwrap());
}

#[test]
fn prune_deletes_old_script_requests_too() {
    let f = fx();
    let filed = |text: &str| {
        let Filing::Written(path) = f
            .inbox
            .request_scripts(&f.trust, f.coll.path(), "r.yml", &[script_hash(text)])
            .unwrap()
        else {
            panic!("not written")
        };
        path
    };
    let old = filed("old");
    filed("new");
    let eight_days = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(eight_days)
        .unwrap();
    let week = std::time::Duration::from_secs(7 * 86_400);
    assert_eq!(f.inbox.prune(week).unwrap(), 1);
    let left = f.inbox.pending_scripts().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].hashes, [script_hash("new")]);
}

#[test]
fn malformed_hashes_are_dropped_and_none_left_files_nothing() {
    let f = fx();
    let bad = [
        "".to_owned(),
        "not-a-hash".to_owned(),
        script_hash("a").to_uppercase(),
    ];
    let filed = f
        .inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &bad)
        .unwrap();
    assert_eq!(filed, Filing::Pending);
    assert!(f.inbox.pending_scripts().unwrap().is_empty());
    let mixed = [bad[1].clone(), script_hash("b")];
    f.inbox
        .request_scripts(&f.trust, f.coll.path(), "r.yml", &mixed)
        .unwrap();
    let scripts = f.inbox.pending_scripts().unwrap();
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].hashes, [script_hash("b")]);
}
