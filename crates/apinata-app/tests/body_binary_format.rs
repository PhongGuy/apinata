//! The Hex and Base64 formatters behind the binary body views.

use apinata_app::body_text::{
    BINARY_VIEW_CAP, BinaryMode, ROW_WIDTH, base64_text, binary_rows, hex_rows,
};

// The cap is checked at compile time: a whole number of hex rows, never zero.
const _: () = assert!(BINARY_VIEW_CAP > 0 && BINARY_VIEW_CAP.is_multiple_of(16));

#[test]
fn hex_rows_follow_the_reference_layout() {
    let mut bytes = b"Hello, World!".to_vec();
    bytes.extend([0x00, 0x01, 0xff, b'A', b'B']);
    let rows = hex_rows(&bytes);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].as_ref(),
        "00000000: 48 65 6C 6C 6F 2C 20 57 6F 72 6C 64 21 00 01 FF  Hello, World!..."
    );
    assert_eq!(rows[1].as_ref(), format!("00000010: {:<48} AB", "41 42"));
}

#[test]
fn hex_offsets_are_eight_lowercase_digits() {
    let rows = hex_rows(&[0u8; 16 * 11]);
    assert!(rows[10].starts_with("000000a0: "), "{}", rows[10]);
    assert!(hex_rows(&[]).is_empty());
}

#[test]
fn base64_is_standard_with_padding() {
    assert_eq!(base64_text(b"hello"), "aGVsbG8=");
    assert_eq!(base64_text(&[0xfb, 0xff]), "+/8=");
    assert_eq!(base64_text(&[]), "");
}

#[test]
fn binary_rows_stop_at_the_cap() {
    let bytes = vec![7u8; BINARY_VIEW_CAP + 32];
    assert_eq!(
        binary_rows(&bytes, BinaryMode::Hex).len(),
        BINARY_VIEW_CAP / 16
    );
    let joined: String = binary_rows(&bytes, BinaryMode::Base64).concat();
    assert_eq!(joined, base64_text(&bytes[..BINARY_VIEW_CAP]));
    assert!(
        binary_rows(&bytes, BinaryMode::Base64)
            .iter()
            .all(|r| r.len() <= ROW_WIDTH)
    );
}

#[test]
fn an_empty_body_has_no_base64_rows() {
    assert!(binary_rows(&[], BinaryMode::Base64).is_empty());
}

#[test]
fn hex_is_the_default_mode() {
    assert_eq!(BinaryMode::default(), BinaryMode::Hex);
}

#[test]
fn base64_rows_are_mime_lines_that_join_to_the_full_encoding() {
    // 1000 bytes encode to 1336 characters: seventeen full 76-character rows and a short one.
    let bytes: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
    let rows = binary_rows(&bytes, BinaryMode::Base64);
    assert!(rows.iter().all(|r| r.len() <= 76), "a row is wider than 76");
    assert_eq!(rows.len(), 1336_usize.div_ceil(76));
    assert_eq!(rows.concat(), base64_text(&bytes));
}
