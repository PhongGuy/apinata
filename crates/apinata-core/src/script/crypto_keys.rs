//! Encrypted PKCS#8 private keys and JWK conversion for the `crypto` shim.
//!
//! Decryption turns a key into plain PKCS#8 before the paths in `crypto.rs` see it. KDF
//! parameters come from an untrusted file, so they meet the script work caps before any
//! derivation. JWK host functions convert field bytes to and from the DER a KeyObject keeps.

use aws_lc_rs::encoding::{AsBigEndian, EcPrivateKeyBin};
use aws_lc_rs::signature::{
    self, EcdsaKeyPair, EcdsaSigningAlgorithm, EcdsaVerificationAlgorithm, Ed25519KeyPair, KeyPair,
    ParsedPublicKey,
};
use pkcs8::der::pem::LineEnding;
use pkcs8::der::zeroize::Zeroizing;
use pkcs8::pkcs5::pbes2::{Kdf, Pbkdf2Prf};
use rquickjs::convert::List;
use rquickjs::{Ctx, Exception};
use rsa::BigUint;
use rsa::pkcs1::UintRef;
use rsa::pkcs1::der::Encode;
use rsa::pkcs8::EncodePublicKey;

use super::crypto::{
    Curve, MAX_ITERATIONS, MAX_PBKDF2_WORK, MAX_SCRYPT_MEMORY, MAX_SCRYPT_WORK, OID_RSA, Private,
    Public, RSA_BITS, ec_pair, element, first_value, key_bytes, key_object, private_from,
    rsa_public_parts, rsa_size, spki_key, spki_of,
};
use super::jwt::KeyInput;

/// Node's message for an encrypted PEM read without a usable passphrase, once OpenSSL's code is dropped.
const NO_PASSPHRASE_PEM: &str = "interrupted or cancelled";
const NO_PASSPHRASE_DER: &str = "Passphrase required for encrypted key";
const BAD_DECRYPT: &str = "bad decrypt";
/// Node's message for DER in the wrong shape, once OpenSSL's code is dropped.
const WRONG_TAG: &str = "wrong tag";
/// OpenSSL refuses a longer passphrase while reading a key.
const MAX_PASSPHRASE: usize = 1024;

const UNREADABLE: &str = "Unsupported key: the encrypted private key could not be read";
pub(super) const OPENED_UNSUPPORTED: &str = "Unsupported key: the encrypted key opened, but it is not a supported type (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519)";
const LEGACY_HEADER: &[u8] = b"Proc-Type: 4,ENCRYPTED";
const LEGACY: &str = "Unsupported key: a legacy encrypted PEM (Proc-Type: 4,ENCRYPTED) (convert it to encrypted PKCS#8)";
const PBES1: &str = "Unsupported key: an encrypted key using a PBES1 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC or AES-GCM)";
const PBKDF2_SHA1: &str = "Unsupported key: an encrypted key using PBKDF2 with HMAC-SHA-1 (supported: PBES2 with PBKDF2 using HMAC-SHA-224 to HMAC-SHA-512, or scrypt, and AES-CBC or AES-GCM)";
const PBES2: &str = "Unsupported key: an encrypted key with an unreadable or unsupported PBES2 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC or AES-GCM)";
const PBKDF2_OVER: &str = "Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)";
const SCRYPT_OVER: &str = "Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)";

const OID_PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];

const INVALID_JWK: &str = "Invalid JWK data";
const INVALID_JWK_EC: &str = "Invalid JWK EC key";
const JWK_KEY_TYPE: &str = "Unsupported JWK Key Type.";
const JWK_MISMATCH: &str = "Unsupported key: the JWK's private and public values do not match";
const JWK_RSA: &str = "Unsupported key: the JWK's RSA values do not form a usable key";
const JWK_COMPRESSED: &str = "Unsupported key: JWK export of a compressed EC public key";
pub(super) const UNREADABLE_PUBLIC: &str = "Failed to read asymmetric key";

/// Throws `message` with the error type Node gives it.
pub(super) fn throw<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    if [NO_PASSPHRASE_DER, INVALID_JWK, INVALID_JWK_EC].contains(&message) {
        Exception::throw_type(ctx, message)
    } else {
        Exception::throw_message(ctx, message)
    }
}

