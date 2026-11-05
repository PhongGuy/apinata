//! `apinata graphql schema`: fetch a GraphQL request's schema by introspection through core and
//! print it as the JSON answer or as SDL.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, anyhow};
use apinata_core::{
    CancelHandle, ContextOptions, ExecOptions, Executor, IntrospectOptions, NamedSecret,
    OAuth2Error, SCHEMA_BODY_LIMIT, SchemaError, introspect, known_secrets, replace_secrets,
};
use clap::{Subcommand, ValueEnum};

use crate::import::secret_store;
use crate::{EXIT_USAGE, EnvVar, NetworkArgs, network, parse_env_var, run};

/// Exit code for an answer that holds no schema.
const EXIT_NO_SCHEMA: u8 = 1;
/// Exit code for a request that got no response.
const EXIT_NO_RESPONSE: u8 = 2;

#[derive(clap::Args)]
pub struct GraphqlArgs {
    #[command(subcommand)]
    pub action: GraphqlAction,
}

#[derive(Subcommand)]
pub enum GraphqlAction {
    /// Fetch a GraphQL request's schema by introspection and print it.
    ///
    /// The request is resolved as `apinata run` would send it (environment, variables, auth,
    /// headers, proxy and TLS settings), then the standard introspection query is POSTed to its
    /// URL.
    Schema(Box<SchemaArgs>),
}

#[derive(clap::Args)]
pub struct SchemaArgs {
    /// The GraphQL request file.
    pub path: PathBuf,
    /// The environment to use, by file name without `.yml`.
    #[arg(long)]
    pub env: Option<String>,
    /// Set or override an environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the value
    /// from the process environment (empty if set but empty; kept secret).
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
    /// Do not read secret values from the platform keychain.
    #[arg(long)]
    pub no_keychain: bool,
    /// What to print: the introspection answer as received (json) or the schema as SDL.
    #[arg(long, value_enum, default_value_t = SchemaFormat::Json)]
    pub format: SchemaFormat,
    /// Write the schema to this file instead of stdout, unredacted.
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Accept invalid TLS certificates.
    #[arg(long)]
    pub insecure: bool,
    /// Cap the request's timeout at this many whole seconds.
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..=86_400))]
    pub timeout: Option<u64>,
    #[command(flatten)]
    pub network: NetworkArgs,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SchemaFormat {
    Json,
    Sdl,
}

pub fn run(args: &GraphqlArgs) -> ExitCode {
    match &args.action {
        GraphqlAction::Schema(args) => ExitCode::from(schema(args)),
    }
}

/// A failure with its exit code; the message is printed redacted.
struct Failure {
    code: u8,
    message: String,
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Failure {
        Failure {
            code: EXIT_USAGE,
            message: format!("{e:#}"),
        }
    }
}

fn schema(args: &SchemaArgs) -> u8 {
    let mut secrets = Vec::new();
    match fetch(args, &mut secrets) {
        Ok(()) => 0,
        Err(Failure { code, message }) => {
            eprintln!("error: {}", replace_secrets(&message, &secrets));
            code
        }
    }
}

/// Fetch and print the schema, filling `secrets` with the values to hide from any message.
fn fetch(args: &SchemaArgs, secrets: &mut Vec<NamedSecret>) -> Result<(), Failure> {
    let path = std::fs::canonicalize(&args.path)
        .with_context(|| format!("cannot open {}", args.path.display()))?;
    if path.is_dir() {
        return Err(anyhow!("graphql schema takes one request file").into());
    }
    let options = ContextOptions::new()
        .client_certificates(network::client_certificates(&args.network)?)
        .secrets(secret_store(args.no_keychain));
    let target = run::open_at(
        &path,
        args.env.as_deref(),
        &args.env_var,
        &BTreeSet::new(),
        options,
    )?;
    let Some(file) = target.plan.requests().next() else {
        let error = target.plan.load_errors().first();
        let message = error.map_or("the file holds no request", |e| e.message());
        return Err(anyhow!("{message}").into());
    };
    // Unresolvable requests report their own error below; hiding nothing is then correct.
    *secrets = known_secrets(file, &target.ctx).unwrap_or_default();
    let mut exec = ExecOptions::new()
        .insecure(args.insecure)
        .network(network::settings(&args.network))
        .http_version(args.network.http_version)
        .timeout_cap(args.timeout.map(Duration::from_secs))
        .max_body_bytes(Some(SCHEMA_BODY_LIMIT));
    if let Some(check) = crate::network_notice::check() {
        exec = exec.network_check(check);
    }
    let executor = Executor::new(exec).map_err(|e| anyhow!(e))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow!(e))?;
    let answer = runtime
        .block_on(introspect(
            &executor,
            file,
            &target.ctx,
            &IntrospectOptions::new(),
            &CancelHandle::new(),
        ))
        .map_err(failure)?;
    let mut text = match args.format {
        SchemaFormat::Json => answer.text().to_owned(),
        SchemaFormat::Sdl => answer.schema().to_sdl(),
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    match &args.output {
        Some(out) => {
            std::fs::write(out, &text).with_context(|| format!("cannot write {}", out.display()))?
        }
        None => out!("{}", replace_secrets(&text, secrets)),
    }
    Ok(())
}

/// The exit code a schema error ends with: 1 for an answer without a schema, 2 for no answer.
fn failure(e: SchemaError) -> Failure {
    let code = match &e {
        SchemaError::Status { .. }
        | SchemaError::TooLarge { .. }
        | SchemaError::NotJson { .. }
        | SchemaError::NoData
        | SchemaError::Graphql { .. }
        | SchemaError::Invalid { .. } => EXIT_NO_SCHEMA,
        SchemaError::Send(_) | SchemaError::Auth(OAuth2Error::Exec(_)) => EXIT_NO_RESPONSE,
        _ => EXIT_USAGE,
    };
    let message = match &e {
        SchemaError::Send(e) | SchemaError::Auth(OAuth2Error::Exec(e)) => {
            e.hinted_line().unwrap_or_else(|| e.to_string())
        }
        other => other.to_string(),
    };
    Failure { code, message }
}
