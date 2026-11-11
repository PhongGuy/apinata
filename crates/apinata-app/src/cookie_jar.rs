//! The cookie jar dialog: lists the shared executor's cookies grouped by domain, adds and
//! edits one through an inline form, and deletes one cookie, one domain or all of them. Each
//! change is saved in the background; nothing is editable until the saved jar has loaded.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use apinata_core::{CookieError, CookieInfo, CookieJar, Persistence};
use chrono::{DateTime, Local, NaiveDateTime, SecondsFormat, TimeZone as _};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, Focusable, Pixels, Point, Result, ScrollHandle, SharedString, Subscription,
    Task, TestSupportExt, Window, div, px,
};

use crate::executor::{save_cookies, shared};
use crate::import_report::failure_text;
use crate::kv_grid::{ActionShow, action_slot};
use crate::response_pane::NAME_MASK;
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_2, TEXT_SMALL};
use crate::ui::{self, LockButton as _};

/// How often the dialog relists while the saved jar is still loading.
const LOAD_POLL: Duration = Duration::from_millis(250);

/// `Ok(None)` for blank text, else the instant `text` names as local `YYYY-MM-DD HH:MM` or
/// RFC 3339; `Err` with a message for the user when unparseable or not after `now`.
pub fn parse_expires(text: &str, now: SystemTime) -> Result<Option<SystemTime>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let at = local_minutes(text)
        .or_else(|| {
            DateTime::parse_from_rfc3339(text)
                .ok()
                .map(SystemTime::from)
        })
        .ok_or_else(|| format!("Enter a date and time like {EXPIRES_EXAMPLE}"))?;
    if at <= now {
        return Err("Expires must be in the future".into());
    }
    Ok(Some(at))
}

// The Secure and HttpOnly flags in plain words, on the saved rows and the form's checkboxes.
const SECURE_LABEL: &str = "HTTPS only";
const HTTP_ONLY_LABEL: &str = "No script access";

// The local date and time the Expires field shows as its example.
const EXPIRES_EXAMPLE: &str = "2027-01-01 12:00";

// The instant a local `YYYY-MM-DD HH:MM` names. A time in a DST gap has none; an overlap
// takes the earlier one.
fn local_minutes(text: &str) -> Option<SystemTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|t| Local.from_local_datetime(&t).earliest())
        .map(SystemTime::from)
}

// A saved expiry as the form shows it: the short local form a person types when that reads
// back as the same instant, else local RFC 3339 with seconds, so an untouched field saves it.
fn expires_text(at: SystemTime) -> String {
    let local = DateTime::<Local>::from(at);
    let short = local.format("%Y-%m-%d %H:%M").to_string();
    if local_minutes(&short) == Some(at) {
        short
    } else {
        local.to_rfc3339_opts(SecondsFormat::Secs, false)
    }
}

/// One text field of the add/edit form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieField {
    /// The host, a leading dot making the cookie domain-wide.
    Domain,
    /// The path, starting with "/".
    Path,
    /// The cookie name.
    Name,
    /// The value, kept untrimmed.
    Value,
    /// The expiry text `parse_expires` reads.
    Expires,
}

/// The open add/edit form: its inputs, flags, and the cookie it edits, if any.
struct CookieForm {
    editing: Option<CookieInfo>,
    domain: Entity<InputState>,
    path: Entity<InputState>,
    name: Entity<InputState>,
    value: Entity<InputState>,
    expires: Entity<InputState>,
    secure: bool,
    http_only: bool,
    error: Option<String>,
    // Each refused field's input id and its message, drawn under that field; every refused
    // field is marked, not only the first.
    field_errors: Vec<(&'static str, String)>,
    // Refusals no field owns, drawn above the buttons.
    loose_errors: Vec<String>,
    // What the fields and flags held when the form opened, to tell an edit from a glance.
    opened: (Vec<String>, bool, bool),
    // Enter in any field saves, as the Save button does.
    _enter: Vec<Subscription>,
}

impl CookieForm {
    fn state(&self, cx: &App) -> (Vec<String>, bool, bool) {
        let fields = [
            &self.domain,
            &self.path,
            &self.name,
            &self.value,
            &self.expires,
        ];
        let text = fields.map(|f| f.read(cx).value().to_string());
        (text.into(), self.secure, self.http_only)
    }
}

// The form input a core refusal names, when it names one of them.
fn field_id(field: &str) -> Option<&'static str> {
    Some(match field {
        "domain" => "jar-form-domain",
        "path" => "jar-form-path",
        "name" => "jar-form-name",
        "value" => "jar-form-value",
        "expires" => "jar-form-expires",
        _ => return None,
    })
}

