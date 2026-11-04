//! Node's `crypto` key derivation, AES-CBC, AES-CTR and AES-GCM ciphers, RSA (PKCS#1 v1.5 and
//! PSS), ECDSA and Ed25519 signatures, and key object encoding, behind `crypto.js`. Encrypted
//! keys and JWK live in `crypto_keys.rs`.
//!
//! The shim validates arguments and owns Buffers; these host functions take and return bytes
//! and throw Node's error types. Messages name the problem, never key bytes.

use std::num::NonZeroU32;

use aws_lc_rs::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::cipher::{
    self, DecryptingKey, DecryptionContext, EncryptingKey, EncryptionContext, UnboundCipherKey,
};
use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::digest::{self, Digest};
use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der};
use aws_lc_rs::iv::FixedLength;
use aws_lc_rs::pbkdf2;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    self, EcdsaKeyPair, EcdsaSigningAlgorithm, EcdsaVerificationAlgorithm, Ed25519KeyPair, KeyPair,
    RsaKeyPair, UnparsedPublicKey,
};
use rquickjs::convert::List;
use rquickjs::{Ctx, Exception, Function};
use rsa::pkcs1::der::Encode;
use rsa::pkcs1::der::asn1::BitStringRef;
use rsa::pkcs1::{DecodeRsaPrivateKey, EncodeRsaPublicKey};
use rsa::pkcs8::spki::SubjectPublicKeyInfoRef;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use rsa::rand_core::OsRng;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Sign, Pss, RsaPrivateKey, RsaPublicKey};

use super::crypto_keys::{
    OPENED_UNSUPPORTED, UNREADABLE_PUBLIC, decrypted, jwk_key_host, jwk_parts_host,
    public_readable, rsa_pkcs1_fits, rsa_pkcs8_fits, throw,
};
use super::engine::Installer;
use super::jwt::{KeyInput, ec_private_curve};
use crate::oauth1::SHA1_PREFIX;

const CRYPTO_JS: &str = include_str!("crypto.js");

/// PBKDF2 and key-length caps: host calls cannot be interrupted by the script timeout.
pub(super) const MAX_ITERATIONS: u32 = 10_000_000;
const MAX_KEYLEN: u32 = 65_536;
/// Caps iterations x output blocks for PBKDF2 and N x r x p for scrypt.
pub(super) const MAX_PBKDF2_WORK: u64 = 1 << 24;
pub(super) const MAX_SCRYPT_WORK: f64 = 33_554_432.0;
/// The largest scrypt `maxmem` the shim accepts.
pub(super) const MAX_SCRYPT_MEMORY: f64 = 268_435_456.0;

const SIG_SUPPORTED: &str = "(supported: RSA with sha1, sha256, sha384 or sha512; RSA-PSS with sha256, sha384 or sha512; ECDSA P-256, P-384 or P-521 with sha1, sha256, sha384 or sha512; Ed25519 with null; require('crypto-js') has more)";
const BAD_PRIVATE: &str = "Unsupported key: expected a PEM private key (PKCS#8, PKCS#1 or SEC1)";
const BAD_PUBLIC: &str =
    "Unsupported key: expected a PEM public key (SPKI, PKCS#1 or X.509 certificate) or private key";
/// The salt length crypto.js passes when a PSS sign names none, unlike an explicit -1.
const PSS_DEFAULT_SALT: i64 = -3;
/// The RSA modulus sizes that sign and verify, in bits.
pub(super) const RSA_BITS: std::ops::RangeInclusive<usize> = 1024..=8192;

/// Installs the host functions and `crypto.js`, which adds them to the `crypto` object.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        let g = ctx.globals();
        let equal = |a: Vec<u8>, b: Vec<u8>| verify_slices_are_equal(&a, &b).is_ok();
        g.set("__timingSafeEqual", Function::new(ctx.clone(), equal)?)?;
        g.set("__pbkdf2", Function::new(ctx.clone(), pbkdf2_host)?)?;
        g.set("__scrypt", Function::new(ctx.clone(), scrypt_host)?)?;
        g.set("__cipher", Function::new(ctx.clone(), cipher_host)?)?;
        g.set("__sign", Function::new(ctx.clone(), sign_host)?)?;
        g.set("__verify", Function::new(ctx.clone(), verify_host)?)?;
        g.set("__keyObject", Function::new(ctx.clone(), key_object_host)?)?;
        g.set("__jwkKey", Function::new(ctx.clone(), jwk_key_host)?)?;
        g.set("__jwkParts", Function::new(ctx.clone(), jwk_parts_host)?)?;
        ctx.eval::<(), _>(CRYPTO_JS)
    })
}

/// `keylen` bytes of PBKDF2-HMAC over `digest` (sha1, sha256, sha384 or sha512).
fn pbkdf2_host<'js>(
    ctx: Ctx<'js>,
    digest: String,
    password: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    keylen: u32,
) -> rquickjs::Result<Vec<u8>> {
    let (algorithm, block) = match digest.as_str() {
        "sha1" => (pbkdf2::PBKDF2_HMAC_SHA1, 20),
        "sha256" => (pbkdf2::PBKDF2_HMAC_SHA256, 32),
        "sha384" => (pbkdf2::PBKDF2_HMAC_SHA384, 48),
        "sha512" => (pbkdf2::PBKDF2_HMAC_SHA512, 64),
        _ => {
            return Err(Exception::throw_message(
                &ctx,
                &format!(
                    "Digest method not supported: \"{digest}\" (supported: sha1, sha256, sha384, sha512; require('crypto-js') has more)"
                ),
            ));
        }
    };
    let iterations = NonZeroU32::new(iterations)
        .filter(|n| n.get() <= MAX_ITERATIONS)
        .ok_or_else(|| {
            Exception::throw_range(&ctx, "The value of \"iterations\" is out of range")
        })?;
    if keylen > MAX_KEYLEN {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"keylen\" is out of range",
        ));
    }
    if u64::from(iterations.get()) * u64::from(keylen.div_ceil(block)) > MAX_PBKDF2_WORK {
        return Err(Exception::throw_range(
            &ctx,
            "The pbkdf2 work (iterations x ceil(keylen / digest length)) must be <= 16777216",
        ));
    }
    let mut out = vec![0u8; keylen as usize];
    if !out.is_empty() {
        pbkdf2::derive(algorithm, iterations, &salt, &password, &mut out);
    }
    Ok(out)
}

