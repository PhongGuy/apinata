//! Ports of Bruno's `sanitizeName` and `validateName` (bruno-common `utils/naming.ts`), which
//! turn display names into file names and check them.

use crate::js::is_js_whitespace;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// Characters no file name may hold: `<>:"/\|?*` and the C0 controls.
fn is_invalid(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c <= '\u{1f}'
}

/// Replace invalid characters with `-`, then strip leading whitespace and hyphens and trailing
/// dots and whitespace.
pub(crate) fn sanitize_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if is_invalid(c) { '-' } else { c })
        .collect();
    replaced
        .trim_start_matches(|c| is_js_whitespace(c) || c == '-')
        .trim_end_matches(|c| is_js_whitespace(c) || c == '.')
        .to_owned()
}

/// Whether `name` is usable as a file name on every platform Bruno supports.
pub(crate) fn validate_name(name: &str) -> bool {
    // JavaScript's `length` counts UTF-16 code units.
    if name.is_empty() || name.encode_utf16().count() > 255 || is_reserved(name) {
        return false;
    }
    let first = name
        .chars()
        .next()
        .is_some_and(|c| !is_js_whitespace(c) && c != '-');
    let last = name
        .chars()
        .last()
        .is_some_and(|c| !is_js_whitespace(c) && c != '.');
    first && last && !name.chars().any(is_invalid)
}

/// Windows device names, matched case-insensitively against the whole name.
fn is_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let bytes = upper.as_bytes();
            bytes.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && bytes[3].is_ascii_digit()
        }
    }
}

/// Longest base that still fits a file name once `.ext` and Bruno's widest suffix (`" 199"`)
/// are added, measured in UTF-16 units like Bruno but cut at a character boundary.
pub(crate) fn truncate_for_suffix(base: &str, ext: Option<&str>) -> String {
    let reserved = 4 + ext.map_or(0, |e| e.encode_utf16().count() + 1);
    let limit = 255usize.saturating_sub(reserved);
    let mut units = 0;
    let mut out = String::new();
    for c in base.chars() {
        units += c.len_utf16();
        if units > limit {
            break;
        }
        out.push(c);
    }
    out
}

/// `base`, `base n`, with `.ext` appended; `n == 0` adds no suffix.
pub(crate) fn suffixed(base: &str, ext: Option<&str>, n: u32) -> String {
    let mut name = base.to_owned();
    if n > 0 {
        name.push_str(&format!(" {n}"));
    }
    if let Some(ext) = ext {
        name.push('.');
        name.push_str(ext);
    }
    name
}

/// Claim `base`, `base 1` ... `base 199` (truncated first): `create` failing with
/// `AlreadyExists` means taken; any other error is returned at once.
pub(crate) fn create_unique<T>(
    dir: &Path,
    base: &str,
    ext: Option<&str>,
    mut create: impl FnMut(&Path) -> io::Result<T>,
) -> io::Result<(PathBuf, T)> {
    create_unique_numbered(dir, base, ext, |path, _| create(path))
}

