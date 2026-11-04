//! The OAuth 2.0 editor inside the auth tab: each grant's fields, selects and toggles.

use std::sync::Arc;

use apinata_core::{
    ClientAuth, OAuth2Flow, OAuth2Param, OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage,
    TokenPlacement, TokenSource, VariableSet,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::IndexPath;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::auth_section::{field, secret_field};
use crate::kv_table::set_text;
use crate::network_secrets::is_template;
use crate::token_panel::TokenPanel;
use crate::{style, ui};

/// The notice beside a client secret or password the file holds as plain text.
pub const PLAIN_NOTICE: &str = "Stored in plain text";

/// The whole spec after any change in the editor, with the owner's load stamp at sending.
pub struct OAuth2Edit(pub OAuth2Spec, pub u64);

/// A request to move the plain-text client secret and password to `.env`.
pub struct OAuth2MoveSecrets;

/// One of the editor's text inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum OAuth2Field {
    AuthorizationUrl,
    AccessTokenUrl,
    RefreshTokenUrl,
    CallbackUrl,
    ClientId,
    ClientSecret,
    Username,
    Password,
    Scope,
    State,
    TokenId,
    HeaderPrefix,
    QueryKey,
}

const FIELDS: [OAuth2Field; 13] = [
    OAuth2Field::AuthorizationUrl,
    OAuth2Field::AccessTokenUrl,
    OAuth2Field::RefreshTokenUrl,
    OAuth2Field::CallbackUrl,
    OAuth2Field::ClientId,
    OAuth2Field::ClientSecret,
    OAuth2Field::Username,
    OAuth2Field::Password,
    OAuth2Field::Scope,
    OAuth2Field::State,
    OAuth2Field::TokenId,
    OAuth2Field::HeaderPrefix,
    OAuth2Field::QueryKey,
];

impl OAuth2Field {
    /// The input's element id.
    pub fn id(self) -> &'static str {
        match self {
            OAuth2Field::AuthorizationUrl => "oauth2-authorization-url",
            OAuth2Field::AccessTokenUrl => "oauth2-access-token-url",
            OAuth2Field::RefreshTokenUrl => "oauth2-refresh-token-url",
            OAuth2Field::CallbackUrl => "oauth2-callback-url",
            OAuth2Field::ClientId => "oauth2-client-id",
            OAuth2Field::ClientSecret => "oauth2-client-secret",
            OAuth2Field::Username => "oauth2-username",
            OAuth2Field::Password => "oauth2-password",
            OAuth2Field::Scope => "oauth2-scope",
            OAuth2Field::State => "oauth2-state",
            OAuth2Field::TokenId => "oauth2-token-id",
            OAuth2Field::HeaderPrefix => "oauth2-header-prefix",
            OAuth2Field::QueryKey => "oauth2-query-key",
        }
    }

    // The file key this field edits, which names it in core's errors.
    fn key(self) -> &'static str {
        match self {
            OAuth2Field::AuthorizationUrl => "authorizationUrl",
            OAuth2Field::AccessTokenUrl => "accessTokenUrl",
            OAuth2Field::RefreshTokenUrl => "refreshTokenUrl",
            OAuth2Field::CallbackUrl => "callbackUrl",
            OAuth2Field::ClientId => "clientId",
            OAuth2Field::ClientSecret => "clientSecret",
            OAuth2Field::Username => "username",
            OAuth2Field::Password => "password",
            OAuth2Field::Scope => "scope",
            OAuth2Field::State => "state",
            OAuth2Field::TokenId => "tokenConfig.id",
            OAuth2Field::HeaderPrefix => "tokenConfig.placement.header",
            OAuth2Field::QueryKey => "tokenConfig.placement.query",
        }
    }

    // Labels come from core so the editor and its error messages name a field alike.
    fn label(self) -> &'static str {
        apinata_core::oauth2_field_label(self.key())
    }

    fn masked(self) -> bool {
        matches!(self, OAuth2Field::ClientSecret | OAuth2Field::Password)
    }

    // Whether the grant uses this field; the token fields show for every grant.
    fn shown_for(self, flow: OAuth2Flow) -> bool {
        use OAuth2Field as F;
        use OAuth2Flow::{AuthorizationCode, Implicit};
        match self {
            F::AuthorizationUrl | F::CallbackUrl | F::State => {
                matches!(flow, AuthorizationCode | Implicit)
            }
            F::AccessTokenUrl | F::RefreshTokenUrl | F::ClientSecret => flow != Implicit,
            F::Username | F::Password => flow == OAuth2Flow::Password,
            F::ClientId | F::Scope | F::TokenId | F::HeaderPrefix | F::QueryKey => true,
        }
    }

    fn text(self, o: &OAuth2Spec) -> Option<&str> {
        Some(match self {
            OAuth2Field::AuthorizationUrl => &o.authorization_url,
            OAuth2Field::AccessTokenUrl => &o.access_token_url,
            OAuth2Field::RefreshTokenUrl => &o.refresh_token_url,
            OAuth2Field::CallbackUrl => &o.callback_url,
            OAuth2Field::ClientId => &o.client_id,
            OAuth2Field::ClientSecret => &o.client_secret,
            OAuth2Field::Username => &o.username,
            OAuth2Field::Password => &o.password,
            OAuth2Field::Scope => &o.scope,
            OAuth2Field::State => &o.state,
            OAuth2Field::TokenId => &o.token_id,
            OAuth2Field::HeaderPrefix => match &o.token_placement {
                TokenPlacement::Header { prefix } => prefix,
                _ => return None,
            },
            OAuth2Field::QueryKey => match &o.token_placement {
                TokenPlacement::Query { key } => key,
                _ => return None,
            },
        })
    }
}

