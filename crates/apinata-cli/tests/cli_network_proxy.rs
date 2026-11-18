//! `apinata network proxy`, `network cert` and `network move-secrets`: the collection's proxy,
//! client certificates and their secrets, written exactly as the app's core calls write them.

use crate::common;

use apinata_core::{ClientCertificate, Collection, CollectionProxy, ProxyAuth, ProxyConfig};
use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use std::fs;
use std::path::Path;

fn network(dir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    apinata()
        .arg("network")
        .args(args)
        .arg("--collection")
        .arg(dir)
        .assert()
}

fn network_stdin(dir: &Path, args: &[&str], stdin: &str) -> assert_cmd::assert::Assert {
    apinata()
        .arg("network")
        .args(args)
        .arg("--collection")
        .arg(dir)
        .write_stdin(stdin)
        .assert()
}

fn root_text(dir: &Path) -> String {
    fs::read_to_string(dir.join("opencollection.yml")).expect("root")
}

fn stdout(a: assert_cmd::assert::Assert) -> String {
    String::from_utf8(a.get_output().stdout.clone()).unwrap()
}

/// The root file a fresh collection holds after the app's core edit `edit`.
fn core_written(edit: impl FnOnce(&mut Collection)) -> String {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let mut c = Collection::open(dir.path()).expect("open");
    edit(&mut c);
    c.file_mut().save().expect("save");
    root_text(dir.path())
}

#[test]
fn proxy_set_then_show_lists_the_custom_proxy() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(
        dir.path(),
        &[
            "proxy",
            "set",
            "--protocol",
            "socks5",
            "--host",
            "proxy.test",
            "--port",
            "1080",
            "--bypass",
            "localhost,*.lan",
        ],
    )
    .code(0);
    let shown = stdout(network(dir.path(), &["proxy", "show"]).code(0));
    assert_eq!(
        shown,
        "mode: custom\nprotocol: socks5\nhost: proxy.test\nport: 1080\nbypass: localhost,*.lan\n"
    );
}

#[test]
fn proxy_set_keeps_the_fields_it_is_not_given() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(
        dir.path(),
        &["proxy", "set", "--host", "a.test", "--port", "8080"],
    )
    .code(0);
    network(dir.path(), &["proxy", "set", "--port", "9090"]).code(0);
    let shown = stdout(network(dir.path(), &["proxy", "show"]).code(0));
    assert!(shown.contains("host: a.test\nport: 9090\n"), "{shown}");
    assert!(shown.contains("protocol: http\n"), "{shown}");
}

#[test]
fn proxy_off_and_clear_switch_the_mode() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(dir.path(), &["proxy", "set", "--host", "a.test"]).code(0);
    network(dir.path(), &["proxy", "off"]).code(0);
    assert_eq!(
        stdout(network(dir.path(), &["proxy", "show"]).code(0)),
        "mode: off\n"
    );
    network(dir.path(), &["proxy", "clear"]).code(0);
    assert_eq!(
        stdout(network(dir.path(), &["proxy", "show"]).code(0)),
        "mode: inherit\n"
    );
    // A fresh collection shows the default too.
    let fresh = write_tree(&[("opencollection.yml", ROOT)]);
    assert_eq!(
        stdout(network(fresh.path(), &["proxy", "show"]).code(0)),
        "mode: inherit\n"
    );
}

#[test]
fn proxy_file_matches_the_core_call_byte_for_byte() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(
        dir.path(),
        &[
            "proxy",
            "set",
            "--host",
            "p.test",
            "--port",
            "3128",
            "--bypass",
            "localhost",
        ],
    )
    .code(0);
    let expected = core_written(|c| {
        let mut p = ProxyConfig::new("http", "p.test", "");
        p.set_port("3128");
        p.bypass = "localhost".into();
        c.file_mut().set_proxy(&CollectionProxy::Custom(p));
    });
    assert_eq!(root_text(dir.path()), expected);

    network(dir.path(), &["proxy", "off"]).code(0);
    let off = core_written(|c| {
        let mut p = ProxyConfig::new("http", "p.test", "");
        p.set_port("3128");
        p.bypass = "localhost".into();
        c.file_mut().set_proxy(&CollectionProxy::Custom(p));
        c.file_mut().set_proxy(&CollectionProxy::Off);
    });
    assert_eq!(root_text(dir.path()), off);
}

