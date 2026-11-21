//! Values printed inside a command a person may paste into a shell.

/// `value` as one word of the platform's shell: bare when every character is safe unquoted,
/// else single-quoted for a POSIX shell, or double-quoted for cmd and PowerShell on Windows.
pub fn shell_word(value: &str) -> String {
    if cfg!(windows) {
        windows_word(value)
    } else {
        posix_word(value)
    }
}

/// Whether `value` reads as one word unquoted in every shell this targets.
fn is_bare(value: &str) -> bool {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c);
    !value.is_empty() && value.chars().all(safe)
}

/// `value` for a POSIX shell: single-quoted, each `'` closed, escaped and reopened.
pub(crate) fn posix_word(value: &str) -> String {
    if is_bare(value) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// `value` for cmd and PowerShell: double-quoted with each `"` doubled, which both read as
/// one literal quote.
pub(crate) fn windows_word(value: &str) -> String {
    if is_bare(value) {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

#[cfg(test)]
mod tests {
    use super::{posix_word, windows_word};

    #[test]
    fn posix_words_single_quote_what_a_shell_would_split() {
        assert_eq!(posix_word("dev"), "dev");
        assert_eq!(posix_word(""), "''");
        assert_eq!(posix_word("my dir"), "'my dir'");
        assert_eq!(posix_word("it's"), "'it'\\''s'");
    }

    #[test]
    fn windows_words_double_quote_and_double_embedded_quotes() {
        assert_eq!(windows_word("dev"), "dev");
        assert_eq!(windows_word(""), "\"\"");
        assert_eq!(windows_word("my dir"), "\"my dir\"");
        assert_eq!(windows_word("it's"), "\"it's\"");
        assert_eq!(windows_word("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
