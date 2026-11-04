use super::*;
use base64::Engine as _;

const SERVER_CHALLENGE: &str = "0123456789abcdef";
const TARGET_INFO: &str =
    "02000c0044006f006d00610069006e0001000c0053006500720076006500720000000000";
const NTOWF: &str = "0c868a403bfd7a93a3001ef22ef02e3f";
const NT_PROOF: &str = "68cd0ab851e51c96aabc927bebef6a1c";
const TEMP: &str = "01010000000000000000000000000000aaaaaaaaaaaaaaaa0000000002000c0044006f006d00610069006e0001000c005300650072007600650072000000000000000000";
const LM: &str = "86c35097ac9cec102554764a57cccc19aaaaaaaaaaaaaaaa";
const SERVER_FLAGS: u32 = 0xe289_8233;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}
fn sec(len: usize, off: usize) -> Vec<u8> {
    let mut v = (len as u16).to_le_bytes().to_vec();
    v.extend((len as u16).to_le_bytes());
    v.extend((off as u32).to_le_bytes());
    v
}
/// A Type 2 with a version field, so the payload starts at byte 56.
fn type2(flags: u32, name: &[u8], info: &[u8]) -> Vec<u8> {
    let mut m = b"NTLMSSP\0".to_vec();
    m.extend(2u32.to_le_bytes());
    m.extend(sec(name.len(), 56));
    m.extend(flags.to_le_bytes());
    m.extend(unhex(SERVER_CHALLENGE));
    m.extend([0u8; 8]);
    m.extend(sec(info.len(), 56 + name.len()));
    m.extend([0u8; 8]);
    m.extend(name);
    m.extend(info);
    m
}
fn field(msg: &[u8], at: usize) -> Vec<u8> {
    let len = u16::from_le_bytes([msg[at], msg[at + 1]]) as usize;
    let off = u32::from_le_bytes(msg[at + 4..at + 8].try_into().unwrap()) as usize;
    msg[off..off + len].to_vec()
}
fn vector_creds() -> Credentials<'static> {
    Credentials {
        username: "User",
        password: "Password",
        domain: "Domain",
        workstation: "COMPUTER",
    }
}
fn vector_challenge() -> Challenge {
    parse_type2(&type2(SERVER_FLAGS, &utf16("Domain"), &unhex(TARGET_INFO))).unwrap()
}

#[test]
fn ntowf_v2_matches_the_specification_vector() {
    assert_eq!(hex(&ntowf_v2("User", "Password", "Domain")), NTOWF);
}

#[test]
fn ntowf_v2_uppercases_the_user_but_not_the_domain() {
    assert_eq!(
        ntowf_v2("user", "Password", "Domain"),
        ntowf_v2("USER", "Password", "Domain")
    );
    assert_ne!(
        ntowf_v2("User", "Password", "domain"),
        ntowf_v2("User", "Password", "Domain")
    );
    assert_eq!(
        hex(&ntowf_v2("user", "Password", "")),
        "4cf86da43b3cd4785ab26bcee1e1884b"
    );
}

#[test]
fn type1_is_the_fixed_negotiate_message() {
    assert_eq!(type1().len(), 32);
    assert_eq!(
        authorization(&type1()),
        "NTLM TlRMTVNTUAABAAAABYIIoAAAAAAgAAAAAAAAACAAAAA="
    );
}

#[test]
fn type2_parse_reads_flags_challenge_and_target_info() {
    let c = vector_challenge();
    assert_eq!(c.flags, SERVER_FLAGS);
    assert_eq!(hex(&c.server_challenge), SERVER_CHALLENGE);
    assert_eq!(hex(&c.target_info), TARGET_INFO);
}

#[test]
fn type2_without_a_version_field_still_parses() {
    let full = type2(SERVER_FLAGS, &[], &unhex(TARGET_INFO));
    // Drop the 8 version bytes and move the target info offset to 48.
    let mut m = full[..48].to_vec();
    m.extend(&full[56..]);
    m[40..44].copy_from_slice(&sec(TARGET_INFO.len() / 2, 48)[..4]);
    m[44..48].copy_from_slice(&48u32.to_le_bytes());
    assert_eq!(hex(&parse_type2(&m).unwrap().target_info), TARGET_INFO);
}

#[test]
fn type2_failures_are_typed() {
    let good = type2(SERVER_FLAGS, &utf16("Domain"), &unhex(TARGET_INFO));
    let mut bad_sig = good.clone();
    bad_sig[0] = b'X';
    assert_eq!(parse_type2(&bad_sig).unwrap_err(), NtlmError::BadSignature);
    let mut one = good.clone();
    one[8] = 1;
    assert_eq!(parse_type2(&one).unwrap_err(), NtlmError::WrongType(1));
    assert_eq!(parse_type2(&good[..47]).unwrap_err(), NtlmError::Truncated);
    assert_eq!(
        parse_type2(&good[..good.len() - 4]).unwrap_err(),
        NtlmError::Truncated
    );
    assert_eq!(
        parse_type2(&type2(SERVER_FLAGS, &[], &[])).unwrap_err(),
        NtlmError::NoTargetInfo
    );
    assert_eq!(
        parse_type2(&type2(SERVER_FLAGS & !1, &[], &unhex(TARGET_INFO))).unwrap_err(),
        NtlmError::NoUnicode
    );
}