#[test]
fn proxy_password_from_stdin_goes_to_dotenv_and_is_never_shown() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let a = network_stdin(
        dir.path(),
        &[
            "proxy",
            "set",
            "--host",
            "p.test",
            "--username",
            "me",
            "--password-stdin",
        ],
        "s3cret-pw\n",
    )
    .code(0);
    let out = a.get_output();
    let (o, e) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        !o.contains("s3cret-pw") && !e.contains("s3cret-pw"),
        "{o}{e}"
    );
    let yaml = root_text(dir.path());
    assert!(!yaml.contains("s3cret-pw"), "{yaml}");
    let expected = core_written(|c| {
        let mut p = ProxyConfig::new("http", "p.test", "");
        p.auth = Some(ProxyAuth::new(
            "me",
            "{{process.env.APINATA_PROXY_PASSWORD}}",
        ));
        c.file_mut().set_proxy(&CollectionProxy::Custom(p));
    });
    assert_eq!(yaml, expected);
    let env = fs::read_to_string(dir.path().join(".env")).expect(".env");
    assert_eq!(env, "APINATA_PROXY_PASSWORD='s3cret-pw'\n");
    let ignore = fs::read_to_string(dir.path().join(".gitignore")).expect(".gitignore");
    assert!(ignore.lines().any(|l| l == ".env"), "{ignore}");

    let shown = stdout(network(dir.path(), &["proxy", "show"]).code(0));
    assert!(shown.contains("username: me\n"), "{shown}");
    assert!(
        shown.contains("password: {{process.env.APINATA_PROXY_PASSWORD}}\n"),
        "{shown}"
    );
    assert!(!shown.contains("s3cret-pw"), "{shown}");
}

#[test]
fn a_second_password_reuses_the_name_the_field_refers_to() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (".env", "APINATA_PROXY_PASSWORD=x\n"),
    ]);
    // The name is taken by another value, so the first move picks the next free one.
    network_stdin(
        dir.path(),
        &[
            "proxy",
            "set",
            "--host",
            "p.test",
            "--username",
            "me",
            "--password-stdin",
        ],
        "one\n",
    )
    .code(0);
    assert!(root_text(dir.path()).contains("{{process.env.APINATA_PROXY_PASSWORD_2}}"));
    network_stdin(dir.path(), &["proxy", "set", "--password-stdin"], "two\n").code(0);
    let env = fs::read_to_string(dir.path().join(".env")).unwrap();
    assert_eq!(
        env,
        "APINATA_PROXY_PASSWORD=x\nAPINATA_PROXY_PASSWORD_2='two'\n"
    );
}

#[test]
fn a_literal_password_flag_is_refused_but_a_reference_is_kept() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(
        dir.path(),
        &["proxy", "set", "--host", "p.test", "--password", "hunter2"],
    )
    .code(3)
    .stderr(contains("--password-stdin"));
    assert_eq!(root_text(dir.path()), ROOT);
    network(
        dir.path(),
        &[
            "proxy",
            "set",
            "--host",
            "p.test",
            "--username",
            "me",
            "--password",
            "{{proxyPass}}",
        ],
    )
    .code(0);
    assert!(root_text(dir.path()).contains("password: \"{{proxyPass}}\""));
    assert!(!dir.path().join(".env").exists());
}

#[test]
fn no_auth_drops_the_credentials() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(
        dir.path(),
        &[
            "proxy",
            "set",
            "--host",
            "p.test",
            "--username",
            "me",
            "--password",
            "{{p}}",
        ],
    )
    .code(0);
    network(dir.path(), &["proxy", "set", "--no-auth"]).code(0);
    let shown = stdout(network(dir.path(), &["proxy", "show"]).code(0));
    assert!(!shown.contains("username"), "{shown}");
}

#[test]
fn bad_proxy_input_exits_3_and_writes_nothing() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(dir.path(), &["proxy", "set", "--port", "8080"])
        .code(3)
        .stderr(contains("--host"));
    network(
        dir.path(),
        &["proxy", "set", "--host", "p.test", "--port", "99999"],
    )
    .code(3)
    .stderr(contains("65535"));
    network(
        dir.path(),
        &["proxy", "set", "--host", "p.test", "--protocol", "ftp"],
    )
    .code(3);
    network(
        dir.path(),
        &[
            "proxy",
            "set",
            "--host",
            "p.test",
            "--password",
            "{{p}}",
            "--password-stdin",
        ],
    )
    .code(3);
    assert_eq!(root_text(dir.path()), ROOT);
}

#[test]
fn cert_set_list_and_remove() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let cert = dir.path().join("certs/c.pem");
    let key = dir.path().join("certs/k.pem");
    network(
        dir.path(),
        &[
            "cert",
            "set",
            "api.test",
            "--cert",
            cert.to_str().unwrap(),
            "--key",
            key.to_str().unwrap(),
        ],
    )
    .code(0);
    network(
        dir.path(),
        &[
            "cert",
            "set",
            "*.corp.test",
            "--pfx",
            "/etc/ssl/me.p12",
            "--disabled",
        ],
    )
    .code(0);
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(
        stdout(network(dir.path(), &["cert", "list"]).code(0)),
        format!(
            "api.test -> pem certs{sep}c.pem certs{sep}k.pem\n*.corp.test -> pkcs12 /etc/ssl/me.p12 (disabled)\n"
        )
    );
    network(dir.path(), &["cert", "remove", "api.test"])
        .code(0)
        .stdout(contains("removed api.test"));
    network(dir.path(), &["cert", "remove", "api.test"])
        .code(0)
        .stdout(contains("was not listed"));
    network(dir.path(), &["cert", "remove", "*.corp.test"]).code(0);
    // The last removal drops the key and the emptied `config` with it.
    assert!(!root_text(dir.path()).contains("config"));
}

