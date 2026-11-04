//! The app's command line: a version query and the collection roots to open.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The app's command line: a version query, collection roots to open, and a note per bad argument.
#[derive(Debug, Default, PartialEq)]
pub struct LaunchArgs {
    /// `--version` or `-V` was given.
    pub version: bool,
    /// Directories to open as collections, in argument order.
    pub roots: Vec<PathBuf>,
    /// One message per argument that could not be opened.
    pub notes: Vec<String>,
}

/// `args` excludes argv[0]. `--version`/`-V` set `version`; other `-` args are skipped; a path
/// (relative ones joined to `cwd`) that is a directory becomes a root, anything else adds the note
/// "Cannot open `<arg>`: not a directory".
pub fn parse_args(args: &[OsString], cwd: &Path) -> LaunchArgs {
    let mut out = LaunchArgs::default();
    for arg in args {
        if arg == "--version" || arg == "-V" {
            out.version = true;
            continue;
        }
        // Platform launchers may add their own flags, so any other dash argument is ignored.
        if arg.to_string_lossy().starts_with('-') {
            continue;
        }
        // Joining keeps an absolute argument as is and anchors a relative one at the caller's cwd.
        let path = cwd.join(arg);
        if path.is_dir() {
            out.roots.push(path);
        } else {
            out.notes.push(format!(
                "Cannot open `{}`: not a directory",
                arg.to_string_lossy()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn version_flags_are_recognised() {
        let cwd = TempDir::new().unwrap();
        assert!(parse_args(&os(&["--version"]), cwd.path()).version);
        assert!(parse_args(&os(&["-V"]), cwd.path()).version);
        assert!(!parse_args(&os(&[]), cwd.path()).version);
    }

    #[test]
    fn absolute_and_relative_directories_become_roots_in_order() {
        let cwd = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        std::fs::create_dir(cwd.path().join("rel")).unwrap();
        let got = parse_args(&os(&[other.path().to_str().unwrap(), "rel"]), cwd.path());
        assert_eq!(
            got.roots,
            vec![other.path().to_path_buf(), cwd.path().join("rel")]
        );
        assert!(got.notes.is_empty());
    }

    #[test]
    fn a_missing_path_or_a_file_is_skipped_with_a_note() {
        let cwd = TempDir::new().unwrap();
        std::fs::write(cwd.path().join("f.yml"), "x").unwrap();
        let got = parse_args(&os(&["nope", "f.yml"]), cwd.path());
        assert!(got.roots.is_empty());
        assert_eq!(
            got.notes,
            vec![
                "Cannot open `nope`: not a directory".to_owned(),
                "Cannot open `f.yml`: not a directory".to_owned(),
            ]
        );
    }

    #[test]
    fn unknown_flags_are_ignored() {
        let cwd = TempDir::new().unwrap();
        assert_eq!(
            parse_args(&os(&["-psn_0_1", "--foo"]), cwd.path()),
            LaunchArgs::default()
        );
    }
}