/// `keylen` bytes of scrypt with cost `n` (a power of two above 1), block size `r` and
/// parallelization `p`; the shim has already applied Node's memory limit.
fn scrypt_host<'js>(
    ctx: Ctx<'js>,
    password: Vec<u8>,
    salt: Vec<u8>,
    keylen: u32,
    n: f64,
    r: u32,
    p: u32,
) -> rquickjs::Result<Vec<u8>> {
    let invalid = || Exception::throw_range(&ctx, "Invalid scrypt params");
    if keylen > MAX_KEYLEN {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"keylen\" is out of range",
        ));
    }
    // OpenSSL's estimate, which Node checks against `maxmem`.
    if n * f64::from(r) * f64::from(p) > MAX_SCRYPT_WORK {
        return Err(Exception::throw_range(
            &ctx,
            "Invalid scrypt params: N * r * p must be <= 33554432",
        ));
    }
    let memory = 128.0 * f64::from(r) * (n + 2.0 + f64::from(p));
    if memory > MAX_SCRYPT_MEMORY {
        return Err(Exception::throw_range(
            &ctx,
            "Invalid scrypt params: memory limit exceeded",
        ));
    }
    let log_n = n.log2();
    if !(n > 1.0 && log_n.fract() == 0.0 && log_n < 64.0) {
        return Err(invalid());
    }
    let params = scrypt::Params::new(log_n as u8, r, p).map_err(|_| invalid())?;
    let mut out = vec![0u8; keylen as usize];
    if !out.is_empty() {
        scrypt::scrypt(&password, &salt, &params, &mut out).map_err(|_| invalid())?;
    }
    Ok(out)
}

/// One AES-CBC, AES-CTR or AES-GCM operation. `spec` is the cipher name, a space, and
/// `encrypt` or `decrypt`, with `-nopad` when auto padding is off. A CTR spec runs either way:
/// it XORs `data` with the keystream from the counter block `iv`. GCM encryption returns the
/// ciphertext followed by the 16-byte tag; GCM decryption checks `tag` and `aad`.
fn cipher_host<'js>(
    ctx: Ctx<'js>,
    spec: String,
    key: Vec<u8>,
    iv: Vec<u8>,
    data: Vec<u8>,
    aad: Vec<u8>,
    tag: Vec<u8>,
) -> rquickjs::Result<Vec<u8>> {
    let (algorithm, op) = spec.split_once(' ').unwrap_or((spec.as_str(), ""));
    let (key_len, mode) = match algorithm.split_at_checked(8) {
        Some(("aes-128-", mode)) => (16, mode),
        Some(("aes-192-", mode)) => (24, mode),
        Some(("aes-256-", mode)) => (32, mode),
        _ => return Err(Exception::throw_message(&ctx, "Unknown cipher")),
    };
    let iv_len = match mode {
        "cbc" => cipher::AES_CBC_IV_LEN,
        "ctr" => cipher::AES_CTR_IV_LEN,
        "gcm" => aead::NONCE_LEN,
        _ => return Err(Exception::throw_message(&ctx, "Unknown cipher")),
    };
    if key.len() != key_len {
        return Err(Exception::throw_range(&ctx, "Invalid key length"));
    }
    if iv.len() != iv_len {
        return Err(Exception::throw_type(&ctx, "Invalid initialization vector"));
    }
    let encrypt = op.starts_with("encrypt");
    let result = match mode {
        "gcm" => gcm_run(key_len, encrypt, &key, &iv, data, &aad, &tag),
        "ctr" => ctr_run(key_len, &key, &iv, data),
        _ => cbc_run(key_len, encrypt, !op.ends_with("-nopad"), &key, &iv, data),
    };
    result.map_err(|message| Exception::throw_message(&ctx, message))
}

const AUTH_FAILED: &str = "Unsupported state or unable to authenticate data";

fn gcm_run(
    key_len: usize,
    encrypt: bool,
    key: &[u8],
    iv: &[u8],
    mut data: Vec<u8>,
    aad: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, &'static str> {
    let algorithm = match key_len {
        16 => &aead::AES_128_GCM,
        24 => &aead::AES_192_GCM,
        _ => &aead::AES_256_GCM,
    };
    let key = LessSafeKey::new(UnboundKey::new(algorithm, key).map_err(|_| "Invalid key length")?);
    let nonce =
        Nonce::try_assume_unique_for_key(iv).map_err(|_| "Invalid initialization vector")?;
    if encrypt {
        let tag = key
            .seal_in_place_separate_tag(nonce, Aad::from(aad), &mut data)
            .map_err(|_| AUTH_FAILED)?;
        data.extend_from_slice(tag.as_ref());
        return Ok(data);
    }
    if tag.len() != aead::MAX_TAG_LEN {
        return Err(AUTH_FAILED);
    }
    data.extend_from_slice(tag);
    let plain = key
        .open_in_place(nonce, Aad::from(aad), &mut data)
        .map_err(|_| AUTH_FAILED)?;
    Ok(plain.to_vec())
}

