//! Writing one key of the collection's `.env`, and keeping `.env` out of git.

use super::Collection;
use crate::document::{hash_bytes, is_special_file, read_regular, temp_path};
use crate::dotenv::parse_dotenv;
use crate::post_response::variable_name_problem;
use std::collections::BTreeSet;
use std::fs;
use std::io::{Error, ErrorKind, Write};
use std::path::Path;

/// `<root>/.env` and `<root>/.gitignore` as they were, for [`Collection::restore_dotenv`].
/// Its `Debug` shows only the sizes, since `.env` holds secret values.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DotenvBackup {
    dotenv: Option<Vec<u8>>,
    gitignore: Option<Vec<u8>>,
}

impl std::fmt::Debug for DotenvBackup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size = |bytes: &Option<Vec<u8>>| bytes.as_ref().map(Vec::len);
        f.debug_struct("DotenvBackup")
            .field("dotenv_len", &size(&self.dotenv))
            .field("gitignore_len", &size(&self.gitignore))
            .finish()
    }
}

impl Collection {
    /// The bytes of `<root>/.env` and `<root>/.gitignore` now; a missing file is recorded as
    /// missing. Any other read failure is an error.
    pub fn backup_dotenv(&self) -> std::io::Result<DotenvBackup> {
        Ok(DotenvBackup {
            dotenv: read_bytes(&self.root().join(".env"))?,
            gitignore: read_bytes(&self.root().join(".gitignore"))?,
        })
    }

    /// Put `.env` and `.gitignore` back as `backup` recorded them, removing a file that did
    /// not exist then, and remember `.env`'s hash so a reload reports no change.
    pub fn restore_dotenv(&mut self, backup: &DotenvBackup) -> std::io::Result<()> {
        let dotenv = self.root().join(".env");
        restore(&dotenv, backup.dotenv.as_deref())?;
        restore(&self.root().join(".gitignore"), backup.gitignore.as_deref())?;
        self.dotenv_hash = backup.dotenv.as_deref().map(hash_bytes);
        Ok(())
    }

    /// Undo a write: restore `backup` only while both files are still exactly as `written`
    /// recorded them after the write. Ok(false) means something else changed them since, and
    /// they are left alone.
    pub fn undo_dotenv(
        &mut self,
        backup: &DotenvBackup,
        written: &DotenvBackup,
    ) -> std::io::Result<bool> {
        if self.backup_dotenv()? != *written {
            return Ok(false);
        }
        self.restore_dotenv(backup)?;
        Ok(true)
    }

    /// The keys `<root>/.env` defines; empty when there is no file, or a pipe, socket or device
    /// stands there. Not UTF-8 is `InvalidData`.
    pub fn dotenv_keys(&self) -> std::io::Result<BTreeSet<String>> {
        let text = read_optional(&self.root().join(".env"))?;
        Ok(parse_dotenv(&text).into_iter().map(|(k, _)| k).collect())
    }

    /// Set `name` in `<root>/.env` so the dotenv reader yields exactly `value`, keeping every
    /// other line byte for byte, and remember the new hash so a reload reports no change.
    /// A pipe, socket or device at `.env` is `InvalidInput` and stays as it is.
    pub fn set_dotenv(&mut self, name: &str, value: &str) -> std::io::Result<()> {
        // One name rule for every variable writer; only the wording is local.
        if variable_name_problem(name).is_some() {
            return Err(invalid(
                "the variable name may hold only letters, digits, _, . and -",
            ));
        }
        let line = format!("{name}={}", quote(value)?);
        let path = self.root().join(".env");
        refuse_special(&path)?;
        let text = read_optional(&path)?;

        // Rewrite the last line the reader takes the key from, keeping its terminator;
        // with no such line, append one in the file's own line-ending style.
        let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
        let mut out = String::with_capacity(text.len() + line.len() + 2);
        match lines.iter().rposition(|l| defines(l, name)) {
            Some(at) => {
                let ending = &lines[at][lines[at].trim_end_matches(['\r', '\n']).len()..];
                let replaced = format!("{line}{ending}");
                lines[at] = &replaced;
                out.extend(lines);
            }
            None => {
                let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
                out.push_str(&text);
                if !text.is_empty() && !text.ends_with('\n') {
                    out.push_str(ending);
                }
                out.push_str(&line);
                out.push_str(ending);
            }
        }
        write_replacing(&path, out.as_bytes())?;
        self.dotenv_hash = Some(hash_bytes(out.as_bytes()));
        Ok(())
    }

