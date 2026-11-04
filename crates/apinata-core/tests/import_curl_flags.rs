//! `parse_curl` maps digest, bearer and AWS v4 auth, `--max-time` and `--max-redirs`, and no
//! curl flag is dropped without a note.

use apinata_core::import::{ImportedRequest, NoteKind, ParsedCurl, add_request, parse_curl};
use apinata_core::{AuthSpec, Collection, ContextOptions, ItemPath, Timeout, VarContext};

use crate::common;
use std::fs;
use tempfile::TempDir;

const URL: &str = "https://x.test/a";

fn parse(command: &str) -> ParsedCurl {
    parse_curl(command).unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn notes(parsed: &ParsedCurl) -> Vec<(String, NoteKind)> {
    parsed
        .notes
        .iter()
        .map(|n| (n.source.clone(), n.kind))
        .collect()
}

fn note(source: &str, kind: NoteKind) -> (String, NoteKind) {
    (source.to_owned(), kind)
}

fn digest(username: &str, password: &str) -> AuthSpec {
    AuthSpec::Digest {
        username: username.into(),
        password: password.into(),
    }
}

fn aws(access: &str, region: &str, service: &str) -> AuthSpec {
    AuthSpec::AwsV4 {
        access_key_id: access.into(),
        secret_access_key: "{{awsSecretAccessKey}}".into(),
        session_token: String::new(),
        service: service.into(),
        region: region.into(),
        profile_name: String::new(),
    }
}

#[test]
fn digest_with_a_user_is_digest_auth() {
    let parsed = parse(&format!("curl --digest -u a:b {URL}"));
    assert_eq!(parsed.request.auth, digest("a", "b"));
    assert_eq!(notes(&parsed), []);
    let parsed = parse(&format!("curl -u a:b:c --digest {URL}"));
    assert_eq!(parsed.request.auth, digest("a", "b:c"));
}

#[test]
fn digest_without_a_password_asks_for_one() {
    let parsed = parse(&format!("curl --digest --user a {URL}"));
    assert_eq!(parsed.request.auth, digest("a", ""));
    assert_eq!(
        notes(&parsed),
        [note("curl flag --user", NoteKind::NeedsInput)]
    );
}

#[test]
fn digest_without_a_user_carries_nothing_and_says_so() {
    let parsed = parse(&format!("curl --digest {URL}"));
    assert_eq!(parsed.request.auth, AuthSpec::Inherit);
    assert_eq!(
        notes(&parsed),
        [note("curl flag --digest", NoteKind::Dropped)]
    );
}

#[test]
fn no_digest_turns_digest_off_so_the_user_is_basic() {
    let parsed = parse(&format!("curl --digest --no-digest -u a:b {URL}"));
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Basic {
            username: "a".into(),
            password: "b".into()
        }
    );
    assert_eq!(notes(&parsed), []);
}

#[test]
fn a_user_alone_stays_basic() {
    let parsed = parse(&format!("curl -u a:b {URL}"));
    assert!(matches!(parsed.request.auth, AuthSpec::Basic { .. }));
}

#[test]
fn bearer_token_is_bearer_auth_and_the_last_flag_wins() {
    let parsed = parse(&format!("curl --oauth2-bearer tok {URL}"));
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Bearer {
            token: "tok".into()
        }
    );
    assert_eq!(notes(&parsed), []);
    let parsed = parse(&format!(
        "curl --oauth2-bearer one --oauth2-bearer two {URL}"
    ));
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Bearer {
            token: "two".into()
        }
    );
    let parsed = parse(&format!("curl --oauth2-bearer=tok {URL}"));
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Bearer {
            token: "tok".into()
        }
    );
}

#[test]
fn bearer_wins_over_a_user_and_the_user_is_noted() {
    let parsed = parse(&format!("curl -u a:pw --oauth2-bearer tok {URL}"));
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Bearer {
            token: "tok".into()
        }
    );
    assert_eq!(notes(&parsed), [note("curl flag -u", NoteKind::Dropped)]);
    assert!(parsed.notes.iter().all(|n| !n.detail.contains("pw")));
}