/// One of the editor's checkboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum OAuth2Toggle {
    AutoFetch,
    AutoRefresh,
    Pkce,
}

/// Where the fetched token goes, without the prefix or key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PlacementKind {
    Header,
    Query,
}

static FLOWS: [(OAuth2Flow, &str); 4] = [
    (OAuth2Flow::ClientCredentials, "Client credentials"),
    (OAuth2Flow::Password, "Password"),
    (OAuth2Flow::AuthorizationCode, "Authorization code"),
    (OAuth2Flow::Implicit, "Implicit"),
];
static CLIENT_AUTHS: [(ClientAuth, &str); 2] = [
    (ClientAuth::Body, "Send in body"),
    (ClientAuth::BasicHeader, "Basic auth header"),
];
static PLACEMENTS: [(PlacementKind, &str); 2] = [
    (PlacementKind::Header, "Header"),
    (PlacementKind::Query, "Query parameter"),
];
static SOURCES: [(TokenSource, &str); 2] = [
    (TokenSource::AccessToken, "Access token"),
    (TokenSource::IdToken, "ID token"),
];

static STAGES: [(OAuth2Stage, &str); 3] = [
    (OAuth2Stage::Authorization, "Authorization"),
    (OAuth2Stage::Token, "Token"),
    (OAuth2Stage::Refresh, "Refresh"),
];
static PARAM_PLACEMENTS: [(OAuth2ParamPlacement, &str); 3] = [
    (OAuth2ParamPlacement::Header, "Header"),
    (OAuth2ParamPlacement::Query, "Query"),
    (OAuth2ParamPlacement::Body, "Body"),
];

pub(crate) type Choice = Entity<SelectState<Vec<SharedString>>>;

/// A text cell of an additional parameter row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ParamCell {
    Name,
    Value,
}

// The controls of one additional parameter; its stage and placement live in the base spec.
struct ParamRow {
    name: Entity<InputState>,
    value: Entity<InputState>,
    stage: Choice,
    placement: Choice,
    _subs: [Subscription; 4],
}

/// Edits an OAuth 2.0 auth. The loaded spec is the base every edit rebuilds on, so values
/// no control shows (other grants' fields, unknown strings, parameters) survive an edit.
pub struct OAuth2Editor {
    base: OAuth2Spec,
    inputs: Vec<Entity<InputState>>,
    flow: Choice,
    client_auth: Choice,
    placement: Choice,
    source: Choice,
    params: Vec<ParamRow>,
    vars: Option<Arc<VariableSet>>,
    notice: Option<String>,
    token_panel: Option<Entity<TokenPanel>>,
    stamp: u64,
    _subs: Vec<Subscription>,
}

impl EventEmitter<OAuth2Edit> for OAuth2Editor {}
impl EventEmitter<OAuth2MoveSecrets> for OAuth2Editor {}

