//! Apinata core: reads and writes OpenCollection YAML collections and resolves their
//! requests against collection, environment, folder and process variables, then sends them.
//! All logic lives here; the CLI, MCP server and app are thin shells over it.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod approval;
mod assertion;
mod aws_profile;
mod body_filter;
mod cancel;
pub mod check;
mod codegen;
mod collection;
mod compare;
mod config_file;
mod connect_hint;
mod context;
mod cookies;
mod credential_names;
mod curl_export;
mod decomment;
mod default_headers;
mod digest;
mod digest_key;
mod document;
mod dotenv;
mod dynamic;
mod dynamic_data;
mod edgegrid;
mod edit;
mod encode_url;
mod env_file;
mod env_ops;
mod env_overlay;
mod environment;
mod error;
mod example;
mod execute;
pub mod export;
mod git;
mod global_env;
mod graphql_query;
mod graphql_schema;
mod graphql_variables;
#[cfg(test)]
mod hang_guard;
mod header_check;
mod header_hints;
mod hidden_char;
mod host_guard;
mod http_version;
pub mod import;
mod interpolate;
pub mod item_ops;
mod item_path;
mod iteration_data;
mod js;
mod json_filter;
mod json_node;
mod json_syntax;
pub mod json_text;
mod jwt;
#[cfg(feature = "keychain")]
mod keychain;
mod kind_convert;
#[cfg(feature = "keychain")]
mod lazy_keychain;
pub mod legacy;
mod list_items;
mod method;
mod mime_table;
pub mod mock;
mod multipart;
mod naming;
mod network;
mod not_found;
mod ntlm;
mod oauth1;
mod oauth2;
mod opaque;
mod order;
mod paging;
mod post_response;
mod presets;
mod prompt;
mod replace;
mod request;
mod request_edit;
mod resolve;
mod response;
pub mod response_view;
mod results_report;
mod retry;
mod runner;
mod runtime_vars;
mod schema_test;
mod script;
mod script_snippets;
mod script_trust;
mod sealed;
mod search;
mod secret_forms;
mod secret_names;
mod secrets;
mod sequence;
mod session_log;
mod sigv4;
pub mod sse;
mod stream;
mod system_trash;
mod tags;
mod timing;
mod trash;
mod trash_log;
mod trust;
#[cfg(feature = "update")]
pub mod update;
mod url_params;
mod url_password;
mod variable_set;
pub mod watch;
mod workspace_file;
mod wsse;
mod xml_filter;
mod yaml;

