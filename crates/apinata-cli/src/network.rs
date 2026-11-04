//! Mapping the network flags onto core's settings and the extra client certificates.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use apinata_core::{ClientCertificate, GlobalProxy, NetworkSettings, PacSource};
use serde::Deserialize;

use crate::NetworkArgs;

/// Settings for `run` and `mcp`: the system proxy unless `--noproxy` or `--proxy-pac`, plus the
/// CA flags.
pub fn settings(args: &NetworkArgs) -> NetworkSettings {
    settings_from(
        args.noproxy,
        args.cacert.clone(),
        args.ignore_truststore,
        args.proxy_pac.clone(),
    )
}

/// The settings the proxy and CA flags describe, whichever command carries them.
pub fn settings_from(
    noproxy: bool,
    cacert: Option<PathBuf>,
    ignore_truststore: bool,
    proxy_pac: Option<PacSource>,
) -> NetworkSettings {
    let settings = NetworkSettings::new()
        .force_no_proxy(noproxy)
        .ca_file(cacert)
        .keep_default_roots(!ignore_truststore);
    match proxy_pac {
        Some(source) => settings.proxy(GlobalProxy::Pac { source }),
        None => settings,
    }
}

/// `text` as a PAC source, a relative path joined to the working directory. A text with no
/// scheme whose first segment (before `/` or `\`) holds `@` is refused, since as a path its
/// shown form would keep the password. Errors never repeat the input.
pub fn parse_pac_source(text: &str) -> Result<PacSource, String> {
    let text = text.trim();
    // Only a `://` right at the first separator is a scheme; one later in a query is not.
    let cut = text.find(['/', '\\']).unwrap_or(text.len());
    let has_scheme = text[..cut].ends_with(':') && text[cut..].starts_with("//");
    if !has_scheme && text[..cut].contains('@') {
        return Err(
            "the PAC source looks like a URL with credentials; add http:// or https://".to_owned(),
        );
    }
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
    PacSource::parse(text, Some(&cwd)).map_err(|e| e.to_string())
}

/// The file behind `--client-cert-config`: certificates apply only while `enabled` is true.
#[derive(Deserialize)]
struct CertConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    certs: Vec<CertEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CertEntry {
    #[serde(default)]
    domain: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    cert_file_path: String,
    #[serde(default)]
    key_file_path: String,
    #[serde(default)]
    pfx_file_path: String,
    #[serde(default)]
    passphrase: String,
}

/// Read `--client-cert-config`: `cert` entries become PEM certificates, `pfx` entries PKCS#12
/// ones and any other type is skipped. Errors name the file but never its contents.
pub fn client_certificates(args: &NetworkArgs) -> anyhow::Result<Vec<ClientCertificate>> {
    let Some(path) = &args.client_cert_config else {
        return Ok(Vec::new());
    };
    let text = read(path)?;
    // serde_json's messages quote the offending value, which may be a passphrase: keep only
    // the position.
    let config: CertConfig = serde_json::from_str(&text).map_err(|e| {
        anyhow!(
            "cannot parse client certificate config {}: invalid JSON or field type at line {} \
             column {}",
            path.display(),
            e.line(),
            e.column()
        )
    })?;
    if !config.enabled {
        return Ok(Vec::new());
    }
    Ok(config
        .certs
        .into_iter()
        .filter_map(|entry| {
            let mut cert = match entry.kind.as_str() {
                "cert" => {
                    ClientCertificate::pem(entry.domain, entry.cert_file_path, entry.key_file_path)
                }
                "pfx" => ClientCertificate::pkcs12(entry.domain, entry.pfx_file_path),
                _ => return None,
            };
            cert.passphrase = entry.passphrase;
            Some(cert)
        })
        .collect())
}

fn read(path: &Path) -> anyhow::Result<String> {
    if !path.exists() {
        return Err(anyhow!(
            "client certificate config {} does not exist",
            path.display()
        ));
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("cannot read client certificate config {}", path.display()))
}