impl OAuth2Editor {
    /// An editor showing a fresh authorization code auth.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut choice = |items: Vec<&str>| {
            let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
            cx.new(|cx| SelectState::new(items, None, window, cx))
        };
        let flow = choice(FLOWS.iter().map(|(_, l)| *l).collect());
        let client_auth = choice(CLIENT_AUTHS.iter().map(|(_, l)| *l).collect());
        let placement = choice(PLACEMENTS.iter().map(|(_, l)| *l).collect());
        let source = choice(SOURCES.iter().map(|(_, l)| *l).collect());
        let inputs: Vec<Entity<InputState>> = FIELDS
            .iter()
            .map(|f| {
                cx.new(|cx| {
                    let state = InputState::new(window, cx).masked(f.masked());
                    match f {
                        OAuth2Field::CallbackUrl => {
                            state.placeholder("Redirect URL registered with the provider")
                        }
                        _ => state,
                    }
                })
            })
            .collect();
        let mut subs = Vec::new();
        // Each select maps its confirmed label back to a value and goes through its setter,
        // skipping a confirm that names the value already shown.
        subs.push(on_choice(&flow, &FLOWS, window, cx, |t, v, w, cx| {
            if t.base.flow != v {
                t.set_flow(v, w, cx);
            }
        }));
        subs.push(on_choice(
            &client_auth,
            &CLIENT_AUTHS,
            window,
            cx,
            |t, v, w, cx| {
                if t.base.client_auth.as_ref() != Some(&v) {
                    t.set_client_auth(v, w, cx);
                }
            },
        ));
        subs.push(on_choice(
            &placement,
            &PLACEMENTS,
            window,
            cx,
            |t, v, w, cx| {
                if t.kind() != Some(v) {
                    t.set_token_placement(v, w, cx);
                }
            },
        ));
        subs.push(on_choice(&source, &SOURCES, window, cx, |t, v, w, cx| {
            if t.base.token_source != v {
                t.set_token_source(v, w, cx);
            }
        }));
        // A keystroke sends the whole spec; typing a prefix or key while the file sets no
        // placement chooses that placement. A masked field sends on Enter or blur only, so a
        // secret moves to `.env` once, never a typed prefix of it.
        for (f, state) in FIELDS.iter().zip(&inputs) {
            let f = *f;
            subs.push(
                cx.subscribe_in(state, window, move |t, _, event: &InputEvent, w, cx| {
                    if f.masked() {
                        // The blur a close prompt causes keeps the typing; only its Save stores it.
                        let prompted =
                            matches!(event, InputEvent::Blur) && crate::closing::prompting(w, cx);
                        if !prompted
                            && matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
                        {
                            t.commit_secret(f, cx);
                        }
                        return;
                    }
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    if t.base.token_placement == TokenPlacement::Unset {
                        t.base.token_placement = match f {
                            OAuth2Field::HeaderPrefix => TokenPlacement::Header {
                                prefix: String::new(),
                            },
                            OAuth2Field::QueryKey => TokenPlacement::Query { key: String::new() },
                            _ => TokenPlacement::Unset,
                        };
                        t.show_choices(w, cx);
                    }
                    t.emit(cx);
                }),
            );
        }
        let mut editor = OAuth2Editor {
            base: OAuth2Spec::new(OAuth2Flow::AuthorizationCode),
            inputs,
            flow,
            client_auth,
            placement,
            source,
            params: Vec::new(),
            vars: None,
            notice: None,
            token_panel: None,
            stamp: 0,
            _subs: subs,
        };
        // A query key never set starts as the conventional name.
        set_text(
            editor.input(OAuth2Field::QueryKey),
            "access_token",
            window,
            cx,
        );
        let fresh = editor.base.clone();
        editor.show(&fresh, window, cx);
        editor
    }

    /// Shows `spec` without emitting; skipped when `spec == self.spec(cx).on_disk()`, the echo of its own edit.
    pub fn load(&mut self, spec: &OAuth2Spec, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.spec(cx);
        if *spec == current || *spec == current.on_disk() {
            return;
        }
        self.show(spec, window, cx);
    }

    /// Sets the owner's load count that every edit sent from now on carries.
    pub fn set_stamp(&mut self, stamp: u64) {
        self.stamp = stamp;
    }

    /// The spec the controls describe, over the loaded base; a masked field counts once it
    /// is committed by Enter or blur.
    pub fn spec(&self, cx: &App) -> OAuth2Spec {
        let mut o = self.base.clone();
        let text = |f: OAuth2Field| self.input(f).read(cx).value().to_string();
        o.authorization_url = text(OAuth2Field::AuthorizationUrl);
        o.access_token_url = text(OAuth2Field::AccessTokenUrl);
        o.refresh_token_url = text(OAuth2Field::RefreshTokenUrl);
        o.callback_url = text(OAuth2Field::CallbackUrl);
        o.client_id = text(OAuth2Field::ClientId);
        o.username = text(OAuth2Field::Username);
        o.scope = text(OAuth2Field::Scope);
        o.state = text(OAuth2Field::State);
        o.token_id = text(OAuth2Field::TokenId);
        o.token_placement = match o.token_placement {
            TokenPlacement::Header { .. } => TokenPlacement::Header {
                prefix: text(OAuth2Field::HeaderPrefix),
            },
            TokenPlacement::Query { .. } => TokenPlacement::Query {
                key: text(OAuth2Field::QueryKey),
            },
            other => other,
        };
        for (p, row) in o.params.iter_mut().zip(&self.params) {
            p.name = row.name.read(cx).value().to_string();
            p.value = row.value.read(cx).value().to_string();
        }
        o
    }

    /// Ends the typing run of every input's own undo, parameter cells included, as at a save
    /// or a reload.
    pub fn end_typing(&self, window: &mut Window, cx: &mut App) {
        let cells = self.params.iter().flat_map(|row| [&row.name, &row.value]);
        for input in self.inputs.iter().chain(cells).cloned().collect::<Vec<_>>() {
            crate::ui::end_input_run(&input, window, cx);
        }
    }

    /// The input behind `field`.
    pub fn input(&self, field: OAuth2Field) -> &Entity<InputState> {
        &self.inputs[field as usize]
    }

    /// The "Add token to" select.
    pub fn placement_choice(&self) -> &Entity<SelectState<Vec<SharedString>>> {
        &self.placement
    }

    /// Switches the grant, keeping every other grant's fields.
    pub fn set_flow(&mut self, flow: OAuth2Flow, window: &mut Window, cx: &mut Context<Self>) {
        self.base.flow = flow;
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Sets how the client authenticates to the token endpoint.
    pub fn set_client_auth(
        &mut self,
        auth: ClientAuth,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.base.client_auth = Some(auth);
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Sets where the token goes; the prefix and key typed for each are kept apart.
    pub fn set_token_placement(
        &mut self,
        kind: PlacementKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only the variant matters: `spec` fills the prefix or key from its input.
        self.base.token_placement = match kind {
            PlacementKind::Header => TokenPlacement::Header {
                prefix: String::new(),
            },
            PlacementKind::Query => TokenPlacement::Query { key: String::new() },
        };
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Sets which token of the response is sent.
    pub fn set_token_source(
        &mut self,
        source: TokenSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.base.token_source = source;
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Turns one checkbox on or off.
    pub fn set_toggle(
        &mut self,
        toggle: OAuth2Toggle,
        on: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match toggle {
            OAuth2Toggle::AutoFetch => self.base.auto_fetch = on,
            OAuth2Toggle::AutoRefresh => self.base.auto_refresh = on,
            OAuth2Toggle::Pkce => self.base.pkce = on,
        }
        self.emit(cx);
    }

    /// Appends a row: the flow's first stage, `Query` for the authorization stage and `Body` otherwise.
    pub fn add_param(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stage = self.base.flow.stages()[0];
        let placement = match stage {
            OAuth2Stage::Authorization => OAuth2ParamPlacement::Query,
            _ => OAuth2ParamPlacement::Body,
        };
        let param = OAuth2Param::new(stage, "", "", placement);
        let row = self.new_row(&param, window, cx);
        self.base.params.push(param);
        self.params.push(row);
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Removes row `ix` of `spec().params`.
    pub fn remove_param(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) {
        if ix < self.params.len() {
            self.base.params.remove(ix);
            self.params.remove(ix);
            self.emit(cx);
        }
    }

    /// Sets the request row `ix` is sent with.
    pub fn set_param_stage(
        &mut self,
        ix: usize,
        stage: OAuth2Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.base.params.get_mut(ix) {
            p.stage = stage;
            self.show_choices(window, cx);
            self.emit(cx);
        }
    }

    /// Sets where row `ix` is sent.
    pub fn set_param_placement(
        &mut self,
        ix: usize,
        placement: OAuth2ParamPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.base.params.get_mut(ix) {
            p.placement = placement;
            self.show_choices(window, cx);
            self.emit(cx);
        }
    }

    /// The name or value input of row `ix` of `spec().params`, hidden rows included.
    pub fn param_input(&self, ix: usize, cell: ParamCell) -> &Entity<InputState> {
        let row = &self.params[ix];
        match cell {
            ParamCell::Name => &row.name,
            ParamCell::Value => &row.value,
        }
    }

    /// Shows `notice` under the grant's fields, or clears it.
    pub fn set_notice(&mut self, notice: Option<String>, cx: &mut Context<Self>) {
        self.notice = notice;
        cx.notify();
    }

    /// The notice shown, from the last refused or warned auth edit.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Shows `panel` after the parameters table, or none.
    pub fn set_token_panel(&mut self, panel: Option<Entity<TokenPanel>>, cx: &mut Context<Self>) {
        self.token_panel = panel;
        cx.notify();
    }

    /// The token panel shown after the parameters table.
    pub fn token_panel(&self) -> Option<&Entity<TokenPanel>> {
        self.token_panel.as_ref()
    }

    /// Tint and explain placeholders in the unmasked fields.
    pub fn set_variables(&mut self, vars: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.vars = vars;
        cx.notify();
    }

    fn kind(&self) -> Option<PlacementKind> {
        match self.base.token_placement {
            TokenPlacement::Header { .. } => Some(PlacementKind::Header),
            TokenPlacement::Query { .. } => Some(PlacementKind::Query),
            _ => None,
        }
    }

    // Handlers find their row by entity, since a removal shifts the indices they were made at.
    fn new_row(&self, p: &OAuth2Param, window: &mut Window, cx: &mut Context<Self>) -> ParamRow {
        let mut input = |text: &str, hint: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(hint)
                    .default_value(text.to_owned())
            })
        };
        let name = input(&p.name, "Name", cx);
        let value = input(&p.value, "Value", cx);
        let stage = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let placement = cx.new(|cx| {
            let items = PARAM_PLACEMENTS.iter().map(|(_, l)| (*l).into()).collect();
            SelectState::new(items, None, window, cx)
        });
        let typed = |input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(input, window, |t, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    t.emit(cx);
                }
            })
        };
        let subs = [
            typed(&name, window, cx),
            typed(&value, window, cx),
            on_row_choice(&stage, &STAGES, window, cx, |t, ix, v, w, cx| {
                if t.base.params[ix].stage != v {
                    t.set_param_stage(ix, v, w, cx);
                }
            }),
            on_row_choice(
                &placement,
                &PARAM_PLACEMENTS,
                window,
                cx,
                |t, ix, v, w, cx| {
                    if t.base.params[ix].placement != v {
                        t.set_param_placement(ix, v, w, cx);
                    }
                },
            ),
        ];
        ParamRow {
            name,
            value,
            stage,
            placement,
            _subs: subs,
        }
    }

    // Takes a masked field's text into the base and sends it, unless it is already there.
    fn commit_secret(&mut self, f: OAuth2Field, cx: &mut Context<Self>) {
        let text = self.input(f).read(cx).value().to_string();
        let slot = match f {
            OAuth2Field::ClientSecret => &mut self.base.client_secret,
            OAuth2Field::Password => &mut self.base.password,
            _ => return,
        };
        if *slot != text {
            *slot = text;
            self.emit(cx);
        }
    }

    /// Whether a client secret or password is typed but not yet committed.
    pub fn has_pending_secrets(&self, cx: &App) -> bool {
        let text = |f: OAuth2Field| self.input(f).read(cx).value().to_string();
        text(OAuth2Field::ClientSecret) != self.base.client_secret
            || text(OAuth2Field::Password) != self.base.password
    }

    /// Takes a client secret or password typed but not yet committed into the base and
    /// returns the spec to send, without emitting; None when nothing was pending.
    pub fn take_pending_secrets(&mut self, cx: &mut Context<Self>) -> Option<OAuth2Spec> {
        let text = |f: OAuth2Field| self.input(f).read(cx).value().to_string();
        let (secret, password) = (text(OAuth2Field::ClientSecret), text(OAuth2Field::Password));
        if secret == self.base.client_secret && password == self.base.password {
            return None;
        }
        self.base.client_secret = secret;
        self.base.password = password;
        Some(self.spec(cx))
    }

    // A secret typed but not yet committed goes first, since a click does not blur it.
    fn move_secrets(&mut self, cx: &mut Context<Self>) {
        self.commit_secret(OAuth2Field::ClientSecret, cx);
        self.commit_secret(OAuth2Field::Password, cx);
        cx.emit(OAuth2MoveSecrets);
    }

    // Whether the loaded spec holds a client secret or password as plain text.
    fn plain_secrets(&self) -> bool {
        [&self.base.client_secret, &self.base.password]
            .iter()
            .any(|s| !s.is_empty() && !is_template(s))
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        let spec = self.spec(cx);
        cx.emit(OAuth2Edit(spec, self.stamp));
        cx.notify();
    }

    // Puts `spec` on screen as the new base. Inputs change only when their text differs, so
    // an echo leaves the caret alone; a placement's prefix or key the spec lacks is kept.
    fn show(&mut self, spec: &OAuth2Spec, window: &mut Window, cx: &mut Context<Self>) {
        let old = std::mem::replace(&mut self.base, spec.clone());
        for f in FIELDS {
            // A masked field commits on Enter or blur, so its typing stays while the spec keeps
            // the value last committed.
            if let Some(text) = f.text(spec)
                && !(f.masked() && f.text(&old) == Some(text))
            {
                set_text(self.input(f), text, window, cx);
            }
        }
        // Rows are rebuilt only when their count changes, so typing in a row keeps its caret.
        if self.params.len() != spec.params.len() {
            self.params = spec
                .params
                .iter()
                .map(|p| self.new_row(p, window, cx))
                .collect();
        } else {
            for (p, row) in spec.params.iter().zip(&self.params) {
                set_text(&row.name, &p.name, window, cx);
                set_text(&row.value, &p.value, window, cx);
            }
        }
        self.show_choices(window, cx);
    }

    // Selects the base's value in each select, or nothing for a value it does not offer.
    fn show_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = &self.base;
        pick(&self.flow, &FLOWS, &base.flow, window, cx);
        match &base.client_auth {
            Some(a) => pick(&self.client_auth, &CLIENT_AUTHS, a, window, cx),
            None => pick(&self.client_auth, &[], &ClientAuth::Body, window, cx),
        }
        match self.kind() {
            Some(k) => pick(&self.placement, &PLACEMENTS, &k, window, cx),
            None => pick(&self.placement, &[], &PlacementKind::Header, window, cx),
        }
        pick(&self.source, &SOURCES, &self.base.token_source, window, cx);
        // A row's stage select offers only the stages the current flow sends.
        let stages: Vec<(OAuth2Stage, &'static str)> = STAGES
            .iter()
            .filter(|(s, _)| base.flow.stages().contains(s))
            .cloned()
            .collect();
        let labels: Vec<SharedString> = stages.iter().map(|(_, l)| (*l).into()).collect();
        for (p, row) in base.params.iter().zip(&self.params) {
            row.stage
                .update(cx, |s, cx| s.set_items(labels.clone(), window, cx));
            pick(&row.stage, &stages, &p.stage, window, cx);
            pick(&row.placement, &PARAM_PLACEMENTS, &p.placement, window, cx);
        }
        cx.notify();
    }

    fn select(
        &self,
        label: &'static str,
        id: &'static str,
        state: &Choice,
        unknown: Option<String>,
    ) -> impl IntoElement {
        ui::form_row(
            SharedString::from(format!("{id}-row")),
            label,
            div().w(style::MODE_SELECT_WIDTH).max_w_full().child(
                Select::new(state)
                    .id(id)
                    .when_some(unknown, |s, p| s.placeholder(p)),
            ),
        )
    }

    fn checkbox(
        &self,
        toggle: OAuth2Toggle,
        id: &'static str,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        ui::checkbox(id, checked)
            .label(label)
            .on_click(cx.listener(move |t, on: &bool, w, cx| t.set_toggle(toggle, *on, w, cx)))
    }
}

