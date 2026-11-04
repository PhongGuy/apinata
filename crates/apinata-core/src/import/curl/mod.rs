//! Importing a single curl command line as a request.

mod body;
mod flags;
mod shell;

use super::{ImportNote, ImportedRequest, ImportedSecret, NoteKind};
use crate::edit::{AuthSpec, KeyValue, Param, ParamKind, RequestSettings, Timeout};
use crate::url_params::{parse_path_params, parse_query_params, split_on_first};
use flags::{Arg, Flag, Known};
use shell::Word;

/// One curl command turned into a request, with a note for everything that was not mapped.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ParsedCurl {
    /// The request the command sends.
    pub request: ImportedRequest,
    /// Flags, words and shell text that were not carried over, in input order.
    pub notes: Vec<ImportNote>,
}

/// Why a curl command could not be imported.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CurlError {
    /// The text is not one POSIX-shell curl command.
    #[error("{form} at byte {offset}")]
    Syntax {
        /// Byte offset in the input where the problem starts.
        offset: usize,
        /// What was found there.
        form: CurlSyntax,
    },
    /// An option that takes a value is the last word.
    #[error("`{flag}` at byte {offset} needs a value")]
    MissingValue {
        /// The option as written.
        flag: String,
        /// Byte offset of the option in the input.
        offset: usize,
    },
    /// The command names no URL.
    #[error("the curl command has no URL")]
    MissingUrl,
}

/// The kind of syntax problem behind [`CurlError::Syntax`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CurlSyntax {
    /// The first word is not `curl`.
    NotCurl,
    /// A quote is never closed.
    UnclosedQuote,
    /// The text uses Windows cmd escapes.
    Cmd,
    /// The text uses PowerShell line continuations.
    PowerShell,
}

impl std::fmt::Display for CurlSyntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CurlSyntax::NotCurl => "not a curl command",
            CurlSyntax::UnclosedQuote => "unclosed quote",
            CurlSyntax::Cmd => "Windows cmd syntax",
            CurlSyntax::PowerShell => "PowerShell syntax",
        })
    }
}

/// One data option's value, handed to the body mapping in input order.
pub(super) struct DataPiece {
    pub flag: Known,
    pub written: String,
    pub offset: usize,
    pub text: String,
}

/// Everything that decides the body: data pieces, `-F` fields as `(written flag, value)`, and `-G`.
#[derive(Default)]
pub(super) struct Collected {
    pub data: Vec<DataPiece>,
    pub form: Vec<(String, Word)>,
    pub get: bool,
}

/// One curl command as a request. Notes cover every flag, word or shell text that was not mapped.
pub fn parse_curl(command: &str) -> Result<ParsedCurl, CurlError> {
    let split = shell::split(command)?;
    let Some((first, rest)) = split.words.split_first() else {
        return Err(not_curl(0));
    };
    if first.text != "curl" && !first.text.ends_with("/curl") {
        return Err(not_curl(first.offset));
    }
    let mut mapper = Mapper::default();
    for arg in flags::scan(rest)? {
        match arg {
            Arg::Positional(word) => mapper.url(word),
            Arg::Flag(flag) => mapper.flag(flag),
        }
    }
    if let Some(tail) = split.tail {
        let detail = format!("shell text from `{}` on was not run", tail.operator);
        mapper.note(tail.offset, "shell", NoteKind::Dropped, detail);
    }
    mapper.finish()
}

fn not_curl(offset: usize) -> CurlError {
    CurlError::Syntax {
        offset,
        form: CurlSyntax::NotCurl,
    }
}

/// The request as the flags build it up; later flags of the same kind override earlier ones,
/// as they do in curl.
#[derive(Default)]
struct Mapper {
    url: Option<String>,
    method: Option<String>,
    head: bool,
    headers: Vec<KeyValue>,
    // Where the joined `Cookie` header sits, and its pieces.
    cookie_at: Option<usize>,
    cookies: Vec<String>,
    compressed: bool,
    user: Option<User>,
    // The first `--digest` or `--ntlm`; the flag says whether the credentials become NTLM
    // auth or, for digest, are dropped.
    challenge: Option<(usize, String, bool)>,
    bearer: Option<Valued>,
    sigv4: Option<Valued>,
    timeout: Option<Timeout>,
    max_redirects: Option<u32>,
    follow_redirects: bool,
    secrets: Vec<ImportedSecret>,
    collected: Collected,
    notes: Vec<(usize, ImportNote)>,
}

