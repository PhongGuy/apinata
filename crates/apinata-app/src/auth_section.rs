//! The Auth section: a mode picker over the inputs of the modes the editor models.

use std::collections::HashMap;
use std::sync::Arc;

use apinata_core::{
    ApiKeyPlacement, AuthSource, AuthSpec, Collection, EdgeGridSpec, InheritedAuth, OAuth1Spec,
    OAuth2Flow, OAuth2Spec, VariableSet,
};
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::command::Edit;
use crate::kv_table::set_text;
use crate::oauth1_editor::{OAuth1Edit, OAuth1Editor};
use crate::oauth2_editor::{OAuth2Edit, OAuth2Editor, OAuth2MoveSecrets};
use crate::section::SectionEdit;
use crate::var_tokens::VarField;
use crate::{style, ui};

/// An auth mode the editor models, in picker order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum AuthMode {
    None,
    Inherit,
    Basic,
    Bearer,
    ApiKey,
    Digest,
    Ntlm,
    AwsV4,
    Wsse,
    EdgeGrid,
    OAuth1,
    OAuth2,
}

impl AuthMode {
    /// Every mode, in picker order.
    pub const ALL: [AuthMode; 12] = [
        AuthMode::None,
        AuthMode::Inherit,
        AuthMode::Basic,
        AuthMode::Bearer,
        AuthMode::ApiKey,
        AuthMode::Digest,
        AuthMode::Ntlm,
        AuthMode::AwsV4,
        AuthMode::Wsse,
        AuthMode::EdgeGrid,
        AuthMode::OAuth1,
        AuthMode::OAuth2,
    ];

    /// The picker's label.
    pub fn label(self) -> &'static str {
        match self {
            AuthMode::None => "No Auth",
            AuthMode::Inherit => "Inherit",
            AuthMode::Basic => "Basic Auth",
            AuthMode::Bearer => "Bearer Token",
            AuthMode::ApiKey => "API Key",
            AuthMode::Digest => "Digest Auth",
            AuthMode::Ntlm => "NTLM",
            AuthMode::AwsV4 => "AWS Sig v4",
            AuthMode::Wsse => "WSSE",
            AuthMode::EdgeGrid => "Akamai EdgeGrid",
            AuthMode::OAuth1 => "OAuth 1.0",
            AuthMode::OAuth2 => "OAuth 2.0",
        }
    }

    /// The mode of `auth`; `None` for a mode the editor does not model.
    pub fn of(auth: &AuthSpec) -> Option<AuthMode> {
        Some(match auth {
            AuthSpec::None => AuthMode::None,
            AuthSpec::Inherit => AuthMode::Inherit,
            AuthSpec::Basic { .. } => AuthMode::Basic,
            AuthSpec::Bearer { .. } => AuthMode::Bearer,
            AuthSpec::ApiKey { .. } => AuthMode::ApiKey,
            AuthSpec::Digest { .. } => AuthMode::Digest,
            AuthSpec::Ntlm { .. } => AuthMode::Ntlm,
            AuthSpec::AwsV4 { .. } => AuthMode::AwsV4,
            AuthSpec::Wsse { .. } => AuthMode::Wsse,
            AuthSpec::EdgeGrid(_) => AuthMode::EdgeGrid,
            AuthSpec::OAuth1(_) => AuthMode::OAuth1,
            AuthSpec::OAuth2(_) => AuthMode::OAuth2,
            _ => return None,
        })
    }

    fn empty(self) -> AuthSpec {
        match self {
            AuthMode::None => AuthSpec::None,
            AuthMode::Inherit => AuthSpec::Inherit,
            AuthMode::Basic => AuthSpec::Basic {
                username: String::new(),
                password: String::new(),
            },
            AuthMode::Bearer => AuthSpec::Bearer {
                token: String::new(),
            },
            AuthMode::ApiKey => AuthSpec::ApiKey {
                key: String::new(),
                value: String::new(),
                placement: ApiKeyPlacement::Header,
            },
            AuthMode::Digest => AuthSpec::Digest {
                username: String::new(),
                password: String::new(),
            },
            AuthMode::Ntlm => AuthSpec::Ntlm {
                username: String::new(),
                password: String::new(),
                domain: String::new(),
            },
            AuthMode::AwsV4 => AuthSpec::AwsV4 {
                access_key_id: String::new(),
                secret_access_key: String::new(),
                session_token: String::new(),
                service: String::new(),
                region: String::new(),
                profile_name: String::new(),
            },
            AuthMode::Wsse => AuthSpec::Wsse {
                username: String::new(),
                password: String::new(),
            },
            AuthMode::EdgeGrid => AuthSpec::EdgeGrid(EdgeGridSpec::default()),
            AuthMode::OAuth1 => AuthSpec::OAuth1(OAuth1Spec::default()),
            AuthMode::OAuth2 => AuthSpec::OAuth2(OAuth2Spec::new(OAuth2Flow::AuthorizationCode)),
        }
    }
}