#[test]
fn an_empty_bearer_token_is_noted_and_sends_nothing() {
    let parsed = parse(&format!("curl --oauth2-bearer '' {URL}"));
    assert_eq!(parsed.request.auth, AuthSpec::Inherit);
    assert_eq!(
        notes(&parsed),
        [note("curl flag --oauth2-bearer", NoteKind::Dropped)]
    );
}

#[test]
fn the_bearer_token_never_appears_in_a_note() {
    let parsed = parse(&format!("curl -u a:b --oauth2-bearer s3cret-token {URL}"));
    assert!(
        parsed
            .notes
            .iter()
            .all(|n| !n.detail.contains("s3cret-token") && !n.source.contains("s3cret-token"))
    );
}

#[test]
fn aws_sigv4_with_a_user_is_aws_auth_and_the_secret_is_a_variable() {
    let parsed = parse(&format!(
        "curl --aws-sigv4 aws:amz:us-east-1:service --user AKID:TOPSECRET {URL}"
    ));
    assert_eq!(parsed.request.auth, aws("AKID", "us-east-1", "service"));
    assert_eq!(
        notes(&parsed),
        [note("curl flag --aws-sigv4", NoteKind::NeedsInput)]
    );
    let detail = &parsed.notes[0].detail;
    assert!(detail.contains("awsSecretAccessKey"), "{detail}");
    assert!(!format!("{parsed:?}").contains("TOPSECRET"));
}

#[test]
fn aws_sigv4_leaves_out_what_the_string_omits() {
    let parsed = parse(&format!("curl --aws-sigv4 aws:amz -u AKID:S {URL}"));
    assert_eq!(parsed.request.auth, aws("AKID", "", ""));
    let parsed = parse(&format!(
        "curl --aws-sigv4 aws:amz:eu-west-1 -u AKID:S {URL}"
    ));
    assert_eq!(parsed.request.auth, aws("AKID", "eu-west-1", ""));
    let parsed = parse(&format!("curl --aws-sigv4=aws:amz::s3 -u AKID:S {URL}"));
    assert_eq!(parsed.request.auth, aws("AKID", "", "s3"));
}

#[test]
fn aws_sigv4_consumes_the_user_instead_of_basic_auth() {
    let parsed = parse(&format!("curl -u AKID:S --aws-sigv4 aws:amz:r:s {URL}"));
    assert!(matches!(parsed.request.auth, AuthSpec::AwsV4 { .. }));
    assert_eq!(parsed.notes.len(), 1);
}

#[test]
fn aws_sigv4_without_a_user_is_noted_and_sends_nothing() {
    let parsed = parse(&format!("curl --aws-sigv4 aws:amz:r:s {URL}"));
    assert_eq!(parsed.request.auth, AuthSpec::Inherit);
    assert_eq!(
        notes(&parsed),
        [note("curl flag --aws-sigv4", NoteKind::Dropped)]
    );
}

#[test]
fn aws_sigv4_with_a_user_but_no_secret_still_references_the_variable() {
    let parsed = parse(&format!("curl --aws-sigv4 aws:amz:r:s --user AKID {URL}"));
    assert_eq!(parsed.request.auth, aws("AKID", "r", "s"));
    assert_eq!(
        notes(&parsed),
        [note("curl flag --aws-sigv4", NoteKind::NeedsInput)]
    );
}

#[test]
fn a_malformed_provider_string_is_noted_and_the_user_is_not_sent_as_basic() {
    for value in ["", ":amz:r:s", "aws:amz:r:s:extra"] {
        let parsed = parse(&format!("curl --aws-sigv4 '{value}' -u AKID:S {URL}"));
        assert_eq!(parsed.request.auth, AuthSpec::Inherit, "{value}");
        assert_eq!(
            notes(&parsed),
            [note("curl flag --aws-sigv4", NoteKind::Dropped)],
            "{value}"
        );
    }
}

#[test]
fn other_provider_names_are_mapped_with_a_conversion_note() {
    let parsed = parse(&format!("curl --aws-sigv4 foo:bar:r:s -u AKID:S {URL}"));
    assert_eq!(parsed.request.auth, aws("AKID", "r", "s"));
    let kinds: Vec<NoteKind> = parsed.notes.iter().map(|n| n.kind).collect();
    assert_eq!(kinds, [NoteKind::Converted, NoteKind::NeedsInput]);
}

