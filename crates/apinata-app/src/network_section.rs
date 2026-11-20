//! The collection's Network tab: the proxy choice and its fields, the client certificates,
//! and a way to move literal secrets out of the collection file.

use crate::ui::SolidRing as _;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use apinata_core::{
    CertificateKind, ClientCertificate, CollectionNetwork, CollectionProxy, HostOverride,
    ProxyAuth, ProxyConfig,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ElementId, Entity, EventEmitter, FocusHandle, Focusable, Hsla, PathPromptOptions, Pixels,
    Rems, SharedString, Subscription, TestSupportExt, Window, div, px,
};

use crate::body_section::collection_relative;
use crate::command::Edit;
use crate::kv_table::set_text;
use crate::network_secrets::env_name;
use crate::section::SectionEdit;
use crate::snapshot::{DotenvNames, SecretHome};
use crate::{style, ui};

/// How the collection picks its proxy, in segment order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ProxyChoice {
    Inherit,
    Off,
    Custom,
}

const CHOICES: [(ProxyChoice, &str); 3] = [
    (ProxyChoice::Inherit, "Inherit"),
    (ProxyChoice::Off, "Off"),
    (ProxyChoice::Custom, "Custom"),
];

/// The proxy protocols offered, in segment order.
pub const PROTOCOLS: [&str; 4] = ["http", "https", "socks4", "socks5"];

/// One of the custom proxy's text inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum NetworkField {
    Host,
    Port,
    Username,
    Password,
    Bypass,
}

/// One text input of a certificate row; a PKCS#12 row's `Certificate` is its bundle file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CertField {
    Domain,
    Certificate,
    Key,
    Passphrase,
}

/// One text input of a host override row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum HostField {
    Host,
    Address,
}

const HOST_INCOMPLETE: &str = "Not saved until both the host and the address are filled in.";
const PROXY_HOST_INCOMPLETE: &str = "Not saved until a host is filled in.";
const PROXY_HOST_FAILS: &str = "Requests fail until a host is filled in.";
const OUTSIDE_NOTICE: &str = "Outside the collection: saved as an absolute path";
const PLACEHOLDERS: [&str; 4] = ["Domain", "Certificate", "Key", "Passphrase"];
const BUNDLE_PLACEHOLDER: &str = "PKCS#12 file";

// The certificate types a row offers, in its type select's order.
const CERT_TYPES: [&str; 2] = ["PEM", "PKCS#12"];
// Fits the wider type, so every row's fields start at the same place.
const CERT_TYPE_WIDTH: Rems = style::ui(84.);

// The inputs of a row, indexed by `CertField` order.
struct CertInputs([Entity<InputState>; 4]);

// One certificate as loaded or edited. The value keeps its YAML origin; a row writes its
// typed text back into it, and its passphrase is the last committed one.
struct CertRow {
    cert: ClientCertificate,
    inputs: CertInputs,
    // An added row stays local until its domain and paths are filled, as the reference app
    // validates a row before adding it; loaded rows are never drafts.
    draft: bool,
    // The outside answer for the certificate and key texts it was computed from, so a render
    // reads it and only an edited path pays for the filesystem check again.
    outside: RefCell<Option<([String; 2], bool)>>,
    // Whether its `.env` passphrase is being replaced, so the passphrase input shows.
    replacing: bool,
    // The width the path input's text had when last drawn, which fits the shown path.
    file_room: Rc<Cell<Option<Pixels>>>,
    _subs: Vec<Subscription>,
}

impl CertRow {
    // Where the passphrase's `.env` value is when it shows the badge rather than its input.
    fn stored_home(&self, names: &DotenvNames) -> Option<SecretHome> {
        if self.replacing {
            return None;
        }
        names.home(env_name(&self.cert.passphrase)?)
    }

    fn outside(&self, root: &Path, canonical_root: Option<&Path>) -> bool {
        let [_, certificate, key, _] = cert_texts(&self.cert);
        let mut cache = self.outside.borrow_mut();
        match &*cache {
            Some((paths, answer)) if paths[0] == certificate && paths[1] == key => *answer,
            _ => {
                let answer = names_outside(root, canonical_root, &self.cert);
                *cache = Some(([certificate.to_owned(), key.to_owned()], answer));
                answer
            }
        }
    }
}

// One host override as loaded or edited. The value keeps its YAML origin, so an unedited row
// and the keys this tab does not know are written back as read.
struct HostRow {
    row: HostOverride,
    host: Entity<InputState>,
    address: Entity<InputState>,
    // Whether the row may show its error: a loaded row, or one edited and left. A fresh row
    // stays quiet until then.
    checked: bool,
    // Whether the user changed the row; leaving an untouched row does not check it.
    edited: bool,
    // A row added or edited without both a host and an address stays local until both are
    // filled, as an unfilled certificate row does; loaded rows are never drafts.
    draft: bool,
    _subs: Vec<Subscription>,
}

