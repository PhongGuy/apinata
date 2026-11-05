//! Finding the collection around a path, planning its requests and running them through core.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use apinata_core::{
    AuthorizeOptions, CancelHandle, Collection, ContextOptions, CookieJar, Environment,
    ExecOptions, ExecutionMode, Executor, GlobalEnvironments, HttpVersion, IterationData,
    NetworkEntry, RequestResult, RunEvent, RunOptions, RunPlan, ScriptOptions, SecretStore,
    SelectError, SessionLog, StreamBound, StreamEnd, StreamLimits, TagFilter, TokenCache,
    VarContext,
};

use crate::import::{default_keychain, platform_store, secret_store};
use crate::network;
use crate::show::Show;
use crate::{EnvVar, RunArgs, collection_arg};
use apinata_core::{ConnectCause, ResultsReport, WorkspaceFile};

/// An opened collection, the run planned from the selected path and its variable context.
pub struct Target {
    pub collection: Collection,
    pub plan: RunPlan,
    pub ctx: VarContext,
}

/// The collection root around `path`: the nearest directory, `path` itself included, that holds
/// `opencollection.yml`. Fails naming `path` when no ancestor does, and naming the directory
/// when a legacy collection root is met first.
pub fn find_root(path: &Path) -> anyhow::Result<&Path> {
    for dir in path.ancestors() {
        if dir.join("opencollection.yml").is_file() {
            return Ok(dir);
        }
        if apinata_core::legacy::is_legacy_collection(dir) {
            bail!(
                "{0} is in the legacy .bru format; convert it with `apinata convert {0} --out <new dir>`",
                dir.display()
            );
        }
    }
    bail!(
        "no opencollection.yml in {} or any parent directory",
        path.display()
    )
}

/// The OAuth 2.0 token cache: the encrypted file under the data directory, its key in the
/// keychain (opened only when a request needs a token), or memory when there is no data directory.
fn token_cache() -> TokenCache {
    if let Some(cache) = test_token_cache() {
        return cache;
    }
    match TokenCache::default_path() {
        Some(path) => TokenCache::open(path, platform_store()),
        None => TokenCache::in_memory(),
    }
}

/// Debug builds only, for tests: `APINATA_TEST_TOKEN_CACHE` names a directory to keep the cache
/// in and `APINATA_TEST_TOKEN_KEY` supplies its key, so no test touches the keychain. Release
/// builds compile this away.
#[cfg(debug_assertions)]
fn test_token_cache() -> Option<TokenCache> {
    use apinata_core::{MemoryStore, SecretKey, SecretStore};
    let dir = std::env::var_os("APINATA_TEST_TOKEN_CACHE")?;
    let keys = MemoryStore::new();
    if let Ok(key) = std::env::var("APINATA_TEST_TOKEN_KEY") {
        keys.set(&SecretKey::new("", "", "oauth2-token-key"), &key)
            .expect("a memory store accepts a key");
    }
    Some(TokenCache::open(
        Path::new(&dir).join("oauth2-tokens.json"),
        Arc::new(keys),
    ))
}

#[cfg(not(debug_assertions))]
fn test_token_cache() -> Option<TokenCache> {
    None
}

/// The context options for the selected global environment and its `--global-env-var` overrides,
/// plus the names that environment declares secret.
pub fn global_options(
    global_env: Option<&str>,
    workspace: Option<&Path>,
    global_env_var: &[EnvVar],
) -> anyhow::Result<(ContextOptions, BTreeSet<String>)> {
    let mut options = ContextOptions::new();
    let mut global_secrets = BTreeSet::new();
    if let Some(name) = global_env {
        let (globals, name) = resolve_global(name, workspace)?;
        global_secrets = declared_secrets(globals.environments(), &name);
        options = options.global_env(Arc::new(globals), name);
    }
    for EnvVar {
        key,
        value,
        from_env,
    } in global_env_var
    {
        // A value read from the process environment is a secret even when undeclared.
        options = if *from_env || global_secrets.contains(key) {
            options.sensitive_global_env_var(key, value)
        } else {
            options.global_env_var(key, value)
        };
    }
    Ok((options, global_secrets))
}

/// Open the collection that contains `args.path`, keep the requests that pass `--tags` and
/// `--exclude-tags`, and build its variable context.
pub fn open(args: &RunArgs) -> anyhow::Result<Target> {
    open_with(args, default_keychain())
}

