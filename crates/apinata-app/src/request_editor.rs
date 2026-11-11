//! The editor of one request tab: breadcrumb, method, URL and the section tables.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Weak};

use apinata_core::import::{
    ImportNote, ImportedRequest, ParsedRequest, RequestFormat, parse_request,
};
use apinata_core::{
    AuthSpec, BodySpec, CollectionNetwork, EnvVariable, GlobalEnvironments, HeaderSource, ItemPath,
    KeyValue, Param, ParamKind, PostResponseVar, RequestKind, RequestResult, RequestSettings,
    ScriptPhase, Snippet, VarValue, Variable, VariableSet,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem as _, SelectState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::Tab;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, ResizablePanelEvent, ResizableState, Sizable as _, h_flex, h_resizable,
    resizable_panel, v_flex, v_resizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Bounds, ClickEvent, ClipboardItem, ElementId, Empty, Entity,
    FocusHandle, Focusable, Global, Pixels, Point, Rems, Role, ScrollHandle, SharedString,
    StyleRefinement, Subscription, Task, TestSupportExt, Window, actions, canvas, div, px,
    relative, rems,
};

use crate::app::AppState;
use crate::assertions_section::AssertionsSection;
use crate::auth_section::{AuthMode, AuthSection};
use crate::banner::{self, Banner, Choice};
use crate::body_section::BodySection;
use crate::cached_part::{Same, Slot};
use crate::command::{Command, Edit, Field, Notice, NoticeKind, Target};
use crate::confirm;
use crate::default_headers_section::DefaultHeadersSection;
use crate::edit_history::EditHistory;
use crate::edit_track::{self, LoadStamps, same_value};
use crate::graphql_section::{GraphqlSection, SchemaAction};
use crate::history_key::history_key;
use crate::inherited_headers_section::{InheritedHeadersSection, OpenSource};
use crate::keymap::{SHELL_CONTEXT, SaveActive};
use crate::kv_table::{
    self, KvColumns, KvEvent, KvRow, KvTable, header_rows, headers_from, param_rows, params_from,
    post_response_block, post_response_rows, post_response_vars_from, var_rows, vars_from,
    vars_heading,
};
use crate::last_response::ResponseStore;
use crate::request_chrome::{Chrome, ChromePart};
use crate::response_pane::{HistoryTarget, ResponsePane, response_strip};
use crate::script_section::{ScriptSection, SideChosen, SnippetChosen};
use crate::scroll_follow::FocusFollow;
use crate::section::SectionEdit;
use crate::send_model::{Restored, SendModel, SendState};
use crate::settings::Orientation;
use crate::settings_section::{DocsSection, SettingsSection};
use crate::snapshot::{CollectionSnapshot, ConfigView, EnvView, RequestView, ScriptCode};
use crate::style;
use crate::tab_area::GenerateCode;
use crate::tabs::TabId;
use crate::tabs::TabKind;
use crate::token_panel::{Locate, TokenPanel};
use crate::ui::{self, IdItem, IdSelect, RingRoom as _, RingSides};
use crate::url_unit::{Side, UrlParamsUnit};
use crate::var_tokens::VarField;
use crate::workspace::{Workspace, WorkspaceEvent};

actions!(
    apinata,
    [
        /// Saves the request to its file.
        SaveRequest,
        /// Sends the request's current draft.
        SendRequest,
        /// Undoes the last edit to a table: params, headers, variables, assertions or form body.
        UndoEdit,
        /// Redoes the table edit [`UndoEdit`] took back.
        RedoEdit
    ]
);

/// The key context of the request editor, where Save and Send bind.
pub const KEY_CONTEXT: &str = "RequestEditor";

/// The custom method input's placeholder, short to fit; its tooltip says "Custom method".
pub const CUSTOM_METHOD_PLACEHOLDER: &str = "Other";

/// The URL bar's hint while empty; a pasted curl command becomes the request.
pub const URL_PLACEHOLDER: &str = "Enter URL or paste a curl command";

/// The methods the method picker always offers.
const METHODS: [&str; 9] = [
    "GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD", "TRACE", "QUERY",
];

/// A part of the request the editor shows, in Bruno's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Section {
    Params,
    Body,
    Headers,
    Auth,
    Vars,
    Script,
    Assertions,
    Tests,
    Docs,
    Settings,
}

impl Section {
    /// Every section, in tab order.
    pub const ALL: [Section; 10] = [
        Section::Params,
        Section::Body,
        Section::Headers,
        Section::Auth,
        Section::Vars,
        Section::Script,
        Section::Assertions,
        Section::Tests,
        Section::Docs,
        Section::Settings,
    ];

    /// The tab's label on an HTTP request.
    pub fn label(self) -> &'static str {
        match self {
            Section::Params => "Params",
            Section::Body => "Body",
            Section::Headers => "Headers",
            Section::Auth => "Auth",
            Section::Vars => "Vars",
            Section::Script => "Script",
            Section::Assertions => "Assertions",
            Section::Tests => "Tests",
            Section::Docs => "Docs",
            Section::Settings => "Settings",
        }
    }

    /// The tab's label on a request of `kind`: a GraphQL request's body is its query.
    pub fn label_for(self, kind: RequestKind) -> &'static str {
        match (self, kind) {
            (Section::Body, RequestKind::Graphql) => "Query",
            _ => self.label(),
        }
    }
}

/// The method picker's items: the usual methods, plus the file's own when it is not one of them.
pub fn method_items(current: &str) -> Vec<SharedString> {
    let mut items: Vec<SharedString> = METHODS.iter().map(|m| (*m).into()).collect();
    if !current.is_empty() && !METHODS.contains(&current) {
        items.push(current.to_string().into());
    }
    items
}

// The method select's rows, each found by its method as typed.
fn method_rows(current: &str) -> Vec<IdItem> {
    method_items(current)
        .into_iter()
        .map(|m| IdItem::new("method", &m, m.clone()))
        .collect()
}

/// What the last import paste into the URL bar produced, while its banner shows.
#[derive(Debug, Clone)]
pub enum Pasted {
    /// The paste was imported; these notes say what did not carry over unchanged.
    Imported(Vec<ImportNote>),
    /// The paste looked like an import but did not parse; the draft is unchanged.
    Failed(String),
}

/// Edits the request of one tab through the workspace; buffers follow the snapshots only
/// where no edit of theirs is in flight, so the user's own echo never moves the caret.
pub struct RequestEditor {
    workspace: Entity<Workspace>,
    compare: Option<Entity<crate::compare_dialog::CompareDialog>>,
    tab: TabId,
    url: Entity<InputState>,
    method: Entity<IdSelect>,
    // Takes any other verb; the select shows it once committed, so this stays empty.
    custom_method: Entity<InputState>,
    // Why the last commit was refused; cleared by a valid commit or any method change.
    custom_error: Option<SharedString>,
    // The draft text the refusal was raised for.
    custom_error_for: Option<SharedString>,
    params: Entity<KvTable>,
    headers: Entity<KvTable>,
    default_headers: Entity<DefaultHeadersSection>,
    inherited_headers: Entity<InheritedHeadersSection>,
    vars: Entity<KvTable>,
    post_response: Entity<KvTable>,
    body: Entity<BodySection>,
    graphql: Entity<GraphqlSection>,
    auth: Entity<AuthSection>,
    assertions: Entity<AssertionsSection>,
    settings: Entity<SettingsSection>,
    docs: Entity<DocsSection>,
    script: Entity<ScriptSection>,
    tests_script: Entity<ScriptSection>,
    send_model: Entity<SendModel>,
    // The Send, Cancel and Stop buttons swap inside this focus scope, and the last one drawn.
    slot_focus: FocusHandle,
    // Tracks the section body so a switch can tell the focused control is about to unmount.
    section_body_focus: FocusHandle,
    slot_kind: Cell<u8>,
    // The last live result seen, so each successful send refreshes the token panel once.
    last_result: Weak<RequestResult>,
    response: Entity<ResponsePane>,
    split: Entity<ResizableState>,
    split_axis: Orientation,
    // Stores a dragged split's share in the workspace; replaced with the split.
    _split_sub: Subscription,
    // The ratio the open split was last sized to; `None` until it is sized afresh.
    applied_ratio: Option<f32>,
    // The split's room, shared with the response pane for sizing a comparison.
    response_room: Rc<Cell<Pixels>>,
    base: Vec<Param>,
    header_base: Vec<KeyValue>,
    var_base: Vec<Variable>,
    post_response_base: Vec<PostResponseVar>,
    section: Section,
    // The section strip's scroll, so the chosen section can be scrolled into view.
    section_scroll: ScrollHandle,
    // The section body's scroll, reset on a section change so each section opens at its top.
    body_scroll: ScrollHandle,
    // Scrolls the section body to the control Tab lands on.
    follow: FocusFollow,
    // The request column's scroll, read for the cue that more lies below.
    column_scroll: ScrollHandle,
    // The focused GraphQL caret last followed, and how many renders may still follow it.
    caret_key: Option<(usize, usize)>,
    caret_follow: u8,
    // Renders left in which the section body follows the script caret a snippet insert placed.
    snippet_follow: u8,
    section_reveal: ui::Reveal,
    // Keeps the section tab that holds keyboard focus in view, apart from the chosen one.
    section_focus_reveal: ui::Reveal,
    section_fades: ui::StripFades,
    section_focus: Vec<FocusHandle>,
    pinned: bool,
    unit: UrlParamsUnit,
    // The structured edits this tab can undo and redo.
    history: EditHistory,
    // The dirty flag and reload stamp the last refresh saw; a save or a reload ends typing runs.
    checkpoint: Option<(bool, u64)>,
    // The generated test text just appended; its edit is an undo step, unlike script typing.
    script_step: Option<String>,
    sent: HashMap<u64, Section>,
    // The snapshots each field's controls loaded; its edits are built from that content.
    loaded: LoadStamps<Field>,
    // Set while a table change is sent whose rows the table no longer shows.
    lagging: bool,
    pasted: Option<Pasted>,
    // Counts pastes and dismissals, so a late reply to an older paste is ignored.
    paste_generation: u64,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
    _restore: Option<Task<()>>,
    variables: Option<Arc<VariableSet>>,
    // The context generation `variables` was built in.
    variables_context: u64,
    vars_key: Option<VarsKey>,
    _variables: Option<Task<()>>,
    // What the request sends, the selected environment and what it inherits as the last
    // refresh saw them; a change in any makes a shown error stale.
    seen_view: Option<SentShape>,
    seen_env: Option<Option<String>>,
    seen_inherited: Option<Inherited>,
    // The breadcrumb, URL row parts and section bar, each drawn as its own cached view.
    chrome: Vec<Slot<RequestEditor, PartInputs>>,
}

// Everything a chrome part's render reads that does not notify the part itself: a frame
// whose inputs differ from the last draws the part afresh. Theme, focus and key binding
// changes refresh the whole window, so they need no place here; the slot adds the layout.
#[derive(Clone, PartialEq)]
enum PartInputs {
    // Whether a refused verb marks the input; the icon appears or clears without the part
    // notifying, so the flag keys the redraw (the input view itself notifies on edits).
    CustomMethod(bool),
    // The method shown: the select's setters do not notify, so a reload's method must.
    Method(Option<SharedString>),
    Breadcrumb(Option<Crumbs>),
    // Whether the URL is locked, and the variable set (identity and revision) it tints from.
    Url(bool, Option<(Same<VariableSet>, u64)>),
    // Whether Save shows, and which of Send, Cancel and Stop the slot holds.
    Buttons(bool, u8),
    // The request kind, the shown section, and each section's count and mode mark.
    Sections(RequestKind, Section, Vec<(Option<usize>, bool)>),
}

// What the breadcrumb line shows.
#[derive(Clone, PartialEq)]
struct Crumbs {
    root: PathBuf,
    item: ItemPath,
    collection: SharedString,
    folders: Vec<(String, ItemPath)>,
    title: String,
    tooltip: Option<String>,
}

/// What the variable set was asked for: the request's view and the actor's context
/// generation. A view keeps its `Arc` until the request is touched, so identity is enough.
#[derive(Clone)]
struct VarsKey {
    request: Arc<RequestView>,
    context: u64,
}

impl PartialEq for VarsKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.request, &other.request) && self.context == other.context
    }
}

