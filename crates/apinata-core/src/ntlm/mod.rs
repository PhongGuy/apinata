//! Pure NTLMv2 message building and parsing for HTTP authentication; no network.
//!
//! Type 3 always answers with NTLMv2 (LMv2 plus NT response). It signs with the
//! caller's timestamp, carries no MIC, channel binding or key exchange, and a Type 2
//! without unicode or target info is refused so NTLMv1 and LM are never answered.

use base64::alphabet::STANDARD as STANDARD_ALPHABET;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use hmac::{Hmac, KeyInit, Mac};
use md4::{Digest as _, Md4};
use md5::Md5;

const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";
const NEGOTIATE_UNICODE: u32 = 0x0000_0001;
/// Unicode, request target, NTLM, always sign, extended session security, target info,
/// 128 and 56: the only flags a Type 3 echoes from the server's Type 2.
const FLAG_MASK: u32 = 0xa088_8205;
const TYPE1_FLAGS: u32 = 0xa008_8205;
const TYPE1_LEN: u32 = 32;
const TYPE3_HEADER_LEN: usize = 64;
/// 100 ns ticks between 1601-01-01 and the Unix epoch.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// Credentials for one NTLM exchange.
pub(crate) struct Credentials<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub domain: &'a str,
    pub workstation: &'a str,
}

impl std::fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password_len", &self.password.len())
            .field("domain", &self.domain)
            .field("workstation", &self.workstation)
            .finish()
    }
}

/// The parts of a Type 2 message the client answers.
#[derive(Debug)]
pub(crate) struct Challenge {
    pub flags: u32,
    pub server_challenge: [u8; 8],
    pub target_info: Vec<u8>,
}