/// Edits the collection's proxy. The custom fields keep their text while another choice is
/// shown, so switching back restores them; the password is sent on Enter or blur only.
pub struct NetworkSection {
    root: PathBuf,
    // The root's resolved form, read once rather than per outside check.
    canonical_root: Option<PathBuf>,
    choice: ProxyChoice,
    // The choice the file holds, as last loaded or sent. A custom choice without a host is
    // not sent, so the file keeps this one meanwhile.
    filed: ProxyChoice,
    // A loaded protocol outside `PROTOCOLS`, such as a template, stays until a segment is
    // clicked.
    protocol: String,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    bypass: Entity<InputState>,
    // The password last loaded or sent; every edit carries it, never half-typed text.
    password_committed: String,
    // Whether the `.env` password is being replaced, so its input shows.
    password_replacing: bool,
    // The `.env` names that are stored or held for the collection, which decide the badges.
    dotenv: DotenvNames,
    // The port last loaded or typed valid; a refused port shows `port_error` and every edit
    // carries this one instead.
    port_committed: String,
    port_error: Option<String>,
    // The proxy fields last shown from a load, so an unchanged port keeps the form it was read in.
    loaded_proxy: ProxyConfig,
    certs: Vec<CertRow>,
    hosts: Vec<HostRow>,
    plain_secrets: bool,
    notice: Option<String>,
    // What changed outside the app, or what a never-seen collection sets.
    change: Option<crate::network_gate::Notice>,
    // Takes focus when the custom field holding it goes away, so the editor's keys still work.
    focus: FocusHandle,
    // Counts loads, so an edit can tell whether one came between its sending and delivery.
    loads: u64,
    // Collects the edits instead of emitting them while a save takes the pending typing.
    outbox: Option<Vec<Edit>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SectionEdit> for NetworkSection {}

/// The person asked to acknowledge the settings or to reset the acknowledgement record.
pub struct NetworkAction(pub crate::command::NetworkOp);

impl EventEmitter<NetworkAction> for NetworkSection {}

impl NetworkSection {
    /// A section showing an inherited proxy, for the collection at `root`.
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut input = |masked| cx.new(|cx| InputState::new(window, cx).masked(masked));
        let (host, port, username, password, bypass) = (
            input(false),
            input(false),
            input(false),
            input(true),
            input(false),
        );
        let mut subs = Vec::new();
        subs.push(cx.subscribe(&port, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                this.port_changed(text, cx);
            }
        }));
        for state in [&host, &username, &bypass] {
            subs.push(cx.subscribe(state, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.emit(cx);
                }
            }));
        }
        subs.push(cx.subscribe(&password, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_password(cx);
            }
        }));
        NetworkSection {
            canonical_root: root.canonicalize().ok(),
            root,
            choice: ProxyChoice::Inherit,
            filed: ProxyChoice::Inherit,
            protocol: PROTOCOLS[0].into(),
            host,
            port,
            username,
            password,
            bypass,
            password_committed: String::new(),
            password_replacing: false,
            dotenv: DotenvNames::default(),
            port_committed: String::new(),
            port_error: None,
            loaded_proxy: ProxyConfig::new(PROTOCOLS[0], "", ""),
            certs: Vec::new(),
            hosts: Vec::new(),
            plain_secrets: false,
            notice: None,
            change: None,
            focus: cx.focus_handle(),
            loads: 0,
            outbox: None,
            _subs: subs,
        }
    }

    /// Shows `network` from a snapshot without emitting an edit; `plain_secrets` offers the
    /// move of literal secrets to `.env`.
    pub fn load(
        &mut self,
        network: &CollectionNetwork,
        plain_secrets: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loads += 1;
        self.plain_secrets = plain_secrets;
        let (choice, fields) = match &network.proxy {
            CollectionProxy::Off => (ProxyChoice::Off, network.stored_proxy.as_ref()),
            CollectionProxy::Custom(proxy) => (ProxyChoice::Custom, Some(proxy)),
            _ => (ProxyChoice::Inherit, network.stored_proxy.as_ref()),
        };
        self.filed = choice;
        // A custom proxy still waiting for its host stays on screen with its typing while the
        // file holds another choice.
        let waiting = self.held_back(cx) && choice != ProxyChoice::Custom;
        if !waiting {
            self.choice = choice;
            // Inherit and Off keep the file's custom fields, so switching back sends them,
            // never blanks over them.
            if let Some(proxy) = fields {
                self.show_fields(proxy, window, cx);
            }
        }
        self.load_certificates(&network.certificates, window, cx);
        self.load_hosts(&network.host_overrides, window, cx);
        cx.notify();
    }

    /// Drops everything shown but never sent, as a discard does: a custom proxy still waiting
    /// for its host, a refused port, an uncommitted password or passphrase, unfinished rows.
    pub fn discard_unsent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.held_back(cx) {
            self.choice = self.filed;
        }
        let proxy = self.loaded_proxy.clone();
        set_text(&self.host, &proxy.hostname, window, cx);
        self.port_error = None;
        self.show_fields(&proxy, window, cx);
        self.password_replacing = false;
        let password = self.password_committed.clone();
        set_text(&self.password, &password, window, cx);
        self.certs.retain(|row| !row.draft);
        for row in &mut self.certs {
            row.replacing = false;
            let pass = &row.inputs.0[CertField::Passphrase as usize];
            set_text(pass, &row.cert.passphrase, window, cx);
        }
        self.hosts.retain(|slot| !slot.draft);
        cx.notify();
    }

    fn show_fields(&mut self, proxy: &ProxyConfig, window: &mut Window, cx: &mut Context<Self>) {
        // A snapshot echoing the last loaded host leaves a cleared, unsent host blank; any
        // other host replaces it.
        if !(self.held_back(cx) && proxy.hostname == self.loaded_proxy.hostname) {
            set_text(&self.host, &proxy.hostname, window, cx);
        }
        self.loaded_proxy = proxy.clone();
        self.protocol.clone_from(&proxy.protocol);
        // A snapshot echoing the last valid port leaves a refused port's typing and error in
        // place; any other port replaces both.
        if self.port_error.is_none() || proxy.port != self.port_committed {
            set_text(&self.port, &proxy.port, window, cx);
            self.port_committed.clone_from(&proxy.port);
            self.port_error = None;
        }
        set_text(&self.bypass, &proxy.bypass, window, cx);
        let (user, password) = proxy
            .auth
            .as_ref()
            .map_or(("", ""), |a| (&a.username, &a.password));
        set_text(&self.username, user, window, cx);
        // The password commits on Enter or blur, so typing over the committed value stays until
        // the file changes that value itself.
        if password != self.password_committed {
            set_text(&self.password, password, window, cx);
            self.password_committed = password.to_string();
        }
    }

    // Rows keep their input entities while the count matches, so a snapshot echoing the
    // user's own typing leaves focus and caret alone. The file never holds a draft, so drafts
    // are left out of the match and kept after the loaded rows.
    fn load_certificates(
        &mut self,
        certs: &[ClientCertificate],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.certs.iter().filter(|row| !row.draft).count() != certs.len() {
            let drafts: Vec<CertRow> = std::mem::take(&mut self.certs)
                .into_iter()
                .filter(|row| row.draft)
                .collect();
            self.certs = certs
                .iter()
                .map(|cert| self.new_row(cert.clone(), window, cx))
                .collect();
            self.certs.extend(drafts);
            return;
        }
        // The passphrase commits on Enter or blur, so its typing stays while the file keeps
        // the same certificate (domain and paths) with the passphrase last committed. A
        // PKCS#12 row keeps its hidden key text, so switching back to PEM restores it.
        let rows = self.certs.iter_mut().filter(|row| !row.draft);
        for (row, cert) in rows.zip(certs) {
            let typed_over = row.cert.passphrase == cert.passphrase
                && row.cert.domain == cert.domain
                && row.cert.kind == cert.kind;
            if is_pem(&row.cert) != is_pem(cert) {
                set_file_placeholder(&row.inputs, cert, window, cx);
            }
            row.cert = cert.clone();
            for (i, (state, text)) in row.inputs.0.iter().zip(cert_texts(cert)).enumerate() {
                let kept = (typed_over && i == CertField::Passphrase as usize)
                    || (!is_pem(cert) && i == CertField::Key as usize);
                if !kept {
                    set_text(state, text, window, cx);
                }
            }
        }
    }

    // Rows keep their input entities while the count matches, so a snapshot echoing the user's
    // own typing leaves focus and caret alone. The file never holds a draft, so drafts are left
    // out of the match and kept after the loaded rows.
    fn load_hosts(&mut self, rows: &[HostOverride], window: &mut Window, cx: &mut Context<Self>) {
        if self.hosts.iter().filter(|slot| !slot.draft).count() != rows.len() {
            let drafts: Vec<HostRow> = std::mem::take(&mut self.hosts)
                .into_iter()
                .filter(|slot| slot.draft)
                .collect();
            self.hosts = rows
                .iter()
                .map(|row| {
                    let mut slot = self.new_host_row(row.clone(), window, cx);
                    slot.checked = true;
                    slot
                })
                .collect();
            self.hosts.extend(drafts);
            return;
        }
        let committed = self.hosts.iter_mut().filter(|slot| !slot.draft);
        for (slot, row) in committed.zip(rows) {
            slot.row = row.clone();
            set_text(&slot.host, &row.host, window, cx);
            set_text(&slot.address, &row.address, window, cx);
        }
    }

    fn new_host_row(
        &self,
        row: HostOverride,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> HostRow {
        let input =
            |placeholder: &'static str, text: &str, window: &mut Window, cx: &mut Context<Self>| {
                let text = text.to_owned();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(text)
                })
            };
        let host = input("Host", &row.host, window, cx);
        let address = input("Address", &row.address, window, cx);
        // A row is found by its input entity, since removing a row shifts the indices.
        let subs = [&host, &address]
            .into_iter()
            .map(|state| {
                cx.subscribe_in(
                    state,
                    window,
                    |this, state, event: &InputEvent, window, cx| match event {
                        InputEvent::Change => this.host_text_changed(state, cx),
                        InputEvent::Blur => this.host_left(state, window, cx),
                        _ => {}
                    },
                )
            })
            .collect();
        HostRow {
            row,
            host,
            address,
            checked: false,
            edited: false,
            draft: false,
            _subs: subs,
        }
    }

    // Leaving a row lets it show its error; moving between its two inputs does not.
    fn host_left(&mut self, state: &Entity<InputState>, window: &Window, cx: &mut Context<Self>) {
        let Some(slot) = self
            .hosts
            .iter_mut()
            .find(|slot| slot.host == *state || slot.address == *state)
        else {
            return;
        };
        let inside = [&slot.host, &slot.address]
            .iter()
            .any(|i| i.read(cx).focus_handle(cx).is_focused(window));
        if !inside && slot.edited {
            slot.checked = true;
            cx.notify();
        }
    }

    /// The host override rows as they would be sent, in order.
    pub fn host_overrides(&self) -> Vec<HostOverride> {
        let sent = self.hosts.iter().filter(|slot| !slot.draft);
        sent.map(|slot| slot.row.clone()).collect()
    }

    /// Appends an empty host override row, sent once its host and address are filled.
    pub fn add_host_override(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut slot = self.new_host_row(HostOverride::new("", ""), window, cx);
        slot.draft = true;
        slot.host.focus_handle(cx).focus(window, cx);
        self.hosts.push(slot);
        cx.notify();
    }

    /// Removes host override row `row`, moves focus to the next row's host, else the previous
    /// one's, else the section itself (so the editor's keys keep working), and sends the list.
    pub fn remove_host_override(
        &mut self,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row < self.hosts.len() {
            let draft = self.hosts.remove(row).draft;
            let next = self
                .hosts
                .get(row)
                .or_else(|| self.hosts.get(row.wrapping_sub(1)));
            match next {
                Some(slot) => slot.host.focus_handle(cx).focus(window, cx),
                None => self.focus.focus(window, cx),
            }
            match draft {
                true => cx.notify(),
                false => self.emit_hosts(cx),
            }
        }
    }

    /// The input behind `field` of host override row `row`.
    pub fn host_input(&self, row: usize, field: HostField) -> Option<&Entity<InputState>> {
        let slot = self.hosts.get(row)?;
        Some(match field {
            HostField::Host => &slot.host,
            HostField::Address => &slot.address,
        })
    }

    // Copies the row's typed text into its value and sends the list when it changed.
    fn host_text_changed(&mut self, state: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(slot) = self
            .hosts
            .iter_mut()
            .find(|slot| slot.host == *state || slot.address == *state)
        else {
            return;
        };
        let before = slot.row.clone();
        slot.row.host = slot.host.read(cx).value().to_string();
        slot.row.address = slot.address.read(cx).value().to_string();
        if slot.row != before {
            slot.edited = true;
            // A row that was and stays incomplete changes nothing the file holds; one that
            // turns incomplete leaves the list, one that completes joins it.
            let was_draft = slot.draft;
            slot.draft = incomplete(&slot.row);
            match was_draft && slot.draft {
                true => cx.notify(),
                false => self.emit_hosts(cx),
            }
        }
    }

    fn set_host_enabled(&mut self, row: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(slot) = self.hosts.get_mut(row) {
            slot.row.enabled = on;
            match slot.draft {
                true => cx.notify(),
                false => self.emit_hosts(cx),
            }
        }
    }

    fn emit_hosts(&mut self, cx: &mut Context<Self>) {
        self.send(
            SectionEdit::new(Edit::HostOverrides(self.host_overrides())).after_loads(self.loads),
            cx,
        );
        cx.notify();
    }

    fn new_row(
        &self,
        cert: ClientCertificate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CertRow {
        let texts = cert_texts(&cert);
        let inputs = std::array::from_fn(|i| {
            let placeholder = match i == CertField::Certificate as usize && !is_pem(&cert) {
                true => BUNDLE_PLACEHOLDER,
                false => PLACEHOLDERS[i],
            };
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(i == CertField::Passphrase as usize)
                    .placeholder(placeholder)
                    .default_value(texts[i].to_string())
            })
        });
        // A row is found by its input entity, since removing a row shifts the indices.
        let subs = inputs
            .iter()
            .enumerate()
            .map(|(i, state)| {
                cx.subscribe(state, move |this, state, event: &InputEvent, cx| {
                    let passphrase = i == CertField::Passphrase as usize;
                    let commits = if passphrase {
                        matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
                    } else {
                        matches!(event, InputEvent::Change)
                    };
                    if commits {
                        this.cert_text_changed(&state, cx);
                    }
                })
            })
            .collect();
        CertRow {
            cert,
            inputs: CertInputs(inputs),
            draft: false,
            outside: RefCell::default(),
            replacing: false,
            file_room: Rc::default(),
            _subs: subs,
        }
    }

    /// The certificate rows as they would be sent, in order.
    pub fn certificates(&self) -> Vec<ClientCertificate> {
        let sent = self.certs.iter().filter(|row| !row.draft);
        sent.map(|row| row.cert.clone()).collect()
    }

    /// Appends an empty PEM row, sent once its domain and paths are filled.
    pub fn add_certificate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut row = self.new_row(ClientCertificate::pem("", "", ""), window, cx);
        row.draft = true;
        let domain = row.inputs.0[CertField::Domain as usize].clone();
        self.certs.push(row);
        domain.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Removes row `row`, moves focus to the next row's domain, else the previous
    /// one's, else the section itself (so the editor's keys keep working), and sends the list.
    pub fn remove_certificate(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if row < self.certs.len() {
            let draft = self.certs.remove(row).draft;
            let domain = |r: &CertRow| r.inputs.0[CertField::Domain as usize].clone();
            let next = self.certs.get(row).map(domain);
            let prev = row
                .checked_sub(1)
                .and_then(|i| self.certs.get(i))
                .map(domain);
            match next.or(prev) {
                Some(input) => input.focus_handle(cx).focus(window, cx),
                None => self.focus.focus(window, cx),
            }
            if draft {
                cx.notify();
            } else {
                self.emit_certificates(cx);
            }
        }
    }

    /// The input behind `field` of row `row`; `None` for no such row.
    pub fn cert_input(&self, row: usize, field: CertField) -> Option<&Entity<InputState>> {
        Some(&self.certs.get(row)?.inputs.0[field as usize])
    }

    // Copies the row's typed text into its value and sends it when it changed; the
    // passphrase input reports here on Enter or blur only.
    fn cert_text_changed(&mut self, state: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(ix) = self
            .certs
            .iter()
            .position(|row| row.inputs.0.contains(state))
        else {
            return;
        };
        let row = &mut self.certs[ix];
        let before = row.cert.clone();
        let [domain, _, _, passphrase] = row
            .inputs
            .0
            .each_ref()
            .map(|s| s.read(cx).value().to_string());
        row.cert.domain = domain;
        row.cert.kind = typed_kind(&row.inputs, is_pem(&row.cert), cx);
        // Other fields carry the last committed passphrase, never half-typed text. A
        // stored passphrase stays unless replaced, and a replace left empty keeps it.
        if *state == row.inputs.0[CertField::Passphrase as usize] {
            let replacing = std::mem::take(&mut row.replacing);
            let stored = row.stored_home(&self.dotenv).is_some();
            let keep = if replacing {
                passphrase.is_empty()
            } else {
                stored
            };
            if !keep {
                row.cert.passphrase = passphrase;
            }
            if replacing {
                cx.notify();
            }
        }
        self.settle_cert(ix, before, cx);
    }

    // Switches row `ix` between PEM and PKCS#12. The typed paths stay in their inputs, so the
    // certificate file becomes the bundle file and switching back restores the key.
    fn set_cert_type(&mut self, ix: usize, pem: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.certs.get_mut(ix) else {
            return;
        };
        if is_pem(&row.cert) == pem {
            return;
        }
        let before = row.cert.clone();
        row.cert.kind = typed_kind(&row.inputs, pem, cx);
        set_file_placeholder(&row.inputs, &row.cert, window, cx);
        self.settle_cert(ix, before, cx);
    }

    // Sends the list after row `ix` changed from `before`; a draft stays local until complete.
    fn settle_cert(&mut self, ix: usize, before: ClientCertificate, cx: &mut Context<Self>) {
        let row = &mut self.certs[ix];
        let completed = row.draft && is_complete(&row.cert);
        if row.draft && !completed {
            cx.notify();
            return;
        }
        row.draft = false;
        if row.cert != before || completed {
            self.emit_certificates(cx);
        }
    }

    fn set_cert_enabled(&mut self, row: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(row) = self.certs.get_mut(row) {
            row.cert.enabled = on;
            match row.draft {
                true => cx.notify(),
                false => self.emit_certificates(cx),
            }
        }
    }

    // Asks for one file and puts it, relative to the collection when inside it, into the
    // row's certificate or key. A cancelled prompt does nothing.
    fn pick(&mut self, row: usize, field: CertField, window: &mut Window, cx: &mut Context<Self>) {
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Some(paths) =
                crate::file_dialog::picked(reply, "choose a certificate file", cx).await
            else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(path) = paths.first() else {
                    return;
                };
                let text = collection_relative(&this.root, path);
                if let Some(state) = this.cert_input(row, field).cloned() {
                    set_text(&state, &text, window, cx);
                    this.cert_text_changed(&state, cx);
                }
            });
        })
        .detach();
    }

    // Typed but uncommitted passphrases go first, as the proxy password does.
    fn commit_passphrases(&mut self, cx: &mut Context<Self>) {
        let states: Vec<_> = self
            .certs
            .iter()
            .map(|row| row.inputs.0[CertField::Passphrase as usize].clone())
            .collect();
        for state in states {
            self.cert_text_changed(&state, cx);
        }
    }

    /// Whether a password or passphrase still being typed would send an edit on Enter or blur,
    /// so a close asks to save it as it does for other unsent secrets.
    pub fn holds_pending(&self, cx: &App) -> bool {
        // A replace left empty keeps the stored value, and a stored one stays unless replaced.
        let sends = |text: &str, before: &str, replacing: bool, stored: bool| {
            text != before && if replacing { !text.is_empty() } else { !stored }
        };
        let password = self.password.read(cx).value();
        let proxy = self.choice == ProxyChoice::Custom
            && !self.held_back(cx)
            && sends(
                &password,
                &self.password_committed,
                self.password_replacing,
                self.password_home().is_some(),
            );
        proxy
            || self.certs.iter().filter(|row| !row.draft).any(|row| {
                let typed = row.inputs.0[CertField::Passphrase as usize]
                    .read(cx)
                    .value();
                let stored = row.stored_home(&self.dotenv).is_some();
                sends(&typed, &row.cert.passphrase, row.replacing, stored)
            })
    }

    /// The edits a password or passphrase still being typed owes, as Enter or blur would send
    /// them. They are returned rather than emitted, so a save can send them ahead of itself.
    pub fn take_pending(&mut self, cx: &mut Context<Self>) -> Vec<Edit> {
        self.outbox = Some(Vec::new());
        self.commit_password(cx);
        self.commit_passphrases(cx);
        self.outbox.take().unwrap_or_default()
    }

    fn send(&mut self, edit: SectionEdit, cx: &mut Context<Self>) {
        match &mut self.outbox {
            Some(outbox) => outbox.push(edit.edit),
            None => cx.emit(edit),
        }
    }

    fn emit_certificates(&mut self, cx: &mut Context<Self>) {
        self.send(
            SectionEdit::new(Edit::ClientCertificates(self.certificates())).after_loads(self.loads),
            cx,
        );
        cx.notify();
    }

    /// The proxy choice shown.
    pub fn choice(&self) -> ProxyChoice {
        self.choice
    }

    /// Shows `choice` and sends the proxy it describes.
    pub fn set_choice(&mut self, choice: ProxyChoice, window: &mut Window, cx: &mut Context<Self>) {
        // A typed password goes out under the old choice first; the blur the focus move causes
        // would come only after the choice has changed.
        self.commit_password(cx);
        let fields = [
            &self.host,
            &self.port,
            &self.username,
            &self.password,
            &self.bypass,
        ];
        let in_fields = fields
            .iter()
            .any(|f| f.read(cx).focus_handle(cx).is_focused(window));
        if choice != ProxyChoice::Custom && in_fields {
            self.focus.focus(window, cx);
        }
        if self.choice != choice {
            self.choice = choice;
            self.emit(cx);
        }
    }

    /// The input behind `field`.
    pub fn input(&self, field: NetworkField) -> &Entity<InputState> {
        match field {
            NetworkField::Host => &self.host,
            NetworkField::Port => &self.port,
            NetworkField::Username => &self.username,
            NetworkField::Password => &self.password,
            NetworkField::Bypass => &self.bypass,
        }
    }

    /// Shows `notice` under the section, or clears it.
    pub fn set_notice(&mut self, notice: Option<String>, cx: &mut Context<Self>) {
        self.notice = notice;
        cx.notify();
    }

    /// Shows what changed outside the app above the proxy settings, or clears it.
    pub fn set_change_notice(
        &mut self,
        notice: Option<crate::network_gate::Notice>,
        cx: &mut Context<Self>,
    ) {
        if self.change != notice {
            self.change = notice;
            cx.notify();
        }
    }

    /// The outside-change notice shown, if any.
    pub fn change_notice(&self) -> Option<&crate::network_gate::Notice> {
        self.change.as_ref()
    }

    /// The notice shown, from the last refused or warned network edit.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    fn set_protocol(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(protocol) = PROTOCOLS.get(ix) {
            self.protocol = (*protocol).into();
            self.emit(cx);
        }
    }

    /// The warnings a save gives for typed values this tab cannot take: a refused port, whose
    /// last valid value is what gets saved.
    pub fn unsaved(&self, cx: &App) -> Vec<String> {
        let mut out: Vec<String> = self
            .port_error
            .iter()
            .map(|reason| format!("Not saved: Port: {reason}"))
            .collect();
        if self.held_back(cx) && !self.saved_without_host() {
            out.push("Not saved: Proxy: fill in a host to use a custom proxy".into());
        }
        if self.hosts.iter().any(|slot| slot.draft) {
            out.push("Not saved: Host override: fill in both the host and the address".into());
        }
        out
    }

    // Whether the file itself holds a custom proxy without a host, which fails every request.
    fn saved_without_host(&self) -> bool {
        self.filed == ProxyChoice::Custom && self.loaded_proxy.hostname.trim().is_empty()
    }

    // Whether a custom proxy is shown without a host, which is never sent.
    fn held_back(&self, cx: &App) -> bool {
        self.choice == ProxyChoice::Custom && self.host.read(cx).value().trim().is_empty()
    }

    // A port is blank, 1 to 65535 or a template; anything else stays typed but unsent.
    fn port_changed(&mut self, text: String, cx: &mut Context<Self>) {
        match crate::preferences::parse_proxy_port(&text) {
            Err(e) if !crate::network_secrets::is_template(&text) => {
                self.port_error = Some(e);
                cx.notify();
            }
            _ => {
                self.port_error = None;
                self.port_committed = text;
                self.emit(cx);
            }
        }
    }

    // Sends the typed password once per change, as the environment table does for secrets.
    // A stored password stays unless replaced, and a replace left empty keeps it.
    fn commit_password(&mut self, cx: &mut Context<Self>) {
        let text = self.password.read(cx).value().to_string();
        let replacing = std::mem::take(&mut self.password_replacing);
        let keep = if replacing {
            text.is_empty()
        } else {
            self.password_home().is_some()
        };
        if replacing {
            cx.notify();
        }
        if !keep && text != self.password_committed {
            self.password_committed = text;
            self.emit(cx);
        }
    }

    /// Sets the `.env` names stored or held for the collection, which decide the badges.
    pub fn set_dotenv(&mut self, names: DotenvNames, cx: &mut Context<Self>) {
        if self.dotenv != names {
            self.dotenv = names;
            cx.notify();
        }
    }

    // Where the proxy password's `.env` value is when it shows the badge rather than its input.
    fn password_home(&self) -> Option<SecretHome> {
        if self.password_replacing {
            return None;
        }
        self.dotenv.home(env_name(&self.password_committed)?)
    }

    // Opens the proxy password empty and focused for a new value in place of the stored one.
    fn replace_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password_replacing = true;
        let input = self.password.clone();
        set_text(&input, "", window, cx);
        input.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    // Opens row `ix`'s passphrase empty and focused for a new value in place of the stored one.
    fn replace_passphrase(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.certs.get_mut(ix) else {
            return;
        };
        row.replacing = true;
        let input = row.inputs.0[CertField::Passphrase as usize].clone();
        set_text(&input, "", window, cx);
        input.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    // A password typed but not yet committed goes first, since a click does not blur it.
    fn move_secrets(&mut self, cx: &mut Context<Self>) {
        self.commit_password(cx);
        self.commit_passphrases(cx);
        cx.emit(SectionEdit::new(Edit::MoveNetworkSecrets));
    }

    /// Whether `edit`, which this section sent, was built from settings a load has since
    /// replaced: a load came after it and the proxy, certificates or host overrides it would
    /// send now differ. Without a load between, a handler's first of two edits never lags.
    pub fn lags(&self, edit: &SectionEdit, cx: &App) -> bool {
        if edit.loads == self.loads {
            return false;
        }
        match &edit.edit {
            Edit::Proxy(sent) => *sent != self.current(cx),
            Edit::ClientCertificates(sent) => *sent != self.certificates(),
            Edit::HostOverrides(sent) => *sent != self.host_overrides(),
            _ => false,
        }
    }

    fn current(&self, cx: &App) -> CollectionProxy {
        let text = |state: &Entity<InputState>| state.read(cx).value().to_string();
        match self.choice {
            ProxyChoice::Inherit => CollectionProxy::Inherit,
            ProxyChoice::Off => CollectionProxy::Off,
            ProxyChoice::Custom => {
                let mut proxy = self.loaded_proxy.clone();
                proxy.protocol.clone_from(&self.protocol);
                proxy.hostname = text(&self.host);
                proxy.set_port(self.port_committed.clone());
                proxy.auth = None;
                let username = text(&self.username);
                if !username.is_empty() || !self.password_committed.is_empty() {
                    proxy.auth = Some(ProxyAuth::new(username, self.password_committed.clone()));
                }
                proxy.bypass = text(&self.bypass);
                CollectionProxy::Custom(proxy)
            }
        }
    }

    // A custom proxy without a host is held back, and so is a return to the choice the file
    // already holds, so an abandoned custom proxy leaves the file as it was.
    fn emit(&mut self, cx: &mut Context<Self>) {
        let unchanged = self.choice != ProxyChoice::Custom && self.choice == self.filed;
        if !self.held_back(cx) && !unchanged {
            let proxy = self.current(cx);
            self.filed = self.choice;
            self.send(
                SectionEdit::new(Edit::Proxy(proxy)).after_loads(self.loads),
                cx,
            );
        }
        cx.notify();
    }

    fn custom_rows(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let protocol = PROTOCOLS
            .iter()
            .position(|p| *p == self.protocol)
            .unwrap_or(usize::MAX);
        let protocols = ui::segmented("proxy-protocol", PROTOCOLS, protocol, move |ix, _, cx| {
            this.update(cx, |s, cx| s.set_protocol(*ix, cx))
        });
        // The compact cell input the certificate and host grids use, so the tab has one style.
        let field = |id: &'static str, state: &Entity<InputState>, width: Rems| {
            ui::padded_input(
                ui::table_input(state, false, window, cx).id(id).w(width),
                state,
            )
        };
        let mut host_row = ui::form_row(
            "proxy-host-row",
            "Host",
            field("proxy-host", &self.host, style::WIDE_INPUT_WIDTH),
        );
        if self.held_back(cx) {
            host_row = match self.saved_without_host() {
                true => host_row.hint("proxy-host-fails", PROXY_HOST_FAILS),
                false => host_row.hint("proxy-host-incomplete", PROXY_HOST_INCOMPLETE),
            };
        }
        v_flex()
            .gap(style::SPACE_2)
            .child(ui::form_row("proxy-protocol-row", "Protocol", protocols))
            .child(host_row)
            .child(ui::form_row(
                "proxy-port-row",
                "Port",
                ui::invalid_input(
                    ui::table_input(&self.port, false, window, cx)
                        .id("proxy-port")
                        .w(style::SHORT_INPUT_WIDTH),
                    div().w(style::SHORT_INPUT_WIDTH),
                    &self.port,
                    self.port_error.is_some(),
                    (
                        "proxy-port-danger-ring".into(),
                        "proxy-port-invalid-icon".into(),
                    ),
                    window,
                    cx,
                ),
            ))
            .when_some(self.port_error.clone(), |el, error| {
                el.child(ui::form_note("proxy-port-error", error).danger())
            })
            .child(ui::form_row(
                "proxy-username-row",
                "Username",
                field("proxy-username", &self.username, style::WIDE_INPUT_WIDTH),
            ))
            .child(ui::form_row(
                "proxy-password-row",
                "Password",
                if let Some(home) = self.password_home() {
                    ui::stored_secret(
                        "proxy-password-stored",
                        "proxy-password-replace",
                        home,
                        cx.listener(|this, _, window, cx| this.replace_password(window, cx)),
                        cx,
                    )
                    .into_any_element()
                } else {
                    field("proxy-password", &self.password, style::WIDE_INPUT_WIDTH)
                        .into_any_element()
                },
            ))
            .child(
                ui::form_row(
                    "proxy-bypass-row",
                    "Bypass",
                    field("proxy-bypass", &self.bypass, style::WIDE_INPUT_WIDTH),
                )
                .hint(
                    "proxy-bypass-note",
                    "Comma separated hosts that connect directly.",
                ),
            )
    }
}