impl Mapper {
    fn note(
        &mut self,
        offset: usize,
        source: impl Into<String>,
        kind: NoteKind,
        detail: impl Into<String>,
    ) {
        self.notes
            .push((offset, ImportNote::new(source, kind, detail)));
    }

    fn flag_note(&mut self, flag: &Flag, kind: NoteKind, detail: impl Into<String>) {
        let source = format!("curl flag {}", flag.written);
        self.note(flag.offset, source, kind, detail);
    }

    /// The first non-empty URL wins; every later one is noted.
    fn url(&mut self, word: Word) {
        if word.text.is_empty() {
            return;
        }
        if self.url.is_some() {
            let detail = "only the first URL is imported, so this one was left out";
            self.note(word.offset, "curl url", NoteKind::Dropped, detail);
        } else {
            self.url = Some(word.text);
        }
    }

    fn flag(&mut self, flag: Flag) {
        let value = flag.value.clone();
        let text = value.as_ref().map(|v| v.text.clone()).unwrap_or_default();
        // `--no-digest` switches a digest challenge off again; NTLM is a separate option.
        if flag.negated && flag.option.is_some_and(|o| o.long == "digest") {
            if matches!(self.challenge, Some((_, _, false))) {
                self.challenge = None;
            }
            return;
        }
        // The last of `-L` and `--no-location` decides, as in curl.
        if flag.negated && flag.option.is_some_and(|o| o.long == "location") {
            self.follow_redirects = false;
            return;
        }
        let Some(known) = flag.known else {
            let detail = match flag.option {
                None => format!(
                    "`{}` is not a curl 8.22.0 option and was left out",
                    flag.written
                ),
                Some(_) => format!(
                    "`{}` has no counterpart in a request and was left out",
                    flag.written
                ),
            };
            self.flag_note(&flag, NoteKind::Dropped, detail);
            return;
        };
        match known {
            Known::Url => {
                if let Some(word) = value {
                    self.url(word);
                }
            }
            Known::Method => self.method = Some(text.to_uppercase()),
            Known::Head => self.head = true,
            Known::Get => self.collected.get = true,
            Known::Header => self.header(&flag, &text),
            Known::UserAgent => self.headers.push(KeyValue::new("User-Agent", text)),
            Known::Cookie => self.cookie(&flag, text),
            Known::Compressed => self.compressed = true,
            Known::User => {
                // curl splits at the first colon; without one it would prompt for the password.
                let (username, password) = match text.split_once(':') {
                    Some((name, password)) => (name.to_owned(), Some(password.to_owned())),
                    None => (text, None),
                };
                self.user = Some(User {
                    offset: flag.offset,
                    written: flag.written.clone(),
                    username,
                    password,
                });
            }
            Known::Digest | Known::Ntlm => {
                self.challenge.get_or_insert((
                    flag.offset,
                    flag.written.clone(),
                    matches!(known, Known::Ntlm),
                ));
            }
            Known::Bearer => self.bearer = Some(Valued::new(&flag, text)),
            Known::AwsSigv4 => self.sigv4 = Some(Valued::new(&flag, text)),
            Known::MaxTime => self.max_time(&flag, &text),
            Known::MaxRedirs => self.max_redirs(&flag, &text),
            Known::Location => self.follow_redirects = true,
            Known::Insecure => self.flag_note(
                &flag,
                NoteKind::Dropped,
                "certificate checks cannot be turned off for one request",
            ),
            Known::Form => {
                if let Some(word) = value {
                    self.collected.form.push((flag.written.clone(), word));
                }
            }
            Known::Data
            | Known::DataUrlencode
            | Known::DataRaw
            | Known::DataBinary
            | Known::Json => {
                self.collected.data.push(DataPiece {
                    flag: known,
                    written: flag.written.clone(),
                    offset: flag.offset,
                    text,
                });
            }
        }
    }

    /// `--max-time` seconds become milliseconds, rounded up so a sub-millisecond limit is not
    /// read as none; `0` is curl's "no limit". Text curl would reject is noted.
    fn max_time(&mut self, flag: &Flag, text: &str) {
        match seconds_to_millis(text) {
            Some((0, _)) => self.timeout = Some(Timeout::Unlimited),
            Some((millis, rounded)) => {
                self.timeout = Some(Timeout::Millis(millis));
                if rounded {
                    let detail = format!("the limit was rounded up to {millis} ms");
                    self.flag_note(flag, NoteKind::Converted, detail);
                }
            }
            None => {
                self.timeout = None;
                let detail = "the limit is not a number of seconds, so the default timeout applies";
                self.flag_note(flag, NoteKind::Dropped, detail);
            }
        }
    }

