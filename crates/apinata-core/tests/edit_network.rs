//! `config.proxy` and `config.clientCertificates` on `opencollection.yml`: read as Bruno's
//! reader does, written with Bruno's bytes, untouched files left byte for byte.

use crate::config_parity;

use apinata_core::{
    CertificateKind, ClientCertificate, Collection, CollectionNetwork, CollectionProxy, ProxyAuth,
    ProxyConfig,
};
use config_parity::check_corpus;
use std::fs;
use tempfile::TempDir;

fn open(root: &str) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("opencollection.yml"), root).expect("root");
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn saved(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("opencollection.yml")).expect("read")
}

const FULL: &str = r#"opencollection: "1.0.0"
info:
  name: t
config:
  proxy:
    inherit: false
    config:
      protocol: socks5
      hostname: "{{proxyHost}}"
      port: 1080
      auth:
        username: u
        password: "{{process.env.PW}}"
      bypassProxy: localhost, .internal
  clientCertificates:
    - domain: "*.example.com"
      type: pem
      certificateFilePath: certs/client.pem
      privateKeyFilePath: certs/client.key
      passphrase: pp
      x-note: kept
    - domain: api.example.com:8443
      type: pkcs12
      pkcs12FilePath: certs/client.p12
      disabled: true
    - domain: other.test
      type: jks
      storeFilePath: certs/x.jks
"#;

fn proxy_of(block: &str) -> CollectionProxy {
    let (_dir, c) = open(&format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n{block}"
    ));
    c.file().network().proxy
}

#[test]
fn every_field_reads_as_bruno_reads_it() {
    let (_dir, c) = open(FULL);
    let network = c.file().network();
    let CollectionProxy::Custom(proxy) = &network.proxy else {
        panic!("{:?}", network.proxy)
    };
    assert_eq!(
        (
            proxy.protocol.as_str(),
            proxy.hostname.as_str(),
            proxy.port.as_str()
        ),
        ("socks5", "{{proxyHost}}", "1080")
    );
    assert_eq!(proxy.auth, Some(ProxyAuth::new("u", "{{process.env.PW}}")));
    assert_eq!(proxy.bypass, "localhost, .internal");
    assert_eq!(
        network.certificates.len(),
        2,
        "the unknown type is not offered"
    );
    let first = &network.certificates[0];
    assert_eq!(first.domain, "*.example.com");
    assert_eq!(
        first.kind,
        CertificateKind::Pem {
            certificate: "certs/client.pem".into(),
            private_key: "certs/client.key".into()
        }
    );
    assert_eq!((first.passphrase.as_str(), first.enabled), ("pp", true));
    let second = &network.certificates[1];
    assert_eq!(
        second.kind,
        CertificateKind::Pkcs12 {
            file: "certs/client.p12".into()
        }
    );
    assert_eq!((second.passphrase.as_str(), second.enabled), ("", false));
    assert!(!format!("{network:?}").contains("{{process.env.PW}}"));
}

#[test]
fn modes_follow_disabled_then_inherit() {
    let config = "    config:\n      protocol: http\n      hostname: h\n      port: \"\"\n";
    assert_eq!(
        proxy_of(&format!("    inherit: true\n{config}")),
        CollectionProxy::Inherit
    );
    assert_eq!(
        proxy_of(&format!("    inherit: true\n{config}    disabled: true\n")),
        CollectionProxy::Off
    );
    assert_eq!(
        proxy_of(&format!("    inherit: false\n{config}    disabled: true\n")),
        CollectionProxy::Off
    );
    assert!(matches!(
        proxy_of(&format!("    inherit: false\n{config}")),
        CollectionProxy::Custom(_)
    ));
}

#[test]
fn a_block_without_boolean_inherit_and_config_mapping_reads_as_inherit() {
    assert_eq!(
        proxy_of("    enabled: true\n    hostname: h\n"),
        CollectionProxy::Inherit
    );
    assert_eq!(
        proxy_of("    inherit: \"false\"\n    config:\n      hostname: h\n"),
        CollectionProxy::Inherit
    );
    assert_eq!(proxy_of("    inherit: false\n"), CollectionProxy::Inherit);
    let (_dir, c) = open("opencollection: \"1.0.0\"\ninfo:\n  name: t\n");
    assert_eq!(c.file().network(), CollectionNetwork::default());
}