fn timeout(command: &str) -> Timeout {
    parse(command).request.settings.expect("settings").timeout
}

#[test]
fn max_time_sets_the_timeout_in_milliseconds() {
    assert_eq!(timeout(&format!("curl -m 5 {URL}")), Timeout::Millis(5000));
    assert_eq!(
        timeout(&format!("curl --max-time 2.5 {URL}")),
        Timeout::Millis(2500)
    );
    assert_eq!(
        timeout(&format!("curl --max-time=3 {URL}")),
        Timeout::Millis(3000)
    );
    assert_eq!(
        timeout(&format!("curl -m 1 -m 2 {URL}")),
        Timeout::Millis(2000)
    );
    let parsed = parse(&format!("curl -m 5 {URL}"));
    assert_eq!(notes(&parsed), []);
}

#[test]
fn max_time_zero_means_no_limit() {
    assert_eq!(timeout(&format!("curl -m 0 {URL}")), Timeout::Unlimited);
    assert_eq!(timeout(&format!("curl -m 0.0 {URL}")), Timeout::Unlimited);
}

#[test]
fn max_time_below_a_millisecond_rounds_up_with_a_note() {
    let command = format!("curl --max-time 1.2345 {URL}");
    assert_eq!(timeout(&command), Timeout::Millis(1235));
    assert_eq!(
        notes(&parse(&command)),
        [note("curl flag --max-time", NoteKind::Converted)]
    );
    assert_eq!(
        timeout(&format!("curl -m 0.0005 {URL}")),
        Timeout::Millis(1)
    );
}

#[test]
fn an_unreadable_max_time_is_noted_and_keeps_the_default() {
    for value in ["abc", "-3", "1e3", "", "99999999999999999999"] {
        let command = format!("curl --max-time '{value}' {URL}");
        assert_eq!(timeout(&command), Timeout::Inherit, "{value}");
        assert_eq!(
            notes(&parse(&command)),
            [note("curl flag --max-time", NoteKind::Dropped)],
            "{value}"
        );
    }
}

fn redirects(command: &str) -> u32 {
    parse(command)
        .request
        .settings
        .expect("settings")
        .max_redirects
}

#[test]
fn max_redirs_sets_the_redirect_limit() {
    assert_eq!(redirects(&format!("curl --max-redirs 3 {URL}")), 3);
    assert_eq!(redirects(&format!("curl --max-redirs 0 {URL}")), 0);
    assert_eq!(
        redirects(&format!("curl --max-redirs 2 --max-redirs 7 {URL}")),
        7
    );
    assert_eq!(notes(&parse(&format!("curl --max-redirs 3 {URL}"))), []);
    assert_eq!(redirects(&format!("curl {URL}")), 5);
}

#[test]
fn unlimited_redirects_map_to_the_largest_limit_with_a_note() {
    for value in ["-1", "99999999999"] {
        let command = format!("curl --max-redirs={value} {URL}");
        assert_eq!(redirects(&command), u32::MAX, "{value}");
        assert_eq!(
            notes(&parse(&command)),
            [note("curl flag --max-redirs", NoteKind::Converted)],
            "{value}"
        );
    }
}

#[test]
fn an_unreadable_max_redirs_is_noted_and_keeps_the_default() {
    for value in ["abc", "-2", "1.5", ""] {
        let command = format!("curl --max-redirs '{value}' {URL}");
        assert_eq!(redirects(&command), 5, "{value}");
        assert_eq!(
            notes(&parse(&command)),
            [note("curl flag --max-redirs", NoteKind::Dropped)],
            "{value}"
        );
    }
}

#[test]
fn the_combined_command_from_the_issue_maps_everything() {
    let parsed = parse(&format!(
        "curl --oauth2-bearer tok --max-time 5 --max-redirs 3 {URL}"
    ));
    let settings = parsed.request.settings.expect("settings");
    assert_eq!(
        parsed.request.auth,
        AuthSpec::Bearer {
            token: "tok".into()
        }
    );
    assert_eq!(settings.timeout, Timeout::Millis(5000));
    assert_eq!(settings.max_redirects, 3);
    assert_eq!(notes(&parsed), []);
}

