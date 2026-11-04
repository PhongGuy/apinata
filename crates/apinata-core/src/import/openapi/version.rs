//! The OpenAPI or Swagger version a document declares.

use crate::import::DocumentError;
use serde_json::Value;

/// The specification versions the importer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Version {
    Swagger2,
    V30,
    V31,
    V32,
}

impl Version {
    /// Read the version from `root`'s `openapi` key, else its `swagger` key. A version outside
    /// 3.0, 3.1, 3.2 and Swagger 2.0 is `Unsupported`; a root that is not an object or declares
    /// neither key is `Invalid`.
    pub(crate) fn of(root: &Value) -> Result<Version, DocumentError> {
        let Some(map) = root.as_object() else {
            return Err(invalid("the document is not an object"));
        };
        if let Some(declared) = map.get("openapi") {
            let text = version_text(declared);
            // Only major.minor counts, so `3.1` and `3.1.0` agree while `3.10.0` stays apart.
            let mut parts = text.split('.');
            let version = match (parts.next(), parts.next()) {
                (Some("3"), Some("0")) => Some(Version::V30),
                (Some("3"), Some("1")) => Some(Version::V31),
                (Some("3"), Some("2")) => Some(Version::V32),
                _ => None,
            };
            return version.ok_or(DocumentError::Unsupported {
                found: format!("openapi {text}"),
            });
        }
        if let Some(declared) = map.get("swagger") {
            let text = version_text(declared);
            return match text.as_str() {
                "2.0" | "2" => Ok(Version::Swagger2),
                _ => Err(DocumentError::Unsupported {
                    found: format!("swagger {text}"),
                }),
            };
        }
        Err(invalid(
            "the document declares neither an `openapi` nor a `swagger` version",
        ))
    }
}

/// A declared version as written: a YAML number keeps its decimal form, so `3.0` stays `3.0`.
fn version_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn invalid(message: &str) -> DocumentError {
    DocumentError::Invalid {
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_are_read_from_strings_or_numbers() {
        let cases = [
            (json!({"openapi": "3.0.3"}), Version::V30),
            (json!({"openapi": "3.0"}), Version::V30),
            (json!({"openapi": 3.0}), Version::V30),
            (json!({"openapi": "3.1.0"}), Version::V31),
            (json!({"openapi": 3.1}), Version::V31),
            (json!({"openapi": "3.2.0"}), Version::V32),
            (json!({"swagger": "2.0"}), Version::Swagger2),
            (json!({"swagger": 2.0}), Version::Swagger2),
            (json!({"swagger": 2}), Version::Swagger2),
            (json!({"openapi": "3.1.0", "swagger": "2.0"}), Version::V31),
        ];
        for (doc, want) in cases {
            assert_eq!(Version::of(&doc), Ok(want), "{doc}");
        }
    }

    #[test]
    fn unknown_versions_are_unsupported() {
        let cases = [
            (json!({"openapi": "4.0.0"}), "openapi 4.0.0"),
            (json!({"openapi": "3.10.0"}), "openapi 3.10.0"),
            (json!({"openapi": "2.0"}), "openapi 2.0"),
            (json!({"openapi": 3}), "openapi 3"),
            (json!({"openapi": true}), "openapi true"),
            (json!({"swagger": "1.2"}), "swagger 1.2"),
        ];
        for (doc, found) in cases {
            assert_eq!(
                Version::of(&doc),
                Err(DocumentError::Unsupported {
                    found: found.into()
                }),
                "{doc}"
            );
        }
        assert!(matches!(
            Version::of(&json!([1])),
            Err(DocumentError::Invalid { .. })
        ));
        assert!(matches!(
            Version::of(&json!({"info": {}})),
            Err(DocumentError::Invalid { .. })
        ));
    }
}
