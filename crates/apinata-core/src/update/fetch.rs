//! The update client, the manifest check and the artifact download. The client is private to the updater: it sends
//! only the documented headers, never decodes bodies, and follows redirects only within the
//! URL rule, so byte caps apply to the bytes on the wire.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use reqwest::header::{ACCEPT, ACCEPT_ENCODING, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Client, Response, StatusCode, redirect};
use sha2::{Digest as _, Sha256};

use super::{
    Available, Limits, Staged, UpdateConfig, UpdateError, allowed, evaluate, verify_manifest,
};

const MANIFEST_CAP: u64 = 64 * 1024;
const SIGNATURE_CAP: u64 = 4 * 1024;
const MAX_REDIRECTS: usize = 5;

/// Fetches `manifest_url` (at most 64 KiB) and `<manifest_url>.sig` (at most 4 KiB), verifies,
/// then evaluates. `Ok(None)` when no newer stable release exists.
pub async fn check(cfg: &UpdateConfig) -> Result<Option<Available>, UpdateError> {
    let client = client(&cfg.limits)?;
    let response = get(&client, &cfg.manifest_url, "application/json", &cfg.limits).await?;
    if !response.status().is_success() {
        return Err(UpdateError::Http(response.status().as_u16()));
    }
    let manifest = read_capped(response, MANIFEST_CAP, "the manifest").await?;

    // The signature sits beside the configured URL, wherever the manifest was redirected to.
    let sig_url = format!("{}.sig", cfg.manifest_url);
    let response = get(&client, &sig_url, "application/octet-stream", &cfg.limits).await?;
    if !response.status().is_success() {
        return Err(UpdateError::ManifestSignature(format!(
            "the signature answered HTTP {}",
            response.status().as_u16()
        )));
    }
    let signature = read_capped(response, SIGNATURE_CAP, "the signature").await?;

    // The bytes verified are the bytes parsed; nothing is read from an unverified manifest.
    verify_manifest(&manifest, &signature, &cfg.keys)?;
    evaluate(&manifest, cfg)
}

/// Streams the artifact of `a` to `apinata-update-<version>.part` in `staging_dir`, refusing
/// it as soon as it outgrows the listed size, then checks length and SHA-256 and renames it to
/// `apinata-update-<version>`. `progress(received, size)` follows each chunk written. Any
/// failure removes the `.part`; an unwritable `staging_dir` is refused before any request.
pub async fn download(
    a: &Available,
    staging_dir: &Path,
    progress: impl FnMut(u64, u64),
) -> Result<Staged, UpdateError> {
    let part = staging_dir.join(format!("apinata-update-{}.part", a.version));
    // A `.part` left by an earlier crash is never resumed: its bytes were never verified.
    let _ = std::fs::remove_file(&part);
    // Create exclusively, so a link planted after the remove is refused rather than followed.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options
        .open(&part)
        .map_err(|_| UpdateError::NotWritable(staging_dir.to_path_buf()))?;
    match stream(a, file, &part, progress).await {
        Ok(()) => {
            let path = staging_dir.join(format!("apinata-update-{}", a.version));
            std::fs::rename(&part, &path).map_err(|e| {
                let _ = std::fs::remove_file(&part);
                io(&path, e)
            })?;
            Ok(Staged {
                path,
                version: a.version.clone(),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Writes the body into `file`, never past `a.size` bytes, and checks its length and digest.
async fn stream(
    a: &Available,
    mut file: File,
    part: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), UpdateError> {
    let client = client(&a.limits)?;
    let mut response = client
        .get(&a.url)
        .header(ACCEPT, "application/octet-stream")
        .timeout(a.limits.artifact_total)
        .send()
        .await
        .map_err(transport)?;
    match response.status() {
        StatusCode::NOT_FOUND => return Err(UpdateError::ArtifactMissing),
        status if !status.is_success() => return Err(UpdateError::Http(status.as_u16())),
        _ => {}
    }
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        let got = received + chunk.len() as u64;
        if got > a.size {
            return Err(UpdateError::SizeMismatch {
                expected: a.size,
                got,
            });
        }
        file.write_all(&chunk).map_err(|e| io(part, e))?;
        hasher.update(&chunk);
        received = got;
        progress(received, a.size);
    }
    if received != a.size {
        return Err(UpdateError::SizeMismatch {
            expected: a.size,
            got: received,
        });
    }
    file.sync_all().map_err(|e| io(part, e))?;
    // sha2 digests have no `LowerHex`, so the hex is built byte by byte.
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest != a.sha256 {
        return Err(UpdateError::HashMismatch);
    }
    Ok(())
}

fn io(path: &Path, e: std::io::Error) -> UpdateError {
    UpdateError::Io {
        path: PathBuf::from(path),
        message: e.to_string(),
    }
}

/// The updater's HTTP client, shared by the check and the artifact download.
pub(crate) fn client(limits: &Limits) -> Result<Client, UpdateError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("apinata/", env!("CARGO_PKG_VERSION"))),
    );
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    // Every hop must pass the same URL rule as the configured URL, and the chain is bounded.
    let policy = redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if !allowed(attempt.url()) {
            let refused = format!("refused a redirect to {}", attempt.url());
            attempt.error(refused)
        } else {
            attempt.follow()
        }
    });
    Client::builder()
        .default_headers(headers)
        .http1_only()
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .connect_timeout(limits.connect)
        .read_timeout(limits.artifact_idle)
        .redirect(policy)
        .referer(false)
        .build()
        .map_err(|e| UpdateError::Offline(e.to_string()))
}

async fn get(
    client: &Client,
    url: &str,
    accept: &'static str,
    limits: &Limits,
) -> Result<Response, UpdateError> {
    client
        .get(url)
        .header(ACCEPT, accept)
        .timeout(limits.manifest)
        .send()
        .await
        .map_err(transport)
}

/// Maps a send or body error: a timeout is `Timeout`, anything else (a refused redirect
/// included) means the server could not be reached.
pub(crate) fn transport(e: reqwest::Error) -> UpdateError {
    if e.is_timeout() {
        UpdateError::Timeout
    } else {
        UpdateError::Offline(e.to_string())
    }
}

/// The whole body, refused as soon as it is known to exceed `cap` bytes.
async fn read_capped(mut response: Response, cap: u64, what: &str) -> Result<Vec<u8>, UpdateError> {
    let too_big = || UpdateError::ManifestInvalid(format!("{what} is larger than {cap} bytes"));
    if response.content_length().is_some_and(|n| n > cap) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if (body.len() + chunk.len()) as u64 > cap {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