/// [`create_unique`], telling `create` the suffix number it tries (0 for none), so what it
/// writes can carry the same number.
pub(crate) fn create_unique_numbered<T>(
    dir: &Path,
    base: &str,
    ext: Option<&str>,
    mut create: impl FnMut(&Path, u32) -> io::Result<T>,
) -> io::Result<(PathBuf, T)> {
    let base = truncate_for_suffix(base, ext);
    for n in 0..=199 {
        let path = dir.join(suffixed(&base, ext, n));
        match create(&path, n) {
            Ok(made) => return Ok((path, made)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

/// Bruno's `getUniqueRenamePath`: `desired`, else the first free suffixed sibling. The source
/// itself counts as free, so renaming to the same name is not a collision.
pub(crate) fn unique_rename_target(source: &Path, desired: &Path) -> io::Result<PathBuf> {
    let free = |candidate: &Path| -> io::Result<bool> {
        if !candidate.try_exists()? {
            return Ok(true);
        }
        Ok(matches!(
            (candidate.canonicalize(), source.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        ))
    };
    if free(desired)? {
        return Ok(desired.to_owned());
    }
    let dir = desired.parent().unwrap_or_else(|| Path::new(""));
    // A directory has no extension, whatever dots its name holds.
    let ext = if source.is_dir() {
        None
    } else {
        desired.extension().and_then(|e| e.to_str())
    };
    let stem = if ext.is_some() {
        desired.file_stem()
    } else {
        desired.file_name()
    };
    let base = truncate_for_suffix(&stem.unwrap_or_default().to_string_lossy(), ext);
    for n in 1..=199 {
        let candidate = dir.join(suffixed(&base, ext, n));
        if free(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::ErrorKind;

    #[test]
    fn sanitize_matches_bruno() {
        let cases = [
            ("dev", "dev"),
            (" a/b?. ", "a-b-"),
            ("--x", "x"),
            ("-\u{a0} lead", "lead"),
            ("trail. . ", "trail"),
            ("a<b>c:d\"e|f*g\\h", "a-b-c-d-e-f-g-h"),
            ("tab\there", "tab-here"),
            ("in ner", "in ner"),
            ("...", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_name(input), expected, "{input:?}");
        }
    }

    #[test]
    fn validate_matches_bruno() {
        for good in ["dev", "a b", "x.y", "COM10", "con1", "é"] {
            assert!(validate_name(good), "{good:?}");
        }
        let long = "a".repeat(256);
        for bad in [
            "",
            " a",
            "-a",
            "a.",
            "a ",
            "a/b",
            "nul",
            "Com3",
            "LPT0",
            long.as_str(),
        ] {
            assert!(!validate_name(bad), "{bad:?}");
        }
        // 128 characters outside the BMP are 256 UTF-16 code units.
        assert!(!validate_name(&"😀".repeat(128)));
        assert!(validate_name(&"😀".repeat(127)));
    }

    #[test]
    fn names_match_brunos_sanitize_and_validate() {
        let table: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/reference/2026-10-09-tree-parity/names.json"
        ))
        .expect("json");
        let rows = table.as_array().expect("array");
        assert_eq!(rows.len(), 38);
        for row in rows {
            let input = row["input"].as_str().expect("input");
            let sanitized = row["sanitized"].as_str().expect("sanitized");
            assert_eq!(sanitize_name(input), sanitized, "sanitize {input:?}");
            let valid = row["valid"].as_bool().expect("valid");
            assert_eq!(validate_name(sanitized), valid, "validate {sanitized:?}");
        }
    }

    #[test]
    fn the_base_is_truncated_to_leave_room_for_a_suffix() {
        assert_eq!(
            truncate_for_suffix(&"x".repeat(250), Some("yml")).len(),
            247
        );
        assert_eq!(truncate_for_suffix(&"x".repeat(255), None).len(), 251);
        assert_eq!(truncate_for_suffix("short", Some("yml")), "short");
        // 125 emoji are 250 UTF-16 units; a cut at 247 would split a pair, so it rounds down.
        let cut = truncate_for_suffix(&"\u{1F600}".repeat(125), Some("yml"));
        assert_eq!(cut.chars().count(), 123);
    }

    #[test]
    fn suffixes_follow_brunos_spelling() {
        assert_eq!(suffixed("a", Some("yml"), 0), "a.yml");
        assert_eq!(suffixed("a", Some("yml"), 2), "a 2.yml");
        assert_eq!(suffixed("a", None, 1), "a 1");
    }

    #[test]
    fn create_unique_takes_the_first_free_name() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        fs::write(dir.path().join("a.yml"), "").expect("write");
        fs::write(dir.path().join("a 1.yml"), "").expect("write");
        let (path, ()) = create_unique(dir.path(), "a", Some("yml"), |p| {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(p)
                .map(drop)
        })
        .expect("creates");
        assert_eq!(path, dir.path().join("a 2.yml"));
    }

    #[test]
    fn create_unique_gives_up_after_200_names() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let mut tries = 0;
        let err = create_unique(dir.path(), "a", Some("yml"), |_| -> std::io::Result<()> {
            tries += 1;
            Err(ErrorKind::AlreadyExists.into())
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(tries, 200);
    }

    #[test]
    fn a_rename_target_avoids_other_files_but_not_the_source() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let (a, b) = (dir.path().join("a.yml"), dir.path().join("b.yml"));
        fs::write(&a, "").expect("write");
        fs::write(&b, "").expect("write");
        let c = dir.path().join("c.yml");
        assert_eq!(unique_rename_target(&a, &c).expect("free"), c);
        assert_eq!(
            unique_rename_target(&a, &b).expect("taken"),
            dir.path().join("b 1.yml")
        );
        assert_eq!(unique_rename_target(&a, &a).expect("itself"), a);
        fs::create_dir(dir.path().join("f")).expect("mkdir");
        fs::create_dir(dir.path().join("g")).expect("mkdir");
        let moved = unique_rename_target(&dir.path().join("f"), &dir.path().join("g"));
        assert_eq!(moved.expect("dir"), dir.path().join("g 1"));
    }
}