// A value each option accepts, so the sweep exercises its mapping rather than its error path.
fn sample_value(long: &str) -> &'static str {
    match long {
        "aws-sigv4" => "aws:amz:r:s",
        "max-time" => "5",
        "max-redirs" => "3",
        "oauth2-bearer" => "tok",
        "user" => "a:b",
        "header" => "X-A: 1",
        "request" => "PUT",
        "url" => "https://y.test/",
        "cookie" => "a=b",
        "json" | "data" | "data-ascii" | "data-raw" | "data-binary" | "data-urlencode" | "form" => {
            "a=1"
        }
        _ => "v",
    }
}

// Flags that only act together with another one are tried next to it.
fn companion(long: &str) -> &'static str {
    match long {
        "get" => "-d a=1",
        "digest" | "ntlm" | "aws-sigv4" => "-u a:b",
        _ => "",
    }
}

fn shape(request: &ImportedRequest) -> String {
    format!("{request:?}")
}

#[test]
fn every_curl_option_is_mapped_or_noted() {
    let table = include_str!("../src/import/curl/curl-options-8.22.0.txt");
    let mut checked = 0;
    for line in table.lines() {
        let mut parts = line.split(' ');
        let short = parts.next().and_then(|s| s.strip_prefix('-')).unwrap_or("");
        let long = parts
            .next()
            .and_then(|l| l.strip_prefix("--"))
            .expect("long");
        let takes_value = parts.next() == Some("value");
        let mut forms = vec![format!("--{long}")];
        if !short.is_empty() {
            forms.push(format!("-{short}"));
        }
        let with = companion(long);
        let baseline = parse(&format!("curl {with} {URL}"));
        for flag in forms {
            let value = if takes_value { sample_value(long) } else { "" };
            let command = format!("curl {with} {flag} {value} {URL}");
            let parsed = parse(&command);
            let mapped = shape(&parsed.request) != shape(&baseline.request);
            assert!(
                mapped || !parsed.notes.is_empty(),
                "{command}: neither mapped nor noted"
            );
            checked += 1;
        }
    }
    assert!(checked >= 274, "{checked}");
}

#[test]
fn imported_auth_and_settings_save_byte_stable() {
    for command in [
        format!("curl --digest -u a:b --max-time 2.5 --max-redirs 3 {URL}"),
        format!("curl --oauth2-bearer tok -m 0 --max-redirs -1 {URL}"),
        format!("curl --aws-sigv4 aws:amz:us-east-1:service --user AKID:TOPSECRET {URL}"),
    ] {
        let dir = TempDir::new().expect("tempdir");
        let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
        let parsed = parse(&command);
        let path = add_request(&mut c, &ItemPath::root(), "R", &parsed.request).expect("adds");
        let file = c.root().join("R.yml");
        let first = fs::read(&file).expect("reads");
        assert!(
            !String::from_utf8_lossy(&first).contains("TOPSECRET"),
            "{command}"
        );
        let mut reopened = Collection::open(c.root()).expect("reopens");
        let request = reopened.request_mut(&path).expect("loaded");
        let settings = request.settings();
        request.set_settings(settings);
        let auth = request.auth();
        request.set_auth(auth);
        request.save().expect("saves");
        assert_eq!(fs::read(&file).expect("reads"), first, "{command}");
    }
}

fn follows(command: &str) -> (bool, u32) {
    let settings = parse(command).request.settings.expect("settings");
    (settings.follow_redirects, settings.max_redirects)
}

#[test]
fn redirects_are_followed_only_with_location() {
    assert!(!follows(&format!("curl {URL}")).0);
    assert!(follows(&format!("curl -L {URL}")).0);
    assert!(follows(&format!("curl --location {URL}")).0);
    assert!(!follows(&format!("curl -L --no-location {URL}")).0);
    assert!(follows(&format!("curl --no-location -L {URL}")).0);
    assert_eq!(notes(&parse(&format!("curl -L {URL}"))), []);
    assert_eq!(follows(&format!("curl -L --max-redirs 3 {URL}")), (true, 3));
    assert_eq!(follows(&format!("curl --max-redirs 3 {URL}")), (false, 3));
}