/// AES-CTR from the counter block `iv`; encryption and decryption are the same keystream XOR.
fn ctr_run(
    key_len: usize,
    key: &[u8],
    iv: &[u8],
    mut data: Vec<u8>,
) -> Result<Vec<u8>, &'static str> {
    let algorithm = match key_len {
        16 => &cipher::AES_128,
        24 => &cipher::AES_192,
        _ => &cipher::AES_256,
    };
    let iv: FixedLength<16> = iv.try_into().map_err(|_| "Invalid initialization vector")?;
    if data.is_empty() {
        return Ok(data);
    }
    UnboundCipherKey::new(algorithm, key)
        .and_then(EncryptingKey::ctr)
        .and_then(|k| k.less_safe_encrypt(&mut data, EncryptionContext::Iv128(iv)))
        .map_err(|_| "bad encrypt")?;
    Ok(data)
}

/// AES-CBC over whole blocks, with PKCS#7 padding added or checked here when `pad` is set.
fn cbc_run(
    key_len: usize,
    encrypt: bool,
    pad: bool,
    key: &[u8],
    iv: &[u8],
    mut data: Vec<u8>,
) -> Result<Vec<u8>, &'static str> {
    const BLOCK: usize = 16;
    let algorithm = match key_len {
        16 => &cipher::AES_128,
        24 => &cipher::AES_192,
        _ => &cipher::AES_256,
    };
    let iv: FixedLength<BLOCK> = iv.try_into().map_err(|_| "Invalid initialization vector")?;
    let key = UnboundCipherKey::new(algorithm, key).map_err(|_| "Invalid key length")?;
    if encrypt && pad {
        let fill = BLOCK - data.len() % BLOCK;
        data.resize(data.len() + fill, fill as u8);
    }
    if !data.len().is_multiple_of(BLOCK) || (!encrypt && pad && data.is_empty()) {
        return Err("wrong final block length");
    }
    if data.is_empty() {
        return Ok(data);
    }
    if encrypt {
        EncryptingKey::cbc(key)
            .and_then(|k| k.less_safe_encrypt(&mut data, EncryptionContext::Iv128(iv)))
            .map_err(|_| "bad encrypt")?;
        return Ok(data);
    }
    let decrypting = DecryptingKey::cbc(key).map_err(|_| "bad decrypt")?;
    decrypting
        .decrypt(&mut data, DecryptionContext::Iv128(iv))
        .map_err(|_| "bad decrypt")?;
    if pad {
        let fill = usize::from(data[data.len() - 1]);
        let valid = (1..=BLOCK).contains(&fill)
            && data[data.len() - fill..]
                .iter()
                .all(|&b| usize::from(b) == fill);
        if !valid {
            return Err("bad decrypt");
        }
        data.truncate(data.len() - fill);
    }
    Ok(data)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Curve {
    P256,
    P384,
    P521,
}

/// A signing key: RSA and Ed25519 parsed up front, EC kept as DER since the signature
/// encoding picks the aws-lc-rs algorithm it is parsed with. RSA keeps its DER too, for the
/// sha1 path aws-lc-rs cannot sign.
pub(super) enum Private {
    Rsa {
        key: RsaPrivate,
        der: Vec<u8>,
        pkcs1: bool,
    },
    Ec {
        curve: Curve,
        der: Vec<u8>,
        sec1: bool,
    },
    Ed(Ed25519KeyPair),
}

/// aws-lc-rs holds the RSA keys it accepts (2048 to 8192 bits); the `rsa` crate holds the rest,
/// so a size outside the supported range is named when the key is used, not when it is read.
/// Integers over the top of the range stop at the read, before either crate works on them.
pub(super) enum RsaPrivate {
    Native(RsaKeyPair),
    Fallback(Box<RsaPrivateKey>),
}

/// A verifying key as bytes aws-lc-rs reads: SPKI, PKCS#1, an EC point or a raw Ed25519 key.
pub(super) enum Public {
    Rsa(Vec<u8>),
    Ec(Curve, Vec<u8>),
    Ed(Vec<u8>),
}

fn unsupported(what: &str) -> String {
    format!("Signature algorithm not supported: \"{what}\" {SIG_SUPPORTED}")
}

fn curve(name: &str) -> Curve {
    match name {
        "prime256v1" => Curve::P256,
        "secp384r1" => Curve::P384,
        _ => Curve::P521,
    }
}

fn private_key(key: &KeyInput) -> Result<Private, String> {
    decrypted(key, None)?;
    let pem = key.private_pem().ok_or(BAD_PRIVATE)?;
    private_from(pem.label, pem.der)
}

pub(super) fn private_from(label: &str, der: Vec<u8>) -> Result<Private, String> {
    let der = first_value(&der).to_vec();
    match label {
        "RSA PRIVATE KEY" => {
            rsa_pkcs1_fits(&der)?;
            let key = match RsaKeyPair::from_der(&der) {
                Ok(pair) => RsaPrivate::Native(pair),
                Err(_) => RsaPrivateKey::from_pkcs1_der(&der)
                    .map(|k| RsaPrivate::Fallback(Box::new(k)))
                    .map_err(|_| BAD_PRIVATE)?,
            };
            Ok(Private::Rsa {
                key,
                der,
                pkcs1: true,
            })
        }
        "EC PRIVATE KEY" => match ec_private_curve(&der, true) {
            Some(name) => Ok(Private::Ec {
                curve: curve(name),
                der,
                sec1: true,
            }),
            None => Err(BAD_PRIVATE.into()),
        },
        _ => {
            rsa_pkcs8_fits(&der)?;
            let rsa = match RsaKeyPair::from_pkcs8(&der) {
                Ok(pair) => Some(RsaPrivate::Native(pair)),
                Err(_) => RsaPrivateKey::from_pkcs8_der(&der)
                    .ok()
                    .map(|k| RsaPrivate::Fallback(Box::new(k))),
            };
            if let Some(key) = rsa {
                return Ok(Private::Rsa {
                    key,
                    der,
                    pkcs1: false,
                });
            }
            if let Some(name) = ec_private_curve(&der, false) {
                return Ok(Private::Ec {
                    curve: curve(name),
                    der,
                    sec1: false,
                });
            }
            Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der)
                .map(Private::Ed)
                .map_err(|_| BAD_PRIVATE.into())
        }
    }
}

