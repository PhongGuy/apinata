//! Multipart bodies framed byte for byte as form-data 4.0.4 frames them for bruno-cli
//! reqwest's own form cannot take a supplied
//! boundary, so framing is done here.

use std::fmt;
use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regress::Regex;

use crate::mime_table::MIME_TYPES;

/// One part of a multipart body: a field name, its text or file value, and an optional
/// content type that overrides the one looked up for a file.
#[derive(Clone)]
#[non_exhaustive]
pub struct Part {
    name: String,
    value: PartValue,
    content_type: Option<String>,
    /// The file path as the request file wrote it, reported in errors instead of the
    /// interpolated path so a secret never shows.
    written: Option<String>,
}

impl PartialEq for Part {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.value == other.value
            && self.content_type == other.content_type
    }
}

/// The value of a multipart part.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum PartValue {
    /// A text value, sent as written.
    Text(String),
    /// A file, read when the request is sent.
    File(PathBuf),
}

impl Part {
    /// A text part.
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Part {
        Part::new(name.into(), PartValue::Text(value.into()))
    }

    /// A file part, typed from the path's extension unless a content type is given.
    pub fn file(name: impl Into<String>, path: impl Into<PathBuf>) -> Part {
        Part::new(name.into(), PartValue::File(path.into()))
    }

    fn new(name: String, value: PartValue) -> Part {
        Part {
            name,
            value,
            content_type: None,
            written: None,
        }
    }

    /// The same part with an explicit `Content-Type`.
    pub fn with_content_type(self, content_type: impl Into<String>) -> Part {
        Part {
            content_type: Some(content_type.into()),
            ..self
        }
    }

    /// The field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The text or file value.
    pub fn value(&self) -> &PartValue {
        &self.value
    }

    /// The explicit content type, if one was given.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// The same part remembering the path as the request file wrote it, before interpolation.
    pub(crate) fn with_written(self, written: impl Into<String>) -> Part {
        Part {
            written: Some(written.into()),
            ..self
        }
    }
}

// Values and paths may hold secrets, so Debug shows only names and a text value's size.
impl fmt::Debug for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Part")
            .field("name", &self.name)
            .field("value", &self.value)
            .field("content_type", &self.content_type.is_some())
            .finish()
    }
}

impl fmt::Debug for PartValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartValue::Text(text) => write!(f, "Text({} bytes)", text.len()),
            PartValue::File(_) => f.write_str("File"),
        }
    }
}

/// bruno-common `extractBoundaryFromContentType`: a quoted boundary first, else a bare one.
// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static BOUNDARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::with_flags(r#"boundary="([^"]+)"|boundary=([^;\s]+)"#, "i")
        .expect("bruno-common boundary regex compiles")
});

/// The boundary a `Content-Type` value names, if any.
pub(crate) fn boundary_from(content_type: &str) -> Option<String> {
    let found = BOUNDARY.find(content_type)?;
    let group = found.group(1).or_else(|| found.group(2))?;
    Some(content_type[group].to_owned())
}

/// A fresh boundary in form-data's shape: 26 hyphens and 24 hex digits.
pub(crate) fn new_boundary() -> String {
    // Each RandomState is freshly seeded, so two hashes give 96 unpredictable-enough bits.
    let high = RandomState::new().hash_one(0u8);
    let low = RandomState::new().hash_one(1u8) as u32;
    format!("{}{high:016x}{low:08x}", "-".repeat(26))
}

/// The MIME type of the file at `path`, looked up in mime-db by its lower-cased extension;
/// `application/octet-stream` when it has no extension or an unknown one.
pub fn file_content_type(path: &Path) -> &'static str {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .and_then(|ext| {
            MIME_TYPES
                .binary_search_by(|(key, _)| (*key).cmp(ext.as_str()))
                .ok()
        })
        .map_or("application/octet-stream", |i| MIME_TYPES[i].1)
}

