//! Hawk auth read from and written to a request file.

use apinata_core::{AuthSpec, HawkSpec, RequestFile};
use std::fs;
use tempfile::TempDir;

const HEAD: &str =
    "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test/\n";

const FULL: &str = "  auth:
    type: hawk
    authId: dh37fgj492je
    authKey: \"{{hawk_key}}\"
    algorithm: sha1
    ext: some-app-ext-data
    app: my-app
    dlg: their-app
    nonce: j4h3g2
    timestamp: \"1353832234\"
    includePayloadHash: true
";

fn hawk(auth: AuthSpec) -> HawkSpec {
    match auth {
        AuthSpec::Hawk(spec) => spec,
        other => panic!("not hawk: {other:?}"),
    }
}

fn file(auth: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    fs::write(&path, format!("{HEAD}{auth}")).unwrap();
    (dir, path)
}

#[test]
fn every_key_reads_into_the_spec() {
    let (_d, path) = file(FULL);
    let spec = hawk(RequestFile::load(&path).unwrap().auth());
    assert_eq!(spec.auth_id, "dh37fgj492je");
    assert_eq!(spec.auth_key, "{{hawk_key}}");
    assert_eq!(spec.algorithm, "sha1");
    assert_eq!(spec.ext, "some-app-ext-data");
    assert_eq!(spec.app, "my-app");
    assert_eq!(spec.dlg, "their-app");
    assert_eq!(spec.nonce, "j4h3g2");
    assert_eq!(spec.timestamp, "1353832234");
    assert!(spec.include_payload_hash);
}

#[test]
fn an_unchanged_spec_leaves_the_file_byte_for_byte() {
    let (_d, path) = file(FULL);
    let mut r = RequestFile::load(&path).unwrap();
    r.set_auth(r.auth());
    r.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{HEAD}{FULL}"));
}

#[test]
fn a_new_spec_writes_every_key_in_order() {
    let (_d, path) = file("");
    let mut r = RequestFile::load(&path).unwrap();
    let mut spec = HawkSpec::default();
    spec.auth_id = "id1".into();
    spec.auth_key = "{{hawk_key}}".into();
    r.set_auth(AuthSpec::Hawk(spec.clone()));
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let want = "  auth:
    type: hawk
    authId: id1
    authKey: \"{{hawk_key}}\"
    algorithm: sha256
    ext: \"\"
    app: \"\"
    dlg: \"\"
    nonce: \"\"
    timestamp: \"\"
    includePayloadHash: false
";
    assert!(
        text.ends_with(&format!("url: https://x.test/\n{want}")),
        "{text}"
    );
    assert_eq!(hawk(RequestFile::load(&path).unwrap().auth()), spec);
}

#[test]
fn a_sparse_block_reads_defaults_and_a_quoted_flag_reads_as_its_word() {
    let sparse = hawk(
        RequestFile::load(&file("  auth:\n    type: hawk\n").1)
            .unwrap()
            .auth(),
    );
    assert_eq!(sparse.algorithm, "sha256");
    assert!(!sparse.include_payload_hash);
    let quoted =
        "  auth:\n    type: hawk\n    includePayloadHash: \"true\"\n    timestamp: 1353832234\n";
    let spec = hawk(RequestFile::load(&file(quoted).1).unwrap().auth());
    assert!(spec.include_payload_hash);
    assert_eq!(spec.timestamp, "1353832234");
}

#[test]
fn debug_hides_the_key() {
    let mut spec = HawkSpec::default();
    spec.auth_key = "hunter2-hawk-key".into();
    let shown = format!("{:?}", AuthSpec::Hawk(spec));
    assert!(!shown.contains("hunter2-hawk-key"), "{shown}");
}
