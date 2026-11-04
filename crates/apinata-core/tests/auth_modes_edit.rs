//! Digest and AWS SigV4 auth written as the reference writes it, from the
//! auth-modes parity corpus.

use apinata_core::{AuthSpec, Collection, ItemPath, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CORPUS: &str = "2026-10-12-auth-modes-parity";

fn research() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    kind: String,
    input: String,
    files: Vec<String>,
}

fn cases(kind: &str) -> Vec<Case> {
    let path = research().join(CORPUS).join("cases.json");
    let all: Vec<Case> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let selected: Vec<Case> = all.into_iter().filter(|c| c.kind == kind).collect();
    assert!(!selected.is_empty(), "no {kind} cases");
    selected
}

fn expected(id: &str, file: &str) -> String {
    fs::read_to_string(research().join(CORPUS).join("expected").join(id).join(file)).unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn digest(u: &str, p: &str) -> AuthSpec {
    AuthSpec::Digest {
        username: u.into(),
        password: p.into(),
    }
}

fn aws(ak: &str, sk: &str, token: &str, service: &str, region: &str, profile: &str) -> AuthSpec {
    AuthSpec::AwsV4 {
        access_key_id: ak.into(),
        secret_access_key: sk.into(),
        session_token: token.into(),
        service: service.into(),
        region: region.into(),
        profile_name: profile.into(),
    }
}

fn request_edit(id: &str) -> AuthSpec {
    match id {
        "digest-new" => digest("u", "{{pw}}"),
        "digest-edit" => digest("alice", "new"),
        "digest-sparse" => digest("alice", "pw"),
        "awsv4-new" => aws("AKID", "{{secret}}", "", "execute-api", "eu-west-1", ""),
        "awsv4-edit" => aws(
            "AKID",
            "{{secret}}",
            "tok",
            "execute-api",
            "us-west-2",
            "dev",
        ),
        "awsv4-to-digest" => digest("u", "p"),
        other => panic!("no Apinata edit for {other}"),
    }
}

fn report(failures: Vec<String>, total: usize) {
    assert!(
        failures.is_empty(),
        "{} of {total} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn request_edits_match_the_reference() {
    let selected = cases("request");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(&case.files[0]);
        fs::copy(research().join(&case.input), &path).unwrap();
        let mut r = RequestFile::load(&path).unwrap();
        r.set_auth(request_edit(&case.id));
        r.save().unwrap();
        let (actual, want) = (
            fs::read_to_string(&path).unwrap(),
            expected(&case.id, &case.files[0]),
        );
        if actual != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {actual:?}",
                case.id
            ));
        }
    }
    report(failures, selected.len());
}

#[test]
fn config_edits_match_the_reference() {
    let selected = cases("config");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        copy_tree(&research().join(&case.input), dir.path());
        let mut c = Collection::open(dir.path()).unwrap();
        match case.id.as_str() {
            "folder-digest" => {
                let f = c
                    .folder_file_mut(&ItemPath::new("folder_1").unwrap())
                    .unwrap();
                f.set_auth(digest("svc", "{{pw}}"));
                f.save().unwrap();
            }
            "collection-awsv4" => {
                let f = c.file_mut();
                f.set_auth(aws("", "", "", "execute-api", "eu-west-1", "dev"));
                f.save().unwrap();
            }
            other => panic!("no Apinata edit for {other}"),
        }
        for file in &case.files {
            let actual = fs::read_to_string(dir.path().join(file)).unwrap();
            let want = expected(&case.id, file);
            if actual != want {
                failures.push(format!(
                    "{} {file}\n  expected {want:?}\n  actual   {actual:?}",
                    case.id
                ));
            }
        }
    }
    report(failures, selected.len());
}

#[test]
fn both_modes_read_with_missing_keys_as_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    let head = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: u\n";
    fs::write(
        &path,
        format!("{head}  auth:\n    type: digest\n    username: alice\n"),
    )
    .unwrap();
    assert_eq!(
        RequestFile::load(&path).unwrap().auth(),
        digest("alice", "")
    );
    fs::write(
        &path,
        format!("{head}  auth:\n    type: awsv4\n    region: eu-west-1\n    profileName: dev\n"),
    )
    .unwrap();
    assert_eq!(
        RequestFile::load(&path).unwrap().auth(),
        aws("", "", "", "", "eu-west-1", "dev")
    );
}

#[test]
fn round_trip_fixtures_read_as_the_new_modes() {
    let base = research().join("2026-10-09-import-parity/expected");
    let d = RequestFile::load(base.join("round-trip-digest/Own auth.yml")).unwrap();
    assert!(
        matches!(d.auth(), AuthSpec::Digest { .. }),
        "{:?}",
        d.auth()
    );
    let a = RequestFile::load(base.join("round-trip-aws/Own Auth.yml")).unwrap();
    assert!(matches!(a.auth(), AuthSpec::AwsV4 { .. }), "{:?}", a.auth());
}

#[test]
fn debug_hides_credentials() {
    let text = format!(
        "{:?} {:?}",
        digest("alice", "pw-123"),
        aws("AKID-9", "sk-456", "tok-789", "svc", "r1", "dev")
    );
    for secret in ["pw-123", "AKID-9", "sk-456", "tok-789"] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(
        text.contains("alice") && text.contains("svc") && text.contains("dev"),
        "{text}"
    );
}