/// Frames `parts` under `boundary`, reading each file part from disk. On a read failure it
/// returns the part's written path (else the path read) with the I/O error.
pub(crate) async fn encode(
    parts: &[Part],
    boundary: &str,
) -> Result<Vec<u8>, (PathBuf, std::io::Error)> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        // Names and file names go out unescaped, as form-data writes them.
        let mut head = format!("Content-Disposition: form-data; name=\"{}\"", part.name);
        let (bytes, looked_up) = match &part.value {
            PartValue::Text(text) => (text.as_bytes().to_vec(), None),
            PartValue::File(path) => {
                let bytes = tokio::fs::read(path).await.map_err(|err| {
                    let shown = part
                        .written
                        .as_ref()
                        .map_or_else(|| path.clone(), PathBuf::from);
                    (shown, err)
                })?;
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                head.push_str(&format!("; filename=\"{file_name}\""));
                (bytes, Some(file_content_type(path)))
            }
        };
        head.push_str("\r\n");
        if let Some(content_type) = part.content_type.as_deref().or(looked_up) {
            head.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        head.push_str("\r\n");
        body.extend_from_slice(head.as_bytes());
        body.extend_from_slice(&bytes);
        body.extend_from_slice(b"\r\n");
    }
    // form-data writes no closing delimiter for an empty form.
    if !parts.is_empty() {
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire bytes recorded from bruno-cli's createFormData with form-data 4.0.4 on 2026-10-09,
    /// the boundary written as `<B>`.
    const FIXTURE_WIRE: &str = "--<B>\r\nContent-Disposition: form-data; name=\"mp_str\"\r\n\r\nform_string\r\n--<B>\r\nContent-Disposition: form-data; name=\"mp_num\"\r\n\r\n42\r\n--<B>\r\nContent-Disposition: form-data; name=\"mp_bool\"\r\n\r\ntrue\r\n--<B>\r\nContent-Disposition: form-data; name=\"mp_obj\"\r\n\r\n{\"key\":\"value\"}\r\n--<B>--\r\n";
    const FILES_WIRE: &str = "--<B>\r\nContent-Disposition: form-data; name=\"f1\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello file\n\r\n--<B>\r\nContent-Disposition: form-data; name=\"f2\"; filename=\"b.json\"\r\nContent-Type: application/json\r\n\r\n{\"k\":1}\r\n--<B>\r\nContent-Disposition: form-data; name=\"f3\"; filename=\"c.unknownext\"\r\nContent-Type: application/octet-stream\r\n\r\n\u{0}\u{1}\u{2}bin\r\n--<B>\r\nContent-Disposition: form-data; name=\"f4\"; filename=\"d.png\"\r\nContent-Type: image/x-custom\r\n\r\nPNGDATA\r\n--<B>\r\nContent-Disposition: form-data; name=\"t1\"\r\nContent-Type: application/json\r\n\r\nv\r\n--<B>\r\nContent-Disposition: form-data; name=\"list\"\r\n\r\nx\r\n--<B>\r\nContent-Disposition: form-data; name=\"list\"\r\n\r\ny\r\n--<B>\r\nContent-Disposition: form-data; name=\"many\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello file\n\r\n--<B>\r\nContent-Disposition: form-data; name=\"many\"; filename=\"b.json\"\r\nContent-Type: application/json\r\n\r\n{\"k\":1}\r\n--<B>\r\nContent-Disposition: form-data; name=\"empty\"\r\n\r\n\r\n--<B>--\r\n";

    fn wire(golden: &str, boundary: &str) -> Vec<u8> {
        golden.replace("<B>", boundary).into_bytes()
    }

    #[tokio::test]
    async fn text_parts_frame_as_form_data_does() {
        let parts = [
            Part::text("mp_str", "form_string"),
            Part::text("mp_num", "42"),
            Part::text("mp_bool", "true"),
            Part::text("mp_obj", "{\"key\":\"value\"}"),
        ];
        let boundary = "--------------------------0123456789abcdef01234567";
        let body = encode(&parts, boundary).await.expect("encodes");
        assert_eq!(body, wire(FIXTURE_WIRE, boundary));
    }

    #[tokio::test]
    async fn file_parts_carry_a_filename_and_a_looked_up_or_given_type() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).expect("write");
            path
        };
        let a = file("a.txt", b"hello file\n");
        let b = file("b.json", b"{\"k\":1}");
        let c = file("c.unknownext", b"\x00\x01\x02bin");
        let d = file("d.png", b"PNGDATA");
        let parts = [
            Part::file("f1", &a),
            Part::file("f2", &b),
            Part::file("f3", &c),
            Part::file("f4", &d).with_content_type("image/x-custom"),
            Part::text("t1", "v").with_content_type("application/json"),
            Part::text("list", "x"),
            Part::text("list", "y"),
            Part::file("many", &a),
            Part::file("many", &b),
            Part::text("empty", ""),
        ];
        let body = encode(&parts, "XyZ123").await.expect("encodes");
        assert_eq!(
            String::from_utf8_lossy(&body),
            String::from_utf8_lossy(&wire(FILES_WIRE, "XyZ123"))
        );
    }

    #[tokio::test]
    async fn no_parts_is_an_empty_body_and_a_missing_file_names_its_path() {
        assert_eq!(encode(&[], "b").await.expect("encodes"), b"");
        let gone = Path::new("/nonexistent-apinata-dir/x.bin");
        let parts = [Part::text("a", "1"), Part::file("f", gone)];
        let (path, _) = encode(&parts, "b").await.expect_err("missing file");
        assert_eq!(path, gone);
        // The path as the request file wrote it is reported instead, so no secret is shown.
        let parts = [Part::file("f", gone).with_written("{{token}}/x.bin")];
        let (path, _) = encode(&parts, "b").await.expect_err("missing file");
        assert_eq!(path, Path::new("{{token}}/x.bin"));
    }

    #[test]
    fn a_boundary_is_read_from_the_content_type_as_bruno_common_does() {
        let cases = [
            ("multipart/form-data; boundary=XyZ123", Some("XyZ123")),
            (
                "multipart/form-data; BOUNDARY=\"a b\"; charset=x",
                Some("a b"),
            ),
            ("multipart/mixed;boundary=abc;charset=utf-8", Some("abc")),
            ("multipart/form-data; boundary=\"open", Some("\"open")),
            ("multipart/form-data", None),
            ("text/plain", None),
        ];
        for (content_type, want) in cases {
            assert_eq!(
                boundary_from(content_type).as_deref(),
                want,
                "{content_type}"
            );
        }
    }

    #[test]
    fn new_boundaries_have_form_datas_shape_and_differ() {
        let one = new_boundary();
        assert_eq!(one.len(), 50, "{one}");
        assert!(one.starts_with(&"-".repeat(26)), "{one}");
        assert!(one[26..].chars().all(|c| c.is_ascii_hexdigit()), "{one}");
        assert_ne!(one, new_boundary());
    }

    #[test]
    fn file_types_come_from_the_generated_mime_db_table() {
        assert_eq!(MIME_TYPES.len(), 1180);
        assert!(
            MIME_TYPES.windows(2).all(|w| w[0].0 < w[1].0),
            "sorted, no repeats"
        );
        let cases = [
            ("/x/a.txt", "text/plain"),
            ("/x/B.JSON", "application/json"),
            ("/x/c.tar.gz", "application/gzip"),
            ("/x/clip.mp4", "video/mp4"),
            ("/x/c.unknownext", "application/octet-stream"),
            ("/x/noext", "application/octet-stream"),
            ("/x/.json", "application/octet-stream"),
        ];
        for (path, want) in cases {
            assert_eq!(file_content_type(Path::new(path)), want, "{path}");
        }
    }

    #[test]
    fn debug_shows_names_and_sizes_only() {
        let parts = vec![
            Part::text("token", "SECRET-VALUE"),
            Part::file("upload", "/home/SECRET-PATH/key.pem"),
        ];
        let shown = format!("{parts:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(
            shown.contains("token") && shown.contains("upload"),
            "{shown}"
        );
    }
}
