//! RSA-PSS keys (RFC 4055): the parameters such a key is held to, and EMSA-PSS signing and
//! verification (RFC 8017 section 9.1) under the hash and MGF1 hash it names. A key without
//! parameters signs with any digest; one with them takes its own digest and at least its salt.
//! Refusals carry the message OpenSSL gives Node, without its code.

use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::digest;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::rand_core::OsRng;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, RsaPrivateKey, RsaPublicKey};

use super::crypto::{
    BAD_PRIVATE, BAD_PUBLIC, RSA_BITS, element, rsa_public_parts, rsa_size, unsupported,
};

pub(super) const OID_RSA_PSS: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a];
const OID_MGF1: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x08];
/// The hashes an RSA-PSS key may name, in Node's spelling.
const HASHES: [(&str, &[u8]); 7] = [
    ("sha1", &[0x2b, 0x0e, 0x03, 0x02, 0x1a]),
    (
        "sha224",
        &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x04],
    ),
    (
        "sha256",
        &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01],
    ),
    (
        "sha384",
        &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02],
    ),
    (
        "sha512",
        &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03],
    ),
    (
        "sha512-224",
        &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x05],
    ),
    (
        "sha512-256",
        &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x06],
    ),
];

pub(super) const DIGEST_NOT_ALLOWED: &str = "digest not allowed";
pub(super) const ILLEGAL_PADDING: &str = "illegal or unsupported padding mode";
pub(super) const INVALID_SALT: &str = "invalid salt length";
pub(super) const SALT_TOO_SMALL: &str = "pss saltlen too small";
const TOO_LARGE: &str = "data too large for key size";

/// The RSASSA-PSS-params a key carries: its hash, its MGF1 hash and its smallest salt.
#[derive(Clone, Copy)]
pub(super) struct PssParams {
    pub(super) hash: &'static str,
    pub(super) mgf1: &'static str,
    pub(super) salt: usize,
}

/// What a script asked of an RSA key: `padding` 1 or 6 and `saltLength`, each None when unset.
#[derive(Clone, Copy, Default)]
pub(crate) struct Scheme {
    pub(crate) padding: Option<i64>,
    pub(crate) salt: Option<i64>,
}

/// The pieces of a PKCS#8 or SubjectPublicKeyInfo key: its algorithm identifier's whole
/// encoding, OID and parameters, and the key contents after it.
struct Parts<'a> {
    algorithm: &'a [u8],
    oid: &'a [u8],
    params: &'a [u8],
    key: &'a [u8],
}

fn parts(der: &[u8], private: bool) -> Option<Parts<'_>> {
    let (_, mut body, _) = element(der)?;
    if private {
        body = element(body)?.2;
    }
    let (_, algorithm, rest) = element(body)?;
    let whole = &body[..body.len() - rest.len()];
    let (_, oid, params) = element(algorithm)?;
    let (_, key, _) = element(rest)?;
    Some(Parts {
        algorithm: whole,
        oid,
        params,
        key,
    })
}

/// An RSA-PSS key's parameters (None when it has none) and its PKCS#1 key, or the bit string
/// holding it; Err for parameters this reader does not take.
pub(super) type PssKey<'a> = Result<(Option<PssParams>, &'a [u8]), ()>;

/// `PssKey` for an RSA-PSS key, None for any other.
pub(super) fn pss_key(der: &[u8], private: bool) -> Option<PssKey<'_>> {
    let Parts {
        oid, params, key, ..
    } = parts(der, private)?;
    (oid == OID_RSA_PSS).then(|| read_params(params).map(|p| (p, key)))
}

fn hash_name(algorithm: &[u8]) -> Result<&'static str, ()> {
    let (tag, oid, rest) = element(algorithm).ok_or(())?;
    if tag != 0x06 || !(rest.is_empty() || rest == [0x05, 0x00]) {
        return Err(());
    }
    HASHES
        .iter()
        .find(|(_, known)| *known == oid)
        .map(|(name, _)| *name)
        .ok_or(())
}

/// The fields of RSASSA-PSS-params, each optional, in order, and defaulting to SHA-1, MGF1 with
/// SHA-1 and a 20-byte salt; the trailer field may only be 1.
fn read_params(params: &[u8]) -> Result<Option<PssParams>, ()> {
    if params.is_empty() {
        return Ok(None);
    }
    let (tag, mut body, rest) = element(params).ok_or(())?;
    if tag != 0x30 || !rest.is_empty() {
        return Err(());
    }
    let mut found = PssParams {
        hash: "sha1",
        mgf1: "sha1",
        salt: 20,
    };
    let mut last = 0;
    while !body.is_empty() {
        let (field, inner, rest) = element(body).ok_or(())?;
        let (tag, value, after) = element(inner).ok_or(())?;
        if field <= last || !after.is_empty() {
            return Err(());
        }
        (body, last) = (rest, field);
        match (field, tag) {
            (0xa0, 0x30) => found.hash = hash_name(value)?,
            (0xa1, 0x30) => {
                let (oid_tag, oid, hash) = element(value).ok_or(())?;
                let (hash_tag, hash, after) = element(hash).ok_or(())?;
                if oid_tag != 0x06 || oid != OID_MGF1 || hash_tag != 0x30 || !after.is_empty() {
                    return Err(());
                }
                found.mgf1 = hash_name(hash)?;
            }
            (0xa2, 0x02) if value.len() <= 2 && value.first().is_some_and(|b| b & 0x80 == 0) => {
                found.salt = value.iter().fold(0, |a, &b| (a << 8) | usize::from(b));
            }
            (0xa3, 0x02) if value == [1] => {}
            _ => return Err(()),
        }
    }
    Ok(Some(found))
}

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len().to_be_bytes();
    let skip = len.iter().take_while(|&&b| b == 0).count();
    let mut out = vec![tag];
    if body.len() < 0x80 {
        out.push(body.len() as u8);
    } else {
        out.push(0x80 | (len.len() - skip) as u8);
        out.extend_from_slice(&len[skip..]);
    }
    out.extend_from_slice(body);
    out
}