/// One of the section's text inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum AuthField {
    Username,
    Password,
    Domain,
    Token,
    Key,
    Value,
    AccessKeyId,
    SecretAccessKey,
    SessionToken,
    Service,
    Region,
    ProfileName,
    AccessToken,
    ClientToken,
    ClientSecret,
    BaseUrl,
    Nonce,
    Timestamp,
    HeadersToSign,
    MaxBodySize,
}

/// The level whose auth the section edits; it picks the No Auth wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(missing_docs)]
pub enum AuthScope {
    #[default]
    Request,
    Folder,
    Collection,
}

impl AuthScope {
    // Folder and collection auth is what their requests inherit, so their wording names them.
    fn no_auth_text(self) -> &'static str {
        match self {
            AuthScope::Request => "This request sends no auth.",
            AuthScope::Folder => "Requests that inherit from this folder send no auth.",
            AuthScope::Collection => "Requests that inherit from the collection send no auth.",
        }
    }
}

// The AWS Sig v4 inputs in `AuthField` order: label, element id, and whether it is masked.
const AWS_FIELDS: [(&str, &str, bool); 6] = [
    ("Access Key ID", "auth-aws-access-key-id", false),
    ("Secret Access Key", "auth-aws-secret-access-key", true),
    ("Session Token", "auth-aws-session-token", true),
    ("Service", "auth-aws-service", false),
    ("Region", "auth-aws-region", false),
    ("Profile Name", "auth-aws-profile-name", false),
];

// The EdgeGrid inputs in `AuthField` order: label, element id, and whether it is masked.
const EDGEGRID_FIELDS: [(&str, &str, bool); 7] = [
    ("Access Token", "auth-eg-access-token", true),
    ("Client Token", "auth-eg-client-token", true),
    ("Client Secret", "auth-eg-client-secret", true),
    ("Base URL", "auth-eg-base-url", false),
    ("Nonce", "auth-eg-nonce", false),
    ("Timestamp", "auth-eg-timestamp", false),
    ("Headers to Sign", "auth-eg-headers-to-sign", false),
];

const PLACEMENTS: [(ApiKeyPlacement, &str); 2] = [
    (ApiKeyPlacement::Header, "Header"),
    (ApiKeyPlacement::Query, "Query Params"),
];

/// The decoded claims shown under the Bearer field; never holds the token's own text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtView {
    /// The header as pretty JSON.
    pub header: String,
    /// The payload as pretty JSON.
    pub payload: String,
    /// The expiry line, marked "expired" once past.
    pub expiry: String,
}

impl JwtView {
    // The clock is read here, once per decode, and handed to core.
    fn of(jwt: &apinata_core::DecodedJwt) -> JwtView {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let date = |at: i64| {
            chrono::DateTime::from_timestamp(at, 0).map_or_else(
                || at.to_string(),
                |d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            )
        };
        let expiry = match jwt.expiry(now) {
            apinata_core::Expiry::Absent => "No expiry".to_owned(),
            apinata_core::Expiry::Valid { expires_at } => format!("Expires {}", date(expires_at)),
            apinata_core::Expiry::Expired { expires_at } => {
                format!("Expires {} (expired)", date(expires_at))
            }
            _ => String::new(),
        };
        JwtView {
            header: jwt.header.clone(),
            payload: jwt.payload.clone(),
            expiry,
        }
    }
}