/// The PKCS#1 public key of an RSA private key.
fn rsa_public_pkcs1(key: &RsaPrivate) -> Result<Vec<u8>, &'static str> {
    match key {
        RsaPrivate::Native(pair) => Ok(pair.public_key().as_ref().to_vec()),
        RsaPrivate::Fallback(key) => key
            .to_public_key()
            .to_pkcs1_der()
            .map(|d| d.into_vec())
            .map_err(|_| BAD_PUBLIC),
    }
}

/// The modulus and exponent of a PKCS#1 or SubjectPublicKeyInfo RSA public key.
pub(super) fn rsa_public_parts(der: &[u8]) -> Option<(BigUint, BigUint)> {
    let (_, mut body, _) = element(der)?;
    if body.first() == Some(&0x30) {
        // SubjectPublicKeyInfo: past the algorithm, a bit string wraps the PKCS#1 key.
        let (_, _, rest) = element(body)?;
        let (tag, bits, _) = element(rest)?;
        if tag != 0x03 {
            return None;
        }
        body = element(bits.strip_prefix(&[0u8])?)?.1;
    }
    let (n_tag, n, rest) = element(body)?;
    let (e_tag, e, after) = element(rest)?;
    (n_tag == 0x02 && e_tag == 0x02 && after.is_empty())
        .then(|| (BigUint::from_bytes_be(n), BigUint::from_bytes_be(e)))
}

/// Refuses an RSA modulus outside the supported sizes, naming its size.
pub(super) fn rsa_size(bits: usize) -> Result<(), String> {
    if RSA_BITS.contains(&bits) {
        return Ok(());
    }
    Err(format!(
        "Unsupported key: a {bits}-bit RSA key (supported: {} to {} bits)",
        RSA_BITS.start(),
        RSA_BITS.end()
    ))
}

pub(super) fn key_bytes(key: &KeyInput) -> &[u8] {
    match key {
        KeyInput::Text(s) => s.as_bytes(),
        KeyInput::Bytes(b) => b,
    }
}

/// The SubjectPublicKeyInfo a public PEM block holds, None for a private block. Every path that
/// reads a public key comes through here, so verify refuses what createPublicKey refuses.
fn public_block(label: &str, der: &[u8]) -> Result<Option<(Public, Vec<u8>)>, String> {
    let spki = match label {
        "PUBLIC KEY" => first_value(der).to_vec(),
        "RSA PUBLIC KEY" => rsa_spki(first_value(der))?,
        "CERTIFICATE" | "X509 CERTIFICATE" => certificate_spki(der).ok_or(BAD_PUBLIC)?.to_vec(),
        _ => return Ok(None),
    };
    let parsed = spki_key(&spki)?;
    public_readable(&parsed)?;
    Ok(Some((parsed, spki)))
}

/// A PKCS#1 public key wrapped as SubjectPublicKeyInfo. Its integers are not judged here: Node
/// opens a zero modulus or exponent, which neither crate would encode.
fn rsa_spki(pkcs1: &[u8]) -> Result<Vec<u8>, &'static str> {
    let bits = BitStringRef::from_bytes(pkcs1).map_err(|_| UNREADABLE_PUBLIC)?;
    let spki = SubjectPublicKeyInfoRef {
        algorithm: rsa::pkcs1::ALGORITHM_ID,
        subject_public_key: bits,
    };
    spki.to_der().map_err(|_| UNREADABLE_PUBLIC)
}

pub(super) fn public_key(key: &KeyInput) -> Result<Public, String> {
    let pem = key.public_pem().ok_or(BAD_PUBLIC)?;
    if let Some((public, _)) = public_block(pem.label, &pem.der)? {
        return Ok(public);
    }
    decrypted(key, None)?;
    match private_from(pem.label, pem.der) {
        Ok(Private::Rsa { key, .. }) => Ok(Public::Rsa(rsa_public_pkcs1(&key)?)),
        Ok(Private::Ec { curve, der, sec1 }) => {
            let pair = ec_pair(curve, false, &der, sec1).map_err(|_| BAD_PUBLIC)?;
            Ok(Public::Ec(curve, pair.public_key().as_ref().to_vec()))
        }
        Ok(Private::Ed(pair)) => Ok(Public::Ed(pair.public_key().as_ref().to_vec())),
        Err(e) if e == BAD_PRIVATE => Err(BAD_PUBLIC.into()),
        Err(e) => Err(e),
    }
}

/// One DER element of `input`: its tag, its contents, and what follows it.
pub(super) fn element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |a, &b| (a << 8) | usize::from(b));
        (len, &rest[n..])
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

/// `input` without what follows its first DER value: OpenSSL reads that value and stops.
pub(super) fn first_value(input: &[u8]) -> &[u8] {
    element(input).map_or(input, |(.., after)| &input[..input.len() - after.len()])
}

pub(super) const OID_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
pub(super) const OID_EC: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const OID_P521: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x23];
pub(super) const OID_ED25519: &[u8] = &[0x2b, 0x65, 0x70];

/// The key a SubjectPublicKeyInfo holds, picked by its algorithm identifier.
pub(super) fn spki_key(der: &[u8]) -> Result<Public, String> {
    let parsed = || {
        let (_, spki, _) = element(der)?;
        let (_, algorithm, _) = element(spki)?;
        let (_, oid, params) = element(algorithm)?;
        Some((oid, params))
    };
    let (oid, params) = parsed().ok_or(BAD_PUBLIC)?;
    let curve = element(params).map(|(_, curve, _)| curve);
    match (oid, curve) {
        (OID_RSA, _) => Ok(Public::Rsa(der.to_vec())),
        (OID_EC, Some(OID_P256)) => Ok(Public::Ec(Curve::P256, der.to_vec())),
        (OID_EC, Some(OID_P384)) => Ok(Public::Ec(Curve::P384, der.to_vec())),
        (OID_EC, Some(OID_P521)) => Ok(Public::Ec(Curve::P521, der.to_vec())),
        (OID_EC, _) => Err(unsupported("an EC key on an unsupported curve")),
        // Ed25519 takes no parameters, so any bytes here, parsable or not, are refused.
        (OID_ED25519, _) if params.is_empty() => Ok(Public::Ed(der.to_vec())),
        (OID_ED25519, _) => Err(UNREADABLE_PUBLIC.into()),
        _ => Err(BAD_PUBLIC.into()),
    }
}

