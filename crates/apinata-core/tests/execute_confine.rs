//! `ContextOptions::file_access`: by default every file a collection names (body files,
//! multipart parts, JWT and OAuth 1.0 keys, client certificates) must stay inside the
//! collection root through `..` and symlinks, or nothing is read or sent.
#![cfg(unix)]

use crate::common;

use apinata_core::{
    ClientCertificate, Collection, ContextOptions, ExecError, ExecOptions, Executor, FileAccess,
    NetworkAnswer, NetworkCheck, NetworkQuestion, NetworkSnapshot, QuestionKind, ResolveError,
    ResolvedRequest, VarContext,
};
use common::{at, write_tree};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// Text in every file outside the root; no error may ever carry it.
const SENTINEL: &str = "outside-sentinel";

/// `<tmp>/coll` is the collection, with `data/in.txt`, `link.txt` pointing at
/// `<tmp>/outside.txt` and `ldir` pointing at `<tmp>/outdir`.
fn layout() -> TempDir {
    let tmp = write_tree(&[
        ("coll/opencollection.yml", ROOT),
        ("coll/data/in.txt", "inside"),
        ("outside.txt", SENTINEL),
        ("outdir/secret.txt", SENTINEL),
    ]);
    symlink(
        tmp.path().join("outside.txt"),
        tmp.path().join("coll/link.txt"),
    )
    .unwrap();
    symlink(tmp.path().join("outdir"), tmp.path().join("coll/ldir")).unwrap();
    tmp
}

/// A POST to `url` whose file-mode body selects `file`.
fn file_body(tmp: &Path, url: &str, file: &str) -> ResolvedRequest {
    let body = format!(
        "  body:\n    type: file\n    data:\n      - filePath: \"{file}\"\n        contentType: text/plain\n        selected: true\n"
    );
    resolve_in(tmp, url, &body, ContextOptions::new()).expect("resolves")
}

/// A POST to `url` whose multipart body has one file part with path `file`.
fn multipart(tmp: &Path, url: &str, file: &str, options: ContextOptions) -> ResolvedRequest {
    let body = format!(
        "  body:\n    type: multipart-form\n    data:\n      - name: f\n        type: file\n        value:\n          - \"{file}\"\n"
    );
    resolve_in(tmp, url, &body, options).expect("resolves")
}

fn resolve_in(
    tmp: &Path,
    url: &str,
    extra: &str,
    options: ContextOptions,
) -> Result<ResolvedRequest, ResolveError> {
    let text =
        format!("info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n{extra}");
    std::fs::write(tmp.join("coll/r.yml"), text).unwrap();
    let c = Collection::open(tmp.join("coll")).expect("opens");
    let ctx = VarContext::new(&c, options).unwrap();
    at(&c, &ctx, "r.yml")
}

fn exec() -> Executor {
    Executor::new(ExecOptions::new()).unwrap()
}

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/up"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

fn outside(err: ExecError) -> PathBuf {
    assert!(!err.to_string().contains(SENTINEL), "{err}");
    match err {
        ExecError::FileOutsideCollection { path, .. } => path,
        other => panic!("expected FileOutsideCollection, got {other:?}"),
    }
}

fn resolve_outside(err: ResolveError) -> PathBuf {
    assert!(!err.to_string().contains(SENTINEL), "{err}");
    match err {
        ResolveError::FileOutsideCollection { path, .. } => path,
        other => panic!("expected FileOutsideCollection, got {other:?}"),
    }
}

/// The ways a written path can leave `<tmp>/coll`.
fn escapes(tmp: &Path) -> Vec<String> {
    vec![
        tmp.join("coll/data/in.txt").to_str().unwrap().to_owned(),
        tmp.join("outside.txt").to_str().unwrap().to_owned(),
        "../outside.txt".to_owned(),
        "data/../../outside.txt".to_owned(),
        "link.txt".to_owned(),
        "ldir/secret.txt".to_owned(),
    ]
}

#[tokio::test]
async fn files_inside_the_root_are_sent() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    for file in ["data/in.txt", "./data/../data/in.txt"] {
        let res = exec().send(&file_body(tmp.path(), &url, file)).await;
        assert_eq!(res.expect(file).status(), 200, "{file}");
    }
    let req = multipart(tmp.path(), &url, "data/in.txt", ContextOptions::new());
    assert_eq!(exec().send(&req).await.expect("multipart").status(), 200);
    let bodies = server.received_requests().await.unwrap();
    assert_eq!(bodies[0].body, b"inside");
}

