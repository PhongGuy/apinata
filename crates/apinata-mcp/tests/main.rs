//! The apinata-mcp integration suite.
//!
//! Each file beside this one is a module of this single binary, so the
//! suite links once. A new test file must be declared below, or it never runs.

mod common;
#[path = "../../apinata-core/tests/git_support/mod.rs"]
mod git_support;
#[path = "../../apinata-core/tests/network_support/mod.rs"]
mod network_support;
#[path = "../../apinata-core/tests/ntlm_support/mod.rs"]
mod ntlm_support;

mod argument_errors;
mod bodies;
mod body_filter;
mod check;
mod cookies;
mod derived_credentials;
mod empty_url;
mod env_not_found;
mod env_ops;
mod generate_code;
mod git_tools;
mod global_env;
mod graphql_requests;
mod graphql_schema;
mod harness;
mod host_overrides;
mod http_version;
mod import_collection;
mod import_request;
mod leak;
mod list_items;
mod mock_tools;
mod network;
mod oauth2;
mod oauth2_token;
mod output_schemas;
mod post_response_vars;
mod prompt_vars;
mod prompts;
mod read_config;
mod read_request;
mod refusals;
mod remote_addr;
mod replace;
mod resource_notify;
mod resource_safety;
mod resources;
mod restore_item;
mod run_fail_on_status;
mod run_folder;
mod run_retry;
mod run_timeout;
mod runtime_set;
mod save_example;
mod scripts;
mod scripts_run;
mod scrub_userinfo;
mod search;
mod send_graphql;
mod send_request;
mod send_set_cookies;
mod send_stream;
mod send_url_headers;
mod serde_defaults;
mod tree_tools;
mod trusted_hosts;
mod undefined_hint;
mod update_environment;
mod update_folder;
mod update_request;