/// `key` as plain PKCS#8 PEM when it is encrypted, None when it is not. Without a passphrase
/// every encrypted key fails, so a path that takes none can call this as its gate.
pub(super) fn decrypted(
    key: &KeyInput,
    passphrase: Option<&[u8]>,
) -> Result<Option<KeyInput>, String> {
    // Legacy headers break the PEM body, so only a key no block can be read from is refused.
    let legacy = key_bytes(key)
        .windows(LEGACY_HEADER.len())
        .any(|w| w == LEGACY_HEADER);
    if legacy && key.public_pem().is_none() {
        return Err(LEGACY.into());
    }
    let Some(pem) = key.private_pem() else {
        return Ok(None);
    };
    if !["ENCRYPTED PRIVATE KEY", "PRIVATE KEY"].contains(&pem.label) {
        return Ok(None);
    }
    // The shim labels DER input by its `type`, and pkcs8 DER may be either form: a plain key
    // opens with an INTEGER, an encrypted one with a SEQUENCE.
    let from_der = pem.label == "PRIVATE KEY";
    let der = first_value(&pem.der);
    let opens_with_sequence = element(der).is_some_and(|(_, body, _)| body.first() == Some(&0x30));
    if from_der && !opens_with_sequence {
        return Ok(None);
    }
    let (_, info, _) = element(der).ok_or(UNREADABLE)?;
    let (_, algorithm, data) = element(info).ok_or(UNREADABLE)?;
    let (_, oid, _) = element(algorithm).ok_or(UNREADABLE)?;
    // Encrypted data is an OCTET STRING; a BIT STRING here is SPKI handed over as PKCS#8.
    if element(data).is_some_and(|(tag, ..)| tag != 0x04) {
        return Err(WRONG_TAG.into());
    }
    if oid != OID_PBES2 {
        return Err(PBES1.into());
    }
    let info = pkcs8::EncryptedPrivateKeyInfoRef::try_from(der).map_err(|_| PBES2)?;
    let scheme = info.encryption_algorithm.pbes2().ok_or(PBES1)?;
    check_work(&scheme.kdf, scheme.encryption.key_size())?;
    if let Kdf::Pbkdf2(params) = &scheme.kdf
        && matches!(params.prf, Pbkdf2Prf::HmacWithSha1)
    {
        return Err(PBKDF2_SHA1.into());
    }
    let missing = if from_der {
        NO_PASSPHRASE_DER
    } else {
        NO_PASSPHRASE_PEM
    };
    let passphrase = passphrase.ok_or(missing)?;
    if passphrase.len() > MAX_PASSPHRASE {
        return Err(NO_PASSPHRASE_PEM.into());
    }
    let plain = info
        .encryption_algorithm
        .decrypt(passphrase, info.encrypted_data.as_ref())
        .map(Zeroizing::new)
        .map_err(|e| match e {
            pkcs8::pkcs5::Error::DecryptFailed => BAD_DECRYPT,
            _ => PBES2,
        })?;
    // As OpenSSL does, read the one DER value and ignore what follows it.
    let (.., after) = element(&plain)
        .filter(|(tag, ..)| *tag == 0x30)
        .ok_or(BAD_DECRYPT)?;
    let key = &plain[..plain.len() - after.len()];
    pkcs8::der::pem::encode_string("PRIVATE KEY", LineEnding::LF, key)
        .map(|text| Some(KeyInput::Text(text)))
        .map_err(|_| UNREADABLE.into())
}

/// Holds a key file's KDF parameters to the caps scripts get. This runs before the crate reads
/// them: the crate cannot be interrupted, and its scrypt setup does not survive N = 0.
fn check_work(kdf: &Kdf, key_size: usize) -> Result<(), &'static str> {
    match kdf {
        Kdf::Pbkdf2(params) => {
            let digest: usize = match params.prf {
                Pbkdf2Prf::HmacWithSha1 => 20,
                Pbkdf2Prf::HmacWithSha224 => 28,
                Pbkdf2Prf::HmacWithSha256 => 32,
                Pbkdf2Prf::HmacWithSha384 => 48,
                Pbkdf2Prf::HmacWithSha512 => 64,
                _ => return Err(PBES2),
            };
            let iterations = params.iteration_count;
            let work = u64::from(iterations) * key_size.div_ceil(digest) as u64;
            let within = (1..=MAX_ITERATIONS).contains(&iterations) && work <= MAX_PBKDF2_WORK;
            within.then_some(()).ok_or(PBKDF2_OVER)
        }
        Kdf::Scrypt(params) => {
            let n = params.cost_parameter;
            let (r, p) = (
                f64::from(params.block_size),
                f64::from(params.parallelization),
            );
            let cost = n as f64;
            // OpenSSL's memory estimate, the one the shim holds script calls to.
            let within = n.is_power_of_two()
                && n > 1
                && r >= 1.0
                && p >= 1.0
                && cost * r * p <= MAX_SCRYPT_WORK
                && 128.0 * r * (cost + 2.0 + p) <= MAX_SCRYPT_MEMORY;
            within.then_some(()).ok_or(SCRYPT_OVER)
        }
        _ => Err(PBES2),
    }
}