// Subscribes to a select's confirmed label, handing the matching value to `then`.
pub(crate) fn on_choice<V: 'static, T: Clone + 'static>(
    state: &Choice,
    items: &'static [(T, &'static str)],
    window: &mut Window,
    cx: &mut Context<V>,
    then: impl Fn(&mut V, T, &mut Window, &mut Context<V>) + 'static,
) -> Subscription {
    cx.subscribe_in(
        state,
        window,
        move |t, _, event: &SelectEvent<Vec<SharedString>>, w, cx| {
            if let SelectEvent::Confirm(Some(label)) = event
                && let Some((v, _)) = items.iter().find(|(_, l)| *l == label.as_ref())
            {
                then(t, v.clone(), w, cx);
            }
        },
    )
}

// Like `on_choice` for a parameter row's select, also handing over the row's current index.
fn on_row_choice<T: Clone + 'static>(
    state: &Choice,
    items: &'static [(T, &'static str)],
    window: &mut Window,
    cx: &mut Context<OAuth2Editor>,
    then: impl Fn(&mut OAuth2Editor, usize, T, &mut Window, &mut Context<OAuth2Editor>) + 'static,
) -> Subscription {
    cx.subscribe_in(
        state,
        window,
        move |t, select, event: &SelectEvent<Vec<SharedString>>, w, cx| {
            let ix = t
                .params
                .iter()
                .position(|r| &r.stage == select || &r.placement == select);
            if let SelectEvent::Confirm(Some(label)) = event
                && let Some(ix) = ix
                && let Some((v, _)) = items.iter().find(|(_, l)| *l == label.as_ref())
            {
                then(t, ix, v.clone(), w, cx);
            }
        },
    )
}