impl NetworkSection {
    fn host_rows(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows = self
            .hosts
            .iter()
            .enumerate()
            .map(|(ix, slot)| {
                let reason = (!slot.draft)
                    .then(|| slot.row.check().err().map(|e| e.to_string()))
                    .flatten();
                let draft = slot.draft;
                let error = reason.clone().filter(|_| slot.row.enabled && slot.checked);
                let muted = reason.filter(|_| !slot.row.enabled && slot.checked);
                // check() does not say which field failed, so both inputs are marked, by
                // the icon alone while unfocused like any other invalid input.
                let framed = |id: &'static str,
                              state: &Entity<InputState>,
                              ring: &'static str,
                              icon: &'static str,
                              cx: &App| {
                    let frame = div()
                        .flex_1()
                        .border_1()
                        .rounded(style::RADIUS)
                        .border_color(gpui_kit::transparent_black());
                    ui::invalid_input(
                        ui::table_input(state, false, window, cx)
                            .id((id, ix))
                            .flex_1(),
                        frame,
                        state,
                        error.is_some(),
                        ((ring, ix).into(), (icon, ix).into()),
                        window,
                        cx,
                    )
                };
                let row = h_flex()
                    .id(("host-row", ix))
                    .test_support()
                    .h(style::ROW_HEIGHT)
                    .gap(style::SPACE_2)
                    .items_center()
                    .w_full()
                    .child(
                        ui::checkbox(("host-enabled", ix), slot.row.enabled)
                            .tab_stop(false)
                            .on_click(cx.listener(move |this, on: &bool, _, cx| {
                                this.set_host_enabled(ix, *on, cx)
                            })),
                    )
                    .child(framed(
                        "host-name",
                        &slot.host,
                        "host-danger-ring",
                        "host-invalid-icon",
                        cx,
                    ))
                    .child(framed(
                        "host-address",
                        &slot.address,
                        "address-danger-ring",
                        "address-invalid-icon",
                        cx,
                    ))
                    .child(
                        ui::icon_button(
                            ("host-remove", ix),
                            IconName::Close,
                            "Remove host override",
                            None,
                            cx,
                        )
                        .tab_stop(false)
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.remove_host_override(ix, window, cx),
                        )),
                    );
                v_flex()
                    .gap(style::SPACE_1)
                    .child(row)
                    .when(draft, |el| {
                        el.child(host_note(
                            ("host-incomplete", ix),
                            HOST_INCOMPLETE,
                            cx.theme().muted_foreground,
                        ))
                    })
                    .when_some(error, |el, reason| {
                        el.child(host_note(("host-error", ix), reason, cx.theme().danger))
                    })
                    .when_some(muted, |el, reason| {
                        let text = format!("Not used until the address is fixed: {reason}");
                        el.child(host_note(
                            ("host-muted", ix),
                            text,
                            cx.theme().muted_foreground,
                        ))
                    })
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(style::SPACE_2)
            .child(ui::form_heading("hosts-heading", "Host overrides"))
            .child(muted_note(
                "hosts-note",
                "Send a host name to an address instead of its DNS answer. A port in the address counts only for a URL that names none.",
                cx,
            ))
            .children(rows)
            .child(
                h_flex().child(
                    Button::new("host-add").solid_ring(cx)
                        .label("Add host override")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_host_override(window, cx)
                        })),
                ),
            )
    }

    fn certificate_rows(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let rows = self
            .certs
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                v_flex()
                    .gap(style::SPACE_1)
                    .child(self.cert_row(ix, row, window, cx))
                    .when(
                        row.outside(&self.root, self.canonical_root.as_deref()),
                        |el| el.child(muted_note(("cert-outside", ix), OUTSIDE_NOTICE, cx)),
                    )
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(style::SPACE_2)
            .child(ui::form_heading("cert-heading", "Client certificates"))
            .child(muted_note("cert-note", CERT_NOTE, cx))
            .children(rows)
            .child(
                h_flex().child(
                    Button::new("cert-add")
                        .solid_ring(cx)
                        .label("Add certificate")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_certificate(window, cx)),
                        ),
                ),
            )
    }

    // The certificate path input, covered while unfocused by the path with its middle folders
    // elided when the whole path does not fit, so the file name stays in view. The cover
    // takes no pointer events, so a click lands in the input and shows the whole path.
    fn path_cell(
        &self,
        ix: usize,
        row: &CertRow,
        input: gpui_kit::AnyElement,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = &row.inputs.0[CertField::Certificate as usize];
        let size = gpui_kit::rems(PATH_TEXT_REMS).to_pixels(window.rem_size());
        let font = gpui_kit::font(cx.theme().mono_font_family.clone());
        let shown = row.file_room.get().and_then(|room| {
            if state.focus_handle(cx).is_focused(window) {
                return None;
            }
            let path = state.read(cx).value().to_string();
            let measure = crate::path_display::font_measurer(window, font, size);
            let shown =
                crate::path_display::display(Path::new(&path), None, f32::from(room), &measure);
            (shown != path).then_some(SharedString::from(shown))
        });
        // A changed room redraws the section once the frame is done, so the path refits.
        let measure = {
            let (cell, me) = (Rc::clone(&row.file_room), cx.entity().downgrade());
            gpui_kit::canvas(
                move |bounds, window, cx| {
                    let now = Some(bounds.size.width - style::SMALL_INPUT_TEXT_INSET * 2.);
                    if cell.replace(now) != now {
                        window.defer(cx, move |_, cx| {
                            me.update(cx, |_, cx| cx.notify()).ok();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        let cover = shown.map(|shown| {
            div()
                .id(("cert-file-shown", ix))
                .test_support()
                .absolute()
                .top(px(1.))
                .bottom(px(1.))
                .left(px(1.))
                .right(px(1.))
                .pl(style::SMALL_INPUT_TEXT_INSET - px(1.))
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                // Opaque, so the input's own text cannot show through: the field's translucent
                // fill as it lands on the page's background.
                .rounded(cx.theme().radius - px(1.))
                .bg(cx.theme().background.blend(ui::field_fill(false, cx)))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(gpui_kit::rems(PATH_TEXT_REMS))
                .text_color(cx.theme().foreground)
                // No accessible name of its own: the input under it already reads the path.
                .child(div().child(shown.clone()).test_text(shown))
        });
        h_flex()
            .relative()
            .flex_1()
            .min_w_0()
            .child(input)
            .child(measure)
            .children(cover)
    }

    fn cert_row(
        &self,
        ix: usize,
        row: &CertRow,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let [domain, file, key, passphrase] = &row.inputs.0;
        let pem = is_pem(&row.cert);
        let field = |id: &'static str, state: &Entity<InputState>, cx: &App| {
            ui::padded_input(
                ui::table_input(state, false, window, cx)
                    .id((id, ix))
                    .flex_1(),
                state,
            )
        };
        let picker = |id: &'static str, tip: &str, which: CertField, cx: &mut Context<Self>| {
            ui::icon_button((id, ix), IconName::FolderOpen, tip, None, cx)
                .tab_stop(false)
                .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, which, window, cx)))
        };
        // A path and its picker share one column.
        let column = || {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(style::SPACE_2)
                .items_center()
        };
        let this = cx.entity();
        let kind = ui::select(
            format!("cert-type-{ix}"),
            "Certificate type",
            CERT_TYPES[usize::from(!pem)].into(),
            CERT_TYPES.map(SharedString::from).to_vec(),
            usize::from(!pem),
            move |pick, window, cx| {
                this.update(cx, |s, cx| s.set_cert_type(ix, pick == 0, window, cx));
            },
            cx,
        );
        let file_pick = match pem {
            true => "Choose certificate",
            false => "Choose PKCS#12 file",
        };
        h_flex()
            .id(("cert-row", ix))
            .test_support()
            .h(style::ROW_HEIGHT)
            .gap(style::SPACE_2)
            .items_center()
            .w_full()
            .child(
                ui::checkbox(("cert-enabled", ix), row.cert.enabled)
                    .tab_stop(false)
                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                        this.set_cert_enabled(ix, *on, cx)
                    })),
            )
            .child(div().flex_none().w(CERT_TYPE_WIDTH).child(kind))
            .child(field("cert-domain", domain, cx))
            // Both path columns share one box, which a bundle's one column fills alone, so
            // its path takes the key's room and every row's passphrase lines up.
            .child(
                h_flex()
                    .flex_1()
                    .flex_grow(2.)
                    .min_w_0()
                    .gap(style::SPACE_2)
                    .child(
                        column()
                            .child(self.path_cell(
                                ix,
                                row,
                                field("cert-file", file, cx).into_any_element(),
                                window,
                                cx,
                            ))
                            .child(picker(
                                "cert-file-pick",
                                file_pick,
                                CertField::Certificate,
                                cx,
                            )),
                    )
                    .when(pem, |el| {
                        el.child(column().child(field("cert-key", key, cx)).child(picker(
                            "cert-key-pick",
                            "Choose key",
                            CertField::Key,
                            cx,
                        )))
                    }),
            )
            .child(h_flex().flex_1().min_w_0().child(
                if let Some(home) = row.stored_home(&self.dotenv) {
                    ui::stored_secret(
                        ("cert-passphrase-stored", ix),
                        ("cert-passphrase-replace", ix),
                        home,
                        cx.listener(move |this, _, window, cx| {
                            this.replace_passphrase(ix, window, cx)
                        }),
                        cx,
                    )
                    .into_any_element()
                } else {
                    field("cert-passphrase", passphrase, cx).into_any_element()
                },
            ))
            .child(
                ui::icon_button(
                    ("cert-remove", ix),
                    IconName::Close,
                    "Remove certificate",
                    None,
                    cx,
                )
                .tab_stop(false)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.remove_certificate(ix, window, cx)),
                ),
            )
    }
}

