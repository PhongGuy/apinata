//! Exporting collections to other tools' formats. An exporter walks the loaded collection
//! through its file accessors and returns the file text plus a note for everything the target
//! format has no place for.

mod auth;
mod body;
mod environment;
mod extras;
mod har;
mod html;
mod openapi;
mod postman;

pub use environment::{
    EnvironmentFile, postman_environment, postman_environment_files, postman_global_environment,
    postman_global_environment_files,
};
pub use har::{HarExchange, HarRefusal, har, har_log, sent_exchange};
pub use html::{html, html_with};
pub use openapi::openapi;
pub use postman::{postman, postman_with};

/// Longest stem in UTF-16 units, leaving room for `.postman_environment.json` and a ` (n)`
/// suffix inside Windows' 255-unit file name limit.
const MAX_STEM_UNITS: usize = 200;

/// A file name stem for `name` that every platform can create: `trim`med, with the characters
/// Windows forbids (`<>:"/\|?*`) and control characters replaced by `_`, trailing dots and
/// spaces dropped, a Windows device name (`CON`, `COM1`, also before a dot) given a trailing
/// `_`, and the length capped. Blank gives `untitled`.
pub fn file_stem(name: &str) -> String {
    let replaced: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut stem = String::new();
    let mut units = 0;
    for c in replaced.chars() {
        units += c.len_utf16();
        if units > MAX_STEM_UNITS {
            break;
        }
        stem.push(c);
    }
    let mut stem = stem
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_owned();
    if stem.is_empty() {
        return "untitled".to_owned();
    }
    // Windows reads `CON.anything` as the device, so the part before the first dot decides.
    let head = stem.split('.').next().unwrap_or_default().trim_end();
    if is_device_name(head) {
        stem.insert(head.len(), '_');
    }
    stem
}

/// Whether `name` is a reserved Windows device name, in any case: `CON`, `PRN`, `AUX`, `NUL`,
/// `CONIN$`, `CONOUT$`, and `COM`/`LPT` followed by a digit or a superscript 1, 2 or 3.
fn is_device_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    matches!(prefix.as_str(), "COM" | "LPT")
        && matches!(chars.next(), Some(c) if c.is_ascii_digit() || matches!(c, '\u{b9}' | '\u{b2}' | '\u{b3}'))
        && chars.next().is_none()
}

/// Writes `bytes` to a temporary file beside `path`, then renames it over `path`, so a reader
/// never sees a half-written export. The file is meant to be shared, so on unix a new one gets
/// the umask's ordinary mode and a replaced one keeps its mode. Fails when the parent is missing.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = tempfile::Builder::new();
    // tempfile defaults to 0600; 0666 is filtered by the umask like any other new file.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let mut file = builder.tempfile_in(parent)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    if let Ok(existing) = std::fs::metadata(path) {
        file.as_file().set_permissions(existing.permissions())?;
    }
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// A collection exported as Postman v2.1 JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PostmanExport {
    /// The collection file's text, two-space indented and ending in a newline.
    pub json: String,
    /// What the file could not carry, in tree order.
    pub notes: Vec<ExportNote>,
}

/// Whether `path` names a YAML file by its extension (`.yaml` or `.yml`, any case).
pub fn is_yaml_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"))
}

/// A collection exported as an OpenAPI 3.1 document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpenApiExport {
    /// Two-space indented JSON with a trailing newline.
    pub json: String,
    /// What was dropped or degraded, in tree order.
    pub notes: Vec<ExportNote>,
}

impl OpenApiExport {
    /// The same document as block-style YAML with a trailing newline, keys in the JSON's order.
    pub fn yaml(&self) -> Result<String, String> {
        // Re-reading the text we just wrote keeps one source of truth for the document's shape.
        let value: serde_json::Value =
            serde_json::from_str(&self.json).map_err(|e| e.to_string())?;
        yaml_serde::to_string(&value).map_err(|e| e.to_string())
    }
}

/// A collection exported as one HTML documentation page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HtmlExport {
    /// A complete UTF-8 HTML5 document with a trailing newline.
    pub html: String,
    /// What was left out or degraded, in tree order.
    pub notes: Vec<ExportNote>,
}

/// Which optional sections the HTML documentation page includes; both are off by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HtmlOptions {
    include_scripts: bool,
    include_environments: bool,
}