    /// Make `<root>/.gitignore` ignore `.env`; true when a line was appended. A pipe, socket or
    /// device there is `InvalidInput`.
    pub fn ensure_dotenv_ignored(&self) -> std::io::Result<bool> {
        let path = self.root().join(".gitignore");
        refuse_special(&path)?;
        let mut text = read_optional(&path)?;
        if text.lines().any(|l| matches!(l.trim(), ".env" | "/.env")) {
            return Ok(false);
        }
        let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push_str(ending);
        }
        text.push_str(".env");
        text.push_str(ending);
        fs::write(&path, text)?;
        Ok(true)
    }
}

/// The file's bytes, or None when it does not exist.
fn read_bytes(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    refuse_special(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Write `bytes` to `path` keeping its mode, or remove it when there were none.
fn restore(path: &Path, bytes: Option<&[u8]>) -> std::io::Result<()> {
    match bytes {
        Some(bytes) => write_replacing(path, bytes),
        None => match fs::remove_file(path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// The file's text, or empty when it does not exist or [`read_regular`] takes it as gone.
fn read_optional(path: &Path) -> std::io::Result<String> {
    match read_regular(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|e| Error::new(ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

// A writer never takes a special file as absent: a write waits on it, and a rename would swap
// it for a file that keeps its mode.
fn refuse_special(path: &Path) -> std::io::Result<()> {
    if is_special_file(path) {
        return Err(invalid(&format!(
            "{} is a pipe, socket or device, not a file",
            path.display()
        )));
    }
    Ok(())
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

/// Whether `set_dotenv` can write `value`: it has no line break and some quote kind fits.
pub fn dotenv_quotable(value: &str) -> bool {
    quote_char(value).is_some()
}

/// The quote the reader strips and nothing else: single quotes first, then backticks, then
/// double quotes, which only qualify while no `\n` or `\r` text would be expanded.
fn quote_char(value: &str) -> Option<char> {
    if value.contains(['\n', '\r']) {
        None
    } else if !value.contains('\'') {
        Some('\'')
    } else if !value.contains('`') {
        Some('`')
    } else if !value.contains('"') && !value.contains("\\n") && !value.contains("\\r") {
        Some('"')
    } else {
        None
    }
}

/// The value quoted by [`quote_char`].
fn quote(value: &str) -> std::io::Result<String> {
    if value.contains(['\n', '\r']) {
        return Err(invalid("a .env value cannot hold a line break"));
    }
    let q = quote_char(value).ok_or_else(|| invalid("this value cannot be written to .env"))?;
    Ok(format!("{q}{value}{q}"))
}

/// Whether `line` defines `key` as the reader matches it: optional leading whitespace and
/// `export `, the key, then `=` after optional whitespace or `:` followed by whitespace.
fn defines(line: &str, key: &str) -> bool {
    let rest = line.trim_start();
    let after_export = rest
        .strip_prefix("export")
        .filter(|r| r.starts_with(char::is_whitespace))
        .map(str::trim_start);
    [Some(rest), after_export].into_iter().flatten().any(|r| {
        r.strip_prefix(key).is_some_and(|tail| {
            tail.trim_start().starts_with('=')
                || tail
                    .strip_prefix(':')
                    .is_some_and(|t| t.starts_with(char::is_whitespace))
        })
    })
}

/// Replace `path` through a temporary file and a rename, keeping an existing file's mode;
/// a new file is private to its owner on Unix.
fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let existing = match fs::metadata(path) {
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let temp = temp_path(path);
    let _ = fs::remove_file(&temp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&temp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| match existing {
            Some(permissions) => fs::set_permissions(&temp, permissions),
            None => Ok(()),
        })
        .and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}
