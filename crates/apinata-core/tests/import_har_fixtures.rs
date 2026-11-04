//! Every HAR fixture and a synthetic Chrome capture: written trees, unknown members,
//! credentials never written, hostile sizes, and no other tool named in any text.

use apinata_core::AuthSpec;
use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, FormatError, ImportBundle, ImportReport,
    ImportedItem, NoteKind, parse_collections, write_collection,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Bare request objects without `log`: not HAR logs.
const INVALID: &[&str] = &[
    "form-data-input.json",
    "minimal-input.json",
    "shallow-input.json",
];
const SECRETS: &[&str] = &[
    "s3cr3t",
    "YW5uOnMzY3IzdC1wdy05MQ==",
    "cHJveHk6czNjcjN0LXByb3h5LTkx",
];

/// Chrome-style: pseudo-headers, Basic, Bearer on two hosts, a query token, Digest, a proxy, a cookie header
/// duplicated by cookies[], custom fields, a (binary) upload, a urlencoded body, two non-HTTP entries.
const CHROME: &str = r#"{"log":{"version":"1.2","creator":{"name":"WebInspector","version":"537.36"},"pages":[],"entries":[
 {"_initiator":{"type":"other"},"_priority":"High","_resourceType":"fetch","request":{"method":"GET","url":"https://api.shop.test/me","httpVersion":"http/2.0",
  "headers":[{"name":":authority","value":"api.shop.test"},{"name":":method","value":"GET"},{"name":":path","value":"/me"},{"name":":scheme","value":"https"},
   {"name":"accept-encoding","value":"gzip"},{"name":"authorization","value":"Basic YW5uOnMzY3IzdC1wdy05MQ=="},
   {"name":"cookie","value":"sid=s3cr3t-sid-91; theme=dark"},{"name":"user-agent","value":"Mozilla/5.0"}],
  "cookies":[{"name":"sid","value":"s3cr3t-sid-91"},{"name":"theme","value":"dark"}],"queryString":[],"headersSize":-1,"bodySize":0},
  "response":{"status":200},"cache":{},"timings":{"wait":1}},
 {"request":{"method":"POST","url":"https://api.shop.test/orders?src=web&access_token=s3cr3t-qs-91","httpVersion":"http/2.0",
  "headers":[{"name":"authorization","value":"Bearer s3cr3t-tok-91"},{"name":"content-type","value":"application/json"},{"name":"content-length","value":"9"}],
  "cookies":[],"queryString":[{"name":"src","value":"web"},{"name":"access_token","value":"s3cr3t-qs-91"}],"postData":{"mimeType":"application/json","text":"{\"qty\":1}"},"headersSize":-1,"bodySize":9}},
 {"request":{"method":"POST","url":"https://api.shop.test/upload","httpVersion":"http/2.0",
  "headers":[{"name":"authorization","value":"Digest username=\"ann\", response=\"s3cr3t-dig-91\""},
   {"name":"proxy-authorization","value":"Basic cHJveHk6czNjcjN0LXByb3h5LTkx"},{"name":"content-type","value":"multipart/form-data; boundary=B"}],
  "cookies":[],"queryString":[],"postData":{"mimeType":"multipart/form-data; boundary=B","text":"--B\r\n(binary)\r\n--B--",
   "params":[{"name":"title","value":"Hi"},{"name":"file","value":"(binary)"}]},"headersSize":-1,"bodySize":-1}},
 {"request":{"method":"POST","url":"https://static.shop.test:8443/form","httpVersion":"HTTP/1.1",
  "headers":[{"name":"Authorization","value":"Bearer s3cr3t-tok-91"}],"cookies":[],"queryString":[],
  "postData":{"mimeType":"application/x-www-form-urlencoded","text":"a=1&b=x%20y","params":[{"name":"a","value":"1"},{"name":"b","value":"x%20y"}]},
  "headersSize":-1,"bodySize":11}},
 {"request":{"method":"GET","url":"data:text/plain,hi","httpVersion":"","headers":[],"cookies":[],"queryString":[],"headersSize":-1,"bodySize":0}},
 {"request":{"method":"GET","url":"wss://api.shop.test/live","httpVersion":"","headers":[],"cookies":[],"queryString":[],"headersSize":-1,"bodySize":0}}
]}}"#;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/har/kong")
}

fn parse(input: &[u8]) -> Result<Vec<ImportBundle>, FormatError> {
    parse_collections(
        Some(CollectionFormat::Har),
        input,
        &CollectionOptions::default(),
    )
}

fn files(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(dir).expect("dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            files(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, fs::read_to_string(&path).expect("utf-8"));
        }
    }
}