/// Why an NTLM message could not be built or read; texts never carry credentials.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NtlmError {
    #[error("the server did not offer NTLM")]
    NotOffered,
    #[error("the server sent no NTLM challenge")]
    NoChallenge,
    #[error("the NTLM challenge is not valid base64")]
    BadBase64,
    #[error("the NTLM challenge has a bad signature")]
    BadSignature,
    #[error("expected an NTLM challenge but got message type {0}")]
    WrongType(u32),
    #[error("the NTLM challenge is truncated")]
    Truncated,
    #[error("the NTLM challenge does not support unicode")]
    NoUnicode,
    #[error("the NTLM challenge has no target info")]
    NoTargetInfo,
    #[error("the {0} is too long")]
    FieldTooLong(&'static str),
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    // HMAC accepts a key of any length, so building the MAC cannot fail.
    #[allow(clippy::expect_used)]
    let mut mac = <Hmac<Md5> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// The fixed Negotiate message: empty domain and workstation buffers at offset 32.
pub(crate) fn type1() -> Vec<u8> {
    let mut m = SIGNATURE.to_vec();
    m.extend(1u32.to_le_bytes());
    m.extend(TYPE1_FLAGS.to_le_bytes());
    for _ in 0..2 {
        m.extend([0u8; 4]);
        m.extend(TYPE1_LEN.to_le_bytes());
    }
    m
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads a Type 2 by offset and length, so a version field or a target name before the
/// target info does not matter.
pub(crate) fn parse_type2(bytes: &[u8]) -> Result<Challenge, NtlmError> {
    if bytes.len() < 12 {
        return Err(NtlmError::Truncated);
    }
    if &bytes[..8] != SIGNATURE {
        return Err(NtlmError::BadSignature);
    }
    let kind = le32(bytes, 8);
    if kind != 2 {
        return Err(NtlmError::WrongType(kind));
    }
    if bytes.len() < 48 {
        return Err(NtlmError::Truncated);
    }
    let flags = le32(bytes, 20);
    if flags & NEGOTIATE_UNICODE == 0 {
        return Err(NtlmError::NoUnicode);
    }
    let mut server_challenge = [0u8; 8];
    server_challenge.copy_from_slice(&bytes[24..32]);
    let len = usize::from(u16::from_le_bytes([bytes[40], bytes[41]]));
    let offset = le32(bytes, 44) as usize;
    if len == 0 {
        return Err(NtlmError::NoTargetInfo);
    }
    let target_info = offset
        .checked_add(len)
        .and_then(|end| bytes.get(offset..end))
        .ok_or(NtlmError::Truncated)?
        .to_vec();
    Ok(Challenge {
        flags,
        server_challenge,
        target_info,
    })
}

/// NTOWFv2: HMAC-MD5 keyed by MD4 of the password over the uppercased user plus domain.
pub(crate) fn ntowf_v2(username: &str, password: &str, domain: &str) -> [u8; 16] {
    let key = Md4::digest(utf16(password));
    let ident = utf16(&format!("{}{}", username.to_uppercase(), domain));
    hmac_md5(&key, &[&ident])
}

fn buffer_len(field: &'static str, data: &[u8]) -> Result<u16, NtlmError> {
    u16::try_from(data.len()).map_err(|_| NtlmError::FieldTooLong(field))
}

/// Builds the Authenticate message from the server's challenge.
pub(crate) fn type3(
    c: &Challenge,
    cred: &Credentials<'_>,
    client_challenge: [u8; 8],
    filetime: u64,
) -> Result<Vec<u8>, NtlmError> {
    let key = ntowf_v2(cred.username, cred.password, cred.domain);

    let mut temp = vec![1u8, 1, 0, 0, 0, 0, 0, 0];
    temp.extend(filetime.to_le_bytes());
    temp.extend(client_challenge);
    temp.extend([0u8; 4]);
    temp.extend(&c.target_info);
    temp.extend([0u8; 4]);

    let proof = hmac_md5(&key, &[&c.server_challenge, &temp]);
    let mut nt = proof.to_vec();
    nt.extend(&temp);
    let mut lm = hmac_md5(&key, &[&c.server_challenge, &client_challenge]).to_vec();
    lm.extend(client_challenge);

    let domain = utf16(cred.domain);
    let user = utf16(cred.username);
    let workstation = utf16(cred.workstation);
    // Payload order is domain, user, workstation, LM, NT; the session key is empty.
    let payload = [&domain, &user, &workstation, &lm, &nt];
    let names = [
        "domain",
        "username",
        "workstation",
        "LM response",
        "NT response",
    ];
    let mut lens = [0u16; 5];
    for (i, p) in payload.iter().enumerate() {
        lens[i] = buffer_len(names[i], p)?;
    }
    let mut offsets = [0u32; 5];
    let mut at = TYPE3_HEADER_LEN;
    for (i, p) in payload.iter().enumerate() {
        offsets[i] = u32::try_from(at).map_err(|_| NtlmError::FieldTooLong(names[i]))?;
        at += p.len();
    }
    let end = u32::try_from(at).map_err(|_| NtlmError::FieldTooLong("message"))?;

    let mut m = SIGNATURE.to_vec();
    m.extend(3u32.to_le_bytes());
    // Header order: LM, NT, domain, user, workstation, then the empty session key.
    for i in [3, 4, 0, 1, 2] {
        m.extend(lens[i].to_le_bytes());
        m.extend(lens[i].to_le_bytes());
        m.extend(offsets[i].to_le_bytes());
    }
    m.extend([0u8; 4]);
    m.extend(end.to_le_bytes());
    m.extend((c.flags & FLAG_MASK).to_le_bytes());
    for p in payload {
        m.extend(p);
    }
    Ok(m)
}

/// A random client challenge from the OS and the current time as a Windows FILETIME.
pub(crate) fn fresh() -> Option<([u8; 8], u64)> {
    use rand::{TryRngCore, rngs::OsRng};
    let mut challenge = [0u8; 8];
    OsRng.try_fill_bytes(&mut challenge).ok()?;
    let micros = u64::try_from(chrono::Utc::now().timestamp_micros()).ok()?;
    Some((challenge, micros * 10 + FILETIME_UNIX_EPOCH))
}

/// The `Authorization` header value for an NTLM message.
pub(crate) fn authorization(message: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "NTLM {}",
        base64::engine::general_purpose::STANDARD.encode(message)
    )
}

/// Each header value may list several challenges; yields `(scheme, token)` pairs.
fn challenges<'a>(headers: &'a [&'a str]) -> impl Iterator<Item = (&'a str, Option<&'a str>)> {
    headers
        .iter()
        .flat_map(|h| h.split(','))
        .filter_map(|part| {
            let mut words = part.split_whitespace();
            let scheme = words.next()?;
            Some((scheme, words.next()))
        })
}

/// Whether any `WWW-Authenticate` value offers the NTLM scheme.
pub(crate) fn offers_ntlm(www_authenticate: &[&str]) -> bool {
    challenges(www_authenticate).any(|(s, _)| s.eq_ignore_ascii_case("ntlm"))
}

/// Decodes the base64 token of the NTLM challenge in the `WWW-Authenticate` values.
pub(crate) fn challenge_blob(www_authenticate: &[&str]) -> Result<Vec<u8>, NtlmError> {
    let mut ntlm = challenges(www_authenticate).filter(|(s, _)| s.eq_ignore_ascii_case("ntlm"));
    let first = ntlm.next().ok_or(NtlmError::NotOffered)?;
    let token = std::iter::once(first)
        .chain(ntlm)
        .find_map(|(_, t)| t)
        .ok_or(NtlmError::NoChallenge)?;
    use base64::Engine as _;
    // Servers differ on trailing padding, so accept both.
    let lenient = GeneralPurpose::new(
        &STANDARD_ALPHABET,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    lenient.decode(token).map_err(|_| NtlmError::BadBase64)
}

#[cfg(test)]
mod tests;