impl RequestEditor {
    /// An editor for the request tab `tab` of `workspace`.
    pub fn new(
        workspace: Entity<Workspace>,
        tab: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = target_of(&workspace, tab, cx)
            .and_then(|(root, item)| workspace.read(cx).request(&root, &item, cx));
        let url_text = view.as_ref().map(|v| v.url.clone()).unwrap_or_default();
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(URL_PLACEHOLDER)
                .default_value(url_text)
        });

        let current = view.as_ref().map(|v| v.method.clone()).unwrap_or_default();
        let methods = method_rows(&current);
        let selected = methods
            .iter()
            .position(|m| m.value() == current.as_str())
            .map(IndexPath::new);
        let method = cx.new(|cx| SelectState::new(methods, selected, window, cx));
        let custom_method =
            cx.new(|cx| InputState::new(window, cx).placeholder(CUSTOM_METHOD_PLACEHOLDER));

        // One watch serves every table and section in the section body's scroll area.
        let follow = FocusFollow::default();
        let params = cx.new(|cx| {
            let mut t = KvTable::new(window, cx);
            t.set_follow(follow.clone());
            t.set_columns(KvColumns::DESCRIBED, cx);
            t
        });
        let headers = cx.new(|cx| {
            let mut t = KvTable::new(window, cx);
            t.set_follow(follow.clone());
            t.set_columns(KvColumns::DESCRIBED, cx);
            t.set_header_hints(true);
            t
        });
        let vars = cx.new(|cx| {
            let mut t = KvTable::new(window, cx);
            t.set_follow(follow.clone());
            t
        });
        let post_response = cx.new(|cx| {
            let mut t = KvTable::new(window, cx);
            t.set_follow(follow.clone());
            t.set_placeholders("Name", "res.body.token", window, cx);
            t.set_headings("Name", "Expression", cx);
            t
        });
        let collection_root = target_of(&workspace, tab, cx)
            .map(|(root, _)| root)
            .unwrap_or_default();
        let body = cx.new(|cx| BodySection::new(collection_root, window, cx));
        let body_table = body.read(cx).table().clone();
        body_table.update(cx, |t, _| t.set_follow(follow.clone()));
        let graphql = cx.new(|cx| GraphqlSection::new(window, cx));
        let auth = cx.new(|cx| AuthSection::new(window, cx));
        // Only an OAuth 2.0 auth has a token to read; the weak handle avoids a cycle
        // through the auth section, which owns the panel.
        let locate: Locate = {
            let (ws, auth) = (workspace.clone(), auth.downgrade());
            Box::new(move |cx| {
                let auth = auth.upgrade()?;
                if auth.read(cx).mode() != Some(AuthMode::OAuth2) {
                    return None;
                }
                target_of(&ws, tab, cx).map(|(r, i)| (r, Target::Request(i)))
            })
        };
        let panel = cx.new(|cx| TokenPanel::new(workspace.clone(), locate, cx));
        let oauth2 = auth.read(cx).oauth2().clone();
        oauth2.update(cx, |o, cx| {
            o.set_token_panel(Some(panel), cx);
            o.set_follow(follow.clone());
        });
        let assertions = cx.new(|cx| {
            let mut section = AssertionsSection::new(window, cx);
            section.set_follow(follow.clone());
            section
        });
        let settings = cx.new(|cx| {
            let mut section = SettingsSection::new(window, cx);
            section.set_follow(follow.clone());
            section
        });
        let default_headers = cx.new(|cx| DefaultHeadersSection::new(window, cx));
        let inherited_headers = cx.new(|cx| InheritedHeadersSection::new(window, cx));
        let docs = cx.new(|cx| DocsSection::new(window, cx));
        let script = cx.new(|cx| {
            ScriptSection::new(
                &[ScriptPhase::PreRequest, ScriptPhase::PostResponse],
                window,
                cx,
            )
        });
        // A side chosen earlier for this request this session wins over the first load's default.
        if let Some(side) = target_of(&workspace, tab, cx)
            .and_then(|(root, item)| workspace.read(cx).script_side(&root, &item))
        {
            script.update(cx, |s, cx| s.show(side, cx));
        }
        let tests_script = cx.new(|cx| ScriptSection::new(&[ScriptPhase::Tests], window, cx));
        let send_model = cx.new(|_| SendModel::new(workspace.clone()));
        let history_target: HistoryTarget = {
            let ws = workspace.clone();
            Rc::new(move |cx| target_of(&ws, tab, cx))
        };
        let key: HistoryTarget = {
            let ws = workspace.clone();
            Rc::new(move |cx| history_key(ws.read(cx), tab))
        };
        let response = cx.new(|cx| {
            ResponsePane::new(send_model.clone(), window, cx).with_history(history_target, key, cx)
        });
        let response_body = response.read(cx).body().clone();
        response_body.update(cx, |b, _| b.set_assert_offered(true));
        let response_room = response.read(cx).room_cell();
        let restore = Self::restore_kept(&workspace, tab, &send_model, cx);
        let mut subs = vec![
            // The URL bar swaps Send and Cancel as the model starts and stops sending.
            cx.observe(&send_model, |this, _, cx| {
                this.clear_notice_after_send(cx);
                cx.notify()
            }),
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.refresh(window, cx)
            }),
            cx.subscribe_in(
                &url,
                window,
                |this, state, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = state.read(cx).value().to_string();
                        this.edit_url(&text, window, cx);
                    }
                },
            ),
            cx.subscribe_in(&workspace, window, |this, _, event, _, cx| match event {
                WorkspaceEvent::Notices { root, notices, .. } => {
                    this.mark_refused(root, notices, cx)
                }
                WorkspaceEvent::Delivered { root, acked, .. } => this.delivered(root, *acked, cx),
                WorkspaceEvent::SplitRatio => cx.notify(),
                _ => {}
            }),
            cx.subscribe_in(
                &custom_method,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    match event {
                        // An edit of the draft retires the refusal shown for the old one. Enter
                        // re-emits Change with the text unchanged, which must not.
                        InputEvent::Change => {
                            let text = this.custom_method.read(cx).value();
                            if this.custom_error_for.as_deref() != Some(text.as_ref())
                                && this.custom_error.take().is_some()
                            {
                                cx.notify();
                            }
                        }
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            this.commit_custom_method(window, cx);
                        }
                        _ => {}
                    }
                },
            ),
            cx.subscribe_in(
                &method,
                window,
                |this, _, event: &SelectEvent<Vec<IdItem>>, window, cx| {
                    if let SelectEvent::Confirm(Some(method)) = event {
                        // A user pick supersedes any draft; undo refreshes leave it alone.
                        this.custom_method
                            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
                        this.set_method(method, window, cx);
                    }
                },
            ),
        ];
        for (table, section) in [
            (&params, Section::Params),
            (&headers, Section::Headers),
            (&vars, Section::Vars),
        ] {
            subs.push(cx.subscribe_in(
                table,
                window,
                move |this, table, event: &KvEvent, window, cx| {
                    let KvEvent::Changed(rows) = event;
                    this.lagging = kv_table::lags(table, rows, cx);
                    this.edit_rows(section, rows.clone(), window, cx);
                    this.lagging = false;
                },
            ));
        }
        subs.push(cx.subscribe_in(
            &post_response,
            window,
            |this, table, event: &KvEvent, _, cx| {
                let KvEvent::Changed(rows) = event;
                this.lagging = kv_table::lags(table, rows, cx);
                this.edit_post_response_rows(rows, cx);
                this.lagging = false;
            },
        ));
        // Each section's edit goes out through the same send path as the tables, held back
        // like theirs when it came from rows a reload replaced. An edit is delivered after any
        // refresh queued ahead of it, so `lags` also asks whether the section, reloaded since,
        // would now send something else.
        let on_edit = |this: &mut Self, edit: &SectionEdit, lags: bool, cx: &mut Context<Self>| {
            this.lagging = edit.lagging || lags;
            this.send_typed(edit.edit.clone(), None, edit.typed, cx);
            this.lagging = false;
        };
        subs.extend([
            cx.subscribe_in(&body, window, move |this, section, e, _, cx| {
                let lags = section.read(cx).lags(&e.edit);
                on_edit(this, e, lags, cx)
            }),
            cx.subscribe_in(&graphql, window, move |this, _, e, _, cx| {
                on_edit(this, e, false, cx)
            }),
            cx.subscribe_in(
                &graphql,
                window,
                |this, section, _: &SchemaAction, window, cx| {
                    let Some((root, item)) = this.target(cx) else {
                        return;
                    };
                    let url = section.read(cx).schema_url().to_owned();
                    crate::schema_load::load_from_server(
                        &this.workspace,
                        &root,
                        item,
                        url,
                        window,
                        cx,
                    );
                },
            ),
            cx.subscribe_in(
                &auth,
                window,
                move |this, section, e: &SectionEdit, _, cx| {
                    this.lagging = section.read(cx).lags(e);
                    // Keystrokes in the auth fields share one typing key, so a run is one step.
                    this.send_typed(e.edit.clone(), Some(Section::Auth), Some(AUTH_TYPING), cx);
                    this.lagging = false;
                },
            ),
            cx.subscribe_in(&assertions, window, move |this, section, e, _, cx| {
                let lags = match &e.edit {
                    Edit::Assertions(sent) => section.read(cx).lags(sent, cx),
                    _ => false,
                };
                on_edit(this, e, lags, cx)
            }),
            // Assert this value on the response body adds a row to this request.
            cx.subscribe_in(
                &response_body,
                window,
                |this, _, e: &crate::body_view::AssertValue, window, cx| {
                    this.show(Section::Assertions, cx);
                    this.assertions
                        .update(cx, |s, cx| s.add_assertion(&e.0, window, cx));
                },
            ),
            // Schema test on the response body appends a generated test to the tests script.
            cx.subscribe_in(
                &response_body,
                window,
                |this, _, e: &crate::body_view::SchemaTest, window, cx| {
                    let current = this.tests_script.read(cx).texts(cx).concat();
                    let Some(text) = apinata_core::schema_test_script(&current, &e.0) else {
                        return;
                    };
                    this.show(Section::Tests, cx);
                    this.script_step = Some(text.clone());
                    let added = this.tests_script.update(cx, |s, cx| {
                        s.append_text(ScriptPhase::Tests, &text, window, cx)
                    });
                    // Nothing was emitted without an editor, so the mark must not linger.
                    if !added {
                        this.script_step = None;
                    }
                },
            ),
            cx.subscribe_in(&settings, window, move |this, section, e, _, cx| {
                let lags = section.read(cx).lags(&e.edit, cx);
                on_edit(this, e, lags, cx)
            }),
            cx.subscribe_in(&default_headers, window, move |this, section, e, _, cx| {
                let lags = section.read(cx).lags(&e.edit);
                on_edit(this, e, lags, cx)
            }),
            cx.subscribe_in(
                &inherited_headers,
                window,
                |this, _, e: &OpenSource, _, cx| this.open_header_source(&e.0, cx),
            ),
            // The text sections read their editor when its change arrives, after any refresh,
            // so their edits never predate a load.
            cx.subscribe_in(&docs, window, move |this, _, e, _, cx| {
                on_edit(this, e, false, cx)
            }),
            cx.subscribe_in(&script, window, move |this, _, e, _, cx| {
                on_edit(this, e, false, cx)
            }),
            cx.subscribe_in(&tests_script, window, move |this, _, e, _, cx| {
                on_edit(this, e, false, cx)
            }),
            cx.subscribe_in(&script, window, |this, _, e: &SnippetChosen, window, cx| {
                this.insert_snippet(e.0, e.1, window, cx);
            }),
            cx.subscribe_in(
                &tests_script,
                window,
                |this, _, e: &SnippetChosen, window, cx| {
                    this.insert_snippet(e.0, e.1, window, cx);
                },
            ),
            cx.subscribe(&script, |this, _, e: &SideChosen, cx| {
                let ws = this.workspace.clone();
                if let Some((root, item)) = target_of(&ws, this.tab, cx) {
                    ws.update(cx, |w, _| w.set_script_side(&root, item, e.0));
                }
            }),
        ]);
        // The split follows a settings change, such as the orientation toggle, at once.
        if let Some(app) = cx.try_global::<AppState>() {
            subs.push(cx.observe(&app.settings.clone(), |_, _, cx| cx.notify()));
        }
        let split = cx.new(|_| ResizableState::default());
        let split_sub = Self::watch_split(&split, cx);
        let split_axis = Self::orientation_in(cx);
        let me = cx.weak_entity();
        let chrome = ChromePart::ALL
            .iter()
            .map(|&part| Slot::new(me.clone(), part, part.sized_by_content(), cx))
            .collect();
        let mut editor = RequestEditor {
            workspace,
            compare: None,
            tab,
            url,
            method,
            custom_method,
            custom_error: None,
            custom_error_for: None,
            params,
            headers,
            default_headers,
            inherited_headers,
            vars,
            post_response,
            body,
            graphql,
            auth,
            assertions,
            settings,
            docs,
            script,
            tests_script,
            send_model,
            slot_focus: cx.focus_handle(),
            section_body_focus: cx.focus_handle(),
            slot_kind: Cell::new(0),
            last_result: Weak::new(),
            response,
            split,
            split_axis,
            _split_sub: split_sub,
            applied_ratio: None,
            response_room,
            base: Vec::new(),
            header_base: Vec::new(),
            var_base: Vec::new(),
            post_response_base: Vec::new(),
            section: Section::Params,
            section_scroll: ScrollHandle::new(),
            body_scroll: ScrollHandle::new(),
            follow,
            column_scroll: ScrollHandle::new(),
            caret_key: None,
            caret_follow: 0,
            snippet_follow: 0,
            section_reveal: ui::Reveal::default(),
            section_focus_reveal: ui::Reveal::default(),
            section_fades: ui::StripFades::default(),
            section_focus: Section::ALL
                .iter()
                .map(|_| cx.focus_handle().tab_stop(true))
                .collect(),
            pinned: false,
            unit: UrlParamsUnit::default(),
            history: EditHistory::default(),
            checkpoint: None,
            script_step: None,
            sent: HashMap::new(),
            loaded: LoadStamps::default(),
            lagging: false,
            pasted: None,
            paste_generation: 0,
            focus: cx.focus_handle(),
            _subs: subs,
            _restore: restore,
            variables: None,
            variables_context: 0,
            vars_key: None,
            _variables: None,
            seen_view: None,
            seen_env: None,
            seen_inherited: None,
            chrome,
        };
        editor.refresh(window, cx);
        editor
    }

    // The kept response loads off the UI thread; `restore` drops it if a send began meanwhile.
    fn restore_kept(
        workspace: &Entity<Workspace>,
        tab: TabId,
        send_model: &Entity<SendModel>,
        cx: &mut Context<Self>,
    ) -> Option<Task<()>> {
        let (root, item) = history_key(workspace.read(cx), tab)?;
        let store = cx.try_global::<ResponseStore>()?.clone();
        // Kept sends stay visible with remembering off, but then reading them must not migrate
        // or drop anything in the store.
        let remember = cx
            .try_global::<AppState>()
            .is_none_or(|app| app.settings.read(cx).get().remember_responses);
        let model = send_model.downgrade();
        Some(cx.spawn(async move |_, cx| {
            let load = cx.background_spawn(async move {
                if remember {
                    store.load(&root, &item)
                } else {
                    store.peek(&root, &item)
                }
            });
            let Some(stored) = load.await else { return };
            let restored = Restored {
                saved_at: stored.saved_at,
                body_dropped: stored.body_dropped,
                body_masked: stored.body_masked,
                masked_headers: stored.masked_headers,
                header_digests: stored.header_digests,
            };
            let result = Arc::new(stored.result);
            let _ = model.update(cx, |m, cx| m.restore(result, restored, cx));
        }))
    }

    // A new live result without an error clears the token panel's stale notice; a kept or
    // history result (`restored` set) is not a new send.
    fn clear_notice_after_send(&mut self, cx: &mut Context<Self>) {
        let model = self.send_model.read(cx);
        let SendState::Done(result) = model.state() else {
            return;
        };
        if model.restored().is_some() || Weak::ptr_eq(&self.last_result, &Arc::downgrade(result)) {
            return;
        }
        let ok = result.error.is_none();
        self.last_result = Arc::downgrade(result);
        if ok {
            refresh_token_panel(&self.auth, cx);
        }
    }

    /// The model sending this tab's request.
    pub fn send_model(&self) -> &Entity<SendModel> {
        &self.send_model
    }

    /// The pane showing this tab's response.
    pub fn response(&self) -> &Entity<ResponsePane> {
        &self.response
    }

    /// Sends the request as it is now; the tab's root and item are read at call time, so
    /// a rename or move the tab followed sends the new path.
    pub fn send(&mut self, _: &SendRequest, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((root, item)) = self.target(cx) else {
            return;
        };
        self.commit_pending_secret(cx);
        let key = history_key(self.workspace.read(cx), self.tab);
        self.send_model
            .update(cx, |m, cx| m.send_keyed(&root, item, key, cx));
    }

    /// The variables the request reads, once the actor has answered.
    pub fn variables(&self) -> Option<&Arc<VariableSet>> {
        self.variables.as_ref()
    }

    /// The URL bar's input.
    pub fn url_input(&self) -> &Entity<InputState> {
        &self.url
    }

    /// The URL as typed.
    pub fn url(&self, cx: &App) -> String {
        self.url.read(cx).value().to_string()
    }

    /// The URL input's focus.
    pub fn url_focus(&self, cx: &App) -> FocusHandle {
        self.url.read(cx).focus_handle(cx)
    }

    /// Puts the URL input's caret after its last character.
    pub fn url_caret_to_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let end = gpui_kit::component::input::Position::new(0, u32::MAX);
        self.url
            .update(cx, |u, cx| u.set_cursor_position(end, window, cx));
    }

    /// Where the URL input's caret is, in bytes.
    pub fn url_caret(&self, cx: &App) -> usize {
        self.url.read(cx).cursor()
    }

    /// The method picker's focus.
    pub fn method_focus(&self, cx: &App) -> FocusHandle {
        self.method.read(cx).focus_handle(cx)
    }

    /// The focus of section tab `ix`, an index into [`Section::ALL`].
    pub fn section_focus(&self, ix: usize) -> FocusHandle {
        self.section_focus[ix].clone()
    }

    /// The section shown under the URL bar.
    pub fn section(&self) -> Section {
        self.section
    }

    /// The method picker's state.
    pub fn method_select(&self) -> &Entity<IdSelect> {
        &self.method
    }

    /// The method the picker shows.
    pub fn method(&self, cx: &App) -> Option<SharedString> {
        self.method.read(cx).selected_value().cloned()
    }

    /// The table of a table section: params, headers or vars.
    pub fn table(&self, section: Section) -> Option<&Entity<KvTable>> {
        match section {
            Section::Params => Some(&self.params),
            Section::Headers => Some(&self.headers),
            Section::Vars => Some(&self.vars),
            _ => None,
        }
    }

    /// The Post-response table under the Vars table.
    pub fn post_response_table(&self) -> &Entity<KvTable> {
        &self.post_response
    }

    /// The skip hints of the enabled rows with an expression, in row order.
    pub fn post_response_hints(&self, cx: &App) -> Vec<String> {
        let rows = self.post_response.read(cx).rows(cx);
        kv_table::post_response_hints(&self.post_response_base, &rows)
    }

    /// The body section.
    pub fn body(&self) -> &Entity<BodySection> {
        &self.body
    }

    /// The Default headers group under the headers.
    pub fn default_headers(&self) -> &Entity<DefaultHeadersSection> {
        &self.default_headers
    }

    /// Beautifies the body of an HTTP request; a GraphQL request has no body editor, so it
    /// does nothing there.
    pub fn beautify_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view(cx).is_some_and(|v| v.kind == RequestKind::Http) {
            self.body.update(cx, |b, cx| b.beautify(window, cx));
        }
    }

    /// Whether Beautify and Minify would change anything: an HTTP request with a non-empty
    /// JSON or XML body.
    pub fn can_reformat_body(&self, cx: &App) -> bool {
        self.view(cx).is_some_and(|v| v.kind == RequestKind::Http)
            && self.body.read(cx).can_reformat(cx)
    }

    /// Whether a response is streaming in, which Stop can end.
    pub fn is_streaming(&self, cx: &App) -> bool {
        self.send_model.read(cx).is_streaming()
    }

    /// Minifies the body of an HTTP request; a GraphQL request has no body editor, so it
    /// does nothing there.
    pub fn minify_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view(cx).is_some_and(|v| v.kind == RequestKind::Http) {
            self.body.update(cx, |b, cx| b.minify(window, cx));
        }
    }

    /// Formats the query of a GraphQL request; an HTTP request has no query, so it does nothing.
    pub fn beautify_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .view(cx)
            .is_some_and(|v| v.kind == RequestKind::Graphql)
        {
            self.graphql
                .update(cx, |g, cx| g.beautify_query(window, cx));
        }
    }

    /// Whether Beautify query would do anything: a GraphQL request with a non-blank query.
    pub fn can_beautify_query(&self, cx: &App) -> bool {
        self.view(cx)
            .is_some_and(|v| v.kind == RequestKind::Graphql)
            && self.graphql.read(cx).can_beautify_query(cx)
    }

    /// The Query section of a GraphQL request.
    pub fn graphql(&self) -> &Entity<GraphqlSection> {
        &self.graphql
    }

    /// The auth section.
    pub fn auth(&self) -> &Entity<AuthSection> {
        &self.auth
    }

    /// The assertions section.
    pub fn assertions(&self) -> &Entity<AssertionsSection> {
        &self.assertions
    }

    /// The settings section.
    pub fn settings(&self) -> &Entity<SettingsSection> {
        &self.settings
    }

    /// The docs section.
    pub fn docs(&self) -> &Entity<DocsSection> {
        &self.docs
    }

    /// The Script section: the pre-request and post-response editors.
    pub fn script(&self) -> &Entity<ScriptSection> {
        &self.script
    }

    /// The Tests section's editor.
    pub fn tests_script(&self) -> &Entity<ScriptSection> {
        &self.tests_script
    }

    /// Inserts `snippet` at the caret of the `phase` script editor, one undo step like the
    /// Schema test. Shows that editor; false when it is missing or locked.
    pub fn insert_snippet(
        &mut self,
        phase: ScriptPhase,
        snippet: &Snippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (section, view) = match phase {
            ScriptPhase::Tests => (Section::Tests, self.tests_script.clone()),
            _ => (Section::Script, self.script.clone()),
        };
        let Some(text) = view.read(cx).snippet_text(phase, snippet, cx) else {
            return false;
        };
        self.show(section, cx);
        view.update(cx, |s, cx| s.show(phase, cx));
        self.script_step = Some(text);
        let added = view.update(cx, |s, cx| s.insert_snippet(phase, snippet, window, cx));
        if !added {
            self.script_step = None;
        } else {
            self.snippet_follow = 3;
            cx.notify();
        }
        added
    }

    /// Sends the URL `text`, unless the params side has an edit in flight.
    pub fn edit_url(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let in_flight = self.in_flight(cx);
        if !self.unit.edit(Side::Url, in_flight) {
            return;
        }
        self.send_edit(Edit::Url(text.to_string()), None, cx);
    }

    /// Sends `method` as the request's method.
    pub fn set_method(&mut self, method: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.custom_error = None;
        self.send_edit(Edit::Method(method.to_string()), None, cx);
    }

    /// The custom method input's focus handle, the tab stop between the select and the URL.
    pub fn custom_method_focus(&self, cx: &App) -> FocusHandle {
        self.custom_method.read(cx).focus_handle(cx)
    }

    /// The custom method input.
    pub fn custom_method_input(&self) -> &Entity<InputState> {
        &self.custom_method
    }

    /// Why the last custom method was refused, while the line shows.
    pub fn custom_method_error(&self) -> Option<&str> {
        self.custom_error.as_deref()
    }

    /// Commits the custom method input (Enter and blur): a valid verb becomes the request's
    /// method and the input empties; an invalid one shows an error and changes nothing.
    pub fn commit_custom_method(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.custom_method.read(cx).value().to_string();
        if text.is_empty() {
            self.custom_error = None;
            cx.notify();
            return;
        }
        match apinata_core::normalize_method(&text) {
            Ok(method) => {
                self.custom_method
                    .update(cx, |s, cx| s.set_value(String::new(), window, cx));
                self.set_method(&method, window, cx);
            }
            Err(e) => {
                self.custom_error = Some(e.to_string().into());
                self.custom_error_for = Some(text.into());
            }
        }
        cx.notify();
    }

    /// Sends `rows` as the new contents of `section` and clears its refusal message; a
    /// params edit is refused while the URL side has an edit in flight.
    pub fn edit_rows(
        &mut self,
        section: Section,
        rows: Vec<KvRow>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = match section {
            Section::Params => {
                let in_flight = self.in_flight(cx);
                if !self.unit.edit(Side::Params, in_flight) {
                    return;
                }
                Edit::Params(params_from(&self.base, &rows))
            }
            Section::Headers => {
                let headers = headers_from(&self.header_base, &rows);
                self.default_headers
                    .update(cx, |d, cx| d.set_own(&headers, cx));
                self.inherited_headers
                    .update(cx, |d, cx| d.set_own(&headers, cx));
                Edit::Headers(headers)
            }
            Section::Vars => Edit::Vars(vars_from(&self.var_base, &rows)),
            _ => return,
        };
        if let Some(table) = self.table(section).cloned() {
            table.update(cx, |t, cx| t.set_error(None, None, cx));
        }
        let typed = self.table(section).and_then(|t| t.read(cx).last_typed());
        self.send_typed(edit, Some(section), typed, cx);
    }

    fn edit_post_response_rows(&mut self, rows: &[KvRow], cx: &mut Context<Self>) {
        let vars = post_response_vars_from(&self.post_response_base, rows);
        let typed = self.post_response.read(cx).last_typed();
        self.send_typed(Edit::PostResponseVars(vars), None, typed, cx);
    }

    /// Takes back the last structured edit (one table action or a run of keystrokes in a
    /// cell) by sending its earlier value. Does nothing when there is none, when the field
    /// was changed from outside (which forgets that field's steps), or when the params side
    /// is locked by a URL edit in flight.
    pub fn undo_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, cx);
    }

    /// Redoes the edit [`Self::undo_edit`] took back, under the same conditions.
    pub fn redo_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, cx);
    }

    // Applies the history's next value as an ordinary, unrecorded edit. While the field has
    // an edit in flight the view lags, so the history trusts its own last sent value;
    // otherwise the view must still hold what the step left. The tables follow the echo.
    fn step_history(&mut self, undo: bool, cx: &mut Context<Self>) {
        let Some(field) = self.history.next_field(undo) else {
            return;
        };
        let Some((_, item)) = self.target(cx) else {
            return;
        };
        let Some(view) = self.view(cx) else {
            return;
        };
        let busy = self.busy(field, cx);
        if field == Field::UrlParams && self.unit.locked(Side::Params, busy) {
            return;
        }
        let holds = |edit: &Edit| view_holds(&view, edit);
        let Some(edit) = edit_track::step(&mut self.history, undo, busy, &holds) else {
            return;
        };
        // Only now is something to be sent, so only now does the params side claim the lock.
        if field == Field::UrlParams {
            self.unit.edit(Side::Params, busy);
        }
        // Scripts are only history steps when appended, so a redone one is the user's own text.
        if let (false, Edit::Script(phase, text)) = (undo, &edit) {
            self.script.update(cx, |s, _| s.expect_redo(*phase, text));
            (self.tests_script).update(cx, |s, _| s.expect_redo(*phase, text));
        }
        let section = table_section(field);
        if let Some(table) = section.and_then(|s| self.table(s).cloned()) {
            table.update(cx, |t, cx| t.set_error(None, None, cx));
        }
        let command = Command::Edit {
            target: Target::Request(item),
            edit,
        };
        self.send_command(command, section, cx);
    }

    /// Takes `text` when `RequestFormat::detect` recognises it: parses it, sends `ApplyImport` for
    /// this tab's request, and keeps the notes (or the error) for the banner. When the paste
    /// would lose unsaved changes or a body it asks first, and Cancel changes nothing. `false`
    /// leaves the paste to the input.
    pub fn paste_url(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if RequestFormat::detect(text).is_none() {
            return false;
        }
        // A recognised paste is taken even when it fails to parse, so a broken command never
        // lands in the URL; it reports at once, since there is nothing to replace.
        let parsed = match parse_request(None, text) {
            Ok(parsed) => parsed,
            Err(e) => {
                self.paste_generation += 1;
                self.pasted = Some(Pasted::Failed(e.to_string()));
                cx.notify();
                return true;
            }
        };
        // A GraphQL request keeps its body against a paste, so only an HTTP body is at stake.
        let (dirty, body) = self.view(cx).map_or((false, false), |v| {
            let body = v.kind == RequestKind::Http && !matches!(v.body, BodySpec::None);
            (v.dirty, body)
        });
        if !dirty && !body {
            self.apply_paste(parsed, cx);
            return true;
        }
        let detail = match (dirty, body) {
            (true, true) => "Unsaved changes and the current body will be replaced.",
            (true, false) => "Unsaved changes will be replaced.",
            _ => "The current body will be replaced.",
        };
        let ask = confirm::Ask {
            title: "Replace this request with the pasted command?".into(),
            detail: Some(detail.into()),
            choices: vec![
                confirm::Choice {
                    id: "confirm-replace",
                    label: "Replace".into(),
                    danger: true,
                },
                confirm::Choice {
                    id: "confirm-cancel",
                    label: "Cancel".into(),
                    danger: false,
                },
            ],
            default: 1,
            cancel: 1,
        };
        let answer = confirm::ask(ask, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.recv().await == Ok(0) {
                this.update(cx, |e, cx| e.apply_paste(parsed, cx)).ok();
            }
        })
        .detach();
        true
    }

    // Sends the parsed paste to this tab's request and shows its notes, if any, in the banner.
    fn apply_paste(&mut self, mut parsed: ParsedRequest, cx: &mut Context<Self>) {
        let mut notes = parsed.notes;
        if let Some(view) = self.view(cx) {
            // A GraphQL request takes a pasted GraphQL payload as its query and variables.
            if view.kind == RequestKind::Graphql
                && let Some(graphql) = parsed.request.as_graphql()
            {
                parsed.request = graphql;
            }
            notes.extend(parsed.request.notes_on(view.kind));
        }
        let secrets = parsed.request.secrets.clone();
        let mut stored = ImportedRequest::new("GET", "");
        stored.secrets = secrets.clone();
        // Replies of earlier pastes must not touch this paste's banner.
        self.paste_generation += 1;
        let generation = self.paste_generation;
        let mut apply = None;
        if let Some((_, item)) = self.target(cx) {
            let request = parsed.request;
            apply = self
                .send_command(
                    Command::ApplyImport {
                        item: item.clone(),
                        request,
                    },
                    None,
                    cx,
                )
                .map(|_| item);
        }
        // The old response answered the old request.
        self.send_model.update(cx, |m, cx| m.clear_result(cx));
        // A secret the command carried is kept by the actor once the import landed; it answers
        // with the notes updated to say where it went or why it was not stored.
        if let (Some(item), false) = (apply, secrets.is_empty()) {
            let (reply, answer) = async_channel::bounded(1);
            let command = Command::StoreImportSecrets {
                item,
                request: stored,
                notes: notes.clone(),
                reply,
            };
            if self.send_command(command, None, cx).is_some() {
                cx.spawn(async move |this, cx| {
                    if let Ok(notes) = answer.recv().await {
                        this.update(cx, |e, cx| {
                            if e.paste_generation == generation {
                                e.pasted = Some(Pasted::Imported(notes));
                                cx.notify();
                            }
                        })
                        .ok();
                    }
                })
                .detach();
            }
        }
        self.pasted = (!notes.is_empty()).then_some(Pasted::Imported(notes));
        cx.notify();
    }

    /// What the last import paste produced, while its banner shows; `None` after a clean import.
    pub fn pasted(&self) -> Option<&Pasted> {
        self.pasted.as_ref()
    }

    /// Hides the paste banner.
    pub fn dismiss_pasted(&mut self, cx: &mut Context<Self>) {
        self.paste_generation += 1;
        self.pasted = None;
        cx.notify();
    }

    /// Enabled entries of a counted section (Params, Headers, Vars, Assertions) in the
    /// snapshot's view of this tab's request; `None` for the other sections or no request.
    pub fn section_count(&self, section: Section, cx: &App) -> Option<usize> {
        let view = self.view(cx)?;
        match section {
            Section::Params => Some(view.params.iter().filter(|p| p.enabled).count()),
            Section::Headers => Some(view.headers.iter().filter(|h| h.enabled).count()),
            Section::Vars => {
                let vars = view.vars.iter().filter(|v| v.enabled).count();
                Some(
                    vars + view
                        .post_response_vars
                        .iter()
                        .filter(|v| v.enabled())
                        .count(),
                )
            }
            Section::Assertions => Some(view.assertions.iter().filter(|a| a.enabled()).count()),
            _ => None,
        }
    }

    /// Whether Body or Auth has a mode other than none, Script holds a pre-request or
    /// post-response script, or Tests holds a tests script; `false` for every other section.
    pub fn section_marked(&self, section: Section, cx: &App) -> bool {
        self.view(cx).is_some_and(|view| match section {
            Section::Body => match &view.body {
                BodySpec::Graphql { query, variables } => {
                    !query.is_empty() || !variables.is_empty()
                }
                body => !matches!(body, BodySpec::None),
            },
            Section::Auth => !matches!(view.auth, AuthSpec::None),
            Section::Script => {
                !view.scripts.pre_request.is_empty() || !view.scripts.post_response.is_empty()
            }
            Section::Tests => !view.scripts.tests.is_empty(),
            _ => false,
        })
    }

    // What a marked section's ring says when hovered.
    fn mark_words(section: Section) -> &'static str {
        match section {
            Section::Body => "Has a body",
            Section::Auth => "Has auth",
            Section::Tests => "Has tests",
            _ => "Has a script",
        }
    }

    // The kind of the request shown; HTTP until its snapshot arrives.
    fn kind(&self, cx: &App) -> RequestKind {
        self.view(cx).map_or(RequestKind::Http, |v| v.kind)
    }

    fn view(&self, cx: &App) -> Option<Arc<RequestView>> {
        let (root, item) = self.target(cx)?;
        self.workspace.read(cx).request(&root, &item, cx)
    }

    /// Writes the request to its file; a draft goes to the shell's save instead, which asks
    /// for its name and folder.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending_secret(cx);
        if let Some((root, item)) = self.target(cx) {
            if item.is_draft() {
                window.dispatch_action(Box::new(SaveActive), cx);
                return;
            }
            // The typed scripts are approved only once the write is confirmed.
            let target = Target::Request(item);
            let texts = self.typed_scripts(cx);
            self.workspace.update(cx, |w, cx| {
                let id = w.send(&root, Command::Save(target.clone()), cx);
                w.approve_when_saved(&root, id, Some(target), texts);
            });
        }
    }

    /// The script and test texts the user typed here, never text loaded from the file:
    /// what a save of this editor vouches for.
    pub fn typed_scripts(&self, cx: &App) -> Vec<String> {
        let mut texts = self.script.read(cx).typed_texts(cx);
        texts.extend(self.tests_script.read(cx).typed_texts(cx));
        texts
    }

    /// Whether an OAuth secret is typed here but not yet sent.
    pub fn holds_secret_typing(&self, cx: &App) -> bool {
        self.auth.read(cx).holds_pending(cx)
    }

    /// Sends the edit an OAuth secret still focused owes, as its blur would, so a save or
    /// send after it finds the secret moved to `.env`; true when one went out.
    pub fn commit_pending_secret(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.auth.update(cx, |a, cx| a.take_pending(cx)) else {
            return false;
        };
        // Moving a typed secret out is not an undo step; it only retires the auth steps.
        self.send_unrecorded(edit, Some(Section::Auth), cx);
        true
    }

    /// Whether `side` is read-only because the other side has an edit in flight.
    pub fn locked(&self, side: Side, cx: &App) -> bool {
        self.unit.locked(side, self.in_flight(cx))
    }

    /// Test seam: records `side` as the last sender without notifying, so a test can move the
    /// URL lock alone.
    #[doc(hidden)]
    pub fn set_sender_silently(&mut self, side: Side) {
        self.unit.edit(side, false);
    }

    fn target(&self, cx: &App) -> Option<(PathBuf, ItemPath)> {
        target_of(&self.workspace, self.tab, cx)
    }

    fn in_flight(&self, cx: &App) -> bool {
        self.busy(Field::UrlParams, cx)
    }

    fn busy(&self, field: Field, cx: &App) -> bool {
        self.target(cx).is_some_and(|(root, item)| {
            let target = Target::Request(item);
            self.workspace.read(cx).in_flight(&root, &target, field, cx)
        })
    }

    // The first edit pins a preview tab. Ids the actor has acknowledged had their notices
    // delivered already, so only newer ones stay mapped to the section that sent them.
    fn send_edit(&mut self, edit: Edit, section: Option<Section>, cx: &mut Context<Self>) {
        self.send_typed(edit, section, None, cx);
    }

    // `typed` is the cell a keystroke landed in, so a run of keystrokes there is one step.
    fn send_typed(
        &mut self,
        edit: Edit,
        section: Option<Section>,
        typed: Option<(usize, u8)>,
        cx: &mut Context<Self>,
    ) {
        let Some((_, item)) = self.target(cx) else {
            self.script_step = None;
            return;
        };
        match edit {
            // The secret move rewrites values the history cannot restore.
            Edit::MoveAuthSecrets => {}
            // Script keystrokes stay with the code editor's own undo, unless they follow a script
            // step: then they are a typing run of their own, so undo takes the typing and the
            // step apart instead of dropping the step as stale.
            Edit::Script(_, ref text) if self.script_step.take().as_ref() != Some(text) => {
                // The editor's own undo or redo of a step moves that step across the stacks
                // and is not a new step.
                let followed = self.history.follow_editor(&edit);
                if !followed && self.history.next_field(true) == Some(edit.field()) {
                    self.track(&edit, Some((0, 0)), cx);
                } else if !followed {
                    self.history.note_sent(&edit);
                }
            }
            _ => self.track(&edit, typed, cx),
        }
        self.dispatch(edit, section, item, cx);
    }

    fn send_unrecorded(&mut self, edit: Edit, section: Option<Section>, cx: &mut Context<Self>) {
        let Some((_, item)) = self.target(cx) else {
            return;
        };
        self.history.note_sent(&edit);
        self.dispatch(edit, section, item, cx);
    }

    fn dispatch(
        &mut self,
        edit: Edit,
        section: Option<Section>,
        item: ItemPath,
        cx: &mut Context<Self>,
    ) {
        let auth = edit.field() == Field::Auth;
        if auth {
            self.auth.update(cx, |a, cx| a.set_notice(None, cx));
        }
        let command = Command::Edit {
            target: Target::Request(item),
            edit,
        };
        self.send_command(command, section, cx);
        // The panel's plan queues behind the edit, so it reads the edited auth.
        if auth {
            refresh_token_panel(&self.auth, cx);
        }
    }

    // Notes the snapshot behind the fields this refresh loads, which are the idle ones; the
    // Script section loads both of its phases or neither.
    fn note_loaded(&mut self, root: &Path, cx: &App) {
        let Some(seq) = self.workspace.read(cx).snapshot(root, cx).map(|s| s.seq) else {
            return;
        };
        let script = [Field::ScriptPreRequest, Field::ScriptPostResponse];
        let idle_script = !script.iter().any(|&f| self.busy(f, cx));
        let fields = LOADED_ALONE.iter().filter(|&&f| !self.busy(f, cx)).copied();
        let loaded: Vec<Field> = fields
            .chain(script.into_iter().filter(|_| idle_script))
            .collect();
        for field in loaded {
            self.loaded.note(field, seq);
        }
    }

    // Request-only cases come first: URL text has its own undo in the bar and only retires
    // params steps, and text typing in a body is left to the body editor. The rest is shared.
    fn track(&mut self, edit: &Edit, typed: Option<(usize, u8)>, cx: &App) {
        let Some(view) = self.view(cx) else {
            self.history.note_sent(edit);
            return;
        };
        let read = |e: &Edit| view_edit(&view, e);
        if matches!(edit, Edit::Url(_)) {
            self.history.note_sent(edit);
            return;
        }
        let busy = self.busy(edit.field(), cx);
        edit_track::rebase(&mut self.history, busy, &read, edit);
        let prior = edit_track::prior_of(&self.history, &read, edit);
        if prior.is_some_and(|before| body_typing(&before, edit) && !same_value(&before, edit)) {
            self.history.note_sent(edit);
            return;
        }
        edit_track::record(&mut self.history, edit, typed, &read);
    }

    fn send_command(
        &mut self,
        command: Command,
        section: Option<Section>,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let Some((root, _)) = self.target(cx) else {
            self.history.revert_staged();
            return None;
        };
        // An edit carries the snapshot its field last loaded, so the actor holds it back when
        // a reload replaced that content before the field could show it. Lagging rows came
        // from the load before that.
        let basis = match &command {
            Command::Edit { edit, .. } => self.loaded.basis(&edit.field(), self.lagging),
            _ => None,
        };
        let id = self
            .workspace
            .update(cx, |w, cx| w.send_from(&root, command, basis, cx));
        self.history.bind(id);
        if let (Some(id), Some(section)) = (id, section) {
            self.sent.insert(id, section);
        }
        if !self.pinned {
            self.pinned = true;
            let tab = self.tab;
            self.workspace
                .update(cx, |w, cx| w.update_tabs(cx, |t| t.keep(tab)));
        }
        self.sync_locks(cx);
        id
    }

    // Commands up to `acked` had their notices handed over already, so only newer ones stay
    // mapped; a send before that delivery must not drop an outcome still on its way.
    fn delivered(&mut self, root: &Path, acked: u64, cx: &App) {
        if self.target(cx).is_some_and(|(ours, _)| ours == root) {
            self.sent.retain(|&id, _| id > acked);
            self.history.settle(acked);
        }
    }

    // A `Failed` notice for an edit this editor sent shows its message under that
    // section's table, as do an auth edit's warnings; the next edit there clears it.
    fn mark_refused(&mut self, root: &Path, notices: &[Notice], cx: &mut Context<Self>) {
        if self.target(cx).is_none_or(|(ours, _)| ours != root) {
            return;
        }
        for notice in notices {
            let Some(section) = self.sent.remove(&notice.command) else {
                continue;
            };
            // The refused edit never took effect, so the history forgets what it did.
            edit_track::refused(&mut self.history, notice);
            if section == Section::Auth {
                if let Some(text) = crate::section::notice_text(&notice.kind) {
                    self.auth.update(cx, |a, cx| a.set_notice(Some(text), cx));
                }
                continue;
            }
            if let (NoticeKind::Failed { message, row, .. }, Some(table)) =
                (&notice.kind, self.table(section).cloned())
            {
                let row = row.filter(|_| section == Section::Headers);
                table.update(cx, |t, cx| t.set_error(Some(message.clone()), row, cx));
            }
        }
    }

    fn sync_locks(&mut self, cx: &mut Context<Self>) {
        let locked = self.locked(Side::Params, cx);
        self.params.update(cx, |t, cx| {
            if t.locked() != locked {
                t.set_locked(locked, cx);
            }
        });
        cx.notify();
    }

    // Ask the actor for a fresh variable set whenever the request's view or the context
    // generation moved; runtime captures need no new set, since the set reads them live.
    fn watch_variables(&mut self, cx: &mut Context<Self>) {
        let Some((root, item)) = self.target(cx) else {
            return;
        };
        let Some(snap) = self.workspace.read(cx).snapshot(&root, cx) else {
            return;
        };
        let Some(request) = snap.content.requests.get(&item).cloned() else {
            return;
        };
        let key = VarsKey {
            request,
            context: snap.context_generation,
        };
        if self.vars_key.as_ref() == Some(&key) {
            return;
        }
        let context = key.context;
        self.vars_key = Some(key);
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::Variables { item, reply };
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        self._variables = Some(cx.spawn(async move |this, cx| {
            // A context that fails to build drops the old set, so no tooltip shows the
            // previous environment's values; a closed channel leaves the set as it was.
            let Ok(reply) = answer.recv().await else {
                return;
            };
            let set = reply.ok().map(Arc::new);
            let _ = this.update(cx, |e, cx| e.set_variables(set, context, cx));
        }));
    }

    // Keep the set the URL reads its placeholders from and hand it to the params, headers,
    // auth and body; the vars tables are left out, as they define names rather than use them.
    // A set built in the same context from the same request inputs answers every lookup as the
    // current one does, so the current one stays and nothing that reads it redraws.
    fn set_variables(
        &mut self,
        set: Option<Arc<VariableSet>>,
        context: u64,
        cx: &mut Context<Self>,
    ) {
        if let (Some(old), Some(new)) = (&self.variables, &set)
            && self.variables_context == context
            && old.same_request_inputs(new)
        {
            return;
        }
        self.variables_context = context;
        for table in [&self.params, &self.headers] {
            table.update(cx, |t, cx| t.set_variables(set.clone(), cx));
        }
        self.auth
            .update(cx, |a, cx| a.set_variables(set.clone(), cx));
        self.body
            .update(cx, |b, cx| b.set_variables(set.clone(), cx));
        self.graphql
            .update(cx, |g, cx| g.set_variables(set.clone(), cx));
        self.variables = set;
        cx.notify();
    }

    // A save or a reload is a point undo stops at: the next keystroke in any field starts a
    // step of its own, in the structured history and in each text input's own undo.
    fn end_typing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history.end_typing();
        ui::end_input_run(&self.url, window, cx);
        self.auth.update(cx, |a, cx| a.end_typing(window, cx));
        self.script.update(cx, |s, cx| s.end_typing(window, cx));
        (self.tests_script).update(cx, |s, cx| s.end_typing(window, cx));
        self.settings.update(cx, |s, cx| s.end_typing(window, cx));
        let graphql = self.graphql.read(cx);
        let editors = [
            self.body.read(cx).code().clone(),
            graphql.query().clone(),
            graphql.variables().clone(),
            self.docs.read(cx).code().clone(),
        ];
        for editor in &editors {
            ui::end_editor_run(editor, window, cx);
        }
    }

    // Each field follows the snapshot only while it has nothing in flight, so the user's
    // own echo is never written back. Once the URL and params settle, both show the
    // re-derived value and any keystroke a locked side refused. Only differing text is set.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch_variables(cx);
        self.sync_locks(cx);
        let Some((root, item)) = self.target(cx) else {
            return;
        };
        let Some(view) = self.workspace.read(cx).request(&root, &item, cx) else {
            return;
        };
        self.clear_stale_error(&root, &item, &view, cx);
        self.note_loaded(&root, cx);
        let saved =
            |&(dirty, reloaded): &(bool, u64)| (dirty && !view.dirty) || reloaded != view.reloaded;
        if self.checkpoint.as_ref().is_some_and(saved) {
            self.end_typing(window, cx);
        }
        self.checkpoint = Some((view.dirty, view.reloaded));
        (self.graphql).update(cx, |s, cx| s.set_schema_url(view.url.clone(), cx));
        if !self.in_flight(cx) {
            ui::sync_input_text(&self.url, &view.url, window, cx);
            self.base = view.params.clone();
            let rows = param_rows(&self.base);
            self.params.update(cx, |t, cx| t.load(rows, window, cx));
        }
        if !self.busy(Field::Method, cx) && self.method(cx).as_deref() != Some(view.method.as_str())
        {
            let method = SharedString::from(view.method.clone());
            self.custom_error = None;
            self.method.update(cx, |s, cx| {
                s.set_items(method_rows(&view.method), window, cx);
                s.set_selected_value(&method, window, cx);
            });
        }
        if !self.busy(Field::Headers, cx) {
            self.header_base = view.headers.clone();
            let rows = header_rows(&self.header_base);
            self.headers.update(cx, |t, cx| t.load(rows, window, cx));
            let own = self.header_base.clone();
            self.default_headers.update(cx, |d, cx| d.set_own(&own, cx));
            self.inherited_headers
                .update(cx, |d, cx| d.set_own(&own, cx));
            self.inherited_headers
                .update(cx, |d, cx| d.load(&view.inherited_headers, cx));
        }
        if !self.busy(Field::OmitHeaders, cx) {
            self.default_headers
                .update(cx, |d, cx| d.load(&view.omitted_headers, cx));
        }
        if !self.busy(Field::Vars, cx) {
            self.var_base = view.vars.clone();
            let rows = var_rows(&self.var_base);
            self.vars.update(cx, |t, cx| t.load(rows, window, cx));
        }
        if !self.busy(Field::PostResponseVars, cx) {
            self.post_response_base = view.post_response_vars.clone();
            let rows = post_response_rows(&self.post_response_base);
            self.post_response
                .update(cx, |t, cx| t.load(rows, window, cx));
        }
        if !self.busy(Field::Body, cx) {
            match view.kind {
                RequestKind::Graphql => {
                    (self.graphql).update(cx, |s, cx| s.load(&view.body, window, cx))
                }
                _ => self.body.update(cx, |s, cx| s.load(&view.body, window, cx)),
            }
        }
        if let Some(snap) = self.workspace.read(cx).snapshot(&root, cx) {
            let names = snap.dotenv_names(&Target::Request(item.clone()));
            self.auth.update(cx, |a, cx| a.set_dotenv(names, cx));
        }
        if !self.busy(Field::Auth, cx) {
            let inherited = Some(view.inherited_auth.clone().into());
            self.auth.update(cx, |s, cx| s.set_inherited(inherited, cx));
            load_auth(&self.auth, &view.auth, window, cx);
        }
        if !self.busy(Field::Assertions, cx) {
            self.assertions
                .update(cx, |s, cx| s.load(&view.assertions, window, cx));
        }
        if !self.busy(Field::Settings, cx) {
            self.settings
                .update(cx, |s, cx| s.load(&view.settings, window, cx));
        }
        if !self.busy(Field::Tags, cx) {
            self.settings
                .update(cx, |s, cx| s.load_tags(&view.tags, cx));
        }
        if !self.busy(Field::Docs, cx) {
            self.docs.update(cx, |s, cx| s.load(&view.docs, window, cx));
        }
        // The Script section loads whole, so an edit in flight in either editor holds it back.
        if !self.busy(Field::ScriptPreRequest, cx) && !self.busy(Field::ScriptPostResponse, cx) {
            self.script
                .update(cx, |s, cx| s.load(&view.scripts, window, cx));
        }
        if !self.busy(Field::ScriptTests, cx) {
            self.tests_script
                .update(cx, |s, cx| s.load(&view.scripts, window, cx));
        }
    }
}

