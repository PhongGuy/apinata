//! Team server addresses, tokens, roles and names, without a network.

use apinata_core::team::{NewWorkspace, Role, ServerUrl, TeamError, TeamToken, valid_name};

fn url(input: &str) -> String {
    ServerUrl::parse(input).unwrap().as_str().to_owned()
}

#[test]
fn an_address_is_normalized_to_scheme_host_and_path_without_a_trailing_slash() {
    assert_eq!(url("https://team.example.com/"), "https://team.example.com");
    assert_eq!(url("  http://127.0.0.1:8080  "), "http://127.0.0.1:8080");
    assert_eq!(url("HTTPS://Team.Example.COM"), "https://team.example.com");
    assert_eq!(
        url("https://h.example/potluck/"),
        "https://h.example/potluck"
    );
    assert_eq!(url("team.example.com"), "https://team.example.com");
}

#[test]
fn an_address_with_credentials_a_query_or_another_scheme_is_refused() {
    for bad in [
        "",
        "   ",
        "ftp://h.example",
        "https://user:pw@h.example",
        "https://h.example/?x=1",
        "https://h.example/#top",
        "https://",
    ] {
        assert!(
            matches!(ServerUrl::parse(bad), Err(TeamError::BadServerUrl(_))),
            "{bad:?}"
        );
    }
    assert_eq!(
        ServerUrl::parse(""),
        Err(TeamError::BadServerUrl("Enter the server address.".into()))
    );
}

#[test]
fn only_plain_http_to_another_host_counts_as_unencrypted() {
    let unencrypted = |s: &str| ServerUrl::parse(s).unwrap().is_unencrypted();
    assert!(unencrypted("http://team.example.com"));
    assert!(!unencrypted("https://team.example.com"));
    assert!(!unencrypted("http://127.0.0.1:8080"));
    assert!(!unencrypted("http://localhost:8080"));
    assert!(!unencrypted("http://[::1]:8080"));
}

#[test]
fn a_token_is_plk_and_43_base64url_characters_and_never_debug_prints() {
    let text = format!("plk_{}", "Ab9-_".repeat(8) + "Ab9");
    let token = TeamToken::parse(&text).unwrap();
    assert_eq!(token.expose(), text);
    assert_eq!(format!("{token:?}"), "TeamToken(plk_…)");
    for bad in [
        format!("plk_{}", "A".repeat(42)),
        format!("plk_{}", "A".repeat(44)),
        format!("pat_{}", "A".repeat(43)),
        format!("plk_{}+", "A".repeat(42)),
    ] {
        assert!(
            matches!(TeamToken::parse(&bad), Err(TeamError::Protocol(_))),
            "{bad}"
        );
    }
}

#[test]
fn roles_order_read_write_admin_and_an_unknown_role_reads_as_read() {
    assert_eq!(Role::parse("read"), Role::Read);
    assert_eq!(Role::parse("write"), Role::Write);
    assert_eq!(Role::parse("admin"), Role::Admin);
    assert_eq!(Role::parse("owner"), Role::Read);
    assert!(Role::Read < Role::Write && Role::Write < Role::Admin);
    assert_eq!(
        [Role::Read.label(), Role::Write.label(), Role::Admin.label()],
        ["Read", "Write", "Admin"]
    );
    assert!(!Role::Read.can_write() && Role::Write.can_write() && Role::Admin.can_write());
}

#[test]
fn names_are_lowercase_labels_of_at_most_63_characters() {
    let long = "a".repeat(63);
    let too_long = "a".repeat(64);
    for good in ["a", "team-1", "0x", long.as_str()] {
        assert!(valid_name(good), "{good}");
    }
    for bad in ["", "-a", "A", "a_b", "a b", too_long.as_str()] {
        assert!(!valid_name(bad), "{bad}");
    }
}

#[test]
fn a_new_workspace_checks_each_field_and_names_the_bad_one() {
    let ok = NewWorkspace::new(" core ", "api-tests", " API tests ").unwrap();
    assert_eq!(
        (ok.team.as_str(), ok.slug.as_str(), ok.name.as_str()),
        ("core", "api-tests", "API tests")
    );
    let field = |r: Result<NewWorkspace, TeamError>| match r {
        Err(TeamError::Invalid { field, .. }) => field,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        field(NewWorkspace::new("Core", "api", "API")).as_deref(),
        Some("team")
    );
    assert_eq!(
        field(NewWorkspace::new("core", "api_tests", "API")).as_deref(),
        Some("slug")
    );
    assert_eq!(
        field(NewWorkspace::new("core", "api", "  ")).as_deref(),
        Some("name")
    );
}
