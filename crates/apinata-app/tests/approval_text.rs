//! The text the approval modal shows: agent text defanged and bounded, IDN hosts flagged.

use apinata_app::approval_prompt::{host_note, untrusted_text};

#[test]
fn agent_text_is_defanged_and_bounded() {
    assert_eq!(untrusted_text("users/get.yml"), "users/get.yml");
    // One character from each stripped range: C0 and C1 controls, U+061C, U+200B-U+200F,
    // U+2028-U+202E, U+2060-U+206F, U+FEFF.
    for c in [
        '\n', '\u{1b}', '\u{85}', '\u{61c}', '\u{200b}', '\u{200f}', '\u{2028}', '\u{202e}',
        '\u{2060}', '\u{2066}', '\u{206f}', '\u{feff}',
    ] {
        assert_eq!(
            untrusted_text(&format!("a{c}b")),
            "a\u{fffd}b",
            "U+{:04X}",
            c as u32
        );
    }
    // Neighbours just outside the ranges stay.
    assert_eq!(
        untrusted_text("b\u{fc}cher\u{2010}\u{2030}"),
        "b\u{fc}cher\u{2010}\u{2030}"
    );
    assert_eq!(
        untrusted_text(&"\u{e9}".repeat(250)),
        format!("{}...", "\u{e9}".repeat(200))
    );
    assert_eq!(untrusted_text(&"\u{e9}".repeat(200)), "\u{e9}".repeat(200));
}

#[test]
fn tags_and_fillers_are_defanged() {
    // One character from each: tag characters U+E0000-U+E007F, soft hyphen U+00AD, the
    // Hangul fillers U+115F, U+1160, U+3164, U+FFA0, and the combining grapheme joiner U+034F.
    for c in [
        '\u{e0041}',
        '\u{ad}',
        '\u{115f}',
        '\u{1160}',
        '\u{3164}',
        '\u{ffa0}',
        '\u{34f}',
    ] {
        assert_eq!(
            untrusted_text(&format!("a{c}b")),
            "a\u{fffd}b",
            "U+{:04X}",
            c as u32
        );
    }
}

#[test]
fn variation_selectors_are_removed() {
    // U+FE00-U+FE0F and U+E0100-U+E01EF only pick a glyph style, so a heart reads as a heart.
    assert_eq!(untrusted_text("\u{2764}\u{fe0f}"), "\u{2764}");
    assert_eq!(untrusted_text("a\u{fe00}b\u{e0100}c"), "abc");
}

#[test]
fn only_an_encoded_international_name_gets_a_note() {
    assert!(host_note("xn--bcher-kva.example").is_some());
    assert!(host_note("shop.xn--p1ai").is_some());
    assert!(host_note("api.example.com").is_none());
    assert!(host_note("127.0.0.1").is_none());
}

#[test]
fn the_mapped_line_names_both_the_host_and_the_address() {
    use apinata_app::approval_prompt::mapped_line;
    assert_eq!(
        mapped_line("api.example.com", "203.0.113.5"),
        "api.example.com mapped to 203.0.113.5"
    );
    // The name is agent text, so it is defanged like the rest.
    assert_eq!(
        mapped_line("a\u{202e}b", "10.0.0.1"),
        "a\u{fffd}b mapped to 10.0.0.1"
    );
    // An IPv4-mapped address reads dotted.
    assert_eq!(
        mapped_line("x.test", "[::ffff:10.0.0.1]"),
        "x.test mapped to [::ffff:10.0.0.1]"
    );
}