#[tokio::test]
async fn body_and_multipart_escapes_are_refused_by_default_before_sending() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    for file in &escapes(tmp.path()) {
        let err = exec()
            .send(&file_body(tmp.path(), &url, file))
            .await
            .unwrap_err();
        assert_eq!(outside(err), PathBuf::from(file), "{file}");
        let req = multipart(tmp.path(), &url, file, ContextOptions::new());
        let err = exec().send(&req).await.unwrap_err();
        assert_eq!(outside(err), PathBuf::from(file), "multipart {file}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_multipart_path_made_absolute_by_a_variable_reports_the_written_path() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    let dir = tmp.path().join("outdir");
    let options = ContextOptions::new().process_env([("DIR", dir.to_str().unwrap())]);
    let req = multipart(tmp.path(), &url, "{{process.env.DIR}}/secret.txt", options);
    let err = exec().send(&req).await.unwrap_err();
    assert_eq!(
        outside(err),
        PathBuf::from("{{process.env.DIR}}/secret.txt")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_directory_swapped_for_a_symlink_after_resolving_is_refused() {
    let tmp = layout();
    std::fs::create_dir(tmp.path().join("coll/swap")).unwrap();
    std::fs::write(tmp.path().join("coll/swap/secret.txt"), "inside").unwrap();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    let req = multipart(tmp.path(), &url, "swap/secret.txt", ContextOptions::new());
    std::fs::remove_dir_all(tmp.path().join("coll/swap")).unwrap();
    symlink(tmp.path().join("outdir"), tmp.path().join("coll/swap")).unwrap();
    let err = exec().send(&req).await.unwrap_err();
    assert_eq!(outside(err), PathBuf::from("swap/secret.txt"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_file_inside_the_root_is_a_file_error() {
    let tmp = layout();
    let err = exec()
        .send(&file_body(
            tmp.path(),
            "http://127.0.0.1:9/",
            "data/missing.txt",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::File { path, .. } if path == Path::new("data/missing.txt")),
        "{err:?}"
    );
}

#[tokio::test]
async fn anywhere_reads_paths_outside_the_root_as_before() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    let anywhere = ContextOptions::new().file_access(FileAccess::Anywhere);
    let req = multipart(tmp.path(), &url, "../outside.txt", anywhere);
    let res = exec().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    let body = &server.received_requests().await.unwrap()[0].body;
    assert!(String::from_utf8_lossy(body).contains(SENTINEL));
}

/// The auth block of a request that signs with the key file `key`.
type AuthFor = fn(&str) -> String;

fn jwt_auth(key: &str) -> String {
    format!(
        "  auth:\n    type: jwt\n    algorithm: RS256\n    privateKey:\n      type: file\n      value: \"{key}\"\n    payload: '{{\"sub\":\"1\"}}'\n    placement: header\n"
    )
}

fn oauth1_auth(key: &str) -> String {
    format!(
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    signatureMethod: RSA-SHA1\n    privateKey:\n      type: file\n      value: \"{key}\"\n    placement: header\n"
    )
}

/// `<tmp>/coll` with `keys/in.pem` and `<tmp>/out.pem` both holding `pem`, `keys/link.pem`
/// pointing at the outside copy.
fn key_layout(pem: &str) -> TempDir {
    let tmp = layout();
    std::fs::create_dir(tmp.path().join("coll/keys")).unwrap();
    std::fs::copy(pem, tmp.path().join("coll/keys/in.pem")).unwrap();
    std::fs::copy(pem, tmp.path().join("out.pem")).unwrap();
    symlink(
        tmp.path().join("out.pem"),
        tmp.path().join("coll/keys/link.pem"),
    )
    .unwrap();
    tmp
}

fn fixture(rel: &str) -> String {
    format!("{}/tests/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn key_files_outside_the_root_are_refused_by_default() {
    let cases: [(&str, AuthFor); 2] = [
        ("jwt/rsa.pem", jwt_auth),
        ("oauth1/rsa-pkcs8.pem", oauth1_auth),
    ];
    for (pem, auth) in cases {
        let tmp = key_layout(&fixture(pem));
        let url = "http://127.0.0.1:9/up";
        resolve_in(tmp.path(), url, &auth("keys/in.pem"), ContextOptions::new())
            .unwrap_or_else(|e| panic!("{pem} inside: {e}"));
        let out = tmp.path().join("out.pem");
        for key in [out.to_str().unwrap(), "../out.pem", "keys/link.pem"] {
            let err = resolve_in(tmp.path(), url, &auth(key), ContextOptions::new()).unwrap_err();
            assert_eq!(resolve_outside(err), PathBuf::from(key), "{pem} {key}");
        }
        let anywhere = ContextOptions::new().file_access(FileAccess::Anywhere);
        resolve_in(tmp.path(), url, &auth("../out.pem"), anywhere)
            .unwrap_or_else(|e| panic!("{pem} anywhere: {e}"));
    }
}

fn cert_root(cert: &str, key: &str) -> String {
    format!(
        "{ROOT}config:\n  clientCertificates:\n    - domain: \"127.0.0.1\"\n      type: pem\n      certificateFilePath: '{cert}'\n      privateKeyFilePath: '{key}'\n"
    )
}

#[tokio::test]
async fn client_certificates_outside_the_root_are_refused_by_default() {
    let tmp = layout();
    let coll = tmp.path().join("coll");
    std::fs::copy(fixture("network/client.pem"), coll.join("c.pem")).unwrap();
    std::fs::copy(fixture("network/client.key"), coll.join("c.key")).unwrap();
    std::fs::copy(fixture("network/client.key"), tmp.path().join("out.key")).unwrap();
    let url = "https://127.0.0.1:9/up";
    let abs = fixture("network/client.pem");
    for (cert, key) in [(abs.as_str(), "c.key"), ("c.pem", "../out.key")] {
        std::fs::write(coll.join("opencollection.yml"), cert_root(cert, key)).unwrap();
        let req = resolve_in(tmp.path(), url, "", ContextOptions::new()).expect("resolves");
        let err = exec().send(&req).await.unwrap_err();
        let written = if cert == "c.pem" { key } else { cert };
        assert_eq!(outside(err), PathBuf::from(written), "{cert} {key}");
    }
    // Inside the root, the identity loads and the send fails only at the closed port.
    std::fs::write(coll.join("opencollection.yml"), cert_root("c.pem", "c.key")).unwrap();
    let req = resolve_in(tmp.path(), url, "", ContextOptions::new()).expect("resolves");
    let err = exec().send(&req).await.unwrap_err();
    assert!(
        !matches!(
            err,
            ExecError::FileOutsideCollection { .. } | ExecError::Certificate { .. }
        ),
        "{err:?}"
    );
}

/// `<tmp>/coll` whose one certificate for 127.0.0.1 is `c.pem` inside the root with the key
/// `link.key`, a link to `<tmp>/out.key`.
fn linked_key_layout() -> TempDir {
    let tmp = layout();
    let coll = tmp.path().join("coll");
    std::fs::copy(fixture("network/client.pem"), coll.join("c.pem")).unwrap();
    std::fs::copy(fixture("network/client.key"), tmp.path().join("out.key")).unwrap();
    symlink(tmp.path().join("out.key"), coll.join("link.key")).unwrap();
    std::fs::write(
        coll.join("opencollection.yml"),
        cert_root("c.pem", "link.key"),
    )
    .unwrap();
    tmp
}

#[tokio::test]
async fn a_client_certificate_linked_out_of_the_root_is_refused() {
    let tmp = linked_key_layout();
    let url = "https://127.0.0.1:9/up";
    let req = resolve_in(tmp.path(), url, "", ContextOptions::new()).expect("resolves");
    let err = exec().send(&req).await.unwrap_err();
    assert_eq!(outside(err), PathBuf::from("link.key"));
}

/// The fingerprint of `<tmp>/coll`'s network settings in a context reading under `access`.
fn fingerprint(tmp: &Path, access: FileAccess) -> String {
    let c = Collection::open(tmp.join("coll")).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().file_access(access)).unwrap();
    NetworkSnapshot::resolve(&c.file().network(), &ctx, "salt").fingerprint()
}

#[test]
fn a_certificate_outside_the_root_is_never_hashed() {
    let tmp = linked_key_layout();
    let coll = tmp.path().join("coll");
    let out = tmp.path().join("out.key");
    for key in [out.to_str().unwrap(), "../out.key", "link.key"] {
        std::fs::write(coll.join("opencollection.yml"), cert_root("c.pem", key)).unwrap();
        std::fs::write(&out, "first").unwrap();
        let (confined, anywhere) = (
            fingerprint(tmp.path(), FileAccess::Collection),
            fingerprint(tmp.path(), FileAccess::Anywhere),
        );
        std::fs::write(&out, "second").unwrap();
        assert_eq!(
            fingerprint(tmp.path(), FileAccess::Collection),
            confined,
            "{key}"
        );
        assert_ne!(
            fingerprint(tmp.path(), FileAccess::Anywhere),
            anywhere,
            "{key}"
        );
    }
}

#[tokio::test]
async fn the_network_check_never_reads_a_certificate_outside_the_root() {
    let tmp = linked_key_layout();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let ask = move |q: NetworkQuestion| {
        log.lock().unwrap().push(q.kind());
        async { NetworkAnswer::Send }
    };
    let check = NetworkCheck::new(tmp.path().join("ack.json"), ask);
    let exec = Executor::new(ExecOptions::new().network_check(check)).unwrap();
    let url = "https://127.0.0.1:9/up";
    for content in ["first", "second"] {
        std::fs::write(tmp.path().join("out.key"), content).unwrap();
        let req = resolve_in(tmp.path(), url, "", ContextOptions::new()).expect("resolves");
        let err = exec.send(&req).await.unwrap_err();
        assert_eq!(outside(err), PathBuf::from("link.key"), "{content}");
    }
    // The outside file's change is never seen, so only the first sight was asked about.
    assert_eq!(*asked.lock().unwrap(), [QuestionKind::FirstSeen]);
}

#[tokio::test]
async fn certificates_from_the_options_are_read_anywhere() {
    let tmp = layout();
    let url = "https://127.0.0.1:9/up";
    let (cert, key) = (fixture("network/client.pem"), fixture("network/client.key"));
    let options = ContextOptions::new().client_certificates(vec![ClientCertificate::pem(
        "127.0.0.1",
        cert,
        key,
    )]);
    let req = resolve_in(tmp.path(), url, "", options).expect("resolves");
    let err = exec().send(&req).await.unwrap_err();
    // The identity loads, so the send fails only at the closed port.
    assert!(
        !matches!(
            err,
            ExecError::FileOutsideCollection { .. } | ExecError::Certificate { .. }
        ),
        "{err:?}"
    );
}
