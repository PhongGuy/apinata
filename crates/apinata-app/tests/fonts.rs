//! The embedded fonts are the pinned TrueType files of the two families the theme names.

use apinata_app::theme::FONTS;

fn utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn six_truetype_files_inter_first() {
    assert_eq!(FONTS.len(), 6);
    for (ix, f) in FONTS.iter().enumerate() {
        assert_eq!(&f[..4], &[0, 1, 0, 0], "font {ix} is not TrueType");
    }
    for f in &FONTS[..4] {
        assert!(holds(f, &utf16be("Inter")));
    }
    for f in &FONTS[4..] {
        assert!(holds(f, &utf16be("JetBrains Mono")));
    }
}

#[test]
fn italic_is_the_fourth_inter_file() {
    // The Italic file's subfamily name is "Italic"; the upright weights lack it.
    assert!(holds(FONTS[3], &utf16be("Italic")));
    assert!(!holds(FONTS[0], &utf16be("Italic")));
}