#[test]
fn type3_matches_the_ntlmv2_vector() {
    let msg = type3(&vector_challenge(), &vector_creds(), [0xaa; 8], 0).unwrap();
    assert_eq!(
        &msg[..12],
        &[b"NTLMSSP\0".as_slice(), &3u32.to_le_bytes()].concat()[..]
    );
    assert_eq!(hex(&field(&msg, 12)), LM);
    assert_eq!(hex(&field(&msg, 20)), format!("{NT_PROOF}{TEMP}"));
    assert_eq!(field(&msg, 28), utf16("Domain"));
    assert_eq!(field(&msg, 36), utf16("User"));
    assert_eq!(field(&msg, 44), utf16("COMPUTER"));
    assert!(field(&msg, 52).is_empty());
    assert_eq!(
        u32::from_le_bytes(msg[60..64].try_into().unwrap()),
        0xa088_8201
    );
    // Every byte after the 64-byte header belongs to a buffer.
    let end = [12, 20, 28, 36, 44]
        .iter()
        .map(|&at| {
            u32::from_le_bytes(msg[at + 4..at + 8].try_into().unwrap()) as usize
                + field(&msg, at).len()
        })
        .max()
        .unwrap();
    assert_eq!(end, msg.len());
}

#[test]
fn type3_with_an_empty_domain_sends_an_empty_domain_buffer() {
    let cred = Credentials {
        domain: "",
        workstation: "",
        ..vector_creds()
    };
    let msg = type3(&vector_challenge(), &cred, [0xaa; 8], 0).unwrap();
    assert!(field(&msg, 28).is_empty());
    assert!(field(&msg, 44).is_empty());
    // The NT proof differs: NTOWFv2 was keyed without a domain.
    assert_ne!(hex(&field(&msg, 20)[..16]), NT_PROOF);
}

#[test]
fn type3_encodes_non_ascii_names_by_utf16_code_unit() {
    let cred = Credentials {
        username: "Jos\u{e9}",
        domain: "\u{1f600}",
        ..vector_creds()
    };
    let msg = type3(&vector_challenge(), &cred, [0xaa; 8], 0).unwrap();
    assert_eq!(field(&msg, 36), utf16("Jos\u{e9}"));
    assert_eq!(field(&msg, 28), utf16("\u{1f600}"));
}

#[test]
fn a_changed_timestamp_or_password_changes_the_proof() {
    let a = type3(&vector_challenge(), &vector_creds(), [0xaa; 8], 0).unwrap();
    let b = type3(&vector_challenge(), &vector_creds(), [0xaa; 8], 1).unwrap();
    let wrong = Credentials {
        password: "Passw0rd",
        ..vector_creds()
    };
    let c = type3(&vector_challenge(), &wrong, [0xaa; 8], 0).unwrap();
    assert_ne!(field(&a, 20)[..16], field(&b, 20)[..16]);
    assert_ne!(field(&a, 20)[..16], field(&c, 20)[..16]);
}

#[test]
fn errors_and_debug_carry_no_credentials() {
    let long = "x".repeat(40_000);
    let cred = Credentials {
        username: &long,
        password: "hunter2-secret",
        ..vector_creds()
    };
    let err = type3(&vector_challenge(), &cred, [0xaa; 8], 0).unwrap_err();
    assert_eq!(err, NtlmError::FieldTooLong("username"));
    assert!(err.to_string().contains("username"), "{err}");
    assert!(!err.to_string().contains("xxxx"), "{err}");
    let shown = format!("{cred:?}");
    assert!(!shown.contains("hunter2-secret"), "{shown}");
    assert!(shown.contains("Domain"), "{shown}");
    for e in [
        NtlmError::NotOffered,
        NtlmError::NoChallenge,
        NtlmError::BadBase64,
        NtlmError::BadSignature,
        NtlmError::WrongType(1),
        NtlmError::Truncated,
        NtlmError::NoUnicode,
        NtlmError::NoTargetInfo,
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn fresh_values_are_random_and_a_current_filetime() {
    let (a, ft) = fresh().unwrap();
    let (b, _) = fresh().unwrap();
    assert_ne!(a, b);
    let now = chrono::Utc::now().timestamp() as u64 * 10_000_000 + 116_444_736_000_000_000;
    assert!(ft.abs_diff(now) < 600_000_000, "{ft} vs {now}");
}

#[test]
fn ntlm_is_found_in_www_authenticate_in_any_case_and_inside_a_list() {
    assert!(offers_ntlm(&["NTLM"]));
    assert!(offers_ntlm(&["Basic realm=\"x\"", "ntlm"]));
    assert!(offers_ntlm(&["Negotiate, NTLM"]));
    assert!(offers_ntlm(&["NTLM TlRMTVNTUAAC"]));
    assert!(!offers_ntlm(&["Negotiate abc", "Basic realm=\"ntlm\""]));
    assert!(!offers_ntlm(&["NTLMX"]));
    assert!(!offers_ntlm(&[]));
}

#[test]
fn the_challenge_blob_is_decoded_or_refused_with_a_typed_error() {
    let blob = type2(SERVER_FLAGS, &[], &unhex(TARGET_INFO));
    let header = format!(
        "NTLM {}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    );
    assert_eq!(challenge_blob(&["Negotiate", &header]).unwrap(), blob);
    assert_eq!(
        challenge_blob(&["NTLM"]).unwrap_err(),
        NtlmError::NoChallenge
    );
    assert_eq!(
        challenge_blob(&["NTLM !!!"]).unwrap_err(),
        NtlmError::BadBase64
    );
    assert_eq!(
        challenge_blob(&["Basic x"]).unwrap_err(),
        NtlmError::NotOffered
    );
}
