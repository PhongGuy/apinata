//! Which characters could disguise untrusted text shown to a person, shared by every shell.

/// Whether `c` is a control, bidi, invisible or filler character that a shell must make
/// visible before showing untrusted text: `char::is_control`, U+00AD, U+034F, U+061C,
/// U+115F, U+1160, U+200B-U+200F, U+2028-U+202E, U+2060-U+206F, U+3164, U+FEFF, U+FFA0
/// and U+E0000-U+E007F. Variation selectors are not included; see [`is_variation_selector`].
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{034f}'
                | '\u{061c}'
                | '\u{115f}'
                | '\u{1160}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{3164}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// Whether `c` is a variation selector (U+FE00-U+FE0F, U+E0100-U+E01EF). These only choose
/// a glyph's style, so a shell drops them rather than marking them.
pub fn is_variation_selector(c: char) -> bool {
    matches!(c, '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
}
