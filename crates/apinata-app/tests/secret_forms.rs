//! Transformed echoes of a secret (lowercase, percent, `\u` escapes, base64) never reach a
//! saved example or the response history.

use crate::support;

use std::sync::Arc;

use apinata_app::last_response::ResponseStore;
use apinata_app::scrub::{scrub_text, scrub_url};
use apinata_core::encoded_forms;
use apinata_core::{Collection, MemoryStore, SecretStore};
use base64::Engine as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const TOKEN: &str = "Sk Live/9é😀Q";

/// Every encoded form of `TOKEN`, written out by hand so the test does not trust the code it checks.
const FORMS: [&str; 17] = [
    r"\u0053\u006B\u0020\u004C\u0069\u0076\u0065\u002F\u0039\u00E9\uD83D\uDE00\u0051",
    r"\u0053\u006b\u0020\u004c\u0069\u0076\u0065\u002f\u0039\u00e9\ud83d\ude00\u0051",
    "Sk%20Live%2F9%C3%A9%F0%9F%98%80Q",
    "Sk%20Live%2f9%c3%a9%f0%9f%98%80Q",
    "Sk%20Live/9%C3%A9%F0%9F%98%80Q",
    "Sk%20Live/9%c3%a9%f0%9f%98%80Q",
    "Sk+Live%2F9%C3%A9%F0%9F%98%80Q",
    "Sk+Live%2f9%c3%a9%f0%9f%98%80Q",
    r"Sk Live/9\u00E9\uD83D\uDE00Q",
    r"Sk Live/9\u00e9\ud83d\ude00Q",
    "U2sgTGl2ZS85w6nwn5iAUQ==",
    "U2sgTGl2ZS85w6nwn5iAUQ",
    "TayBMaXZlLznDqfCfmIBR",
    "U2sgTGl2ZS85w6nwn5iAU",
    "NrIExpdmUvOcOp8J+YgF",
    "NrIExpdmUvOcOp8J-YgF",
    "sk live/9é😀q",
];

/// Base64 of `u:`, `us:`, `use:` and `me:` (URL-safe) followed by `TOKEN`.
const PAIRS: [&str; 4] = [
    "dTpTayBMaXZlLznDqfCfmIBR",
    "dXM6U2sgTGl2ZS85w6nwn5iAUQ==",
    "dXNlOlNrIExpdmUvOcOp8J+YgFE=",
    "bWU6U2sgTGl2ZS85w6nwn5iAUQ==",
];

/// Survives scrubbing, proving non-ASCII text is kept as written.
const CANARY: &str = "keep-é😀-line";

/// The forms and pairs found in `text`.
fn leaks(text: &str) -> Vec<&'static str> {
    FORMS
        .iter()
        .chain(&PAIRS)
        .copied()
        .filter(|f| text.contains(f))
        .collect()
}

/// A 127.0.0.1 server answering every request with each form on its own line, and the
/// padded base64 form in an `x-echo` header.
fn echo_server(cx: &mut TestAppContext) -> wiremock::MockServer {
    let body: Vec<&str> = std::iter::once(CANARY).chain(FORMS).chain(PAIRS).collect();
    mock_server(
        cx,
        ResponseTemplate::new(200)
            .insert_header("x-echo", FORMS[10])
            .set_body_raw(body.join("\n"), "text/plain"),
    )
}

/// The app over a collection whose `a.yml` posts `{{token}}` (a `dev` secret holding
/// `TOKEN`) to `server`, with `dev` selected and `a.yml` sent once from its own tab.
fn sent(cx: &mut TestAppContext) -> (tempfile::TempDir, Started, wiremock::MockServer) {
    http_ready(cx);
    let server = echo_server(cx);
    let a = format!(
        "{}  body:\n    type: text\n    data: \"{{{{token}}}}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = Collection::open(col.path()).unwrap();
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), TOKEN).unwrap();
    let s = started_with_store(cx, &col, Arc::new(store));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let root = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    let live = result.response.as_ref().expect("a response");
    assert!(
        live.text().contains(FORMS[0]),
        "the live response shows the echo"
    );
    frame(cx, s.win);
    (col, s, server)
}

#[gpui_kit::test]
fn a_saved_example_keeps_no_encoded_form_of_a_secret(cx: &mut TestAppContext) {
    let (col, s, _server) = sent(cx);
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-example-dialog"));
    // Only encoded forms came back, so finding them is what names the secret.
    assert!(present(cx, s.win, "example-secrets"));
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    let file = col.path().join("a.yml");
    wait_for(cx, "the example", |_| {
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("examples:")
    });
    // Read back through the parser, so YAML escaping cannot hide a leak.
    let c = Collection::open(col.path()).unwrap();
    let examples = c.tree().requests().first().expect("a.yml").examples();
    let response = examples[0].response.clone().expect("a response half");
    let mut saved = response.body.expect("a body").data;
    for (_, value) in &response.headers {
        saved.push('\n');
        saved.push_str(value);
    }
    assert!(saved.contains(CANARY), "{saved}");
    assert!(saved.contains("[redacted]"), "{saved}");
    assert!(leaks(&saved).is_empty(), "{:?} in {saved}", leaks(&saved));
}