impl HtmlOptions {
    /// No optional sections.
    pub fn new() -> HtmlOptions {
        HtmlOptions::default()
    }

    /// Show each request's assertions and scripts as escaped code; nothing is run.
    pub fn include_scripts(mut self, on: bool) -> HtmlOptions {
        self.include_scripts = on;
        self
    }

    /// List each environment's variables as stored, with secret values masked.
    pub fn include_environments(mut self, on: bool) -> HtmlOptions {
        self.include_environments = on;
        self
    }

    /// Whether requests show their assertions and scripts.
    pub fn scripts(&self) -> bool {
        self.include_scripts
    }

    /// Whether the page lists the environments.
    pub fn environments(&self) -> bool {
        self.include_environments
    }
}

/// How the Postman export treats scripts; scripts are translated by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PostmanOptions {
    /// Export scripts exactly as written, without translating the script API.
    pub keep_scripts: bool,
}

impl PostmanOptions {
    /// The defaults: scripts are translated.
    pub fn new() -> PostmanOptions {
        PostmanOptions::default()
    }

    /// Export scripts exactly as written instead of translating their API calls.
    pub fn keep_scripts(mut self, on: bool) -> PostmanOptions {
        self.keep_scripts = on;
        self
    }
}

/// Something an export left out or could not carry over unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportNote {
    /// Where it was: `collection / <member>`, or item names joined by ` / ` then the member.
    pub source: String,
    /// What was lost.
    pub detail: String,
}

impl ExportNote {
    pub(crate) fn new(source: impl Into<String>, detail: impl Into<String>) -> ExportNote {
        ExportNote {
            source: source.into(),
            detail: detail.into(),
        }
    }
}

/// Header names whose literal value is a credential.
const EXTRA_CREDENTIAL_HEADERS: [&str; 2] = ["x-api-key", "api-key"];

/// Whether `header` is a shared credential header, x-api-key or api-key
/// (ASCII case-insensitive) with a value that is not blank and holds no `{{`.
pub(crate) fn literal_credential(header: &crate::edit::KeyValue) -> bool {
    credential_name(&header.name) && written_value(&header.value)
}

/// Whether `name` is a shared credential header name.
fn credential_name(name: &str) -> bool {
    crate::is_credential_header(name)
        || EXTRA_CREDENTIAL_HEADERS
            .iter()
            .any(|extra| name.eq_ignore_ascii_case(extra))
}

/// A value that is not blank and holds no `{{`.
fn written_value(value: &str) -> bool {
    !crate::js::js_trim(value).is_empty() && !value.contains("{{")
}

/// Query parameter names whose literal value is a credential, compared in lower case with
/// `-` and `_` removed.
const CREDENTIAL_QUERY_NAMES: [&str; 9] = [
    "apikey",
    "accesstoken",
    "authtoken",
    "idtoken",
    "refreshtoken",
    "token",
    "clientsecret",
    "secret",
    "password",
];

/// Whether a query parameter `name=value` is a literal credential: a credential header name
/// or one of the usual credential query names, with a value as for [`literal_credential`].
pub(crate) fn literal_credential_query(name: &str, value: &str) -> bool {
    let plain: String = name
        .chars()
        .filter(|c| !matches!(c, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect();
    (credential_name(name) || CREDENTIAL_QUERY_NAMES.contains(&plain.as_str()))
        && written_value(value)
}

/// The names of the literal credentials in `url`'s query string that no param in `params`
/// already notes, so a credential the params list carries is noted once. Names and values
/// are percent-decoded, and names compare in ASCII case-insensitive form.
pub(crate) fn url_query_credentials(url: &str, params: &[crate::edit::Param]) -> Vec<String> {
    let decode = |s: &str| {
        percent_encoding::percent_decode_str(s)
            .decode_utf8_lossy()
            .into_owned()
    };
    let query = url.split_once('?').map_or("", |(_, q)| q);
    let query = query.split('#').next().unwrap_or("");
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (decode(name), decode(value)))
        .filter(|(name, value)| literal_credential_query(name, value))
        .filter(|(name, _)| {
            !params.iter().any(|p| {
                p.kind == crate::edit::ParamKind::Query
                    && p.name.eq_ignore_ascii_case(name)
                    && literal_credential_query(&p.name, &p.value)
            })
        })
        .map(|(name, _)| name)
        .collect()
}
