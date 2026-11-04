//! OAuth 2.0 auth written as the reference writes it, from the
//! OAuth 2.0 parity corpus.

use apinata_core::{
    AuthSpec, ClientAuth, Collection, ItemPath, OAuth2Flow, OAuth2Param, OAuth2ParamPlacement,
    OAuth2Spec, OAuth2Stage, RequestFile, TokenPlacement, TokenSource,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn research() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    kind: String,
    input: String,
    files: Vec<String>,
}

fn cases(kind: &str) -> Vec<Case> {
    let path = research().join("2026-10-11-oauth2-parity/cases.json");
    let all: Vec<Case> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let selected: Vec<Case> = all.into_iter().filter(|c| c.kind == kind).collect();
    assert!(!selected.is_empty(), "no {kind} cases");
    selected
}

fn expected(id: &str, file: &str) -> String {
    fs::read_to_string(
        research()
            .join("2026-10-11-oauth2-parity/expected")
            .join(id)
            .join(file),
    )
    .unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn report(failures: Vec<String>, total: usize) {
    assert!(
        failures.is_empty(),
        "{} of {total} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn spec(r: &RequestFile) -> OAuth2Spec {
    match r.auth() {
        AuthSpec::OAuth2(o) => o,
        other => panic!("not oauth2: {other:?}"),
    }
}

fn param(
    stage: OAuth2Stage,
    name: &str,
    value: &str,
    placement: OAuth2ParamPlacement,
) -> OAuth2Param {
    OAuth2Param::new(stage, name, value, placement)
}

/// The Apinata edit matching each reference edit in the corpus's `edits.mjs`.
fn request_edit(id: &str, r: &RequestFile) -> AuthSpec {
    use OAuth2ParamPlacement as P;
    use OAuth2Stage as S;
    let fresh = |flow| OAuth2Spec::new(flow);
    let mut o = match id {
        "oauth2-new" | "oauth2-password" | "oauth2-implicit" | "oauth2-to-bearer" => {
            fresh(OAuth2Flow::AuthorizationCode)
        }
        _ => spec(r),
    };
    match id {
        "oauth2-new" => {
            o.authorization_url = "https://id.example.com/authorize".into();
            o.access_token_url = "https://id.example.com/token".into();
            o.client_id = "app".into();
        }
        "oauth2-password" => {
            o = fresh(OAuth2Flow::Password);
            o.access_token_url = "https://id.example.com/token".into();
            o.client_id = "cli".into();
            o.username = "ann".into();
            o.password = "{{pw}}".into();
            o.scope = "profile".into();
            o.client_auth = Some(ClientAuth::BasicHeader);
            o.params = vec![
                param(S::Authorization, "prompt", "login", P::Query),
                param(S::Token, "audience", "api", P::Body),
                param(S::Refresh, "X-R", "1", P::Header),
            ];
        }
        "oauth2-implicit" => {
            o = fresh(OAuth2Flow::Implicit);
            o.authorization_url = "https://id.example.com/authorize".into();
            o.callback_url = "https://app.example.com/cb".into();
            o.client_id = "web".into();
            o.client_secret = "unused".into();
            o.scope = "openid".into();
            o.state = "abc".into();
            o.token_source = TokenSource::IdToken;
            o.params = vec![param(S::Authorization, "nonce", "n1", P::Query)];
        }
        "oauth2-edit-fields" => {
            o.client_id = "app2".into();
            o.scope = "read".into();
            o.state = String::new();
            o.callback_url = String::new();
        }
        "oauth2-whitespace" => {
            o.scope = "   ".into();
            o.params[1].name = "  audience  ".into();
            o.params[2].name = " ".into();
        }
        "oauth2-pkce-off" => {
            o.pkce = false;
            o.auto_fetch = false;
        }
        "oauth2-params-clear" => o.params.clear(),
        "oauth2-switch-to-cc" => o.flow = OAuth2Flow::ClientCredentials,
        "oauth2-switch-to-implicit" => {
            o.flow = OAuth2Flow::Implicit;
            o.authorization_url = "https://id.example.com/authorize".into();
        }
        "oauth2-placement-header" => {
            o.token_placement = TokenPlacement::Header {
                prefix: "Token".into(),
            }
        }
        "oauth2-placement-unset" => o.token_placement = TokenPlacement::Unset,
        "oauth2-client-auth-body" => {
            o.client_auth = Some(ClientAuth::Body);
            o.token_source = TokenSource::AccessToken;
            o.auto_refresh = true;
        }
        "oauth2-param-add" => o.params = vec![param(S::Token, "audience", "", P::Body)],
        "oauth2-to-bearer" => return AuthSpec::Bearer { token: "t".into() },
        "oauth2-sparse-minimal" => o.client_id = "svc".into(),
        "oauth2-sparse-unknown" => o.scope = "read".into(),
        other => panic!("no Apinata edit for {other}"),
    }
    AuthSpec::OAuth2(o)
}

#[test]
fn request_edits_match_the_reference() {
    let selected = cases("request");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(&case.files[0]);
        fs::copy(research().join(&case.input), &path).unwrap();
        let mut r = RequestFile::load(&path).unwrap();
        let auth = request_edit(&case.id, &r);
        r.set_auth(auth);
        r.save().unwrap();
        let (actual, want) = (
            fs::read_to_string(&path).unwrap(),
            expected(&case.id, &case.files[0]),
        );
        if actual != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {actual:?}",
                case.id
            ));
        }
    }
    report(failures, selected.len());
}