pub use approval::{Approval, ApprovalInbox, DISPLAY_MAX_CHARS, Filing, ScriptApproval};
pub use assertion::{Assertion, AssertionOutcome, Verdict, assertion_sensitive_values, evaluate};
pub use aws_profile::AwsSharedFiles;
pub use body_filter::{BodyFilterError, filter_body, validate_filter};
pub use cancel::CancelHandle;
pub use check::{
    CheckError, CheckOptions, CheckReport, Finding, FindingKind, Severity, check_collection,
};
pub use codegen::{Target, UnknownTarget, generate};
pub use collection::{
    Change, ChangeKind, Collection, DraftSnapshot, DropTarget, Environment, FileTexts, Folder,
    IgnorePolicy, ItemKind, OpenOptions, PasteSource, Sibling, SnapshotError, Trashed,
    collection_name, dotenv_quotable,
};
pub use compare::{
    BodyComparison, BodyState, COMPARE_BODY_LIMIT, Comparison, HeaderChange, HeaderChangeKind,
    JsonChange, JsonChangeKind, ResponseSnapshot, Side, StatusChange, StatusLine,
    UnavailableReason, compare,
};
pub use config_file::{CollectionFile, FolderFile};
pub use connect_hint::ConnectHint;
pub use context::{ContextOptions, SecretReadFailure, VarContext};
pub use cookies::{
    CookieError, CookieInfo, CookieJar, CookieOutcome, CookieRejection, SetCookie,
    dropped_cookies_notice,
};
pub use credential_names::{
    is_credential_name, mask_credential_query, mask_credential_shape, mask_credential_value,
};
pub use curl_export::to_curl;
pub use default_headers::{DefaultHeader, default_headers};
pub use digest_key::DigestKey;
pub use document::DiskState;
pub use dynamic::dynamic_variables;
pub use edit::{
    ApiKeyPlacement, AuthJsonError, AuthJsonPosition, AuthSpec, BodySpec, ClientAuth, EdgeGridSpec,
    FileEntry, KeyValue, MultipartField, MultipartValue, OAuth1KeyKind, OAuth1Placement,
    OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec, OAuth2Flow, OAuth2Param,
    OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage, Param, ParamKind, RequestSettings, Timeout,
    TokenPlacement, TokenSource, VarValue, Variable, collection_relative, parse_auth_json,
};
// The auth JSON parser's two paths, for the test that holds them equal.
#[doc(hidden)]
pub use edit::auth_json_paths;
pub use env_file::EnvVariable;
pub use env_ops::{
    MovedRoot, OrphanReport, OrphanedSecret, VariableRefusal, check_environment_name,
    without_variable,
};
pub use env_overlay::{EnvOverlay, OverlayLayer, OverlayVar};
pub use error::{
    ContextError, CreateError, EditError, EnvError, FileError, HintShell, OpenError, ResolveError,
    SaveError, TreeError, UndefinedHint,
};
pub use example::{
    Example, ExampleBody, ExampleBodyKind, ExampleRequest, ExampleResponse, ScrubReport, Scrubbed,
    ScrubbedExample, scrub_example_userinfo,
};
pub use execute::{ConnectCause, ExecError, ExecOptions, Executor, NtlmCause};
pub use git::{
    Branch, CancelFlag, CloneError, CloneProgress, CloneRequest, CloneUrlKind, CommitError,
    CommitOutcome, DiffLine, FetchError, FileDiff, FileStatus, GIT_FILE_CAP, GIT_TIMEOUT, GitError,
    GitOptions, GitState, Hunk, LineKind, Pulled, Pushed, RemoteBranch, RepoStatus, Section,
    StageState, SwitchError, Switched, SyncError, Upstream, branches, clone_repo,
    collection_status, collection_status_with, commit, default_folder_name, fetch, file_diff,
    file_diff_with, pull, push, redact_url, redact_url_userinfo, remote_branches, remotes, stage,
    switch_branch, switch_to_remote, text_hunks, unstage, validate_clone_url, validate_folder_name,
};
pub use global_env::GlobalEnvironments;
pub use graphql_query::{
    CompiledSchema, MAX_FORMAT_OUTPUT_BYTES, QUERY_DIAGNOSTIC_LIMIT, QUERY_FORMAT_LIMIT,
    QUERY_VALIDATE_LIMIT, QueryCheck, QueryDiagnostic, QuerySeverity, QuerySyntaxError,
    SCHEMA_SKIP_NOTE, SchemaUse, check_query, format_query, validate_query,
};
pub use graphql_schema::{
    Completion, CompletionKind, EnumValue, Field, INTROSPECTION_QUERY, InputValue, IntrospectError,
    IntrospectGuard, IntrospectOptions, Introspection, SCHEMA_BODY_LIMIT, SCHEMA_TIMEOUT, Schema,
    SchemaError, SchemaSymbol, SchemaType, SymbolAt, TypeKind, TypeRef, fetch_introspection,
    fetch_schema, introspect, introspection_request,
};
pub use graphql_variables::{
    VariableCompletion, VariableCompletionKind, VariableDefinition, variable_completions,
    variable_definitions,
};
pub use header_check::{HeaderNameProblem, invalid_header_name};
pub use header_hints::{header_names, header_values};
pub use hidden_char::{is_hidden_char, is_variation_selector};
pub use http_version::{HttpVersion, HttpVersionPolicy};
pub use interpolate::{placeholders, unclosed_placeholder};
pub use item_ops::{
    FolderPatch, FolderSections, FolderUpdate, ItemError, NewRequest, Restored, RestoredAll,
};
pub use item_path::ItemPath;
pub use iteration_data::{DataFileError, IterationData, iteration_total};
pub use js::script_code;
pub use json_filter::{FilterError, JsonFilter, ParsedJson, filter_json};
pub use json_node::{JsonNode, json_node_at};
pub use json_syntax::{JsonSyntaxError, json_syntax_error};
pub use jwt::{DecodedJwt, Expiry, JwtError, decode_jwt};
#[cfg(feature = "keychain")]
pub use keychain::KeychainStore;
pub use kind_convert::{ConversionPreview, ConvertError, KindConversion};
#[cfg(feature = "keychain")]
pub use lazy_keychain::{KEYCHAIN_TIMEOUT, LazyKeychain, TimeoutStore};
pub use list_items::{ItemList, ItemPage, ListItemsError, ListOptions, ListedItem, ListedKind};
pub use method::{InvalidMethod, normalize_method};
pub use multipart::{Part, PartValue};
pub use network::{
    AckState, CertPath, CertificateFiles, CertificateKind, ClientCertificate, CollectionNetwork,
    CollectionProxy, GlobalProxy, HostOverride, NetworkAck, NetworkAnswer, NetworkChange,
    NetworkCheck, NetworkQuestion, NetworkSettings, NetworkSnapshot, PacSource, PacSourceError,
    PacWarning, ProxyAuth, ProxyConfig, QuestionKind, RequestNetwork, RequestProxy,
    ResolvedCertificate, ResolvedHostOverride, ResolvedProxy, ShownNetwork, clear_pac_cache,
    saved_network,
};
pub use oauth2::{
    AuthorizationPrompt, AuthorizeCancelled, AuthorizeOptions, Authorizer, OAuth2Error,
    OAuth2Target, TokenCache, TokenStatus, check_redirect, clear_oauth2_token, is_oauth2,
    oauth2_field_label, oauth2_name_hidden, oauth2_status,
};
pub use opaque::OpaqueItem;
pub use paging::{MAX_PAGE_LIMIT, PageError};
pub use post_response::{
    PostResponseVar, VarOutcome, VarResult, apply_post_response_vars, variable_name_problem,
};
pub use presets::CollectionPresets;
pub use prompt::{prompt_label, prompt_labels};
pub use replace::{ReplaceReport, ReplaceScope};
pub use request::{RequestFile, RequestKind};
pub use request_edit::QueryMismatch;
pub use resolve::{
    AuthSource, Body, HeaderSource, InheritedAuth, InheritedHeader, PreparedRequest,
    ResolvedRequest, Warning, prepare, resolve, resolve_prepared,
};
pub use response::{CookieOrigin, HOP_BODY_LIMIT, Hop, Response};
pub use results_report::ResultsReport;
pub use runner::{
    Outcome, RequestResult, RunError, RunEvent, RunOptions, RunPlan, RunReport, SelectError,
    StatusRule, Summary, run, send_one,
};
pub use runtime_vars::{RuntimeVar, RuntimeVars};
pub use schema_test::schema_test_script;
pub use script::{
    ConsoleEntry, ConsoleLevel, ExecutionMode, ScriptOptions, ScriptPhase, ScriptText,
    ScriptTextKind, TestOutcome, plan_script_texts, plan_script_texts_across, script_texts,
};
pub use script_snippets::{Snippet, SnippetPhase, script_snippets};
pub use script_trust::{ScriptDecision, ScriptTrust, script_hash};
pub use sealed::Persistence;
pub use search::{
    SEARCH_HIT_CAP, SearchError, SearchField, SearchHit, SearchOptions, SearchPage, SearchResults,
};
pub use secret_forms::{FormKind, FormPolicy, WireForm, host_forms, replace_ascii_ci, wire_forms};
pub use secret_names::{
    NamedSecret, base64_fragment_forms, bytes_hold_secret, derived_secrets, encoded_forms,
    is_credential_header, known_secrets, mask_response_header, named_secrets, redact_base64_runs,
    replace_base64_fragment, replace_secrets, request_secrets,
};
pub use secrets::{EnvWarning, MemoryStore, SecretError, SecretKey, SecretStore};
pub use session_log::{
    ConsoleLine, LogLimits, LogSlice, LoggedBody, LoggedHop, NetworkEntry, SendSource, SessionLog,
};
pub use stream::{StreamBound, StreamEnd, StreamEvent, StreamLimits, StreamOptions};
pub use system_trash::SystemTrash;
pub use tags::{TagFilter, normalize_tags};
pub use timing::{ConnectKind, HopTiming};
pub use trash::{Trash, TrashError, TrashHandle};
pub use trash_log::TrashLog;
pub use trust::{
    TrustError, TrustStore, bare_host_refusal, display_host, is_loopback, normalize_bare_host,
    normalize_host,
};
pub use url_password::{text_without_password, url_without_password};
pub use variable_set::{ScopedVariable, VarScope, VariableInfo, VariableSet};
pub use workspace_file::{
    WorkspaceCollection, WorkspaceError, WorkspaceFile, WorkspaceSpec, is_workspace_dir,
};
pub use xml_filter::{ParsedXml, XmlFilter, filter_xml};