// The fields `refresh` loads one by one, each while it has no edit in flight.
const LOADED_ALONE: [Field; 13] = [
    Field::UrlParams,
    Field::Method,
    Field::Headers,
    Field::OmitHeaders,
    Field::Vars,
    Field::PostResponseVars,
    Field::Body,
    Field::Auth,
    Field::Assertions,
    Field::Settings,
    Field::Tags,
    Field::Docs,
    Field::ScriptTests,
];

// Up to three names in order, the rest counted: "z", "z and a", "z, a, b and 2 more".
fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] if names.len() <= 3 => format!("{} and {last}", first.join(", ")),
        _ => format!("{} and {} more", names[..3].join(", "), names.len() - 3),
    }
}

// The table section a field is edited in, and back.
fn table_section(field: Field) -> Option<Section> {
    match field {
        Field::UrlParams => Some(Section::Params),
        Field::Headers => Some(Section::Headers),
        Field::Vars => Some(Section::Vars),
        _ => None,
    }
}

// The typing key shared by every auth field, since the section does not say which one changed.
pub(crate) const AUTH_TYPING: (usize, u8) = (usize::MAX, 0);

// The view's value of the field `edit` writes, as an edit; `None` for an unrecorded kind.
fn view_edit(view: &RequestView, edit: &Edit) -> Option<Edit> {
    Some(match edit {
        Edit::Params(_) => Edit::Params(view.params.clone()),
        Edit::Headers(_) => Edit::Headers(view.headers.clone()),
        Edit::Vars(_) => Edit::Vars(view.vars.clone()),
        Edit::PostResponseVars(_) => Edit::PostResponseVars(view.post_response_vars.clone()),
        Edit::Assertions(_) => Edit::Assertions(view.assertions.clone()),
        Edit::Body(_) => Edit::Body(view.body.clone()),
        Edit::Method(_) => Edit::Method(view.method.clone()),
        Edit::Auth(_) => Edit::Auth(view.auth.clone()),
        Edit::Script(phase, _) => Edit::Script(*phase, view.scripts.get(*phase).to_string()),
        _ => return None,
    })
}