#[test]
fn empty_fields_take_bruno_defaults_and_disabled_or_empty_auth_is_none() {
    let CollectionProxy::Custom(p) = proxy_of(
        "    inherit: false\n    config:\n      hostname: h\n      auth:\n        username: u\n        password: p\n        disabled: true\n",
    ) else {
        panic!("custom")
    };
    assert_eq!(
        (p.protocol.as_str(), p.port.as_str(), p.bypass.as_str()),
        ("http", "", "")
    );
    assert_eq!(p.auth, None);
    let CollectionProxy::Custom(p) = proxy_of(
        "    inherit: false\n    config:\n      hostname: h\n      auth:\n        username: \"\"\n        password: \"\"\n",
    ) else {
        panic!("custom")
    };
    assert_eq!(p.auth, None);
}

#[test]
fn network_edits_match_bruno() {
    check_corpus("2026-10-11-network-config-parity", "network-", |id, c| {
        let file = c.file_mut();
        match id {
            "network-proxy-custom" => {
                let mut p = ProxyConfig::new("http", "proxy.test", "3128");
                p.auth = Some(ProxyAuth::new(
                    "u",
                    "{{process.env.APINATA_PROXY_PASSWORD}}",
                ));
                p.bypass = "localhost,.internal".into();
                file.set_proxy(&CollectionProxy::Custom(p));
            }
            "network-proxy-socks-noauth" => file.set_proxy(&CollectionProxy::Custom(
                ProxyConfig::new("socks5", "10.0.0.5", "1080"),
            )),
            "network-proxy-off" => file.set_proxy(&CollectionProxy::Off),
            "network-certs-add" => {
                let mut pem =
                    ClientCertificate::pem("*.example.com", "certs/client.pem", "certs/client.key");
                pem.passphrase = "{{process.env.APINATA_CERT_PASSPHRASE_1}}".into();
                let mut p12 = ClientCertificate::pkcs12("api.example.com:8443", "certs/client.p12");
                p12.enabled = false;
                file.set_client_certificates(&[pem, p12]);
            }
            other => panic!("unexpected case {other}"),
        }
        file.save().expect("saves");
    });
}

#[test]
fn reading_and_saving_without_an_edit_keeps_the_bytes() {
    let (dir, mut c) = open(FULL);
    let network = c.file().network();
    c.file_mut().set_client_certificates(&network.certificates);
    assert!(
        !c.file().is_dirty(),
        "the same certificates written back change nothing"
    );
    c.file_mut().save().expect("saves");
    assert_eq!(saved(&dir), FULL);
}

#[test]
fn certificate_edits_keep_unknown_keys_and_unknown_types() {
    let (dir, mut c) = open(FULL);
    let mut certs = c.file().network().certificates;
    certs[0].domain = "*.example.org".into();
    certs.remove(1);
    c.file_mut().set_client_certificates(&certs);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    assert!(text.contains("domain: \"*.example.org\""), "{text}");
    assert!(text.contains("x-note: kept"), "{text}");
    assert!(!text.contains("pkcs12FilePath"), "{text}");
    assert!(text.contains("type: jks"), "{text}");
}

#[test]
fn clearing_the_certificates_removes_the_key() {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  clientCertificates:\n    - domain: a.test\n      type: pkcs12\n      pkcs12FilePath: a.p12\n";
    let (dir, mut c) = open(root);
    c.file_mut().set_client_certificates(&[]);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    assert!(!text.contains("clientCertificates"), "{text}");
    assert!(
        !text.contains("config:"),
        "an emptied config goes too: {text}"
    );
}

#[test]
fn switching_modes_keeps_the_custom_fields() {
    let (dir, mut c) = open(FULL);
    c.file_mut().set_proxy(&CollectionProxy::Off);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    assert!(text.contains("hostname: \"{{proxyHost}}\""), "{text}");
    assert!(text.contains("inherit: false"), "{text}");
    assert!(text.contains("\n    disabled: true\n"), "{text}");
    c.file_mut().set_proxy(&CollectionProxy::Inherit);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    assert!(text.contains("inherit: true"), "{text}");
    assert!(!text.contains("\n    disabled: true\n"), "{text}");
    assert!(text.contains("hostname: \"{{proxyHost}}\""), "{text}");
}