/// [`open`] reading secret values from `keychain`, which `--no-keychain` replaces with a store
/// that holds nothing. A secret `--env-var` supplies is never looked up.
pub fn open_with(args: &RunArgs, keychain: Arc<dyn SecretStore>) -> anyhow::Result<Target> {
    let store: Arc<dyn SecretStore> = if args.no_keychain {
        secret_store(true)
    } else {
        keychain
    };
    let (options, global_secrets) = global_options(
        args.global_env.as_deref(),
        args.workspace.as_deref(),
        &args.global_env_var,
    )?;
    let options = options
        .client_certificates(network::client_certificates(&args.network)?)
        .secrets(store);
    let mut target = collection_arg::with_hint(
        open_at(
            args.target(),
            args.env.as_deref(),
            &args.env_var,
            &global_secrets,
            options,
        ),
        "pass the collection as `apinata run <PATH>` or `apinata run --collection <DIR>`",
    )?;
    target.plan = target.plan.filter_tags(&tag_filter(args));
    Ok(target)
}

/// The filter `--tags` and `--exclude-tags` ask for.
fn tag_filter(args: &RunArgs) -> TagFilter {
    let parse = |list: &Option<String>| {
        list.as_deref()
            .map(TagFilter::parse_list)
            .unwrap_or_default()
    };
    TagFilter::new(parse(&args.tags), parse(&args.exclude_tags))
}

/// Open the collection that contains `path`, plan `path` in it and build the variable context
/// from `options` plus the token cache, the environment `env` and the `env_var` overrides.
/// `global_secrets` names the selected global environment's secrets, which an `env_var` of
/// the same name keeps sensitive.
pub fn open_at(
    path: &Path,
    env: Option<&str>,
    env_var: &[EnvVar],
    global_secrets: &BTreeSet<String>,
    options: ContextOptions,
) -> anyhow::Result<Target> {
    let path =
        std::fs::canonicalize(path).with_context(|| format!("cannot open {}", path.display()))?;
    let root = find_root(&path)?;
    let collection = Collection::open(root)?;
    // Planning precedes the variable context, so a bad path is reported before a bad environment.
    let plan = collection.plan(&path).map_err(|e| match e {
        SelectError::Environment { .. } => anyhow!(
            "{} is an environment, not a request; choose it with --env <name>",
            path.display()
        ),
        e => anyhow!(e),
    })?;
    let mut options = options.oauth2_tokens(token_cache());
    let mut secrets = global_secrets.clone();
    if let Some(env) = env {
        secrets.extend(declared_secrets(collection.environments(), env));
        options = options.env(env);
    }
    for EnvVar {
        key,
        value,
        from_env,
    } in env_var
    {
        // A declared secret, or any value read from the process environment, stays secret so
        // logs and messages hide it.
        options = if *from_env || secrets.contains(key) {
            options.sensitive_env_var(key, value)
        } else {
            options.env_var(key, value)
        };
    }
    let ctx = VarContext::new(&collection, options)?;
    // Environment chain problems go to stderr so a JSON report on stdout stays parseable.
    for warning in ctx.warnings() {
        eprintln!("warning: {warning}");
    }
    Ok(Target {
        collection,
        plan,
        ctx,
    })
}

/// The names environment `name` or any environment it extends declares `secret: true` and enabled.
fn declared_secrets(envs: &[Environment], name: &str) -> BTreeSet<String> {
    let mut secrets = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut next = Some(name.to_owned());
    // Follow `extends` upward; the seen set stops a cycle.
    while let Some(name) = next.take().filter(|n| seen.insert(n.clone())) {
        let Some(env) = envs.iter().find(|e| e.name() == name) else {
            break;
        };
        secrets.extend(
            env.variables()
                .into_iter()
                .filter(|v| v.secret && v.enabled)
                .map(|v| v.name),
        );
        next = env.extends();
    }
    secrets
}

/// The suffix that labels a workspace environment, as the app's picker does.
const WORKSPACE_SUFFIX: &str = " (workspace)";