fn view_holds(view: &RequestView, edit: &Edit) -> bool {
    view_edit(view, edit).is_some_and(|held| same_value(&held, edit))
}

// Whether `edit` is text typing in a body that already had that kind: those editors keep
// their own undo, while a mode switch and form row edits are steps.
fn body_typing(before: &Edit, edit: &Edit) -> bool {
    let (Edit::Body(old), Edit::Body(new)) = (before, edit) else {
        return false;
    };
    let form = |b: &BodySpec| matches!(b, BodySpec::FormUrlEncoded(_) | BodySpec::Multipart(_));
    std::mem::discriminant(old) == std::mem::discriminant(new) && !form(new)
}

/// Each parent folder as (segment, folder path), then the request's display name.
pub fn crumbs(item: &ItemPath, name: &str) -> (Vec<(String, ItemPath)>, String) {
    let mut folders = Vec::new();
    let mut next = item.parent();
    while let Some(folder) = next.filter(|p| !p.is_root()) {
        next = folder.parent();
        folders.push((folder.file_name().to_owned(), folder));
    }
    folders.reverse();
    (folders, name.to_owned())
}

/// The least a shrunk breadcrumb segment keeps: its padding plus room for an ellipsis.
const CRUMB_SHRUNK_MIN: Rems = rems(1.5);