#[test]
fn config_edits_match_the_reference() {
    let selected = cases("config");
    let mut failures = Vec::new();
    for case in &selected {
        let dir = TempDir::new().unwrap();
        copy_tree(&research().join(&case.input), dir.path());
        let mut c = Collection::open(dir.path()).unwrap();
        match case.id.as_str() {
            "folder-oauth2" => {
                let mut o = OAuth2Spec::new(OAuth2Flow::ClientCredentials);
                o.access_token_url = "https://id.example.com/token".into();
                o.client_id = "svc".into();
                o.client_secret = "{{secret}}".into();
                let f = c
                    .folder_file_mut(&ItemPath::new("folder_1").unwrap())
                    .unwrap();
                f.set_auth(AuthSpec::OAuth2(o));
                f.save().unwrap();
            }
            "collection-oauth2" => {
                let mut o = OAuth2Spec::new(OAuth2Flow::Password);
                o.access_token_url = "https://id.example.com/token".into();
                o.username = "ann".into();
                o.password = "pw".into();
                let f = c.file_mut();
                f.set_auth(AuthSpec::OAuth2(o));
                f.save().unwrap();
            }
            other => panic!("no Apinata edit for {other}"),
        }
        for file in &case.files {
            let actual = fs::read_to_string(dir.path().join(file)).unwrap();
            let want = expected(&case.id, file);
            if actual != want {
                failures.push(format!(
                    "{} {file}\n  expected {want:?}\n  actual   {actual:?}",
                    case.id
                ));
            }
        }
    }
    report(failures, selected.len());
}

#[test]
fn an_edit_elsewhere_leaves_a_sparse_block_byte_for_byte() {
    let input = research().join("2026-10-11-oauth2-parity/sparse/unknown.yml");
    let before = fs::read_to_string(&input).unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("unknown.yml");
    fs::write(&path, &before).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    r.set_url("https://api.example.com/other");
    r.save().unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before.replace(
            "https://api.example.com/items",
            "https://api.example.com/other"
        )
    );
}

#[test]
fn setting_the_read_spec_of_a_normalized_file_is_not_an_edit() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("auth-code.yml");
    fs::copy(
        research().join("2026-10-11-oauth2-parity/normalized/synthetic/auth-code.yml"),
        &path,
    )
    .unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    r.set_auth(r.auth());
    assert!(!r.is_dirty());
}

#[test]
fn on_disk_drops_other_flows_fields_and_is_stable() {
    let mut o = OAuth2Spec::new(OAuth2Flow::ClientCredentials);
    o.authorization_url = "https://a".into();
    o.username = "u".into();
    o.pkce = true;
    o.params = vec![
        param(
            OAuth2Stage::Authorization,
            "n",
            "v",
            OAuth2ParamPlacement::Query,
        ),
        param(OAuth2Stage::Token, " t ", "", OAuth2ParamPlacement::Body),
    ];
    let disk = o.on_disk();
    assert_eq!(
        (
            disk.authorization_url.as_str(),
            disk.username.as_str(),
            disk.pkce
        ),
        ("", "", false)
    );
    assert_eq!(
        disk.params,
        [param(
            OAuth2Stage::Token,
            "t",
            "",
            OAuth2ParamPlacement::Body
        )]
    );
    assert_eq!(disk.on_disk(), disk);
}

#[test]
fn stages_follow_the_flow() {
    use OAuth2Stage::*;
    assert_eq!(OAuth2Flow::ClientCredentials.stages(), [Token, Refresh]);
    assert_eq!(OAuth2Flow::Password.stages(), [Token, Refresh]);
    assert_eq!(
        OAuth2Flow::AuthorizationCode.stages(),
        [Authorization, Token, Refresh]
    );
    assert_eq!(OAuth2Flow::Implicit.stages(), [Authorization]);
}
