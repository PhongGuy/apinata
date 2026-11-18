//! `apinata network proxy`, `network cert` and `network move-secrets`: the collection's proxy
//! and client certificates in `opencollection.yml`, through the same core calls as the app.

use std::path::Path;

use anyhow::{bail, ensure};
use apinata_core::{
    CertificateKind, ClientCertificate, Collection, CollectionProxy, PROXY_PASSWORD_DOTENV,
    ProxyAuth, ProxyConfig, collection_relative, is_template, passphrase_dotenv_base,
};

use crate::network_secrets::{masked, moves, proxy_password, write};
use crate::trust::collection_root;
use crate::{CertAction, CertArgs, ProxyAction, ProxyArgs};

/// The proxy protocols the app offers.
const PROTOCOLS: [&str; 4] = ["http", "https", "socks4", "socks5"];

/// Apply a `network proxy` action; Ok holds the lines to print.
pub(crate) fn proxy(args: &ProxyArgs) -> anyhow::Result<Vec<String>> {
    let mut c = Collection::open(&collection_root(args.collection.as_deref())?)?;
    let network = c.file().network();
    let chosen = match &args.action {
        ProxyAction::Show => return Ok(describe_proxy(&network.proxy)),
        ProxyAction::Off => CollectionProxy::Off,
        ProxyAction::Clear => CollectionProxy::Inherit,
        ProxyAction::Set {
            protocol,
            host,
            port,
            bypass,
            username,
            password,
            password_stdin,
            no_auth,
        } => {
            // Start from the custom fields the file holds, as the app's Custom choice does,
            // so a flag left out keeps its value.
            let mut p = match &network.proxy {
                CollectionProxy::Custom(p) => p.clone(),
                _ => network
                    .stored_proxy
                    .clone()
                    .unwrap_or_else(|| ProxyConfig::new(PROTOCOLS[0], "", "")),
            };
            if let Some(protocol) = protocol {
                let protocol = protocol.trim();
                ensure!(
                    PROTOCOLS.contains(&protocol) || is_template(protocol),
                    "the proxy protocol must be one of {}, not `{protocol}`",
                    PROTOCOLS.join(", ")
                );
                p.protocol = protocol.to_owned();
            }
            if let Some(host) = host {
                p.hostname = host.trim().to_owned();
            }
            ensure!(
                !p.hostname.is_empty(),
                "a custom proxy needs a host; pass --host"
            );
            if let Some(port) = port {
                p.set_port(proxy_port(port)?);
            }
            if let Some(bypass) = bypass {
                p.bypass = bypass.trim().to_owned();
            }
            if *no_auth {
                p.auth = None;
            }
            let mut auth = p.auth.take().unwrap_or_else(|| ProxyAuth::new("", ""));
            if let Some(username) = username {
                auth.username = username.clone();
            }
            if let Some(password) = password {
                auth.password = reference(password, "--password", "--password-stdin")?;
            }
            if *password_stdin {
                auth.password = secret_line("Proxy password: ")?;
            }
            if !(auth.username.is_empty() && auth.password.is_empty()) {
                p.auth = Some(auth);
            }
            CollectionProxy::Custom(p)
        }
    };
    let mut after = network.clone();
    after.proxy = chosen;
    let mut plan = moves(&c, &network, &after, false)?;
    if let CollectionProxy::Custom(p) = &mut after.proxy
        && let Some(auth) = &mut p.auth
    {
        let previous = proxy_password(&network.proxy);
        plan.plan(&mut auth.password, previous, PROXY_PASSWORD_DOTENV);
    }
    let mut lines = write(&mut c, &plan)?;
    c.file_mut().set_proxy(&after.proxy);
    c.file_mut().save()?;
    lines.extend(describe_proxy(&after.proxy));
    Ok(lines)
}

/// Apply a `network cert` action; Ok holds the lines to print.
pub(crate) fn cert(args: &CertArgs) -> anyhow::Result<Vec<String>> {
    let root = collection_root(args.collection.as_deref())?;
    let mut c = Collection::open(&root)?;
    let network = c.file().network();
    let mut after = network.clone();
    match &args.action {
        CertAction::List => Ok(network.certificates.iter().map(describe_cert).collect()),
        CertAction::Remove { domain } => {
            let domain = domain.trim();
            after
                .certificates
                .retain(|c| !c.domain.eq_ignore_ascii_case(domain));
            if after.certificates.len() == network.certificates.len() {
                return Ok(vec![format!("{domain} was not listed")]);
            }
            c.file_mut().set_client_certificates(&after.certificates);
            c.file_mut().save()?;
            Ok(vec![format!("removed {domain}")])
        }
        CertAction::Set {
            domain,
            cert,
            key,
            pfx,
            passphrase,
            passphrase_stdin,
            disabled,
        } => {
            let domain = domain.trim();
            ensure!(!domain.is_empty(), "the certificate's domain is empty");
            let mut entry = match (cert, key, pfx) {
                (Some(cert), Some(key), None) => ClientCertificate::pem(
                    domain,
                    stored_path(&root, cert)?,
                    stored_path(&root, key)?,
                ),
                (None, None, Some(pfx)) => {
                    ClientCertificate::pkcs12(domain, stored_path(&root, pfx)?)
                }
                _ => bail!("give either --cert and --key, or --pfx"),
            };
            entry.enabled = !*disabled;
            if let Some(passphrase) = passphrase {
                entry.passphrase = reference(passphrase, "--passphrase", "--passphrase-stdin")?;
            }
            if *passphrase_stdin {
                entry.passphrase = secret_line("Passphrase: ")?;
            }
            let row = match after
                .certificates
                .iter()
                .position(|c| c.domain.eq_ignore_ascii_case(domain))
            {
                Some(row) => {
                    after.certificates[row] = entry;
                    row
                }
                None => {
                    after.certificates.push(entry);
                    after.certificates.len() - 1
                }
            };
            let mut plan = moves(&c, &network, &after, false)?;
            let previous = network
                .certificates
                .get(row)
                .map_or("", |c| c.passphrase.as_str());
            plan.plan(
                &mut after.certificates[row].passphrase,
                previous,
                &passphrase_dotenv_base(row),
            );
            let mut lines = write(&mut c, &plan)?;
            c.file_mut().set_client_certificates(&after.certificates);
            c.file_mut().save()?;
            lines.push(describe_cert(&after.certificates[row]));
            Ok(lines)
        }
    }
}

