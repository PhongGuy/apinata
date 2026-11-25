//! Server-supplied text made safe to show.

use std::fmt::Write as _;

use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// Whether `c` is a control, format (bidi controls and zero-width joiners among them), line
/// separator or paragraph separator character, any of which can reorder, split or hide the
/// text around it on screen.
pub(crate) fn is_hidden(c: char) -> bool {
    matches!(
        c.general_category(),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
    )
}

/// `text` with each hidden character written as a visible `\u{XXXX}` escape (uppercase hex, at
/// least one digit), so what is shown is the text as it is; other characters stay as they are.
pub fn escape_hidden(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_hidden(c) {
            // Writing to a String cannot fail.
            let _ = write!(out, "\\u{{{:X}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}
