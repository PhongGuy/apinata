//! WSSE and Akamai EdgeGrid auth read and written as the reference does, from the
//! WSSE and EdgeGrid parity corpus.

use apinata_core::{AuthSpec, Collection, EdgeGridSpec, ItemPath, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CORPUS: &str = "2026-10-14-m10d-parity";
/// Corpus cases with no Apinata counterpart, each with its reason (spec: File format).
const SKIPPED: &[(&str, &str)] = &[
    (
        "wsse-sparse-untouched",
        "Apinata writes auth only when it is edited",
    ),
    (
        "edgegrid-sparse-untouched",
        "Apinata writes auth only when it is edited",
    ),
    (
        "folder-wsse-mode-only",
        "a mode switch writes the mode's keys with empty values",
    ),
    (
        "folder-edgegrid-mode-only",
        "a mode switch writes the mode's keys with empty values",
    ),
    (
        "collection-edgegrid-mode-only",
        "a mode switch writes the mode's keys with empty values",
    ),
    (
        "edgegrid-max-invalid",
        "the app rejects a non-number Max Body Size before the writer",
    ),
];

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

fn all_cases() -> Vec<Case> {
    let text = fs::read_to_string(research().join(CORPUS).join("cases.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn cases(kind: &str) -> Vec<Case> {
    let picked: Vec<Case> = all_cases()
        .into_iter()
        .filter(|c| c.kind == kind && !SKIPPED.iter().any(|(id, _)| *id == c.id))
        .collect();
    assert!(!picked.is_empty(), "no {kind} cases");
    picked
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

fn wsse(username: &str, password: &str) -> AuthSpec {
    AuthSpec::Wsse {
        username: username.into(),
        password: password.into(),
    }
}

fn edgegrid(auth: AuthSpec) -> EdgeGridSpec {
    match auth {
        AuthSpec::EdgeGrid(s) => s,
        other => panic!("not edgegrid: {other:?}"),
    }
}

fn eg(f: impl FnOnce(&mut EdgeGridSpec)) -> AuthSpec {
    let mut s = EdgeGridSpec::default();
    f(&mut s);
    AuthSpec::EdgeGrid(s)
}

/// The Apinata edit for request case `id`, given the auth the file reads as.
fn request_edit(id: &str, read: AuthSpec) -> AuthSpec {
    let edit = |f: &dyn Fn(&mut EdgeGridSpec)| {
        let mut s = edgegrid(read.clone());
        f(&mut s);
        AuthSpec::EdgeGrid(s)
    };
    let (user, pass) = match &read {
        AuthSpec::Wsse { username, password } => (username.clone(), password.clone()),
        _ => (String::new(), String::new()),
    };
    match id {
        "wsse-new" | "edgegrid-to-wsse" => wsse("", ""),
        "wsse-new-filled" => wsse("bob", "{{wssePassword}}"),
        "wsse-edit" => wsse(&user, "new"),
        "wsse-edit-blank" => wsse("", &pass),
        "wsse-sparse" => wsse(&user, "p"),
        "wsse-to-inherit" | "edgegrid-to-inherit" => AuthSpec::Inherit,
        "edgegrid-new" => AuthSpec::EdgeGrid(EdgeGridSpec::default()),
        "edgegrid-new-filled" => eg(|s| {
            s.access_token = "at".into();
            s.client_token = "ct".into();
            s.client_secret = "{{clientSecret}}".into();
        }),
        "edgegrid-edit" => edit(&|s| s.client_secret = "cs2".into()),
        "edgegrid-edit-blank" => edit(&|s| s.headers_to_sign.clear()),
        "edgegrid-max-set" => edit(&|s| s.max_body_size = Some(4096.0)),
        "edgegrid-max-clear" => edit(&|s| s.max_body_size = None),
        "edgegrid-max-typed-zero" => edit(&|s| s.max_body_size = Some(0.0)),
        "edgegrid-sparse" => edit(&|s| s.client_secret = "cs".into()),
        "edgegrid-max-string" | "edgegrid-max-zero" | "edgegrid-max-fraction" => {
            edit(&|s| s.nonce = "n".into())
        }
        other => panic!("no Apinata edit for {other}"),
    }
}

/// The Apinata edit for config case `id`.
fn config_edit(id: &str) -> AuthSpec {
    match id {
        "folder-wsse" => wsse("fu", ""),
        "folder-edgegrid" => eg(|s| s.access_token = "fat".into()),
        "folder-inherit" => AuthSpec::Inherit,
        "collection-wsse" => wsse("cu", "{{pw}}"),
        "collection-edgegrid" => eg(|s| {
            s.access_token = "at".into();
            s.client_token = "ct".into();
            s.client_secret = "cs".into();
            s.max_body_size = Some(4096.0);
        }),
        other => panic!("no Apinata edit for {other}"),
    }
}

#[test]
fn every_skip_names_a_corpus_case() {
    let ids: Vec<String> = all_cases().into_iter().map(|c| c.id).collect();
    for (id, _) in SKIPPED {
        assert!(ids.iter().any(|c| c == id), "{id} is not in the corpus");
    }
}

#[test]
fn request_edits_match_the_reference() {
    let mut failures = Vec::new();
    let picked = cases("request");
    for case in &picked {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(&case.files[0]);
        fs::copy(research().join(&case.input), &path).unwrap();
        let mut r = RequestFile::load(&path).unwrap();
        r.set_auth(request_edit(&case.id, r.auth()));
        r.save().unwrap();
        let (got, want) = (
            fs::read_to_string(&path).unwrap(),
            expected(&case.id, &case.files[0]),
        );
        if got != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {got:?}",
                case.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        picked.len(),
        failures.join("\n")
    );
}

#[test]
fn config_edits_match_the_reference() {
    let mut failures = Vec::new();
    for case in cases("config") {
        let dir = TempDir::new().unwrap();
        copy_tree(&research().join(&case.input), dir.path());
        let mut c = Collection::open(dir.path()).unwrap();
        let auth = config_edit(&case.id);
        if case.id.starts_with("folder-") {
            let f = c
                .folder_file_mut(&ItemPath::new("folder_1").unwrap())
                .unwrap();
            f.set_auth(auth);
            f.save().unwrap();
        } else {
            let f = c.file_mut();
            f.set_auth(auth);
            f.save().unwrap();
        }
        let got = fs::read_to_string(dir.path().join(&case.files[0])).unwrap();
        let want = expected(&case.id, &case.files[0]);
        if got != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {got:?}",
                case.id
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn reads_fill_blanks_and_take_only_numbers() {
    let read = |rel: &str| {
        RequestFile::load(research().join(CORPUS).join(rel))
            .unwrap()
            .auth()
    };
    assert_eq!(read("sparse/wsse-blank.yml"), wsse("", ""));
    let s = edgegrid(read("sparse/edgegrid-blank.yml"));
    assert_eq!(
        (
            s.access_token.as_str(),
            s.client_token.as_str(),
            s.client_secret.as_str()
        ),
        ("at", "", "")
    );
    assert_eq!(
        (s.nonce.as_str(), s.base_url.as_str(), s.max_body_size),
        ("  ", "", None)
    );
    assert_eq!(
        edgegrid(read("sparse/edgegrid-max-string.yml")).max_body_size,
        None
    );
    assert_eq!(
        edgegrid(read("sparse/edgegrid-max-zero.yml")).max_body_size,
        Some(0.0)
    );
    assert_eq!(
        edgegrid(read("sparse/edgegrid-max-fraction.yml")).max_body_size,
        Some(1.5)
    );
    let full = edgegrid(read("normalized/synthetic/edgegrid.yml"));
    assert_eq!(full.timestamp, "20140321T19:34:21+0000");
    assert_eq!(full.base_url, "https://akaa-base.luna.akamaiapis.net");
    assert_eq!(full.headers_to_sign, "X-Test1, X-Test2");
    assert_eq!(full.max_body_size, Some(2048.0));
    assert_eq!(
        read("normalized/synthetic/wsse.yml"),
        wsse("bob", "{{wssePassword}}")
    );
}

#[test]
fn debug_hides_credentials() {
    let text = format!(
        "{:?}",
        eg(|s| {
            s.access_token = "akab-access-1".into();
            s.client_token = "akab-client-1".into();
            s.client_secret = "eg-secret-1".into();
            s.base_url = "https://base.test".into();
        })
    );
    for secret in ["akab-access-1", "akab-client-1", "eg-secret-1"] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(text.contains("https://base.test"), "{text}");
    let text = format!("{:?}", wsse("alice", "pw-secret-1"));
    assert!(
        !text.contains("pw-secret-1") && text.contains("alice"),
        "{text}"
    );
}

#[test]
fn equality_compares_max_body_size_by_value() {
    let nan = eg(|s| s.max_body_size = Some(f64::NAN));
    assert_eq!(nan.clone(), nan);
    assert_ne!(nan, eg(|s| s.max_body_size = Some(1.0)));
    assert_ne!(
        eg(|s| s.max_body_size = None),
        eg(|s| s.max_body_size = Some(0.0))
    );
}
