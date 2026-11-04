//! `apinata network hosts`: edit the collection's host overrides in `opencollection.yml`.

use std::process::ExitCode;

use anyhow::bail;
use apinata_core::{Collection, HostOverride, normalize_bare_host};

use crate::trust::{collection_root, shown};
use crate::{EXIT_USAGE, HostsAction, NetworkAction, NetworkCmdArgs};

/// Apply the action: exit 0 on success, 3 with `error: ...` on any failure.
pub fn run(args: &NetworkCmdArgs) -> ExitCode {
    let NetworkAction::Hosts(hosts) = &args.action;
    match apply(hosts) {
        Ok(lines) => {
            for line in lines {
                outln!("{}", shown(&line));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {}", shown(&format!("{e:#}")));
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn apply(args: &crate::HostsArgs) -> anyhow::Result<Vec<String>> {
    let root = collection_root(args.collection.as_deref())?;
    let mut collection = Collection::open(&root)?;
    let mut rows = collection.file().network().host_overrides;
    match &args.action {
        HostsAction::List => Ok(rows.iter().map(describe).collect()),
        HostsAction::Set {
            host,
            address,
            disabled,
        } => {
            let host = name(host)?;
            let mut row = HostOverride::new(host.clone(), address.trim());
            row.enabled = !*disabled;
            row.check()?;
            match rows.iter_mut().find(|r| same(&r.host, &host)) {
                Some(existing) => *existing = row.clone(),
                None => rows.push(row.clone()),
            }
            save(&mut collection, &rows)?;
            Ok(vec![describe(&row)])
        }
        HostsAction::Remove { host } => {
            let host = name(host)?;
            let before = rows.len();
            rows.retain(|r| !same(&r.host, &host));
            if rows.len() == before {
                return Ok(vec![format!("{host} was not listed")]);
            }
            save(&mut collection, &rows)?;
            Ok(vec![format!("removed {host}")])
        }
    }
}

fn save(collection: &mut Collection, rows: &[HostOverride]) -> anyhow::Result<()> {
    collection.file_mut().set_host_overrides(rows);
    collection.file_mut().save()?;
    Ok(())
}

/// The host as stored: a template is kept as typed, a name is normalised like a URL host.
fn name(input: &str) -> anyhow::Result<String> {
    if input.contains("{{") {
        return Ok(input.trim().to_owned());
    }
    match normalize_bare_host(input) {
        Some(host) => Ok(host),
        None => bail!("`{}` is not a bare host name", input.trim()),
    }
}

fn same(stored: &str, host: &str) -> bool {
    stored.eq_ignore_ascii_case(host)
        || normalize_bare_host(stored).is_some_and(|s| s.eq_ignore_ascii_case(host))
}

fn describe(row: &HostOverride) -> String {
    let off = if row.enabled { "" } else { " (disabled)" };
    format!("{} -> {}{off}", row.host, row.address)
}
