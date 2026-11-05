//! Which PEM block of a key file is the key. This reader fails closed: it opens plain files, and
//! anything else with a PEM marker is refused rather than guessed at.
//!
//! A body is base64 lines ending LF or CRLF between exact markers, and text may sit around blocks.
//! A line holding `-----BEGIN` that is not such a block refuses the whole text. A private read takes
//! the first key block, past clean blocks only. A public read needs every block clean, then tries
//! its labels in Node's order and falls back to the private read.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::crypto::{OID_EC, OID_ED25519, OID_RSA, certificate_spki, element, first_value};
use super::crypto_pss::OID_RSA_PSS;
use super::jwt::KeyInput;

const BYTE_ORDER_MARK: &[u8] = &[0xef, 0xbb, 0xbf];
const MARKER: &[u8] = b"-----BEGIN";
const CERTIFICATES: [&str; 2] = ["CERTIFICATE", "X509 CERTIFICATE"];
/// The public labels in the order Node tries them.
const PUBLIC_PASSES: [&[&str]; 3] = [&["PUBLIC KEY"], &["RSA PUBLIC KEY"], &CERTIFICATES];

const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const OCTET_STRING: u8 = 0x04;
const BIT_STRING: u8 = 0x03;
const OID: u8 = 0x06;
const RSA_INTEGERS: usize = 9;
const RSA_PUBLIC_INTEGERS: usize = 2;
const DSA_INTEGERS: usize = 6;

/// One PEM block: the label it reads under and its DER body.
pub(crate) struct Pem {
    pub(crate) label: &'static str,
    pub(crate) der: Vec<u8>,
}

/// The label of an exact `-----<word> label-----` line.
fn marker<'a>(line: &'a [u8], word: &str) -> Option<&'a str> {
    let label = line
        .strip_prefix(format!("-----{word} ").as_bytes())?
        .strip_suffix(b"-----")?;
    std::str::from_utf8(label).ok()
}

/// Every block in file order with its DER; None when any line holding a BEGIN is not a readable
/// block. A body line with anything but base64 fails the decode, a marker included.
fn blocks(bytes: &[u8]) -> Option<Vec<(&str, Vec<u8>)>> {
    let bytes = bytes.strip_prefix(BYTE_ORDER_MARK).unwrap_or(bytes);
    let mut lines = bytes
        .split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line));
    let mut found = Vec::new();
    while let Some(line) = lines.next() {
        if !line.windows(MARKER.len()).any(|w| w == MARKER) {
            continue;
        }
        let label = marker(line, "BEGIN")?;
        let mut text = Vec::new();
        loop {
            let line = lines.next().filter(|line| !line.is_empty())?;
            if marker(line, "END") == Some(label) {
                break;
            }
            text.extend_from_slice(line);
        }
        found.push((label, STANDARD.decode(text).ok()?));
    }
    Some(found)
}

/// `element` for a value of one tag, with what follows it.
fn tagged(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (found, body, rest) = element(input)?;
    (found == tag).then_some((body, rest))
}

/// A SEQUENCE opening with `count` integers, what PKCS#1 and traditional DSA keys are by shape.
/// Node reads no further, so other primes or trailing members may follow.
fn opens_with_integers(der: &[u8], count: usize) -> bool {
    let Some((mut rest, _)) = tagged(der, SEQUENCE) else {
        return false;
    };
    (0..count).all(|_| {
        let after = tagged(rest, INTEGER).map(|(_, after)| after);
        rest = after.unwrap_or_default();
        after.is_some()
    })
}

/// A SEC1 ECPrivateKey by shape: Node keeps a scalar or point the crates would refuse.
fn sec1_shaped(der: &[u8]) -> bool {
    tagged(der, SEQUENCE)
        .and_then(|(key, _)| tagged(key, INTEGER))
        .and_then(|(_, rest)| tagged(rest, OCTET_STRING))
        .is_some()
}

/// A PKCS#8 key's algorithm OID when its key body has the shape that algorithm needs. Any other
/// algorithm passes on its outer shape: the crates refuse it, but it is a key to Node.
fn pkcs8_oid(der: &[u8]) -> Option<&[u8]> {
    let (info, _) = tagged(der, SEQUENCE)?;
    let (_, rest) = tagged(info, INTEGER)?;
    let (algorithm, rest) = tagged(rest, SEQUENCE)?;
    let (key, _) = tagged(rest, OCTET_STRING)?;
    let (oid, _) = tagged(algorithm, OID)?;
    let shaped = match oid {
        OID_RSA | OID_RSA_PSS => opens_with_integers(key, RSA_INTEGERS),
        OID_EC => sec1_shaped(key),
        OID_ED25519 => tagged(key, OCTET_STRING).is_some_and(|(seed, _)| seed.len() == 32),
        _ => true,
    };
    shaped.then_some(oid)
}