/// The field size and algorithms of a JWK curve.
fn jwk_curve(
    name: &str,
) -> Option<(
    usize,
    &'static EcdsaVerificationAlgorithm,
    &'static EcdsaSigningAlgorithm,
)> {
    match name {
        "P-256" => Some((
            32,
            &signature::ECDSA_P256_SHA256_ASN1,
            &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        )),
        "P-384" => Some((
            48,
            &signature::ECDSA_P384_SHA384_ASN1,
            &signature::ECDSA_P384_SHA384_ASN1_SIGNING,
        )),
        "P-521" => Some((
            66,
            &signature::ECDSA_P521_SHA512_ASN1,
            &signature::ECDSA_P521_SHA512_ASN1_SIGNING,
        )),
        _ => None,
    }
}

fn crv(curve: Curve) -> &'static str {
    match curve {
        Curve::P256 => "P-256",
        Curve::P384 => "P-384",
        Curve::P521 => "P-521",
    }
}

fn trimmed(bytes: &[u8]) -> &[u8] {
    let zeros = bytes.iter().take_while(|&&b| b == 0).count();
    &bytes[zeros..]
}

/// `value` as a big-endian integer of exactly `size` bytes, None when it does not fit.
fn field(value: &[u8], size: usize) -> Option<Vec<u8>> {
    let value = trimmed(value);
    let mut out = vec![0u8; size.checked_sub(value.len())?];
    out.extend_from_slice(value);
    Some(out)
}

/// A JWK as the DER a KeyObject keeps, with its `asymmetricKeyType`. `name` is `RSA`, a curve
/// or `Ed25519`; `parts` are the decoded members the shim picked for that name and `public`.
fn jwk_key(name: &str, parts: &[Vec<u8>], public: bool) -> Result<(&'static str, Vec<u8>), String> {
    if let Some((size, verifying, signing)) = jwk_curve(name) {
        return jwk_ec(parts, public, size, verifying, signing);
    }
    match (name, parts) {
        ("RSA", [n, e]) if public => jwk_rsa_public(n, e),
        ("RSA", parts) if !public => jwk_rsa_private(parts),
        ("Ed25519", [value]) if value.len() == 32 && public => {
            let key = ParsedPublicKey::new(&signature::ED25519, value).map_err(|_| INVALID_JWK)?;
            Ok(("ed25519", spki_of(&key)?))
        }
        // As in Node, the seed is the key: `x` is not read, so it cannot disagree.
        ("Ed25519", [value]) if value.len() == 32 => {
            let pair = Ed25519KeyPair::from_seed_unchecked(value).map_err(|_| INVALID_JWK)?;
            let der = pair.to_pkcs8v1().map_err(|_| INVALID_JWK)?;
            Ok(("ed25519", der.as_ref().to_vec()))
        }
        _ => Err(INVALID_JWK.into()),
    }
}

