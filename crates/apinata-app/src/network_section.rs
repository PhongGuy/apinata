//! The collection's Network tab: the proxy choice and its fields, the client certificates,
//! and a way to move literal secrets out of the collection file.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use apinata_core::{
    CertificateKind, ClientCertificate, CollectionNetwork, CollectionProxy, HostOverride,
    ProxyAuth, ProxyConfig,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::body_section::collection_relative;
use crate::command::Edit;
use crate::kv_table::set_text;
use crate::section::SectionEdit;
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

/// One text input of a PEM certificate row.
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

const OUTSIDE_NOTICE: &str = "Outside the collection: saved as an absolute path";
const PKCS12_HINT: &str =
    "PKCS#12 is not supported yet; convert with openssl pkcs12 -in x.pfx -nodes -out x.pem";

const PLACEHOLDERS: [&str; 4] = ["Domain", "Certificate", "Key", "Passphrase"];

// The inputs of a PEM row, indexed by `CertField` order.
struct CertInputs([Entity<InputState>; 4]);

// One certificate as loaded or edited. The value keeps its YAML origin; a PEM row writes
// its typed text back into it, and its passphrase is the last committed one.
struct CertRow {
    cert: ClientCertificate,
    inputs: Option<CertInputs>,
    // An added row stays local until its domain and paths are filled, as the reference app
    // validates a row before adding it; loaded rows are never drafts.
    draft: bool,
    // The outside answer for the certificate and key texts it was computed from, so a render
    // reads it and only an edited path pays for the filesystem check again.
    outside: RefCell<Option<([String; 2], bool)>>,
    _subs: Vec<Subscription>,
}

impl CertRow {
    fn outside(&self, root: &Path, canonical_root: Option<&Path>) -> bool {
        let [_, certificate, key, _] = pem_texts(&self.cert);
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
    _subs: Vec<Subscription>,
}

/// Edits the collection's proxy. The custom fields keep their text while another choice is
/// shown, so switching back restores them; the password is sent on Enter or blur only.
pub struct NetworkSection {
    root: PathBuf,
    // The root's resolved form, read once rather than per outside check.
    canonical_root: Option<PathBuf>,
    choice: ProxyChoice,
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
            protocol: PROTOCOLS[0].into(),
            host,
            port,
            username,
            password,
            bypass,
            password_committed: String::new(),
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
        self.choice = choice;
        // Inherit and Off keep the file's custom fields, so switching back sends them, never
        // blanks over them.
        if let Some(proxy) = fields {
            self.show_fields(proxy, window, cx);
        }
        self.load_certificates(&network.certificates, window, cx);
        self.load_hosts(&network.host_overrides, window, cx);
        cx.notify();
    }

