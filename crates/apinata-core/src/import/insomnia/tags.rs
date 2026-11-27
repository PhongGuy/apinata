//! Insomnia's built-in template tags and what each becomes: a dynamic value, a value computed
//! at import, a chained response variable, or nothing.

use super::chain::Chains;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

/// One argument as Insomnia's tokenizer reads it: quoted text, with `b64::…::46b` decoded, or
/// anything unquoted (a number, a boolean, a variable or an expression).
#[derive(Debug, PartialEq)]
enum Arg {
    Text(String),
    Other,
}

/// What a tag converts to.
pub(super) enum Converted {
    /// Text that takes the tag's place.
    Value(String),
    /// A variable that `source` sets from its response.
    Chain {
        variable: String,
        source: String,
        resent: bool,
    },
    /// No equivalent, for this reason.
    Kept(String),
}

/// Convert tag `name` with argument text `args`, linking a response tag through `chains`.
pub(super) fn convert(name: &str, args: &str, chains: &mut Chains) -> Converted {
    let args = tokenize(args);
    let text = |i: usize| match args.get(i) {
        Some(Arg::Text(t)) => Some(t.as_str()),
        // A missing optional argument takes Insomnia's default of empty text.
        None => Some(""),
        Some(Arg::Other) => None,
    };
    let converted = match name {
        "faker" => faker(&args),
        "uuid" => Ok("{{$randomUUID}}".to_owned()),
        "now" => now(text(0)),
        "base64" => base64(text(0), text(1), text(2)),
        "hash" => hash(text(0), text(1), text(2)),
        "response" => return response(&args, chains),
        "prompt" => Err("it asks for a value each time it is sent"),
        "cookie" => Err("it reads the cookie jar when sent"),
        "file" => Err("it reads a file when sent"),
        "request" => Err("it reads the sending request"),
        "os" => Err("it reads the sending machine"),
        _ => Err("it has no equivalent"),
    };
    // A computed value is plain text, so neither it nor its input may read as a template.
    let templated = |t: &str| t.contains("{{") || t.contains("{%");
    let computed = matches!(name, "base64" | "hash");
    let templated_input = args
        .iter()
        .any(|a| matches!(a, Arg::Text(t) if templated(t)));
    match converted {
        Ok(text) if computed && (templated(&text) || templated_input) => {
            Converted::Kept("its value would read as a template".to_owned())
        }
        Ok(text) => Converted::Value(text),
        Err(reason) => Converted::Kept(reason.to_owned()),
    }
}

/// Insomnia's faker names follow the same naming as the generated `$` values.
fn faker(args: &[Arg]) -> Result<String, &'static str> {
    match args.first() {
        Some(Arg::Text(name)) if crate::dynamic_variables().contains(&name.as_str()) => {
            Ok(format!("{{{{${name}}}}}"))
        }
        _ => Err("this faker value has no equivalent"),
    }
}

fn now(kind: Option<&str>) -> Result<String, &'static str> {
    match kind.map(str::to_ascii_lowercase).as_deref() {
        Some("" | "iso-8601") => Ok("{{$isoTimestamp}}".to_owned()),
        Some("unix" | "seconds" | "s") => Ok("{{$timestamp}}".to_owned()),
        _ => Err("only ISO 8601 and Unix seconds have an equivalent"),
    }
}

/// Insomnia's base64 tag on literal text, computed now; output that Insomnia would decode
/// lossily or from bytes it would read differently is not converted.
fn base64(
    action: Option<&str>,
    kind: Option<&str>,
    text: Option<&str>,
) -> Result<String, &'static str> {
    let variable = "only literal text can be computed on import";
    let (Some(action), Some(kind), Some(text)) = (action, kind, text) else {
        return Err(variable);
    };
    let invalid = "the input is not valid for this encoding";
    match (action, kind) {
        ("encode", "normal") => Ok(STANDARD.encode(text)),
        ("encode", "url") => Ok(URL_SAFE_NO_PAD.encode(text)),
        ("encode", "hex") => Ok(STANDARD.encode(hex(text).ok_or(invalid)?)),
        ("decode", "normal" | "url" | "hex") => {
            let bytes = STANDARD
                .decode(text)
                .or_else(|_| STANDARD_NO_PAD.decode(text))
                .map_err(|_| invalid)?;
            if kind == "hex" {
                Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
            } else {
                String::from_utf8(bytes).map_err(|_| invalid)
            }
        }
        _ => Err("this base64 action has no equivalent"),
    }
}

/// Even-length hex digits as bytes.
fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let pairs = text.as_bytes().chunks(2);
    pairs
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}

fn hash(
    algorithm: Option<&str>,
    encoding: Option<&str>,
    value: Option<&str>,
) -> Result<String, &'static str> {
    let (Some(algorithm), Some(encoding), Some(value)) = (algorithm, encoding, value) else {
        return Err("only literal text can be computed on import");
    };
    let digest = match algorithm.to_ascii_lowercase().as_str() {
        "md5" => Md5::digest(value).to_vec(),
        "sha1" => Sha1::digest(value).to_vec(),
        "sha256" => Sha256::digest(value).to_vec(),
        "sha512" => Sha512::digest(value).to_vec(),
        _ => return Err("this hash algorithm has no equivalent"),
    };
    match encoding {
        "hex" => Ok(digest.iter().map(|b| format!("{b:02x}")).collect()),
        "base64" => Ok(STANDARD.encode(digest)),
        _ => Err("this digest encoding has no equivalent"),
    }
}

