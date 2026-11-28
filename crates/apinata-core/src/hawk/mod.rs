//! The Hawk `Authorization` header signer, built from interpolated fields.

use crate::edit::HawkSpec;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Digest;
use std::time::SystemTime;
use url::Url;

/// One outgoing hop as the signer sees it.
pub(crate) struct Outgoing<'a> {
    pub(crate) method: &'a str,
    pub(crate) url: &'a Url,
    /// The Content-Type the hop sends, if any.
    pub(crate) content_type: Option<&'a str>,
    pub(crate) body: &'a [u8],
}

/// A signed hop: the whole header value and the bare MAC, both to be hidden from logs.
pub(crate) struct Signed {
    pub(crate) header: String,
    pub(crate) mac: String,
}

/// The MAC hash.
#[derive(Clone, Copy, PartialEq)]
enum Algorithm {
    Sha1,
    Sha256,
}

/// A validated Hawk configuration ready to sign hops.
#[derive(Clone, PartialEq)] // no Debug: it holds the key
pub(crate) struct Signer {
    id: String,
    key: String,
    algorithm: Algorithm,
    ext: String,
    app: String,
    dlg: String,
    /// A fixed nonce, `None` for a fresh one per signature.
    nonce: Option<String>,
    /// A fixed timestamp, `None` for the current time.
    timestamp: Option<u64>,
    include_payload_hash: bool,
}

impl Signer {
    /// From interpolated fields. Errors name the field, never the key: a blank id or key, an
    /// algorithm other than sha1 or sha256, a timestamp that is not a whole number, or an
    /// attribute holding a character a header attribute cannot carry.
    pub(crate) fn new(spec: &HawkSpec) -> Result<Signer, String> {
        if spec.auth_id.trim().is_empty() {
            return Err("authId is required".to_owned());
        }
        if spec.auth_key.is_empty() {
            return Err("authKey is required".to_owned());
        }
        let algorithm = match spec.algorithm.trim().to_ascii_lowercase().as_str() {
            "sha256" | "" => Algorithm::Sha256,
            "sha1" => Algorithm::Sha1,
            _ => return Err("unknown algorithm; use sha256 or sha1".to_owned()),
        };
        let timestamp = match spec.timestamp.trim() {
            "" => None,
            ts => Some(
                ts.parse::<u64>()
                    .map_err(|_| "timestamp is not a whole number of seconds".to_owned())?,
            ),
        };
        // Header attributes travel quoted, so only printable ASCII fits; nothing is stripped.
        for (name, value) in [
            ("authId", &spec.auth_id),
            ("ext", &spec.ext),
            ("app", &spec.app),
            ("dlg", &spec.dlg),
            ("nonce", &spec.nonce),
        ] {
            if !value.chars().all(|c| (' '..='~').contains(&c)) {
                return Err(format!(
                    "{name} holds a character a header cannot carry; use printable ASCII"
                ));
            }
        }
        let present = |v: &String| (!v.trim().is_empty()).then(|| v.clone());
        Ok(Signer {
            id: spec.auth_id.clone(),
            key: spec.auth_key.clone(),
            algorithm,
            ext: spec.ext.clone(),
            app: spec.app.clone(),
            dlg: spec.dlg.clone(),
            nonce: present(&spec.nonce),
            timestamp,
            include_payload_hash: spec.include_payload_hash,
        })
    }

    /// The key, so a log can hide it.
    pub(crate) fn secrets(&self) -> [&str; 1] {
        [&self.key]
    }

    /// Signs one hop with the fixed nonce and timestamp, or fresh ones.
    pub(crate) fn sign(&self, out: &Outgoing<'_>) -> Result<Signed, String> {
        let ts = self.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        });
        let nonce = match &self.nonce {
            Some(nonce) => nonce.clone(),
            None => fresh_nonce().ok_or("the OS random generator failed, so no Hawk nonce")?,
        };
        self.sign_at(out, ts, &nonce)
    }

    /// Signs one hop at `ts` with `nonce`: the MAC covers the method, path and query, host,
    /// port, optional body hash, ext and, when an app is set, app and dlg.
    pub(crate) fn sign_at(
        &self,
        out: &Outgoing<'_>,
        ts: u64,
        nonce: &str,
    ) -> Result<Signed, String> {
        let host = out.url.host_str().ok_or("the URL has no host")?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = out.url.port_or_known_default().unwrap_or(443);
        let mut resource = out.url.path().to_owned();
        if let Some(query) = out.url.query() {
            resource.push('?');
            resource.push_str(query);
        }
        let hash = if self.include_payload_hash {
            self.payload_hash(out.content_type.unwrap_or(""), out.body)
        } else {
            String::new()
        };
        let mut normalized = format!(
            "hawk.1.header\n{ts}\n{nonce}\n{}\n{resource}\n{}\n{port}\n{hash}\n{}\n",
            out.method.to_ascii_uppercase(),
            host.to_ascii_lowercase(),
            self.ext.replace('\\', "\\\\").replace('\n', "\\n"),
        );
        if !self.app.is_empty() {
            normalized.push_str(&format!("{}\n{}\n", self.app, self.dlg));
        }
        let mac = STANDARD.encode(self.hmac(normalized.as_bytes()));
        let mut header = format!(
            "Hawk id=\"{}\", ts=\"{ts}\", nonce=\"{}\"",
            escape(&self.id),
            escape(nonce)
        );
        if !hash.is_empty() {
            header.push_str(&format!(", hash=\"{hash}\""));
        }
        if !self.ext.is_empty() {
            header.push_str(&format!(", ext=\"{}\"", escape(&self.ext)));
        }
        header.push_str(&format!(", mac=\"{mac}\""));
        if !self.app.is_empty() {
            header.push_str(&format!(", app=\"{}\"", escape(&self.app)));
            if !self.dlg.is_empty() {
                header.push_str(&format!(", dlg=\"{}\"", escape(&self.dlg)));
            }
        }
        Ok(Signed { header, mac })
    }

    /// The body hash over the media type alone (parameters cut, lowercased) and the bytes.
    fn payload_hash(&self, content_type: &str, body: &[u8]) -> String {
        let media = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let mut text = format!("hawk.1.payload\n{media}\n").into_bytes();
        text.extend_from_slice(body);
        text.push(b'\n');
        let digest = match self.algorithm {
            Algorithm::Sha1 => sha1::Sha1::digest(&text).to_vec(),
            Algorithm::Sha256 => sha2::Sha256::digest(&text).to_vec(),
        };
        STANDARD.encode(digest)
    }

    fn hmac(&self, text: &[u8]) -> Vec<u8> {
        // HMAC takes a key of any length, so construction cannot fail.
        match self.algorithm {
            Algorithm::Sha1 => {
                let mut mac = Hmac::<sha1::Sha1>::new_from_slice(self.key.as_bytes())
                    .expect("HMAC accepts any key length");
                mac.update(text);
                mac.finalize().into_bytes().to_vec()
            }
            Algorithm::Sha256 => {
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(self.key.as_bytes())
                    .expect("HMAC accepts any key length");
                mac.update(text);
                mac.finalize().into_bytes().to_vec()
            }
        }
    }
}

/// A header attribute with its backslashes and quotes escaped.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Six URL-safe characters from the OS generator, as the protocol's client makes them.
fn fresh_nonce() -> Option<String> {
    use rand::{TryRngCore, rngs::OsRng};
    let mut bytes = [0u8; 6];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    let mut text = URL_SAFE_NO_PAD.encode(bytes);
    text.truncate(6);
    Some(text)
}

#[cfg(test)]
mod tests;