    /// `--max-redirs` counts redirects; `-1` is curl's "no limit", which the settings only
    /// express as the largest count.
    fn max_redirs(&mut self, flag: &Flag, text: &str) {
        let count = match text.strip_prefix('-') {
            Some("1") => Some(None),
            Some(_) => None,
            None => (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                .then(|| text.parse::<u32>().ok()),
        };
        match count {
            Some(Some(n)) => self.max_redirects = Some(n),
            Some(None) => {
                self.max_redirects = Some(u32::MAX);
                let detail = "an unlimited count is written as the largest limit, 4294967295";
                self.flag_note(flag, NoteKind::Converted, detail);
            }
            None => {
                self.max_redirects = None;
                let detail = "the count is not a whole number, so the default limit applies";
                self.flag_note(flag, NoteKind::Dropped, detail);
            }
        }
    }

    /// `Name: value` sends a header, `Name;` sends it empty; `Name:` removes one, so like a
    /// header file or text with neither separator it is noted instead.
    fn header(&mut self, flag: &Flag, text: &str) {
        let colon = text.find(':');
        let semicolon = text.find(';');
        let parsed = if text.starts_with('@') {
            None
        } else {
            match (colon, semicolon) {
                (Some(c), s) if s.is_none_or(|s| c < s) => {
                    let value = text[c + 1..].trim_start_matches([' ', '\t']);
                    (!value.trim().is_empty()).then(|| (&text[..c], value))
                }
                (_, Some(s)) if text[s + 1..].trim().is_empty() => Some((&text[..s], "")),
                _ => None,
            }
        };
        match parsed {
            Some((name, value)) if !name.trim().is_empty() => {
                self.headers.push(KeyValue::new(name, value));
            }
            _ => self.flag_note(
                flag,
                NoteKind::Dropped,
                "curl would remove this header, read it from a file or not send it, so it was left out",
            ),
        }
    }

    /// `-b name=value` adds to one joined `Cookie` header; a `-b` without `=` names a cookie file.
    fn cookie(&mut self, flag: &Flag, text: String) {
        if !text.contains('=') {
            let detail = "a cookie file cannot be imported";
            self.flag_note(flag, NoteKind::Dropped, detail);
            return;
        }
        if self.cookie_at.is_none() {
            self.cookie_at = Some(self.headers.len());
            self.headers.push(KeyValue::new("Cookie", ""));
        }
        self.cookies.push(text);
    }

    /// The request's auth. AWS v4 signing outranks a bearer token, which outranks `-u` with or
    /// without `--digest`; whatever loses is noted.
    fn auth(&mut self) -> AuthSpec {
        let user = self.user.take();
        let challenge = self.challenge.take();
        let mut bearer = self.bearer.take();
        if let Some(empty) = bearer.take_if(|b| b.text.is_empty()) {
            let detail = "the token is empty, so no bearer auth was set";
            self.note(empty.offset, empty.source(), NoteKind::Dropped, detail);
        }
        if let Some(sig) = self.sigv4.take() {
            let lost = "the request is signed with AWS v4, so this was left out";
            if let Some(b) = bearer {
                self.note(b.offset, b.source(), NoteKind::Dropped, lost);
            }
            if let Some((at, written, _)) = challenge {
                self.note(at, format!("curl flag {written}"), NoteKind::Dropped, lost);
            }
            return self.aws_auth(sig, user);
        }
        if let Some(b) = bearer {
            let lost = "a bearer token is set, so this was left out";
            if let Some(user) = user {
                let source = format!("curl flag {}", user.written);
                self.note(user.offset, source, NoteKind::Dropped, lost);
            }
            if let Some((at, written, _)) = challenge {
                self.note(at, format!("curl flag {written}"), NoteKind::Dropped, lost);
            }
            return AuthSpec::Bearer { token: b.text };
        }
        let Some(user) = user else {
            if let Some((at, written, _)) = challenge {
                // A challenge flag without `-u` sends nothing extra, so auth stays inherited.
                let detail = format!("`{written}` has no user to authenticate, so it was left out");
                self.note(
                    at,
                    format!("curl flag {written}"),
                    NoteKind::Dropped,
                    detail,
                );
            }
            return AuthSpec::Inherit;
        };
        if user.password.is_none() {
            let source = format!("curl flag {}", user.written);
            let detail = "the password is empty; enter it before sending";
            self.note(user.offset, source, NoteKind::NeedsInput, detail);
        }
        let password = user.password.unwrap_or_default();
        match challenge {
            Some((_, _, true)) => {
                let (domain, username) = split_ntlm_user(&user.username);
                AuthSpec::Ntlm {
                    username,
                    password,
                    domain,
                }
            }
            Some((_, _, false)) => AuthSpec::Digest {
                username: user.username,
                password,
            },
            None => AuthSpec::Basic {
                username: user.username,
                password,
            },
        }
    }

    /// `--aws-sigv4 provider1[:provider2[:region[:service]]]` with the access key from `--user`.
    /// The secret key is never written: the request refers to the secret variable
    /// `awsSecretAccessKey`, which the user defines.
    fn aws_auth(&mut self, sig: Valued, user: Option<User>) -> AuthSpec {
        let parts: Vec<&str> = sig.text.split(':').collect();
        let well_formed = parts.len() <= 4 && parts.iter().take(2).all(|p| !p.is_empty());
        if !well_formed {
            let detail =
                "the value is not provider1[:provider2[:region[:service]]], so no AWS auth was set";
            self.note(sig.offset, sig.source(), NoteKind::Dropped, detail);
            return AuthSpec::Inherit;
        }
        let Some(user) = user else {
            let detail = "no --user gives the access key, so no AWS auth was set";
            self.note(sig.offset, sig.source(), NoteKind::Dropped, detail);
            return AuthSpec::Inherit;
        };
        let part = |i: usize| parts.get(i).copied().unwrap_or_default().to_owned();
        if (parts[0], parts.get(1).copied()) != ("aws", Some("amz")) {
            let detail = "AWS auth always signs as `aws:amz`, so the provider names were not kept";
            self.note(sig.offset, sig.source(), NoteKind::Converted, detail);
        }
        let detail = format!(
            "the secret access key is not written to the file; define a secret variable named `{AWS_SECRET_VARIABLE}`"
        );
        self.note(sig.offset, sig.source(), NoteKind::NeedsInput, detail);
        if let Some(value) = user.password.filter(|p| !p.is_empty()) {
            self.secrets.push(ImportedSecret {
                name: AWS_SECRET_VARIABLE.to_owned(),
                value,
                source: sig.source(),
            });
        }
        AuthSpec::AwsV4 {
            access_key_id: user.username,
            secret_access_key: format!("{{{{{AWS_SECRET_VARIABLE}}}}}"),
            session_token: String::new(),
            service: part(3),
            region: part(2),
            profile_name: String::new(),
        }
    }

    fn finish(mut self) -> Result<ParsedCurl, CurlError> {
        let url = with_scheme(self.url.take().ok_or(CurlError::MissingUrl)?);
        if let Some(at) = self.cookie_at {
            self.headers[at].value = self.cookies.join("; ");
        }
        let has_encoding = self
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("accept-encoding"));
        if self.compressed && !has_encoding {
            self.headers
                .push(KeyValue::new("Accept-Encoding", "deflate, gzip"));
        }
        let has_body = (!self.collected.data.is_empty() && !self.collected.get)
            || !self.collected.form.is_empty();
        let method = match (self.method.take(), has_body, self.head) {
            (Some(method), _, _) => method,
            (None, true, _) => "POST".to_owned(),
            (None, false, true) => "HEAD".to_owned(),
            (None, false, false) => "GET".to_owned(),
        };
        let mut request = ImportedRequest::new(method, url);
        request.headers = std::mem::take(&mut self.headers);
        body::apply(
            std::mem::take(&mut self.collected),
            &mut request,
            &mut self.notes,
        );
        request.auth = self.auth();
        request.secrets = std::mem::take(&mut self.secrets);
        // The URL is what curl sends. Encoding is turned off where it would change it, or where
        // a variable's value might, since a send with encoding off warns it is not honoured.
        let mut settings = RequestSettings::for_new_file();
        settings.encode_url = !request.url.contains("{{")
            && crate::encode_url::encode_url(&request.url, &[]) == request.url;
        settings.forward_authorization_header = false;
        // curl stays on the first response unless -L; the file default would follow.
        settings.follow_redirects = self.follow_redirects;
        if let Some(timeout) = self.timeout {
            settings.timeout = timeout;
        }
        if let Some(max) = self.max_redirects {
            settings.max_redirects = max;
        }
        request.settings = Some(settings);
        request.params = params_of(&request.url);
        self.notes.sort_by_key(|(offset, _)| *offset);
        Ok(ParsedCurl {
            request,
            notes: self.notes.into_iter().map(|(_, note)| note).collect(),
        })
    }
}