/// A clickable breadcrumb segment, padded to a 20 px hit target while the text stays
/// link-sized. The button's label ellipsizes when the segment is made narrower than its text.
fn crumb_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .link()
        .xsmall()
        .px(style::SPACE_1)
        .py(style::SPACE_0)
        .min_h(style::CRUMB_MIN_HEIGHT)
        .label(label)
        .on_click(on_click)
}

/// The full path from the collection root: the file path, or a draft's folder path then its
/// name. `None` for a draft at the root, whose path is only the name the line already shows.
pub fn crumb_tooltip(item: &ItemPath, name: &str) -> Option<String> {
    if !item.is_draft() {
        return Some(item.as_str().to_owned());
    }
    item.parent()
        .filter(|p| !p.is_root())
        .map(|folder| format!("{}/{name}", folder.as_str()))
}

/// Shows `spec` in `auth`, reading the token status when the shown mode turns OAuth 2.0.
pub(crate) fn load_auth(
    auth: &Entity<AuthSection>,
    spec: &AuthSpec,
    window: &mut Window,
    cx: &mut App,
) {
    let before = auth.read(cx).mode();
    auth.update(cx, |s, cx| s.load(spec, window, cx));
    let oauth2 = Some(AuthMode::OAuth2);
    if before != oauth2 && auth.read(cx).mode() == oauth2 {
        refresh_token_panel(auth, cx);
    }
}

impl RequestEditor {
    // The snapshot rebuilds every view on a reload or tree op, so the error clears only when
    // what this request sends, the selected environment, or what it inherits (its folders, the
    // collection, the globals, `.env`; coarse by design) differs from the last refresh.
    fn clear_stale_error(
        &mut self,
        root: &Path,
        item: &ItemPath,
        view: &RequestView,
        cx: &mut Context<Self>,
    ) {
        let ws = self.workspace.read(cx);
        let env = ws.selected_environment(root, cx);
        let inherited = (ws.snapshot(root, cx)).map(|snap| Inherited::of(&snap, item, ws));
        let sent = SentShape::of(view);
        let edited = self.seen_view.as_ref().is_some_and(|seen| *seen != sent);
        let switched = self.seen_env.as_ref().is_some_and(|seen| *seen != env);
        let moved =
            (self.seen_inherited.as_ref()).is_some_and(|seen| Some(seen) != inherited.as_ref());
        if edited || switched || moved {
            self.send_model.update(cx, |m, cx| m.clear_error(cx));
        }
        self.seen_view = Some(sent);
        self.seen_env = Some(env);
        self.seen_inherited = inherited;
    }
}

/// What a request inherits: its folders, the collection, and the selected collection and
/// global environments with what they extend, compared by value so a rebuild that changes none
/// of it keeps a shown error. Environments are held by `Arc`, so a notify copies pointers, not
/// variable lists; values are compared only when a pointer differs.
struct Inherited {
    configs: Vec<Arc<ConfigView>>,
    locals: Vec<(String, Option<Arc<EnvView>>)>,
    globals: Option<Arc<GlobalEnvironments>>,
    selected_global: Option<String>,
    dotenv_seq: u64,
}

impl PartialEq for Inherited {
    // A config view reused by the snapshot is unchanged, so only rebuilt ones are compared.
    fn eq(&self, other: &Inherited) -> bool {
        let same = |a: &Arc<ConfigView>, b: &Arc<ConfigView>| {
            Arc::ptr_eq(a, b) || ConfigShape::of(a) == ConfigShape::of(b)
        };
        let same_env = |a: &Option<Arc<EnvView>>, b: &Option<Arc<EnvView>>| match (a, b) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || env_shape(a) == env_shape(b),
            (None, None) => true,
            _ => false,
        };
        let same_globals = || match (&self.globals, &other.globals) {
            (Some(a), Some(b)) if Arc::ptr_eq(a, b) => true,
            (None, None) => true,
            _ => self.global_shapes() == other.global_shapes(),
        };
        self.dotenv_seq == other.dotenv_seq
            && self.selected_global == other.selected_global
            && self.locals.len() == other.locals.len()
            && (self.locals.iter().zip(&other.locals))
                .all(|(a, b)| a.0 == b.0 && same_env(&a.1, &b.1))
            && self.configs.len() == other.configs.len()
            && (self.configs.iter().zip(&other.configs)).all(|(a, b)| same(a, b))
            && same_globals()
    }
}

impl Inherited {
    fn of(snap: &CollectionSnapshot, item: &ItemPath, ws: &Workspace) -> Inherited {
        let effective = ws.effective_global();
        let folders = std::iter::successors(item.parent(), ItemPath::parent)
            .filter_map(|p| snap.content.folders.get(&p));
        Inherited {
            configs: folders.chain([&snap.content.collection]).cloned().collect(),
            locals: local_chain(
                &snap.content.environments,
                snap.selected_environment.clone(),
            ),
            globals: effective.as_ref().map(|(g, _)| Arc::clone(g)),
            selected_global: effective.map(|(_, n)| n),
            dotenv_seq: snap.dotenv_seq,
        }
    }

    // The selected global environment's chain as values; built only when the globals changed.
    fn global_shapes(&self) -> Vec<EnvShape> {
        let find = |name: &str| {
            let env = self.globals.as_ref()?.environment(name)?;
            Some((shape_of(&env.variables()), env.extends()))
        };
        env_chain(self.selected_global.clone(), find)
    }
}

/// One environment's name and variables (name, value, enabled, secret).
#[derive(PartialEq)]
struct EnvShape(String, Vec<VarRow>);

type VarRow = (String, VarValue, bool, bool);

fn shape_of(vars: &[EnvVariable]) -> Vec<VarRow> {
    (vars.iter())
        .map(|v| (v.name.clone(), v.value.clone(), v.enabled, v.secret))
        .collect()
}

// What a local environment contributes to the chain: its variables and what it extends.
fn env_shape(env: &EnvView) -> (Vec<VarRow>, Option<&str>) {
    (shape_of(&env.variables), env.extends.as_deref())
}

// The environment `first` and each one it extends, as shared pointers into the snapshot,
// stopping at a missing name or a cycle; a missing name still counts, so creating it later
// reads as a change.
fn local_chain(
    envs: &[Arc<EnvView>],
    first: Option<String>,
) -> Vec<(String, Option<Arc<EnvView>>)> {
    let mut chain: Vec<(String, Option<Arc<EnvView>>)> = Vec::new();
    let mut next = first;
    while let Some(name) = next.take() {
        if chain.iter().any(|e| e.0 == name) {
            break;
        }
        let found = envs.iter().find(|e| e.name == name).cloned();
        next = found.as_ref().and_then(|e| e.extends.clone());
        chain.push((name, found));
    }
    chain
}

// The same walk over values, for the global environments.
fn env_chain(
    first: Option<String>,
    find: impl Fn(&str) -> Option<(Vec<VarRow>, Option<String>)>,
) -> Vec<EnvShape> {
    let mut chain: Vec<EnvShape> = Vec::new();
    let mut next = first;
    while let Some(name) = next.take() {
        if chain.iter().any(|e| e.0 == name) {
            break;
        }
        let found = find(&name);
        let vars = found.as_ref().map(|f| f.0.clone()).unwrap_or_default();
        next = found.and_then(|f| f.1);
        chain.push(EnvShape(name, vars));
    }
    chain
}

#[cfg(test)]
mod inherited_tests {
    use super::{Arc, EnvVariable, EnvView, VarValue, local_chain};
    use crate::snapshot::DiskState;

    fn env(name: &str, extends: Option<&str>) -> Arc<EnvView> {
        Arc::new(EnvView {
            name: name.into(),
            variables: vec![EnvVariable::new("k", VarValue::Text("v".into()))],
            color: None,
            extends: extends.map(Into::into),
            dirty: false,
            disk: DiskState::Present,
            reloaded: 0,
        })
    }

    // The chain shares the snapshot's environments, so a notify copies no variable list.
    #[test]
    fn the_local_chain_shares_the_snapshots_environments() {
        let envs = vec![env("a", Some("b")), env("b", None)];
        let first = local_chain(&envs, Some("a".into()));
        let second = local_chain(&envs, Some("a".into()));
        assert_eq!(first.len(), 2);
        for (chain, source) in [(&first, &envs), (&second, &envs)] {
            for (link, env) in chain.iter().zip(source) {
                assert!(Arc::ptr_eq(link.1.as_ref().unwrap(), env));
            }
        }
    }

    #[test]
    fn a_cycle_and_a_missing_name_end_the_chain() {
        let envs = vec![env("a", Some("b")), env("b", Some("a"))];
        assert_eq!(local_chain(&envs, Some("a".into())).len(), 2);
        let missing = local_chain(&envs, Some("zzz".into()));
        assert!(missing.len() == 1 && missing[0].1.is_none());
    }
}

/// The parts of a folder or collection config that shape a send.
#[derive(PartialEq)]
struct ConfigShape {
    headers: Vec<(String, String, bool)>,
    auth: AuthSpec,
    vars: Vec<(String, VarValue, bool)>,
    post_response_vars: Vec<(String, String, bool)>,
    scripts: ScriptCode,
    network: Option<CollectionNetwork>,
}

impl ConfigShape {
    fn of(c: &ConfigView) -> ConfigShape {
        ConfigShape {
            headers: (c.headers.iter())
                .map(|h| (h.name.clone(), h.value.clone(), h.enabled))
                .collect(),
            auth: c.auth.clone(),
            vars: (c.vars.iter())
                .map(|x| (x.name.clone(), x.value.clone(), x.enabled))
                .collect(),
            post_response_vars: (c.post_response_vars.iter())
                .map(|x| (x.name().to_owned(), x.expression().to_owned(), x.enabled()))
                .collect(),
            scripts: c.scripts.clone(),
            network: c.network.clone(),
        }
    }
}

