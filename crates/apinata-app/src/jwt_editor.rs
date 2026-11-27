//! The JWT Bearer editor inside the auth tab: the algorithm and its key, the token's header and
//! claims, and where the signed token is sent.

use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{
    JwtAlgorithm, JwtPlacement, JwtSpec, OAuth1KeyKind, OAuth1PrivateKey, VariableSet,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, EventEmitter, SharedString, Subscription, Window, div};

use crate::auth_section::{SecretPlace, field, secret_field};
use crate::command::MOVE_SLOT;
use crate::oauth2_editor::{Choice, on_choice, pick};
use crate::{style, ui};

/// The whole spec after any change in the editor, with the owner's load stamp at sending.
pub struct JwtEdit(pub JwtSpec, pub u64);

/// "Move to a secret" on a field: the spec with `MOVE_SLOT` in place of that field.
pub struct JwtMoveSecret(pub JwtSpec);

/// One of the editor's text inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum JwtField {
    Secret,
    PrivateKey,
    Headers,
    Payload,
    ExpiresIn,
    HeaderPrefix,
    QueryParamKey,
}

const FIELDS: [JwtField; 7] = [
    JwtField::Secret,
    JwtField::PrivateKey,
    JwtField::Headers,
    JwtField::Payload,
    JwtField::ExpiresIn,
    JwtField::HeaderPrefix,
    JwtField::QueryParamKey,
];

impl JwtField {
    /// The input's element id.
    pub fn id(self) -> &'static str {
        match self {
            JwtField::Secret => "jwt-secret",
            JwtField::PrivateKey => "jwt-private-key",
            JwtField::Headers => "jwt-headers",
            JwtField::Payload => "jwt-payload",
            JwtField::ExpiresIn => "jwt-expires-in",
            JwtField::HeaderPrefix => "jwt-header-prefix",
            JwtField::QueryParamKey => "jwt-query-param-key",
        }
    }

    fn label(self) -> &'static str {
        match self {
            JwtField::Secret => "Secret",
            JwtField::PrivateKey => "Private key",
            JwtField::Headers => "Headers",
            JwtField::Payload => "Payload",
            JwtField::ExpiresIn => "Expires in (seconds)",
            JwtField::HeaderPrefix => "Header prefix",
            JwtField::QueryParamKey => "Query param name",
        }
    }

    fn masked(self) -> bool {
        matches!(self, JwtField::Secret | JwtField::PrivateKey)
    }

    fn placeholder(self) -> Option<&'static str> {
        match self {
            JwtField::Headers => Some("{\"kid\": \"key-1\"}"),
            JwtField::Payload => Some("{\"sub\": \"{{user}}\"}"),
            JwtField::ExpiresIn => Some("No expiry"),
            JwtField::QueryParamKey => Some("token"),
            _ => None,
        }
    }

    // The field's text in `j`; an absent key or prefix shows as empty.
    fn text(self, j: &JwtSpec) -> &str {
        match self {
            JwtField::Secret => &j.secret,
            JwtField::PrivateKey => j.private_key.as_ref().map_or("", |k| &k.value),
            JwtField::Headers => &j.headers,
            JwtField::Payload => &j.payload,
            JwtField::ExpiresIn => &j.expires_in,
            JwtField::HeaderPrefix => j.header_prefix.as_deref().unwrap_or(""),
            JwtField::QueryParamKey => &j.query_param_key,
        }
    }

    // Writes `text` into this field of `j`; a private key keeps its kind, `new_key` when it had none.
    fn set(self, j: &mut JwtSpec, text: String, new_key: OAuth1KeyKind) {
        match self {
            JwtField::Secret => j.secret = text,
            JwtField::PrivateKey => {
                let kind = j.private_key.as_ref().map_or(new_key, |k| k.kind);
                j.private_key = Some(OAuth1PrivateKey::new(kind, text));
            }
            JwtField::Headers => j.headers = text,
            JwtField::Payload => j.payload = text,
            JwtField::ExpiresIn => j.expires_in = text,
            JwtField::HeaderPrefix => j.header_prefix = Some(text),
            JwtField::QueryParamKey => j.query_param_key = text,
        }
    }
}