/// The environment set that holds `--global-env` `name`, and the name inside it. Without a
/// workspace this is the profile. With one, `<name> (workspace)` picks the workspace's `<name>`,
/// and a plain name picks the profile's when it has (or failed to load) that file, else the
/// workspace's. An unknown name returns the profile so the caller's not-found path reports it.
pub fn resolve_global(
    name: &str,
    workspace: Option<&Path>,
) -> anyhow::Result<(GlobalEnvironments, String)> {
    let Some(dir) = workspace else {
        return Ok((global_environments()?, name.to_owned()));
    };
    let shared = WorkspaceFile::open(dir)?.environments()?;
    if let Some(plain) = name.strip_suffix(WORKSPACE_SUFFIX)
        && shared.environment(plain).is_some()
    {
        return Ok((shared, plain.to_owned()));
    }
    let profile = global_environments()?;
    let file = profile.dir().join(format!("{name}.yml"));
    let in_profile = profile.environment(name).is_some()
        || profile.load_errors().iter().any(|e| e.path() == file);
    if !in_profile && shared.environment(name).is_some() {
        return Ok((shared, name.to_owned()));
    }
    Ok((profile, name.to_owned()))
}

/// The profile's global environments, opened only when `--global-env` names one.
pub fn global_environments() -> anyhow::Result<GlobalEnvironments> {
    let dir = GlobalEnvironments::default_dir()
        .ok_or_else(|| anyhow!("no profile directory for global environments"))?;
    Ok(GlobalEnvironments::open(dir)?)
}