/// The parts of a request view that shape what a send does, compared by value.
#[derive(PartialEq)]
struct SentShape {
    kind: RequestKind,
    method: String,
    url: String,
    params: Vec<(String, String, ParamKind, bool)>,
    headers: Vec<(String, String, bool)>,
    body: BodySpec,
    auth: AuthSpec,
    inherited_auth: String,
    inherited_headers: Vec<crate::inherited_headers_section::InheritedRow>,
    vars: Vec<(String, VarValue, bool)>,
    settings: RequestSettings,
    omitted_headers: Vec<String>,
    scripts: ScriptCode,
}

impl SentShape {
    fn of(v: &RequestView) -> SentShape {
        SentShape {
            kind: v.kind,
            method: v.method.clone(),
            url: v.url.clone(),
            params: (v.params.iter())
                .map(|p| (p.name.clone(), p.value.clone(), p.kind, p.enabled))
                .collect(),
            headers: (v.headers.iter())
                .map(|h| (h.name.clone(), h.value.clone(), h.enabled))
                .collect(),
            body: v.body.clone(),
            auth: v.auth.clone(),
            inherited_auth: v.inherited_auth.clone(),
            inherited_headers: v.inherited_headers.clone(),
            vars: (v.vars.iter())
                .map(|x| (x.name.clone(), x.value.clone(), x.enabled))
                .collect(),
            settings: v.settings,
            omitted_headers: v.omitted_headers.clone(),
            scripts: v.scripts.clone(),
        }
    }
}

/// Re-reads the token status of the OAuth 2.0 editor inside `auth`.
pub(crate) fn refresh_token_panel(auth: &Entity<AuthSection>, cx: &mut App) {
    let panel = auth.read(cx).oauth2().read(cx).token_panel().cloned();
    if let Some(panel) = panel {
        panel.update(cx, |p, cx| p.refresh(cx));
    }
}

fn target_of(workspace: &Entity<Workspace>, tab: TabId, cx: &App) -> Option<(PathBuf, ItemPath)> {
    let entry = workspace.read(cx).tabs().get(tab)?;
    Some((entry.root.clone(), entry.item()?.clone()))
}

impl Focusable for RequestEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RequestEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One inset on the root keeps the request and response panes flush with each other.
        let root = v_flex()
            .size_full()
            .gap_2()
            .px(style::PANE_INSET)
            .pt(style::PANE_INSET)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SaveRequest, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &UndoEdit, window, cx| this.undo_edit(window, cx)))
            .on_action(cx.listener(|this, _: &RedoEdit, window, cx| this.redo_edit(window, cx)))
            .on_action(cx.listener(Self::send));
        if self.target(cx).is_none() {
            return root;
        }
        self.follow_graphql_caret(window, cx);
        self.follow_snippet_caret(window, cx);
        // The section body scrolls under a fixed header; the column scrolls too, only once a
        // pane at its minimum is shorter than the header plus the body's minimum.
        // The URL keeps 16 rem; a pane narrower than that (even stacked) bounds it instead.
        let url_min = {
            let floor = style::resolve(style::URL_FIELD_MIN, window);
            match cx.try_global::<TabAreaRoom>() {
                Some(r) => {
                    floor.min((r.0 - style::resolve(style::URL_ROW_SLACK, window)).max(px(0.)))
                }
                None => floor,
            }
        };
        let breadcrumb = self.chrome_element(
            ChromePart::Breadcrumb,
            StyleRefinement::default().w_full().min_w_0().flex_none(),
            window,
            cx,
        );
        let method = self.chrome_element(
            ChromePart::Method,
            StyleRefinement::default()
                .w(style::METHOD_SELECT)
                .flex_none(),
            window,
            cx,
        );
        let custom_method = self.chrome_element(
            ChromePart::CustomMethod,
            StyleRefinement::default()
                .w(style::CUSTOM_METHOD)
                .flex_none(),
            window,
            cx,
        );
        let url = self.chrome_element(
            ChromePart::Url,
            StyleRefinement::default().flex_1().min_w(url_min),
            window,
            cx,
        );
        let buttons = self.chrome_element(
            ChromePart::Buttons,
            StyleRefinement::default().flex_none(),
            window,
            cx,
        );
        let sections = self.chrome_element(
            ChromePart::Sections,
            StyleRefinement::default().flex_none(),
            window,
            cx,
        );
        // Last frame's rows decide this frame's stacked minimum; a watch asks for one more
        // frame when this frame's rows measure differently.
        let rows_used = Self::rows_need(&self.column_scroll);
        let request = v_flex()
            .id("request-column")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.column_scroll)
            // The pane and this column clip flush on the left and top, where the breadcrumb,
            // the method select and the section tabs would lose their rings.
            .ring_room(RingSides::TOP | RingSides::LEFT)
            .gap_2()
            // Rows keep their height in a short pane; the column scrolls instead of the rows
            // shrinking under their content and painting over each other.
            .child(breadcrumb)
            .children(
                self.banner_rows(cx)
                    .into_iter()
                    .chain(self.paste_banner(cx))
                    .map(|b| div().flex_none().child(b)),
            )
            .child(
                // Only the URL gives way on a narrow editor, and once it would drop below a
                // usable width, the buttons wrap to their own line; nothing else shrinks.
                // The right inset leaves room for the last button's keyboard ring, which the
                // pane would otherwise clip; the ring does not shrink with zoom, so the inset has a floor.
                h_flex()
                    .flex_none()
                    .flex_wrap()
                    .justify_end()
                    .gap(style::SPACE_2)
                    .w_full()
                    .pr(style::SPACE_1
                        .to_pixels(window.rem_size())
                        .max(style::FOCUS_RING_ROOM))
                    .child(method)
                    .child(custom_method)
                    .child(url)
                    // Save, Send and Generate code wrap together, so the group never splits.
                    .child(buttons),
            )
            .children(self.custom_error.clone().map(|message| {
                div()
                    .id("custom-method-error")
                    .test_support()
                    .flex_none()
                    .text_size(style::TEXT_SMALL)
                    .text_color(cx.theme().danger)
                    .child(message)
            }))
            .child(sections)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(style::SECTION_BODY_MIN)
                    .child(
                        // The body grows into the column's ring room, so its first controls
                        // keep their rings without moving.
                        v_flex()
                            .id("section-body")
                            .test_support()
                            .track_focus(&self.section_body_focus)
                            .absolute()
                            .inset_0()
                            .ring_outset(RingSides::TOP | RingSides::LEFT)
                            .pr(style::SCROLLBAR_GUTTER)
                            .overflow_y_scroll()
                            .track_scroll(&self.body_scroll)
                            .child(self.section_body(cx))
                            .child(self.follow.watch(&self.body_scroll)),
                    )
                    .children(ui::scroll_bar_y(
                        "section-body-scrollbar",
                        &self.body_scroll,
                    )),
            )
            .child(Self::rows_watch(&self.column_scroll, rows_used));
        // The cue sits over the column's bottom edge, outside the scrolling box.
        let request = div()
            .relative()
            .size_full()
            .child(request)
            .children(ui::scroll_cue(
                "request-column-fade",
                &self.column_scroll,
                cx,
            ));
        // Side by side the response needs its header's measured width plus its gutter; it is
        // intrinsic, so a drag never changes it and the axis flips only on a resize or zoom.
        let response_need = self
            .response
            .read(cx)
            .min_width()
            .unwrap_or_else(|| style::resolve(style::RESPONSE_STRIP_WIDTH, window))
            + style::resolve(style::SPACE_2, window);
        // A new orientation gets a fresh split, sized again from the workspace's ratio.
        let axis = Self::axis_in(window, cx, response_need);
        if axis != self.split_axis {
            self.split = cx.new(|_| ResizableState::default());
            self._split_sub = Self::watch_split(&self.split, cx);
            self.applied_ratio = None;
            self.split_axis = axis;
        }
        let group = match axis {
            Orientation::Vertical => v_resizable("request-split"),
            Orientation::Horizontal => h_resizable("request-split"),
        };
        // A short zoomed room caps each minimum at a quarter of it, so the divider still moves.
        let room = self.split.read(cx).container_size();
        let min = if room > Pixels::ZERO {
            style::PANE_MIN.min(room * 0.25)
        } else {
            style::PANE_MIN
        };
        if axis == Orientation::Vertical {
            self.response_room.set(room);
        }
        let ratio = self.workspace.read(cx).split_ratio();
        // An untouched split favours the request on a short room, up to the height its
        // editor rows need, while the response keeps RESPONSE_FLOOR; where that floor does
        // not fit, or the ratio was dragged, the ratio rules.
        let ratio = if axis == Orientation::Vertical
            && room > Pixels::ZERO
            && (ratio - style::DEFAULT_SPLIT_RATIO).abs() < f32::EPSILON
            && room - style::resolve(style::REQUEST_PANE_NEEDED, window) >= style::RESPONSE_FLOOR
        {
            ratio.max(style::resolve(style::REQUEST_PANE_NEEDED, window) / room)
        } else {
            ratio
        };
        // Side by side, the request panel never gets narrower than its URL row; stacked, never
        // shorter than its rows above the section body plus one editor row, giving way on a
        // short room so the response keeps RESPONSE_KEPT. The clamped share is not written back.
        let request_min = match axis {
            _ if room <= Pixels::ZERO => min,
            Orientation::Horizontal => min.max(Self::url_row_need(window)),
            Orientation::Vertical => match rows_used {
                Some(rows) => {
                    let need = rows
                        + style::resolve(style::ROW_HEIGHT, window)
                        + style::resolve(style::SPACE_2, window);
                    need.min(room - style::RESPONSE_KEPT).max(min)
                }
                None => min,
            },
        };
        // The response keeps its header whole beside the request, giving way to the request's
        // URL row for the frame before the axis stacks.
        let response_min = match axis {
            Orientation::Horizontal if room > Pixels::ZERO => {
                response_need.min(room - request_min).max(min)
            }
            _ => min,
        };
        let ratio = if room > Pixels::ZERO {
            ratio
                .max(request_min / room)
                .min((room - response_min) / room)
        } else {
            ratio
        };
        let panel = || resizable_panel().size_range(response_min..Pixels::MAX);
        // Each pane keeps a gutter on the divider side; the ids stay on the inner boxes.
        // Each pane clips, so a toolbar wider than a zoomed panel cannot paint over the other.
        let pane = || div().size_full().min_w_0().min_h_0().overflow_hidden();
        let horizontal = axis == Orientation::Horizontal;
        let (request_pane, response_pane) = if horizontal {
            (pane().pr(style::SPACE_2), pane().pl(style::SPACE_2))
        } else {
            (pane().pb(style::SPACE_2), pane().pt(style::SPACE_2))
        };
        // Until the tab first sends or restores a result, the response is a strip beside a
        // full request column, with no resizable group.
        if !self.send_model.read(cx).ever_shown() {
            self.applied_ratio = None;
            // Side by side the strip takes the request column's top ring room, so both panes
            // start on one line.
            let strip = div()
                .id("response")
                .test_support()
                .when(horizontal, |d| d.ring_room(RingSides::TOP))
                .when(horizontal, |d| d.h_full())
                .child(response_strip(horizontal, window, cx));
            let (column, strip_pane) = match horizontal {
                true => (h_flex(), response_pane.w_auto()),
                false => (v_flex(), response_pane.h_auto()),
            };
            return root.child(
                column
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(request_pane.flex_1().child(request))
                    .child(strip_pane.flex_none().child(strip)),
            );
        }
        // A panel size is read only while the split holds none, so a new target ratio (or a
        // fresh expand) clears the split once; a drag in flight keeps its target and its sizes.
        if self.applied_ratio != Some(ratio) {
            self.split.update(cx, |s, _| s.clear());
            self.applied_ratio = Some(ratio);
        }
        // A relative basis, since the last laid-out room is stale for a tab hidden during a resize.
        let request_panel = resizable_panel()
            .size_range(request_min..Pixels::MAX)
            .flex_basis(relative(ratio));
        let response = div()
            .id("response")
            .test_support()
            .size_full()
            .ring_room(RingSides::TOP)
            .child(ui::cached(&self.response));
        root.child(
            div().flex_1().min_h_0().min_w_0().child(
                group
                    .with_state(&self.split)
                    .child(request_panel.child(request_pane.child(request)))
                    .child(
                        panel()
                            .flex_basis(relative(1. - ratio))
                            .child(response_pane.child(response)),
                    ),
            ),
        )
    }
}

// The chrome parts: each is drawn by its own `Chrome` view, which calls back in here.
impl RequestEditor {
    /// The view that draws `part` of this editor's chrome.
    pub fn chrome(&self, part: ChromePart) -> &Entity<Chrome> {
        self.chrome[part as usize].view()
    }

    fn chrome_element(
        &mut self,
        part: ChromePart,
        outer: StyleRefinement,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let inputs = self.chrome_inputs(part, cx);
        self.chrome[part as usize].element(inputs, outer, window, cx)
    }

    fn chrome_inputs(&self, part: ChromePart, cx: &App) -> PartInputs {
        match part {
            ChromePart::CustomMethod => PartInputs::CustomMethod(self.custom_error.is_some()),
            ChromePart::Method => PartInputs::Method(self.method(cx)),
            ChromePart::Breadcrumb => PartInputs::Breadcrumb(self.crumbs_shown(cx)),
            ChromePart::Url => PartInputs::Url(
                self.locked(Side::Url, cx),
                self.variables.as_ref().map(|v| (Same::of(v), v.revision())),
            ),
            ChromePart::Buttons => PartInputs::Buttons(self.save_shown(cx), self.slot_now(cx)),
            ChromePart::Sections => PartInputs::Sections(
                self.kind(cx),
                self.section,
                Section::ALL
                    .iter()
                    .map(|&s| (self.section_count(s, cx), self.section_marked(s, cx)))
                    .collect(),
            ),
        }
    }

    /// Draws `part` for its `Chrome` view.
    pub(crate) fn render_chrome(
        &mut self,
        part: ChromePart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match part {
            ChromePart::Breadcrumb => match self.crumbs_shown(cx) {
                Some(crumbs) => self.breadcrumb(crumbs, cx),
                None => Empty.into_any_element(),
            },
            ChromePart::Method => div()
                .w_full()
                .child(
                    Select::new(&self.method)
                        .id("method")
                        .font_family(cx.theme().mono_font_family.clone()),
                )
                .into_any_element(),
            ChromePart::CustomMethod => div()
                .id("custom-method-box")
                .w_full()
                .tooltip(|window, cx| Tooltip::new("Custom method").build(window, cx))
                .child(ui::invalid_input(
                    Input::new(&self.custom_method)
                        .id("custom-method")
                        .bg(ui::field_fill(false, cx))
                        .font_family(cx.theme().mono_font_family.clone()),
                    div().w_full(),
                    &self.custom_method,
                    self.custom_error.is_some(),
                    (
                        "custom-method-danger-ring".into(),
                        "custom-method-invalid-icon".into(),
                    ),
                    window,
                    cx,
                ))
                .into_any_element(),
            ChromePart::Url => self.url_field(window, cx),
            ChromePart::Buttons => h_flex()
                .flex_none()
                .gap(style::SPACE_1)
                .children(self.save_button(cx))
                .child(self.send_button(window, cx))
                .child(self.generate_code_button(cx))
                .into_any_element(),
            ChromePart::Sections => self.section_bar(window, cx).into_any_element(),
        }
    }