// The text size of a small kit input, which the fitted path matches.
const PATH_TEXT_REMS: f32 = 0.875;

// The 1px frame around each host input, which sits between the row gap and the input.
const HOST_FRAME_BORDER: Pixels = px(1.);

/// What a client certificate row does and what each of its fields takes.
pub const CERT_NOTE: &str = "Sends a certificate to HTTPS addresses that start with the domain, where * matches anything, as in *.example.com. Each row takes its type, the domain, the certificate and key files of a PEM pair or the one file of a PKCS#12 bundle, and the key's passphrase, if it has one.";

// A small muted line in a grid's left edge, labelled with its text for assistive tools.
fn muted_note(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .text_size(style::TEXT_SMALL)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

// A note under a host row, indented past the checkbox so it starts at the host input.
fn host_note(
    id: (&'static str, usize),
    text: impl Into<SharedString>,
    color: Hsla,
) -> impl IntoElement {
    let text = text.into();
    div().pl(style::CHECKBOX_SIZE + style::SPACE_2).child(
        div()
            .id(id)
            .test_support()
            .aria_label(text.clone())
            .ml(HOST_FRAME_BORDER)
            .text_size(style::TEXT_SMALL)
            .text_color(color)
            .child(text),
    )
}

// Whether a row names a file by an absolute path outside the collection at `root`,
// which will not resolve once the collection moves. A path that misses lexically is still
// inside when its canonical form lies under `canonical_root`.
fn names_outside(root: &Path, canonical_root: Option<&Path>, cert: &ClientCertificate) -> bool {
    let [_, certificate, key, _] = cert_texts(cert);
    [certificate, key].iter().any(|p| {
        let p = Path::new(p);
        let resolved_inside =
            || canonical_root.is_some_and(|r| p.canonicalize().is_ok_and(|p| p.starts_with(r)));
        p.is_absolute() && !p.starts_with(root) && !resolved_inside()
    })
}

fn is_pem(cert: &ClientCertificate) -> bool {
    matches!(cert.kind, CertificateKind::Pem { .. })
}

// A certificate's texts in `CertField` order; a PKCS#12 bundle fills the certificate slot.
fn cert_texts(cert: &ClientCertificate) -> [&str; 4] {
    let (file, key) = match &cert.kind {
        CertificateKind::Pem {
            certificate,
            private_key,
        } => (certificate.as_str(), private_key.as_str()),
        CertificateKind::Pkcs12 { file } => (file.as_str(), ""),
        _ => ("", ""),
    };
    [&cert.domain, file, key, &cert.passphrase]
}

// The key material a row's typed paths name as a PEM pair or a PKCS#12 bundle.
fn typed_kind(inputs: &CertInputs, pem: bool, cx: &App) -> CertificateKind {
    let text = |f: CertField| inputs.0[f as usize].read(cx).value().to_string();
    match pem {
        true => CertificateKind::Pem {
            certificate: text(CertField::Certificate),
            private_key: text(CertField::Key),
        },
        false => CertificateKind::Pkcs12 {
            file: text(CertField::Certificate),
        },
    }
}

fn set_file_placeholder(
    inputs: &CertInputs,
    cert: &ClientCertificate,
    window: &mut Window,
    cx: &mut App,
) {
    let hint = match is_pem(cert) {
        true => PLACEHOLDERS[CertField::Certificate as usize],
        false => BUNDLE_PLACEHOLDER,
    };
    let state = &inputs.0[CertField::Certificate as usize];
    state.update(cx, |s, cx| s.set_placeholder(hint, window, cx));
}

// Whether a host override row lacks its host or its address, so it cannot be sent yet.
fn incomplete(row: &HostOverride) -> bool {
    row.host.trim().is_empty() || row.address.trim().is_empty()
}

/// Whether a row has what the reference app requires before adding it: a domain and the
/// paths its type needs.
fn is_complete(cert: &ClientCertificate) -> bool {
    let filled = |s: &str| !s.trim().is_empty();
    filled(&cert.domain)
        && match &cert.kind {
            CertificateKind::Pem {
                certificate,
                private_key,
            } => filled(certificate) && filled(private_key),
            CertificateKind::Pkcs12 { file } => filled(file),
            _ => false,
        }
}

impl Render for NetworkSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let passphrases =
            (self.certs.iter()).map(|row| &row.inputs.0[CertField::Passphrase as usize]);
        for input in std::iter::once(&self.password).chain(passphrases) {
            ui::mask_unless_template(input, window, cx);
        }
        // Inputs hidden behind the badge keep the stored value, so nothing reads as typed.
        let mut hidden = Vec::new();
        if self.password_home().is_some() {
            hidden.push((self.password.clone(), self.password_committed.clone()));
        }
        for row in (self.certs.iter()).filter(|row| row.stored_home(&self.dotenv).is_some()) {
            let input = row.inputs.0[CertField::Passphrase as usize].clone();
            hidden.push((input, row.cert.passphrase.clone()));
        }
        for (input, value) in hidden {
            if input.read(cx).value() != value.as_str() {
                set_text(&input, &value, window, cx);
            }
        }
        let this = cx.entity();
        let selected = CHOICES
            .iter()
            .position(|(c, _)| *c == self.choice)
            .unwrap_or_default();
        let modes = ui::segmented(
            "proxy-mode",
            CHOICES.map(|(_, label)| label),
            selected,
            move |ix, window, cx| {
                if let Some(&(choice, _)) = CHOICES.get(*ix) {
                    this.update(cx, |s, cx| s.set_choice(choice, window, cx));
                }
            },
        );
        let explain = match self.choice {
            ProxyChoice::Inherit => Some("Requests use the proxy set in Preferences."),
            ProxyChoice::Off => Some("Requests connect directly."),
            ProxyChoice::Custom => None,
        };
        let plain = self.plain_secrets.then(|| {
            // Wraps so the button drops below the note when the pane is too narrow for both.
            h_flex()
                .id("network-plain-notice")
                .test_support()
                .flex_wrap()
                .gap(style::SPACE_2)
                .items_center()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().warning)
                .child("Stored in plain text")
                .child(
                    Button::new("network-move-secrets")
                        .solid_ring(cx)
                        .label("Move to .env")
                        .on_click(cx.listener(|this, _, _, cx| this.move_secrets(cx))),
                )
        });
        let notice = self.notice.clone().map(|text| {
            div()
                .id("network-notice")
                .test_support()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(text)
        });
        let changed = self.change.clone().map(|notice| {
            let title = if notice.changed {
                "Network settings changed outside Apinata"
            } else {
                "This collection sets"
            };
            let unreadable = notice.unreadable;
            v_flex()
                .id("network-change-notice")
                .test_support()
                .gap(style::SPACE_1)
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().warning)
                .child(title)
                .children(notice.lines.into_iter().map(|line| format!("- {line}")))
                .child(div().child(if unreadable {
                    Button::new("network-reset-acks")
                        .solid_ring(cx)
                        .label("Reset network acknowledgements")
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(NetworkAction(crate::command::NetworkOp::Reset));
                        }))
                } else {
                    Button::new("network-change-ack")
                        .solid_ring(cx)
                        .label("Acknowledge")
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(NetworkAction(crate::command::NetworkOp::Acknowledge));
                        }))
                }))
        });
        v_flex()
            .track_focus(&self.focus)
            .size_full()
            .gap(style::SPACE_2)
            .children(changed)
            .child(ui::form_heading("proxy-heading", "Proxy"))
            .child(ui::form_row("proxy-mode-row", "Mode", modes))
            .when_some(explain, |el, text| {
                el.child(ui::form_note("proxy-mode-note", text))
            })
            .when(self.choice == ProxyChoice::Custom, |el| {
                el.child(self.custom_rows(window, cx))
            })
            .child(self.certificate_rows(window, cx))
            .child(self.host_rows(window, cx))
            .children(plain)
            .children(notice)
    }
}
