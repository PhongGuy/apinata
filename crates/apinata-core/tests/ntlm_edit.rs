//! NTLM auth read and written as the reference does, from the
//! NTLM parity corpus.

use apinata_core::{AuthSpec, Collection, ItemPath, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CORPUS: &str = "2026-10-17-m10r-parity";
/// Corpus cases with no Apinata counterpart, each with its reason.
const SKIPPED: &[(&str, &str)] = &[
    (
        "ntlm-sparse-untouched",
        "Apinata writes auth only when it is edited",
    ),
    (
        "folder-ntlm-mode-only",
        "a mode switch writes the mode's keys with empty values",
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

fn ntlm(username: &str, password: &str, domain: &str) -> AuthSpec {
    AuthSpec::Ntlm {
        username: username.into(),
        password: password.into(),
        domain: domain.into(),
    }
}

/// The Apinata edit for request case `id`, given the auth the file reads as.
fn request_edit(id: &str, read: AuthSpec) -> AuthSpec {
    let (user, pass, domain) = match &read {
        AuthSpec::Ntlm {
            username,
            password,
            domain,
        } => (username.clone(), password.clone(), domain.clone()),
        _ => (String::new(), String::new(), String::new()),
    };
    match id {
        "ntlm-new" => ntlm("", "", ""),
        "ntlm-new-filled" => ntlm("bob", "{{ntlmPassword}}", "CORP"),
        "ntlm-edit" => ntlm(&user, "new", &domain),
        "ntlm-edit-domain-blank" => ntlm(&user, &pass, ""),
        "ntlm-sparse" => ntlm(&user, "p", &domain),
        "ntlm-to-inherit" => AuthSpec::Inherit,
        other => panic!("no Apinata edit for {other}"),
    }
}

fn config_edit(id: &str) -> AuthSpec {
    match id {
        "folder-ntlm" => ntlm("fu", "", ""),
        "collection-ntlm" => ntlm("cu", "{{pw}}", "CORP"),
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
fn reads_fill_blanks_and_keep_every_value() {
    let read = |rel: &str| {
        RequestFile::load(research().join(CORPUS).join(rel))
            .unwrap()
            .auth()
    };
    assert_eq!(read("sparse/ntlm-blank.yml"), ntlm("", "", ""));
    assert_eq!(
        read("normalized/synthetic/ntlm.yml"),
        ntlm("alice", "{{ntlmPassword}}", "CORP")
    );
}

#[test]
fn debug_hides_the_password_only() {
    let text = format!("{:?}", ntlm("alice", "pw-secret-1", "CORP"));
    assert!(!text.contains("pw-secret-1"), "{text}");
    assert!(text.contains("alice") && text.contains("CORP"), "{text}");
}