/// The SubjectPublicKeyInfo of an X.509 certificate, the seventh field of its TBS
/// certificate when the explicit version tag is present.
pub(super) fn certificate_spki(der: &[u8]) -> Option<&[u8]> {
    let (_, certificate, _) = element(der)?;
    let (_, mut tbs, _) = element(certificate)?;
    if tbs.first() == Some(&0xa0) {
        tbs = element(tbs)?.2;
    }
    // Skip serial number, signature algorithm, issuer, validity and subject.
    for _ in 0..5 {
        tbs = element(tbs)?.2;
    }
    let (_, _, rest) = element(tbs)?;
    Some(&tbs[..tbs.len() - rest.len()])
}

/// The hash a scheme uses: an empty name (null in the script) is sha256, as in Node.
fn hash_of(digest: &str) -> Result<&'static digest::Algorithm, String> {
    match digest {
        "" | "sha256" => Ok(&digest::SHA256),
        "sha1" => Ok(&digest::SHA1_FOR_LEGACY_USE_ONLY),
        "sha384" => Ok(&digest::SHA384),
        "sha512" => Ok(&digest::SHA512),
        other => Err(unsupported(other)),
    }
}

/// Each curve signs with its native hash; any other digest is fitted to it below.
fn ec_native(curve: Curve) -> &'static digest::Algorithm {
    match curve {
        Curve::P256 => &digest::SHA256,
        Curve::P384 => &digest::SHA384,
        Curve::P521 => &digest::SHA512,
    }
}

pub(super) fn ec_pair(
    curve: Curve,
    p1363: bool,
    der: &[u8],
    sec1: bool,
) -> Result<EcdsaKeyPair, String> {
    let algorithm: &'static EcdsaSigningAlgorithm = match (curve, p1363) {
        (Curve::P256, false) => &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        (Curve::P256, true) => &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        (Curve::P384, false) => &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
        (Curve::P384, true) => &signature::ECDSA_P384_SHA384_FIXED_SIGNING,
        (Curve::P521, false) => &signature::ECDSA_P521_SHA512_ASN1_SIGNING,
        (Curve::P521, true) => &signature::ECDSA_P521_SHA512_FIXED_SIGNING,
    };
    let pair = if sec1 {
        EcdsaKeyPair::from_private_key_der(algorithm, der)
    } else {
        EcdsaKeyPair::from_pkcs8(algorithm, der)
    };
    pair.map_err(|_| BAD_PRIVATE.into())
}

/// `data` hashed with `digest` and fitted to the curve's native digest length: truncated to
/// its leftmost bytes or left-padded with zeros, which is FIPS 186-5's leftmost-bits rule here.
fn ec_digest(curve: Curve, digest: &str, data: &[u8]) -> Result<Digest, String> {
    let native = ec_native(curve);
    let hash = digest::digest(hash_of(digest)?, data);
    let (hash, len) = (hash.as_ref(), native.output_len());
    let mut fitted = vec![0u8; len];
    if hash.len() >= len {
        fitted.copy_from_slice(&hash[..len]);
    } else {
        fitted[len - hash.len()..].copy_from_slice(hash);
    }
    Digest::import_less_safe(&fitted, native).map_err(|_| BAD_PRIVATE.into())
}

fn rsa_digest(digest: &str) -> Result<&'static dyn signature::RsaEncoding, String> {
    match digest {
        "" | "sha256" => Ok(&signature::RSA_PKCS1_SHA256),
        "sha384" => Ok(&signature::RSA_PKCS1_SHA384),
        "sha512" => Ok(&signature::RSA_PKCS1_SHA512),
        other => Err(unsupported(&format!("{other} with an RSA key"))),
    }
}

/// RSA PKCS#1 v1.5 over SHA-1, which aws-lc-rs only verifies, signed with the `rsa` crate.
fn rsa_sha1_sign(der: &[u8], pkcs1: bool, data: &[u8]) -> Result<Vec<u8>, String> {
    let key = if pkcs1 {
        RsaPrivateKey::from_pkcs1_der(der).map_err(|_| BAD_PRIVATE)?
    } else {
        RsaPrivateKey::from_pkcs8_der(der).map_err(|_| BAD_PRIVATE)?
    };
    crate_sign(&key, "sha1", data, Padding::Pkcs1)
}

/// The PKCS#1 v1.5 digest and the matching `rsa` crate scheme.
fn pkcs1_scheme(digest: &str) -> Result<(&'static digest::Algorithm, Pkcs1v15Sign), String> {
    use rsa::sha2::{Sha256, Sha384, Sha512};
    match digest {
        "sha1" => Ok((
            &digest::SHA1_FOR_LEGACY_USE_ONLY,
            Pkcs1v15Sign {
                hash_len: Some(20),
                prefix: SHA1_PREFIX.into(),
            },
        )),
        "" | "sha256" => Ok((&digest::SHA256, Pkcs1v15Sign::new::<Sha256>())),
        "sha384" => Ok((&digest::SHA384, Pkcs1v15Sign::new::<Sha384>())),
        "sha512" => Ok((&digest::SHA512, Pkcs1v15Sign::new::<Sha512>())),
        other => Err(unsupported(&format!("{other} with an RSA key"))),
    }
}