fn jwk_ec(
    parts: &[Vec<u8>],
    public: bool,
    size: usize,
    verifying: &'static EcdsaVerificationAlgorithm,
    signing: &'static EcdsaSigningAlgorithm,
) -> Result<(&'static str, Vec<u8>), String> {
    let [x, y, rest @ ..] = parts else {
        return Err(INVALID_JWK_EC.into());
    };
    let (x, y) = field(x, size).zip(field(y, size)).ok_or(INVALID_JWK_EC)?;
    let point = [&[0x04], x.as_slice(), y.as_slice()].concat();
    let key = ParsedPublicKey::new(verifying, &point).map_err(|_| INVALID_JWK_EC)?;
    if public {
        return Ok(("ec", spki_of(&key)?));
    }
    // Node keeps a `d` that does not belong to the point; no key here can hold that pair.
    let d = rest
        .first()
        .and_then(|d| field(d, size))
        .ok_or(JWK_MISMATCH)?;
    let pair = EcdsaKeyPair::from_private_key_and_public_key(signing, &d, &point)
        .map_err(|_| JWK_MISMATCH)?;
    let der = pair.to_pkcs8v1().map_err(|_| JWK_MISMATCH)?;
    Ok(("ec", der.as_ref().to_vec()))
}

fn jwk_rsa_public(n: &[u8], e: &[u8]) -> Result<(&'static str, Vec<u8>), String> {
    let (n, e) = (BigUint::from_bytes_be(n), BigUint::from_bytes_be(e));
    let der = rsa::RsaPublicKey::new_with_max_size(n, e, *RSA_BITS.end())
        .ok()
        .and_then(|key| key.to_public_key_der().ok())
        .ok_or(JWK_RSA)?;
    Ok(("rsa", der.into_vec()))
}

/// Refuses integers over the RSA size cap. The crates' key checks grow with their size and a
/// host call cannot be interrupted, so this runs before either crate sees them.
fn rsa_integers_fit<'a>(integers: impl Iterator<Item = &'a [u8]>) -> Result<(), String> {
    let bits = |value: &[u8]| {
        let value = trimmed(value);
        value
            .first()
            .map_or(0, |first| value.len() * 8 - first.leading_zeros() as usize)
    };
    match integers.map(bits).max() {
        Some(largest) if largest > *RSA_BITS.end() => rsa_size(largest),
        _ => Ok(()),
    }
}

/// `rsa_integers_fit` for the integers of a PKCS#1 RSAPrivateKey.
pub(super) fn rsa_pkcs1_fits(der: &[u8]) -> Result<(), String> {
    let mut rest = element(der).map_or(&[][..], |(_, body, _)| body);
    rsa_integers_fit(std::iter::from_fn(|| {
        let (tag, value, after) = element(rest)?;
        rest = after;
        (tag == 0x02).then_some(value)
    }))
}

/// `rsa_pkcs1_fits` for a PKCS#8 key, which only an `rsaEncryption` key can fail.
pub(super) fn rsa_pkcs8_fits(der: &[u8]) -> Result<(), String> {
    pkcs8_rsa_key(der).map_or(Ok(()), rsa_pkcs1_fits)
}

/// Encodes the members as PKCS#1 and reads that back through the normal key path. Node keeps
/// values that do not form a key, and a fallback read recomputes the CRT members, so what
/// comes out must equal what went in.
fn jwk_rsa_private(parts: &[Vec<u8>]) -> Result<(&'static str, Vec<u8>), String> {
    let [n, e, d, p, q, dp, dq, qi] = parts else {
        return Err(JWK_RSA.into());
    };
    rsa_integers_fit(parts.iter().map(Vec::as_slice))?;
    fn uint(bytes: &[u8]) -> rsa::pkcs1::der::Result<UintRef<'_>> {
        UintRef::new(trimmed(bytes))
    }
    let encoded = (|| {
        rsa::pkcs1::RsaPrivateKey {
            modulus: uint(n)?,
            public_exponent: uint(e)?,
            private_exponent: uint(d)?,
            prime1: uint(p)?,
            prime2: uint(q)?,
            exponent1: uint(dp)?,
            exponent2: uint(dq)?,
            coefficient: uint(qi)?,
            other_prime_infos: None,
        }
        .to_der()
    })()
    .map_err(|_| JWK_RSA)?;
    let pem = rsa::pkcs1::der::pem::encode_string("RSA PRIVATE KEY", LineEnding::LF, &encoded)
        .map_err(|_| JWK_RSA)?;
    let (kind, der) =
        key_object(&KeyInput::Text(pem), false, None).map_err(|_| JWK_RSA.to_string())?;
    let held = rsa_private_parts(&der).ok_or(JWK_RSA)?;
    let same = held
        .iter()
        .zip(parts)
        .all(|(held, given)| held.as_slice() == trimmed(given));
    same.then_some((kind, der)).ok_or_else(|| JWK_RSA.into())
}