/// Write each bundle with its environments; return the written files, the reports and a rendering.
fn write(bundles: &[ImportBundle]) -> (Vec<String>, Vec<ImportReport>, String) {
    let parent = TempDir::new().expect("tempdir");
    let (mut texts, mut reports, mut out) = (Vec::new(), Vec::new(), String::new());
    for b in bundles {
        let (c, report) = write_collection(
            parent.path(),
            &b.collection,
            &b.environments,
            &MemoryStore::new(),
        )
        .expect("writes");
        let mut all = BTreeMap::new();
        files(c.root(), c.root(), &mut all);
        for (path, text) in &all {
            out.push_str(&format!("== {path}\n{text}"));
            texts.push(text.clone());
        }
        out.push_str("== report\n");
        for n in &report.notes {
            out.push_str(&format!(
                "{} {}: {} ({})\n",
                n.kind.id(),
                n.source,
                n.detail,
                n.file.as_deref().unwrap_or("-")
            ));
        }
        out.push_str(&format!(
            "secrets_to_enter: {:?}\n",
            report.secrets_to_enter
        ));
        reports.push(report);
    }
    (texts, reports, out)
}

fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures())
        .expect("fixtures")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names
}

#[test]
fn every_fixture_imports_or_is_invalid_as_listed() {
    for name in fixture_names() {
        let input = fs::read(fixtures().join(&name)).expect("read");
        match parse(&input) {
            Ok(bundles) => {
                assert!(!INVALID.contains(&name.as_str()), "{name} parsed");
                let (_, _, text) = write(&bundles);
                insta::assert_snapshot!(
                    format!("har_{}", name.trim_end_matches(".json").replace('-', "_")),
                    text
                );
            }
            Err(FormatError::Har(DocumentError::Invalid { message })) => {
                assert!(INVALID.contains(&name.as_str()), "{name}: {message}");
                assert_eq!(message, "log.entries is missing");
            }
            Err(e) => panic!("{name}: {e}"),
        }
    }
    let auto = parse_collections(
        None,
        &fs::read(fixtures().join("deep-input.json")).expect("read"),
        &CollectionOptions::default(),
    );
    assert_eq!(
        format!("{auto:?}"),
        format!(
            "{:?}",
            parse(&fs::read(fixtures().join("deep-input.json")).expect("read"))
        )
    );
}

#[test]
fn the_synthetic_chrome_capture_is_written_without_any_credential() {
    let bundles = parse(CHROME.as_bytes()).expect("parses");
    let debug = format!("{bundles:?}");
    let (texts, reports, rendered) = write(&bundles);
    for secret in SECRETS {
        assert!(!debug.contains(secret), "{secret} in Debug");
        assert!(!rendered.contains(secret), "{secret} written or in a note");
        assert!(
            texts.iter().all(|t| !t.contains(secret)),
            "{secret} written"
        );
    }
    let want: Vec<(String, String)> = [
        "api_shop_test_username",
        "api_shop_test_password",
        "api_shop_test_cookie",
        "api_shop_test_access_token",
        "api_shop_test_token",
        "api_shop_test_authorization",
        "api_shop_test_proxy_authorization",
        "static_shop_test_token",
    ]
    .iter()
    .map(|v| ("Capture".to_owned(), v.to_string()))
    .collect();
    assert_eq!(reports[0].secrets_to_enter, want);
    insta::assert_snapshot!("har_synthetic_chrome", rendered);
}

#[test]
fn no_captured_credential_is_written_anywhere() {
    // deep-input carries `cookie: foo=bar` and two cookies[].
    let bundles =
        parse(&fs::read(fixtures().join("deep-input.json")).expect("read")).expect("parses");
    let (texts, reports, rendered) = write(&bundles);
    for secret in ["foo=bar", "df7b43586f071c4aaa79dc8a9de0845771478571055"] {
        assert!(
            texts.iter().all(|t| !t.contains(secret)) && !rendered.contains(secret),
            "{secret}"
        );
    }
    assert_eq!(
        reports[0].secrets_to_enter,
        [("Capture".to_owned(), "insomnia_rest_cookie".to_owned())]
    );
}

const PROBE: &str = "zzApinataProbe";

/// Add `PROBE: 1` to the object at `at` under `node`, when that object exists.
fn plant(node: &mut Value, at: &[&str]) -> bool {
    let mut target = node;
    for key in at {
        target = match key.parse::<usize>() {
            Ok(i) => match target.get_mut(i) {
                Some(t) => t,
                None => return false,
            },
            Err(_) => match target.get_mut(*key) {
                Some(t) => t,
                None => return false,
            },
        };
    }
    target
        .as_object_mut()
        .map(|o| o.insert(PROBE.into(), Value::from(1)))
        .is_some()
}