/// The variable that stands for the AWS secret access key in an imported request.
const AWS_SECRET_VARIABLE: &str = "awsSecretAccessKey";

/// The last occurrence of an option that takes a value.
struct Valued {
    offset: usize,
    written: String,
    text: String,
}

impl Valued {
    fn new(flag: &Flag, text: String) -> Valued {
        Valued {
            offset: flag.offset,
            written: flag.written.clone(),
            text,
        }
    }

    fn source(&self) -> String {
        format!("curl flag {}", self.written)
    }
}

/// Whole milliseconds in a decimal number of seconds such as `2.5`, and whether a finer digit
/// was rounded up. `None` for anything but digits with one optional point, or for an overflow.
fn seconds_to_millis(text: &str) -> Option<(u64, bool)> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.len() + fraction.len() == 0 || !digits(whole) || !digits(fraction) {
        return None;
    }
    let seconds: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let kept = format!("{:0<3}", &fraction[..fraction.len().min(3)]);
    let finer = fraction.get(3..).unwrap_or("").bytes().any(|b| b != b'0');
    let millis = seconds
        .checked_mul(1000)?
        .checked_add(kept.parse::<u64>().ok()?)?;
    Some(if finer {
        (millis.checked_add(1)?, true)
    } else {
        (millis, false)
    })
}