/// True when `query` is empty or a case-insensitive substring of `domain`; no trimming.
pub fn domain_matches(domain: &str, query: &str) -> bool {
    domain.to_lowercase().contains(&query.to_lowercase())
}

/// When a cookie expires, relative to `now`: "Session", "Expired", or "in N min|h|d".
pub fn expiry_label(expires: Option<SystemTime>, now: SystemTime) -> String {
    let Some(at) = expires else {
        return "Session".into();
    };
    let secs = match at.duration_since(now) {
        Ok(left) if !left.is_zero() => left.as_secs(),
        _ => return "Expired".into(),
    };
    // Each unit is floored, but anything still live shows at least one minute.
    match secs {
        s if s < 3600 => format!("in {} min", (s / 60).max(1)),
        s if s < 86_400 => format!("in {} h", s / 3600),
        s => format!("in {} d", s / 86_400),
    }
}

/// The dialog body: a snapshot of the shared jar, relisted after every action.
pub struct CookieJarView {
    cookies: Vec<CookieInfo>,
    available: bool,
    persistence: Option<Persistence>,
    form: Option<CookieForm>,
    load_poll: Option<Task<()>>,
    search: Entity<InputState>,
    query: String,
    /// Cookies whose value is shown, keyed (domain, path, name) so it survives relisting.
    revealed: BTreeSet<(String, String, String)>,
    /// Scrolls the dialog body, the list or the form standing in for it.
    scroll: ScrollHandle,
    // Where the list was scrolled when the form opened; the short form caps the shared
    // offset, so closing the form puts it back.
    list_offset: Point<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl CookieJarView {
    /// Lists the shared jar, with an empty domain search.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search by domain"));
        let subscriptions =
            vec![
                cx.subscribe_in(&search, window, |this, input, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        this.query = input.read(cx).value().to_string();
                        cx.notify();
                    }
                }),
            ];
        let mut view = CookieJarView {
            cookies: Vec::new(),
            available: false,
            persistence: None,
            form: None,
            load_poll: None,
            search,
            query: String::new(),
            revealed: BTreeSet::new(),
            scroll: ScrollHandle::new(),
            list_offset: Point::default(),
            _subscriptions: subscriptions,
        };
        view.refresh(cx);
        view
    }

    /// True when an executor exists and its jar is not `NotLoaded`.
    pub fn editable(&self) -> bool {
        self.available && !matches!(self.persistence, None | Some(Persistence::NotLoaded))
    }

    /// "Loading saved cookies", "Cookies are saved encrypted." or
    /// "Cookies are kept for this session only: <reason>"; empty without an executor.
    pub fn footer(&self) -> String {
        match &self.persistence {
            None => String::new(),
            Some(Persistence::NotLoaded) => "Loading saved cookies".into(),
            Some(Persistence::Encrypted) => "Cookies are saved encrypted.".into(),
            Some(Persistence::MemoryOnly { reason }) => {
                format!("Cookies are kept for this session only: {reason}")
            }
            Some(_) => String::new(),
        }
    }

    /// Opens the form empty (path "/"), or on the listed cookie at `ix` with domain, path and
    /// name locked. A no-op while not `editable`.
    pub fn open_form(&mut self, edit: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let editing = match edit {
            Some(ix) => match self.cookies.get(ix) {
                Some(c) => Some(c.clone()),
                None => return,
            },
            None => None,
        };
        let text = |c: Option<&CookieInfo>, f: fn(&CookieInfo) -> String, default: &str| {
            c.map(f).unwrap_or_else(|| default.to_owned())
        };
        let c = editing.as_ref();
        let mut input =
            |value: String| cx.new(|cx| InputState::new(window, cx).default_value(value));
        let expires = c.and_then(|c| c.expires).map(expires_text);
        let domain = input(text(c, |c| c.domain.clone(), ""));
        let path = input(text(c, |c| c.path.clone(), "/"));
        let name = input(text(c, |c| c.name.clone(), ""));
        let value = input(text(c, |c| c.value.clone(), ""));
        let expires = input(expires.unwrap_or_default());
        // The form takes focus at its first editable field; an edit locks Domain, Path and Name.
        let first = if c.is_some() { &value } else { &domain };
        first.read(cx).focus_handle(cx).focus(window, cx);
        let enter = [&domain, &path, &name, &value, &expires]
            .into_iter()
            .map(|field| {
                cx.subscribe_in(field, window, |this, _, e: &InputEvent, window, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        this.submit_form(window, cx);
                    }
                })
            })
            .collect();
        let mut form = CookieForm {
            opened: Default::default(),
            domain,
            path,
            name,
            value,
            expires,
            secure: c.is_some_and(|c| c.secure),
            http_only: c.is_some_and(|c| c.http_only),
            error: None,
            field_errors: Vec::new(),
            loose_errors: Vec::new(),
            _enter: enter,
            editing,
        };
        form.opened = form.state(cx);
        if self.form.is_none() {
            self.list_offset = self.scroll.offset();
        }
        self.form = Some(form);
        cx.notify();
    }

    /// Whether the open form holds anything typed or toggled since it opened.
    pub fn form_edited(&self, cx: &App) -> bool {
        self.form.as_ref().is_some_and(|f| f.state(cx) != f.opened)
    }

    /// The form's input for `field`, `None` while the form is closed.
    pub fn form_input(&self, field: CookieField) -> Option<Entity<InputState>> {
        let form = self.form.as_ref()?;
        Some(
            match field {
                CookieField::Domain => &form.domain,
                CookieField::Path => &form.path,
                CookieField::Name => &form.name,
                CookieField::Value => &form.value,
                CookieField::Expires => &form.expires,
            }
            .clone(),
        )
    }

    /// The form's last refusal, `None` while it is closed or has none.
    pub fn form_error(&self) -> Option<&str> {
        self.form.as_ref()?.error.as_deref()
    }

    /// The dialog's title: "Add cookie" or "Edit cookie" while the form is open, else "Cookies".
    pub fn title(&self) -> &'static str {
        match &self.form {
            None => "Cookies",
            Some(f) if f.editing.is_some() => "Edit cookie",
            Some(_) => "Add cookie",
        }
    }

    /// Validates and inserts; on success closes the form, relists, saves in the background
    /// and moves focus to the search field, so Escape still reaches the dialog.
    pub fn submit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.editable() {
            return false;
        }
        let (Some(form), Some(exec)) = (self.form.as_mut(), shared(cx)) else {
            return false;
        };
        let read = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let value = read(&form.value);
        // Every problem is collected so each bad field is marked at once: the form's own
        // expiry check and then the jar's field checks. An empty value is a valid cookie.
        let mut problems: Vec<(Option<&'static str>, String)> = Vec::new();
        let expires = match parse_expires(&read(&form.expires), SystemTime::now()) {
            Ok(expires) => expires,
            Err(e) => {
                problems.push((Some("jar-form-expires"), e));
                None
            }
        };
        // An edit keeps the listed identity and host_only; a new cookie takes them from the
        // typed fields, a leading dot making it domain-wide.
        let mut c = form.editing.clone().unwrap_or_else(|| {
            let mut c = CookieInfo::new(read(&form.domain).trim(), read(&form.name).trim(), "");
            c.path = read(&form.path).trim().to_owned();
            c
        });
        c.value = value;
        c.expires = expires;
        c.secure = form.secure;
        c.http_only = form.http_only;
        for e in CookieJar::check(&c) {
            let field = match &e {
                CookieError::Invalid { field, .. } => field_id(field),
                _ => None,
            };
            // The expiry check above already spoke for its field.
            if !problems.iter().any(|(f, _)| f.is_some() && *f == field) {
                problems.push((field, e.to_string()));
            }
        }
        if problems.is_empty()
            && let Err(e) = exec.cookies().insert(c)
        {
            let field = match &e {
                CookieError::Invalid { field, .. } => field_id(field),
                _ => None,
            };
            problems.push((field, e.to_string()));
        }
        if !problems.is_empty() {
            // Core's messages start lowercase; each line is an alert of its own.
            let problems: Vec<_> = problems
                .into_iter()
                .map(|(f, m)| (f, failure_text(&m)))
                .collect();
            let messages: Vec<_> = problems.iter().map(|(_, m)| m.as_str()).collect();
            form.error = Some(messages.join("\n"));
            form.field_errors = problems
                .iter()
                .filter_map(|(f, m)| f.map(|f| (f, m.clone())))
                .collect();
            form.loose_errors = problems
                .into_iter()
                .filter_map(|(f, m)| f.is_none().then_some(m))
                .collect();
            cx.notify();
            return false;
        }
        self.close_form();
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        self.changed(cx);
        true
    }

    /// Closes the form without inserting anything.
    pub fn cancel_form(&mut self, cx: &mut Context<Self>) {
        self.close_form();
        cx.notify();
    }

    // Drops the form and returns the list to where it was scrolled when the form opened.
    fn close_form(&mut self) {
        if self.form.take().is_some() {
            self.scroll.set_offset(self.list_offset);
        }
    }

    /// The listed cookies, in the jar's domain, path and name order.
    pub fn cookies(&self) -> &[CookieInfo] {
        &self.cookies
    }

    /// Indices into `cookies()` whose domain matches the search, in list order.
    pub fn shown(&self) -> Vec<usize> {
        (0..self.cookies.len())
            .filter(|&ix| domain_matches(&self.cookies[ix].domain, &self.query))
            .collect()
    }

    /// Deletes the listed cookie at `ix` from the jar. A no-op while not `editable`.
    pub fn delete(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        if let (Some(c), Some(exec)) = (self.cookies.get(ix), shared(cx)) {
            exec.cookies().delete(&c.domain, &c.path, &c.name);
        }
        self.changed(cx);
    }

    /// Deletes every cookie stored under `domain`. A no-op while not `editable`.
    pub fn delete_domain(&mut self, domain: &str, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        if let Some(exec) = shared(cx) {
            exec.cookies().delete_domain(domain);
        }
        self.changed(cx);
    }

    /// Deletes every cookie. A no-op while not `editable`.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        if let Some(exec) = shared(cx) {
            exec.cookies().clear();
        }
        self.changed(cx);
    }

    /// Deletes every cookie whose domain the search shows; with no search, clears the
    /// jar. A no-op while not `editable`.
    pub fn clear_shown(&mut self, cx: &mut Context<Self>) {
        if self.query.is_empty() {
            return self.clear(cx);
        }
        if !self.editable() {
            return;
        }
        let mut domains: Vec<String> = self
            .shown()
            .into_iter()
            .map(|ix| self.cookies[ix].domain.clone())
            .collect();
        domains.dedup();
        if let Some(exec) = shared(cx) {
            for domain in &domains {
                exec.cookies().delete_domain(domain);
            }
        }
        self.changed(cx);
    }

    /// Relists the jar and its persistence; while it loads, keeps one poll relisting it.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let exec = shared(cx);
        self.available = exec.is_some();
        self.persistence = exec.as_ref().map(|e| e.cookies().persistence());
        self.cookies = exec.map(|e| e.cookies().list()).unwrap_or_default();
        if self.persistence != Some(Persistence::NotLoaded) {
            self.load_poll = None;
        } else if self.load_poll.is_none() {
            self.load_poll = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(LOAD_POLL).await;
                    // Once loading ends, refresh drops this task's handle and the loop exits.
                    let loading = this.update(cx, |v, cx| {
                        v.refresh(cx);
                        v.persistence == Some(Persistence::NotLoaded)
                    });
                    if !matches!(loading, Ok(true)) {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    // After an edit: relist, then write the jar off the UI thread.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        if let Some(exec) = shared(cx) {
            cx.background_spawn(async move { save_cookies(exec.cookies()) })
                .detach();
        }
    }

    fn cookie_row(
        &self,
        ix: usize,
        c: &CookieInfo,
        now: SystemTime,
        cx: &mut Context<Self>,
    ) -> Div {
        let flags: Vec<&str> = [
            (c.secure, SECURE_LABEL),
            (c.http_only, HTTP_ONLY_LABEL),
            (c.host_only, "This host only"),
        ]
        .into_iter()
        .filter_map(|(on, label)| on.then_some(label))
        .collect();
        // The value stays masked until its eye is clicked; the edit form shows the real one.
        let key = (c.domain.clone(), c.path.clone(), c.name.clone());
        let revealed = self.revealed.contains(&key);
        let shown: SharedString = if c.value.is_empty() {
            SharedString::default()
        } else if revealed {
            c.value.clone().into()
        } else {
            NAME_MASK.into()
        };
        let eye = (!c.value.is_empty()).then(|| {
            let (id, icon, words) = if revealed {
                ("jar-hide", IconName::EyeOff, "Hide value")
            } else {
                ("jar-reveal", IconName::Eye, "Show value")
            };
            // Each jar button takes Enter as its own press, not the dialog's Confirm.
            let button = ui::swap_icon_button(("jar-eye", ix), (id, ix), icon, words)
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.revealed.remove(&key) {
                        this.revealed.insert(key.clone());
                    }
                    cx.notify();
                }));
            action_slot(button, ActionShow::Always)
        });
        h_flex()
            .gap_2()
            .child(
                h_flex()
                    .id(("jar-cookie", ix))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(c.name.clone())
                    .child(
                        h_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .id(("jar-value", ix))
                                    .test_support()
                                    .aria_label(shown.clone())
                                    .min_w_0()
                                    .truncate()
                                    .child(shown),
                            )
                            .children(eye),
                    )
                    .child(c.path.clone())
                    .child(expiry_label(c.expires, now))
                    .child({
                        let flags: SharedString = flags.join(", ").into();
                        div()
                            .id(("jar-flags", ix))
                            .test_support()
                            .aria_label(flags.clone())
                            .child(flags)
                    }),
            )
            .child(
                Button::new(("jar-edit", ix))
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .label("Edit")
                    .lock(!self.editable(), cx)
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.open_form(Some(ix), window, cx)
                        }),
                    ),
            )
            .child(
                Button::new(("jar-delete", ix))
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .label("Delete")
                    .lock(!self.editable(), cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.delete(ix, cx))),
            )
    }

    fn form_view(
        &self,
        form: &CookieForm,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // An edit keeps the cookie's identity, so its domain, path and name are locked. The
        // element sets the state's read-only flag on each render.
        let locked = form.editing.is_some();
        let danger = cx.theme().danger;
        // A refused field's message sits right under its input, in the control column.
        let field = |id: &'static str, label: &'static str, input: &Entity<InputState>, lock| {
            let error = form
                .field_errors
                .iter()
                .find(|(f, _)| *f == id)
                .map(|(_, m)| SharedString::from(m.clone()));
            let control = ui::invalid_input(
                ui::lock_input(Input::new(input), input, lock, window, cx)
                    .id(id)
                    .w_full()
                    .bg(ui::field_fill(lock, cx)),
                div().w_full(),
                input,
                error.is_some(),
                (
                    SharedString::from(format!("{id}-danger-ring")).into(),
                    SharedString::from(format!("{id}-invalid-icon")).into(),
                ),
                window,
                cx,
            );
            ui::form_row(
                format!("{id}-row"),
                label,
                v_flex()
                    .w_full()
                    .child(control)
                    .when_some(error, |c, error| {
                        c.child(
                            div()
                                .id(SharedString::from(format!("{id}-error")))
                                .test_support()
                                .aria_label(error.clone())
                                .text_size(TEXT_SMALL)
                                .text_color(danger)
                                .child(error),
                        )
                    }),
            )
            .label_width(SHORT_FORM_LABEL_WIDTH)
        };
        let hint: SharedString = format!(
            "Leave Expires blank for a session cookie, or enter a local date and time like \
             {EXPIRES_EXAMPLE} (UTC{}).",
            Local::now().format("%:z")
        )
        .into();
        v_flex()
            .id("jar-form")
            .test_support()
            .gap(SPACE_2)
            .child(field("jar-form-domain", "Domain", &form.domain, locked))
            .child(field("jar-form-path", "Path", &form.path, locked))
            .child(field("jar-form-name", "Name", &form.name, locked))
            .child(field("jar-form-value", "Value", &form.value, false))
            .child(field("jar-form-expires", "Expires", &form.expires, false))
            .child(
                div()
                    .id("jar-form-expires-hint")
                    .test_support()
                    .aria_label(hint.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(hint),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        ui::checkbox("jar-form-secure", form.secure)
                            .label(SECURE_LABEL)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                if let Some(f) = this.form.as_mut() {
                                    f.secure = *on;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        ui::checkbox("jar-form-http-only", form.http_only)
                            .label(HTTP_ONLY_LABEL)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                if let Some(f) = this.form.as_mut() {
                                    f.http_only = *on;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .when(!form.loose_errors.is_empty(), |el| {
                el.child(
                    v_flex()
                        .id("jar-form-error")
                        .test_support()
                        .text_color(danger)
                        .children(
                            form.loose_errors
                                .iter()
                                .map(|line| div().child(line.clone())),
                        ),
                )
            })
            .child(
                h_flex()
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("jar-form-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_form(cx))),
                    )
                    .child(
                        Button::new("jar-form-save")
                            .label("Save")
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_form(window, cx);
                            })),
                    ),
            )
    }
}