static ALGORITHMS: [(JwtAlgorithm, &str); 12] = [
    (JwtAlgorithm::Hs256, "HS256"),
    (JwtAlgorithm::Hs384, "HS384"),
    (JwtAlgorithm::Hs512, "HS512"),
    (JwtAlgorithm::Rs256, "RS256"),
    (JwtAlgorithm::Rs384, "RS384"),
    (JwtAlgorithm::Rs512, "RS512"),
    (JwtAlgorithm::Ps256, "PS256"),
    (JwtAlgorithm::Ps384, "PS384"),
    (JwtAlgorithm::Ps512, "PS512"),
    (JwtAlgorithm::Es256, "ES256"),
    (JwtAlgorithm::Es384, "ES384"),
    (JwtAlgorithm::Es512, "ES512"),
];
static KEY_KINDS: [(OAuth1KeyKind, &str); 2] =
    [(OAuth1KeyKind::Text, "Text"), (OAuth1KeyKind::File, "File")];
static PLACEMENTS: [(JwtPlacement, &str); 2] = [
    (JwtPlacement::Header, "Header"),
    (JwtPlacement::Query, "Query params"),
];

/// Edits a JWT auth. The loaded spec is the base each edit changes one field of, so the key of
/// the family not shown and unknown names survive an unrelated edit.
pub struct JwtEditor {
    base: JwtSpec,
    inputs: Vec<Entity<InputState>>,
    masks: ui::SecretMasks,
    algorithm: Choice,
    key_kind: Choice,
    /// The kind a key typed into an empty field gets; the file holds no kind without a key.
    new_key: OAuth1KeyKind,
    placement: Choice,
    vars: Option<Arc<VariableSet>>,
    place: SecretPlace,
    stamp: u64,
    _subs: Vec<Subscription>,
}

impl EventEmitter<JwtEdit> for JwtEditor {}
impl EventEmitter<JwtMoveSecret> for JwtEditor {}

