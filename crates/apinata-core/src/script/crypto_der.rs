//! Canonical DER for the keys a KeyObject holds. OpenSSL rebuilds every key it reads, so an
//! encoding it accepts but DER forbids (long-form lengths, padded integers, unused bits) exports
//! as the canonical one, and the crates, which read DER strictly, see that form too.

use super::crypto::element;

pub(super) const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;
const CONSTRUCTED: u8 = 0x20;

/// One DER value with a minimal length.
pub(super) fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
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

/// An INTEGER holding the unsigned big-endian `magnitude`, which may be empty for zero.
pub(super) fn uint(magnitude: &[u8]) -> Vec<u8> {
    let zeros = magnitude.iter().take_while(|&&b| b == 0).count();
    let value = &magnitude[zeros..];
    let pad = value.first().is_none_or(|b| b & 0x80 != 0);
    let body = [if pad { &[0u8][..] } else { &[] }, value].concat();
    tlv(INTEGER, &body)
}

/// A SubjectPublicKeyInfo of an encoded algorithm identifier and a key with no unused bits.
pub(super) fn spki(algorithm: &[u8], key: &[u8]) -> Vec<u8> {
    let bits = [&[0u8][..], key].concat();
    tlv(SEQUENCE, &[algorithm, &tlv(BIT_STRING, &bits)].concat())
}

/// An algorithm identifier: the OID's contents and the one parameter value, if any.
pub(super) fn algorithm(oid: &[u8], params: Option<&[u8]>) -> Vec<u8> {
    tlv(
        SEQUENCE,
        &[&tlv(OID, oid), params.unwrap_or_default()].concat(),
    )
}

/// The pieces of a SubjectPublicKeyInfo as OpenSSL reads them.
pub(super) struct SpkiParts<'a> {
    pub(super) oid: &'a [u8],
    /// The one parameter value after the OID, with its tag and length.
    pub(super) params: Option<&'a [u8]>,
    /// The key bits, with the unused bits of the last byte cleared.
    pub(super) key: Vec<u8>,
}

/// Reads a SubjectPublicKeyInfo with the tags, member count and BIT STRING rules OpenSSL holds
/// it to; None for anything else.
pub(super) fn spki_parts(der: &[u8]) -> Option<SpkiParts<'_>> {
    let (SEQUENCE, body, _) = element(der)? else {
        return None;
    };
    let (SEQUENCE, identifier, rest) = element(body)? else {
        return None;
    };
    let (BIT_STRING, bits, after) = element(rest)? else {
        return None;
    };
    let (OID, oid, params) = element(identifier)? else {
        return None;
    };
    if !after.is_empty() || element(params).is_some_and(|(.., tail)| !tail.is_empty()) {
        return None;
    }
    if !params.is_empty() && element(params).is_none() {
        return None;
    }
    let (&unused, key) = bits.split_first()?;
    if unused > 7 || (unused > 0 && key.is_empty()) {
        return None;
    }
    let mut key = key.to_vec();
    if let Some(last) = key.last_mut() {
        *last &= 0xff << unused;
    }
    Some(SpkiParts {
        oid,
        params: (!params.is_empty()).then_some(params),
        key,
    })
}

/// `der`'s first value re-encoded with minimal lengths and integers, going into constructed
/// values only, so primitive key bytes stay as they are; None when it does not parse.
pub(super) fn canonical(der: &[u8]) -> Option<Vec<u8>> {
    let (tag, body, _) = element(der)?;
    let body = match tag {
        INTEGER => {
            let redundant = |b: &[u8]| match b {
                [0x00, next, ..] => next & 0x80 == 0,
                [0xff, next, ..] => next & 0x80 != 0,
                _ => false,
            };
            let mut value = body;
            while redundant(value) {
                value = &value[1..];
            }
            value.to_vec()
        }
        tag if tag & CONSTRUCTED != 0 => {
            let mut rest = body;
            let mut out = Vec::with_capacity(body.len());
            while !rest.is_empty() {
                let (.., after) = element(rest)?;
                out.extend(canonical(rest)?);
                rest = after;
            }
            out
        }
        _ => body.to_vec(),
    };
    Some(tlv(tag, &body))
}

/// `canonical` for a PrivateKeyInfo, which also goes into the DER key its third member wraps.
pub(super) fn canonical_pkcs8(der: &[u8]) -> Option<Vec<u8>> {
    let (SEQUENCE, body, _) = element(der)? else {
        return None;
    };
    let mut rest = body;
    let mut out = Vec::with_capacity(body.len());
    for index in 0.. {
        if rest.is_empty() {
            break;
        }
        let (tag, value, after) = element(rest)?;
        let wraps_one = element(value).is_some_and(|(.., tail)| tail.is_empty());
        if index == 2 && tag == OCTET_STRING && wraps_one {
            out.extend(tlv(OCTET_STRING, &canonical(value)?));
        } else {
            out.extend(canonical(rest)?);
        }
        rest = after;
    }
    Some(tlv(SEQUENCE, &out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_take_their_shortest_form() {
        assert_eq!(uint(&[]), [2, 1, 0]);
        assert_eq!(uint(&[0, 0, 5]), [2, 1, 5]);
        assert_eq!(uint(&[0x81]), [2, 2, 0, 0x81]);
        assert_eq!(canonical(&[2, 3, 0, 0, 5]), Some(vec![2, 1, 5]));
        assert_eq!(canonical(&[2, 2, 0xff, 0x80]), Some(vec![2, 1, 0x80]));
    }

    #[test]
    fn long_lengths_shrink_inside_sequences() {
        let long = [0x30, 0x82, 0, 4, 2, 0x81, 1, 7];
        assert_eq!(canonical(&long), Some(vec![0x30, 3, 2, 1, 7]));
        assert_eq!(canonical(&[0x30, 3, 2, 1]), None);
    }

    #[test]
    fn unused_bits_are_cleared_and_bounded() {
        let key = |unused: u8, bits: &[u8]| {
            let bits = tlv(BIT_STRING, &[&[unused][..], bits].concat());
            let der = tlv(SEQUENCE, &[algorithm(&[1], None), bits].concat());
            spki_parts(&der).map(|p| p.key)
        };
        assert_eq!(key(1, &[0x0f]), Some(vec![0x0e]));
        assert_eq!(key(7, &[0xff]), Some(vec![0x80]));
        assert_eq!(key(8, &[0xff]), None);
        assert_eq!(key(1, &[]), None);
    }
}