// Selects `value`'s label, or clears the select when `items` does not offer it.
pub(crate) fn pick<T: PartialEq>(
    state: &Choice,
    items: &[(T, &'static str)],
    value: &T,
    window: &mut Window,
    cx: &mut App,
) {
    let label = items.iter().find(|(v, _)| v == value).map(|(_, l)| *l);
    state.update(cx, |s, cx| match label {
        Some(l) => s.set_selected_value(&SharedString::from(l), window, cx),
        None => s.set_selected_index(None::<IndexPath>, window, cx),
    });
}

impl Render for OAuth2Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for f in [OAuth2Field::ClientSecret, OAuth2Field::Password] {
            ui::mask_unless_template(self.input(f), window, cx);
        }
        let o = self.base.clone();
        let flow = o.flow;
        let input = |f: OAuth2Field, cx: &App| {
            let vars = self.vars.clone();
            if f.masked() {
                secret_field(f.label(), f.id(), self.input(f), vars, cx).into_any_element()
            } else {
                field(f.label(), f.id(), self.input(f), vars, cx).into_any_element()
            }
        };
        let shown = |f: OAuth2Field| f.shown_for(flow);
        let mut grant =
            v_flex()
                .gap_2()
                .child(self.select("Grant type", "oauth2-flow", &self.flow, None));
        for f in &FIELDS[..10] {
            if shown(*f) {
                grant = grant.child(input(*f, cx));
            }
        }
        let unknown_auth = match &o.client_auth {
            Some(ClientAuth::Other(s)) => Some(s.clone()),
            _ => None,
        };
        let unknown_source = match &o.token_source {
            TokenSource::Other(s) => Some(s.clone()),
            _ => None,
        };
        let kind = self.kind();
        let plain = self.plain_secrets().then(|| {
            // Wraps so the button drops below the note when the pane is too narrow for both.
            h_flex()
                .id("oauth2-plain-notice")
                .test_support()
                .flex_wrap()
                .gap(style::SPACE_2)
                .items_center()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().warning)
                .child(PLAIN_NOTICE)
                .child(
                    Button::new("oauth2-move-secrets")
                        .label("Move to .env")
                        .on_click(cx.listener(|this, _, _, cx| this.move_secrets(cx))),
                )
        });
        let notice = self.notice.clone().map(|text| {
            div()
                .id("oauth2-notice")
                .test_support()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(text)
        });
        // The token panel leads so its buttons stay in view above the long parameter list.
        v_flex()
            .gap_2()
            .children(self.token_panel.clone())
            .child(grant)
            .children(plain)
            .children(notice)
            .when(flow != OAuth2Flow::Implicit, |el| {
                el.child(self.select(
                    "Client authentication",
                    "oauth2-client-auth",
                    &self.client_auth,
                    unknown_auth,
                ))
            })
            .when(flow == OAuth2Flow::AuthorizationCode, |el| {
                // An empty label keeps the label cell, so the box lines up with the fields.
                el.child(ui::form_row(
                    "oauth2-pkce-row",
                    "",
                    self.checkbox(OAuth2Toggle::Pkce, "oauth2-pkce", "Use PKCE", o.pkce, cx),
                ))
            })
            .child(input(OAuth2Field::TokenId, cx))
            .child(self.select(
                "Add token to",
                "oauth2-token-placement",
                &self.placement,
                kind.is_none().then(|| "Not set".to_owned()),
            ))
            .when(kind != Some(PlacementKind::Query), |el| {
                el.child(input(OAuth2Field::HeaderPrefix, cx))
            })
            .when(kind != Some(PlacementKind::Header), |el| {
                el.child(input(OAuth2Field::QueryKey, cx))
            })
            .child(self.select(
                "Token to send",
                "oauth2-token-source",
                &self.source,
                unknown_source,
            ))
            .child(ui::form_row(
                "oauth2-auto-fetch-row",
                "",
                self.checkbox(
                    OAuth2Toggle::AutoFetch,
                    "oauth2-auto-fetch",
                    "Fetch a token automatically",
                    o.auto_fetch,
                    cx,
                ),
            ))
            .child(ui::form_row(
                "oauth2-auto-refresh-row",
                "",
                self.checkbox(
                    OAuth2Toggle::AutoRefresh,
                    "oauth2-auto-refresh",
                    "Refresh the token automatically",
                    o.auto_refresh,
                    cx,
                ),
            ))
            .child(self.render_params(cx))
    }
}

