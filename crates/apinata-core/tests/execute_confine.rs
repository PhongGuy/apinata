//! `ExecOptions::confine_files`: body files must be relative and stay inside the collection
//! root through `..` and symlinks, or nothing is read or sent.
#![cfg(unix)]

use crate::common;

use apinata_core::{
    Collection, ContextOptions, ExecError, ExecOptions, Executor, ResolvedRequest, VarContext,
};
use common::{at, write_tree};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// `<tmp>/coll` is the collection, with `data/in.txt`, `link.txt` pointing at
/// `<tmp>/outside.txt` and `ldir` pointing at `<tmp>/outdir`.
fn layout() -> TempDir {
    let tmp = write_tree(&[
        ("coll/opencollection.yml", ROOT),
        ("coll/data/in.txt", "inside"),
        ("outside.txt", "outside"),
        ("outdir/secret.txt", "secret"),
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
    resolve_in(tmp, url, &body, &[])
}

/// A POST to `url` whose multipart body has one file part with path `file`.
fn multipart(tmp: &Path, url: &str, file: &str, process: &[(&str, &str)]) -> ResolvedRequest {
    let body = format!(
        "  body:\n    type: multipart-form\n    data:\n      - name: f\n        type: file\n        value:\n          - \"{file}\"\n"
    );
    resolve_in(tmp, url, &body, process)
}

fn resolve_in(tmp: &Path, url: &str, body: &str, process: &[(&str, &str)]) -> ResolvedRequest {
    let text =
        format!("info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n{body}");
    std::fs::write(tmp.join("coll/r.yml"), text).unwrap();
    let c = Collection::open(tmp.join("coll")).expect("opens");
    let options = ContextOptions::new().process_env(process.iter().copied());
    let ctx = VarContext::new(&c, options).unwrap();
    at(&c, &ctx, "r.yml").expect("resolves")
}

fn confined(tmp: &Path) -> Executor {
    Executor::new(ExecOptions::new().confine_files(tmp.join("coll"))).unwrap()
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
    match err {
        ExecError::FileOutsideCollection { path, .. } => path,
        other => panic!("expected FileOutsideCollection, got {other:?}"),
    }
}

#[tokio::test]
async fn files_inside_the_root_are_sent() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    let exec = confined(tmp.path());
    for file in ["data/in.txt", "./data/../data/in.txt"] {
        let res = exec.send(&file_body(tmp.path(), &url, file)).await;
        assert_eq!(res.expect(file).status(), 200, "{file}");
    }
    let res = exec
        .send(&multipart(tmp.path(), &url, "data/in.txt", &[]))
        .await;
    assert_eq!(res.expect("multipart").status(), 200);
    let bodies = server.received_requests().await.unwrap();
    assert_eq!(bodies[0].body, b"inside");
}

#[tokio::test]
async fn absolute_parent_and_symlink_escapes_are_refused_before_sending() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    let exec = confined(tmp.path());
    let inside_abs = tmp.path().join("coll/data/in.txt");
    let cases = [
        inside_abs.to_str().unwrap().to_owned(),
        "../outside.txt".to_owned(),
        "data/../../outside.txt".to_owned(),
        "link.txt".to_owned(),
        "ldir/secret.txt".to_owned(),
    ];
    for file in &cases {
        let err = exec
            .send(&file_body(tmp.path(), &url, file))
            .await
            .unwrap_err();
        assert_eq!(outside(err), PathBuf::from(file), "{file}");
        let err = exec
            .send(&multipart(tmp.path(), &url, file, &[]))
            .await
            .unwrap_err();
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
    let req = multipart(
        tmp.path(),
        &url,
        "{{process.env.DIR}}/secret.txt",
        &[("DIR", dir.to_str().unwrap())],
    );
    let err = confined(tmp.path()).send(&req).await.unwrap_err();
    assert_eq!(
        outside(err),
        PathBuf::from("{{process.env.DIR}}/secret.txt")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_file_inside_the_root_is_a_file_error() {
    let tmp = layout();
    let err = confined(tmp.path())
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
async fn without_confinement_any_path_is_read_as_before() {
    let tmp = layout();
    let server = server().await;
    let url = format!("{}/up", server.uri());
    let exec = Executor::new(ExecOptions::new()).unwrap();
    let res = exec
        .send(&file_body(tmp.path(), &url, "../outside.txt"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(
        server.received_requests().await.unwrap()[0].body,
        b"outside"
    );
}