/// Signs with the `rsa` crate, blinded by the OS RNG, for the keys and digests aws-lc-rs does not
/// sign. A default PSS salt is the digest length, shortened to the largest salt the key fits.
fn crate_sign(
    key: &RsaPrivateKey,
    digest: &str,
    data: &[u8],
    padding: Padding,
) -> Result<Vec<u8>, String> {
    let bits = key.n().bits();
    rsa_size(bits)?;
    let signed = match padding {
        Padding::Pkcs1 => {
            let (hash, scheme) = pkcs1_scheme(digest)?;
            key.sign_with_rng(&mut OsRng, scheme, digest::digest(hash, data).as_ref())
        }
        Padding::Pss(salt) => {
            let (hash, scheme) = pss_digest(digest)?;
            let h_len = hash.output_len();
            let room = (bits - 1).div_ceil(8).checked_sub(h_len + 2);
            let salt = match salt {
                PSS_DEFAULT_SALT => room.map_or(h_len, |r| h_len.min(r)),
                n if n == -1 || n == h_len as i64 => h_len,
                n => return Err(unsupported(&format!("RSA-PSS with saltLength {n}"))),
            };
            if room.is_none_or(|r| salt > r) {
                let name = if digest.is_empty() { "sha256" } else { digest };
                return Err(format!(
                    "Unsupported key: RSA-PSS with {name} and saltLength {salt} does not fit a {bits}-bit RSA key (data too large for key size)"
                ));
            }
            key.sign_with_rng(
                &mut OsRng,
                scheme(salt),
                digest::digest(hash, data).as_ref(),
            )
        }
    };
    signed.map_err(|_| BAD_PRIVATE.into())
}

/// Builds the `rsa` crate's PSS scheme for one digest at a given salt length.
type PssScheme = fn(usize) -> Pss;

/// The PSS digest and the matching `rsa` crate scheme.
fn pss_digest(digest: &str) -> Result<(&'static digest::Algorithm, PssScheme), String> {
    match digest {
        "" | "sha256" => Ok((&digest::SHA256, Pss::new_with_salt::<rsa::sha2::Sha256>)),
        "sha384" => Ok((&digest::SHA384, Pss::new_with_salt::<rsa::sha2::Sha384>)),
        "sha512" => Ok((&digest::SHA512, Pss::new_with_salt::<rsa::sha2::Sha512>)),
        "sha1" => Err(unsupported("sha1 with RSA-PSS padding")),
        other => Err(unsupported(&format!("{other} with an RSA key"))),
    }
}

/// How a script asked for RSA: PKCS#1 v1.5, or PSS with a salt length where -1 is the digest
/// length and -2 is recovered from the signature (Node's `RSA_PSS_SALTLEN_*`), or
/// `PSS_DEFAULT_SALT` when a signing script named none.
#[derive(Clone, Copy)]
pub(super) enum Padding {
    Pkcs1,
    Pss(i64),
}

/// Signs `data`; `digest` is sha1, sha256, sha384, sha512 or empty, `dsa` is `der` or `ieee-p1363`.
pub(super) fn sign(
    digest: &str,
    data: &[u8],
    key: &KeyInput,
    dsa: &str,
    padding: Padding,
) -> Result<Vec<u8>, String> {
    match (private_key(key)?, padding) {
        (
            Private::Rsa {
                key: RsaPrivate::Fallback(key),
                ..
            },
            _,
        ) => crate_sign(&key, digest, data, padding),
        // aws-lc-rs always salts with the digest length, so only that length is offered.
        (
            Private::Rsa {
                key: RsaPrivate::Native(pair),
                ..
            },
            Padding::Pss(salt),
        ) => {
            let (hash, _) = pss_digest(digest)?;
            if ![-1, PSS_DEFAULT_SALT, hash.output_len() as i64].contains(&salt) {
                return Err(unsupported(&format!("RSA-PSS with saltLength {salt}")));
            }
            let encoding: &'static dyn signature::RsaEncoding = match hash.output_len() {
                32 => &signature::RSA_PSS_SHA256,
                48 => &signature::RSA_PSS_SHA384,
                _ => &signature::RSA_PSS_SHA512,
            };
            let mut out = vec![0u8; pair.public_modulus_len()];
            pair.sign(encoding, &SystemRandom::new(), data, &mut out)
                .map_err(|_| BAD_PRIVATE)?;
            Ok(out)
        }
        (Private::Rsa { der, pkcs1, .. }, _) if digest == "sha1" => {
            rsa_sha1_sign(&der, pkcs1, data)
        }
        (
            Private::Rsa {
                key: RsaPrivate::Native(pair),
                ..
            },
            _,
        ) => {
            let encoding = rsa_digest(digest)?;
            let mut out = vec![0u8; pair.public_modulus_len()];
            pair.sign(encoding, &SystemRandom::new(), data, &mut out)
                .map_err(|_| BAD_PRIVATE)?;
            Ok(out)
        }
        (Private::Ec { curve, der, sec1 }, _) => {
            let hashed = ec_digest(curve, digest, data)?;
            let pair = ec_pair(curve, dsa == "ieee-p1363", &der, sec1)?;
            let signature = pair.sign_digest(&hashed).map_err(|_| BAD_PRIVATE)?;
            Ok(signature.as_ref().to_vec())
        }
        (Private::Ed(pair), _) => {
            if !digest.is_empty() {
                return Err(unsupported(&format!("{digest} with an Ed25519 key")));
            }
            Ok(pair.sign(data).as_ref().to_vec())
        }
    }
}