#[test]
fn unknown_members_are_reported_at_every_level() {
    let mut inputs: Vec<(String, Vec<u8>)> = vec![("synthetic".into(), CHROME.as_bytes().to_vec())];
    for name in fixture_names()
        .into_iter()
        .filter(|n| !INVALID.contains(&n.as_str()))
    {
        inputs.push((
            name.clone(),
            fs::read(fixtures().join(&name)).expect("read"),
        ));
    }
    for (name, bytes) in inputs {
        let mut json: Value = serde_json::from_slice(&bytes).expect("json");
        let mut want = vec![
            "collection / zzApinataProbe".to_owned(),
            "collection / log.zzApinataProbe".to_owned(),
        ];
        plant(&mut json, &[]);
        plant(&mut json, &["log"]);
        let entries = json["log"]["entries"].as_array().map_or(0, Vec::len);
        for i in 0..entries {
            let url = json["log"]["entries"][i]["request"]["url"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            if !url.starts_with("http") {
                continue;
            }
            let e = i.to_string();
            for (at, suffix) in [
                (vec!["log", "entries", e.as_str()], " / zzApinataProbe"),
                (
                    vec!["log", "entries", e.as_str(), "request"],
                    " / request.zzApinataProbe",
                ),
                (
                    vec!["log", "entries", e.as_str(), "request", "postData"],
                    " / request.postData.zzApinataProbe",
                ),
                (
                    vec!["log", "entries", e.as_str(), "request", "headers", "0"],
                    " / request.headers[0].zzApinataProbe",
                ),
            ] {
                if plant(&mut json, &at) {
                    want.push(suffix.to_owned());
                }
            }
        }
        let bundles = parse(&serde_json::to_vec(&json).expect("json")).expect("parses");
        let (_, reports, _) = write(&bundles);
        let dropped: Vec<&str> = reports[0]
            .notes
            .iter()
            .filter(|n| n.kind == NoteKind::Dropped)
            .map(|n| n.source.as_str())
            .collect();
        for w in &want {
            let found = if w.starts_with("collection") {
                dropped.contains(&w.as_str())
            } else {
                dropped
                    .iter()
                    .any(|s| s.ends_with(w.as_str()) && !s.starts_with("collection"))
            };
            assert!(found, "{name}: no {w} in {dropped:?}");
        }
        // Exempt members make no note: every other Dropped note is an unwritten entry.
        assert!(
            dropped
                .iter()
                .all(|s| s.contains(PROBE) || s.starts_with("entries[")),
            "{name}: {dropped:?}"
        );
    }
}

#[test]
fn hostile_sizes_import_without_trouble() {
    // Each entry brings a new token for one host, so the naming runs 100,000 times on one base.
    let entry = |i: usize| {
        format!(
            r#"{{"request":{{"method":"GET","url":"https://h.test/{i}","headers":[{{"name":"Authorization","value":"Bearer t-{i}"}}]}}}}"#
        )
    };
    let har = |n: usize| {
        let many: Vec<String> = (0..n).map(entry).collect();
        format!(r#"{{"log":{{"entries":[{}]}}}}"#, many.join(","))
    };
    // A scan per new token would make the cost grow with the square of the entries. Compare
    // against a run a tenth the size on the same machine at the same moment: linear work costs
    // about 10 times as much, a scan about 100 times, and load slows both runs alike.
    let small = har(10_000);
    let started = Instant::now();
    parse(small.as_bytes()).expect("parses");
    let baseline = started.elapsed().max(Duration::from_millis(50));
    let input = har(100_000);
    let started = Instant::now();
    let bundles = parse(input.as_bytes()).expect("parses");
    assert!(
        started.elapsed() < baseline * 40,
        "took {:?} against {baseline:?} for a tenth of the entries",
        started.elapsed()
    );
    let ImportedItem::Folder(f) = &bundles[0].collection.items[0] else {
        panic!("folder")
    };
    assert_eq!(f.items.len(), 100_000);
    let ImportedItem::Request(last) = &f.items[99_999] else {
        panic!("request")
    };
    assert_eq!(
        last.request.auth,
        AuthSpec::Bearer {
            token: "{{h_test_token_100000}}".into()
        }
    );
    assert_eq!(bundles[0].environments[0].variables.len(), 100_000);
    let big = "a".repeat(1 << 20);
    let input = format!(
        r#"{{"log":{{"entries":[{{"request":{{"method":"GET","url":"https://h.test/","headers":[{{"name":"X-Big","value":"{big}"}}]}}}}]}}}}"#
    );
    let bundles = parse(input.as_bytes()).expect("parses");
    let ImportedItem::Folder(f) = &bundles[0].collection.items[0] else {
        panic!("folder")
    };
    let ImportedItem::Request(r) = &f.items[0] else {
        panic!("request")
    };
    assert_eq!(r.request.headers[0].value.len(), 1 << 20);
}

#[test]
fn no_text_names_another_tool() {
    let mut texts = Vec::new();
    let mut inputs: Vec<Vec<u8>> = fixture_names()
        .iter()
        .map(|n| fs::read(fixtures().join(n)).expect("read"))
        .collect();
    inputs.push(CHROME.as_bytes().to_vec());
    for extra in [
        r#"{"log":{"version":"2.0","entries":[]}}"#,
        r#"{"log":{}}"#,
        "{",
    ] {
        inputs.push(extra.as_bytes().to_vec());
    }
    for input in inputs {
        match parse(&input) {
            Ok(bundles) => {
                let (_, reports, _) = write(&bundles);
                texts.extend(
                    reports
                        .iter()
                        .flat_map(|r| r.notes.iter().map(|n| n.detail.clone())),
                );
                texts.extend(bundles.iter().map(|b| b.collection.name.clone()));
            }
            Err(e) => texts.push(e.to_string()),
        }
    }
    assert!(texts.len() > 10);
    for t in texts {
        assert!(!t.to_lowercase().contains("bruno"), "{t}");
    }
}
