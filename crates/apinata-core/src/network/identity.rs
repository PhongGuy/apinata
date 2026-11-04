//! Client identities and extra CA roots, read from PEM files on every use.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pkcs8::der::pem::LineEnding;

use crate::document::{read_regular, read_regular_async};
use crate::execute::ExecError;

use super::{CertPath, CertificateFiles, Preloaded, ResolvedCertificate};

const PKCS12_UNSUPPORTED: &str = "PKCS#12 files are not supported yet; convert it with \
    `openssl pkcs12 -in x.pfx -nodes -out x.pem` and use the result as a PEM certificate and key";
const TRADITIONAL_ENCRYPTED: &str = "this encrypted key format is not supported; convert it with \
    `openssl pkcs8 -topk8 -in key.pem -out key-pkcs8.pem`";
const NO_CERTIFICATE: &str = "no certificate in file";

fn failure(path: impl Into<PathBuf>, message: impl Into<String>) -> ExecError {
    ExecError::Certificate {
        path: path.into(),
        message: message.into(),
    }
}

/// One PEM buffer (certificate chain, then an unencrypted PKCS#8, PKCS#1 or SEC1 key) for
/// `reqwest::Identity::from_pem`, from the bytes the network check read when it ran, else read
/// fresh from disk.
pub(crate) async fn load_identity(
    cert: &ResolvedCertificate,
    preloaded: Option<&Preloaded>,
) -> Result<Arc<[u8]>, ExecError> {
    let (certificate, private_key) = match cert.files() {
        CertificateFiles::Pem {
            certificate,
            private_key,
        } => (certificate, private_key),
        CertificateFiles::Pkcs12 { file } => {
            return Err(failure(&file.written, PKCS12_UNSUPPORTED));
        }
    };
    let chain: String = sections(&read_given(certificate, preloaded, 0).await?)
        .into_iter()
        .filter(|(label, _)| *label == "CERTIFICATE")
        .map(|(_, block)| block)
        .collect();
    if chain.is_empty() {
        return Err(failure(&certificate.written, NO_CERTIFICATE));
    }
    let key = plain_key(
        &read_given(private_key, preloaded, 1).await?,
        cert.passphrase(),
    )
    .map_err(|message| failure(&private_key.written, message))?;
    let pem = format!("{chain}{key}").into_bytes();
    // Checked here so a pair reqwest rejects is reported on the key, never as a client-build error.
    reqwest::Identity::from_pem(&pem).map_err(|_| {
        failure(
            &private_key.written,
            "the certificate and key could not be used together",
        )
    })?;
    Ok(pem.into())
}

/// The CA file's certificates; an unreadable file or one with none is `ExecError::Certificate`.
pub(crate) fn read_ca(path: &Path) -> Result<Vec<reqwest::Certificate>, ExecError> {
    let bytes = read_regular(path).map_err(|e| failure(path, e.to_string()))?;
    match reqwest::Certificate::from_pem_bundle(&bytes) {
        Ok(certs) if !certs.is_empty() => Ok(certs),
        Ok(_) => Err(failure(path, NO_CERTIFICATE)),
        Err(_) => Err(failure(path, "the file is not a PEM certificate bundle")),
    }
}

/// File `at` of the preloaded bytes when the check read them (its error when that read failed,
/// so a file is never read twice), else `file` read now.
async fn read_given(
    file: &CertPath,
    preloaded: Option<&Preloaded>,
    at: usize,
) -> Result<String, ExecError> {
    match preloaded.and_then(|p| p.0.get(at)) {
        Some(Ok(bytes)) => Ok(String::from_utf8_lossy(bytes).into_owned()),
        Some(Err(message)) => Err(failure(&file.written, message.clone())),
        None => read(file).await,
    }
}

async fn read(file: &CertPath) -> Result<String, ExecError> {
    let bytes = read_regular_async(&file.read)
        .await
        .map_err(|e| failure(&file.written, e.to_string()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The first private key in `text` as an unencrypted PEM block, decrypting an encrypted PKCS#8
/// key with `passphrase`. The error is the message for the key's path.
fn plain_key(text: &str, passphrase: &str) -> Result<String, &'static str> {
    let (label, block) = sections(text)
        .into_iter()
        .find(|(label, _)| label.ends_with("PRIVATE KEY"))
        .ok_or("no private key in file")?;
    // OpenSSL's traditional format marks encryption with headers inside an otherwise plain label.
    if block.contains("Proc-Type: 4,ENCRYPTED") {
        return Err(TRADITIONAL_ENCRYPTED);
    }
    if label != "ENCRYPTED PRIVATE KEY" {
        return Ok(block);
    }
    if passphrase.is_empty() {
        return Err("the key is encrypted; set the certificate's passphrase");
    }
    let unreadable = "the encrypted key could not be read";
    let (_, doc) = pkcs8::der::SecretDocument::from_pem(&block).map_err(|_| unreadable)?;
    let info =
        pkcs8::EncryptedPrivateKeyInfoRef::try_from(doc.as_bytes()).map_err(|_| unreadable)?;
    let plain = info.decrypt(passphrase).map_err(|_| "wrong passphrase")?;
    pkcs8::der::pem::encode_string("PRIVATE KEY", LineEnding::LF, plain.as_bytes())
        .map_err(|_| unreadable)
}

/// Every `-----BEGIN <label>-----` to `-----END <label>-----` block in `text`, with its label
/// and its full text ending in a newline.
fn sections(text: &str) -> Vec<(&str, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN ") {
        let after = &rest[start + 11..];
        let Some(label_end) = after.find("-----") else {
            break;
        };
        let label = &after[..label_end];
        let end_marker = format!("-----END {label}-----");
        let Some(end) = rest[start..].find(&end_marker) else {
            break;
        };
        let stop = start + end + end_marker.len();
        found.push((label, format!("{}\n", &rest[start..stop])));
        rest = &rest[stop..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/network")
            .join(name)
    }

    // The identity is built from the bytes the check read, so a file swapped after the check
    // never reaches the send.
    #[tokio::test]
    async fn the_identity_comes_from_the_preloaded_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let (cert, key) = (dir.path().join("c.pem"), dir.path().join("c.key"));
        std::fs::copy(fixture("client.pem"), &cert).unwrap();
        std::fs::copy(fixture("client.key"), &key).unwrap();
        let path = |p: &PathBuf| CertPath {
            read: p.clone(),
            written: p.display().to_string(),
        };
        let resolved = ResolvedCertificate::new(
            "127.0.0.1".into(),
            CertificateFiles::Pem {
                certificate: path(&cert),
                private_key: path(&key),
            },
            String::new(),
        );
        let original = load_identity(&resolved, None).await.unwrap();
        let loaded = Preloaded(vec![
            Ok(std::fs::read(&cert).unwrap()),
            Ok(std::fs::read(&key).unwrap()),
        ]);
        std::fs::copy(fixture("server.pem"), &cert).unwrap();
        std::fs::copy(fixture("server.key"), &key).unwrap();
        let sent = load_identity(&resolved, Some(&loaded)).await.unwrap();
        assert_eq!(sent, original);
        let unread = Preloaded(vec![Err("gone".into()), Err("gone".into())]);
        assert!(load_identity(&resolved, Some(&unread)).await.is_err());
    }
}