/// The salt length of an RSA-PSS signature, read from its encoded message (RFC 8017 section
/// 9.1.2); None for anything malformed, which then fails verification.
fn pss_salt_len(key: &RsaPublicKey, sig: &[u8], hash: &'static digest::Algorithm) -> Option<usize> {
    let s = BigUint::from_bytes_be(sig);
    if sig.len() != key.size() || &s >= key.n() {
        return None;
    }
    let em_bits = key.n().bits() - 1;
    let em_len = em_bits.div_ceil(8);
    let m = rsa::hazmat::rsa_encrypt(key, &s).ok()?.to_bytes_be();
    let h_len = hash.output_len();
    if m.len() > em_len || em_len < h_len + 2 {
        return None;
    }
    let mut em = vec![0u8; em_len - m.len()];
    em.extend_from_slice(&m);
    if em[em_len - 1] != 0xbc {
        return None;
    }
    let (masked, h) = em[..em_len - 1].split_at(em_len - h_len - 1);
    // MGF1 over H unmasks DB; the bits above emBits are cleared, then zeros lead to the 0x01.
    let mut db = Vec::with_capacity(masked.len() + h_len);
    for counter in 0u32.. {
        if db.len() >= masked.len() {
            break;
        }
        let mut block = digest::Context::new(hash);
        block.update(h);
        block.update(&counter.to_be_bytes());
        db.extend_from_slice(block.finish().as_ref());
    }
    db.truncate(masked.len());
    db.iter_mut().zip(masked).for_each(|(d, m)| *d ^= m);
    db[0] &= 0xff >> (8 * em_len - em_bits);
    let one = db.iter().position(|&b| b != 0)?;
    (db[one] == 0x01).then(|| db.len() - one - 1)
}

/// RSA verification: aws-lc-rs from 2048 bits for PKCS#1 v1.5 and for PSS salted to the digest
/// length, the `rsa` crate for smaller keys and for any other PSS salt, at the asked length or
/// the one the signature carries.
fn rsa_verify(
    der: &[u8],
    digest: &str,
    data: &[u8],
    sig: &[u8],
    padding: Padding,
) -> Result<bool, String> {
    let (n, e) = rsa_public_parts(der).ok_or(BAD_PUBLIC)?;
    // Node opens a key with a zero member, and nothing verifies under one.
    if n.bits() == 0 || e.bits() == 0 {
        return Ok(false);
    }
    let bits = n.bits();
    rsa_size(bits)?;
    let salt = match padding {
        Padding::Pkcs1 if bits >= 2048 => {
            let params = match digest {
                "sha1" => &signature::RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
                "" | "sha256" => &signature::RSA_PKCS1_2048_8192_SHA256,
                "sha384" => &signature::RSA_PKCS1_2048_8192_SHA384,
                "sha512" => &signature::RSA_PKCS1_2048_8192_SHA512,
                other => return Err(unsupported(&format!("{other} with an RSA key"))),
            };
            return Ok(UnparsedPublicKey::new(params, der)
                .verify(data, sig)
                .is_ok());
        }
        Padding::Pkcs1 => None,
        Padding::Pss(salt) => {
            let len = pss_digest(digest)?.0.output_len();
            if bits >= 2048 && (salt == -1 || salt == len as i64) {
                let params = match len {
                    32 => &signature::RSA_PSS_2048_8192_SHA256,
                    48 => &signature::RSA_PSS_2048_8192_SHA384,
                    _ => &signature::RSA_PSS_2048_8192_SHA512,
                };
                return Ok(UnparsedPublicKey::new(params, der)
                    .verify(data, sig)
                    .is_ok());
            }
            Some(salt)
        }
    };
    let key = RsaPublicKey::new_with_max_size(n, e, *RSA_BITS.end()).map_err(|_| BAD_PUBLIC)?;
    let Some(salt) = salt else {
        let (hash, scheme) = pkcs1_scheme(digest)?;
        let hashed = digest::digest(hash, data);
        return Ok(key.verify(scheme, hashed.as_ref(), sig).is_ok());
    };
    // The crate's PSS scheme reduces a signature past the modulus instead of refusing it.
    if &BigUint::from_bytes_be(sig) >= key.n() {
        return Ok(false);
    }
    let (hash, scheme) = pss_digest(digest)?;
    let salt_len = match salt {
        -1 => Some(hash.output_len()),
        -2 => pss_salt_len(&key, sig, hash),
        n => usize::try_from(n).ok(),
    };
    let Some(salt_len) = salt_len else {
        return Ok(false);
    };
    let hashed = digest::digest(hash, data);
    Ok(key.verify(scheme(salt_len), hashed.as_ref(), sig).is_ok())
}

/// Whether `sig` signs `data`; Err only for an unusable key or an unsupported scheme.
pub(super) fn verify(
    digest: &str,
    data: &[u8],
    key: &KeyInput,
    sig: &[u8],
    dsa: &str,
    padding: Padding,
) -> Result<bool, String> {
    let (algorithm, bytes): (&'static dyn signature::VerificationAlgorithm, Vec<u8>) =
        match (public_key(key)?, padding) {
            (Public::Rsa(der), _) => return rsa_verify(&der, digest, data, sig, padding),
            (Public::Ec(curve, der), _) => {
                let hashed = ec_digest(curve, digest, data)?;
                let algorithm: &'static EcdsaVerificationAlgorithm =
                    match (curve, dsa == "ieee-p1363") {
                        (Curve::P256, false) => &signature::ECDSA_P256_SHA256_ASN1,
                        (Curve::P256, true) => &signature::ECDSA_P256_SHA256_FIXED,
                        (Curve::P384, false) => &signature::ECDSA_P384_SHA384_ASN1,
                        (Curve::P384, true) => &signature::ECDSA_P384_SHA384_FIXED,
                        (Curve::P521, false) => &signature::ECDSA_P521_SHA512_ASN1,
                        (Curve::P521, true) => &signature::ECDSA_P521_SHA512_FIXED,
                    };
                return Ok(UnparsedPublicKey::new(algorithm, der)
                    .verify_digest(&hashed, sig)
                    .is_ok());
            }
            (Public::Ed(der), _) => {
                if !digest.is_empty() {
                    return Err(unsupported(&format!("{digest} with an Ed25519 key")));
                }
                (&signature::ED25519, der)
            }
        };
    Ok(UnparsedPublicKey::new(algorithm, bytes)
        .verify(data, sig)
        .is_ok())
}

/// The padding the shim named: `pss` is null for PKCS#1 v1.5, else the PSS salt length.
fn padding_of(pss: Option<f64>) -> Padding {
    pss.map_or(Padding::Pkcs1, |salt| Padding::Pss(salt as i64))
}

fn sign_host<'js>(
    ctx: Ctx<'js>,
    digest: String,
    data: Vec<u8>,
    key: Vec<u8>,
    dsa: String,
    pss: Option<f64>,
) -> rquickjs::Result<Vec<u8>> {
    sign(&digest, &data, &KeyInput::Bytes(key), &dsa, padding_of(pss)).map_err(|e| throw(&ctx, &e))
}