/// The SubjectPublicKeyInfo of an RSA-PSS private key, under the PKCS#8 algorithm identifier.
pub(super) fn public_spki(pkcs8: &[u8], key: &RsaPrivateKey) -> Result<Vec<u8>, &'static str> {
    let algorithm = parts(pkcs8, true).ok_or(BAD_PUBLIC)?.algorithm;
    let public = key.to_public_key().to_pkcs1_der().map_err(|_| BAD_PUBLIC)?;
    let bits = [&[0u8][..], public.as_bytes()].concat();
    Ok(tlv(0x30, &[algorithm, &tlv(0x03, &bits)].concat()))
}

fn algorithm(name: &str) -> Result<&'static digest::Algorithm, String> {
    match name {
        "sha1" => Ok(&digest::SHA1_FOR_LEGACY_USE_ONLY),
        "sha256" => Ok(&digest::SHA256),
        "sha384" => Ok(&digest::SHA384),
        "sha512" => Ok(&digest::SHA512),
        other => Err(unsupported(&format!("{other} with an RSA-PSS key"))),
    }
}

/// The hash and MGF1 hash a signature uses: the key's, which an asked digest must equal, or for
/// a key without parameters the asked digest for both, sha256 when none is named.
fn hashes(
    params: Option<PssParams>,
    digest: &str,
) -> Result<(&'static digest::Algorithm, &'static digest::Algorithm), String> {
    match params {
        Some(p) if !digest.is_empty() && digest != p.hash => Err(DIGEST_NOT_ALLOWED.into()),
        Some(p) => Ok((algorithm(p.hash)?, algorithm(p.mgf1)?)),
        None => {
            let hash = algorithm(if digest.is_empty() { "sha256" } else { digest })?;
            Ok((hash, hash))
        }
    }
}

/// MGF1 (RFC 8017 appendix B.2.1): `len` bytes of hash(seed || counter) blocks.
pub(super) fn mgf1(hash: &'static digest::Algorithm, seed: &[u8], len: usize) -> Vec<u8> {
    let mut mask = Vec::with_capacity(len + hash.output_len());
    for counter in 0u32.. {
        if mask.len() >= len {
            break;
        }
        let mut block = digest::Context::new(hash);
        block.update(seed);
        block.update(&counter.to_be_bytes());
        mask.extend_from_slice(block.finish().as_ref());
    }
    mask.truncate(len);
    mask
}

/// The salt a signature is checked against: one length, or whatever it carries.
enum Salt {
    Exact(usize),
    Any,
}

/// The salt OpenSSL settles on for a key: -1 is the digest length, -2 the largest that fits
/// (refused under parameters), -3 the largest, -4 the digest length if it fits (refused under
/// parameters); unset, the key's own salt or else that -4. Verifying, -2 and -4 accept any.
fn settle(
    params: Option<PssParams>,
    scheme: Scheme,
    h_len: usize,
    max: Option<usize>,
    signing: bool,
) -> Result<Salt, String> {
    let fitted = max.map_or(h_len, |m| h_len.min(m));
    let asked = match (scheme.padding, scheme.salt) {
        (None, None) => match params {
            Some(p) => return Ok(Salt::Exact(p.salt)),
            None if signing => return Ok(Salt::Exact(fitted)),
            None => return Ok(Salt::Any),
        },
        (_, None) => -2,
        (_, Some(n)) => n,
    };
    let salt = match asked {
        -1 => Salt::Exact(h_len),
        -2 | -4 if !signing => Salt::Any,
        -2 | -4 if params.is_some() => return Err(INVALID_SALT.into()),
        -2 | -3 => Salt::Exact(max.ok_or(TOO_LARGE)?),
        -4 => Salt::Exact(fitted),
        n => Salt::Exact(usize::try_from(n).map_err(|_| INVALID_SALT)?),
    };
    match (salt, params) {
        (Salt::Exact(n), Some(p)) if n < p.salt => Err(SALT_TOO_SMALL.into()),
        (salt, _) => Ok(salt),
    }
}