/// The last `-u`, split into user name and password; `None` when no colon gave one.
struct User {
    offset: usize,
    written: String,
    username: String,
    password: Option<String>,
}

/// Splits `DOMAIN\user` at its first backslash; without one the domain is empty.
fn split_ntlm_user(username: &str) -> (String, String) {
    match username.split_once('\\') {
        Some((domain, user)) => (domain.to_owned(), user.to_owned()),
        None => (String::new(), username.to_owned()),
    }
}

/// Bruno's `setURL`: a URL without a `scheme://` gains `https://`, unless it starts with a template.
fn with_scheme(url: String) -> String {
    let scheme = url.split_once("://").map(|(scheme, _)| scheme);
    let has_scheme =
        scheme.is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic()));
    if has_scheme || url.starts_with("{{") {
        url
    } else {
        format!("https://{url}")
    }
}

/// The URL's query params in order, then its path params, as a new request derives them.
fn params_of(url: &str) -> Vec<Param> {
    let (base, query) = split_on_first(url, '?');
    let query = query.map(parse_query_params).unwrap_or_default();
    let mut params: Vec<Param> = query
        .into_iter()
        .map(|(name, value)| Param::new(name, value.unwrap_or_default(), ParamKind::Query))
        .collect();
    params.extend(
        parse_path_params(base)
            .into_iter()
            .map(|name| Param::new(name, "", ParamKind::Path)),
    );
    params
}

#[cfg(test)]
mod tests {
    use super::flags::{known, options};
    use super::parse_curl;
    use crate::import::NoteKind;

    // Every curl option outside Bruno's table is noted once, and its value never becomes the URL.
    #[test]
    fn every_unmapped_curl_option_is_noted_and_skips_its_value() {
        for option in options().iter().filter(|o| known(o.long).is_none()) {
            let mut forms = vec![format!("--{}", option.long)];
            forms.extend(option.short.map(|c| format!("-{c}")));
            for flag in forms {
                let value = if option.takes_value {
                    " s3cret.example"
                } else {
                    ""
                };
                let parsed = parse_curl(&format!("curl {flag}{value} https://x.test/"))
                    .unwrap_or_else(|e| panic!("{flag}: {e}"));
                assert_eq!(parsed.request.url, "https://x.test/", "{flag}");
                let notes: Vec<(&str, NoteKind)> = parsed
                    .notes
                    .iter()
                    .map(|n| (n.source.as_str(), n.kind))
                    .collect();
                assert_eq!(
                    notes,
                    [(format!("curl flag {flag}").as_str(), NoteKind::Dropped)],
                    "{flag}"
                );
                assert!(!parsed.notes[0].detail.contains("s3cret"), "{flag}");
                assert!(
                    !parsed.notes[0].detail.to_lowercase().contains("bruno"),
                    "{flag}"
                );
            }
        }
    }
}