    // Opens the settings tab of the level that supplies an inherited header.
    fn open_header_source(&self, source: &HeaderSource, cx: &mut Context<Self>) {
        let Some((root, _)) = self.target(cx) else {
            return;
        };
        let kind = match source {
            HeaderSource::Folder(path) => TabKind::Folder(path.clone()),
            _ => TabKind::Collection,
        };
        self.workspace
            .update(cx, |w, cx| w.open_settings(&root, kind, cx));
    }

    // The breadcrumb's contents: the request's folders by display name, its name and path.
    fn crumbs_shown(&self, cx: &App) -> Option<Crumbs> {
        let (root, item) = self.target(cx)?;
        let workspace = self.workspace.read(cx);
        let name = workspace
            .request(&root, &item, cx)
            .map(|v| v.name.clone())
            .unwrap_or_default();
        let (mut folders, title) = crumbs(&item, &name);
        // A folder shows its display name, as the sidebar and its settings tab do.
        if let Some(snapshot) = workspace.snapshot(&root, cx) {
            for (segment, path) in &mut folders {
                if let Some(display) = snapshot
                    .content
                    .folders
                    .get(path)
                    .and_then(|v| v.name.clone())
                {
                    *segment = display;
                }
            }
        }
        Some(Crumbs {
            collection: workspace.collection_name(&root, cx),
            tooltip: crumb_tooltip(&item, &name),
            root,
            item,
            folders,
            title,
        })
    }

