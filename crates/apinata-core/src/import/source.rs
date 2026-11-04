//! Telling what the Import link field holds: an http(s) link to fetch or a local file to read.

use std::path::{Path, PathBuf};

use url::Url;

/// Where an import comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportSource {
    /// An http or https link, unchanged.
    Link(String),
    /// An existing local file.
    File(PathBuf),
}

/// Why the field text is not an importable source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SourceError {
    /// The text is neither a link nor an absolute path.
    #[error("Expected an http or https link, or the full path of a file.")]
    Expected,
    /// A path starting with `~`, which no shell expanded for us.
    #[error("A path starting with ~ is not expanded. Give the full path of the file instead.")]
    Tilde,
    /// A scheme other than http, https or file.
    #[error("Unsupported link ({0}://): only http and https links or file paths can be imported.")]
    Scheme(String),
    /// Text before `://` that cannot be a URL scheme, so it is most likely a path.
    #[error("That is not a link ({0}://). To import a file, give the full path of the file.")]
    NotALink(String),
    /// A `file://` URL naming another machine.
    #[error("Only a local file can be imported, not a file:// URL with a host.")]
    RemoteHost,
    /// No file at the path.
    #[error("No file at that path: {}", .0.display())]
    Missing(PathBuf),
    /// The path is a folder, not a file.
    #[error("That path is a folder, not a file: {}", .0.display())]
    Folder(PathBuf),
}

/// Classifies the trimmed field text as a link or a local file. An absolute path is checked
/// before URL parsing so a Windows drive path is not mistaken for a scheme.
pub fn classify_source(text: &str) -> Result<ImportSource, SourceError> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(ImportSource::Link(text.to_owned()));
    }
    if Path::new(text).is_absolute() {
        return existing_file(PathBuf::from(text));
    }
    if text.starts_with('~') {
        return Err(SourceError::Tilde);
    }
    if lower.starts_with("file://") {
        let url = Url::parse(text).map_err(|_| SourceError::Expected)?;
        if !matches!(url.host_str(), None | Some("" | "localhost")) {
            return Err(SourceError::RemoteHost);
        }
        let path = url.to_file_path().map_err(|()| SourceError::RemoteHost)?;
        return existing_file(path);
    }
    if let Some((scheme, _)) = text.split_once("://") {
        // A real scheme is a letter then letters, digits, `+`, `-` or `.`; anything else is
        // a path that happens to hold `://`.
        let mut chars = scheme.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        return Err(if valid {
            SourceError::Scheme(scheme.to_owned())
        } else {
            SourceError::NotALink(scheme.to_owned())
        });
    }
    Err(SourceError::Expected)
}

// The path must be a regular file that exists now, so the failure shows before any work starts.
fn existing_file(path: PathBuf) -> Result<ImportSource, SourceError> {
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => Err(SourceError::Folder(path)),
        Ok(_) => Ok(ImportSource::File(path)),
        Err(_) => Err(SourceError::Missing(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        classify_source(text).unwrap_err().to_string()
    }

    #[test]
    fn http_links_pass_through_unchanged() {
        for t in [
            "http://a.test/x.json",
            "HTTPS://A.test/x",
            "  https://a.test/y  ",
        ] {
            assert_eq!(
                classify_source(t),
                Ok(ImportSource::Link(t.trim().to_owned())),
                "{t}"
            );
        }
    }

    #[test]
    fn an_existing_absolute_path_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("spec.json");
        std::fs::write(&f, "{}").unwrap();
        assert_eq!(
            classify_source(f.to_str().unwrap()),
            Ok(ImportSource::File(f.clone()))
        );
    }

    #[test]
    fn a_file_url_with_percent_encoding_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("my spec.json");
        std::fs::write(&f, "{}").unwrap();
        let url = url::Url::from_file_path(&f).unwrap();
        assert!(url.as_str().contains("%20"));
        assert_eq!(classify_source(url.as_str()), Ok(ImportSource::File(f)));
    }

    #[test]
    fn a_file_url_to_a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("gone.json");
        let url = url::Url::from_file_path(&f).unwrap();
        let msg = err(url.as_str());
        assert!(msg.contains("No file at"), "{msg}");
        assert!(msg.contains("gone.json"), "{msg}");
    }

    #[test]
    fn a_file_url_with_a_remote_host_is_an_error() {
        let msg = err("file://example.com/etc/spec.json");
        assert!(msg.contains("local"), "{msg}");
    }

    #[test]
    fn a_missing_absolute_path_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("nope.yaml");
        let msg = err(f.to_str().unwrap());
        assert!(msg.contains("No file at that path"), "{msg}");
        assert!(msg.contains("nope.yaml"), "{msg}");
    }

    #[test]
    fn a_directory_gets_its_own_message() {
        let dir = tempfile::tempdir().unwrap();
        let msg = err(dir.path().to_str().unwrap());
        assert!(msg.contains("folder"), "{msg}");
    }

    #[test]
    fn a_leading_tilde_is_not_expanded() {
        let msg = err("~/specs/api.json");
        assert!(msg.contains("not expanded"), "{msg}");
        assert!(msg.contains("full path"), "{msg}");
    }

    #[test]
    fn another_scheme_is_refused() {
        let msg = err("ftp://host/spec.json");
        assert!(
            msg.contains("only http and https links or file paths can be imported"),
            "{msg}"
        );
    }

    #[test]
    fn a_scheme_shaped_path_asks_for_the_full_path() {
        let msg = err("a b://c");
        assert!(msg.contains("full path of the file"), "{msg}");
        assert!(!msg.contains("only http and https"), "{msg}");
        let ftp = err("ftp://host/x");
        assert!(ftp.contains("only http and https"), "{ftp}");
    }

    #[test]
    fn plain_text_and_relative_paths_are_not_sources() {
        for t in ["hello", "specs/api.json", "./api.json"] {
            let msg = err(t);
            assert!(msg.contains("Expected an http or https link"), "{t}: {msg}");
        }
    }
}