/// The PKCS#1 RSAPrivateKey inside a PKCS#8 key, None unless it is plain `rsaEncryption`.
fn pkcs8_rsa_key(der: &[u8]) -> Option<&[u8]> {
    let (_, info, _) = element(der)?;
    let (_, _, rest) = element(info)?;
    let (_, algorithm, rest) = element(rest)?;
    let (_, oid, _) = element(algorithm)?;
    let (_, key, _) = element(rest)?;
    (oid == OID_RSA).then_some(key)
}

/// The eight RSAPrivateKey integers of a PKCS#8 key, None unless it is plain `rsaEncryption`.
pub(super) fn rsa_private_parts(der: &[u8]) -> Option<Vec<Vec<u8>>> {
    let (_, key, _) = element(pkcs8_rsa_key(der)?)?;
    let (_, _, mut rest) = element(key)?;
    let mut parts = Vec::with_capacity(8);
    for _ in 0..8 {
        let (tag, value, after) = element(rest)?;
        if tag != 0x02 {
            return None;
        }
        parts.push(trimmed(value).to_vec());
        rest = after;
    }
    Some(parts)
}

/// The key bytes of a SubjectPublicKeyInfo.
fn spki_bits(der: &[u8]) -> Option<&[u8]> {
    let (_, spki, _) = element(der)?;
    let (_, _, rest) = element(spki)?;
    let (tag, bits, after) = element(rest)?;
    if tag != 0x03 || !after.is_empty() {
        return None;
    }
    bits.split_first().map(|(_, key)| key)
}

/// Refuses a SubjectPublicKeyInfo Node cannot read. A KeyObject keeps these bytes as they came,
/// so this is what stops a bad point or length reaching a JWK. Ed25519 points are not
/// checked, as in Node: any 32 bytes are kept.
pub(super) fn public_readable(key: &Public) -> Result<(), &'static str> {
    let readable = match key {
        Public::Rsa(der) => spki_bits(der)
            .and(rsa_public_parts(der).map(|_| ()))
            .is_some(),
        Public::Ec(curve, der) => jwk_curve(crv(*curve))
            .is_some_and(|(_, verifying, _)| ParsedPublicKey::new(verifying, der).is_ok()),
        Public::Ed(der) => spki_bits(der).is_some_and(|key| key.len() == 32),
    };
    readable.then_some(()).ok_or(UNREADABLE_PUBLIC)
}

/// An uncompressed or hybrid EC point as its x and y; a hybrid prefix also says y's parity.
fn point_parts(point: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let body = match point.split_first() {
        Some((0x04 | 0x06 | 0x07, body)) => body,
        _ => return Err(JWK_COMPRESSED.into()),
    };
    let (x, y) = body.split_at(body.len() / 2);
    Ok(vec![x.to_vec(), y.to_vec()])
}

/// A KeyObject's DER as JWK member bytes in the order the shim names them, after its curve name:
/// RSA n, e then d, p, q, dp, dq, qi; EC x, y then d; Ed25519 d then x, or x alone.
fn jwk_parts(der: &[u8], private: bool) -> Result<(&'static str, Vec<Vec<u8>>), String> {
    if !private {
        return match spki_key(der)? {
            Public::Rsa(der) => {
                let (n, e) = rsa_public_parts(&der).ok_or(JWK_KEY_TYPE)?;
                let member = |value: BigUint| trimmed(&value.to_bytes_be()).to_vec();
                Ok(("", vec![member(n), member(e)]))
            }
            Public::Ec(curve, der) => {
                let point = spki_bits(&der).ok_or(JWK_KEY_TYPE)?;
                Ok((crv(curve), point_parts(point)?))
            }
            Public::Ed(der) => {
                let x = spki_bits(&der).ok_or(JWK_KEY_TYPE)?;
                Ok(("Ed25519", vec![x.to_vec()]))
            }
        };
    }
    match private_from("PRIVATE KEY", der.to_vec())? {
        Private::Rsa { der, .. } => Ok(("", rsa_private_parts(&der).ok_or(JWK_KEY_TYPE)?)),
        Private::Ec { curve, der, sec1 } => {
            let pair = ec_pair(curve, false, &der, sec1)?;
            let mut parts = point_parts(pair.public_key().as_ref())?;
            let d = AsBigEndian::<EcPrivateKeyBin>::as_be_bytes(&pair.private_key())
                .map_err(|_| JWK_KEY_TYPE)?;
            parts.push(d.as_ref().to_vec());
            Ok((crv(curve), parts))
        }
        Private::Ed(pair) => {
            let seed = pair.seed().map_err(|_| JWK_KEY_TYPE)?;
            let seed = seed.as_be_bytes().map_err(|_| JWK_KEY_TYPE)?;
            let x = pair.public_key().as_ref().to_vec();
            Ok(("Ed25519", vec![seed.as_ref().to_vec(), x]))
        }
    }
}