#[test]
fn a_request_without_location_saves_byte_stable_and_keeps_its_choice() {
    for (command, follow) in [
        (format!("curl {URL}"), false),
        (format!("curl -L {URL}"), true),
    ] {
        let dir = TempDir::new().expect("tempdir");
        let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
        let parsed = parse(&command);
        let path = add_request(&mut c, &ItemPath::root(), "R", &parsed.request).expect("adds");
        let file = c.root().join("R.yml");
        let first = fs::read(&file).expect("reads");
        let mut reopened = Collection::open(c.root()).expect("reopens");
        let request = reopened.request_mut(&path).expect("loaded");
        let settings = request.settings();
        assert_eq!(settings.follow_redirects, follow, "{command}");
        request.set_settings(settings);
        request.save().expect("saves");
        assert_eq!(fs::read(&file).expect("reads"), first, "{command}");
    }
}

mod wire {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn status_of(command: &str) -> (u16, String) {
        let dir = TempDir::new().expect("tempdir");
        let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
        add_request(&mut c, &ItemPath::root(), "R", &parse(command).request).expect("adds");
        let c = Collection::open(c.root()).expect("reopens");
        let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
        let resolved = common::at(&c, &ctx, "R.yml").expect("resolves");
        let res = common::executor().send(&resolved).await.expect("sent");
        (res.status(), res.text().to_owned())
    }

