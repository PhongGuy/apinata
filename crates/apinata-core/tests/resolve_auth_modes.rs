//! Digest and AWS SigV4 auth through resolve: inheritance, profiles, warnings and secrets.

use crate::common;

use apinata_core::{
    AuthSource, AuthSpec, AwsSharedFiles, Collection, ContextOptions, ItemPath, ResolveError,
    VarContext, Warning, named_secrets, resolve,
};
use common::{at, request, setup, write_tree};
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: user\n      value: alice\n";

fn http(url: &str, auth: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n{auth}"
    )
}

fn aws_block(
    ak: &str,
    sk: &str,
    token: &str,
    service: &str,
    region: &str,
    profile: &str,
) -> String {
    format!(
        "    type: awsv4\n    accessKeyId: \"{ak}\"\n    secretAccessKey: \"{sk}\"\n    sessionToken: \"{token}\"\n    service: \"{service}\"\n    region: \"{region}\"\n    profileName: \"{profile}\"\n"
    )
}

/// A collection whose context reads AWS profiles from `files` only.
fn with_profiles(
    files: &[(&str, &str)],
    aws: &AwsSharedFiles,
) -> (TempDir, Collection, VarContext) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx =
        VarContext::new(&c, ContextOptions::new().aws_shared_files(aws.clone())).expect("context");
    (dir, c, ctx)
}

fn aws_files(credentials: &str, config: &str) -> (TempDir, AwsSharedFiles) {
    let dir = write_tree(&[("credentials", credentials), ("config", config)]);
    let files = AwsSharedFiles::new(dir.path().join("credentials"), dir.path().join("config"));
    (dir, files)
}

#[test]
fn digest_and_awsv4_resolve_own_and_inherited() {
    let folder = "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n    type: digest\n    username: \"{{user}}\"\n    password: pw\n";
    let root = format!(
        "{ROOT}  auth:\n{}",
        aws_block("AKID", "SECRET", "", "execute-api", "eu-west-1", "")
    );
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", folder),
        (
            "f/r.yml",
            &http("https://x.test/", "    type: inherit\n")
                .replace("  auth:\n    type: inherit\n", "  auth: inherit\n"),
        ),
        (
            "own.yml",
            &http(
                "https://x.test/",
                "    type: digest\n    username: u\n    password: p\n",
            ),
        ),
        (
            "top.yml",
            &http("https://x.test/", "").replace("  auth:\n", "  auth: inherit\n"),
        ),
    ]);
    for rel in ["f/r.yml", "own.yml", "top.yml"] {
        let r = at(&c, &ctx, rel).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(r.warnings().is_empty(), "{rel}: {:?}", r.warnings());
    }
    let got = c.inherited_auth(&ItemPath::new("f/r.yml").unwrap());
    assert_eq!(
        got.auth(),
        &AuthSpec::Digest {
            username: "{{user}}".into(),
            password: "pw".into()
        }
    );
    assert_eq!(
        got.source(),
        &AuthSource::Folder(ItemPath::new("f").unwrap())
    );
    let got = c.inherited_auth(&ItemPath::new("top.yml").unwrap());
    assert!(matches!(got.auth(), AuthSpec::AwsV4 { .. }));
    assert_eq!(got.source(), &AuthSource::Collection);
}

#[test]
fn blank_keys_or_an_unknown_service_send_unsigned_with_a_warning() {
    let cases = [
        (
            "https://x.test/",
            aws_block("", "SECRET", "", "execute-api", "eu-west-1", ""),
            true,
        ),
        (
            "https://x.test/",
            aws_block("AKID", " ", "", "execute-api", "eu-west-1", ""),
            true,
        ),
        (
            "https://api.example.com/",
            aws_block("AKID", "SECRET", "", "", "", ""),
            true,
        ),
        (
            "https://dynamodb.us-west-2.amazonaws.com/",
            aws_block("AKID", "SECRET", "", "", "", ""),
            false,
        ),
    ];
    for (url, block, unsigned) in cases {
        let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &http(url, &block))]);
        let r = at(&c, &ctx, "r.yml").expect("resolves");
        assert_eq!(
            r.warnings().contains(&Warning::AwsV4Unsigned),
            unsigned,
            "{url} {block}"
        );
    }
}