impl JwtEditor {
    /// An editor showing a fresh JWT auth.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut choice = |id: &str, items: Vec<&str>| {
            let items = ui::id_items(id, items);
            cx.new(|cx| SelectState::new(items, None, window, cx))
        };
        let algorithm = choice(
            "jwt-algorithm",
            ALGORITHMS.iter().map(|(_, l)| *l).collect(),
        );
        let key_kind = choice("jwt-key-kind", KEY_KINDS.iter().map(|(_, l)| *l).collect());
        let placement = choice(
            "jwt-placement",
            PLACEMENTS.iter().map(|(_, l)| *l).collect(),
        );
        let inputs: Vec<Entity<InputState>> = FIELDS
            .iter()
            .map(|f| {
                cx.new(|cx| {
                    let state = InputState::new(window, cx).masked(f.masked());
                    match f.placeholder() {
                        Some(hint) => state.placeholder(hint),
                        None => state,
                    }
                })
            })
            .collect();
        // Each select goes through its setter, skipping a confirm of the value already shown.
        let mut subs = vec![
            on_choice(&algorithm, &ALGORITHMS, window, cx, |t, v, w, cx| {
                if t.base.algorithm != v {
                    t.set_algorithm(v, w, cx);
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
        // A keystroke sets only its own field and sends the spec; text equal to what the base
        // shows is no edit, so an absent prefix or key stays absent.
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
        let mut editor = JwtEditor {
            base: JwtSpec::default(),
            inputs,
            masks: ui::SecretMasks::default(),
            algorithm,
            key_kind,
            new_key: OAuth1KeyKind::Text,
            placement,
            vars: None,
            place: SecretPlace::default(),
            stamp: 0,
            _subs: subs,
        };
        let fresh = editor.base.clone();
        editor.show(&fresh, window, cx);
        editor
    }

    /// Shows `spec` without emitting; skipped when it is the spec already shown.
    pub fn load(&mut self, spec: &JwtSpec, window: &mut Window, cx: &mut Context<Self>) {
        if *spec != self.base {
            self.show(spec, window, cx);
        }
    }

    /// Sets the owner's load count that every edit sent from now on carries.
    pub fn set_stamp(&mut self, stamp: u64) {
        self.stamp = stamp;
    }

    /// The spec the editor describes.
    pub fn spec(&self, _: &App) -> JwtSpec {
        self.base.clone()
    }

    /// Sets where the secret fields say their values live.
    pub fn set_place(&mut self, place: SecretPlace, cx: &mut Context<Self>) {
        if self.place != place {
            self.place = place;
            cx.notify();
        }
    }

    /// Asks to move masked field `f`'s literal value to a secret.
    pub fn move_to_secret(&mut self, f: JwtField, cx: &mut Context<Self>) {
        let mut spec = self.base.clone();
        f.set(&mut spec, MOVE_SLOT.to_owned(), self.new_key);
        cx.emit(JwtMoveSecret(spec));
    }

    /// Ends the typing run of every input's own undo, as at a save or a reload.
    pub fn end_typing(&self, window: &mut Window, cx: &mut App) {
        for input in self.inputs.clone() {
            crate::ui::end_input_run(&input, window, cx);
        }
    }

    /// The input behind `field`.
    pub fn input(&self, field: JwtField) -> &Entity<InputState> {
        &self.inputs[field as usize]
    }

    /// Sets the signing algorithm; the HS family shows the secret, the others the private key.
    pub fn set_algorithm(&mut self, a: JwtAlgorithm, window: &mut Window, cx: &mut Context<Self>) {
        self.base.algorithm = a;
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

    /// Sets where the token is sent.
    pub fn set_placement(&mut self, p: JwtPlacement, window: &mut Window, cx: &mut Context<Self>) {
        self.base.placement = p;
        self.show_choices(window, cx);
        self.emit(cx);
    }

    /// Turns base64 decoding of the secret on or off.
    pub fn set_secret_base64(&mut self, on: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.base.secret_base64 = on;
        self.emit(cx);
    }

    /// Tint and explain placeholders in the unmasked fields.
    pub fn set_variables(&mut self, vars: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.vars = vars;
        cx.notify();
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        cx.emit(JwtEdit(self.base.clone(), self.stamp));
        cx.notify();
    }

    // Puts `spec` on screen as the new base; inputs change only when their text differs, so
    // an echo leaves the caret alone.
    fn show(&mut self, spec: &JwtSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.base = spec.clone();
        self.new_key = OAuth1KeyKind::Text;
        for f in FIELDS {
            let input = self.input(f).clone();
            self.masks.fill(&input, f.text(spec), window, cx);
        }
        self.show_choices(window, cx);
    }

    // Selects the base's value in each select, or nothing for a name it does not offer.
    fn show_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = &self.base;
        pick(&self.algorithm, &ALGORITHMS, &base.algorithm, window, cx);
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

impl Render for JwtEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for f in FIELDS.into_iter().filter(|f| f.masked()) {
            let input = self.input(f).clone();
            self.masks.apply(&input, window, cx);
        }
        let j = &self.base;
        let secret = j.algorithm.uses_secret();
        let header = j.placement != JwtPlacement::Query;
        let this = cx.entity().downgrade();
        let input = |f: JwtField, cx: &App| {
            let vars = self.vars.clone();
            if f.masked() {
                let this = this.clone();
                let mover: crate::auth_section::MoveToSecret = Rc::new(move |_, cx| {
                    this.update(cx, |e, cx| e.move_to_secret(f, cx)).ok();
                });
                secret_field(
                    f.label(),
                    f.id(),
                    self.input(f),
                    vars,
                    &self.place,
                    Some(mover),
                    cx,
                )
                .into_any_element()
            } else {
                field(f.label(), f.id(), self.input(f), vars, cx).into_any_element()
            }
        };
        let unknown_algorithm = match &j.algorithm {
            JwtAlgorithm::Other(s) => Some(s.clone()),
            _ => None,
        };
        let unknown_placement = match &j.placement {
            JwtPlacement::Other(s) => Some(s.clone()),
            _ => None,
        };
        let base64 = j.secret_base64;
        v_flex()
            .gap_2()
            .child(self.select(
                "Algorithm",
                "jwt-algorithm",
                &self.algorithm,
                unknown_algorithm,
            ))
            .when(secret, |el| {
                el.child(input(JwtField::Secret, cx)).child(ui::form_row(
                    "jwt-secret-base64-row",
                    "",
                    ui::checkbox("jwt-secret-base64", base64)
                        .label("Secret is base64 encoded")
                        .on_click(
                            cx.listener(|t, on: &bool, w, cx| t.set_secret_base64(*on, w, cx)),
                        ),
                ))
            })
            .when(!secret, |el| {
                el.child(input(JwtField::PrivateKey, cx)).child(self.select(
                    "Private key source",
                    "jwt-key-kind",
                    &self.key_kind,
                    None,
                ))
            })
            .child(input(JwtField::Headers, cx))
            .child(input(JwtField::Payload, cx))
            .child(input(JwtField::ExpiresIn, cx))
            .child(self.select(
                "Add token to",
                "jwt-placement",
                &self.placement,
                unknown_placement,
            ))
            .when(header, |el| el.child(input(JwtField::HeaderPrefix, cx)))
            .when(!header, |el| el.child(input(JwtField::QueryParamKey, cx)))
    }
}