#[gpui_kit::test]
fn history_keeps_no_encoded_form_of_a_secret(cx: &mut TestAppContext) {
    let (_col, s, _server) = sent(cx);
    let store = ResponseStore::new(s.files.path().join("responses"));
    wait_real(cx, |_| store.load(&s.root, &ip("a.yml")).is_some());
    let kept = store.load(&s.root, &ip("a.yml")).unwrap();
    let response = kept.result.response.as_ref().expect("a kept response");
    let mut text = response.text().to_owned();
    for (_, value) in response.headers() {
        text.push('\n');
        text.push_str(value);
    }
    assert!(text.contains(CANARY), "{text}");
    assert!(leaks(&text).is_empty(), "{:?} in {text}", leaks(&text));
    // On disk too, for every form JSON writes unchanged (those without a backslash).
    for entry in std::fs::read_dir(store.dir()).unwrap() {
        let path = entry.unwrap().path();
        let file = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        let on_disk: Vec<&str> = leaks(&file)
            .into_iter()
            .filter(|f| !f.contains('\\'))
            .collect();
        assert!(on_disk.is_empty(), "{on_disk:?} in {}", path.display());
    }
}

fn list(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn text_is_scrubbed_in_lowercase_percent_escape_and_base64_forms() {
    let s = list(&["Key/a b~*é"]);
    assert_eq!(
        scrub_text(r"a Key%2fa+b%7e*%c3%a9 b Key/a b~*\u00e9 c", &s),
        "a [redacted] b [redacted] c"
    );
    assert_eq!(
        scrub_text("Basic dTpLZXkvYSBifirDqQ==", &s),
        "Basic d[redacted]"
    );
}

#[test]
fn a_host_the_parser_lowercased_is_scrubbed() {
    let parsed = url::Url::parse("https://Tenant-Q7x.example.test/p?x=1").unwrap();
    assert_eq!(parsed.as_str(), "https://tenant-q7x.example.test/p?x=1");
    assert_eq!(
        scrub_url(parsed.as_str(), &list(&["Tenant-Q7x"])),
        "https://[redacted].example.test/p?x=1"
    );
}

#[test]
fn a_four_char_secret_leaves_unrelated_base64_alone() {
    let bytes: Vec<u8> = (0..768u32).map(|i| ((i * 7 + 3) % 256) as u8).collect();
    let noise = base64::engine::general_purpose::STANDARD.encode(bytes);
    // Sub-6-char fragments of `ab1Z` (YWIxW, QFiMV, hYjFa) sit apart inside; the floor ignores
    // them. Adjacent they would complete the bytes at some alignment, and at these offsets no
    // start of the run holds `ab1Z`, checked by an independent decoder.
    let body = format!(
        "{}YWIxW{}QFiMV{}hYjFa{}",
        &noise[..100],
        &noise[100..400],
        &noise[400..700],
        &noise[700..]
    );
    assert_eq!(body.len(), 1039);
    assert_eq!(scrub_text(&body, &list(&["ab1Z"])), body);
}

#[test]
fn twenty_secrets_over_a_megabyte_are_all_scrubbed() {
    let secrets: Vec<String> = (0..20).map(|i| format!("Bulk-Secret-{i:02}-Zq")).collect();
    let forms: Vec<Vec<String>> = secrets.iter().map(|s| encoded_forms(s)).collect();
    // Every form of every secret, round robin, between filler, until the body passes 1 MiB.
    let mut body = String::with_capacity((1 << 20) + 64);
    let mut i = 0;
    while body.len() < 1 << 20 {
        let own = &forms[i % 20];
        body.push_str(&own[(i / 20) % own.len()]);
        body.push_str(" filler-text-0123456789 ");
        i += 1;
    }
    let started = std::time::Instant::now();
    let out = scrub_text(&body, &secrets);
    eprintln!(
        "scrubbed 20 secrets over {} bytes in {:?}",
        body.len(),
        started.elapsed()
    );
    for (secret, own) in secrets.iter().zip(&forms) {
        assert!(!out.contains(secret.as_str()), "{secret}");
        for form in own {
            assert!(!out.contains(form.as_str()), "{form} of {secret}");
        }
    }
    assert!(out.contains("filler-text-0123456789"));
}