    // The collection and each folder are links that open their settings tab (a folder also
    // reveals itself in the sidebar); the plain name, in the foreground colour, ends the
    // line, and hovering the line shows the full path.
    fn breadcrumb(&self, crumbs: Crumbs, cx: &mut Context<Self>) -> AnyElement {
        let Crumbs {
            root: root_dir,
            collection: collection_name,
            folders,
            title,
            tooltip,
            ..
        } = crumbs;
        let tooltip: Option<SharedString> = tooltip.map(Into::into);
        let muted = cx.theme().muted_foreground;
        let collection_crumb = {
            let workspace = self.workspace.downgrade();
            let root = root_dir.clone();
            crumb_button(
                "breadcrumb-collection",
                collection_name,
                move |_, window, cx| {
                    let Some(workspace) = workspace.upgrade() else {
                        return;
                    };
                    if crate::scratch::refuse(&workspace, &root, window, cx) {
                        return;
                    }
                    workspace.update(cx, |w, cx| {
                        w.open_settings(&root, TabKind::Collection, cx);
                    });
                },
            )
            .flex_shrink(1.)
            .min_w(CRUMB_SHRUNK_MIN)
        };
        // The line never outgrows the pane: the collection and outer folders give way first
        // (clipped), while the nearest folder, the separators and the request name stay whole.
        let nearest = folders.len().saturating_sub(1);
        // The line clips, so it grows into the column's ring room to keep the crumbs' rings;
        // it stretches rather than taking a width, so the outset keeps its right edge.
        h_flex()
            .id("breadcrumb")
            .test_support()
            .min_w_0()
            .ring_outset(RingSides::TOP | RingSides::BOTTOM | RingSides::LEFT)
            .overflow_hidden()
            .font_family(cx.theme().font_family.clone())
            .text_size(style::TEXT_SMALL)
            .when_some(tooltip, |row, tooltip| {
                row.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .child(collection_crumb)
            .child(div().flex_none().text_color(muted).child(" / "))
            .children(
                folders
                    .into_iter()
                    .enumerate()
                    .flat_map(|(ix, (segment, folder))| {
                        let workspace = self.workspace.downgrade();
                        let root = root_dir.clone();
                        let button =
                            crumb_button(("breadcrumb-folder", ix), segment, move |_, _, cx| {
                                let folder = folder.clone();
                                // Open the settings tab, then reveal so the sidebar follows.
                                _ = workspace.update(cx, |w, cx| {
                                    w.open_settings(&root, TabKind::Folder(folder.clone()), cx);
                                    w.reveal_folder(&root, folder, cx);
                                });
                            });
                        let button = if ix == nearest {
                            button.flex_none()
                        } else {
                            button.flex_shrink(1.).min_w(CRUMB_SHRUNK_MIN)
                        };
                        [
                            button.into_any_element(),
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(" / ")
                                .into_any_element(),
                        ]
                    }),
            )
            .child(
                div()
                    .id("breadcrumb-file")
                    .test_support()
                    .flex_none()
                    .text_color(cx.theme().foreground)
                    .child(title),
            )
            .into_any_element()
    }

    fn url_field(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // The hook runs at dispatch time, so the editor is free to update.
        let url_locked = self.locked(Side::Url, cx);
        let this = cx.entity().downgrade();
        let on_paste = move |item: &ClipboardItem, window: &mut Window, cx: &mut App| {
            let Some(text) = item.text() else {
                return false;
            };
            this.update(cx, |e, cx| e.paste_url(&text, window, cx))
                .unwrap_or(false)
        };
        VarField::new(
            "url-vars",
            &self.url,
            ui::padded_input(
                ui::lock_input(Input::new(&self.url), &self.url, url_locked, window, cx)
                    .id("url")
                    .on_paste(on_paste)
                    .bg(ui::field_fill(url_locked, cx))
                    .font_family(cx.theme().mono_font_family.clone())
                    .w_full(),
                &self.url,
            ),
            self.variables.clone().filter(|_| !url_locked),
        )
        .w_full()
        .into_any_element()
    }
}

/// The width the tab area has: the window less the shown sidebar. The shell sets it each frame,
/// ahead of the editors it renders.
pub struct TabAreaRoom(pub Pixels);

impl Global for TabAreaRoom {}

impl RequestEditor {
    /// The response's place beside the editor: the app's setting, or vertical when no
    /// `AppState` exists.
    pub fn orientation(&self, cx: &App) -> Orientation {
        Self::orientation_in(cx)
    }

    /// The state of the request and response split for the current orientation.
    pub fn split(&self) -> &Entity<ResizableState> {
        &self.split
    }

    // Only a drag (or `resize_panel`) emits `Resized`, so only it writes the shared ratio.
    fn watch_split(split: &Entity<ResizableState>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(split, |this, split, _: &ResizablePanelEvent, cx| {
            let [request, response] = split.read(cx).sizes().as_slice() else {
                return;
            };
            let total = f32::from(*request + *response);
            if total <= 0. {
                return;
            }
            let share = f32::from(*request) / total;
            let stored = this.workspace.update(cx, |w, cx| {
                w.set_split_ratio(share, cx);
                w.split_ratio()
            });
            this.applied_ratio = Some(stored);
            cx.notify();
        })
    }

    /// The axis the split shows: the saved orientation, except that side by side gives way to
    /// stacked while the room cannot hold both the URL row at its 16 rem floor and the
    /// response's `response_need`. The saved setting is untouched, so more room brings the
    /// side-by-side layout back.
    fn axis_in(window: &Window, cx: &App, response_need: Pixels) -> Orientation {
        let saved = Self::orientation_in(cx);
        let Some(room) = cx.try_global::<TabAreaRoom>().map(|r| r.0) else {
            return saved;
        };
        let need = Self::url_row_need(window);
        // The request pane keeps what the response's minimum leaves of the split (the tab
        // area less the editor's insets), not a share a drag changes, so no flip under the divider.
        let pane = room - style::resolve(style::PANE_INSET, window) * 2. - response_need;
        if saved == Orientation::Horizontal && pane < need {
            Orientation::Vertical
        } else {
            saved
        }
    }

    /// The height of the request column's rows above the section body, as last laid out, or
    /// `None` before the column's first layout.
    fn rows_need(column: &ScrollHandle) -> Option<Pixels> {
        let first = column.bounds_for_item(0)?;
        // The section body is the last flowing row; the watch after it is absolute.
        let count = column.children_count();
        let body = column.bounds_for_item(count.checked_sub(2)?)?;
        Some(body.top() - first.top())
    }

    /// A zero-size marker that asks for another frame when the column's rows, laid out this
    /// frame, differ from the `used` height the split's minimum was computed from.
    fn rows_watch(column: &ScrollHandle, used: Option<Pixels>) -> impl IntoElement {
        let column = column.clone();
        canvas(
            move |_, window, _| {
                if Self::rows_need(&column) != used {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// The request pane width that holds the URL row on one line: the URL floor, the method
    /// select, the custom method input (always laid out) and its gap, the button group with
    /// Save shown, and the other gaps.
    fn url_row_need(window: &Window) -> Pixels {
        style::resolve(style::URL_FIELD_MIN, window)
            + style::resolve(style::METHOD_SELECT, window)
            + style::resolve(style::CUSTOM_METHOD, window)
            + style::resolve(style::SPACE_2, window)
            + style::resolve(style::REQUEST_BUTTONS_WIDTH, window)
            + style::resolve(style::URL_ROW_SLACK, window)
    }

    fn orientation_in(cx: &App) -> Orientation {
        cx.try_global::<AppState>()
            .map(|s| s.settings.read(cx).get().response_orientation)
            .unwrap_or_default()
    }

    // Save can show only for edits autosave will not write: autosave never writes a draft,
    // so a draft keeps Save even with autosave on.
    fn save_possible(&self, cx: &App) -> bool {
        let autosave = cx
            .try_global::<AppState>()
            .is_some_and(|s| s.settings.read(cx).get().autosave);
        let draft = self.target(cx).is_some_and(|(_, item)| item.is_draft());
        !autosave || draft
    }

    fn save_shown(&self, cx: &App) -> bool {
        self.save_possible(cx) && self.view(cx).is_some_and(|v| v.dirty)
    }

    // A clean request keeps Save's slot, invisible, inert and unnamed to screen readers, so
    // the URL field never changes width when an edit makes Save appear. Where Save can never
    // show, there is no slot and no gap.
    fn save_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.save_possible(cx) {
            return None;
        }
        if !self.save_shown(cx) {
            let ghost = Button::new("save-slot-button")
                .ghost()
                .small()
                .label("Save")
                .role(None::<Role>)
                .tab_stop(false);
            return div()
                .id("save-slot")
                .test_support()
                .flex_none()
                .mr(style::SPACE_1)
                .invisible()
                .child(ghost)
                .into_any_element()
                .into();
        }
        // Called directly: a dispatched action reaches the editor only while it holds focus.
        let button = Button::new("save-request-button")
            .ghost()
            .small()
            .label("Save")
            .tooltip_with_action(
                crate::commands::label("request.save"),
                &SaveRequest,
                Some(KEY_CONTEXT),
            )
            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)));
        // The group's gap is SPACE_1; Save keeps the SPACE_2 it had apart from Send.
        let slot = div()
            .id("save-request")
            .test_support()
            .flex_none()
            .mr(style::SPACE_1);
        Some(slot.child(button).into_any_element())
    }

    // Opens Generate code on this editor's own request, which need not be the active tab.
    fn generate_code_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let button = Button::new("generate-code-button")
            .ghost()
            .compact()
            .icon(IconName::CodeXml)
            .accessibility_label(crate::commands::label("request.generate_code"))
            .tooltip_with_action(
                crate::commands::label("request.generate_code"),
                &GenerateCode,
                Some(SHELL_CONTEXT),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some((root, item)) = this.target(cx) {
                    let workspace = this.workspace.clone();
                    crate::generate_code::open_generate_code(&workspace, &root, item, window, cx);
                }
            }));
        let slot = div().id("generate-code").test_support().flex_none();
        slot.child(button).into_any_element()
    }

    // Send while idle, Cancel while a send is planning or in flight.
    fn send_button(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let model = self.send_model.clone();
        self.carry_focus(window, cx);
        // A fixed-width slot, so swapping Send for the spinner and Cancel never resizes the URL.
        let slot = h_flex()
            .id("send-slot")
            .test_support()
            .track_focus(&self.slot_focus)
            .flex_none()
            .w(style::SEND_SLOT_WIDTH)
            .justify_end();
        if self.send_model.read(cx).is_streaming() {
            // Same slot, same layout: the response is open, so the button stops it and keeps
            // what arrived.
            slot.child(
                // The spinner is drawn over the button's left padding, so the button itself
                // keeps Send's width and left edge. It never takes clicks.
                div()
                    .id("stop")
                    .test_support()
                    .relative()
                    .w_full()
                    .child(
                        Button::new("stop-button")
                            .w_full()
                            .label("Stop")
                            .on_click(move |_, _, cx| model.update(cx, |m, cx| m.stop(cx))),
                    )
                    .child(
                        h_flex()
                            .id("stop-spinner")
                            .test_support()
                            .absolute()
                            .left(style::SPACE_2)
                            .top_0()
                            .bottom_0()
                            .items_center()
                            .child(Spinner::new().small()),
                    ),
            )
            .into_any_element()
        } else if self.send_model.read(cx).is_sending() {
            // A plain button with the spinner overlaid, not `Button::loading`, so it stays
            // pressable and keeps Send's width.
            slot.child(
                div()
                    .id("cancel")
                    .test_support()
                    .relative()
                    .w_full()
                    .child(
                        Button::new("cancel-button")
                            .w_full()
                            .label("Cancel")
                            .on_click(move |_, _, cx| model.update(cx, |m, cx| m.cancel(cx))),
                    )
                    .child(
                        h_flex()
                            .id("cancel-spinner")
                            .test_support()
                            .absolute()
                            .left(style::SPACE_2)
                            .top_0()
                            .bottom_0()
                            .items_center()
                            .child(Spinner::new().small()),
                    ),
            )
            .into_any_element()
        } else {
            slot.child(
                div().id("send").test_support().w_full().child(
                    Button::new("send-button")
                        .primary()
                        .w_full()
                        .label("Send")
                        .tooltip_with_action("Send", &SendRequest, Some(KEY_CONTEXT))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.send(&SendRequest, window, cx)),
                        ),
                ),
            )
            .into_any_element()
        }
    }

    // When the button in the slot changes while it holds the focus, the new button takes the
    // focus: once this frame is drawn, focus the slot and step to its first tab stop, which is the
    // button. Otherwise a keyboard user loses their place.
    fn carry_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.slot_now(cx);
        if self.slot_kind.replace(kind) != kind && self.slot_focus.contains_focused(window, cx) {
            let slot = self.slot_focus.clone();
            window.defer(cx, move |window, cx| {
                window.focus(&slot, cx);
                window.focus_next(cx);
            });
        }
    }

    // Which button the send slot holds: 0 Send, 1 Cancel, 2 Stop.
    fn slot_now(&self, cx: &App) -> u8 {
        let send = self.send_model.read(cx);
        if send.is_streaming() {
            2
        } else {
            u8::from(send.is_sending())
        }
    }

    // The banners of the current snapshot: the disk state of the request and the edits a
    // reload held back from it. Nothing is kept here, so an answered banner just goes away.
    fn conflicts(&self, cx: &App) -> Option<(PathBuf, Target, Vec<Banner>, Vec<Edit>)> {
        let (root, item) = self.target(cx)?;
        let workspace = self.workspace.read(cx);
        let view = workspace.request(&root, &item, cx)?;
        let target = Target::Request(item);
        let held = workspace
            .snapshot(&root, cx)
            .and_then(|s| s.stale.get(&target).cloned())
            .unwrap_or_default();
        let banners = banner::banners(view.disk, held.len());
        Some((root, target, banners, held))
    }

    /// The open Compare dialog, if one was opened from the banner.
    pub fn compare(&self) -> Option<Entity<crate::compare_dialog::CompareDialog>> {
        self.compare.clone()
    }

    // Opens the read-only diff of disk against draft; the banner's own answer runs from it.
    fn open_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((root, target, shown, _)) = self.conflicts(cx) else {
            return;
        };
        if !shown.contains(&Banner::ChangedOnDisk) {
            return;
        }
        let this = cx.entity().downgrade();
        let answer: crate::compare_dialog::Answer = std::rc::Rc::new(move |choice, _, cx| {
            let _ = this.update(cx, |e, cx| e.answer(Banner::ChangedOnDisk, choice, cx));
        });
        let workspace = self.workspace.clone();
        self.compare = Some(crate::compare_dialog::open(
            workspace, root, target, answer, window, cx,
        ));
    }

    /// Answers the shown `banner` with `choice`, reading the held edits afresh.
    pub fn answer(&mut self, banner: Banner, choice: Choice, cx: &mut Context<Self>) {
        let Some((root, target, shown, held)) = self.conflicts(cx) else {
            return;
        };
        // A click from a frame drawn before the banner went away answers nothing.
        let kind = std::mem::discriminant(&banner);
        if !shown.iter().any(|b| std::mem::discriminant(b) == kind) {
            return;
        }
        let commands = banner::commands(banner, choice, &target, &held);
        // Save and Overwrite write what this editor holds, so once the write is confirmed they
        // vouch for the typing as a save does; the text is captured now.
        let texts = self.typed_scripts(cx);
        self.workspace.update(cx, |w, cx| {
            for command in commands {
                let wrote = match &command {
                    Command::Save(t) | Command::Overwrite(t) => Some(t.clone()),
                    _ => None,
                };
                let id = w.send(&root, command, cx);
                if wrote.is_some() {
                    w.approve_when_saved(&root, id, wrote, texts.clone());
                }
            }
        });
        self.sync_locks(cx);
    }

    fn banner_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some((_, _, shown, _)) = self.conflicts(cx) else {
            return Vec::new();
        };
        let this = cx.entity();
        shown
            .into_iter()
            .map(|b| {
                let this = this.clone();
                banner::banner_row(b, move |b, choice, window, cx| {
                    this.update(cx, |e, cx| match choice {
                        Choice::Tertiary => e.open_compare(window, cx),
                        _ => e.answer(b, choice, cx),
                    })
                })
            })
            .collect()
    }

    // One line per note of the last import paste, or its error, with a dismiss button.
    fn paste_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let lines = match self.pasted.as_ref()? {
            Pasted::Imported(notes) => notes
                .iter()
                .map(|n| format!("{}: {}", n.source, n.detail))
                .collect(),
            Pasted::Failed(message) => vec![format!("Not imported: {message}")],
        };
        let (fill, border) = (cx.theme().muted, cx.theme().border);
        let dismiss = cx.listener(|this, _, _, cx| this.dismiss_pasted(cx));
        // A box of its own, so the notes read apart from the request rows beneath.
        let banner = h_flex()
            .id("paste-banner")
            .test_support()
            .bg(fill)
            .border_1()
            .border_color(border)
            .rounded(style::RADIUS)
            .p_2()
            .gap_2()
            .w_full()
            .min_w_0()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .children(lines.into_iter().map(|l| div().child(l))),
            )
            .child(
                Button::new("dismiss-paste-banner")
                    .label("Dismiss")
                    .on_click(dismiss),
            );
        Some(banner.into_any_element())
    }

    // A file whose URL query and params block disagree gets one muted line over the table,
    // with a button to settle it for either side; the URL is what a send uses meanwhile.
    fn params_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let names = self.view(cx)?.query_mismatch.clone()?.names;
        let text = format!(
            "URL and table disagree on {}. The URL's query is sent.",
            name_list(&names)
        );
        let keep = |id: &'static str, label: &'static str, edit: Edit, cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.keep_query(edit.clone(), cx)))
        };
        let row = h_flex()
            .id("params-mismatch")
            .aria_label(text.clone())
            .test_support()
            .gap_2()
            .w_full()
            .min_w_0()
            .mb(style::SPACE_2)
            .text_size(style::TEXT_SMALL)
            .text_color(cx.theme().muted_foreground)
            .child(div().flex_1().min_w_0().child(text))
            .child(keep("params-keep-url", "Keep URL", Edit::KeepUrlQuery, cx))
            .child(keep(
                "params-keep-table",
                "Keep table",
                Edit::KeepParamsQuery,
                cx,
            ));
        Some(row.into_any_element())
    }

    // Settling rewrites both sides, so it waits while either has an edit in flight. Like a URL
    // edit, it is not an undo step of its own: the history records only table values.
    fn keep_query(&mut self, edit: Edit, cx: &mut Context<Self>) {
        if !self.in_flight(cx) {
            self.send_unrecorded(edit, Some(Section::Params), cx);
        }
    }

    fn section_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        // A counted section shows its enabled rows beside the label (nothing for zero);
        // Body, Auth, Script and Tests instead show a hollow ring when they have content.
        let kind = self.kind(cx);
        let selected = self.section as usize;
        let tabs = Section::ALL.iter().enumerate().map(|(ix, &s)| {
            let label = div()
                .id(("section", ix))
                .test_support()
                .child(s.label_for(kind));
            let mark = match self.section_count(s, cx).and_then(ui::count_badge) {
                Some(badge) => Some(
                    div()
                        .id(("section-count", ix))
                        .test_support()
                        .child(badge)
                        .into_any_element(),
                ),
                None if self.section_marked(s, cx) => Some(
                    ui::mode_ring(("section-mode", ix), Self::mark_words(s))
                        .test_support()
                        .into_any_element(),
                ),
                None => None,
            };
            // Each tab is its own tab stop; Enter or Space opens it as a click would. The
            // focus ring is an overlay on a padded box, since the kit tab clips anything
            // drawn outside its content, so the stroke clears the label and the marker.
            let focus = &self.section_focus[ix];
            let editor = this.clone();
            Tab::new().child(
                h_flex()
                    .relative()
                    .track_focus(focus)
                    .on_key_down(move |event, window, cx| {
                        let key = event.keystroke.key.as_str();
                        if matches!(key, "enter" | "space") {
                            cx.stop_propagation();
                            editor.update(cx, |e, cx| e.show_section(ix, cx));
                        } else if event.keystroke.modifiers.modified() {
                            // Chords with an arrow stay for whoever binds them.
                        } else if let Some(step) = ui::arrow_step(key) {
                            cx.stop_propagation();
                            let next = ix.checked_add_signed(step);
                            let next = next.and_then(|n| editor.read(cx).section_focus.get(n));
                            if let Some(next) = next.cloned() {
                                window.focus(&next, cx);
                            }
                        }
                    })
                    .line_height(relative(style::TAB_LINE_HEIGHT))
                    .px(style::SPACE_0)
                    .gap(style::SPACE_1)
                    .items_center()
                    .child(label)
                    .children(mark)
                    // A click already shows which tab is active, so the ring marks
                    // keyboard focus only.
                    .when(
                        focus.is_focused(window) && crate::focus_mode::is_keyboard(cx),
                        |d| {
                            d.child(
                                div()
                                    .id(("section-focus", ix))
                                    .test_support()
                                    .absolute()
                                    .inset_0()
                                    .border_1()
                                    .border_color(cx.theme().ring),
                            )
                        },
                    ),
            )
        });
        // The menu shows only while last frame's layout overflowed; the watch redraws once
        // when this frame's layout disagrees.
        let overflows = ui::strip_overflows(&self.section_scroll);
        let editor = this.clone();
        let bar = ui::tab_strip("sections", cx)
            .track_scroll(&self.section_scroll)
            .selected_index(selected)
            .children(tabs)
            .when(overflows, |bar| {
                let labels = Section::ALL
                    .iter()
                    .map(|s| s.label_for(kind).into())
                    .collect();
                bar.suffix(ui::strip_overflow(
                    "section-overflow",
                    labels,
                    self.section as usize,
                    move |ix, _, cx| editor.update(cx, |e, cx| e.show_section(ix, cx)),
                ))
            })
            .on_click(move |ix, _, cx| this.update(cx, |e, cx| e.show_section(*ix, cx)));
        div()
            .id("section-strip")
            .test_support()
            .relative()
            .on_scroll(ui::wheel_on_intent(&self.section_scroll))
            .child(bar)
            .children(self.section_fades.render(
                "section-strip",
                &self.section_scroll,
                Section::ALL.len(),
                cx,
            ))
            .child(ui::overflow_watch(&self.section_scroll, overflows))
            .child(
                self.section_reveal
                    .watch(&self.section_scroll, self.section as usize),
            )
            .children(self.section_focus_reveal.watch_focused(
                &self.section_scroll,
                self.section_focus.iter().position(|f| f.is_focused(window)),
            ))
    }

    // The editors scroll inside themselves and know nothing of the section body around them,
    // so a caret that typing pushed past the body's edge is brought back by scrolling the body.
    // Only an edit or a move arms this, so wheeling the body away afterwards is left alone. The
    // caret is measured from the last paint, so a few renders stay armed to catch the new one.
    fn follow_graphql_caret(&mut self, window: &Window, cx: &App) {
        if self.section != Section::Body {
            return;
        }
        let Some(crate::graphql_section::Caret { drawn, key }) =
            self.graphql.read(cx).caret(window, cx)
        else {
            self.caret_key = None;
            return;
        };
        if self.caret_key != Some(key) {
            self.caret_key = Some(key);
            self.caret_follow = 3;
        }
        if self.caret_follow == 0 {
            return;
        }
        self.caret_follow -= 1;
        // A builder tick keeps the editor's view and so can leave the caret scrolled out of
        // it; a caret the editor hides is not the body's to reveal.
        if drawn.is_some_and(|c| self.scroll_body_to(c)) {
            self.caret_follow = 0;
        }
    }

    // A snippet insert leaves the placeholder where the editor's own scroll put it, which can be
    // below the section body's edge; the body scrolls to it over the next few renders.
    fn follow_snippet_caret(&mut self, window: &mut Window, cx: &App) {
        if self.snippet_follow == 0 {
            return;
        }
        // Leaving the script sections ends the follow, so coming back never jumps the body.
        let view = match self.section {
            Section::Tests => &self.tests_script,
            Section::Script => &self.script,
            _ => {
                self.snippet_follow = 0;
                return;
            }
        };
        self.snippet_follow -= 1;
        let Some(caret) = view.read(cx).caret(cx) else {
            return;
        };
        if self.scroll_body_to(caret) {
            self.snippet_follow = 0;
        } else if self.snippet_follow > 0 {
            window.request_animation_frame();
        }
    }

    // Moves the section body just enough to show `caret`; false when it already is in view.
    fn scroll_body_to(&self, caret: Bounds<Pixels>) -> bool {
        let view = self.body_scroll.bounds();
        let past = caret.bottom() - view.bottom();
        let before = view.top() - caret.top();
        let mut offset = self.body_scroll.offset();
        if past > Pixels::ZERO {
            offset.y -= past;
        } else if before > Pixels::ZERO {
            offset.y += before;
        } else {
            return false;
        }
        self.body_scroll.set_offset(offset);
        true
    }

    /// Puts the caret in the URL input.
    pub fn focus_url(&self, window: &mut Window, cx: &mut App) {
        self.url.update(cx, |i, cx| i.focus(window, cx));
    }

    /// Moves focus to the response body, showing the Body tab first; a no-op before any response.
    pub fn focus_response(&self, window: &mut Window, cx: &mut App) {
        // Before the first send the pane is a bare strip with nothing to focus, so focus stays
        // where it was.
        if !self.send_model.read(cx).ever_shown() {
            return;
        }
        self.response
            .update(cx, |pane, cx| pane.focus_body(window, cx));
    }

    /// Shows the section `step` places from the current one, wrapping at either end. Every
    /// section is always in the strip, so the cycle is [`Section::ALL`].
    ///
    /// When focus sat inside the section being left, it moves to the new section's tab in the
    /// strip, since the focused control unmounts and key dispatch would lose its path.
    pub fn step_section(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let inside = self.section_body_focus.contains_focused(window, cx);
        let n = Section::ALL.len() as isize;
        let at = Section::ALL
            .iter()
            .position(|s| *s == self.section)
            .unwrap_or(0) as isize;
        let ix = (at + step).rem_euclid(n) as usize;
        self.show_section(ix, cx);
        if inside && let Some(tab) = self.section_focus.get(ix) {
            window.focus(tab, cx);
        }
    }

    /// Shows `section` under the URL bar.
    pub fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        if let Some(ix) = Section::ALL.iter().position(|s| *s == section) {
            self.show_section(ix, cx);
        }
    }

    fn show_section(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.section != Section::ALL[ix] {
            self.body_scroll.set_offset(Point::default());
        }
        self.section = Section::ALL[ix];
        cx.notify();
    }

    // Each section sits in a wrapper whose id scopes the ids of its controls; Vars heads its
    // table Pre-request and adds the Post-response block under it.
    fn section_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let (id, view): (&'static str, AnyView) = match self.section {
            Section::Params => {
                return div()
                    .id("params")
                    .test_support()
                    .w_full()
                    .children(self.params_notice(cx))
                    .child(self.params.clone())
                    .into_any_element();
            }
            Section::Body => match self.kind(cx) {
                RequestKind::Graphql => ("body", self.graphql.clone().into()),
                _ => ("body", self.body.clone().into()),
            },
            Section::Headers => {
                return div()
                    .id("headers")
                    .test_support()
                    .w_full()
                    .child(self.headers.clone())
                    .child(self.inherited_headers.clone())
                    .child(div().mt(style::SPACE_3).child(self.default_headers.clone()))
                    .into_any_element();
            }
            Section::Auth => ("auth", self.auth.clone().into()),
            Section::Vars => ("vars", self.vars.clone().into()),
            Section::Script => ("script", self.script.clone().into()),
            Section::Assertions => ("assertions", self.assertions.clone().into()),
            Section::Tests => ("tests", self.tests_script.clone().into()),
            Section::Docs => ("docs", self.docs.clone().into()),
            Section::Settings => ("settings", self.settings.clone().into()),
        };
        let body = div().id(id).test_support().w_full().child(view);
        if self.section != Section::Vars {
            return body.into_any_element();
        }
        v_flex()
            .gap(style::SPACE_2)
            .w_full()
            .child(
                v_flex()
                    .gap_1()
                    .w_full()
                    .child(vars_heading("pre-request-heading", "Pre-request", cx))
                    .child(body),
            )
            .child(post_response_block(
                &self.post_response,
                self.post_response_hints(cx),
                (
                    "post-response-heading",
                    "post-response-vars",
                    "post-response-hint",
                ),
                cx,
            ))
            .into_any_element()
    }
}