impl OAuth2Editor {
    // The additional parameters table; rows of stages the flow does not send stay hidden.
    fn render_params(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let stages = self.base.flow.stages();
        let rows = self
            .base
            .params
            .iter()
            .zip(&self.params)
            .enumerate()
            .filter(|(_, (p, _))| stages.contains(&p.stage))
            .map(|(ix, (_, row))| {
                // Cells take the other OAuth fields' default size and font, so a row lines up
                // with the inputs above it; key and value share the width the selects leave,
                // and a narrow pane wraps the row rather than squeezing them below typing room.
                let cell = |state: &Entity<InputState>, id: &'static str| {
                    ui::padded_input(
                        Input::new(state)
                            .id((id, ix))
                            .flex_1()
                            .min_w(style::SHORT_INPUT_WIDTH)
                            .bg(ui::field_fill(false, cx)),
                        state,
                    )
                };
                h_flex()
                    .flex_wrap()
                    .gap(style::SPACE_2)
                    .items_center()
                    .w_full()
                    .child(
                        div()
                            .flex_none()
                            .w(style::PARAM_STAGE_WIDTH)
                            .max_w_full()
                            .child(Select::new(&row.stage).id(("oauth2-param-stage", ix))),
                    )
                    .child(cell(&row.name, "oauth2-param-name"))
                    .child(cell(&row.value, "oauth2-param-value"))
                    .child(
                        div()
                            .flex_none()
                            .w(style::PARAM_PLACEMENT_WIDTH)
                            .max_w_full()
                            .child(Select::new(&row.placement).id(("oauth2-param-placement", ix))),
                    )
                    .child(
                        ui::icon_button(
                            ("oauth2-param-remove", ix),
                            IconName::Close,
                            "Remove parameter",
                            None,
                        )
                        .on_click(cx.listener(move |t, _, w, cx| t.remove_param(ix, w, cx))),
                    )
            });
        v_flex()
            .gap_1()
            .w_full()
            .child("Additional parameters")
            .children(rows)
            // A row wrapper keeps the button at its natural width on the left edge.
            .child(
                h_flex().child(
                    Button::new("oauth2-param-add")
                        .ghost()
                        .small()
                        .label("Add parameter")
                        .on_click(cx.listener(|t, _, w, cx| t.add_param(w, cx))),
                ),
            )
    }
}