#[test]
fn cert_set_replaces_the_row_of_the_same_domain() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(dir.path(), &["cert", "set", "a.test", "--pfx", "one.p12"]).code(0);
    network(dir.path(), &["cert", "set", "a.test", "--pfx", "two.p12"]).code(0);
    assert_eq!(
        stdout(network(dir.path(), &["cert", "list"]).code(0)),
        "a.test -> pkcs12 two.p12\n"
    );
}

#[test]
fn cert_file_matches_the_core_call_and_the_passphrase_goes_to_dotenv() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(
        dir.path(),
        &["cert", "set", "a.test", "--cert", "c.pem", "--key", "k.pem"],
    )
    .code(0);
    let a = network_stdin(
        dir.path(),
        &[
            "cert",
            "set",
            "b.test",
            "--pfx",
            "b.p12",
            "--passphrase-stdin",
        ],
        "pfx-secret\n",
    )
    .code(0);
    let out = a.get_output();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("pfx-secret"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("pfx-secret"));
    let expected = core_written(|c| {
        let mut b = ClientCertificate::pkcs12("b.test", "b.p12");
        b.passphrase = "{{process.env.APINATA_CERT_PASSPHRASE_2}}".into();
        c.file_mut()
            .set_client_certificates(&[ClientCertificate::pem("a.test", "c.pem", "k.pem"), b]);
    });
    assert_eq!(root_text(dir.path()), expected);
    let env = fs::read_to_string(dir.path().join(".env")).unwrap();
    assert_eq!(env, "APINATA_CERT_PASSPHRASE_2='pfx-secret'\n");
    let listed = stdout(network(dir.path(), &["cert", "list"]).code(0));
    assert!(
        listed.contains(
            "b.test -> pkcs12 b.p12 (passphrase {{process.env.APINATA_CERT_PASSPHRASE_2}})"
        ),
        "{listed}"
    );
}

#[test]
fn bad_cert_input_exits_3_and_writes_nothing() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    network(dir.path(), &["cert", "set", "a.test", "--cert", "c.pem"]).code(3);
    network(
        dir.path(),
        &["cert", "set", "a.test", "--pfx", "a.p12", "--cert", "c.pem"],
    )
    .code(3);
    network(dir.path(), &["cert", "set", "a.test"]).code(3);
    network(dir.path(), &["cert", "set", " ", "--pfx", "a.p12"]).code(3);
    network(
        dir.path(),
        &[
            "cert",
            "set",
            "a.test",
            "--pfx",
            "a.p12",
            "--passphrase",
            "plain",
        ],
    )
    .code(3)
    .stderr(contains("--passphrase-stdin"));
    assert_eq!(root_text(dir.path()), ROOT);
}

#[test]
fn move_secrets_moves_literal_password_and_passphrase_to_dotenv() {
    let root = format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: p.test\n      port: \"\"\n      auth:\n        username: me\n        password: lit-pw\n      bypassProxy: \"\"\n  clientCertificates:\n    - domain: a.test\n      type: pkcs12\n      pkcs12FilePath: a.p12\n      passphrase: lit-pass\n"
    );
    let dir = write_tree(&[("opencollection.yml", &root)]);
    let shown = stdout(network(dir.path(), &["proxy", "show"]).code(0));
    assert!(!shown.contains("lit-pw"), "{shown}");
    let listed = stdout(network(dir.path(), &["cert", "list"]).code(0));
    assert!(!listed.contains("lit-pass"), "{listed}");

    let a = network(dir.path(), &["move-secrets"]).code(0);
    let out = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert!(
        !out.contains("lit-pw") && !out.contains("lit-pass"),
        "{out}"
    );
    let yaml = root_text(dir.path());
    assert!(
        !yaml.contains("lit-pw") && !yaml.contains("lit-pass"),
        "{yaml}"
    );
    assert!(
        yaml.contains("{{process.env.APINATA_PROXY_PASSWORD}}"),
        "{yaml}"
    );
    assert!(
        yaml.contains("{{process.env.APINATA_CERT_PASSPHRASE_1}}"),
        "{yaml}"
    );
    let env = fs::read_to_string(dir.path().join(".env")).unwrap();
    assert_eq!(
        env,
        "APINATA_PROXY_PASSWORD='lit-pw'\nAPINATA_CERT_PASSPHRASE_1='lit-pass'\n"
    );
    network(dir.path(), &["move-secrets"])
        .code(0)
        .stdout(contains("nothing to move"));
}
