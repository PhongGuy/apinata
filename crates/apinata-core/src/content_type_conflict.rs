//! Whether a hand-set `Content-Type` header disagrees with the request's body mode. A header
//! the user enabled wins over the body's own type when sending, so a mismatch is only named.

use crate::{BodySpec, KeyValue};

/// An enabled `Content-Type` header whose value the body mode cannot be sent as.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentTypeConflict {
    /// The header's name as written.
    pub name: String,
    /// The header's value, which is what a send uses.
    pub value: String,
    /// The type the body sets when no header overrides it.
    pub body_type: &'static str,
}

/// The `Content-Type` a body mode sends when no header overrides it; `None` for no body and
/// for a file body, whose entry's own type replaces any header.
pub fn body_content_type(body: &BodySpec) -> Option<&'static str> {
    match body {
        // An empty JSON body is no body at all, as resolving it shows.
        BodySpec::Json(text) if text.is_empty() => None,
        BodySpec::Json(_) | BodySpec::Graphql { .. } => Some("application/json"),
        BodySpec::Xml(_) => Some("application/xml"),
        BodySpec::Text(_) => Some("text/plain"),
        BodySpec::Sparql(_) => Some("application/sparql-query"),
        BodySpec::FormUrlEncoded(_) => Some("application/x-www-form-urlencoded"),
        BodySpec::Multipart(_) => Some("multipart/form-data"),
        BodySpec::None | BodySpec::File(_) | BodySpec::Unsupported { .. } => None,
    }
}

/// The last enabled `Content-Type` in `headers` (the one a send uses) when the body mode
/// cannot be sent under it, such as a form under a JSON type or multipart under a type with no
/// boundary. Raw-text modes never conflict, nor does a value holding a `{{variable}}`.
pub fn content_type_conflict(headers: &[KeyValue], body: &BodySpec) -> Option<ContentTypeConflict> {
    let header = headers
        .iter()
        .rev()
        .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))?;
    if header.value.contains("{{") {
        return None;
    }
    let media = media_type(&header.value);
    // Only modes whose encoding the type decides are judged; raw text is sent as written.
    let fits = match body {
        BodySpec::Json(_) | BodySpec::Graphql { .. } => {
            media.ends_with("/json") || media.ends_with("+json")
        }
        BodySpec::Xml(_) => media.ends_with("/xml") || media.ends_with("+xml"),
        BodySpec::FormUrlEncoded(_) => media == "application/x-www-form-urlencoded",
        // A send appends the boundary to any multipart type that lacks one.
        BodySpec::Multipart(_) => media.starts_with("multipart/"),
        _ => return None,
    };
    let body_type = body_content_type(body)?;
    (!fits).then(|| ContentTypeConflict {
        name: header.name.clone(),
        value: header.value.clone(),
        body_type,
    })
}

/// Whether `value` names exactly `body`'s own type and nothing more, so dropping the header
/// sends the same request.
pub(crate) fn restates_body_type(value: &str, body: &BodySpec) -> bool {
    body_content_type(body).is_some_and(|own| value.trim().eq_ignore_ascii_case(own))
}

/// The media type of a `Content-Type` value, lowercased, without its parameters.
fn media_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MultipartField;

    fn header(name: &str, value: &str) -> Vec<KeyValue> {
        vec![KeyValue::new(name, value)]
    }

    fn form() -> BodySpec {
        BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1")])
    }

    fn multipart() -> BodySpec {
        BodySpec::Multipart(vec![MultipartField::text("a", "1")])
    }

    fn json() -> BodySpec {
        BodySpec::Json("{}".to_owned())
    }

    #[test]
    fn multipart_under_a_type_without_a_boundary_conflicts() {
        let found =
            content_type_conflict(&header("Content-Type", "application/json"), &multipart());
        assert_eq!(
            found,
            Some(ContentTypeConflict {
                name: "Content-Type".to_owned(),
                value: "application/json".to_owned(),
                body_type: "multipart/form-data",
            })
        );
    }

    #[test]
    fn multipart_under_a_multipart_type_does_not_conflict() {
        // A send appends the boundary to any multipart type that lacks one.
        let bare = header("Content-Type", "multipart/form-data");
        assert_eq!(content_type_conflict(&bare, &multipart()), None);
        let named = header("Content-Type", "multipart/form-data; boundary=x");
        assert_eq!(content_type_conflict(&named, &multipart()), None);
    }

    #[test]
    fn form_urlencoded_under_a_json_type_conflicts() {
        let found = content_type_conflict(&header("Content-Type", "application/json"), &form());
        assert_eq!(
            found.map(|c| c.body_type),
            Some("application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn json_under_a_json_type_does_not_conflict() {
        let headers = header("Content-Type", "application/json");
        assert_eq!(content_type_conflict(&headers, &json()), None);
        let vendor = header("Content-Type", "application/vnd.api+json");
        assert_eq!(content_type_conflict(&vendor, &json()), None);
    }

    #[test]
    fn json_under_a_form_type_conflicts() {
        let headers = header("Content-Type", "application/x-www-form-urlencoded");
        let found = content_type_conflict(&headers, &json());
        assert_eq!(found.map(|c| c.body_type), Some("application/json"));
    }

    #[test]
    fn charset_and_case_in_the_value_are_ignored() {
        let headers = header("Content-Type", "Application/JSON; charset=utf-8");
        assert_eq!(content_type_conflict(&headers, &json()), None);
        let form_type = header(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        );
        assert_eq!(content_type_conflict(&form_type, &form()), None);
    }

    #[test]
    fn the_header_name_matches_in_any_case() {
        let found = content_type_conflict(&header("content-TYPE", "text/plain"), &form());
        assert_eq!(found.map(|c| c.name), Some("content-TYPE".to_owned()));
    }

    #[test]
    fn a_disabled_header_is_not_sent_so_does_not_conflict() {
        let mut headers = header("Content-Type", "application/json");
        headers[0].enabled = false;
        assert_eq!(content_type_conflict(&headers, &form()), None);
    }

    #[test]
    fn the_last_enabled_header_is_the_one_judged() {
        let headers = vec![
            KeyValue::new("Content-Type", "application/json"),
            KeyValue::new("content-type", "application/x-www-form-urlencoded"),
        ];
        assert_eq!(content_type_conflict(&headers, &form()), None);
    }

    #[test]
    fn raw_modes_and_variables_never_conflict() {
        let headers = header("Content-Type", "application/json");
        assert_eq!(
            content_type_conflict(&headers, &BodySpec::Text("x".to_owned())),
            None
        );
        assert_eq!(content_type_conflict(&headers, &BodySpec::None), None);
        assert_eq!(
            content_type_conflict(&headers, &BodySpec::File(vec![])),
            None
        );
        let variable = header("Content-Type", "{{type}}");
        assert_eq!(content_type_conflict(&variable, &form()), None);
    }

    #[test]
    fn each_mode_names_the_type_it_sends() {
        assert_eq!(body_content_type(&json()), Some("application/json"));
        assert_eq!(
            body_content_type(&form()),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(body_content_type(&multipart()), Some("multipart/form-data"));
        assert_eq!(
            body_content_type(&BodySpec::Xml("<a/>".to_owned())),
            Some("application/xml")
        );
        assert_eq!(body_content_type(&BodySpec::File(vec![])), None);
    }
}
