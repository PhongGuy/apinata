//! `apinata check`: the report of `check_collection` as text or JSON.

use std::process::ExitCode;
use std::sync::Arc;

use apinata_core::{CheckError, CheckOptions, CheckReport, SecretStore, check_collection};

use crate::import::default_keychain;
use crate::{CheckArgs, EXIT_USAGE, collection_arg, plural, trust};

/// Exit status when the report holds at least one error finding.
const EXIT_FINDINGS: u8 = 1;

/// Check the collection: 0 without error findings, 1 with any, 3 when it cannot be checked.
pub fn run(args: &CheckArgs) -> ExitCode {
    let report = match check(args) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    if args.json {
        match serde_json::to_string_pretty(&report.to_json_versioned()) {
            Ok(json) => outln!("{json}"),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    } else {
        out!("{}", text(&report));
    }
    exit_code(&report)
}

fn check(args: &CheckArgs) -> anyhow::Result<CheckReport> {
    check_with(args, default_keychain())
}

/// [`check`] reading secret values from `keychain`. With `--no-keychain` nothing is read and the
/// report says so, as a run with `--no-keychain` would find the same values missing.
fn check_with(args: &CheckArgs, keychain: Arc<dyn SecretStore>) -> anyhow::Result<CheckReport> {
    let named = collection_arg::pick(args.path.as_deref(), args.collection.as_deref())?;
    let root = collection_arg::with_hint(
        trust::collection_root(named.as_deref()),
        &collection_arg::optional_hint("apinata check [COLLECTION]"),
    )?;
    let mut opts = CheckOptions::default();
    if let Some(name) = &args.env {
        opts = opts.env(name);
    }
    for v in &args.env_var {
        opts = opts.env_var(&v.key, &v.value);
    }
    for v in &args.global_env_var {
        opts = opts.global_env_var(&v.key, &v.value);
    }
    // Check looks in the same store `run` does, so it warns exactly where a run would fail.
    opts = if args.no_keychain {
        opts.keychain_not_used()
    } else {
        opts.secrets(keychain)
    };
    check_collection(&root, &opts).map_err(|e| match e {
        CheckError::Open(e) => anyhow::Error::from(e),
        other => anyhow::anyhow!("{other}"),
    })
}

fn exit_code(report: &CheckReport) -> ExitCode {
    if report.errors() > 0 {
        ExitCode::from(EXIT_FINDINGS)
    } else {
        ExitCode::SUCCESS
    }
}

/// One `severity: item [env]: message` line per finding, then the summary line and, when any
/// request needs run-time input, a note saying so.
fn text(report: &CheckReport) -> String {
    let mut out = String::new();
    for f in report.findings() {
        let severity = f.severity().as_str();
        let env = f
            .environment()
            .map(|e| format!(" [{e}]"))
            .unwrap_or_default();
        out.push_str(&format!("{severity}: {}{env}: {}\n", f.item(), f.message()));
    }
    out.push_str(&format!(
        "{}, {}, {} checked\n",
        plural(report.errors(), "error"),
        plural(report.warnings(), "warning"),
        plural(report.items_checked(), "request")
    ));
    let n = report.unresolved().len();
    if n > 0 {
        let verb = if n == 1 { "it needs" } else { "they need" };
        out.push_str(&format!(
            "{} not fully resolved: {verb} run-time input\n",
            plural(n, "request")
        ));
    }
    out
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use crate::{Cli, Command};
    use apinata_core::{MemoryStore, SecretError, SecretKey, SecretStore};
    use clap::Parser;
    use std::sync::Arc;

    const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: tok\n";

    /// A store that cannot be reached, as a keychain with no session bus.
    struct Refusing;

    impl SecretStore for Refusing {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Err(SecretError::Unavailable {
                message: "no session bus".into(),
            })
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            unreachable!("check never writes secrets")
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            unreachable!("check never deletes secrets")
        }
    }

    fn collection() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let write = |rel: &str, text: &str| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        };
        write(
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: c\n",
        );
        write("environments/dev.yml", DEV);
        write(
            "r.yml",
            "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/{{tok}}\"\n",
        );
        dir
    }

    fn args(dir: &std::path::Path, extra: &[&str]) -> CheckArgs {
        let mut argv = vec!["apinata", "check", dir.to_str().expect("utf-8")];
        argv.extend(["--env", "dev"]);
        argv.extend(extra);
        match Cli::try_parse_from(argv).expect("parses").command {
            Command::Check(args) => args,
            _ => unreachable!("a check command"),
        }
    }

    fn messages(report: &CheckReport) -> Vec<String> {
        report
            .findings()
            .iter()
            .map(|f| f.message().to_owned())
            .collect()
    }

    fn store_with_value(dir: &std::path::Path) -> Arc<MemoryStore> {
        let c = apinata_core::Collection::open(dir).expect("opens");
        let store = Arc::new(MemoryStore::new());
        store
            .set(&c.secret_key("dev", "tok"), "from-store")
            .expect("memory store");
        store
    }

    #[test]
    fn a_stored_value_means_no_warning() {
        let dir = collection();
        let store = store_with_value(dir.path());
        let report = check_with(&args(dir.path(), &[]), store).expect("checks");
        assert!(messages(&report).is_empty(), "{:?}", messages(&report));
    }

    #[test]
    fn env_var_means_no_warning_either() {
        let dir = collection();
        let run = args(dir.path(), &["--env-var", "tok=x"]);
        let report = check_with(&run, Arc::new(MemoryStore::new())).expect("checks");
        assert!(messages(&report).is_empty(), "{:?}", messages(&report));
    }

    #[test]
    fn no_keychain_warns_as_before_whatever_the_store_holds() {
        let dir = collection();
        let store = store_with_value(dir.path());
        let report = check_with(&args(dir.path(), &["--no-keychain"]), store).expect("checks");
        let got = messages(&report);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].contains("with no value"), "{got:?}");
    }

    #[test]
    fn an_unreachable_store_is_named_in_the_warning() {
        let dir = collection();
        let report = check_with(&args(dir.path(), &[]), Arc::new(Refusing)).expect("checks");
        let got = messages(&report);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].contains("no session bus"), "{got:?}");
    }
}
