//! The artifact download: caps, hash, staging and cleanup.
#![cfg(feature = "update")]

use crate::update_support;

use apinata_core::update::{UpdateError, download};
use std::time::Duration;
use update_support::*;
use wiremock::{MockServer, ResponseTemplate};

const BODY: &[u8] = b"new binary";

async fn setup() -> (MockServer, TestKey, tempfile::TempDir) {
    (
        MockServer::start().await,
        TestKey::generate(),
        tempfile::tempdir().unwrap(),
    )
}

#[tokio::test]
async fn a_verified_download_is_staged() {
    let (s, k, dir) = setup().await;
    serve_artifact(&s, ResponseTemplate::new(200).set_body_bytes(BODY)).await;
    let a = offer(&s, &k, BODY).await;
    let mut seen = Vec::new();
    let staged = download(&a, dir.path(), |got, total| seen.push((got, total)))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&staged.path).unwrap(), BODY);
    assert_eq!(staged.version, "99.0.0");
    assert_eq!(seen.last(), Some(&(10, 10)));
    assert_eq!(leftovers(dir.path()), ["apinata-update-99.0.0"]);
    assert_eq!(staged.path, dir.path().join("apinata-update-99.0.0"));
}

#[tokio::test]
async fn an_encoded_body_is_kept_as_sent() {
    let (s, k, dir) = setup().await;
    let gz = [0x1f, 0x8b, 8, 0, 1, 2, 3];
    serve_artifact(
        &s,
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_bytes(gz),
    )
    .await;
    let staged = download(&offer(&s, &k, &gz).await, dir.path(), |_, _| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(staged.path).unwrap(), gz);
}

#[tokio::test]
async fn a_longer_body_stops_and_leaves_nothing() {
    let (s, k, dir) = setup().await;
    serve_artifact(
        &s,
        ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 1024 * 1024]),
    )
    .await;
    let mut seen = Vec::new();
    let got = download(&offer(&s, &k, BODY).await, dir.path(), |got, _| {
        seen.push(got)
    })
    .await;
    assert!(
        matches!(got, Err(UpdateError::SizeMismatch { expected: 10, .. })),
        "{got:?}"
    );
    assert!(
        seen.iter().all(|&n| n <= 10),
        "wrote past the cap: {seen:?}"
    );
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn a_short_body_is_refused() {
    let (s, k, dir) = setup().await;
    serve_artifact(
        &s,
        ResponseTemplate::new(200).set_body_bytes(b"new ".to_vec()),
    )
    .await;
    let got = download(&offer(&s, &k, BODY).await, dir.path(), |_, _| {}).await;
    assert_eq!(
        got.unwrap_err(),
        UpdateError::SizeMismatch {
            expected: 10,
            got: 4
        }
    );
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn altered_bytes_fail_the_hash() {
    let (s, k, dir) = setup().await;
    serve_artifact(
        &s,
        ResponseTemplate::new(200).set_body_bytes(b"bad binary".to_vec()),
    )
    .await;
    let got = download(&offer(&s, &k, BODY).await, dir.path(), |_, _| {}).await;
    assert_eq!(got.unwrap_err(), UpdateError::HashMismatch);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn a_missing_artifact_and_a_server_error() {
    let (s, k, dir) = setup().await;
    serve_artifact(&s, ResponseTemplate::new(404)).await;
    let got = download(&offer(&s, &k, BODY).await, dir.path(), |_, _| {}).await;
    assert_eq!(got.unwrap_err(), UpdateError::ArtifactMissing);

    let (s, k, dir) = setup().await;
    serve_artifact(&s, ResponseTemplate::new(503)).await;
    let got = download(&offer(&s, &k, BODY).await, dir.path(), |_, _| {}).await;
    assert_eq!(got.unwrap_err(), UpdateError::Http(503));
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn a_stale_part_is_replaced() {
    let (s, k, dir) = setup().await;
    std::fs::write(
        dir.path().join("apinata-update-99.0.0.part"),
        b"junk from a crash",
    )
    .unwrap();
    serve_artifact(&s, ResponseTemplate::new(200).set_body_bytes(BODY)).await;
    download(&offer(&s, &k, BODY).await, dir.path(), |_, _| {})
        .await
        .unwrap();
    assert_eq!(leftovers(dir.path()), ["apinata-update-99.0.0"]);
}

// A link planted at the staging name must never redirect the download's bytes elsewhere.
#[cfg(unix)]
#[tokio::test]
async fn a_planted_symlink_at_the_part_is_never_followed() {
    let (s, k, dir) = setup().await;
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel");
    std::fs::write(&sentinel, b"keep me").unwrap();
    std::os::unix::fs::symlink(&sentinel, dir.path().join("apinata-update-99.0.0.part")).unwrap();
    serve_artifact(&s, ResponseTemplate::new(200).set_body_bytes(BODY)).await;
    let got = download(&offer(&s, &k, BODY).await, dir.path(), |_, _| {}).await;
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep me");
    if let Ok(staged) = got {
        let meta = std::fs::symlink_metadata(&staged.path).unwrap();
        assert!(meta.file_type().is_file(), "{meta:?}");
        // The staged binary is private to the user until it is installed.
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&staged.path).unwrap(), BODY);
    }
}

#[tokio::test]
async fn an_unwritable_staging_dir_is_refused_before_downloading() {
    let (s, k, dir) = setup().await;
    serve_artifact(&s, ResponseTemplate::new(200).set_body_bytes(BODY)).await;
    let a = offer(&s, &k, BODY).await;
    let missing = dir.path().join("missing");
    assert_eq!(
        download(&a, &missing, |_, _| {}).await.unwrap_err(),
        UpdateError::NotWritable(missing)
    );
    let fetched = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/artifact")
        .count();
    assert_eq!(fetched, 0);
}

#[tokio::test]
async fn a_slow_artifact_times_out_and_leaves_nothing() {
    let (s, k, dir) = setup().await;
    serve_artifact(
        &s,
        ResponseTemplate::new(200)
            .set_body_bytes(BODY)
            .set_delay(Duration::from_secs(5)),
    )
    .await;
    let a = offer_within(
        &s,
        &k,
        BODY,
        Duration::from_millis(200),
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(
        download(&a, dir.path(), |_, _| {}).await.unwrap_err(),
        UpdateError::Timeout
    );
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn a_stalled_artifact_hits_the_idle_limit() {
    let (s, k, dir) = setup().await;
    serve_artifact(
        &s,
        ResponseTemplate::new(200)
            .set_body_bytes(BODY)
            .set_delay(Duration::from_secs(5)),
    )
    .await;
    let a = offer_within(
        &s,
        &k,
        BODY,
        Duration::from_secs(60),
        Duration::from_millis(200),
    )
    .await;
    let started = std::time::Instant::now();
    assert_eq!(
        download(&a, dir.path(), |_, _| {}).await.unwrap_err(),
        UpdateError::Timeout
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the idle limit, not the delay, ended it"
    );
    assert!(leftovers(dir.path()).is_empty());
}
