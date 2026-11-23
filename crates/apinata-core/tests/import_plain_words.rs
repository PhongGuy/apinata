//! Import errors and notes read in plain words: no OS error codes, byte offsets, empty quotes
//! or internal member paths.

use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionOptions, FetchError, SourceFetch, fetch_source, parse_collection, parse_collections,
    parse_request, read_reason, write_collection,
};

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn collection_error(input: &str) -> String {
    parse_collections(None, input.as_bytes(), &CollectionOptions::default())
        .expect_err("the input is refused")
        .to_string()
}

#[test]
fn a_missing_file_reads_in_plain_words() {
    let dir = tempfile::tempdir().unwrap();
    let error = std::fs::read(dir.path().join("missing.json")).unwrap_err();
    let reason = read_reason(&error);
    assert_eq!(reason, "there is no file at that path");
}

#[test]
fn a_folder_given_as_a_file_reads_in_plain_words() {
    let dir = tempfile::tempdir().unwrap();
    let error = std::fs::read(dir.path()).unwrap_err();
    assert_eq!(read_reason(&error), "it is a folder, not a file");
}

#[test]
fn truncated_json_names_the_line_and_column() {
    let message = collection_error("{\"info\": {\"name\": \"x\",\n  \"item\": [");
    assert_eq!(
        message,
        "the JSON ends before the closing ] (line 2, column 12)"
    );
}

#[test]
fn invalid_yaml_names_the_line() {
    let message = collection_error("a: [1\nb: 2\n");
    assert!(message.starts_with("not valid YAML: "), "{message}");
    assert!(message.contains("(line "), "{message}");
    assert!(!message.contains("recognized"), "{message}");
    assert!(!message.contains("byte"), "{message}");
}

#[test]
fn input_that_parses_but_matches_no_format_stays_unrecognized() {
    let message = collection_error("{\"hello\": 1}");
    assert!(
        message.starts_with("not a recognized collection format"),
        "{message}"
    );
}

#[test]
fn a_postman_v1_file_says_it_is_an_older_format() {
    let message = collection_error(r#"{"id":"x","name":"old","requests":[],"order":[]}"#);
    assert!(!message.contains("``"), "{message}");
    assert!(message.contains("no schema"), "{message}");
    assert!(message.contains("older Postman format"), "{message}");
}

#[test]
fn a_windows_cmd_curl_command_says_what_to_do() {
    let message = parse_request(None, r#"curl ^"https://x.test/a^" -H ^"A: b^""#)
        .expect_err("cmd syntax is refused")
        .to_string();
    assert!(!message.contains("byte"), "{message}");
    assert!(message.contains("Windows cmd"), "{message}");
    assert!(message.contains("bash"), "{message}");
}

#[tokio::test]
async fn a_refused_connection_reads_in_plain_words() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let url = format!("http://127.0.0.1:{port}/c.json");
    let error = fetch_source(&url, &SourceFetch::default())
        .await
        .unwrap_err();
    assert!(matches!(error, FetchError::Transport(_)), "{error:?}");
    assert_eq!(error.to_string(), "127.0.0.1 refused the connection");
}

// The notes a Postman collection holding `items` leaves once written, each tied to its file.
fn postman(items: &str) -> Vec<apinata_core::import::ImportNote> {
    let input = format!(r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{items}]}}"#);
    let collection = parse_collection(None, input.as_bytes()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (_, report) = write_collection(dir.path(), &collection, &[], &MemoryStore::new()).unwrap();
    report.notes
}

#[test]
fn a_script_note_names_the_request_and_not_the_event_index() {
    let notes = postman(
        r#"{"name":"Get user","request":{"method":"GET","url":"https://x.test/u"},"event":[{"listen":"test","script":{"exec":["pm.test('ok')"]}}]}"#,
    );
    // The collection's note on how imported scripts run comes first; this one is the script's.
    let note = notes
        .into_iter()
        .find(|n| n.source.ends_with("event[0]"))
        .unwrap();
    assert_eq!(note.source, "Get user / event[0]");
    assert_eq!(note.file.as_deref(), Some("Get user.yml"));
    assert_eq!(note.place(), "request `Get user`");
    let mut unwritten = note.clone();
    unwritten.file = None;
    assert_eq!(unwritten.place(), "`Get user`");
    let mut folder = note;
    folder.file = Some("Users/folder.yml".to_owned());
    assert_eq!(folder.place(), "folder `Get user`");
}

// How imported scripts run is said once per import, on the collection and no one file.
#[test]
fn the_script_trust_note_is_said_once_per_import() {
    let script = |name: &str| {
        format!(
            r#"{{"name":"{name}","request":{{"method":"GET","url":"https://x.test/u"}},"event":[{{"listen":"test","script":{{"exec":["pm.test('ok')"]}}}}]}}"#
        )
    };
    let notes = postman(&format!("{},{}", script("a"), script("b")));
    let trust: Vec<_> = notes.iter().filter(|n| n.detail.contains("ask")).collect();
    assert_eq!(trust.len(), 1, "{notes:?}");
    assert_eq!(trust[0].source, "collection");
    assert_eq!(trust[0].file, None);
    assert_eq!(
        trust[0].detail,
        "imported scripts ask before they first run in the app or the MCP server, and apinata run runs them as your own command"
    );
    let plain = postman(r#"{"name":"c","request":{"method":"GET","url":"https://x.test/u"}}"#);
    assert!(plain.iter().all(|n| !n.detail.contains("ask")), "{plain:?}");
}

#[test]
fn a_member_path_reads_as_words() {
    let notes = postman(
        r#"{"name":"Login","request":{"method":"GET","url":"https://x.test/u","auth":{"type":"basic","basic":[{"key":"username","value":"u"},{"key":"password","value":"p"}]}}}"#,
    );
    let note = notes
        .iter()
        .find(|n| n.source.ends_with("request.auth.password"))
        .expect("the password note");
    assert_eq!(note.place(), "request `Login`, auth password");
}

#[test]
fn a_long_name_is_shortened_in_the_middle() {
    let name = format!("start{}end", "x".repeat(300));
    let item = format!(
        r#"{{"name":"{name}","request":{{"method":"GET","url":"https://x.test/u"}},"event":[{{"listen":"test","script":{{"exec":["pm.test('ok')"]}}}}]}}"#
    );
    let notes = postman(&item);
    let note = notes
        .iter()
        .find(|n| n.source.ends_with("event[0]"))
        .expect("the script note");
    let place = note.place();
    assert!(place.starts_with("request `start"), "{place}");
    assert!(place.ends_with("end`"), "{place}");
    assert!(place.contains('…'), "{place}");
    assert!(place.chars().count() <= 90, "{place}");
}

#[test]
fn a_long_name_in_a_detail_is_shortened_in_the_middle() {
    let name = format!("start{}end", "y".repeat(300));
    let item = format!(
        r#"{{"name":"{name}","request":{{"method":"GET","url":"https://x.test/u","auth":{{"type":"bearer","bearer":[{{"key":"token","value":"t"}}]}}}}}}"#
    );
    let notes = postman(&item);
    let note = notes
        .iter()
        .find(|n| n.source.ends_with("request.auth.token"))
        .expect("the token note");
    assert!(note.detail.chars().count() > 300, "{}", note.detail);
    let shown = note.shown_detail();
    assert!(shown.contains('…'), "{shown}");
    assert!(shown.chars().count() < 200, "{shown}");
}
