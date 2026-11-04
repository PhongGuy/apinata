//! The per-install key that digests kept header values: its construction, its id, its load.

use apinata_core::{DigestKey, MemoryStore, SecretKey, SecretStore};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const NAME: &str = "test-digest-key";

fn slot() -> SecretKey {
    SecretKey::new("", "", NAME)
}

fn stored_key(store: &MemoryStore) -> Vec<u8> {
    let text = store.get(&slot()).unwrap().expect("a stored key");
    STANDARD.decode(text.trim()).unwrap()
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).unwrap();
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[test]
fn the_digest_is_hmac_sha256_over_the_lowercase_name_a_nul_and_the_value() {
    let store = MemoryStore::new();
    let key = DigestKey::load(&store, NAME).unwrap();
    let want = STANDARD.encode(hmac(&stored_key(&store), b"set-cookie\0sid=aaa"));
    assert_eq!(key.digest("Set-Cookie", "sid=aaa"), want);
}

#[test]
fn the_id_is_hex_of_the_first_eight_bytes_of_the_id_mac() {
    let store = MemoryStore::new();
    let key = DigestKey::load(&store, NAME).unwrap();
    let mac = hmac(&stored_key(&store), b"apinata key id");
    let want: String = mac[..8].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(key.id(), want);
    assert_eq!(key.id().len(), 16);
}

#[test]
fn names_are_compared_without_case_and_kept_apart() {
    let key = DigestKey::load(&MemoryStore::new(), NAME).unwrap();
    assert_eq!(key.digest("x-a", "v"), key.digest("x-a", "v"));
    assert_eq!(key.digest("X-A", "v"), key.digest("x-a", "v"));
    assert_ne!(key.digest("x-a", "v"), key.digest("x-b", "v"));
    assert_ne!(key.digest("x-a", "v"), key.digest("x-a", "V"));
    // The separator stops "x-a" + "bv" meeting "x-ab" + "v".
    assert_ne!(key.digest("x-a", "bv"), key.digest("x-ab", "v"));
}

#[test]
fn two_installs_digest_differently() {
    let a = DigestKey::load(&MemoryStore::new(), NAME).unwrap();
    let b = DigestKey::load(&MemoryStore::new(), NAME).unwrap();
    assert_ne!(a.id(), b.id());
    assert_ne!(a.digest("x-a", "v"), b.digest("x-a", "v"));
}

#[test]
fn load_creates_the_key_once_and_reads_it_back() {
    let store = MemoryStore::new();
    assert_eq!(store.get(&slot()).unwrap(), None);
    let first = DigestKey::load(&store, NAME).unwrap();
    let kept = store.get(&slot()).unwrap().expect("made on the first load");
    let again = DigestKey::load(&store, NAME).unwrap();
    assert_eq!(again.id(), first.id());
    assert_eq!(again.digest("x-a", "v"), first.digest("x-a", "v"));
    assert_eq!(store.get(&slot()).unwrap(), Some(kept));
}

#[test]
fn a_malformed_stored_key_is_an_error_and_left_alone() {
    let store = MemoryStore::new();
    store.set(&slot(), "not a key").unwrap();
    assert!(DigestKey::load(&store, NAME).is_err());
    assert_eq!(store.get(&slot()).unwrap().as_deref(), Some("not a key"));
}

#[test]
fn debug_shows_the_id_and_never_the_key() {
    let store = MemoryStore::new();
    let key = DigestKey::load(&store, NAME).unwrap();
    let shown = format!("{key:?}");
    let secret = store.get(&slot()).unwrap().unwrap();
    // Exact text, so any further field (bytes, hex, base64 of the key) fails.
    assert_eq!(shown, format!("DigestKey {{ id: {:?} }}", key.id()));
    assert!(!shown.contains(secret.trim()), "{shown}");
}