impl Render for CookieJarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex().id("cookie-jar").gap_2().test_support();
        if !self.available {
            return root.child(
                div()
                    .id("jar-unavailable")
                    .test_support()
                    .child("No request executor is available."),
            );
        }
        // While a cookie is added or edited, the dialog shows only the form, so Save and
        // Cancel end the dialog instead of sitting above the list.
        if let Some(form) = &self.form {
            return root.child(self.form_view(form, window, cx));
        }
        let mut root = root.child(
            h_flex().child(
                Button::new("jar-add")
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .label("Add cookie")
                    .lock(!self.editable(), cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_form(None, window, cx))),
            ),
        );
        root = root.child(ui::padded_input(
            Input::new(&self.search)
                .id("jar-search")
                .w_full()
                .bg(ui::field_fill(false, cx)),
            &self.search,
        ));
        let shown = self.shown();
        if self.cookies.is_empty() {
            root = root.child(div().id("jar-empty").test_support().child("No cookies."));
        } else {
            if shown.is_empty() {
                root = root.child(
                    div()
                        .id("jar-no-match")
                        .test_support()
                        .child("No cookies match."),
                );
            }
            let now = SystemTime::now();
            // Rows arrive sorted by domain, so each domain heading starts where the domain
            // changes from the previous shown row; rows keep their index into the full list.
            let mut domains = 0usize;
            let mut previous: Option<&str> = None;
            let any_shown = !shown.is_empty();
            for ix in shown {
                let c = &self.cookies[ix];
                if previous != Some(c.domain.as_str()) {
                    let domain = c.domain.clone();
                    root = root.child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .id(("jar-domain", domains))
                                    .test_support()
                                    .flex_1()
                                    .child(domain.clone()),
                            )
                            .child(
                                Button::new(("jar-domain-delete", domains))
                                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                                    .label("Delete domain")
                                    .lock(!self.editable(), cx)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete_domain(&domain, cx)
                                    })),
                            ),
                    );
                    domains += 1;
                }
                previous = Some(c.domain.as_str());
                root = root.child(self.cookie_row(ix, c, now, cx));
            }
            // Under a search the button only reaches what the list shows, so hidden
            // domains survive; with nothing shown there is nothing to offer.
            if any_shown {
                let label = if self.query.is_empty() {
                    "Clear all"
                } else {
                    "Clear shown"
                };
                root = root.child(
                    Button::new("jar-clear")
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .label(label)
                        .lock(!self.editable(), cx)
                        .on_click(cx.listener(|this, _, _, cx| this.clear_shown(cx))),
                );
            }
        }
        root.child(
            div()
                .id("jar-footer")
                .test_support()
                .text_color(cx.theme().muted_foreground)
                .child(self.footer()),
        )
    }
}