#[test]
fn a_collection_without_a_block_gets_brunos_defaults() {
    let (dir, mut c) = open("opencollection: \"1.0.0\"\ninfo:\n  name: t\n");
    c.file_mut().set_proxy(&CollectionProxy::Off);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    for line in [
        "  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"\"\n      port: \"\"\n",
        "      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\n    disabled: true\n",
    ] {
        assert!(text.contains(line), "{text}");
    }
}

fn stored_of(block: &str) -> Option<ProxyConfig> {
    let (_dir, c) = open(&format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n{block}"
    ));
    c.file().network().stored_proxy
}

#[test]
fn an_inherited_or_disabled_proxy_keeps_its_stored_fields() {
    let config = "    config:\n      protocol: http\n      hostname: p.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{{process.env.PW}}\"\n      bypassProxy: \"\"\n";
    let mut want = ProxyConfig::new("http", "p.local", "3128");
    want.auth = Some(ProxyAuth::new("u", "{{process.env.PW}}"));
    let off = format!("    inherit: false\n    disabled: true\n{config}");
    let inherit = format!("    inherit: true\n{config}");
    let custom = format!("    inherit: false\n{config}");
    assert_eq!(stored_of(&off), Some(want.clone()));
    assert_eq!(stored_of(&inherit), Some(want));
    assert_eq!(
        stored_of(&custom),
        None,
        "a custom proxy carries its own fields"
    );
    assert_eq!(
        stored_of("    inherit: true\n    disabled: true\n    config: plain\n"),
        None,
        "a non-mapping config stores nothing"
    );
}

// A custom proxy as the writer lays it out, with `PORT` standing for the port line's value.
const PORTED: &str = "opencollection: 1.0.0\n\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: h\n      port: PORT\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\n";

/// Open `PORTED` with `port`, apply `edit` to its proxy, save, and return the file.
fn port_round_trip(port: &str, edit: impl FnOnce(&mut ProxyConfig)) -> String {
    let (dir, mut c) = open(&PORTED.replace("PORT", port));
    let CollectionProxy::Custom(mut p) = c.file().network().proxy else {
        panic!("custom")
    };
    edit(&mut p);
    c.file_mut().set_proxy(&CollectionProxy::Custom(p));
    c.file_mut().save().expect("saves");
    saved(&dir)
}

// Expected bytes derived from yaml@2.3.4's scalar rules: a number plain, a digit string quoted.
#[test]
fn a_typed_port_of_digits_is_written_as_a_number() {
    let text = port_round_trip("\"3128\"", |p| p.set_port("8080"));
    assert_eq!(text, PORTED.replace("PORT", "8080"));
}

#[test]
fn a_typed_empty_port_is_written_as_an_empty_string() {
    let text = port_round_trip("3128", |p| p.set_port(""));
    assert_eq!(text, PORTED.replace("PORT", "\"\""));
}

#[test]
fn a_port_read_as_a_number_is_written_back_as_one() {
    let text = port_round_trip("3128", |p| {
        p.hostname = "h2".into();
        p.set_port("3128");
    });
    assert_eq!(
        text,
        PORTED
            .replace("PORT", "3128")
            .replace("hostname: h\n", "hostname: h2\n")
    );
}

#[test]
fn a_port_read_as_a_string_is_written_back_as_one() {
    let text = port_round_trip("\"3128\"", |p| {
        p.hostname = "h2".into();
        p.set_port("3128");
    });
    assert_eq!(
        text,
        PORTED
            .replace("PORT", "\"3128\"")
            .replace("hostname: h\n", "hostname: h2\n")
    );
}

// A port read as any YAML number goes back as that number, as the reference app keeps the
// JavaScript number it read; the writer prints it as yaml@2.3.4's stringifyNumber would.
#[test]
fn a_port_read_as_a_non_integer_number_is_written_back_as_one() {
    for (read, written) in [
        ("8080.5", "8080.5"),
        ("-1", "-1"),
        ("1e3", "1000"),
        ("8080.0", "8080"),
    ] {
        let text = port_round_trip(read, |p| {
            p.hostname = "h2".into();
            let same = p.port.clone();
            p.set_port(same);
        });
        assert_eq!(
            text,
            PORTED
                .replace("PORT", written)
                .replace("hostname: h\n", "hostname: h2\n"),
            "port: {read}"
        );
    }
}