/// Run the planned requests in order on one executor through core's runner; the CLI never cancels.
pub async fn run(
    args: &RunArgs,
    show: Show,
    write_failed: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<ResultsReport> {
    let target = open(args)?;
    // A run that would send nothing is a usage error, so a typo in a tag cannot pass CI.
    if let Some(message) = target.plan.empty_message(&tag_filter(args)) {
        bail!(message);
    }
    let data = data_file(args)?;
    let example = crate::example_cmd::plan(args, &target, data.as_ref())?;
    let har = crate::har_cmd::plan(args, &target, data.as_ref());
    let mut exec = ExecOptions::new()
        .insecure(args.insecure)
        .store_cookies(!args.disable_cookies)
        .send_cookies(!args.disable_cookies)
        .network(network::settings(&args.network))
        .http_version(args.network.http_version)
        .timeout_cap(args.timeout.map(Duration::from_secs));
    // Zero lifts the cap; core treats a missing limit on a streamed body as its own default.
    let body_cap = if args.max_body_bytes == 0 {
        u64::MAX
    } else {
        args.max_body_bytes
    };
    exec = exec.max_body_bytes(Some(body_cap));
    let cap_text = match body_cap {
        n if n % (1024 * 1024) == 0 => format!("{} MiB", n / (1024 * 1024)),
        n => format!("{n} bytes"),
    };
    let log = args.verbose.then(SessionLog::new);
    if let Some(log) = &log {
        exec = exec.session_log(log.clone());
    }
    // A malformed jar file stops the run before anything is sent, and is never overwritten.
    // Cookies dropped on read are gone from the jar written back, so the user hears it now.
    let jar = match &args.cookie_jar {
        Some(path) => {
            let jar = Arc::new(CookieJar::default());
            let dropped = jar.read_file(path).map_err(|e| anyhow!(e))?;
            if let Some(notice) = apinata_core::dropped_cookies_notice(dropped) {
                eprintln!("warning: {notice}");
            }
            exec = exec.cookie_jar(Arc::clone(&jar));
            Some((jar, path))
        }
        None => None,
    };
    // Each send's network settings are judged as it resolves them; the notice never blocks.
    if let Some(check) = crate::network_notice::check() {
        exec = exec.network_check(check);
    }
    let executor = Arc::new(Executor::new(exec)?);
    // The event callback and the final drain share one resume point, so no entry prints twice.
    let next = Arc::new(AtomicU64::new(0));
    let limit_text = |b: StreamBound| match b {
        StreamBound::Time => format!("{} s", args.stream_max_time),
        _ => format!("{} events", args.stream_max_events),
    };
    // Both the verbose log drain and the stream-limit note ride the finished-request event.
    let body_files = show.body_files();
    let unfiltered = show.unfiltered();
    let mismatched = crate::query_notice::lines(&target.plan);
    let on_event: Box<dyn FnMut(RunEvent) + Send> = {
        let mut show = show;
        let record_har = har.as_ref().map(|h| h.recorder());
        let log = log.clone();
        let next = Arc::clone(&next);
        let cap_text = cap_text.clone();
        let time = limit_text(StreamBound::Time);
        let events = limit_text(StreamBound::Events);
        Box::new(move |event| {
            if let RunEvent::RequestStarted { path, .. } = &event {
                crate::query_notice::starting(&mismatched, path);
            }
            // Only the item path and the numbers: the URL and headers may hold secrets.
            if let RunEvent::Retrying {
                path,
                attempt,
                of,
                delay,
                ..
            } = &event
            {
                eprintln!(
                    "retrying {path} (attempt {attempt} of {of}) in {} ms",
                    delay.as_millis()
                );
            }
            if let RunEvent::RequestFinished { result, .. } = &event {
                if let Some(record) = &record_har {
                    record(result);
                }
                show.request_finished(result);
                if let Some(log) = &log {
                    drain(log, &next);
                    for line in set_cookie_lines(result) {
                        eprintln!("{line}");
                    }
                }
                if result.response.as_ref().is_some_and(|r| r.truncated()) {
                    eprintln!("body truncated at {cap_text} (raise with --max-body-bytes)");
                }
                let end = result.response.as_ref().and_then(|r| r.stream_end());
                if let Some(StreamEnd::Bound(b)) = end {
                    let what = if *b == StreamBound::Time {
                        &time
                    } else {
                        &events
                    };
                    eprintln!("stream ended at the limit ({what})");
                }
            }
        })
    };
    // Zero lifts a bound; the core default would otherwise apply.
    let limits = StreamLimits::new()
        .max_duration((args.stream_max_time > 0).then(|| Duration::from_secs(args.stream_max_time)))
        .max_events((args.stream_max_events > 0).then_some(args.stream_max_events));
    let mut options = RunOptions::new()
        .delay(Duration::from_millis(args.delay))
        .retries(args.retries)
        .retry_delay(Duration::from_millis(args.retry_delay))
        .retry_unsafe(args.retry_unsafe)
        .fail_on_status(args.fail_on_status)
        .stream(limits);
    if let Some(resolver) = body_files {
        options = options.body_files(resolver);
    }
    if let Some(data) = data {
        options = options.iterations(data);
    }
    if let Some(n) = args.iterations {
        options = options.iteration_count(n as usize);
    }
    let mut report = apinata_core::run(
        target.plan,
        target.ctx,
        executor,
        options
            .bail(args.bail)
            .scripts(
                ScriptOptions::new()
                    .enabled(!args.no_scripts)
                    .timeout(Duration::from_secs(args.script_timeout)),
            )
            .execution_mode(ExecutionMode::Cli)
            .authorize(AuthorizeOptions::new().refetch(args.refetch_tokens)),
        CancelHandle::new(),
        on_event,
    )
    .await;
    if let Some(log) = &log {
        drain(log, &next);
    }
    // Saved after a failed run too: the cookies it collected are still wanted next time.
    if let Some((jar, path)) = &jar {
        match jar.write_file(path) {
            Ok(()) => eprintln!(
                "cookies saved to {}; the file holds session secrets, keep it out of git",
                path.display()
            ),
            Err(e) => {
                eprintln!("error: {e}");
                write_failed.store(true, Ordering::Relaxed);
            }
        }
    }
    // A failed save leaves the run's report as it was and only changes the exit code.
    if let Some(example) = &example
        && let Err(e) = crate::example_cmd::save(example, &report)
    {
        eprintln!("error: {e:#}");
        write_failed.store(true, Ordering::Relaxed);
    }
    if let Some(har) = &har
        && let Err(e) = har.write(&report)
    {
        eprintln!("error: {e:#}");
        write_failed.store(true, Ordering::Relaxed);
    }
    // A request whose body the filter could not read counts as errored in the report.
    let unreadable = unfiltered.lock().map(|l| l.clone()).unwrap_or_default();
    for result in &mut report.results {
        if unreadable.contains(&(Arc::as_ptr(result) as usize))
            && let Some(result) = Arc::get_mut(result)
        {
            result.outcome = apinata_core::Outcome::Errored;
            result.error = Some(apinata_core::RunError::Refused {
                message: "cannot apply --filter: the body is not JSON or XML".to_owned(),
            });
        }
    }
    let root = target.collection.root();
    let load_errors = report
        .load_errors
        .iter()
        .map(|e| (relative(root, e.path()), e.message().to_owned()))
        .collect::<Vec<_>>();
    let rows = || report.results.iter().map(|r| r.as_ref());
    // The core words are neutral; `run` has flags that fix an untrusted certificate.
    let results =
        ResultsReport::new(load_errors, rows()).with_connect_advice(|cause| match cause {
            ConnectCause::TlsUnknownIssuer => Some("Try --cacert <file> or --insecure.".to_owned()),
            _ => None,
        });
    // Only `--show-response` with the JSON report puts the response into it.
    Ok(
        if args.show_response && matches!(args.format, crate::Format::Json) {
            results.with_responses(rows())
        } else {
            results
        },
    )
}

/// Print every network entry from `next` on to stderr and move `next` past them.
fn drain(log: &SessionLog, next: &AtomicU64) {
    let slice = log.network_since(next.load(Ordering::Relaxed));
    for entry in &slice.entries {
        eprintln!("{}", verbose_line(entry));
    }
    next.store(slice.next, Ordering::Relaxed);
}

/// One `--verbose` line: `[source] METHOD url status-or-error[ peer] Nms[ <version>]`, the peer
/// being the last hop's address (`proxy host:port` through a proxy) when known, the version
/// only for a non-HTTP/1.1 last hop; never a body or header.
fn verbose_line(entry: &NetworkEntry) -> String {
    // A multi-line error, or a capped field's truncation marker, would break the
    // one-line-per-exchange shape, so every printed text field is folded onto one line.
    let flat = |text: &str| text.replace(['\r', '\n'], " ");
    let outcome = match (entry.status, &entry.error) {
        (Some(status), _) => status.to_string(),
        (None, Some(error)) => format!("error: {}", flat(error)),
        (None, None) => "error".to_owned(),
    };
    // Only a non-1.1 last hop is marked, so HTTP/1.1 lines keep their old shape.
    let version = match entry.hops.last().and_then(|h| h.version()) {
        Some(v) if v != HttpVersion::Http11 => format!(" {v}"),
        _ => String::new(),
    };
    // The peer sits before the time so every line still ends in its time (or version).
    let peer = match entry.hops.last().and_then(|h| h.peer_label()) {
        Some(label) => format!(" {label}"),
        None => String::new(),
    };
    // A host override the last hop used follows the peer, so the line still ends in its time.
    let mapped = match entry.hops.last().and_then(|h| h.override_from.as_deref()) {
        Some(line) => format!(" (override {})", flat(line)),
        None => String::new(),
    };
    format!(
        "[{}] {} {} {}{peer}{mapped} {}ms{version}",
        entry.source.as_str(),
        flat(&entry.method),
        flat(&entry.url),
        outcome,
        entry.duration.as_millis()
    )
}

/// One `--verbose` line per cookie set for `result`, in the order it was set, as core words
/// it (never the value), indented under the request's line.
fn set_cookie_lines(result: &RequestResult) -> Vec<String> {
    (result.set_cookie_lines().into_iter())
        .map(|line| format!("  {line}"))
        .collect()
}

/// The rows of the `--csv-file-path` or `--json-file-path` file, if either was given. A relative
/// path resolves from the working directory; an unreadable or malformed file fails the run.
fn data_file(args: &RunArgs) -> anyhow::Result<Option<IterationData>> {
    let (path, parse): (_, fn(&str) -> _) = match (&args.csv_file_path, &args.json_file_path) {
        (Some(path), _) => (path, IterationData::from_csv),
        (None, Some(path)) => (path, IterationData::from_json),
        (None, None) => return Ok(None),
    };
    let context = || format!("cannot read data file {}", path.display());
    let text = std::fs::read_to_string(path).with_context(context)?;
    Ok(Some(parse(&text).with_context(context)?))
}

/// `path` relative to the collection root, with `/` separators on every platform.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use crate::{Cli, Command};
    use apinata_core::{MemoryStore, SecretError, SecretKey, SecretStore, resolve};
    use clap::Parser;

    const COLLECTION: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";
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
            unreachable!("run never writes secrets")
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            unreachable!("run never deletes secrets")
        }
    }

    /// A collection whose `dev` environment holds secret `tok`; `uses_tok.yml` reads it and
    /// `plain.yml` does not.
    fn collection() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let write = |rel: &str, text: &str| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        };
        let request = |url: &str| {
            format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
        };
        write("opencollection.yml", COLLECTION);
        write("environments/dev.yml", DEV);
        write("uses_tok.yml", &request("http://127.0.0.1:1/{{tok}}"));
        write("plain.yml", &request("http://127.0.0.1:1/plain"));
        dir
    }

    fn args(dir: &Path, file: &str, extra: &[&str]) -> RunArgs {
        let target = dir.join(file);
        let mut argv = vec!["apinata", "run", target.to_str().expect("utf-8")];
        argv.extend(["--env", "dev"]);
        argv.extend(extra);
        match Cli::try_parse_from(argv).expect("parses").command {
            Command::Run(args) => *args,
            _ => unreachable!("a run command"),
        }
    }

    /// The store holding `value` for `tok` of `dev` in the collection at `dir`.
    fn stored(dir: &Path, value: &str) -> Arc<MemoryStore> {
        let c = Collection::open(dir).expect("opens");
        let store = Arc::new(MemoryStore::new());
        store
            .set(&c.secret_key("dev", "tok"), value)
            .expect("memory store");
        store
    }

    /// The URL of request file `file` once its variables are resolved.
    fn url_of(target: &Target, file: &str) -> Result<String, apinata_core::ResolveError> {
        let req = target
            .collection
            .tree()
            .requests()
            .iter()
            .find(|r| r.path().ends_with(file))
            .expect("the request exists");
        resolve(req, &target.ctx).map(|r| r.url().to_owned())
    }

    #[test]
    fn a_stored_secret_value_resolves() {
        let dir = collection();
        let store = stored(dir.path(), "from-store");
        let target = open_with(&args(dir.path(), "uses_tok.yml", &[]), store).expect("opens");
        assert_eq!(
            url_of(&target, "uses_tok.yml").unwrap(),
            "http://127.0.0.1:1/from-store"
        );
    }

    #[test]
    fn env_var_beats_the_stored_value() {
        let dir = collection();
        let store = stored(dir.path(), "from-store");
        let run = args(dir.path(), "uses_tok.yml", &["--env-var", "tok=from-flag"]);
        let target = open_with(&run, store).expect("opens");
        assert_eq!(
            url_of(&target, "uses_tok.yml").unwrap(),
            "http://127.0.0.1:1/from-flag"
        );
    }

    #[test]
    fn no_keychain_ignores_the_store() {
        let dir = collection();
        let store = stored(dir.path(), "from-store");
        let run = args(dir.path(), "uses_tok.yml", &["--no-keychain"]);
        let target = open_with(&run, store).expect("opens");
        let err = url_of(&target, "uses_tok.yml").expect_err("the secret has no value");
        assert!(
            err.cli_hint().expect("hint").contains("with no value"),
            "{err:?}"
        );
    }

    #[test]
    fn an_unreachable_store_does_not_stop_a_run_that_needs_no_secret() {
        let dir = collection();
        let target =
            open_with(&args(dir.path(), "plain.yml", &[]), Arc::new(Refusing)).expect("opens");
        assert_eq!(
            url_of(&target, "plain.yml").unwrap(),
            "http://127.0.0.1:1/plain"
        );
    }

    #[test]
    fn an_unreachable_store_is_named_in_the_hint_of_a_secret_that_is_needed() {
        let dir = collection();
        let target =
            open_with(&args(dir.path(), "uses_tok.yml", &[]), Arc::new(Refusing)).expect("opens");
        let hint = url_of(&target, "uses_tok.yml")
            .expect_err("undefined")
            .cli_hint()
            .expect("hint");
        assert!(hint.contains("no session bus"), "{hint}");
        assert!(hint.contains("app's environment editor"), "{hint}");
    }

    /// A store whose calls never return.
    struct Blocking;

    impl SecretStore for Blocking {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            loop {
                std::thread::park();
            }
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            unreachable!("run never writes secrets")
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            unreachable!("run never deletes secrets")
        }
    }

    #[test]
    fn a_hung_keychain_gives_the_store_hint_after_the_timeout() {
        let dir = collection();
        let store = crate::import::bounded_keychain(
            Arc::new(Blocking),
            std::time::Duration::from_millis(100),
        );
        let target = open_with(&args(dir.path(), "uses_tok.yml", &[]), store).expect("opens");
        let started = std::time::Instant::now();
        let hint = url_of(&target, "uses_tok.yml")
            .expect_err("undefined")
            .cli_hint()
            .expect("hint");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(hint.contains("no answer within"), "{hint}");
        assert!(hint.contains("app's environment editor"), "{hint}");
    }
}