/// Opens the "Cookies" dialog over `window`, listing the shared jar, and returns its body.
pub fn open_cookie_jar(window: &mut Window, cx: &mut App) -> Entity<CookieJarView> {
    let view = cx.new(|cx| CookieJarView::new(window, cx));
    let guard = crate::dialog_guard::guarded(form_guard(&view), window, cx);
    let body = view.clone();
    let scroll = view.read(cx).scroll.clone();
    let opened = guard.clone();
    window.open_dialog(cx, move |d, window, cx| {
        // Enter never closes the jar: a form field saves its cookie, and nothing else is lost.
        // The body scrolls inside the room above the status bar, so a full jar never lifts
        // the dialog over the title bar. The capped box scrolls itself: the kit's scrollable
        // fills its box by percentage, which a box capped only by a maximum never resolves.
        let room = crate::dialog_frame::titled_body_room(window);
        let title = body.read(cx).title();
        ui::with_close(
            guard.apply(crate::dialog_frame::place(d, window)),
            div()
                .id("jar-title")
                .test_support()
                .aria_label(title)
                .child(title),
        )
        .on_ok(|_, _, _| false)
        .child(
            div()
                .relative()
                .child(
                    div()
                        .id("cookie-jar-scroll")
                        .test_support()
                        .max_h(room.max(px(0.)))
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .child(body.clone()),
                )
                .vertical_scrollbar(&scroll),
        )
    });
    opened.opened(window, cx);
    view
}

// Closing with a typed form asks first: Save submits it (a refusal keeps the jar open with
// the marked fields), Don't save drops it.
fn form_guard(view: &Entity<CookieJarView>) -> crate::dialog_guard::Guard {
    use crate::dialog_guard::{Guard, settled};
    let (dirty, save, discard) = (view.downgrade(), view.downgrade(), view.downgrade());
    Guard::new(
        move |cx| dirty.upgrade().is_some_and(|v| v.read(cx).form_edited(cx)),
        |_| "Save this cookie?".into(),
        move |window, cx| {
            let saved = save
                .upgrade()
                .is_some_and(|v| v.update(cx, |v, cx| v.submit_form(window, cx)));
            settled(if saved { Ok(()) } else { Err("refused".into()) })
        },
        move |_, cx| {
            if let Some(v) = discard.upgrade() {
                v.update(cx, |v, cx| v.cancel_form(cx));
            }
        },
    )
}