/// An EncryptedPrivateKeyInfo opens with a SEQUENCE where a plain key has its version.
fn encrypted_shaped(der: &[u8]) -> bool {
    tagged(der, SEQUENCE).is_some_and(|(info, _)| info.first() == Some(&SEQUENCE))
}

/// The label a private block reads under, None when its body is no key for that label. PKCS#8 is
/// accepted under a label of its own type, and takes the plain label for the parsers after this.
fn private_label(label: &str, der: &[u8]) -> Option<&'static str> {
    let der = first_value(der);
    let oid = pkcs8_oid(der);
    let shaped = |ok: bool, label| ok.then_some(label);
    match label {
        "ENCRYPTED PRIVATE KEY" if oid.is_some() => Some("PRIVATE KEY"),
        "ENCRYPTED PRIVATE KEY" => Some("ENCRYPTED PRIVATE KEY"),
        "PRIVATE KEY" => shaped(oid.is_some() || encrypted_shaped(der), "PRIVATE KEY"),
        "RSA PRIVATE KEY" if oid == Some(OID_RSA) => Some("PRIVATE KEY"),
        "RSA PRIVATE KEY" => shaped(opens_with_integers(der, RSA_INTEGERS), "RSA PRIVATE KEY"),
        "EC PRIVATE KEY" if oid == Some(OID_EC) => Some("PRIVATE KEY"),
        "EC PRIVATE KEY" => shaped(sec1_shaped(der), "EC PRIVATE KEY"),
        "DSA PRIVATE KEY" => shaped(opens_with_integers(der, DSA_INTEGERS), "DSA PRIVATE KEY"),
        _ => None,
    }
}

/// Whether a block that is not the key may stand beside it: a label OpenSSL knows, with its shape.
fn clean(label: &str, der: &[u8]) -> bool {
    let der = first_value(der);
    match label {
        "PUBLIC KEY" => spki_shaped(der),
        "RSA PUBLIC KEY" => opens_with_integers(der, RSA_PUBLIC_INTEGERS),
        "EC PARAMETERS" => tagged(der, OID).is_some(),
        label if CERTIFICATES.contains(&label) => certificate_spki(der).is_some_and(spki_shaped),
        label => private_label(label, der).is_some(),
    }
}

impl KeyInput {
    /// The block a private read opens: the first key block, with only clean blocks before it.
    pub(crate) fn private_pem(&self) -> Option<Pem> {
        for (label, der) in blocks(self.bytes())? {
            if let Some(label) = private_label(label, &der) {
                return Some(Pem { label, der });
            }
            if !clean(label, &der) {
                return None;
            }
        }
        None
    }

    /// The block a public read opens. A block that is not clean refuses, since a later block can
    /// outrank an earlier one; a lone block goes on to its parser, whose refusals are Node's.
    pub(crate) fn public_pem(&self) -> Option<Pem> {
        let blocks = blocks(self.bytes())?;
        if blocks.len() > 1 && !blocks.iter().all(|(label, der)| clean(label, der)) {
            return None;
        }
        for labels in PUBLIC_PASSES {
            let block = blocks.iter().find_map(|(found, der)| {
                let label = labels.iter().find(|label| *label == found)?;
                Some(Pem {
                    label,
                    der: der.clone(),
                })
            });
            if block.is_some() {
                return block;
            }
        }
        self.private_pem()
    }

    /// Whether a public read of this key ends on a public block, so no private block is opened.
    pub(crate) fn holds_public(&self) -> bool {
        self.public_pem()
            .is_some_and(|pem| PUBLIC_PASSES.iter().any(|l| l.contains(&pem.label)))
    }
}

fn spki_shaped(der: &[u8]) -> bool {
    tagged(der, SEQUENCE)
        .and_then(|(spki, _)| tagged(spki, SEQUENCE))
        .is_some_and(|(_, rest)| tagged(rest, BIT_STRING).is_some())
}