#[test]
fn an_undefined_variable_in_a_key_fails() {
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &http(
                "https://x.test/",
                &aws_block("{{nope}}", "SECRET", "", "s", "r", ""),
            ),
        ),
    ]);
    assert!(
        matches!(at(&c, &ctx, "r.yml"), Err(ResolveError::UndefinedVariable { name, .. }) if name == "nope")
    );
}

#[test]
fn a_profile_replaces_the_keys_and_its_secrets_are_sensitive() {
    let (_a, files) = aws_files(
        "[dev]\naws_access_key_id = AKIAPROFILE\naws_secret_access_key = profile-secret-1\n",
        "# comment\n[profile dev]\naws_session_token = profile-token-2\nregion = ignored\n",
    );
    let (_d, c, ctx) = with_profiles(
        &[
            ("opencollection.yml", ROOT),
            (
                "r.yml",
                &http(
                    "https://x.test/",
                    &aws_block("", "", "field-token", "execute-api", "eu-west-1", "dev"),
                ),
            ),
        ],
        &files,
    );
    let req = request(&c, "r.yml");
    let r = resolve(req, &ctx).expect("resolves");
    assert!(r.warnings().is_empty(), "{:?}", r.warnings());
    let debug = format!("{r:?}");
    assert!(
        !debug.contains("profile-secret-1") && !debug.contains("profile-token-2"),
        "{debug}"
    );
    let secrets: Vec<String> = named_secrets(req, &ctx)
        .unwrap()
        .iter()
        .map(|s| s.value().to_owned())
        .collect();
    assert!(
        secrets.contains(&"profile-secret-1".to_owned()),
        "{secrets:?}"
    );
    assert!(
        secrets.contains(&"profile-token-2".to_owned()),
        "{secrets:?}"
    );
}

#[test]
fn a_missing_or_unsupported_profile_keeps_the_fields_and_warns() {
    let (_a, files) = aws_files(
        "[other]\naws_access_key_id = A\naws_secret_access_key = B\n",
        "[profile sso]\nsso_start_url = https://x\naws_access_key_id = A\naws_secret_access_key = B\n",
    );
    for profile in ["dev", "sso"] {
        let block = aws_block("AKID", "SECRET", "", "execute-api", "eu-west-1", profile);
        let (_d, c, ctx) = with_profiles(
            &[
                ("opencollection.yml", ROOT),
                ("r.yml", &http("https://x.test/", &block)),
            ],
            &files,
        );
        let r = at(&c, &ctx, "r.yml").expect("resolves");
        assert_eq!(r.warnings(), [Warning::AwsProfileUnavailable], "{profile}");
    }
}

#[test]
fn a_plain_session_token_is_named_as_an_auth_credential() {
    let block = aws_block(
        "AKID",
        "SECRET",
        "plain-session-token",
        "execute-api",
        "eu-west-1",
        "",
    );
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("https://x.test/", &block)),
    ]);
    let found = named_secrets(request(&c, "r.yml"), &ctx).unwrap();
    let token = found
        .iter()
        .find(|s| s.value() == "plain-session-token")
        .expect("named");
    assert_eq!(token.label(), "auth credentials");
}

#[test]
fn a_secret_variable_in_the_secret_key_stays_out_of_debug() {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
    // A secret variable's value comes from the store or an override, never the file.
    let env = "name: e\nvariables:\n  - name: sk\n    secret: true\n";
    let dir = write_tree(&[
        ("opencollection.yml", root),
        ("environments/e.yml", env),
        (
            "r.yml",
            &http(
                "https://x.test/",
                &aws_block("AKID", "{{sk}}", "", "execute-api", "eu-west-1", ""),
            ),
        ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let options = ContextOptions::new()
        .env("e")
        .sensitive_env_var("sk", "env-secret-key-9");
    let ctx = VarContext::new(&c, options).expect("context");
    let r = at(&c, &ctx, "r.yml").expect("resolves");
    assert!(!format!("{r:?}").contains("env-secret-key-9"));
}
