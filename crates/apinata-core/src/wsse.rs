//! The WSSE `X-WSSE` UsernameToken header, built from a nonce and a creation time
//! the caller supplies, plus the fresh values a send uses.

use base64::Engine as _;
use sha1::Digest as _;

use crate::digest::hex;

/// `UsernameToken Username="<u>", PasswordDigest="<d>", Nonce="<n>", Created="<c>"`, with
/// d = base64(hex(sha1(nonce + created + password))); the username is not escaped.
pub(crate) fn header(username: &str, password: &str, nonce: &str, created: &str) -> String {
    let digest = password_digest(password, nonce, created);
    format!(
        "UsernameToken Username=\"{username}\", PasswordDigest=\"{digest}\", Nonce=\"{nonce}\", Created=\"{created}\""
    )
}

/// The PasswordDigest value alone, which a server may echo without the rest of the header.
pub(crate) fn password_digest(password: &str, nonce: &str, created: &str) -> String {
    // The reference base64-encodes the hex text of the hash, not the raw hash bytes.
    let sha = sha1::Sha1::digest(format!("{nonce}{created}{password}").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hex(&sha))
}

/// 16 OS-random bytes as lowercase hex; `None` when the OS generator fails.
pub(crate) fn nonce() -> Option<String> {
    use rand::{TryRngCore, rngs::OsRng};
    let mut bytes = [0u8; 16];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(hex(&bytes))
}

/// Now in UTC as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub(crate) fn created_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The nonce bytes 00..0f and the clock the vectors injected (vectors note, injection).
    const NONCE: &str = "000102030405060708090a0b0c0d0e0f";
    const CREATED: &str = "2026-10-14T12:00:00.000Z";
    /// Vectors this function must not reproduce, each with its reason.
    const EXCLUDED: &[(&str, &str)] = &[
        (
            "null-fields",
            "deviation 2: a blank field is sent as \"\", never the text null",
        ),
        (
            "yaml-blank-username-no-password",
            "deviation 2: as null-fields",
        ),
        (
            "inherit-folder-edgegrid",
            "an EdgeGrid inheritance case with no X-WSSE header",
        ),
    ];
    /// The inheritance cases' inputs (vectors.mjs, `inherit-*`).
    const INHERITED: &[(&str, &str, &str)] = &[
        ("inherit-folder-wsse", "folder", "fp"),
        ("inherit-folder-inherit-collection-wsse", "coll", "cp"),
    ];

    fn vectors() -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/reference/2026-10-14-m10d-auth-vectors/vectors.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn cases() -> Vec<Value> {
        vectors()["wsse"].as_array().unwrap().clone()
    }

    fn case(id: &str) -> Value {
        cases()
            .into_iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    fn sent(case: &Value) -> String {
        case["headers"]["X-WSSE"]
            .as_str()
            .or(case["xWsse"].as_str())
            .expect("an X-WSSE value")
            .to_owned()
    }

    #[test]
    fn every_vector_header_is_reproduced() {
        let mut checked = 0;
        for case in cases() {
            let id = case["id"].as_str().unwrap();
            if EXCLUDED.iter().any(|(e, _)| *e == id) {
                continue;
            }
            let (user, pass) = match INHERITED.iter().find(|(i, ..)| *i == id) {
                Some((_, u, p)) => ((*u).to_owned(), (*p).to_owned()),
                // An absent field reads as "" in the reference too.
                None => (
                    case["auth"]["username"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    case["auth"]["password"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ),
            };
            assert_eq!(header(&user, &pass, NONCE, CREATED), sent(&case), "{id}");
            checked += 1;
        }
        assert_eq!(checked, 11);
    }

    #[test]
    fn blank_fields_hash_as_empty_never_as_null() {
        for (id, _) in EXCLUDED {
            case(id);
        }
        let blank = header("", "", NONCE, CREATED);
        assert_eq!(blank, sent(&case("absent-fields")));
        assert_ne!(blank, sent(&case("null-fields")));
    }

    #[test]
    fn the_independent_formula_check_agrees() {
        let check = &vectors()["wsseFormulaCheck"];
        let digest = |u: &str, p: &str| {
            let h = header(u, p, NONCE, CREATED);
            let at = h.find("PasswordDigest=\"").unwrap() + 16;
            h[at..at + h[at..].find('"').unwrap()].to_owned()
        };
        assert_eq!(digest("bob", "secret"), check["ascii"].as_str().unwrap());
        assert_eq!(
            digest("b\u{f6}b", "p\u{e4}ss\u{20ac}\u{1F600}"),
            check["nonAscii"].as_str().unwrap()
        );
    }

    #[test]
    fn fresh_values_have_the_reference_shape() {
        let n = nonce().unwrap();
        assert_eq!(n.len(), 32);
        assert!(
            n.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{n}"
        );
        assert_ne!(n, nonce().unwrap());
        let c = created_now();
        assert_eq!(c.len(), 24, "{c}");
        assert!(
            c.ends_with('Z') && chrono::DateTime::parse_from_rfc3339(&c).is_ok(),
            "{c}"
        );
    }
}
