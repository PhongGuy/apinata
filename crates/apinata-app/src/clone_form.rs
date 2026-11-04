//! The clone dialog's rules: where a clone goes, what the form accepts, what the user is told.
//! No UI; the dialog and the job compose these.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use apinata_core::{
    CloneError, CloneRequest, default_folder_name, redact_url, validate_clone_url,
    validate_folder_name,
};

/// The folder under the home directory that clones go into until the user picks another.
pub const DEFAULT_PARENT_DIR: &str = "Apinata";

/// The parent folder to offer: the saved one, else `home/Apinata`, else none.
pub fn default_parent(saved: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    saved
        .map(Path::to_path_buf)
        .or_else(|| home.map(|h| h.join(DEFAULT_PARENT_DIR)))
}

/// Create the parent folder (and its ancestors) if it is missing.
pub fn ensure_parent(parent: &Path) -> io::Result<()> {
    fs::create_dir_all(parent)
}

/// The folder name the URL suggests, or an empty string when it suggests none.
pub fn suggested_name(url: &str) -> String {
    default_folder_name(url).unwrap_or_default()
}

/// Why the form cannot be submitted yet.
#[derive(Debug, PartialEq, Eq)]
pub enum FormError {
    /// The URL field is blank.
    EmptyUrl,
    /// Core refused the URL; carries its message.
    BadUrl(String),
    /// No parent folder is chosen.
    NoParent,
    /// Core refused the folder name; carries its message.
    BadName(String),
    /// The destination holds something already.
    Exists,
}

impl FormError {
    /// The text shown under the form.
    pub fn message(&self) -> String {
        match self {
            FormError::EmptyUrl => "Enter a repository URL".to_owned(),
            FormError::NoParent => "Choose a folder to clone into".to_owned(),
            FormError::Exists => "That folder already exists".to_owned(),
            FormError::BadUrl(m) | FormError::BadName(m) => m.clone(),
        }
    }
}

/// Validate the form in order (URL, parent, name, destination) and build the request.
/// A missing or empty destination folder passes; a non-empty directory or a file does not.
pub fn check(url: &str, parent: Option<&Path>, name: &str) -> Result<CloneRequest, FormError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(FormError::EmptyUrl);
    }
    validate_clone_url(url).map_err(|e| FormError::BadUrl(e.to_string()))?;
    let parent = parent.ok_or(FormError::NoParent)?;
    validate_folder_name(name).map_err(|e| FormError::BadName(e.to_string()))?;
    let dest = parent.join(name);
    let taken = match fs::read_dir(&dest) {
        Ok(mut entries) => entries.next().is_some(),
        // Not a directory: a file in the way counts, a missing path does not.
        Err(_) => dest.symlink_metadata().is_ok(),
    };
    if taken {
        return Err(FormError::Exists);
    }
    Ok(CloneRequest::new(url, parent, name))
}

/// Whether the URL is plain `http://`, whose traffic is not encrypted.
pub fn is_unencrypted(url: &str) -> bool {
    url.trim_start()
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
}

/// The URL as it may be shown, without embedded credentials.
pub fn shown_url(url: &str) -> String {
    redact_url(url)
}

/// A one-line next step for the failures users can fix themselves.
pub fn auth_hint(stderr: &str) -> Option<&'static str> {
    const AUTH: [&str; 4] = [
        "Authentication failed",
        "could not read Username",
        "Permission denied",
        "terminal prompts disabled",
    ];
    if AUTH.iter().any(|m| stderr.contains(m)) {
        Some("Set up a credential helper or ssh-agent, then retry.")
    } else if stderr.contains("Host key verification failed") {
        Some("Connect once with ssh to accept the host key, then retry.")
    } else {
        None
    }
}

/// What to tell the user about a failed clone; `None` for a cancel, which is not an error.
pub fn clone_error_text(e: &CloneError) -> Option<String> {
    match e {
        CloneError::Cancelled => None,
        CloneError::Failed { stderr } => {
            let stderr = stderr.trim();
            if stderr.is_empty() {
                return Some("Git failed without saying why.".to_owned());
            }
            Some(match auth_hint(stderr) {
                Some(hint) => format!("{stderr}\n{hint}"),
                None => stderr.to_owned(),
            })
        }
        CloneError::GitMissing => Some("Git was not found. Install git and try again.".to_owned()),
        CloneError::Exists(_) => Some("That folder already exists".to_owned()),
        other => Some(other.to_string()),
    }
}

/// What a finished clone turned out to hold.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// The root has an `opencollection.yml`; open it.
    Collection(PathBuf),
    /// The root has none; tell the user instead.
    NoCollection(PathBuf),
}

/// Classify a cloned root by whether `opencollection.yml` is a file there.
pub fn classify(root: &Path) -> Opened {
    if root.join("opencollection.yml").is_file() {
        Opened::Collection(root.to_path_buf())
    } else {
        Opened::NoCollection(root.to_path_buf())
    }
}

/// The notice for a clone that holds no collection.
pub fn no_collection_text(root: &Path) -> String {
    format!(
        "Cloned to {}. No collection was found at its root, so nothing was opened.",
        root.display()
    )
}
