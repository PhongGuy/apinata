//! The OAuth 1.0 editor inside the auth tab: credentials, signing choices and the optional
//! protocol parameters, all visible at once.

use std::sync::Arc;

use apinata_core::{
    OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec,
    VariableSet,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, EventEmitter, SharedString, Subscription, Window, div};

use crate::auth_section::{field, secret_field};
use crate::kv_table::set_text;
use crate::oauth2_editor::{Choice, on_choice, pick};
use crate::{style, ui};

/// The whole spec after any change in the editor, with the owner's load stamp at sending.
pub struct OAuth1Edit(pub OAuth1Spec, pub u64);

/// One of the editor's text inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum OAuth1Field {
    ConsumerKey,
    ConsumerSecret,
    AccessToken,
    TokenSecret,
    PrivateKey,
    CallbackUrl,
    Verifier,
    Timestamp,
    Nonce,
    Version,
    Realm,
}

const FIELDS: [OAuth1Field; 11] = [
    OAuth1Field::ConsumerKey,
    OAuth1Field::ConsumerSecret,
    OAuth1Field::AccessToken,
    OAuth1Field::TokenSecret,
    OAuth1Field::PrivateKey,
    OAuth1Field::CallbackUrl,
    OAuth1Field::Verifier,
    OAuth1Field::Timestamp,
    OAuth1Field::Nonce,
    OAuth1Field::Version,
    OAuth1Field::Realm,
];

impl OAuth1Field {
    /// The input's element id.
    pub fn id(self) -> &'static str {
        match self {
            OAuth1Field::ConsumerKey => "oauth1-consumer-key",
            OAuth1Field::ConsumerSecret => "oauth1-consumer-secret",
            OAuth1Field::AccessToken => "oauth1-access-token",
            OAuth1Field::TokenSecret => "oauth1-token-secret",
            OAuth1Field::PrivateKey => "oauth1-private-key",
            OAuth1Field::CallbackUrl => "oauth1-callback-url",
            OAuth1Field::Verifier => "oauth1-verifier",
            OAuth1Field::Timestamp => "oauth1-timestamp",
            OAuth1Field::Nonce => "oauth1-nonce",
            OAuth1Field::Version => "oauth1-version",
            OAuth1Field::Realm => "oauth1-realm",
        }
    }

    fn label(self) -> &'static str {
        match self {
            OAuth1Field::ConsumerKey => "Consumer key",
            OAuth1Field::ConsumerSecret => "Consumer secret",
            OAuth1Field::AccessToken => "Token",
            OAuth1Field::TokenSecret => "Token secret",
            OAuth1Field::PrivateKey => "Private key",
            OAuth1Field::CallbackUrl => "Callback URL",
            OAuth1Field::Verifier => "Verifier",
            OAuth1Field::Timestamp => "Timestamp",
            OAuth1Field::Nonce => "Nonce",
            OAuth1Field::Version => "Version",
            OAuth1Field::Realm => "Realm",
        }
    }

    fn masked(self) -> bool {
        matches!(
            self,
            OAuth1Field::ConsumerSecret | OAuth1Field::TokenSecret | OAuth1Field::PrivateKey
        )
    }

    // The field's text in `o`; an absent value shows as empty.
    fn text(self, o: &OAuth1Spec) -> &str {
        let text = match self {
            OAuth1Field::ConsumerKey => &o.consumer_key,
            OAuth1Field::ConsumerSecret => &o.consumer_secret,
            OAuth1Field::AccessToken => &o.access_token,
            OAuth1Field::TokenSecret => &o.access_token_secret,
            OAuth1Field::PrivateKey => return o.private_key.as_ref().map_or("", |k| &k.value),
            OAuth1Field::CallbackUrl => &o.callback_url,
            OAuth1Field::Verifier => &o.verifier,
            OAuth1Field::Timestamp => &o.timestamp,
            OAuth1Field::Nonce => &o.nonce,
            OAuth1Field::Version => return &o.version,
            OAuth1Field::Realm => &o.realm,
        };
        text.as_deref().unwrap_or("")
    }

    // Writes `text` into this field of `o`; a private key keeps its kind, `new_key` when it had none.
    fn set(self, o: &mut OAuth1Spec, text: String, new_key: OAuth1KeyKind) {
        let slot = match self {
            OAuth1Field::ConsumerKey => &mut o.consumer_key,
            OAuth1Field::ConsumerSecret => &mut o.consumer_secret,
            OAuth1Field::AccessToken => &mut o.access_token,
            OAuth1Field::TokenSecret => &mut o.access_token_secret,
            OAuth1Field::PrivateKey => {
                let kind = o.private_key.as_ref().map_or(new_key, |k| k.kind);
                o.private_key = Some(OAuth1PrivateKey::new(kind, text));
                return;
            }
            OAuth1Field::CallbackUrl => &mut o.callback_url,
            OAuth1Field::Verifier => &mut o.verifier,
            OAuth1Field::Timestamp => &mut o.timestamp,
            OAuth1Field::Nonce => &mut o.nonce,
            OAuth1Field::Version => {
                o.version = text;
                return;
            }
            OAuth1Field::Realm => &mut o.realm,
        };
        *slot = Some(text);
    }
}