    fn show_fields(&mut self, proxy: &ProxyConfig, window: &mut Window, cx: &mut Context<Self>) {
        self.loaded_proxy = proxy.clone();
        self.protocol.clone_from(&proxy.protocol);
        set_text(&self.host, &proxy.hostname, window, cx);
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

    // Rows keep their input entities while the shape matches, so a snapshot echoing the
    // user's own typing leaves focus and caret alone. The file never holds a draft, so drafts
    // are left out of the match and kept after the loaded rows.
    fn load_certificates(
        &mut self,
        certs: &[ClientCertificate],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let committed = || self.certs.iter().filter(|row| !row.draft);
        let same_shape = committed().count() == certs.len()
            && committed()
                .zip(certs)
                .all(|(row, cert)| row.inputs.is_some() == is_pem(cert));
        if !same_shape {
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
        // the same certificate (domain and paths) with the passphrase last committed.
        let rows = self.certs.iter_mut().filter(|row| !row.draft);
        for (row, cert) in rows.zip(certs) {
            let typed_over = row.cert.passphrase == cert.passphrase
                && row.cert.domain == cert.domain
                && row.cert.kind == cert.kind;
            row.cert = cert.clone();
            if let Some(inputs) = &row.inputs {
                for (i, (state, text)) in inputs.0.iter().zip(pem_texts(cert)).enumerate() {
                    if !(typed_over && i == CertField::Passphrase as usize) {
                        set_text(state, text, window, cx);
                    }
                }
            }
        }
    }

    // Rows keep their input entities while the count matches, so a snapshot echoing the user's
    // own typing leaves focus and caret alone.
    fn load_hosts(&mut self, rows: &[HostOverride], window: &mut Window, cx: &mut Context<Self>) {
        if self.hosts.len() != rows.len() {
            self.hosts = rows
                .iter()
                .map(|row| {
                    let mut slot = self.new_host_row(row.clone(), window, cx);
                    slot.checked = true;
                    slot
                })
                .collect();
            return;
        }
        for (slot, row) in self.hosts.iter_mut().zip(rows) {
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
        self.hosts.iter().map(|slot| slot.row.clone()).collect()
    }

    /// Appends an empty host override row and sends the list.
    pub fn add_host_override(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let slot = self.new_host_row(HostOverride::new("", ""), window, cx);
        slot.host.focus_handle(cx).focus(window, cx);
        self.hosts.push(slot);
        self.emit_hosts(cx);
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
            self.hosts.remove(row);
            let next = self
                .hosts
                .get(row)
                .or_else(|| self.hosts.get(row.wrapping_sub(1)));
            match next {
                Some(slot) => slot.host.focus_handle(cx).focus(window, cx),
                None => self.focus.focus(window, cx),
            }
            self.emit_hosts(cx);
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
            self.emit_hosts(cx);
        }
    }

    fn set_host_enabled(&mut self, row: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(slot) = self.hosts.get_mut(row) {
            slot.row.enabled = on;
            self.emit_hosts(cx);
        }
    }

    fn emit_hosts(&mut self, cx: &mut Context<Self>) {
        cx.emit(
            SectionEdit::new(Edit::HostOverrides(self.host_overrides())).after_loads(self.loads),
        );
        cx.notify();
    }

    fn new_row(
        &self,
        cert: ClientCertificate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CertRow {
        if !is_pem(&cert) {
            return CertRow {
                cert,
                inputs: None,
                draft: false,
                outside: RefCell::default(),
                _subs: Vec::new(),
            };
        }
        let texts = pem_texts(&cert);
        let inputs = std::array::from_fn(|i| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(i == CertField::Passphrase as usize)
                    .placeholder(PLACEHOLDERS[i])
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
            inputs: Some(CertInputs(inputs)),
            draft: false,
            outside: RefCell::default(),
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
        self.certs.push(row);
        cx.notify();
    }

    /// Removes row `row`, moves focus to the next editable row's domain, else the previous
    /// one's, else the section itself (so the editor's keys keep working), and sends the list.
    pub fn remove_certificate(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if row < self.certs.len() {
            let draft = self.certs.remove(row).draft;
            let domain = |r: &CertRow| {
                r.inputs
                    .as_ref()
                    .map(|i| i.0[CertField::Domain as usize].clone())
            };
            let next = self.certs[row..].iter().find_map(domain);
            let prev = self.certs[..row].iter().rev().find_map(domain);
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

    /// The input behind `field` of row `row`; `None` for a PKCS#12 row or no such row.
    pub fn cert_input(&self, row: usize, field: CertField) -> Option<&Entity<InputState>> {
        let inputs = self.certs.get(row)?.inputs.as_ref()?;
        Some(&inputs.0[field as usize])
    }

    // Copies the row's typed text into its value and sends it when it changed; the
    // passphrase input reports here on Enter or blur only.
    fn cert_text_changed(&mut self, state: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(row) = self.certs.iter_mut().find(|row| {
            row.inputs
                .as_ref()
                .is_some_and(|inputs| inputs.0.contains(state))
        }) else {
            return;
        };
        let before = row.cert.clone();
        if let Some(inputs) = &row.inputs {
            let [domain, certificate, key, passphrase] =
                inputs.0.each_ref().map(|s| s.read(cx).value().to_string());
            row.cert.domain = domain;
            row.cert.kind = CertificateKind::Pem {
                certificate,
                private_key: key,
            };
            // Other fields carry the last committed passphrase, never half-typed text.
            if *state == inputs.0[CertField::Passphrase as usize] {
                row.cert.passphrase = passphrase;
            }
        }
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
    // row's certificate or key. A cancelled or failed prompt does nothing.
    fn pick(&mut self, row: usize, field: CertField, window: &mut Window, cx: &mut Context<Self>) {
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
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
            .filter_map(|row| row.inputs.as_ref())
            .map(|inputs| inputs.0[CertField::Passphrase as usize].clone())
            .collect();
        for state in states {
            self.cert_text_changed(&state, cx);
        }
    }

    fn emit_certificates(&mut self, cx: &mut Context<Self>) {
        cx.emit(
            SectionEdit::new(Edit::ClientCertificates(self.certificates())).after_loads(self.loads),
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
    pub fn unsaved(&self) -> Vec<String> {
        self.port_error
            .iter()
            .map(|reason| format!("Not saved: Port: {reason}"))
            .collect()
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
    fn commit_password(&mut self, cx: &mut Context<Self>) {
        let text = self.password.read(cx).value().to_string();
        if text != self.password_committed {
            self.password_committed = text;
            self.emit(cx);
        }
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

    fn emit(&mut self, cx: &mut Context<Self>) {
        let proxy = self.current(cx);
        cx.emit(SectionEdit::new(Edit::Proxy(proxy)).after_loads(self.loads));
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
        let field = |id: &'static str, state: &Entity<InputState>, width: Rems| {
            ui::padded_input(
                Input::new(state)
                    .id(id)
                    .w(width)
                    .bg(ui::field_fill(false, cx)),
                state,
            )
        };
        v_flex()
            .gap(style::SPACE_2)
            .child(ui::form_row("proxy-protocol-row", "Protocol", protocols))
            .child(ui::form_row(
                "proxy-host-row",
                "Host",
                field("proxy-host", &self.host, style::WIDE_INPUT_WIDTH),
            ))
            .child(ui::form_row(
                "proxy-port-row",
                "Port",
                ui::invalid_input(
                    Input::new(&self.port)
                        .id("proxy-port")
                        .w(style::SHORT_INPUT_WIDTH)
                        .bg(ui::field_fill(false, cx)),
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
                field("proxy-password", &self.password, style::WIDE_INPUT_WIDTH),
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
                let reason = slot.row.check().err().map(|e| e.to_string());
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
                        )
                        .tab_stop(false)
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.remove_host_override(ix, window, cx),
                        )),
                    );
                v_flex()
                    .gap(style::SPACE_1)
                    .child(row)
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
            .child(ui::form_note(
                "hosts-note",
                "Send a host name to an address instead of its DNS answer. A port in the address counts only for a URL that names none.",
            ))
            .children(rows)
            .child(
                h_flex().child(
                    Button::new("host-add")
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
            .map(|(ix, row)| match &row.inputs {
                Some(inputs) => v_flex()
                    .gap(style::SPACE_1)
                    .child(self.pem_row(ix, row.cert.enabled, inputs, window, cx))
                    .when(
                        row.outside(&self.root, self.canonical_root.as_deref()),
                        |el| el.child(muted_note(("cert-outside", ix), OUTSIDE_NOTICE, cx)),
                    )
                    .into_any_element(),
                None => pkcs12_row(ix, &row.cert, cx).into_any_element(),
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(style::SPACE_2)
            .child(ui::form_heading("cert-heading", "Client certificates"))
            .children(rows)
            .child(
                h_flex().child(
                    Button::new("cert-add").label("Add certificate").on_click(
                        cx.listener(|this, _, window, cx| this.add_certificate(window, cx)),
                    ),
                ),
            )
    }

    fn pem_row(
        &self,
        ix: usize,
        enabled: bool,
        inputs: &CertInputs,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let [domain, file, key, passphrase] = &inputs.0;
        let field = |id: &'static str, state: &Entity<InputState>, cx: &App| {
            ui::padded_input(
                ui::table_input(state, false, window, cx)
                    .id((id, ix))
                    .flex_1(),
                state,
            )
        };
        let picker = |id: &'static str, tip: &str, which: CertField, cx: &mut Context<Self>| {
            ui::icon_button((id, ix), IconName::FolderOpen, tip, None)
                .tab_stop(false)
                .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, which, window, cx)))
        };
        h_flex()
            .id(("cert-row", ix))
            .test_support()
            .h(style::ROW_HEIGHT)
            .gap(style::SPACE_2)
            .items_center()
            .w_full()
            .child(
                ui::checkbox(("cert-enabled", ix), enabled)
                    .tab_stop(false)
                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                        this.set_cert_enabled(ix, *on, cx)
                    })),
            )
            .child(field("cert-domain", domain, cx))
            .child(field("cert-file", file, cx))
            .child(picker(
                "cert-file-pick",
                "Choose certificate",
                CertField::Certificate,
                cx,
            ))
            .child(field("cert-key", key, cx))
            .child(picker("cert-key-pick", "Choose key", CertField::Key, cx))
            .child(field("cert-passphrase", passphrase, cx))
            .child(
                ui::icon_button(
                    ("cert-remove", ix),
                    IconName::Close,
                    "Remove certificate",
                    None,
                )
                .tab_stop(false)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.remove_certificate(ix, window, cx)),
                ),
            )
    }
}

// A PKCS#12 row is shown as text only and sent back unchanged.
fn pkcs12_row(ix: usize, cert: &ClientCertificate, cx: &App) -> impl IntoElement + use<> {
    let file = match &cert.kind {
        CertificateKind::Pkcs12 { file } => file.as_str(),
        _ => "",
    };
    v_flex()
        .gap(style::SPACE_1)
        .child(
            div()
                .id(("cert-pkcs12", ix))
                .test_support()
                .child(format!("{}  {}", cert.domain, file)),
        )
        .child(muted_note(("cert-pkcs12-hint", ix), PKCS12_HINT, cx))
}

// The 1px frame around each host input, which sits between the row gap and the input.
const HOST_FRAME_BORDER: Pixels = px(1.);

// A small muted line under a certificate row, labelled with its text for assistive tools.
fn muted_note(
    id: (&'static str, usize),
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

// Whether a PEM row names a file by an absolute path outside the collection at `root`,
// which will not resolve once the collection moves. A path that misses lexically is still
// inside when its canonical form lies under `canonical_root`.
fn names_outside(root: &Path, canonical_root: Option<&Path>, cert: &ClientCertificate) -> bool {
    let [_, certificate, key, _] = pem_texts(cert);
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

// A PEM certificate's texts in `CertField` order; empty for any other kind.
fn pem_texts(cert: &ClientCertificate) -> [&str; 4] {
    match &cert.kind {
        CertificateKind::Pem {
            certificate,
            private_key,
        } => [&cert.domain, certificate, private_key, &cert.passphrase],
        _ => ["", "", "", ""],
    }
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
        let passphrases = self.certs.iter().filter_map(|row| {
            let inputs = row.inputs.as_ref()?;
            Some(&inputs.0[CertField::Passphrase as usize])
        });
        for input in std::iter::once(&self.password).chain(passphrases) {
            ui::mask_unless_template(input, window, cx);
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
                        .label("Reset network acknowledgements")
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(NetworkAction(crate::command::NetworkOp::Reset));
                        }))
                } else {
                    Button::new("network-change-ack")
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