/// `[asymmetricKeyType, der]` for a JWK's decoded members.
pub(super) fn jwk_key_host<'js>(
    ctx: Ctx<'js>,
    name: String,
    parts: Vec<Vec<u8>>,
    public: bool,
) -> rquickjs::Result<List<(String, Vec<u8>)>> {
    jwk_key(&name, &parts, public)
        .map(|(kind, der)| List((kind.to_string(), der)))
        .map_err(|e| throw(&ctx, &e))
}

/// `[curve name, members]` for a KeyObject's DER.
pub(super) fn jwk_parts_host<'js>(
    ctx: Ctx<'js>,
    der: Vec<u8>,
    private: bool,
) -> rquickjs::Result<List<(String, Vec<Vec<u8>>)>> {
    jwk_parts(&der, private)
        .map(|(curve, parts)| List((curve.to_string(), parts)))
        .map_err(|e| throw(&ctx, &e))
}

#[cfg(test)]
mod tests {
    use pkcs8::pkcs5::pbes2::{Pbkdf2Params, Salt, ScryptParams};

    use super::*;

    fn pbkdf2(iteration_count: u32, prf: Pbkdf2Prf) -> Kdf {
        Kdf::Pbkdf2(Pbkdf2Params {
            salt: Salt::new([1u8; 8]).expect("salt"),
            iteration_count,
            key_length: None,
            prf,
        })
    }

    fn scrypt(cost_parameter: u64, block_size: u16, parallelization: u16) -> Kdf {
        Kdf::Scrypt(ScryptParams {
            salt: Salt::new([1u8; 8]).expect("salt"),
            cost_parameter,
            block_size,
            parallelization,
            key_length: None,
        })
    }

    #[test]
    fn pbkdf2_work_stops_exactly_at_the_script_caps() {
        let sha256 = Pbkdf2Prf::HmacWithSha256;
        assert_eq!(check_work(&pbkdf2(10_000_000, sha256), 32), Ok(()));
        assert_eq!(check_work(&pbkdf2(1, sha256), 16), Ok(()));
        assert_eq!(
            check_work(&pbkdf2(10_000_001, sha256), 32),
            Err(PBKDF2_OVER)
        );
        assert_eq!(check_work(&pbkdf2(0, sha256), 32), Err(PBKDF2_OVER));
        // A digest shorter than the key takes more than one block per iteration.
        let sha1 = Pbkdf2Prf::HmacWithSha1;
        assert_eq!(check_work(&pbkdf2(8_388_608, sha1), 32), Ok(()));
        assert_eq!(check_work(&pbkdf2(8_388_609, sha1), 32), Err(PBKDF2_OVER));
    }

    #[test]
    fn scrypt_work_stops_exactly_at_the_script_caps() {
        assert_eq!(check_work(&scrypt(1 << 17, 8, 1), 32), Ok(()));
        assert_eq!(check_work(&scrypt(1 << 18, 8, 1), 32), Err(SCRYPT_OVER));
        assert_eq!(check_work(&scrypt(2, 1, 1), 32), Ok(()));
        assert_eq!(check_work(&scrypt(2, 1, 40_000), 32), Ok(()));
        assert_eq!(check_work(&scrypt(1 << 16, 1, 1024), 32), Err(SCRYPT_OVER));
        for (n, r, p) in [(0, 8, 1), (1, 8, 1), (3, 8, 1), (16, 0, 1), (16, 8, 0)] {
            assert_eq!(check_work(&scrypt(n, r, p), 32), Err(SCRYPT_OVER), "{n}");
        }
        assert_eq!(check_work(&scrypt(1 << 63, 1, 1), 32), Err(SCRYPT_OVER));
    }
}