static METHODS: [(OAuth1SignatureMethod, &str); 7] = [
    (OAuth1SignatureMethod::HmacSha1, "HMAC-SHA1"),
    (OAuth1SignatureMethod::HmacSha256, "HMAC-SHA256"),
    (OAuth1SignatureMethod::HmacSha512, "HMAC-SHA512"),
    (OAuth1SignatureMethod::RsaSha1, "RSA-SHA1"),
    (OAuth1SignatureMethod::RsaSha256, "RSA-SHA256"),
    (OAuth1SignatureMethod::RsaSha512, "RSA-SHA512"),
    (OAuth1SignatureMethod::Plaintext, "PLAINTEXT"),
];
static KEY_KINDS: [(OAuth1KeyKind, &str); 2] =
    [(OAuth1KeyKind::Text, "Text"), (OAuth1KeyKind::File, "File")];
static PLACEMENTS: [(OAuth1Placement, &str); 3] = [
    (OAuth1Placement::Header, "Header"),
    (OAuth1Placement::Query, "Query params"),
    (OAuth1Placement::Body, "Body"),
];

/// Edits an OAuth 1.0 auth. The loaded spec is the base each edit changes one field of, so
/// absent fields and unknown names no control shows survive an unrelated edit.
pub struct OAuth1Editor {
    base: OAuth1Spec,
    inputs: Vec<Entity<InputState>>,
    masks: ui::SecretMasks,
    method: Choice,
    key_kind: Choice,
    /// The kind a key typed into an empty field gets; the file holds no kind without a key.
    new_key: OAuth1KeyKind,
    placement: Choice,
    vars: Option<Arc<VariableSet>>,
    stamp: u64,
    _subs: Vec<Subscription>,
}

impl EventEmitter<OAuth1Edit> for OAuth1Editor {}