/// Edits a request's auth. Each mode remembers its last value for the life of the view, so
/// switching back restores it; secrets are typed into masked inputs.
pub struct AuthSection {
    mode: Option<AuthMode>,
    unsupported: Option<String>,
    // What Inherit says it inherits; `None` keeps the generic text.
    inherited: Option<SharedString>,
    modes: Vec<AuthMode>,
    scope: AuthScope,
    select: Entity<SelectState<Vec<SharedString>>>,
    placement_select: Entity<SelectState<Vec<SharedString>>>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    domain: Entity<InputState>,
    token: Entity<InputState>,
    key: Entity<InputState>,
    value: Entity<InputState>,
    // The AWS Sig v4 inputs, in `AWS_FIELDS` order.
    aws: [Entity<InputState>; 6],
    // The EdgeGrid inputs, in `EDGEGRID_FIELDS` order.
    edgegrid: [Entity<InputState>; 7],
    // The EdgeGrid Max Body Size last loaded or validly typed; every EdgeGrid edit carries it.
    edgegrid_max: Option<f64>,
    // Its input has its own subscription, so a refused parse never reaches `emit`.
    max_body_size: Entity<InputState>,
    // Set while the Max Body Size text is not a number; the note shows until it parses.
    max_refused: bool,
    oauth1: Entity<OAuth1Editor>,
    oauth2: Entity<OAuth2Editor>,
    // The API key placement last set or loaded; the base each API key edit builds on.
    placement: ApiKeyPlacement,
    memory: HashMap<AuthMode, AuthSpec>,
    // Counts loads, so an edit can tell whether one came between its sending and delivery.
    loads: u64,
    vars: Option<Arc<VariableSet>>,
    // The Bearer token decoded when it last changed; render never decodes.
    jwt: Option<JwtView>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SectionEdit> for AuthSection {}

impl AuthSection {
    /// A section showing No Auth.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let labels: Vec<SharedString> = AuthMode::ALL.iter().map(|m| m.label().into()).collect();
        let select = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(0)), window, cx));
        let places: Vec<SharedString> = PLACEMENTS.iter().map(|(_, l)| (*l).into()).collect();
        let placement_select =
            cx.new(|cx| SelectState::new(places, Some(IndexPath::new(0)), window, cx));
        let mut input = |masked| cx.new(|cx| InputState::new(window, cx).masked(masked));
        let (username, password, domain, token, key, value) = (
            input(false),
            input(true),
            input(false),
            input(true),
            input(false),
            input(true),
        );
        let aws = AWS_FIELDS.map(|(_, _, masked)| input(masked));
        let edgegrid = EDGEGRID_FIELDS.map(|(_, _, masked)| input(masked));
        let max_body_size = input(false);
        let oauth1 = cx.new(|cx| OAuth1Editor::new(window, cx));
        let oauth2 = cx.new(|cx| OAuth2Editor::new(window, cx));
        let mut subs = vec![
            cx.subscribe(&oauth1, |this, _, OAuth1Edit(spec, stamp), cx| {
                if this.mode == Some(AuthMode::OAuth1) {
                    this.emit_stamped(AuthSpec::OAuth1(spec.clone()), *stamp, cx);
                }
            }),
            // The editor's edit goes out while it is shown, and is remembered for switching back.
            cx.subscribe(&oauth2, |this, _, OAuth2Edit(spec, stamp), cx| {
                if this.mode == Some(AuthMode::OAuth2) {
                    this.emit_stamped(AuthSpec::OAuth2(spec.clone()), *stamp, cx);
                }
            }),
            cx.subscribe(&oauth2, |this, _, _: &OAuth2MoveSecrets, cx| {
                if this.mode == Some(AuthMode::OAuth2) {
                    cx.emit(SectionEdit::new(Edit::MoveAuthSecrets));
                }
            }),
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some(mode) = this
                            .modes
                            .iter()
                            .copied()
                            .find(|m| m.label() == label.as_ref())
                    {
                        this.set_mode(mode, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &placement_select,
                window,
                |this, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some((p, _)) = PLACEMENTS.iter().find(|(_, l)| *l == label.as_ref())
                    {
                        this.set_placement(*p, window, cx);
                    }
                },
            ),
        ];
        // Any keystroke sends the whole spec of the shown mode, rebuilt from the inputs.
        for state in [&username, &password, &domain, &token, &key, &value]
            .into_iter()
            .chain(&aws)
            .chain(&edgegrid)
        {
            subs.push(cx.subscribe(state, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(auth) = this.current(cx)
                {
                    this.emit(auth, cx);
                }
            }));
        }
        // The decoded claims follow the Bearer text as it is typed.
        subs.push(cx.subscribe(&token, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh_jwt(cx);
            }
        }));
        // A Max Body Size edit is sent only when it parses; otherwise it is only flagged.
        subs.push(
            cx.subscribe(&max_body_size, |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                match parse_max_body_size(&state.read(cx).value()) {
                    Some(max) => {
                        this.max_refused = false;
                        this.edgegrid_max = max;
                        if let Some(auth) = this.current(cx) {
                            this.emit(auth, cx);
                        }
                    }
                    None => this.max_refused = true,
                }
                cx.notify();
            }),
        );
        AuthSection {
            mode: Some(AuthMode::None),
            unsupported: None,
            inherited: None,
            modes: AuthMode::ALL.to_vec(),
            scope: AuthScope::default(),
            select,
            placement_select,
            username,
            password,
            domain,
            token,
            key,
            value,
            aws,
            edgegrid,
            edgegrid_max: None,
            max_body_size,
            max_refused: false,
            oauth1,
            oauth2,
            placement: ApiKeyPlacement::Header,
            memory: HashMap::new(),
            loads: 0,
            vars: None,
            jwt: None,
            _subs: subs,
        }
    }

    /// Shows `auth` from a snapshot without emitting an edit; a mode the picker does not
    /// offer is shown read-only, as an unmodelled one is.
    pub fn load(&mut self, auth: &AuthSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.loads += 1;
        // A sub-editor's event is delivered after its emit, maybe after this load, so each edit
        // carries the count from when it was sent.
        let loads = self.loads;
        self.oauth1.update(cx, |e, _| e.set_stamp(loads));
        self.oauth2.update(cx, |e, _| e.set_stamp(loads));
        match AuthMode::of(auth) {
            Some(mode) if self.modes.contains(&mode) => {
                self.show(mode, auth, window, cx);
                // The editor may skip the echo of its own edit; remember what it shows, which
                // still holds the other grants' fields.
                let shown = match mode {
                    AuthMode::OAuth2 => AuthSpec::OAuth2(self.oauth2.read(cx).spec(cx)),
                    _ => auth.clone(),
                };
                self.memory.insert(mode, shown);
            }
            modelled => {
                self.mode = None;
                self.unsupported = Some(match (modelled, auth) {
                    (Some(mode), _) => mode.label().into(),
                    (None, AuthSpec::Unsupported { mode }) => mode.clone(),
                    _ => "unknown".into(),
                });
                self.select
                    .update(cx, |s, cx| s.set_selected_index(None, window, cx));
                cx.notify();
            }
        }
    }

    /// Sets the level whose auth this is; only the No Auth wording changes.
    pub fn set_scope(&mut self, scope: AuthScope, cx: &mut Context<Self>) {
        self.scope = scope;
        cx.notify();
    }

    /// Offers only `modes` in the picker, in the order given; a shown mode left out of
    /// them turns read-only.
    pub fn set_modes(&mut self, modes: &[AuthMode], window: &mut Window, cx: &mut Context<Self>) {
        self.modes = modes.to_vec();
        let labels: Vec<SharedString> = modes.iter().map(|m| m.label().into()).collect();
        self.select
            .update(cx, |s, cx| s.set_items(labels, window, cx));
        match self.mode {
            Some(mode) if modes.contains(&mode) => {
                let label: SharedString = mode.label().into();
                self.select
                    .update(cx, |s, cx| s.set_selected_value(&label, window, cx));
            }
            Some(mode) => {
                let auth = self
                    .memory
                    .get(&mode)
                    .cloned()
                    .unwrap_or_else(|| mode.empty());
                self.load(&auth, window, cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// The modes the picker offers; [`AuthMode::ALL`] until [`AuthSection::set_modes`].
    pub fn modes(&self) -> &[AuthMode] {
        &self.modes
    }

    /// Sets the text Inherit shows, as `inherit_text` builds it; `None` shows the generic text.
    pub fn set_inherited(&mut self, text: Option<SharedString>, cx: &mut Context<Self>) {
        if self.inherited != text {
            self.inherited = text;
            cx.notify();
        }
    }

    /// The mode shown; `None` while the auth's mode is not modelled.
    pub fn mode(&self) -> Option<AuthMode> {
        self.mode
    }

    /// The mode picker's placeholder while an unsupported mode is shown, naming that mode;
    /// `None` while a modelled mode is shown.
    pub fn mode_placeholder(&self) -> Option<String> {
        match (self.mode, &self.unsupported) {
            (None, Some(kind)) => Some(format!("{kind} (read-only)")),
            _ => None,
        }
    }

    /// Switches to `mode`, sending its remembered value or else its empty value.
    pub fn set_mode(&mut self, mode: AuthMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Some(mode) {
            return;
        }
        let auth = self
            .memory
            .get(&mode)
            .cloned()
            .unwrap_or_else(|| mode.empty());
        self.show(mode, &auth, window, cx);
        self.emit(auth, cx);
    }

    /// Sets where the API key goes, sending an edit while the API Key mode is shown.
    pub fn set_placement(
        &mut self,
        placement: ApiKeyPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_placement(placement, window, cx);
        if self.mode == Some(AuthMode::ApiKey)
            && let Some(auth) = self.current(cx)
        {
            self.emit(auth, cx);
        }
    }

    /// Tint and explain placeholders in the unmasked fields (username, API key name); the
    /// masked fields never show them.
    pub fn set_variables(&mut self, vars: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.oauth1
            .update(cx, |e, cx| e.set_variables(vars.clone(), cx));
        self.oauth2
            .update(cx, |e, cx| e.set_variables(vars.clone(), cx));
        self.vars = vars;
        self.refresh_jwt(cx);
        cx.notify();
    }

    /// The decoded claims of the Bearer token, when it is a JWT.
    pub fn jwt_view(&self) -> Option<JwtView> {
        self.jwt.clone()
    }

    // Decodes the Bearer text, through the variables when it names any. Only the claims are
    // kept; the expanded text is dropped inside core.
    fn refresh_jwt(&mut self, cx: &mut Context<Self>) {
        let text = self.token.read(cx).value().to_string();
        let decoded = if self.mode != Some(AuthMode::Bearer) {
            None
        } else if text.contains("{{") {
            self.vars.as_ref().and_then(|v| v.decode_jwt(&text).ok())
        } else {
            apinata_core::decode_jwt(&text).ok()
        };
        self.jwt = decoded.as_ref().map(JwtView::of);
        cx.notify();
    }

    /// Ends the typing run of every auth input's own undo, as at a save or a reload.
    pub fn end_typing(&self, window: &mut Window, cx: &mut App) {
        let own = [
            &self.username,
            &self.password,
            &self.domain,
            &self.token,
            &self.key,
        ];
        let fields = own.into_iter().chain([&self.value, &self.max_body_size]);
        for input in fields.chain(&self.aws).chain(&self.edgegrid) {
            crate::ui::end_input_run(input, window, cx);
        }
        self.oauth1.update(cx, |o, cx| o.end_typing(window, cx));
        self.oauth2.update(cx, |o, cx| o.end_typing(window, cx));
    }

    /// The input behind `field`.
    pub fn input(&self, field: AuthField) -> &Entity<InputState> {
        match field {
            AuthField::Username => &self.username,
            AuthField::Password => &self.password,
            AuthField::Domain => &self.domain,
            AuthField::Token => &self.token,
            AuthField::Key => &self.key,
            AuthField::Value => &self.value,
            AuthField::AccessKeyId => &self.aws[0],
            AuthField::SecretAccessKey => &self.aws[1],
            AuthField::SessionToken => &self.aws[2],
            AuthField::Service => &self.aws[3],
            AuthField::Region => &self.aws[4],
            AuthField::ProfileName => &self.aws[5],
            AuthField::AccessToken => &self.edgegrid[0],
            AuthField::ClientToken => &self.edgegrid[1],
            AuthField::ClientSecret => &self.edgegrid[2],
            AuthField::BaseUrl => &self.edgegrid[3],
            AuthField::Nonce => &self.edgegrid[4],
            AuthField::Timestamp => &self.edgegrid[5],
            AuthField::HeadersToSign => &self.edgegrid[6],
            AuthField::MaxBodySize => &self.max_body_size,
        }
    }

    /// Whether an OAuth 2.0 secret is typed but not yet committed.
    pub fn holds_pending(&self, cx: &App) -> bool {
        self.mode == Some(AuthMode::OAuth2) && self.oauth2.read(cx).has_pending_secrets(cx)
    }

    /// The OAuth 2.0 edit a secret typed but not yet committed still owes, so a save or
    /// send can send it first; None when the mode is another or nothing is pending.
    pub fn take_pending(&mut self, cx: &mut Context<Self>) -> Option<Edit> {
        if self.mode != Some(AuthMode::OAuth2) {
            return None;
        }
        let spec = self.oauth2.update(cx, |e, cx| e.take_pending_secrets(cx))?;
        let auth = AuthSpec::OAuth2(spec);
        self.memory.insert(AuthMode::OAuth2, auth.clone());
        Some(Edit::Auth(auth))
    }

    /// Shows `notice` in the OAuth 2.0 editor, the only mode whose edits warn or refuse.
    pub fn set_notice(&mut self, notice: Option<String>, cx: &mut Context<Self>) {
        self.oauth2.update(cx, |e, cx| e.set_notice(notice, cx));
    }

    /// The OAuth 1.0 editor, shown under [`AuthMode::OAuth1`].
    pub fn oauth1(&self) -> &Entity<OAuth1Editor> {
        &self.oauth1
    }

    /// The OAuth 2.0 editor, shown under [`AuthMode::OAuth2`].
    pub fn oauth2(&self) -> &Entity<OAuth2Editor> {
        &self.oauth2
    }

    // The spec of the shown mode as the inputs and placement describe it.
    fn current(&self, cx: &App) -> Option<AuthSpec> {
        let text = |field| self.input(field).read(cx).value().to_string();
        Some(match self.mode? {
            AuthMode::None => AuthSpec::None,
            AuthMode::Inherit => AuthSpec::Inherit,
            AuthMode::Basic => AuthSpec::Basic {
                username: text(AuthField::Username),
                password: text(AuthField::Password),
            },
            AuthMode::Bearer => AuthSpec::Bearer {
                token: text(AuthField::Token),
            },
            AuthMode::ApiKey => AuthSpec::ApiKey {
                key: text(AuthField::Key),
                value: text(AuthField::Value),
                placement: self.placement,
            },
            AuthMode::Digest => AuthSpec::Digest {
                username: text(AuthField::Username),
                password: text(AuthField::Password),
            },
            AuthMode::AwsV4 => AuthSpec::AwsV4 {
                access_key_id: text(AuthField::AccessKeyId),
                secret_access_key: text(AuthField::SecretAccessKey),
                session_token: text(AuthField::SessionToken),
                service: text(AuthField::Service),
                region: text(AuthField::Region),
                profile_name: text(AuthField::ProfileName),
            },
            AuthMode::Wsse => AuthSpec::Wsse {
                username: text(AuthField::Username),
                password: text(AuthField::Password),
            },
            AuthMode::Ntlm => AuthSpec::Ntlm {
                username: text(AuthField::Username),
                password: text(AuthField::Password),
                domain: text(AuthField::Domain),
            },
            AuthMode::EdgeGrid => {
                let mut spec = EdgeGridSpec::default();
                spec.access_token = text(AuthField::AccessToken);
                spec.client_token = text(AuthField::ClientToken);
                spec.client_secret = text(AuthField::ClientSecret);
                spec.base_url = text(AuthField::BaseUrl);
                spec.nonce = text(AuthField::Nonce);
                spec.timestamp = text(AuthField::Timestamp);
                spec.headers_to_sign = text(AuthField::HeadersToSign);
                spec.max_body_size = self.edgegrid_max;
                AuthSpec::EdgeGrid(spec)
            }
            AuthMode::OAuth1 => AuthSpec::OAuth1(self.oauth1.read(cx).spec(cx)),
            AuthMode::OAuth2 => AuthSpec::OAuth2(self.oauth2.read(cx).spec(cx)),
        })
    }

    // Puts `auth` on screen under `mode`. Inputs change only when their text differs, so an
    // echo of the user's own typing leaves the caret alone.
    fn show(
        &mut self,
        mode: AuthMode,
        auth: &AuthSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Some(mode);
        self.unsupported = None;
        let label: SharedString = mode.label().into();
        self.select
            .update(cx, |s, cx| s.set_selected_value(&label, window, cx));
        match auth {
            AuthSpec::Basic { username, password }
            | AuthSpec::Digest { username, password }
            | AuthSpec::Wsse { username, password } => {
                set_text(&self.username, username, window, cx);
                set_text(&self.password, password, window, cx);
            }
            AuthSpec::Ntlm {
                username,
                password,
                domain,
            } => {
                set_text(&self.username, username, window, cx);
                set_text(&self.password, password, window, cx);
                set_text(&self.domain, domain, window, cx);
            }
            AuthSpec::Bearer { token } => set_text(&self.token, token, window, cx),
            AuthSpec::ApiKey {
                key,
                value,
                placement,
            } => {
                set_text(&self.key, key, window, cx);
                set_text(&self.value, value, window, cx);
                self.show_placement(*placement, window, cx);
            }
            AuthSpec::AwsV4 {
                access_key_id,
                secret_access_key,
                session_token,
                service,
                region,
                profile_name,
            } => {
                let values = [
                    access_key_id,
                    secret_access_key,
                    session_token,
                    service,
                    region,
                    profile_name,
                ];
                for (state, value) in self.aws.iter().zip(values) {
                    set_text(state, value, window, cx);
                }
            }
            AuthSpec::EdgeGrid(e) => {
                let values = [
                    &e.access_token,
                    &e.client_token,
                    &e.client_secret,
                    &e.base_url,
                    &e.nonce,
                    &e.timestamp,
                    &e.headers_to_sign,
                ];
                for (state, value) in self.edgegrid.iter().zip(values) {
                    set_text(state, value, window, cx);
                }
                self.edgegrid_max = e.max_body_size;
                self.max_refused = false;
                // An echo whose text already parses to the value keeps the user's spelling.
                let shown = parse_max_body_size(&self.max_body_size.read(cx).value());
                if shown != Some(e.max_body_size) {
                    let text = e.max_body_size.map(js_number).unwrap_or_default();
                    set_text(&self.max_body_size, &text, window, cx);
                }
            }
            AuthSpec::OAuth1(o) => self.oauth1.update(cx, |e, cx| e.load(o, window, cx)),
            AuthSpec::OAuth2(o) => self.oauth2.update(cx, |e, cx| e.load(o, window, cx)),
            _ => {}
        }
        self.refresh_jwt(cx);
        cx.notify();
    }

    fn show_placement(
        &mut self,
        placement: ApiKeyPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.placement = placement;
        if let Some((_, label)) = PLACEMENTS.iter().find(|(p, _)| *p == placement) {
            let label: SharedString = (*label).into();
            self.placement_select
                .update(cx, |s, cx| s.set_selected_value(&label, window, cx));
        }
        cx.notify();
    }

    // A table of inputs; plain ones tint placeholders, masked secrets tint a lone reference.
    fn table_fields(
        table: &[(&'static str, &'static str, bool)],
        states: &[Entity<InputState>],
        vars: Option<Arc<VariableSet>>,
        cx: &App,
    ) -> impl IntoElement {
        v_flex().gap_2().children(
            table
                .iter()
                .zip(states)
                .map(|(&(label, id, masked), state)| {
                    if masked {
                        secret_field(label, id, state, vars.clone(), cx).into_any_element()
                    } else {
                        field(label, id, state, vars.clone(), cx).into_any_element()
                    }
                }),
        )
    }

    /// Whether `edit`, which this section sent, was built from a spec a load has since
    /// replaced: a load came after it and the mode shown or the spec remembered for it is no
    /// longer the one sent. Without a load between, a handler's first of two edits never lags.
    pub fn lags(&self, edit: &SectionEdit) -> bool {
        if edit.loads == self.loads {
            return false;
        }
        let Edit::Auth(sent) = &edit.edit else {
            return false;
        };
        let Some(mode) = AuthMode::of(sent) else {
            return false;
        };
        self.mode != Some(mode) || self.memory.get(&mode) != Some(sent)
    }

    fn emit(&mut self, auth: AuthSpec, cx: &mut Context<Self>) {
        if let Some(mode) = AuthMode::of(&auth) {
            self.memory.insert(mode, auth.clone());
        }
        cx.emit(SectionEdit::new(Edit::Auth(auth)).after_loads(self.loads));
    }

    // A sub-editor's edit sent before a load that has run since keeps its send-time stamp and
    // leaves the memory to that load, so `lags` sees the spec the load replaced it with.
    fn emit_stamped(&mut self, auth: AuthSpec, loads: u64, cx: &mut Context<Self>) {
        if loads == self.loads {
            return self.emit(auth, cx);
        }
        cx.emit(SectionEdit::new(Edit::Auth(auth)).after_loads(loads));
    }
}

/// JavaScript `String(n)`, so a loaded 1e300 reads `1e+300` and a non-finite one `Infinity`.
fn js_number(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

// Max Body Size text as a value: blank clears it, a finite number sets it, anything else
// is refused (`None`).
fn parse_max_body_size(text: &str) -> Option<Option<f64>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    text.parse::<f64>().ok().filter(|n| n.is_finite()).map(Some)
}

// The read-only decoded token: header, payload, then the expiry line.
fn jwt_block(jwt: &JwtView, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    ui::form_row(
        "auth-jwt-row",
        "Decoded",
        v_flex()
            .id("auth-jwt")
            .test_support()
            .gap_1()
            .w(style::WIDE_INPUT_WIDTH)
            .max_w_full()
            .max_h(style::JWT_BLOCK_MAX_HEIGHT)
            .overflow_y_scroll()
            .text_xs()
            .font_family(cx.theme().mono_font_family.clone())
            .child(div().id("auth-jwt-header").child(jwt.header.clone()))
            .child(div().id("auth-jwt-payload").child(jwt.payload.clone()))
            .child(
                div()
                    .id("auth-jwt-expiry")
                    .text_color(muted)
                    .child(jwt.expiry.clone()),
            ),
    )
}

// A labelled input row; with `vars` its placeholders are tinted and explained.
pub(crate) fn field(
    label: &'static str,
    id: &'static str,
    state: &Entity<InputState>,
    vars: Option<Arc<VariableSet>>,
    cx: &App,
) -> impl IntoElement {
    ui::form_row(
        SharedString::from(format!("{id}-row")),
        label,
        var_input(id, state, vars, cx),
    )
}

/// A masked input's row: like `field`, but with variables (tint only) only while its text
/// is one reference, so a literal is never tokenised.
pub(crate) fn secret_field(
    label: &'static str,
    id: &'static str,
    state: &Entity<InputState>,
    vars: Option<Arc<VariableSet>>,
    cx: &App,
) -> impl IntoElement {
    let vars = vars.filter(|_| ui::is_variable_reference(&state.read(cx).value()));
    ui::form_row(
        SharedString::from(format!("{id}-row")),
        label,
        var_input(id, state, vars, cx).tint_only(),
    )
}

// The wide input `id` wrapped in its `VarField`.
fn var_input(
    id: &'static str,
    state: &Entity<InputState>,
    vars: Option<Arc<VariableSet>>,
    cx: &App,
) -> VarField {
    let input = ui::padded_input(
        Input::new(state)
            .id(id)
            .w_full()
            .bg(ui::field_fill(false, cx)),
        state,
    );
    VarField::new(SharedString::from(format!("{id}-vars")), state, input, vars)
        .w(style::WIDE_INPUT_WIDTH)
        .max_w_full()
}

/// The Inherit text for `inherited`: the inherited mode's picker label, or the written mode
/// when the editor does not model it, and the collection or the folder by its name.
pub fn inherit_text(inherited: &InheritedAuth, collection: &Collection) -> String {
    let label = match (AuthMode::of(inherited.auth()), inherited.auth()) {
        (Some(mode), _) => mode.label(),
        (None, AuthSpec::Unsupported { mode }) => mode.as_str(),
        (None, _) => "unknown",
    };
    match inherited.source() {
        AuthSource::Folder(path) => {
            let name = collection
                .folder(path)
                .map_or(path.file_name(), |f| f.name());
            format!("Inherits {label} from folder \"{name}\".")
        }
        _ => format!("Inherits {label} from the collection."),
    }
}

// The masked inputs, re-checked every render so a lone reference shows while typed.
const SECRET_FIELDS: [AuthField; 8] = [
    AuthField::Password,
    AuthField::Token,
    AuthField::Value,
    AuthField::SecretAccessKey,
    AuthField::SessionToken,
    AuthField::AccessToken,
    AuthField::ClientToken,
    AuthField::ClientSecret,
];

impl Render for AuthSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for f in SECRET_FIELDS {
            ui::mask_unless_template(self.input(f), window, cx);
        }
        let vars = self.vars.clone();
        let content = match (self.mode, &self.unsupported) {
            (None, kind) => div()
                .id("auth-unsupported")
                .test_support()
                .child(format!(
                    "This auth mode ({}) is not editable here; it is kept as written.",
                    kind.as_deref().unwrap_or("unknown")
                ))
                .into_any_element(),
            (Some(AuthMode::None), _) => div()
                .id("auth-none")
                .aria_label(self.scope.no_auth_text())
                .test_support()
                .child(self.scope.no_auth_text())
                .into_any_element(),
            (Some(AuthMode::Inherit), _) => {
                let text = self
                    .inherited
                    .clone()
                    .unwrap_or_else(|| "Auth is inherited from the folder or collection.".into());
                div()
                    .id("auth-inherit")
                    .aria_label(text.clone())
                    .test_support()
                    .child(text)
                    .into_any_element()
            }
            (Some(AuthMode::Basic | AuthMode::Digest | AuthMode::Wsse), _) => v_flex()
                .gap_2()
                .child(field(
                    "Username",
                    "auth-username",
                    &self.username,
                    vars.clone(),
                    cx,
                ))
                .child(secret_field(
                    "Password",
                    "auth-password",
                    &self.password,
                    vars.clone(),
                    cx,
                ))
                .into_any_element(),
            (Some(AuthMode::Ntlm), _) => v_flex()
                .gap_2()
                .child(field(
                    "Username",
                    "auth-username",
                    &self.username,
                    vars.clone(),
                    cx,
                ))
                .child(secret_field(
                    "Password",
                    "auth-password",
                    &self.password,
                    vars.clone(),
                    cx,
                ))
                .child(field("Domain", "auth-domain", &self.domain, vars, cx))
                .into_any_element(),
            (Some(AuthMode::Bearer), _) => v_flex()
                .gap_2()
                .child(secret_field("Token", "auth-token", &self.token, vars, cx))
                .children(self.jwt.as_ref().map(|j| jwt_block(j, cx)))
                .into_any_element(),
            (Some(AuthMode::ApiKey), _) => v_flex()
                .gap_2()
                .child(field("Key", "auth-key", &self.key, vars.clone(), cx))
                .child(secret_field("Value", "auth-value", &self.value, vars, cx))
                .child(ui::form_row(
                    "auth-placement-row",
                    "Add To",
                    div()
                        .w(style::MODE_SELECT_WIDTH)
                        .max_w_full()
                        .child(Select::new(&self.placement_select).id("auth-placement")),
                ))
                .into_any_element(),
            (Some(AuthMode::AwsV4), _) => {
                Self::table_fields(&AWS_FIELDS, &self.aws, vars, cx).into_any_element()
            }
            (Some(AuthMode::EdgeGrid), _) => v_flex()
                .gap_2()
                .child(Self::table_fields(
                    &EDGEGRID_FIELDS,
                    &self.edgegrid,
                    vars,
                    cx,
                ))
                .child(field(
                    "Max Body Size",
                    "auth-eg-max-body-size",
                    &self.max_body_size,
                    None,
                    cx,
                ))
                .when(self.max_refused, |v| {
                    v.child(ui::form_note(
                        "auth-eg-max-body-size-error",
                        "Max Body Size must be a number; the last valid value is kept.",
                    ))
                })
                .into_any_element(),
            (Some(AuthMode::OAuth1), _) => self.oauth1.clone().into_any_element(),
            (Some(AuthMode::OAuth2), _) => self.oauth2.clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .gap_2()
            .child(
                div().w(style::MODE_SELECT_WIDTH).max_w_full().child(
                    Select::new(&self.select)
                        .id("auth-mode")
                        .when_some(self.mode_placeholder(), |s, p| s.placeholder(p)),
                ),
            )
            .child(content)
    }
}