/// Move every literal proxy password and passphrase into `.env`, as the app's "Move to .env".
pub(crate) fn move_secrets(collection: Option<&Path>) -> anyhow::Result<Vec<String>> {
    let mut c = Collection::open(&collection_root(collection)?)?;
    let network = c.file().network();
    let mut after = network.clone();
    let mut plan = moves(&c, &network, &after, true)?;
    if let CollectionProxy::Custom(p) = &mut after.proxy
        && let Some(auth) = &mut p.auth
    {
        let previous = auth.password.clone();
        plan.plan(&mut auth.password, &previous, PROXY_PASSWORD_DOTENV);
    }
    for (row, cert) in after.certificates.iter_mut().enumerate() {
        let previous = cert.passphrase.clone();
        plan.plan(
            &mut cert.passphrase,
            &previous,
            &passphrase_dotenv_base(row),
        );
    }
    if plan.planned().is_empty() {
        return Ok(vec!["nothing to move".to_owned()]);
    }
    let lines = write(&mut c, &plan)?;
    c.file_mut().set_proxy(&after.proxy);
    c.file_mut().set_client_certificates(&after.certificates);
    c.file_mut().save()?;
    Ok(lines)
}

fn describe_proxy(proxy: &CollectionProxy) -> Vec<String> {
    let p = match proxy {
        CollectionProxy::Custom(p) => p,
        CollectionProxy::Off => return vec!["mode: off".to_owned()],
        _ => return vec!["mode: inherit".to_owned()],
    };
    let mut lines = vec![
        "mode: custom".to_owned(),
        format!("protocol: {}", p.protocol),
        format!("host: {}", p.hostname),
    ];
    if !p.port.is_empty() {
        lines.push(format!("port: {}", p.port));
    }
    if let Some(auth) = &p.auth {
        lines.push(format!("username: {}", auth.username));
        if !auth.password.is_empty() {
            lines.push(format!("password: {}", masked(&auth.password)));
        }
    }
    if !p.bypass.is_empty() {
        lines.push(format!("bypass: {}", p.bypass));
    }
    lines
}

fn describe_cert(cert: &ClientCertificate) -> String {
    let kind = match &cert.kind {
        CertificateKind::Pem {
            certificate,
            private_key,
        } => format!("pem {certificate} {private_key}"),
        CertificateKind::Pkcs12 { file } => format!("pkcs12 {file}"),
        _ => "unknown".to_owned(),
    };
    let mut line = format!("{} -> {kind}", cert.domain);
    if !cert.passphrase.is_empty() {
        line.push_str(&format!(" (passphrase {})", masked(&cert.passphrase)));
    }
    if !cert.enabled {
        line.push_str(" (disabled)");
    }
    line
}

/// A port as the app takes it: blank, 1 to 65535, or a template.
fn proxy_port(text: &str) -> anyhow::Result<String> {
    let text = text.trim();
    if text.is_empty() || is_template(text) {
        return Ok(text.to_owned());
    }
    match text.parse::<u16>() {
        Ok(n) if n > 0 && text.bytes().all(|b| b.is_ascii_digit()) => Ok(n.to_string()),
        _ => bail!("the proxy port must be blank or a whole number from 1 to 65535, not `{text}`"),
    }
}

/// A secret flag's value, which must be a `{{variable}}` reference: a literal on the command
/// line would land in shell history.
fn reference(value: &str, flag: &str, stdin_flag: &str) -> anyhow::Result<String> {
    ensure!(
        is_template(value),
        "{flag} takes only a {{{{variable}}}} reference; pass the value itself with {stdin_flag}"
    );
    Ok(value.to_owned())
}

/// One secret from stdin or a hidden prompt, without its line ending; empty is refused.
fn secret_line(prompt: &str) -> anyhow::Result<String> {
    let value = crate::env_cmd::read_secret_line(prompt)?;
    let value = value.trim_end_matches(['\r', '\n']);
    ensure!(!value.is_empty(), "the secret on stdin is empty");
    Ok(value.to_owned())
}

/// A key file path as the collection stores it: a template as typed, an absolute path inside
/// the collection relative to it, any other path as given (relative paths are read from the
/// collection).
fn stored_path(root: &Path, path: &str) -> anyhow::Result<String> {
    let path = path.trim();
    ensure!(!path.is_empty(), "a certificate path is empty");
    if is_template(path) || !Path::new(path).is_absolute() {
        return Ok(path.to_owned());
    }
    Ok(collection_relative(root, Path::new(path)))
}