impl OAuth1Editor {
    /// An editor showing a fresh OAuth 1.0 auth.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut choice = |id: &str, items: Vec<&str>| {
            let items = ui::id_items(id, items);
            cx.new(|cx| SelectState::new(items, None, window, cx))
        };
        let method = choice(
            "oauth1-signature-method",
            METHODS.iter().map(|(_, l)| *l).collect(),
        );
        let key_kind = choice(
            "oauth1-key-kind",
            KEY_KINDS.iter().map(|(_, l)| *l).collect(),
        );
        let placement = choice(
            "oauth1-placement",
            PLACEMENTS.iter().map(|(_, l)| *l).collect(),
        );
        let inputs: Vec<Entity<InputState>> = FIELDS
            .iter()
            .map(|f| {
                cx.new(|cx| {
                    let state = InputState::new(window, cx).masked(f.masked());
                    match f {
                        OAuth1Field::Timestamp | OAuth1Field::Nonce => {
                            state.placeholder("Generated when sent")
                        }
                        _ => state,
                    }
                })
            })
            .collect();
        // Each select goes through its setter, skipping a confirm of the value already shown.
        let mut subs = vec![
            on_choice(&method, &METHODS, window, cx, |t, v, w, cx| {
                if t.base.signature_method != v {
                    t.set_signature_method(v, w, cx);
                }
            }),
            on_choice(&key_kind, &KEY_KINDS, window, cx, |t, v, w, cx| {
                if t.base.private_key.as_ref().map_or(t.new_key, |k| k.kind) != v {
                    t.set_key_kind(v, w, cx);
                }
            }),
            on_choice(&placement, &PLACEMENTS, window, cx, |t, v, w, cx| {
                if t.base.placement != v {
                    t.set_placement(v, w, cx);
                }
            }),
        ];
        // A keystroke, masked fields included, sets only its own field and sends the spec; text
        // equal to what the base shows is no edit, so an absent field stays absent.
        for (f, state) in FIELDS.iter().zip(&inputs) {
            let f = *f;
            subs.push(
                cx.subscribe(state, move |t, input, event: &InputEvent, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let text = input.read(cx).value().to_string();
                    if text != f.text(&t.base) {
                        f.set(&mut t.base, text, t.new_key);
                        t.emit(cx);
                    }
                }),
            );
        }
        let mut editor = OAuth1Editor {
            base: OAuth1Spec::default(),
            inputs,
            masks: ui::SecretMasks::default(),
            method,
            key_kind,
            new_key: OAuth1KeyKind::Text,
            placement,
            vars: None,
            stamp: 0,
            _subs: subs,
        };
        let fresh = editor.base.clone();
        editor.show(&fresh, window, cx);
        editor
    }

    /// Shows `spec` without emitting; skipped when it is the spec already shown.
    pub fn load(&mut self, spec: &OAuth1Spec, window: &mut Window, cx: &mut Context<Self>) {
        if *spec != self.base {
            self.show(spec, window, cx);
        }
    }

    /// Sets the owner's load count that every edit sent from now on carries.
    pub fn set_stamp(&mut self, stamp: u64) {
        self.stamp = stamp;
    }

    /// The spec the editor describes.
    pub fn spec(&self, _: &App) -> OAuth1Spec {
        self.base.clone()
    }

    /// Ends the typing run of every input's own undo, as at a save or a reload.
    pub fn end_typing(&self, window: &mut Window, cx: &mut App) {
        for input in self.inputs.clone() {
            crate::ui::end_input_run(&input, window, cx);
        }
    }

    /// The input behind `field`.
    pub fn input(&self, field: OAuth1Field) -> &Entity<InputState> {
        &self.inputs[field as usize]
    }

    /// Sets how requests are signed; an RSA method swaps the consumer secret for the key.
    pub fn set_signature_method(
        &mut self,
        m: OAuth1SignatureMethod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.base.signature_method = m;
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Sets whether the private key is inline text or a file path, keeping its value. With no
    /// key yet the kind waits for the first keystroke, since a kind alone is never written.
    pub fn set_key_kind(
        &mut self,
        kind: OAuth1KeyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.base.private_key.as_mut() else {
            self.new_key = kind;
            self.show_choices(window, cx);
            return;
        };
        key.kind = kind;
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Sets where the OAuth parameters are sent.
    pub fn set_placement(
        &mut self,
        p: OAuth1Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.base.placement = p;
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Turns the body hash on or off.
    pub fn set_include_body_hash(&mut self, on: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.base.include_body_hash = on;
        self.emit(cx);
    }

    /// Tint and explain placeholders in the unmasked fields.
    pub fn set_variables(&mut self, vars: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.vars = vars;
        cx.notify();
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        cx.emit(OAuth1Edit(self.base.clone(), self.stamp));
        cx.notify();
    }

    // Puts `spec` on screen as the new base; inputs change only when their text differs, so
    // an echo leaves the caret alone.
    fn show(&mut self, spec: &OAuth1Spec, window: &mut Window, cx: &mut Context<Self>) {
        self.base = spec.clone();
        self.new_key = OAuth1KeyKind::Text;
        for f in FIELDS {
            set_text(self.input(f), f.text(spec), window, cx);
        }
        self.show_choices(window, cx);
    }

    // Selects the base's value in each select, or nothing for a name it does not offer.
    fn show_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = &self.base;
        pick(&self.method, &METHODS, &base.signature_method, window, cx);
        match &base.private_key {
            Some(k) => pick(&self.key_kind, &KEY_KINDS, &k.kind, window, cx),
            None => pick(&self.key_kind, &KEY_KINDS, &self.new_key, window, cx),
        }
        pick(&self.placement, &PLACEMENTS, &base.placement, window, cx);
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
}

impl Render for OAuth1Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for f in FIELDS.into_iter().filter(|f| f.masked()) {
            let input = self.input(f).clone();
            self.masks.apply(&input, window, cx);
        }
        let o = &self.base;
        let rsa = o.signature_method.is_rsa();
        let input = |f: OAuth1Field, cx: &App| {
            let vars = self.vars.clone();
            if f.masked() {
                secret_field(f.label(), f.id(), self.input(f), vars, cx).into_any_element()
            } else {
                field(f.label(), f.id(), self.input(f), vars, cx).into_any_element()
            }
        };
        let unknown_method = match &o.signature_method {
            OAuth1SignatureMethod::Other(s) => Some(s.clone()),
            _ => None,
        };
        let unknown_placement = match &o.placement {
            OAuth1Placement::Other(s) => Some(s.clone()),
            _ => None,
        };
        let body_hash = o.include_body_hash;
        v_flex()
            .gap_2()
            .child(input(OAuth1Field::ConsumerKey, cx))
            .when(!rsa, |el| el.child(input(OAuth1Field::ConsumerSecret, cx)))
            .child(input(OAuth1Field::AccessToken, cx))
            .child(input(OAuth1Field::TokenSecret, cx))
            .child(self.select(
                "Signature method",
                "oauth1-signature-method",
                &self.method,
                unknown_method,
            ))
            .when(rsa, |el| {
                el.child(input(OAuth1Field::PrivateKey, cx))
                    .child(self.select(
                        "Private key source",
                        "oauth1-key-kind",
                        &self.key_kind,
                        None,
                    ))
            })
            .child(self.select(
                "Add params to",
                "oauth1-placement",
                &self.placement,
                unknown_placement,
            ))
            .child(ui::form_row(
                "oauth1-include-body-hash-row",
                "",
                ui::checkbox("oauth1-include-body-hash", body_hash)
                    .label("Include body hash")
                    .on_click(
                        cx.listener(|t, on: &bool, w, cx| t.set_include_body_hash(*on, w, cx)),
                    ),
            ))
            .children(FIELDS[5..].iter().map(|f| input(*f, cx)))
    }
}
