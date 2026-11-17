//! Apinata's YAML reader kernel: the acceptance gate every collection file passes before it is
//! trusted, shared by the app and the server rules. It links no async, network or UI crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod prepass;

pub use prepass::{MAX_NESTING, Refusal, prepass};

/// The most YAML events a collection file may expand to, counting each alias as the node it names.
pub const MAX_EXPANDED_EVENTS: u64 = 2_000_000;

/// Refuse a YAML text whose alias expansion would exhaust memory or time, before `yaml_serde`
/// builds anything from it. The pre-pass can reject text `yaml_serde` accepts, so past such an
/// error the aliases are uncounted; text with both an anchor and an alias marker is then refused
/// rather than trusted. Any other syntax or nesting error passes through to `yaml_serde`.
pub fn bound_expansion(text: &str) -> Result<(), String> {
    let too_large =
        || format!("file is too large: more than {MAX_EXPANDED_EVENTS} values once aliases expand");
    match prepass(text, MAX_EXPANDED_EVENTS) {
        Err(Refusal::TooManyEvents) => Err(too_large()),
        Err(Refusal::At { .. }) if text.contains('&') && text.contains('*') => {
            Err("file has aliases and a syntax the safety check cannot follow".to_owned())
        }
        _ => Ok(()),
    }
}