/// The leading bits of an encoded message's first byte that lie above `em_bits`.
fn spare_bits(em_len: usize, em_bits: usize) -> u8 {
    !(0xffu8 >> (8 * em_len - em_bits))
}

/// Signs `data` with an RSA-PSS key, held to its parameters as Node holds it.
pub(super) fn sign(
    key: &RsaPrivateKey,
    params: Option<PssParams>,
    digest: &str,
    scheme: Scheme,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let bits = key.n().bits();
    rsa_size(bits)?;
    let (hash, mask_hash) = hashes(params, digest)?;
    if scheme.padding == Some(1) {
        return Err(ILLEGAL_PADDING.into());
    }
    let (h_len, em_bits) = (hash.output_len(), bits - 1);
    let em_len = em_bits.div_ceil(8);
    let max = em_len.checked_sub(h_len + 2);
    let Salt::Exact(s_len) = settle(params, scheme, h_len, max, true)? else {
        return Err(INVALID_SALT.into());
    };
    if max.is_none_or(|m| s_len > m) {
        return Err(TOO_LARGE.into());
    }
    let mut salt = vec![0u8; s_len];
    SystemRandom::new()
        .fill(&mut salt)
        .map_err(|_| BAD_PRIVATE)?;
    let m_hash = digest::digest(hash, data);
    let mut prime = digest::Context::new(hash);
    prime.update(&[0u8; 8]);
    prime.update(m_hash.as_ref());
    prime.update(&salt);
    let h = prime.finish();
    // DB is zeros, 0x01 and the salt, masked by MGF1 over H; EM is DB, H and 0xbc.
    let mut em = vec![0u8; em_len - h_len - 1];
    let one = em.len() - s_len - 1;
    em[one] = 1;
    em[one + 1..].copy_from_slice(&salt);
    let mask = mgf1(mask_hash, h.as_ref(), em.len());
    em.iter_mut().zip(&mask).for_each(|(b, m)| *b ^= m);
    em[0] &= !spare_bits(em_len, em_bits);
    em.extend_from_slice(h.as_ref());
    em.push(0xbc);
    let signed =
        rsa::hazmat::rsa_decrypt_and_check(key, Some(&mut OsRng), &BigUint::from_bytes_be(&em))
            .map_err(|_| BAD_PRIVATE)?
            .to_bytes_be();
    let mut out = vec![0u8; key.size() - signed.len()];
    out.extend_from_slice(&signed);
    Ok(out)
}

/// Whether `sig` signs `data` under an RSA-PSS public key, its SubjectPublicKeyInfo `der`.
pub(super) fn verify(
    der: &[u8],
    params: Option<PssParams>,
    digest: &str,
    scheme: Scheme,
    data: &[u8],
    sig: &[u8],
) -> Result<bool, String> {
    let (n, e) = rsa_public_parts(der).ok_or(BAD_PUBLIC)?;
    if n.bits() == 0 || e.bits() == 0 {
        return Ok(false);
    }
    let bits = n.bits();
    rsa_size(bits)?;
    let (hash, mask_hash) = hashes(params, digest)?;
    if scheme.padding == Some(1) {
        return Err(ILLEGAL_PADDING.into());
    }
    let (h_len, em_bits) = (hash.output_len(), bits - 1);
    let em_len = em_bits.div_ceil(8);
    let max = em_len.checked_sub(h_len + 2);
    let wanted = settle(params, scheme, h_len, max, false)?;
    let key = RsaPublicKey::new_with_max_size(n, e, *RSA_BITS.end()).map_err(|_| BAD_PUBLIC)?;
    let s = BigUint::from_bytes_be(sig);
    if sig.len() != key.size() || &s >= key.n() || max.is_none() {
        return Ok(false);
    }
    let m = rsa::hazmat::rsa_encrypt(&key, &s)
        .map_err(|_| BAD_PUBLIC)?
        .to_bytes_be();
    if m.len() > em_len {
        return Ok(false);
    }
    let mut em = vec![0u8; em_len - m.len()];
    em.extend_from_slice(&m);
    if em[em_len - 1] != 0xbc || em[0] & spare_bits(em_len, em_bits) != 0 {
        return Ok(false);
    }
    let (masked, h) = em[..em_len - 1].split_at(em_len - h_len - 1);
    let mut db = mgf1(mask_hash, h, masked.len());
    db.iter_mut().zip(masked).for_each(|(d, m)| *d ^= m);
    db[0] &= !spare_bits(em_len, em_bits);
    let Some(one) = db.iter().position(|&b| b != 0) else {
        return Ok(false);
    };
    let salt = &db[one + 1..];
    if db[one] != 0x01 || matches!(wanted, Salt::Exact(n) if n != salt.len()) {
        return Ok(false);
    }
    let m_hash = digest::digest(hash, data);
    let mut prime = digest::Context::new(hash);
    prime.update(&[0u8; 8]);
    prime.update(m_hash.as_ref());
    prime.update(salt);
    Ok(verify_slices_are_equal(prime.finish().as_ref(), h).is_ok())
}