/// A body JSONPath or a header of another request's latest response, as a variable that
/// request sets after its response arrives.
fn response(args: &[Arg], chains: &mut Chains) -> Converted {
    let kept = |reason: &str| Converted::Kept(reason.to_owned());
    let (Some(Arg::Text(field)), Some(Arg::Text(id))) = (args.first(), args.get(1)) else {
        return kept("only a response field and request given as text can be converted");
    };
    let filter = match args.get(2) {
        Some(Arg::Text(f)) => f.trim(),
        _ => return kept("only a filter given as text can be converted"),
    };
    let resent = match args.get(3) {
        None => false,
        Some(Arg::Text(b)) => !b.is_empty() && !b.eq_ignore_ascii_case("never"),
        Some(Arg::Other) => true,
    };
    let expression = match field.as_str() {
        "body" => json_path(filter),
        "header" => header(filter),
        _ => return kept("only a body path or a header can be read from a response"),
    };
    let Some((expression, slug)) = expression else {
        return kept("only a plain JSONPath or header name can be converted");
    };
    match chains.link(id, expression, &slug) {
        Ok((variable, source)) => Converted::Chain {
            variable,
            source,
            resent,
        },
        Err(reason) => Converted::Kept(reason),
    }
}

/// `$` followed by `.key`, `[n]` and quoted `['key']` steps, as a `res.body` expression and
/// the steps joined for a name. Anything else (recursion, wildcards, filters, XPath) is `None`.
fn json_path(filter: &str) -> Option<(String, String)> {
    let mut rest = filter.strip_prefix('$')?;
    let mut expression = "res.body".to_owned();
    let mut steps = Vec::new();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(after.len());
            let key = &after[..end];
            if key.is_empty() || key.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            expression.push('.');
            expression.push_str(key);
            steps.push(key);
            rest = &after[end..];
        } else {
            let (inner, after) = rest.strip_prefix('[')?.split_once(']')?;
            let quoted = ['\'', '"']
                .iter()
                .find_map(|q| inner.strip_prefix(*q)?.strip_suffix(*q));
            match quoted {
                Some(key) => expression.push_str(&format!("['{key}']")),
                None if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()) => {
                    expression.push_str(&format!("[{inner}]"));
                }
                None => return None,
            }
            steps.push(quoted.unwrap_or(inner));
            rest = after;
        }
    }
    crate::assertion::lhs::supported_shape(&expression).then(|| (expression, steps.join("_")))
}

/// A response header by its lowercased name; Insomnia matches names ignoring case.
fn header(name: &str) -> Option<(String, String)> {
    let name = name.to_ascii_lowercase();
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&*+-.^_`|~".contains(&b);
    if name.is_empty() || !name.bytes().all(token) {
        return None;
    }
    let expression = format!("res.headers['{name}']");
    crate::assertion::lhs::supported_shape(&expression).then_some((expression, name))
}

/// Insomnia's `tokenizeArgs`: comma- or space-separated arguments, quoted by `'` or `"` with
/// backslash escapes, then `decodeEncoding` on each quoted one.
fn tokenize(args: &str) -> Vec<Arg> {
    let mut out = Vec::new();
    let mut chars = args.chars().chain(std::iter::once(','));
    let mut current: Option<(String, Option<char>)> = None;
    while let Some(c) = chars.next() {
        let Some((text, quote)) = current.as_mut() else {
            match c {
                c if c.is_whitespace() || c == ',' => {}
                '\'' | '"' => current = Some((String::new(), Some(c))),
                c => current = Some((c.to_string(), None)),
            }
            continue;
        };
        let ends = match quote {
            Some(q) => c == *q,
            None => c == ',',
        };
        if !ends {
            if c == '\\' {
                text.extend(chars.next());
            } else {
                text.push(c);
            }
            continue;
        }
        let (text, quote) = current.take().unwrap_or_default();
        out.push(match quote {
            Some(_) => Arg::Text(decode(text)),
            None => Arg::Other,
        });
    }
    // An unterminated quote never reaches its closer, which Insomnia drops along with it.
    out
}

/// Insomnia stores some arguments as `b64::<base64>::46b`.
fn decode(text: String) -> String {
    let inner = text
        .strip_prefix("b64::")
        .and_then(|t| t.strip_suffix("::46b"))
        .filter(|t| !t.is_empty());
    inner
        .and_then(|t| STANDARD.decode(t).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::{Arg, tokenize};

    #[test]
    fn arguments_split_on_commas_outside_quotes() {
        let text = |t: &str| Arg::Text(t.to_owned());
        assert_eq!(
            tokenize(" 'a, b', \"c\\\"d\" , 60, _.x,'b64::aGk=::46b'"),
            [
                text("a, b"),
                text("c\"d"),
                Arg::Other,
                Arg::Other,
                text("hi")
            ]
        );
        assert!(tokenize("").is_empty());
        assert_eq!(tokenize("'open"), []);
    }
}