fn verify_host<'js>(
    ctx: Ctx<'js>,
    digest: String,
    data: Vec<u8>,
    key: Vec<u8>,
    sig: Vec<u8>,
    dsa: String,
    pss: Option<f64>,
) -> rquickjs::Result<bool> {
    verify(
        &digest,
        &data,
        &KeyInput::Bytes(key),
        &sig,
        &dsa,
        padding_of(pss),
    )
    .map_err(|e| throw(&ctx, &e))
}

/// A private key as its PKCS#8 DER, or a public key as its SPKI DER, with its Node
/// `asymmetricKeyType`. PKCS#8 and SPKI values are kept byte for byte; other forms are re-encoded.
/// `passphrase` opens an encrypted private key, and is ignored for any other.
pub(super) fn key_object(
    key: &KeyInput,
    public: bool,
    passphrase: Option<&[u8]>,
) -> Result<(&'static str, Vec<u8>), String> {
    if public && key.holds_public() {
        return plain_key_object(key, public);
    }
    // A public read that refuses opens nothing, whatever a private read would decrypt.
    if public && key.public_pem().is_none() {
        decrypted(key, None)?;
        return Err(BAD_PUBLIC.into());
    }
    let Some(plain) = decrypted(key, passphrase)? else {
        return plain_key_object(key, public);
    };
    // A key only the passphrase could reach has opened, so it is no longer a malformed key file.
    plain_key_object(&plain, public).map_err(|e| {
        if e == BAD_PRIVATE || e == BAD_PUBLIC {
            OPENED_UNSUPPORTED.into()
        } else {
            e
        }
    })
}

fn plain_key_object(key: &KeyInput, public: bool) -> Result<(&'static str, Vec<u8>), String> {
    if !public {
        let parsed = private_key(key)?;
        let pem = key.private_pem().ok_or(BAD_PRIVATE)?;
        let kind = match &parsed {
            Private::Rsa { .. } => "rsa",
            Private::Ec { .. } => "ec",
            Private::Ed(_) => "ed25519",
        };
        if pem.label == "PRIVATE KEY" {
            return Ok((kind, first_value(&pem.der).to_vec()));
        }
        let der = match parsed {
            Private::Rsa {
                key: RsaPrivate::Native(pair),
                ..
            } => AsDer::<Pkcs8V1Der>::as_der(&pair)
                .map(|d| d.as_ref().to_vec())
                .map_err(|_| BAD_PRIVATE)?,
            Private::Rsa {
                key: RsaPrivate::Fallback(key),
                ..
            } => key
                .to_pkcs8_der()
                .map(|d| d.as_bytes().to_vec())
                .map_err(|_| BAD_PRIVATE)?,
            Private::Ec { curve, der, sec1 } => ec_pair(curve, false, &der, sec1)?
                .to_pkcs8v1()
                .map(|d| d.as_ref().to_vec())
                .map_err(|_| BAD_PRIVATE)?,
            Private::Ed(_) => pem.der,
        };
        return Ok((kind, der));
    }
    let pem = key.public_pem().ok_or(BAD_PUBLIC)?;
    if let Some((parsed, spki)) = public_block(pem.label, &pem.der)? {
        return Ok((public_kind(&parsed), spki));
    }
    let spki = match private_from(pem.label, pem.der) {
        Ok(Private::Rsa {
            key: RsaPrivate::Native(pair),
            ..
        }) => spki_of(pair.public_key()),
        Ok(Private::Rsa {
            key: RsaPrivate::Fallback(key),
            ..
        }) => key
            .to_public_key()
            .to_public_key_der()
            .map(|d| d.into_vec())
            .map_err(|_| BAD_PUBLIC),
        Ok(Private::Ec { curve, der, sec1 }) => spki_of(
            ec_pair(curve, false, &der, sec1)
                .map_err(|_| BAD_PUBLIC)?
                .public_key(),
        ),
        Ok(Private::Ed(pair)) => spki_of(pair.public_key()),
        Err(e) if e == BAD_PRIVATE => Err(BAD_PUBLIC),
        Err(e) => return Err(e),
    }?;
    Ok((public_kind(&spki_key(&spki)?), spki))
}

fn public_kind(key: &Public) -> &'static str {
    match key {
        Public::Rsa(_) => "rsa",
        Public::Ec(..) => "ec",
        Public::Ed(_) => "ed25519",
    }
}

pub(super) fn spki_of(
    key: &impl AsDer<PublicKeyX509Der<'static>>,
) -> Result<Vec<u8>, &'static str> {
    key.as_der()
        .map(|d| d.as_ref().to_vec())
        .map_err(|_| BAD_PUBLIC)
}

/// `key` is PEM bytes; returns `[asymmetricKeyType, der]` for the shim's KeyObject.
fn key_object_host<'js>(
    ctx: Ctx<'js>,
    key: Vec<u8>,
    public: bool,
    passphrase: Option<Vec<u8>>,
) -> rquickjs::Result<List<(String, Vec<u8>)>> {
    key_object(&KeyInput::Bytes(key), public, passphrase.as_deref())
        .map(|(kind, der)| List((kind.to_string(), der)))
        .map_err(|e| throw(&ctx, &e))
}
