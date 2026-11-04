//! Signing helpers for the updater tests. Every key here is generated at test time.
#![allow(dead_code)] // each test binary uses a different subset

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub struct TestKey(minisign::KeyPair);

impl TestKey {
    pub fn generate() -> TestKey {
        TestKey(minisign::KeyPair::generate_unencrypted_keypair().unwrap())
    }

    /// The public key as a `keys.txt` line: base64 of the minisign key box.
    pub fn key_line(&self) -> String {
        STANDARD.encode(self.0.pk.to_box().unwrap().to_string())
    }

    /// A `latest.json.sig` body as cargo packager writes it: base64 of the signature box.
    pub fn sign(&self, data: &[u8]) -> String {
        let trusted = Some("timestamp:0\tfile:latest.json");
        let sig = minisign::sign(
            Some(&self.0.pk),
            &self.0.sk,
            std::io::Cursor::new(data),
            trusted,
            None,
        );
        STANDARD.encode(sig.unwrap().to_string())
    }
}

use apinata_core::update::{Product, UpdateConfig, build_config_with, target_id};
use sha2::Digest as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const PAGE: &str = "https://example.com/download";

/// A CLI config whose manifest lives at `<base>/latest.json`, trusting `key`.
pub fn config(base: &str, key: &TestKey, current: &str) -> UpdateConfig {
    let url = format!("{base}/latest.json");
    build_config_with(
        Product::Cli,
        Some(&url),
        Some(PAGE),
        &key.key_line(),
        current,
    )
    .unwrap()
}

pub fn sha256_hex(data: &[u8]) -> String {
    sha2::Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A manifest offering `version` of this machine's CLI at `url`, listing `body`'s size and hash.
pub fn manifest(version: &str, url: &str, body: &[u8]) -> String {
    format!(
        r#"{{"version":"{version}","notes":"Release notes.","targets":{{"{}":{{"url":"{url}","format":"binary","size":{},"sha256":"{}"}}}}}}"#,
        target_id(Product::Cli),
        body.len(),
        sha256_hex(body)
    )
}

/// Serves `manifest` and its signature by `key` at `/latest.json` and `/latest.json.sig`.
pub async fn publish(server: &MockServer, key: &TestKey, manifest: &str) {
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/latest.json.sig"))
        .respond_with(ResponseTemplate::new(200).set_body_string(key.sign(manifest.as_bytes())))
        .mount(server)
        .await;
}

use apinata_core::update::{Available, check};
use std::path::Path;
use std::time::Duration;

/// Mounts `response` at `/artifact`.
pub async fn serve_artifact(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Publishes 99.0.0 with its entry at `<server>/artifact` listing `listed`, and returns the offer.
pub async fn offer(server: &MockServer, key: &TestKey, listed: &[u8]) -> Available {
    offer_within(
        server,
        key,
        listed,
        Duration::from_secs(15 * 60),
        Duration::from_secs(30),
    )
    .await
}

/// As `offer`, with the artifact's total and idle time limits.
pub async fn offer_within(
    server: &MockServer,
    key: &TestKey,
    listed: &[u8],
    total: Duration,
    idle: Duration,
) -> Available {
    publish(
        server,
        key,
        &manifest("99.0.0", &format!("{}/artifact", server.uri()), listed),
    )
    .await;
    let mut cfg = config(&server.uri(), key, "1.0.0");
    cfg.limits.artifact_total = total;
    cfg.limits.artifact_idle = idle;
    check(&cfg).await.unwrap().expect("offered")
}

/// The sorted file names in `dir`.
pub fn leftovers(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// As `manifest`, with the entry's `format`.
pub fn manifest_as(version: &str, format: &str, url: &str, body: &[u8]) -> String {
    format!(
        r#"{{"version":"{version}","targets":{{"{}":{{"url":"{url}","format":"{format}","size":{},"sha256":"{}"}}}}}}"#,
        target_id(Product::Cli),
        body.len(),
        sha256_hex(body)
    )
}

/// Serves `body` at `/artifact` and returns the 99.0.0 offer listing it in `format`.
pub async fn offer_as(server: &MockServer, format: &str, body: &[u8]) -> Available {
    let key = TestKey::generate();
    serve_artifact(
        server,
        ResponseTemplate::new(200).set_body_bytes(body.to_vec()),
    )
    .await;
    let url = format!("{}/artifact", server.uri());
    publish(server, &key, &manifest_as("99.0.0", format, &url, body)).await;
    check(&config(&server.uri(), &key, "1.0.0"))
        .await
        .unwrap()
        .expect("offered")
}