    #[tokio::test]
    async fn the_import_redirects_exactly_when_curl_would() {
        let server = MockServer::start().await;
        Mock::given(path("/from"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/to"))
            .mount(&server)
            .await;
        Mock::given(path("/to"))
            .respond_with(ResponseTemplate::new(200).set_body_string("target"))
            .mount(&server)
            .await;
        let url = format!("{}/from", server.uri());
        let (status, _) = status_of(&format!("curl '{url}'")).await;
        assert_eq!(status, 302);
        let (status, body) = status_of(&format!("curl -L '{url}'")).await;
        assert_eq!((status, body.as_str()), (200, "target"));
    }
}

mod secrets {
    use super::*;
    use apinata_core::import::store_request_secrets;
    use apinata_core::{MemoryStore, SecretError, SecretKey, SecretStore};

    const AWS: &str =
        "curl --aws-sigv4 aws:amz:us-east-1:service --user AKID:TOPSECRET https://x.test/a";

    struct Locked;
    impl SecretStore for Locked {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Unavailable {
                message: "locked".into(),
            })
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }

    fn collection(envs: &[&str]) -> (TempDir, Collection) {
        let dir = TempDir::new().expect("tempdir");
        let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
        for env in envs {
            c.create_environment(env, &MemoryStore::new()).expect("env");
        }
        (dir, c)
    }

    #[test]
    fn the_secret_rides_on_the_request_and_never_shows_in_debug_or_notes() {
        let parsed = parse(AWS);
        let secrets = &parsed.request.secrets;
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].name, "awsSecretAccessKey");
        assert_eq!(secrets[0].value, "TOPSECRET");
        assert!(!format!("{parsed:?}").contains("TOPSECRET"));
        assert!(parsed.notes.iter().all(|n| !n.detail.contains("TOPSECRET")));
    }

    #[test]
    fn no_password_means_no_secret() {
        let parsed = parse("curl --aws-sigv4 aws:amz:r:s --user AKID https://x.test/a");
        assert!(parsed.request.secrets.is_empty());
        assert!(
            parse("curl -u a:b https://x.test/a")
                .request
                .secrets
                .is_empty()
        );
    }

    #[test]
    fn the_only_environment_gets_a_secret_variable_and_the_store_the_value() {
        let (_dir, mut c) = collection(&["dev"]);
        let store = MemoryStore::new();
        let mut parsed = parse(AWS);
        store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
        let key = c.secret_key("dev", "awsSecretAccessKey");
        assert_eq!(store.get(&key).expect("get").as_deref(), Some("TOPSECRET"));
        let reopened = Collection::open(c.root()).expect("reopens");
        let vars = reopened.environment("dev").expect("env").variables();
        let var = vars
            .iter()
            .find(|v| v.name == "awsSecretAccessKey")
            .expect("var");
        assert!(var.secret);
        let text = fs::read_to_string(c.root().join("environments/dev.yml")).expect("reads");
        assert!(!text.contains("TOPSECRET"), "{text}");
        // The stored secret replaces the note that asked the user to supply one.
        let kinds: Vec<NoteKind> = parsed.notes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NoteKind::Converted]);
        assert!(parsed.notes[0].detail.contains("dev"));
        assert!(!parsed.notes[0].detail.contains("TOPSECRET"));
    }

    #[test]
    fn the_request_file_and_the_environment_file_stay_byte_stable() {
        let (_dir, mut c) = collection(&["dev"]);
        let mut parsed = parse(AWS);
        let path = add_request(&mut c, &ItemPath::root(), "R", &parsed.request).expect("adds");
        store_request_secrets(
            &mut c,
            &parsed.request,
            None,
            &MemoryStore::new(),
            &mut parsed.notes,
        );
        let files = [
            c.root().join("R.yml"),
            c.root().join("environments/dev.yml"),
        ];
        let before: Vec<Vec<u8>> = files.iter().map(|f| fs::read(f).expect("reads")).collect();
        assert!(!String::from_utf8_lossy(&before[0]).contains("TOPSECRET"));
        let mut reopened = Collection::open(c.root()).expect("reopens");
        reopened
            .request_mut(&path)
            .expect("loaded")
            .save()
            .expect("saves");
        let env = reopened.environment_mut("dev").expect("env");
        let vars = env.variables();
        env.set_variables(vars);
        env.save().expect("saves");
        let after: Vec<Vec<u8>> = files.iter().map(|f| fs::read(f).expect("reads")).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn several_environments_need_a_choice_and_none_keeps_the_note() {
        let (_dir, mut c) = collection(&["dev", "prod"]);
        let store = MemoryStore::new();
        let mut parsed = parse(AWS);
        store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
        assert_eq!(
            parsed.notes.iter().map(|n| n.kind).collect::<Vec<_>>(),
            [NoteKind::NeedsInput]
        );
        assert!(
            store
                .get(&c.secret_key("dev", "awsSecretAccessKey"))
                .expect("get")
                .is_none()
        );
        let mut parsed = parse(AWS);
        store_request_secrets(
            &mut c,
            &parsed.request,
            Some("prod"),
            &store,
            &mut parsed.notes,
        );
        let key = c.secret_key("prod", "awsSecretAccessKey");
        assert_eq!(store.get(&key).expect("get").as_deref(), Some("TOPSECRET"));
    }

    /// Runs the import of `AWS` naming `prefer`, returning the store and the notes.
    fn store_naming(
        envs: &[&str],
        prefer: &str,
    ) -> (
        TempDir,
        Collection,
        MemoryStore,
        Vec<apinata_core::import::ImportNote>,
    ) {
        let (dir, mut c) = collection(envs);
        let store = MemoryStore::new();
        let mut parsed = parse(AWS);
        store_request_secrets(
            &mut c,
            &parsed.request,
            Some(prefer),
            &store,
            &mut parsed.notes,
        );
        (dir, c, store, parsed.notes)
    }

    fn assert_unknown_stores_nothing(envs: &[&str], expected: &str) {
        let (_dir, c, store, notes) = store_naming(envs, "nope");
        assert_eq!(
            notes.iter().map(|n| n.kind).collect::<Vec<_>>(),
            [NoteKind::NeedsInput]
        );
        let detail = &notes[0].detail;
        assert_eq!(
            detail,
            &format!(
                "the secret access key is not written to the file; define a secret variable named `awsSecretAccessKey` ({expected})"
            )
        );
        for env in envs {
            assert!(
                store
                    .get(&c.secret_key(env, "awsSecretAccessKey"))
                    .expect("get")
                    .is_none()
            );
            let text = fs::read_to_string(c.root().join(format!("environments/{env}.yml")))
                .expect("reads");
            assert!(!text.contains("awsSecretAccessKey"), "{text}");
        }
    }

    #[test]
    fn an_unknown_named_environment_does_not_fall_back_to_the_only_one() {
        assert_unknown_stores_nothing(&["dev"], "environment \"nope\" not found; available: dev");
    }

    #[test]
    fn an_unknown_named_environment_among_several_stores_nothing() {
        assert_unknown_stores_nothing(
            &["dev", "prod"],
            "environment \"nope\" not found; available: dev, prod",
        );
    }

    #[test]
    fn an_unknown_named_environment_with_none_lists_none() {
        assert_unknown_stores_nothing(
            &[],
            "environment \"nope\" not found; this collection has no environments",
        );
    }

    #[test]
    fn a_named_environment_that_exists_among_several_receives_the_secret() {
        let (_dir, c, store, notes) = store_naming(&["dev", "prod"], "prod");
        let key = c.secret_key("prod", "awsSecretAccessKey");
        assert_eq!(store.get(&key).expect("get").as_deref(), Some("TOPSECRET"));
        assert_eq!(notes[0].kind, NoteKind::Converted);
    }

    #[test]
    fn a_collection_without_environments_keeps_the_note() {
        let (_dir, mut c) = collection(&[]);
        let mut parsed = parse(AWS);
        store_request_secrets(
            &mut c,
            &parsed.request,
            None,
            &MemoryStore::new(),
            &mut parsed.notes,
        );
        assert_eq!(
            parsed.notes.iter().map(|n| n.kind).collect::<Vec<_>>(),
            [NoteKind::NeedsInput]
        );
    }

    #[test]
    fn a_refusing_store_keeps_the_note_and_says_why_without_the_value() {
        let (_dir, mut c) = collection(&["dev"]);
        let mut parsed = parse(AWS);
        store_request_secrets(&mut c, &parsed.request, None, &Locked, &mut parsed.notes);
        assert_eq!(
            parsed.notes.iter().map(|n| n.kind).collect::<Vec<_>>(),
            [NoteKind::NeedsInput]
        );
        let detail = &parsed.notes[0].detail;
        assert!(
            detail.contains("locked") && !detail.contains("TOPSECRET"),
            "{detail}"
        );
    }

    /// Make the environments directory unwritable; `None` when this user can write anyway.
    #[cfg(unix)]
    fn lock_environments(c: &Collection) -> Option<crate::common::ModeGuard> {
        let dir = c.root().join("environments");
        let guard = crate::common::ModeGuard::set(&dir, 0o555);
        if fs::write(dir.join(".probe"), b"").is_ok() {
            let _ = fs::remove_file(dir.join(".probe"));
            return None;
        }
        Some(guard)
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_environment_save_leaves_memory_as_it_was_and_the_store_untouched() {
        let (_dir, mut c) = collection(&["dev"]);
        let Some(_guard) = lock_environments(&c) else {
            return;
        };
        let before = c.environment("dev").expect("env").variables();
        let store = MemoryStore::new();
        let mut parsed = parse(AWS);
        store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
        assert_eq!(c.environment("dev").expect("env").variables(), before);
        assert!(
            store
                .get(&c.secret_key("dev", "awsSecretAccessKey"))
                .expect("get")
                .is_none()
        );
        assert_eq!(
            parsed.notes.iter().map(|n| n.kind).collect::<Vec<_>>(),
            [NoteKind::NeedsInput]
        );
        assert!(!parsed.notes[0].detail.contains("TOPSECRET"));
    }

    #[test]
    fn a_refusing_store_leaves_the_variable_declared_without_a_value_in_memory_and_on_disk() {
        let (_dir, mut c) = collection(&["dev"]);
        let mut parsed = parse(AWS);
        store_request_secrets(&mut c, &parsed.request, None, &Locked, &mut parsed.notes);
        let in_memory = c.environment("dev").expect("env").variables();
        assert_eq!(in_memory.len(), 1);
        assert!(in_memory[0].secret && in_memory[0].name == "awsSecretAccessKey");
        let reopened = Collection::open(c.root()).expect("reopens");
        assert_eq!(
            reopened.environment("dev").expect("env").variables(),
            in_memory
        );
    }
}
