//! The one gate for a user-supplied HTTP method.

use std::fmt;

/// A method that is not an RFC 9110 token.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InvalidMethod {
    method: String,
}

impl fmt::Display for InvalidMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.method.is_empty() {
            write!(f, "the HTTP method is empty")
        } else {
            write!(
                f,
                "`{}` is not a valid HTTP method: use letters, digits, - or _, with no spaces",
                self.method
            )
        }
    }
}

impl std::error::Error for InvalidMethod {}

/// Check that `method` is a non-empty RFC 9110 token and return it uppercased.
///
/// Nothing is trimmed, so a stray space is an error rather than silently dropped.
pub fn normalize_method(method: &str) -> Result<String, InvalidMethod> {
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    if method.is_empty() || !method.bytes().all(token) {
        return Err(InvalidMethod {
            method: method.to_owned(),
        });
    }
    Ok(method.to_ascii_uppercase())
}
