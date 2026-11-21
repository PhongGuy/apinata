//! Values printed inside a command a person may paste into a POSIX shell.

/// `value` as one shell word: bare when every character is safe unquoted, else single-quoted
/// with each `'` closed, escaped and reopened.
pub fn shell_word(value: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c);
    if !value.is_empty() && value.chars().all(safe) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
