//! The response body view: a read-only highlighted editor for small bodies, a virtualized
//! list of plain rows for large ones and for a binary body's Hex or Base64 view, a preview
//! for images, and a bar with the mode toggles, the detected format, a JSONPath or XPath
//! filter, copy and save.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use apinata_core::{
    Assertion, FilterError, JsonFilter, ParsedJson, ParsedXml, RequestResult, XmlFilter,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{DiagnosticColors, InputEditorStyle};
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Escape, Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, Theme, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, App, Bounds, ClickEvent, ClipboardItem, Entity, EventEmitter, FocusHandle,
    Focusable, Font, FontFeatures, HighlightStyle, Hsla, Image, ImageFormat,
    ListHorizontalSizingBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels,
    Point, Rems, Result, ScrollStrategy, SharedString, StyledText, Subscription, Task,
    TestSupportExt, UniformListScrollHandle, WeakEntity, Window, canvas, div, img, point, px,
    uniform_list,
};

use crate::app::AppState;
use crate::body_find::{
    CloseFindInResponse, FIND_HIT_CAP, FindInResponse, FindNextInResponse, FindOptions,
    FindPreviousInResponse, Finder, KEY_CONTEXT, RowDown, RowFirst, RowLast, RowPageDown,
    RowPageUp, RowUp, ToggleFindRegex, ToggleFindWholeWord, find_in_chunked_until, find_selection,
    match_colours,
};
use crate::body_node::{
    ASSERT_VALUE, AssertJsonValue, CopyJsonPath, CopyJsonValue, NodeCopy, SET_VARIABLE,
    SetJsonVariable, json_token_starts, node_entries, node_menu, row_offset,
};
use crate::body_text::{
    BINARY_VIEW_CAP, BinaryMode, BodyKind, Chunked, EDITOR_MAX_BYTES, ImageInfo, ImageKind,
    Prepared, ROW_WIDTH, binary_rows, body_kind, cells, chunk_lines, display_text, format_size,
    image_caption, prepare, prepare_body, wrap_columns,
};
use crate::download_model::{DownloadModel, DownloadPolicy, DownloadState, download_policy};
use crate::keymap::{ResetZoom, ZoomIn, ZoomOut};
use crate::menu_focus;
use crate::theme::{SelectedStyle, selected_look};
use crate::ui::{ChooseButton as _, LockButton as _};
use crate::zoom::{self, ZoomStep, ZoomTarget::Response};
use crate::{confirm, open_external, style, ui};
use gpui_kit::component::input::{
    RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};

// The shown result's body with its secrets redacted, built once per result on the first
// prepare of a text body and reused by every later one. `None` inside means redaction changed
// nothing, so the body has no eye and the raw text is the shown text.
#[derive(Default)]
struct Mask {
    text: OnceLock<Option<Arc<str>>>,
    runs: AtomicUsize,
}

impl Mask {
    // The text a body shows: the redacted copy, or `raw` when redaction left it alone.
    fn shown<'a>(&'a self, result: &RequestResult, raw: &'a str) -> &'a str {
        let masked = self.text.get_or_init(|| {
            self.runs.fetch_add(1, Ordering::Relaxed);
            let text = result.redact(raw);
            (text != raw).then(|| text.into())
        });
        masked.as_deref().unwrap_or(raw)
    }

    // Whether redaction changed the body, which is when it is worth an eye.
    fn changed(&self) -> bool {
        matches!(self.text.get(), Some(Some(_)))
    }
}

/// What redaction puts in place of a secret.
pub const MASKED: &str = "[redacted]";

/// The open button's tooltip for an HTML or SVG body.
pub const OPEN_PAGE_TIP: &str = "Opens an inert copy in your browser: scripts, network loads and forms are blocked, and secrets stay redacted";

/// Why a kept body's eye cannot reveal anything: secret values are never saved.
pub const SCRUBBED_TIP: &str = "Secret values are not saved, so they cannot be shown.";

/// How long typing in the filter, or a rows-mode find query, must pause before it runs.
pub const SETTLE: Duration = Duration::from_millis(250);

// The pane measurements a wrapped body is cut by.
#[derive(Clone, Copy)]
struct WrapBasis {
    room: Pixels,
    advance: Pixels,
    pad: Pixels,
}

impl WrapBasis {
    // The columns left beside a gutter wide enough for `lines` line numbers, plus its inset.
    fn columns(&self, lines: usize) -> usize {
        let gutter = self.pad + digit_gutter(self.advance, gutter_digits(lines), self.pad);
        wrap_columns((self.room - gutter).max(Pixels::ZERO), self.advance)
    }
}

/// Rows holding no more text than this are re-cut on the main thread; a few hundred kilobytes
/// take well under a frame.
const RECHUNK_INLINE_BYTES: usize = 64 * 1024;

/// Largest XML body whose parse is kept across filter edits; its tree costs several times the
/// body, so a larger one is parsed afresh for each filter instead.
pub const XML_PARSE_CACHE_MAX_BYTES: usize = 4 << 20;

// An unwrapped line wider than this goes to the rows list, whose rows stay short, because the
// editor's horizontal scroll falls short of the end of very long lines.
const EDITOR_LINE_MAX_WIDTH: f32 = 32_768.;

/// How a body is shown: re-indented and highlighted, or as decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    /// JSON and XML re-indented; JSON, HTML and XML highlighted.
    Pretty,
    /// The decoded text, unhighlighted.
    Raw,
}

/// What the view currently shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// No body, or no response.
    Empty,
    /// The body in the editor.
    Editor,
    /// The body as this many plain rows.
    Lines(usize),
    /// A binary body of this many bytes.
    Binary(u64),
    /// A whole image body, previewed.
    Image(ImageInfo),
}

/// How many digits the gutter holds for a body of `lines` rows (at least one).
pub fn gutter_digits(lines: usize) -> usize {
    lines.max(1).ilog10() as usize + 1
}

/// The gutter width for `digits` mono digits of `advance` each, plus `pad`.
pub fn digit_gutter(advance: Pixels, digits: usize, pad: Pixels) -> Pixels {
    advance * digits as f32 + pad
}

/// The gutter for `digits` digits in the theme's mono font at `size`, from the font's own
/// advance of "0"; a font the text system cannot measure falls back to 0.6 em.
pub fn mono_gutter(digits: usize, size: Pixels, window: &Window, cx: &App) -> Pixels {
    digit_gutter(
        mono_advance(size, window, cx),
        digits,
        style::resolve(style::SPACE_2, window),
    )
}

/// The advance of one mono character at `size`, from the font's own "0".
pub fn mono_advance(size: Pixels, window: &Window, cx: &App) -> Pixels {
    let family = cx.theme().mono_font_family.clone();
    let font = Font {
        family,
        ..window.text_style().font()
    };
    let system = window.text_system();
    system
        .advance(system.resolve_font(&font), size, '0')
        .map_or(size * 0.6, |a| a.width)
}

// The shown body parsed for the filter, filled by the first job that needs it; later jobs
// for the same response and reveal share it, so a keystroke never parses the body again.
type ParsedSlot = Arc<OnceLock<Result<ParsedJson, FilterError>>>;
type XmlSlot = Arc<OnceLock<Result<ParsedXml, FilterError>>>;

#[derive(Clone, Default)]
struct ParsedSlots {
    json: ParsedSlot,
    xml: XmlSlot,
}

struct JsonCache {
    reveal: bool,
    slot: ParsedSlots,
}

// XPath output ends every match in a newline; the last one is dropped on screen, so the view
// ends on the last match rather than an empty line.
fn xpath_shown(out: &str) -> &str {
    out.strip_suffix('\n').unwrap_or(out)
}

// Rides inside a background job and counts it when it ends without a result. A replaced job
// either stops at a cancel check or, when its waiter is dropped before it ran, is dropped
// unpolled; both drop this guard unfinished.
struct JobGuard {
    counted: Arc<AtomicUsize>,
    finished: bool,
}

impl JobGuard {
    fn finish<T>(mut self, out: Option<T>) -> Option<T> {
        self.finished = out.is_some();
        out
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.counted.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Shows one result's response body.
pub struct BodyView {
    editor: Entity<EditorState>,
    // The rows of a long body or a binary view, with each row's source line number.
    chunked: Arc<Chunked>,
    // The byte width the rows are cut at, and the longest row, which a scrolling list measures.
    chunk_width: usize,
    widest: usize,
    line_count: usize,
    // Whether long lines wrap, and the Preferences default last applied.
    wrap: bool,
    default_wrap: bool,
    // A re-chunk in flight, with the width it cuts at.
    rechunk: Option<(usize, Task<()>)>,
    rechunk_generation: u64,
    lines_scroll: UniformListScrollHandle,
    shown: Shown,
    kind: Option<BodyKind>,
    // The shown image's bytes, built once per body so gpui's image cache key stays put.
    image: Option<Arc<Image>>,
    mode: BodyMode,
    binary_mode: BinaryMode,
    editing: bool,
    // Whether the View and Edit toggle is offered at all.
    editable: bool,
    // Focus for the body as a whole, so the find chord reaches the rows too.
    focus: FocusHandle,
    find_input: Entity<InputState>,
    find_open: bool,
    find_case_sensitive: bool,
    find_regex: bool,
    find_whole_word: bool,
    // The query is a pattern that does not compile, so it matches nothing.
    find_invalid: bool,
    // Editor-mode hits while regex or whole-word is on: the kit's search is literal only, so
    // these are found here and painted as range fills, all hits under the current one.
    editor_hits: Vec<Range<usize>>,
    editor_hit: Option<usize>,
    hit_marks: Option<(RangeDecorationCollection, RangeDecorationCollection)>,
    // The (other, current) colours the hits were last painted in.
    mark_colours: Option<(Hsla, Hsla)>,
    // How many editor-mode scans have started, for tests to count.
    editor_scans: usize,
    // The editor-mode hits stopped at the cap with more left.
    editor_capped: bool,
    // Where the next editor-mode scan puts the current hit, when not the first.
    editor_place: Option<HitPlace>,
    // An edit's rescan waiting for the typing in the editor to pause.
    edit_settle: Option<Task<()>>,
    // A hit for the next paint to centre, with how many more frames it may wait to be laid out.
    centre_hit: Rc<RefCell<Option<PendingCentre>>>,
    // Rows-mode find: the matches in row order and the current one. The editor keeps its own,
    // and a scan lands only if no newer query, case switch or text has started since.
    row_matches: Vec<(usize, Range<usize>)>,
    // The later pieces of matches that cross a chunk seam: (match index, row, range).
    row_tails: Vec<(usize, usize, Range<usize>)>,
    // Where the current match sat in the source before a re-cut, for the next scan to resume at.
    find_anchor: Option<(u32, usize)>,
    // Whether the editor holds the shown text (it may be shown as rows), and the cells of its
    // longest line.
    editor_backed: bool,
    editor_cells: usize,
    // Set when the current match was on screen before a re-cut, so the scan that follows
    // brings it back into view.
    reveal_after_cut: bool,
    // Set when the next paint should scroll the rows list sideways to the current match.
    reveal_sideways: Cell<bool>,
    // Set by a wrap toggle: the editor's current match is brought into view after its next layout.
    reveal_editor: Rc<Cell<bool>>,
    row_current: Option<usize>,
    find_generation: u64,
    find_task: Option<Task<()>>,
    // Raised when a newer scan replaces the running one, which then stops at its next line.
    find_cancel: Arc<AtomicBool>,
    // The query the last run searched for.
    find_ran: SharedString,
    filter_input: Entity<InputState>,
    filter_open: bool,
    filter_error: Option<SharedString>,
    // The expression whose output is on screen, so an invalid edit keeps naming the last
    // valid one, and whether that output matched nothing.
    applied_filter: Option<SharedString>,
    filter_no_match: bool,
    // A discard question is open; further mode switches wait for its answer.
    asking: bool,
    // The editor's text as last prepared; a different value means scratch edits.
    prepared_text: SharedString,
    result: Option<Arc<RequestResult>>,
    // The result's redacted body, and whether the eye has revealed the raw one.
    mask: Arc<Mask>,
    reveal: bool,
    // The shown body was kept with its secrets scrubbed, so there is nothing to reveal.
    scrubbed: bool,
    // Where "Download full response" re-runs the request, when the host wired one.
    download: Option<DownloadHook>,
    generation: u64,
    task: Option<Task<()>>,
    // Raised when a newer prepare replaces the running one, so its filter stops early.
    job_cancel: Arc<AtomicBool>,
    // At most one parsed copy of the body; a new response or a closed or empty filter drops it.
    json_cache: Option<JsonCache>,
    // For tests: bodies parsed for the filter, and jobs that stopped on a cancel.
    filter_parses: Arc<AtomicUsize>,
    cancelled_jobs: Arc<AtomicUsize>,
    // A typed filter waiting for the typing to pause; replacing it cancels the wait.
    filter_settle: Option<Task<()>>,
    // A typed rows-mode find query waiting for the typing to pause, likewise.
    find_settle: Option<Task<()>>,
    _settings: Option<Subscription>,
    _filter_changes: Subscription,
    _find_changes: Subscription,
    // The root's bounds as last painted, for `pointer_inside`.
    painted: Rc<Cell<Option<Bounds<Pixels>>>>,
    // Pixel ctrl+scroll not yet stepped.
    scroll_pending: Rc<Cell<f32>>,
    // The selection colour last laid onto the editor's style.
    selection_laid: Rc<Cell<Option<Hsla>>>,
    // A narrow pane offers Copy and Save from its overflow menu, so the bar drops them.
    compact: bool,
    // The node entries and selection state captured by the last right-press on the editor.
    node_pending: Rc<RefCell<(Vec<NodeCopy>, bool)>>,
    // What held focus when the rows menu opened, for `menu_focus` to hand back.
    node_held: Rc<RefCell<Option<FocusHandle>>>,
    // Whether the chosen node entry handed focus to the host, as Assert this value does by
    // focusing the new row; the menu's focus fallback must not take it back.
    node_handed_focus: Rc<Cell<bool>>,
    // Whether the node menu offers Assert this value: only a host that owns a request does.
    assert_offered: bool,
    // The rows-mode row the pointer last rested on, which the value-actions button follows.
    hovered_row: Option<usize>,
    // The row the keyboard last moved to, marked in the rows list; None until a key moves it.
    current_row: Option<usize>,
    // Whether the value-actions menu is open; the hovered row stays put while it is.
    value_menu_open: Rc<Cell<bool>>,
    // Redraws when the editor's caret moves, so the value-actions button follows its line.
    _editor_changes: Subscription,
    _editor_edits: Subscription,
    _theme_changes: Subscription,
}

// Where an editor-mode scan leaves the current hit when it lands: the first one, the index
// it had, or the one at or after a byte offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitPlace {
    First,
    Keep,
    At(usize),
}

// A hit waiting to be centred, and how many more frames it may wait to be laid out.
type PendingCentre = (Range<usize>, u8);

// A filled find highlight over `range`.
fn fill(range: Range<usize>, colour: Hsla) -> RangeDecoration {
    RangeDecoration::new(range)
        .with_style(RangeDecorationStyle::Fill)
        .with_color(colour)
}

/// The id of the button that opens the value actions without a right-click.
pub const VALUE_ACTIONS: &str = "body-value-actions";
const VALUE_ACTIONS_TIP: &str = "Value actions";

/// Emitted when the node menu's Set as variable is chosen: the name to prefill and the node's
/// value as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetVariable {
    /// The prefilled variable name; empty when the node has no key.
    pub name: String,
    /// The value, as Copy value would write it, except that a secret value of a masked body is
    /// the real one.
    pub value: String,
    /// Whether the value was masked in the shown body, so it must not be shown or written to a
    /// file.
    pub sensitive: bool,
}

impl EventEmitter<SetVariable> for BodyView {}

/// Emitted when the node menu's Assert this value is chosen: the assertion to append to the
/// request.
#[derive(Debug, Clone)]
pub struct AssertValue(pub Assertion);

impl EventEmitter<AssertValue> for BodyView {}

/// Asks for a schema test built from the response; carries the unfiltered response text.
pub struct SchemaTest(pub String);

impl EventEmitter<SchemaTest> for BodyView {}

impl BodyView {
    /// An empty view in `Pretty` mode over an editor that starts in View (read-only).
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The kit panel would label "2/5" and offer replace, so the body runs its own bar
        // and leaves the kit's search keys to propagate.
        let default_wrap = crate::settings::wrap_editors(cx);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .searchable(false)
                .soft_wrap(default_wrap)
        });
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("$..id"));
        let filter_changes =
            cx.subscribe_in(&filter_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_typed(window, cx);
                }
            });
        // Typing re-runs the query; Enter and Shift+Enter step through the matches. Enter
        // also reports a change with the text as it was, which must not restart the search.
        let find_changes = cx.subscribe_in(
            &find_input,
            window,
            |this, input, event, window, cx| match event {
                InputEvent::Change => {
                    let query = input.read(cx).value();
                    if query != this.find_ran {
                        this.find_typed(query, cx);
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.find_next(window, cx),
                InputEvent::PressEnter { shift: true, .. } => this.find_previous(window, cx),
                _ => {}
            },
        );
        // Theme writes refresh every window, but an offset-only change refreshes nothing,
        // so the view redraws itself on any settings change.
        let settings = cx
            .try_global::<AppState>()
            .map(|state| state.settings.clone())
            .map(|settings| {
                cx.observe_in(&settings, window, |this, _, window, cx| {
                    let want = crate::settings::wrap_editors(cx);
                    if want != this.default_wrap {
                        this.default_wrap = want;
                        this.set_wrap(want, window, cx);
                    }
                    cx.notify();
                })
            });
        let editor_changes = cx.observe(&editor, |_, _, cx| cx.notify());
        // A theme switch repaints the hits, whose colours come from the theme.
        let theme_changes = cx.observe_global::<Theme>(|this, cx| {
            if !this.editor_hits.is_empty() {
                this.paint_hits(cx);
            }
        });
        // An edit moves text under the editor-mode hits, so they are found again in place.
        let editor_edits = cx.subscribe(&editor, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && this.own_editor_find() && this.find_open {
                // A scan already running read the old text, so its offsets must not land.
                this.void_find_scan();
                let timer = cx.background_executor().timer(SETTLE);
                this.edit_settle = Some(cx.spawn(async move |this, cx| {
                    timer.await;
                    let rescan = |this: &mut Self, cx: &mut Context<Self>| {
                        if this.own_editor_find() && this.find_open && !this.in_rows() {
                            this.find_editor_hits(HitPlace::Keep, cx);
                        }
                    };
                    this.update(cx, rescan).ok();
                }));
            }
        });
        BodyView {
            editor,
            chunked: Arc::default(),
            chunk_width: ROW_WIDTH,
            widest: 0,
            wrap: default_wrap,
            default_wrap,
            rechunk: None,
            rechunk_generation: 0,
            lines_scroll: UniformListScrollHandle::new(),
            shown: Shown::Empty,
            kind: None,
            image: None,
            mode: BodyMode::Pretty,
            binary_mode: BinaryMode::Hex,
            editing: false,
            editable: true,
            focus: cx.focus_handle(),
            find_input,
            find_open: false,
            find_case_sensitive: false,
            find_regex: false,
            find_whole_word: false,
            find_invalid: false,
            editor_hits: Vec::new(),
            editor_hit: None,
            hit_marks: None,
            mark_colours: None,
            editor_scans: 0,
            editor_capped: false,
            editor_place: None,
            edit_settle: None,
            centre_hit: Rc::default(),
            row_matches: Vec::new(),
            row_tails: Vec::new(),
            find_anchor: None,
            editor_backed: false,
            editor_cells: 0,
            reveal_after_cut: false,
            reveal_sideways: Cell::new(false),
            reveal_editor: Rc::default(),
            line_count: 0,
            row_current: None,
            find_generation: 0,
            find_task: None,
            find_cancel: Arc::default(),
            find_ran: SharedString::default(),
            filter_input,
            filter_open: false,
            filter_error: None,
            applied_filter: None,
            filter_no_match: false,
            asking: false,
            prepared_text: SharedString::default(),
            result: None,
            mask: Arc::default(),
            reveal: false,
            scrubbed: false,
            download: None,
            generation: 0,
            task: None,
            job_cancel: Arc::default(),
            json_cache: None,
            filter_parses: Arc::default(),
            cancelled_jobs: Arc::default(),
            filter_settle: None,
            find_settle: None,
            _settings: settings,
            _filter_changes: filter_changes,
            _find_changes: find_changes,
            painted: Rc::default(),
            scroll_pending: Rc::default(),
            selection_laid: Rc::default(),
            compact: false,
            node_pending: Rc::default(),
            node_held: Rc::default(),
            node_handed_focus: Rc::default(),
            assert_offered: false,
            hovered_row: None,
            current_row: None,
            value_menu_open: Rc::default(),
            _editor_changes: editor_changes,
            _editor_edits: editor_edits,
            _theme_changes: theme_changes,
        }
    }

    /// Whether the bar leaves Copy and Save to the pane's overflow menu.
    pub fn compact(&self) -> bool {
        self.compact
    }

    /// Drops Copy and Save from the bar while `on`, redrawing only on a change.
    pub fn set_compact(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.compact != on {
            self.compact = on;
            cx.notify();
        }
    }

    /// How many times the shown result's body was redacted: once per result at most.
    pub fn mask_runs(&self) -> usize {
        self.mask.runs.load(Ordering::Relaxed)
    }

    /// Whether the eye has revealed the raw body.
    pub fn revealed(&self) -> bool {
        self.reveal
    }

    /// Marks the body shown next as kept with its secrets scrubbed; its eye then says why it
    /// cannot reveal them.
    pub fn set_scrubbed(&mut self, scrubbed: bool) {
        self.scrubbed = scrubbed;
    }

    /// Whether the body shows an eye: a text body that redaction changed.
    pub fn maskable(&self) -> bool {
        self.is_text() && self.mask.changed()
    }

    /// Shows the raw body (`true`) or the redacted one. With scratch edits it first asks, and
    /// switches only on Discard.
    pub fn set_reveal(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on == self.reveal || self.asking {
            return;
        }
        self.discarding_edits(window, cx, move |view, window, cx| {
            view.reveal = on;
            // A masked body is read-only.
            if !on {
                view.editing = false;
            }
            view.refresh(window, cx);
            cx.notify();
        });
    }

    // Runs `then` now, or after Discard when the editor holds scratch edits.
    fn discarding_edits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.edited_text(cx).is_none() {
            then(self, window, cx);
            return;
        }
        self.asking = true;
        let answer = confirm::ask(
            confirm::discard_ask("Discard your edits to the response body?"),
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.recv().await;
            this.update_in(cx, |view, window, cx| {
                view.asking = false;
                if choice == Ok(1) {
                    then(view, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether Copy has anything to copy: a non-empty text body, or a binary body's view,
    /// and never a filter output that matched nothing.
    pub fn can_copy(&self) -> bool {
        !self.filter_no_match && self.copy_offered()
    }

    // Copy's slot shows for any copyable body; a filter that matched nothing only disables it,
    // so the toolbar does not shift.
    fn copy_offered(&self) -> bool {
        self.has_body() && (self.is_text() || matches!(self.shown, Shown::Binary(_)))
    }

    /// Whether the open button would act: a binary, HTML or UTF-8 SVG body that was not cut
    /// at the size limit, with an opener installed.
    pub fn can_open(&self, cx: &App) -> bool {
        self.open_offered(cx) && !self.truncated()
    }

    // The open button shows for these bodies even when cut, disabled, so the bar does not shift.
    fn open_offered(&self, cx: &App) -> bool {
        (matches!(self.shown, Shown::Binary(_)) || self.browser_page().is_some())
            && open_external::installed(cx)
    }

    // Which inert browser copy the body opens as. An SVG is redacted as text, so one that is
    // not UTF-8 gets no action.
    fn browser_page(&self) -> Option<BrowserPage> {
        let body = (self.result.as_ref())
            .and_then(|r| r.response.as_ref())
            .map_or(&[][..], |r| r.body());
        match self.shown {
            Shown::Image(info) if info.kind == ImageKind::Svg => std::str::from_utf8(body)
                .is_ok()
                .then_some(BrowserPage::Svg),
            Shown::Editor | Shown::Lines(_) if self.kind == Some(BodyKind::Html) => {
                Some(BrowserPage::Html)
            }
            _ => None,
        }
    }

    /// Why the Schema test button is locked, or `None` when it would act. Only a JSON
    /// response can be inferred from, and a cut body would give a schema of its start.
    pub fn schema_test_lock(&self) -> Option<&'static str> {
        if self.truncated() {
            return Some("The body was cut at the size limit");
        }
        // The redacted copy the test is built from is made with the body, off the main thread.
        let has_response = (self.result.as_ref()).is_some_and(|r| r.response.is_some());
        if self.kind.is_none() && has_response {
            return Some("Preparing the body");
        }
        let json = self.kind == Some(BodyKind::Json) && self.is_text();
        (!json).then_some("Needs a JSON response")
    }

    /// The Schema test button's tooltip: the lock reason, else what it does.
    pub fn schema_test_tooltip(&self) -> &'static str {
        self.schema_test_lock().unwrap_or(
            "Add a test that checks the response against a schema inferred from the full body",
        )
    }

    /// Emits [`SchemaTest`] with the response text as shown, redacted until the eye reveals it
    /// (a JSON key can be a secret and lands in the script), ignoring any filter or edit.
    fn request_schema_test(&mut self, cx: &mut Context<Self>) {
        if self.schema_test_lock().is_some() {
            return;
        }
        if let Some(text) = self.source_text().map(str::to_owned) {
            cx.emit(SchemaTest(text));
        }
    }

    fn truncated(&self) -> bool {
        (self.result.as_ref())
            .and_then(|r| r.response.as_ref())
            .is_some_and(|r| r.truncated())
    }

    /// Why Save body is locked (a cut body would be written short), or `None` when it would write.
    pub fn save_lock(&self) -> Option<&'static str> {
        if !self.truncated() {
            return None;
        }
        Some(if self.download_offered() {
            "The body was cut at the size limit; use Download full response"
        } else {
            "The body was cut at the size limit, so it cannot be saved"
        })
    }

    /// Whether Save would write: a non-empty body that was not cut.
    pub fn can_save(&self) -> bool {
        self.has_body() && self.save_lock().is_none()
    }

    fn has_body(&self) -> bool {
        (self.result.as_ref())
            .and_then(|r| r.response.as_ref())
            .is_some_and(|r| !r.body().is_empty())
    }

    /// The response body's text size: the zoomed mono size plus the saved offset, clamped to
    /// 8..=32 px, independent of the editor font offset. Without an `AppState` it is the
    /// theme mono size.
    pub fn font_size(cx: &App) -> Pixels {
        match cx.try_global::<AppState>() {
            Some(state) => {
                let settings = state.settings.read(cx).get();
                let mono = crate::zoom::mono_font_size(settings.zoom);
                crate::zoom::response_font_size(mono.into(), settings.response_font_offset)
            }
            None => cx.theme().mono_font_size,
        }
    }

    /// Whether the pointer is inside the body as last painted. Hover hitboxes read false
    /// right after keyboard input, so this tests the painted bounds instead.
    pub fn pointer_inside(&self, window: &Window) -> bool {
        self.painted
            .get()
            .is_some_and(|b| b.contains(&window.mouse_position()))
    }

    /// Shows `result`'s body, or nothing for `None`, in the current mode.
    pub fn show(
        &mut self,
        result: Option<Arc<RequestResult>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The old kind describes the old body; until the new prepare lands, Copy and
        // the format label must not act on it.
        // A different result forgets the reveal and gets its own redacted copy.
        let same = matches!((&self.result, &result), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
        if !same {
            self.reveal = false;
            self.editing = false;
            self.mask = Arc::default();
            self.json_cache = None;
        }
        self.result = result;
        self.kind = None;
        self.refresh(window, cx);
    }

    /// Switches between pretty and raw, re-preparing the shown body. Choosing the mode
    /// already shown does nothing; with scratch edits it first asks, and switches only on
    /// Discard.
    pub fn set_mode(&mut self, mode: BodyMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.mode || self.asking {
            return;
        }
        self.discarding_edits(window, cx, move |view, window, cx| {
            view.apply_mode(mode, window, cx)
        });
    }

    fn apply_mode(&mut self, mode: BodyMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.refresh(window, cx);
        cx.notify();
    }

    /// The Hex or Base64 view a binary body shows; Hex until changed, kept across bodies.
    pub fn binary_mode(&self) -> BinaryMode {
        self.binary_mode
    }

    /// Switches a binary body between Hex and Base64, re-formatting it off the main thread.
    /// A binary body is never editable, so nothing is asked first.
    pub fn set_binary_mode(
        &mut self,
        mode: BinaryMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode != self.binary_mode {
            self.binary_mode = mode;
            self.refresh(window, cx);
            cx.notify();
        }
    }

    /// Whether long lines wrap.
    pub fn wrap(&self) -> bool {
        self.wrap
    }

    /// Wraps or unwraps long lines: the editor soft-wraps, and long-body rows are re-cut at
    /// the next frame.
    pub fn set_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.wrap = wrap;
        self.editor
            .update(cx, |e, cx| e.set_soft_wrap(wrap, window, cx));
        self.reshape(true, window, cx);
        // The match sits elsewhere in the new layout; the marker re-reveals it once that exists.
        self.reveal_editor.set(true);
        if let Some(hit) = self.editor_hit.and_then(|ix| self.editor_hits.get(ix)) {
            *self.centre_hit.borrow_mut() = Some((hit.clone(), 1));
        }
        cx.notify();
    }

    /// Per shown row, its source line number on a line's first row and 0 on a continuation.
    pub fn line_numbers(&self) -> &[u32] {
        &self.chunked.line_nos
    }

    /// The byte width the long-body rows are cut at now.
    pub fn wrap_columns(&self) -> usize {
        self.chunk_width
    }

    // Whether the bar offers the Wrap toggle: text in the editor or in long-body rows.
    fn wrap_offered(&self) -> bool {
        matches!(self.shown, Shown::Editor | Shown::Lines(_))
    }

    /// The rows the rows list shows now (long text, or a binary body's view).
    pub fn shown_rows(&self) -> &[SharedString] {
        match self.shown {
            Shown::Lines(_) | Shown::Binary(_) => &self.chunked.rows,
            _ => &[],
        }
    }

    /// Whether the body editor accepts typing; false by default.
    pub fn editing(&self) -> bool {
        self.edit_on()
    }

    // Edit mode as it acts: a masked body is read-only whatever the switch says, so a mask that
    // lands after the switch was set cannot be written back.
    fn edit_on(&self) -> bool {
        self.editing && !self.masked()
    }

    /// Whether the body shows its redacted text, which is read-only: editing and saving the
    /// edits would put the redaction marker into the file.
    pub fn masked(&self) -> bool {
        !self.reveal && self.maskable()
    }

    /// Switches between View (read-only) and Edit (scratch edits). A masked body stays in View.
    pub fn set_editing(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = on && !self.masked();
        self.reshape(true, window, cx);
        cx.notify();
    }

    /// Offers the View and Edit toggle when `on`; without it the body stays read-only.
    pub fn set_editable(&mut self, on: bool) {
        self.editable = on;
    }

    /// Opens the find bar when the editor or the rows show text, focuses its field and
    /// selects its text.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.shown, Shown::Editor | Shown::Lines(_)) {
            return;
        }
        self.find_open = true;
        self.find_input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
        self.run_find(cx);
        cx.notify();
    }

    /// Hides the bar, ends the highlights and focuses the editor, or the body for rows.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.find_settle = None;
        self.editor.update(cx, |e, cx| e.close_search(cx));
        self.clear_editor_hits(cx);
        self.clear_row_matches();
        if self.shown == Shown::Editor {
            self.editor.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Whether the find bar is showing.
    pub fn find_open(&self) -> bool {
        self.find_open
    }

    /// The find bar's query field.
    pub fn find_input(&self) -> &Entity<InputState> {
        &self.find_input
    }

    /// Sets the find field's text and runs it at once, without the settle delay typing gets.
    pub fn set_find_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.find_input
            .update(cx, |i, cx| i.set_value(query.to_string(), window, cx));
        self.run_find(cx);
    }

    /// "" for an empty query, "Invalid pattern", "No matches", else "{current} of {total}"
    /// counting from 1.
    pub fn find_label(&self, cx: &App) -> String {
        if self.find_input.read(cx).value().is_empty() {
            return String::new();
        }
        if self.find_invalid {
            return "Invalid pattern".into();
        }
        let (current, total) = if self.in_rows() {
            (self.row_current, self.row_matches.len())
        } else if self.own_editor_find() {
            (self.editor_hit, self.editor_hits.len())
        } else {
            let matcher = &self.editor.read(cx).search_session().matcher;
            (matcher.current(), matcher.len())
        };
        let more = if self.own_editor_find() && !self.in_rows() && self.editor_capped {
            "+"
        } else {
            ""
        };
        match current {
            Some(ix) => format!("{} of {total}{more}", ix + 1),
            None => "No matches".into(),
        }
    }

    fn can_step_find(&self, cx: &App) -> bool {
        self.find_open && !self.find_input.read(cx).value().is_empty()
    }

    /// Moves to the next match, wrapping, and scrolls it into view.
    pub fn find_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_rows() {
            self.step_row_match(1);
        } else if self.own_editor_find() {
            if let Some(ix) = self.editor_hit {
                self.select_hit((ix + 1) % self.editor_hits.len(), cx);
            }
        } else {
            self.centred_step(window, cx, |e, cx| e.next_search_match(cx));
        }
        cx.notify();
    }

    // Steps the editor's match with the kit's scroll margin at its cap of half the viewport,
    // which lands the match mid-view; clicks, arrows and drags keep the default margin.
    fn centred_step(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        step: impl FnOnce(&mut EditorState, &mut Context<EditorState>) -> Option<Range<usize>>,
    ) {
        self.editor.update(cx, |e, cx| {
            e.set_cursor_surrounding_lines(Some(usize::MAX), window, cx);
            step(e, cx);
            e.set_cursor_surrounding_lines(None, window, cx);
        });
    }

    // A click on a bar button takes focus off the field; hand it back so typing, ctrl+a and
    // Enter keep acting on the query.
    fn refocus_find(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_input.update(cx, |i, cx| i.focus(window, cx));
    }

    // One of the bar's on/off toggles: a ghost icon button drawn selected while `on`, whose
    // click runs `flip` and hands focus back to the field. `ids` are (wrapper, button).
    fn find_toggle(
        &self,
        ids: (&'static str, &'static str),
        icon: IconName,
        tooltip: String,
        on: bool,
        flip: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(ids.0)
            .test_support()
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                flip(this, cx);
                this.refocus_find(window, cx);
            }))
            .child(ui::icon_button(ids.1, icon, &tooltip, None).choose(
                on,
                SelectedStyle::Ghost,
                cx,
            ))
    }

    /// Moves to the previous match, wrapping, and scrolls it into view.
    pub fn find_previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_rows() {
            let back = self.row_matches.len().saturating_sub(1);
            self.step_row_match(back);
        } else if self.own_editor_find() {
            if let Some(ix) = self.editor_hit {
                let len = self.editor_hits.len();
                self.select_hit((ix + len - 1) % len, cx);
            }
        } else {
            self.centred_step(window, cx, |e, cx| e.previous_search_match(cx));
        }
        cx.notify();
    }

    // The kit's Editor projects its whole style from the theme each render, with no way to
    // change one colour. This marker, drawn after it, lays the same style with the find
    // selection onto the state before anything paints.
    fn find_colour_marker(&self) -> impl IntoElement {
        let (editor, painted) = (self.editor.clone(), self.editor.clone());
        let (laid, reveal) = (
            Rc::clone(&self.selection_laid),
            Rc::clone(&self.reveal_editor),
        );
        let centre = Rc::clone(&self.centre_hit);
        canvas(
            move |_, _, cx| {
                let style = editor_style(cx);
                laid.set(Some(style.selection));
                editor.update(cx, |e, _| e.set_editor_style(style));
            },
            // Painted after the editor, so its layout is the current one: stepping off the
            // match and back then scrolls it to mid-view.
            move |_, _, window, cx| {
                painted.update(cx, |e, cx| {
                    // A laid-out hit is centred; one off screen is first selected, which
                    // scrolls it into view, and centred on a later frame.
                    let pending = centre.borrow_mut().take();
                    if let Some((hit, tries)) = pending {
                        let laid = (e.range_to_bounds(&hit), e.text_bounds(), e.line_height());
                        if let (Some(at), Some(view), Some(line)) = laid {
                            let top = (view.size.height - line) / 2. - (at.top() - view.top());
                            let x = e.scroll_offset().x;
                            e.set_scroll_offset(point(x, top.min(Pixels::ZERO)), cx);
                        } else if tries > 0 {
                            e.set_selected_range(hit.clone(), cx);
                            *centre.borrow_mut() = Some((hit, tries - 1));
                            window.refresh();
                        }
                    }
                    if reveal.take() && e.search_session().matcher.current().is_some() {
                        e.set_cursor_surrounding_lines(Some(usize::MAX), window, cx);
                        e.previous_search_match(cx);
                        e.next_search_match(cx);
                        e.set_cursor_surrounding_lines(None, window, cx);
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    fn in_rows(&self) -> bool {
        matches!(self.shown, Shown::Lines(_))
    }

    // Moves the current row match `by` places forward, wrapping, and centres its row.
    fn step_row_match(&mut self, by: usize) {
        let len = self.row_matches.len();
        if let Some(current) = self.row_current.filter(|_| len > 0) {
            self.row_current = Some((current + by) % len);
            self.scroll_to_row_match();
        }
    }

    // Centres the current match's row; the paint then brings it into view sideways too.
    fn scroll_to_row_match(&self) {
        if let Some((row, _)) = self.row_current.and_then(|ix| self.row_matches.get(ix)) {
            self.lines_scroll
                .scroll_to_item(*row, ScrollStrategy::Center);
            self.reveal_sideways.set(true);
        }
    }

    // Moves the unwrapped rows list sideways when the current match is outside the viewport,
    // leaving it centred; `gutter` and `advance` place the match's columns.
    fn reveal_match_sideways(&self, gutter: Pixels, advance: Pixels) {
        if !self.reveal_sideways.take() {
            return;
        }
        let Some((row, range)) = self.row_current.and_then(|ix| self.row_matches.get(ix)) else {
            return;
        };
        let text = &self.chunked.rows[*row];
        let end = range.end.min(text.len());
        let start = range.start.min(end);
        let from = gutter + advance * cells(&text[..start]) as f32;
        let to = from + advance * cells(&text[start..end]) as f32;
        let state = self.lines_scroll.0.borrow();
        let view = state.last_item_size.map_or(Pixels::ZERO, |s| s.item.width);
        let offset = state.base_handle.offset();
        if view <= Pixels::ZERO || (from >= -offset.x && to <= -offset.x + view) {
            return;
        }
        let left = (from - (view - (to - from)) / 2.).max(Pixels::ZERO);
        state.base_handle.set_offset(point(-left, offset.y));
    }

    // Moves the current row to `target(current, page)`, clamped to the rows, keeping it in view
    // and the value-actions button level with it. With no current row the top visible one is
    // the starting point. Acts only while the body itself holds focus, so the find field and
    // the other inputs keep their keys.
    fn move_current(
        &mut self,
        target: impl FnOnce(Step) -> usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let last = self.chunked.len().checked_sub(1);
        let shows_rows = matches!(self.shown, Shown::Lines(_) | Shown::Binary(_));
        let (Some(last), true, true) = (last, shows_rows, self.focus.is_focused(window)) else {
            cx.propagate();
            return;
        };
        let visible = self.visible_rows();
        let page = visible.as_ref().map_or(1, |v| v.len().max(1));
        let fresh = self.current_row.is_none();
        let from = self
            .current_row
            .or_else(|| visible.map(|v| v.start))
            .unwrap_or(0);
        let row = target(Step {
            from: from.min(last),
            page,
            last,
            fresh,
        })
        .min(last);
        self.current_row = Some(row);
        if !self.value_menu_open.get() {
            self.hovered_row = Some(row);
        }
        self.lines_scroll
            .scroll_to_item(row, ScrollStrategy::Nearest);
        cx.notify();
    }

    // The rows on screen, from the viewport and row heights of the last layout.
    fn visible_rows(&self) -> Option<Range<usize>> {
        let state = self.lines_scroll.0.borrow();
        let sizes = state.last_item_size?;
        let rows = self.chunked.len();
        if rows == 0 || sizes.contents.height <= Pixels::ZERO {
            return None;
        }
        let row_height = sizes.contents.height / rows as f32;
        let first = (-state.base_handle.offset().y / row_height).floor().max(0.) as usize;
        let count = (sizes.item.height / row_height).ceil() as usize;
        Some(first..first + count)
    }

    // Drops the row matches and voids any scan still running.
    fn clear_row_matches(&mut self) {
        self.void_find_scan();
        self.row_matches.clear();
        self.row_tails.clear();
        self.find_anchor = None;
        self.row_current = None;
    }

    /// Whether find matches case exactly; false by default.
    pub fn find_case_sensitive(&self) -> bool {
        self.find_case_sensitive
    }

    /// Switches exact-case matching and re-runs the query.
    pub fn set_find_case_sensitive(&mut self, on: bool, cx: &mut Context<Self>) {
        self.find_case_sensitive = on;
        self.run_find(cx);
    }

    /// Whether the query is a regular expression; false by default.
    pub fn find_regex(&self) -> bool {
        self.find_regex
    }

    /// Switches regular-expression queries and re-runs the query.
    pub fn set_find_regex(&mut self, on: bool, cx: &mut Context<Self>) {
        self.find_regex = on;
        self.run_find(cx);
    }

    /// Whether find counts only whole words; false by default.
    pub fn find_whole_word(&self) -> bool {
        self.find_whole_word
    }

    /// Switches whole-word matching and re-runs the query.
    pub fn set_find_whole_word(&mut self, on: bool, cx: &mut Context<Self>) {
        self.find_whole_word = on;
        self.run_find(cx);
    }

    /// Whether the query is a regular expression that does not compile.
    pub fn find_invalid(&self) -> bool {
        self.find_invalid
    }

    /// The editor ranges painted as find hits while regex or whole-word is on.
    pub fn find_marks(&self, cx: &App) -> Vec<Range<usize>> {
        self.hit_marks
            .as_ref()
            .map_or_else(Vec::new, |(all, _)| all.get_ranges(cx))
    }

    /// The (other, current) colours the editor-mode hits were last painted in.
    pub fn find_mark_colours(&self) -> Option<(Hsla, Hsla)> {
        self.mark_colours
    }

    /// How many editor-mode regex or whole-word scans have started.
    pub fn editor_find_scans(&self) -> usize {
        self.editor_scans
    }

    /// How many times a body has been parsed for the JSON or XML filter.
    pub fn filter_parses(&self) -> usize {
        self.filter_parses.load(Ordering::Relaxed)
    }

    /// Whether a parsed copy of the body is kept for the filter.
    pub fn holds_parsed_body(&self) -> bool {
        self.json_cache
            .as_ref()
            .is_some_and(|c| c.slot.json.get().is_some() || c.slot.xml.get().is_some())
    }

    /// How many prepare or find jobs ended without a result because a newer one replaced them,
    /// whether dropped before they ran or stopped at a cancel check.
    pub fn cancelled_jobs(&self) -> usize {
        self.cancelled_jobs.load(Ordering::Relaxed)
    }

    // The running find scan's result never lands, and its worker stops at the next line.
    fn void_find_scan(&mut self) {
        self.find_generation += 1;
        self.find_task = None;
        self.find_cancel.store(true, Ordering::Relaxed);
        self.find_cancel = Arc::default();
    }

    // A job's stop question, answered by `flag`, and the guard that counts the job for tests
    // if it ends without a result.
    fn job_signal(&self, flag: &Arc<AtomicBool>) -> (impl Fn() -> bool + Send + 'static, JobGuard) {
        let flag = Arc::clone(flag);
        let guard = JobGuard {
            counted: Arc::clone(&self.cancelled_jobs),
            finished: false,
        };
        (move || flag.load(Ordering::Relaxed), guard)
    }

    fn find_options(&self) -> FindOptions {
        FindOptions {
            case_sensitive: self.find_case_sensitive,
            regex: self.find_regex,
            whole_word: self.find_whole_word,
        }
    }

    // The kit's search matches literal text only, so the editor's hits are found here while
    // either toggle is on.
    fn own_editor_find(&self) -> bool {
        self.find_regex || self.find_whole_word
    }

    // Scans the editor's text off the UI thread; the result lands only if no newer query, toggle
    // or edit came since, and `place` picks the current hit. A pathological pattern can still
    // keep the worker busy, one line per regex call, until it finishes and is dropped.
    fn find_editor_hits(&mut self, place: HitPlace, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value();
        let found = Finder::new(&query, self.find_options());
        self.find_invalid = found.is_err();
        self.void_find_scan();
        let Some(finder) = found.ok().flatten() else {
            self.clear_editor_hits(cx);
            cx.notify();
            return;
        };
        self.editor_scans += 1;
        let kept = self.editor_hit;
        if place != HitPlace::Keep {
            self.editor_hit = None;
        }
        let generation = self.find_generation;
        let text = self.editor.read(cx).value();
        let (stop, guard) = self.job_signal(&self.find_cancel);
        let job = cx.background_spawn(async move {
            guard.finish(finder.find_in_text_until(&text, FIND_HIT_CAP, stop))
        });
        self.find_task = Some(cx.spawn(async move |this, cx| {
            let Some((hits, more)) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.find_generation == generation {
                    this.land_editor_hits(hits, more, place, kept, cx);
                }
            });
        }));
        cx.notify();
    }

    fn land_editor_hits(
        &mut self,
        hits: Vec<Range<usize>>,
        more: bool,
        place: HitPlace,
        kept: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let ix = match place {
            HitPlace::First => Some(0),
            HitPlace::Keep => kept,
            HitPlace::At(at) => Some(hits.partition_point(|r| r.start < at)),
        };
        let last = hits.len().checked_sub(1);
        self.editor_hits = hits;
        self.editor_capped = more;
        self.editor_hit = last.map(|last| ix.unwrap_or(0).min(last));
        self.paint_hits(cx);
        match self.editor_hit.filter(|_| place != HitPlace::Keep) {
            Some(ix) => self.select_hit(ix, cx),
            None => self.mark_current_hit(cx),
        }
        cx.notify();
    }

    // Fills every hit in the theme's faint find colour; the current one goes on top.
    fn paint_hits(&mut self, cx: &mut Context<Self>) {
        let current = find_selection(cx);
        let other = Hsla { s: 0.1, ..current };
        self.mark_colours = Some((other, current));
        let fills = self
            .editor_hits
            .iter()
            .map(|r| fill(r.clone(), other))
            .collect();
        match &self.hit_marks {
            Some((all, _)) => all.set(fills, cx),
            None => {
                let marks = self.editor.update(cx, |e, cx| {
                    (
                        e.create_range_decorations_collection(fills, cx),
                        e.create_range_decorations_collection(Vec::new(), cx),
                    )
                });
                self.hit_marks = Some(marks);
            }
        }
        self.mark_current_hit(cx);
    }

    // Makes hit `ix` current: it is painted over the others, selected and centred.
    fn select_hit(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(hit) = self.editor_hits.get(ix).cloned() else {
            return;
        };
        self.editor_hit = Some(ix);
        self.mark_current_hit(cx);
        self.editor
            .update(cx, |e, cx| e.set_selected_range(hit.clone(), cx));
        *self.centre_hit.borrow_mut() = Some((hit, 1));
        cx.notify();
    }

    fn mark_current_hit(&self, cx: &mut Context<Self>) {
        if let Some((_, current)) = &self.hit_marks {
            let hit = self.editor_hit.and_then(|ix| self.editor_hits.get(ix));
            let fills = hit.map(|r| fill(r.clone(), find_selection(cx)));
            current.set(fills.into_iter().collect(), cx);
        }
    }

    fn clear_editor_hits(&mut self, cx: &mut Context<Self>) {
        self.editor_hits.clear();
        self.editor_hit = None;
        self.editor_capped = false;
        self.centre_hit.borrow_mut().take();
        if let Some((all, current)) = &self.hit_marks {
            all.clear(cx);
            current.clear(cx);
        }
    }

    // Runs the open bar's query over the editor's text. A new query starts at the first
    // match; stepping back then forward lands on it again and scrolls it into view, which
    // the kit offers no other public way to do. An empty query ends the highlights.
    fn run_find(&mut self, cx: &mut Context<Self>) {
        self.find_settle = None;
        if !self.find_open {
            return;
        }
        self.find_ran = self.find_input.read(cx).value();
        let query = self.find_ran.to_string();
        if self.in_rows() {
            // Rows have no editor behind them, so its leftover session must not count.
            self.editor.update(cx, |e, cx| e.close_search(cx));
            self.clear_editor_hits(cx);
            let found = Finder::new(&query, self.find_options());
            self.find_invalid = found.is_err();
            self.find_in_rows(found.ok().flatten(), cx);
            return;
        }
        self.clear_row_matches();
        if self.own_editor_find() {
            self.editor.update(cx, |e, cx| e.close_search(cx));
            let place = self.editor_place.take().unwrap_or(HitPlace::First);
            self.find_editor_hits(place, cx);
            return;
        }
        self.find_invalid = false;
        self.clear_editor_hits(cx);
        let case_insensitive = !self.find_case_sensitive;
        self.editor.update(cx, |e, cx| {
            if query.is_empty() {
                e.close_search(cx);
            } else {
                e.set_search_query(query, case_insensitive, cx);
                e.previous_search_match(cx);
                e.next_search_match(cx);
            }
        });
        cx.notify();
    }

    // A rows-mode or pattern scan can be long, so a typed query waits for a pause, each
    // keystroke restarting the wait. The current match and any running scan go at once so
    // the label never pairs the new query with the old count; the old highlights stay.
    fn find_typed(&mut self, query: SharedString, cx: &mut Context<Self>) {
        let scans = self.in_rows() || self.own_editor_find();
        if !scans || query.is_empty() {
            self.run_find(cx);
            return;
        }
        self.row_current = None;
        self.editor_hit = None;
        self.find_anchor = None;
        self.void_find_scan();
        self.find_invalid = false;
        let timer = cx.background_executor().timer(SETTLE);
        self.find_settle = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.run_find(cx)).ok();
        }));
        cx.notify();
    }

    // Scans the rows off the main thread, since a capped body can hold many of them. The
    // old highlights stay until the result lands, but no current match, so the label never
    // pairs the new query with the old count; an empty or invalid query clears at once.
    fn find_in_rows(&mut self, finder: Option<Finder>, cx: &mut Context<Self>) {
        let Some(finder) = finder else {
            self.clear_row_matches();
            cx.notify();
            return;
        };
        self.row_current = None;
        self.void_find_scan();
        let generation = self.find_generation;
        let chunked = Arc::clone(&self.chunked);
        let (stop, guard) = self.job_signal(&self.find_cancel);
        let job = cx.background_spawn(async move {
            guard.finish(find_in_chunked_until(&chunked, &finder, stop))
        });
        self.find_task = Some(cx.spawn(async move |this, cx| {
            let Some(found) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.find_generation == generation {
                    // After a re-cut the scan resumes at the match nearest the old one and
                    // leaves the view where it is.
                    let anchor = this.find_anchor.take();
                    let start = anchor
                        .and_then(|(line, offset)| this.chunked.locate(line, offset))
                        .map_or(0, |at| {
                            found
                                .matches
                                .partition_point(|(r, range)| (*r, range.start) < at)
                                .min(found.matches.len().saturating_sub(1))
                        });
                    this.row_current = (!found.matches.is_empty()).then_some(start);
                    this.row_matches = found.matches;
                    this.row_tails = found.tails;
                    if anchor.is_none() || std::mem::take(&mut this.reveal_after_cut) {
                        this.scroll_to_row_match();
                    }
                    cx.notify();
                }
            });
        }));
    }

    /// Whether the filter field is showing.
    pub fn filter_open(&self) -> bool {
        self.filter_open
    }

    /// Opens the field and focuses it, or closes it, clears it and restores the body.
    pub fn toggle_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_open {
            self.filter_open = false;
            self.filter_error = None;
            self.applied_filter = None;
            self.filter_no_match = false;
            self.filter_input
                .update(cx, |i, cx| i.set_value("", window, cx));
            self.refresh(window, cx);
        } else {
            self.filter_open = true;
            self.filter_input.update(cx, |i, cx| i.focus(window, cx));
        }
        cx.notify();
    }

    /// Opens the field with `expression` and re-prepares the body through it.
    pub fn set_filter(&mut self, expression: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_open = true;
        // Setting the value emits no change event, so the body refreshes here.
        self.filter_input
            .update(cx, |i, cx| i.set_value(expression.to_owned(), window, cx));
        self.refresh(window, cx);
    }

    /// The last filter error's text, while the expression stays invalid.
    pub fn filter_error(&self) -> Option<&str> {
        self.filter_error.as_deref()
    }

    /// The expression whose output is shown, while one is; `None` for a whole body.
    pub fn applied_filter(&self) -> Option<&str> {
        self.applied_filter.as_deref()
    }

    /// Whether the applied filter matched nothing.
    pub fn filter_no_match(&self) -> bool {
        self.filter_no_match
    }

    /// The filter field.
    pub fn filter_input(&self) -> &Entity<InputState> {
        &self.filter_input
    }

    /// The filter field's placeholder: an XPath example for an XML body, JSONPath otherwise.
    pub fn filter_placeholder(&self) -> &'static str {
        if self.kind == Some(BodyKind::Xml) {
            "//id"
        } else {
            "$..id"
        }
    }

    // The expression the body is filtered through; `None` with the field closed or empty.
    fn filter_expression(&self, cx: &App) -> Option<String> {
        let value = self.filter_input.read(cx).value();
        let value = value.trim();
        (self.filter_open && !value.is_empty()).then(|| value.to_string())
    }

    /// The current mode.
    pub fn mode(&self) -> BodyMode {
        self.mode
    }

    /// What is currently shown.
    pub fn shown(&self) -> Shown {
        self.shown
    }

    /// The detected format of the shown body: "JSON", "HTML", "XML", "Text",
    /// "Binary" or an image format; `None` with no body.
    pub fn format_label(&self) -> Option<&'static str> {
        match self.kind? {
            BodyKind::Empty => None,
            BodyKind::Json => Some("JSON"),
            BodyKind::Html => Some("HTML"),
            BodyKind::Xml => Some("XML"),
            BodyKind::Text => Some("Text"),
            BodyKind::Binary => Some("Binary"),
            BodyKind::Image(image) => Some(image.label()),
        }
    }

    /// Writes exactly the shown text to the clipboard and confirms with a toast: the editor's
    /// value (scratch edits included), a long body's text unchunked, or for a binary body the
    /// Hex rows one per line or the unbroken Base64. Does nothing without a text or binary body.
    pub fn copy(&self, window: &mut Window, cx: &mut App) {
        let text = match self.shown {
            Shown::Binary(_) => match self.binary_mode {
                BinaryMode::Hex => self.chunked.rows.join("\n"),
                BinaryMode::Base64 => self.chunked.rows.concat(),
            },
            _ if !self.is_text() => return,
            Shown::Editor => self.editor.read(cx).value().to_string(),
            Shown::Lines(_) => match self.lines_text() {
                Some(text) => text,
                None => return,
            },
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(ui::info_toast(self.copy_message()), cx);
    }

    /// The node menu's entries for the caret at byte `offset` of the editor's text; empty
    /// unless the body is JSON in the editor with no filter applied and no edit under way.
    pub fn node_copies(&self, offset: usize, cx: &App) -> Vec<NodeCopy> {
        if !self.node_offered() || self.shown != Shown::Editor {
            return Vec::new();
        }
        self.shown_node_entries(&self.editor.read(cx).value(), offset)
    }

    /// The node menu's entries for a click on rows-mode row `row`; empty under the same
    /// conditions as `node_copies`.
    pub fn node_copies_at_row(&self, row: usize) -> Vec<NodeCopy> {
        if !self.node_offered() || !self.in_rows() {
            return Vec::new();
        }
        // The unchunked text is what the offsets index, so rows are matched back into it.
        let Some(text) = self.lines_text() else {
            return Vec::new();
        };
        match row_offset(&text, &self.chunked.rows, row) {
            Some(offset) => self.shown_node_entries(&text, offset),
            None => Vec::new(),
        }
    }

    /// Offers Assert this value in the node menu, for a body whose host owns the request that
    /// receives the assertion.
    pub fn set_assert_offered(&mut self, offered: bool) {
        self.assert_offered = offered;
    }

    /// Copies the entry labelled `label` ("Copy value" or "Copy path") for the node at the
    /// editor's caret, with a toast. Only the editor offers it; in rows mode, for other kinds
    /// and under a filter or edit it does nothing.
    pub fn copy_caret_node(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let caret = self.editor.read(cx).cursor();
        let entries = self.node_copies(caret, cx);
        if let Some(entry) = entries.iter().find(|e| e.label == label) {
            entry.write(window, cx);
        } else if self.node_offered() && self.shown == Shown::Editor {
            let why = if entries.is_empty() {
                "No JSON value at the caret"
            } else {
                "No path for this value"
            };
            window.push_notification(ui::info_toast(why), cx);
        }
    }

    /// Opens the Set as variable dialog for the node at the editor's caret by emitting
    /// [`SetVariable`]. Inert under the same conditions as `copy_caret_node`.
    pub fn set_caret_node_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let caret = self.editor.read(cx).cursor();
        let entries = self.node_copies(caret, cx);
        if let Some(entry) = entries.iter().find(|e| e.variable.is_some()) {
            self.activate_node(entry, window, cx);
        } else if self.node_offered() && self.shown == Shown::Editor {
            window.push_notification(ui::info_toast("No JSON value at the caret"), cx);
        }
    }

    /// Appends an assertion on the node at the editor's caret by emitting [`AssertValue`].
    /// Inert under the same conditions as `copy_caret_node`; a toast says when the node has
    /// nothing to assert.
    pub fn assert_caret_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let caret = self.editor.read(cx).cursor();
        let entries = self.node_copies(caret, cx);
        if let Some(entry) = entries.iter().find(|e| e.label == ASSERT_VALUE) {
            self.activate_node(entry, window, cx);
        } else if self.node_offered() && self.shown == Shown::Editor && self.assert_offered {
            window.push_notification(ui::info_toast("This value cannot be asserted"), cx);
        }
    }

    // Copies the entry labelled `label` from the node menu last opened on the editor.
    fn copy_pending_node(&self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.node_pending.borrow();
        if let Some(entry) = pending.0.iter().find(|e| e.label == label) {
            self.activate_node(entry, window, cx);
        }
    }

    /// Runs a node menu entry: a copy writes the clipboard, Set as variable asks the shell to
    /// open its dialog by emitting [`SetVariable`].
    pub fn activate_node(&self, entry: &NodeCopy, window: &mut Window, cx: &mut Context<Self>) {
        if entry.locked.is_some() {
            return;
        }
        if let Some(assertion) = &entry.assertion {
            self.node_handed_focus.set(true);
            cx.emit(AssertValue(assertion.clone()));
            return;
        }
        match &entry.variable {
            Some(name) => {
                // The dialog restores whatever held focus when it opened, so settle focus on
                // this view first; the closing menu would leave it on itself.
                self.focus_after_value_menu(window, cx);
                cx.emit(SetVariable {
                    name: name.clone(),
                    value: entry.text.clone(),
                    sensitive: entry.sensitive,
                })
            }
            None => entry.write(window, cx),
        }
    }

    // Resolves the entries the value-actions button will list, on the press that opens it:
    // the caret's node in the editor, the hovered row's in rows mode. A full parse happens
    // here, once per open, never per frame or hover.
    fn prime_value_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        *self.node_held.borrow_mut() = menu_focus::held(window, cx);
        self.node_handed_focus.set(false);
        let entries = match self.shown {
            Shown::Editor => self.node_copies(self.editor.read(cx).cursor(), cx),
            Shown::Lines(_) => self
                .hovered_row
                .map(|row| self.node_copies_at_row(row))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        *self.node_pending.borrow_mut() = (entries, false);
    }

    // The y of the value-actions button inside the editor, level with the caret's line, or
    // None when that line is scrolled out of view. Before the first layout it sits at the top.
    fn editor_button_top(&self, rem: Pixels, cx: &App) -> Option<Pixels> {
        let editor = self.editor.read(cx);
        let Some((caret, line)) = editor.cursor_layout() else {
            return Some(style::SPACE_1.to_pixels(rem));
        };
        let view = editor.input_bounds();
        // The caret's bounds are in content space, so the scroll offset puts them on screen.
        let y = caret.origin.y - view.origin.y + editor.scroll_offset().y;
        let inside = y >= Pixels::ZERO && y + line <= view.size.height;
        inside.then(|| y + (line - VALUE_BUTTON_HEIGHT.to_pixels(rem)) / 2.)
    }

    // The y of the value-actions button inside the rows frame, level with the hovered row, or
    // None with no hovered row or when it is scrolled out of view.
    fn rows_button_top(&self, rem: Pixels) -> Option<Pixels> {
        let row = self.hovered_row?;
        let state = self.lines_scroll.0.borrow();
        let sizes = state.last_item_size?;
        let rows = self.chunked.len();
        if rows == 0 || sizes.contents.height <= Pixels::ZERO {
            return None;
        }
        let row_height = sizes.contents.height / rows as f32;
        let y = row_height * row as f32 + state.base_handle.offset().y;
        let inside = y >= Pixels::ZERO && y + row_height <= sizes.item.height;
        inside.then(|| y + (row_height - VALUE_BUTTON_HEIGHT.to_pixels(rem)) / 2.)
    }

    // The one ellipsis button, `top` px down its container's right edge. It never takes focus
    // or moves the caret; its menu lists what the press resolved.
    fn value_button(&self, top: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let (pending, held, open) = (
            Rc::clone(&self.node_pending),
            Rc::clone(&self.node_held),
            Rc::clone(&self.value_menu_open),
        );
        let menu_view = view.clone();
        let button = Button::new(VALUE_ACTIONS)
            .ghost()
            .compact()
            .icon(IconName::Ellipsis)
            .tooltip(VALUE_ACTIONS_TIP)
            .accessibility_label(VALUE_ACTIONS_TIP)
            .tab_stop(false)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                open.set(true);
                let (held, closed, focus_view) =
                    (Rc::clone(&held), Rc::clone(&open), menu_view.clone());
                let release = menu_view.clone();
                menu_focus::watch(
                    cx,
                    window,
                    move |cx| {
                        closed.set(false);
                        // The pointer may have left while the row was frozen; the next move
                        // over a row picks the hover up again.
                        release.update(cx, |v, cx| v.release_hovered_row(cx)).ok();
                        held.borrow_mut().take().into()
                    },
                    move |window, cx| {
                        focus_view
                            .update(cx, |v, cx| {
                                if !v.node_handed_focus.get() {
                                    v.focus_after_value_menu(window, cx)
                                }
                            })
                            .ok();
                    },
                );
                let entries = pending.borrow().0.clone();
                entry_items(menu, entries, &menu_view, cx)
            });
        div()
            .absolute()
            .top(top)
            .right(style::SPACE_5)
            // Captured so no focus change or caret move happens on the press.
            .capture_any_mouse_down(move |e, window, cx| {
                if e.button == MouseButton::Left {
                    window.prevent_default();
                    view.update(cx, |v, cx| v.prime_value_menu(window, cx)).ok();
                }
            })
            .child(button)
            .into_any_element()
    }

    /// The focus handle of the body as a whole, which the rows list and the find chord use.
    pub fn body_focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// The rows-mode row the value-actions button currently follows.
    pub fn hovered_row(&self) -> Option<usize> {
        self.hovered_row
    }

    // Lets go of the hovered row once its menu has closed.
    fn release_hovered_row(&mut self, cx: &mut Context<Self>) {
        self.hovered_row = None;
        cx.notify();
    }

    // Hands focus back to the text the value-actions menu was opened on.
    fn focus_after_value_menu(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.shown {
            Shown::Editor => self.editor.focus_handle(cx).focus(window, cx),
            _ => self.focus.focus(window, cx),
        }
    }

    /// The node entries the last right-press resolved; empty when it hit no node.
    pub fn pending_node_copies(&self) -> Vec<NodeCopy> {
        self.node_pending.borrow().0.clone()
    }

    // Whether a JSON body is on screen exactly as the response sent it: no filter, no edits.
    fn node_offered(&self) -> bool {
        self.kind == Some(BodyKind::Json) && self.applied_filter.is_none() && !self.edit_on()
    }

    /// The toast text Copy shows: "Copied the filtered body" while a filter is applied, else
    /// "Copied the response body".
    pub fn copy_message(&self) -> &'static str {
        match self.applied_filter {
            Some(_) => "Copied the filtered body",
            None => "Copied the response body",
        }
    }

    // The node entries for `offset` of the shown `text`. In a masked body a secret node keeps its
    // masked Copy value, Set as variable carries the real value and says it is sensitive, and
    // Assert is locked, as it would write the secret into a file.
    fn shown_node_entries(&self, text: &str, offset: usize) -> Vec<NodeCopy> {
        let mut entries = node_entries(text, offset, self.assert_offered);
        if !self.masked() {
            return entries;
        }
        let secret =
            apinata_core::json_node_at(text, offset).is_some_and(|n| n.value.contains(MASKED));
        if !secret {
            return entries;
        }
        let real = self.real_node_value(text, offset);
        for entry in &mut entries {
            match entry.label {
                ASSERT_VALUE => entry.locked = Some("The value is secret"),
                SET_VARIABLE => match &real {
                    Some(value) => {
                        entry.text = value.clone();
                        entry.sensitive = true;
                    }
                    None => entry.locked = Some("The real value is not available"),
                },
                _ => {}
            }
        }
        entries
    }

    // The raw body's value for the node at `offset` of the masked shown `text`. Both texts have
    // the same tokens, so the node is found by its place in the sequence.
    fn real_node_value(&self, text: &str, offset: usize) -> Option<String> {
        let raw = self.display_for(self.result.as_ref()?.response.as_ref()?.text())?;
        let (shown, real) = (json_token_starts(text), json_token_starts(&raw));
        if shown.len() != real.len() {
            return None;
        }
        let ix = shown.partition_point(|&s| s <= offset).checked_sub(1)?;
        apinata_core::json_node_at(&raw, real[ix]).map(|n| n.value)
    }

    // A long body's shown text, recomputed from the response rather than rejoined from its
    // chunked rows; the filter is the applied one, whose output is on screen.
    fn lines_text(&self) -> Option<String> {
        self.display_for(self.source_text()?)
    }

    // The text the body shows for the response text `text`, under the current mode and filter.
    fn display_for(&self, text: &str) -> Option<String> {
        let kind = self.kind?;
        // Filter output reads as JSON for JSONPath and as plain text for XPath, always pretty.
        let (kind, out) = match (kind, self.applied_filter.as_deref()) {
            (BodyKind::Json, Some(expr)) => (kind, apinata_core::filter_json(text, expr).ok()?),
            (BodyKind::Xml, Some(expr)) => (
                BodyKind::Text,
                xpath_shown(&apinata_core::filter_xml(text, expr).ok()?).to_owned(),
            ),
            _ => return display_text(kind, text, self.mode == BodyMode::Pretty).map(|(t, _)| t),
        };
        display_text(kind, &out, true).map(|(t, _)| t)
    }

    // The response text the body shows: redacted unless the eye revealed it. A redacted copy
    // not yet built yields nothing, never the raw text.
    fn source_text(&self) -> Option<&str> {
        let raw = self.result.as_ref()?.response.as_ref()?.text();
        match (self.reveal, self.mask.text.get()) {
            (true, _) | (false, Some(None)) => Some(raw),
            (false, Some(Some(masked))) => Some(masked),
            (false, None) => None,
        }
    }

    // The editor's text when the user has changed it from what was prepared.
    fn edited_text(&self, cx: &App) -> Option<String> {
        if self.shown != Shown::Editor {
            return None;
        }
        let value = self.editor.read(cx).value();
        (value != self.prepared_text).then(|| value.to_string())
    }

    fn is_text(&self) -> bool {
        matches!(
            self.kind,
            Some(BodyKind::Json | BodyKind::Html | BodyKind::Xml | BodyKind::Text)
        )
    }

    /// The editor that holds small bodies.
    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    /// The selection colour last laid onto the editor, if it has been drawn.
    pub fn editor_selection(&self) -> Option<Hsla> {
        self.selection_laid.get()
    }

    /// Asks for a path and writes the shown body there (the scratch edits if any, else the
    /// response's raw bytes), then reports the outcome in a toast. Does nothing without a
    /// response.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save_lock().is_some() {
            return;
        }
        let Some(result) = self.result.clone().filter(|r| r.response.is_some()) else {
            return;
        };
        // Chosen now, so a later send or edit cannot change what this save writes.
        let edited = self.edited_text(cx);
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let url = (result.response.as_ref())
            .and_then(|r| r.timeline().last())
            .map(|hop| hop.url.clone());
        let content_type = (result.response.iter())
            .flat_map(|r| r.headers())
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map_or(String::new(), |(_, value)| value.clone());
        let name = save_name(url.as_deref(), &result.name, &content_type);
        let reply = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(path))) = reply.await else {
                return;
            };
            let write = cx.background_spawn(async move {
                let body = match &edited {
                    Some(text) => text.as_bytes(),
                    None => result.response.as_ref().map_or(&[][..], |r| r.body()),
                };
                std::fs::write(&path, body).map(|()| path)
            });
            let note = match write.await {
                Ok(path) => {
                    ui::info_toast(format!("Saved the response body to {}", path.display()))
                }
                Err(e) => Notification::error(format!("Could not save the response body: {e}")),
            };
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
    }

    /// Wires the "Download full response" button to `model`, which re-runs the shown request
    /// of the collection at `root`. `None` removes the button. Keeping the same model only
    /// moves the root, so a download under way keeps reporting.
    pub fn set_download(
        &mut self,
        source: Option<(Entity<DownloadModel>, PathBuf)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (source, self.download.as_mut()) {
            (Some((model, root)), Some(hook)) if hook.model == model => hook.root = root,
            (Some((model, root)), _) => {
                let watch = cx.observe_in(&model, window, |this, model, window, cx| {
                    this.download_changed(&model, window, cx)
                });
                self.download = Some(DownloadHook {
                    model,
                    root,
                    path: None,
                    was_running: false,
                    _watch: watch,
                });
            }
            // A download under way keeps its controls and its end toast without a root.
            (None, Some(hook)) if hook.model.read(cx).is_running() => {}
            (None, _) => self.download = None,
        }
        cx.notify();
    }

    // The method the shown response was sent with; a hand-built response has no hops.
    fn sent_method(&self) -> String {
        (self.result.as_ref())
            .and_then(|r| r.response.as_ref())
            .and_then(|r| r.timeline().last())
            .map_or_else(|| "GET".to_string(), |hop| hop.method.clone())
    }

    // Whether the truncation notice offers the download: it is wired, the body was cut, and the
    // response is not a HEAD's.
    fn download_offered(&self) -> bool {
        self.download.is_some()
            && self.truncated()
            && download_policy(&self.sent_method()) != DownloadPolicy::Hidden
    }

    /// Starts "Download full response": a confirm first for a method that is not safe to repeat,
    /// then the save prompt, then the request is sent again into the chosen file.
    pub fn download_full(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = (self.download.as_ref()).is_some_and(|h| h.model.read(cx).is_running());
        if !self.download_offered() || running {
            return;
        }
        if download_policy(&self.sent_method()) == DownloadPolicy::Direct {
            return self.pick_download_path(window, cx);
        }
        let method = self.sent_method().to_ascii_uppercase();
        let answer = confirm::ask(
            confirm::Ask {
                title: "Send the request again?".into(),
                detail: Some(
                    format!("Downloading the full response sends the {method} request again.")
                        .into(),
                ),
                choices: vec![
                    confirm::Choice {
                        id: "confirm-cancel",
                        label: "Cancel".into(),
                        danger: false,
                    },
                    confirm::Choice {
                        id: "confirm-send",
                        label: "Send again".into(),
                        danger: false,
                    },
                ],
                default: 1,
                cancel: 0,
            },
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update_in(cx, |view, window, cx| view.pick_download_path(window, cx))
                    .ok();
            }
        })
        .detach();
    }

    // Asks where to save, then starts the download; dismissing the prompt starts nothing.
    fn pick_download_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let url = (self.result.as_ref())
            .and_then(|r| r.response.as_ref())
            .and_then(|r| r.timeline().last())
            .map(|hop| hop.url.clone());
        let name = url
            .as_deref()
            .map_or("response.bin".to_string(), download_name);
        let reply = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = reply.await else {
                return;
            };
            this.update_in(cx, |view, window, cx| view.begin_download(path, window, cx))
                .ok();
        })
        .detach();
    }

    fn begin_download(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(result), Some(hook)) = (self.result.clone(), self.download.as_mut()) else {
            return;
        };
        hook.path = Some(path.clone());
        let (model, root) = (hook.model.clone(), hook.root.clone());
        let started = model.update(cx, |m, cx| m.start(&root, result.path.clone(), path, cx));
        // A start that fails at once never runs, so it is reported here.
        if let (false, DownloadState::Failed(msg)) = (started, model.read(cx).state().clone()) {
            window.push_notification(download_failed(&msg), cx);
        }
    }

    /// Stops the download under way; it leaves no file and no error.
    pub fn cancel_download(&mut self, cx: &mut Context<Self>) {
        if let Some(hook) = &self.download {
            hook.model.update(cx, |m, cx| m.cancel(cx));
        }
    }

    // Toasts how a download ended, once; a cancel says nothing.
    fn download_changed(
        &mut self,
        model: &Entity<DownloadModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = model.read(cx).state().clone();
        let running = matches!(state, DownloadState::Running { .. });
        let Some(hook) = self.download.as_mut() else {
            return;
        };
        let ended = hook.was_running && !running;
        hook.was_running = running;
        if ended {
            let path = hook.path.clone().unwrap_or_default();
            match state {
                DownloadState::Done { status, bytes } => window.push_notification(
                    ui::info_toast(format!(
                        "Saved {} ({status}) to {}",
                        format_size(bytes),
                        path.display()
                    )),
                    cx,
                ),
                DownloadState::Failed(msg) => window.push_notification(download_failed(&msg), cx),
                _ => {}
            }
        }
        cx.notify();
    }

    // The notice's download controls: the button, or the progress text and Cancel while it runs.
    fn download_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.download.as_ref()?.model.read(cx).state().clone();
        // A download under way shows its progress whatever response is on screen now.
        if !self.download_offered() && !matches!(state, DownloadState::Running { .. }) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let controls = match state {
            DownloadState::Running { received, total } => {
                let text = match total {
                    Some(total) => format!(
                        "Downloading {} of {}",
                        format_size(received),
                        format_size(total)
                    ),
                    None => format!("Downloading {}", format_size(received)),
                };
                h_flex()
                    .gap(style::SPACE_2)
                    .child(
                        div()
                            .id("body-download-progress")
                            .test_support()
                            .aria_label(text.clone())
                            .text_size(style::TEXT_SMALL)
                            .text_color(muted)
                            .child(text),
                    )
                    .child(
                        Button::new("body-download-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_download(cx)),
                            ),
                    )
            }
            _ => h_flex().child(
                Button::new("body-download-full")
                    .label("Download full response")
                    .small()
                    .tooltip("Sends the request again")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.download_full(window, cx)
                    })),
            ),
        };
        Some(controls.into_any_element())
    }

    /// Hands the whole binary body to the system's default app through `open_external`, or an
    /// HTML or SVG body to the browser as a redacted inert copy, and toasts a write failure.
    /// Does nothing for a truncated body or without a response.
    pub fn open_external(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_open(cx) {
            return;
        }
        let Some(result) = self.result.clone().filter(|r| r.response.is_some()) else {
            return;
        };
        // The body travels to the writer inside the shared result, so the main thread copies
        // nothing. A page is always redacted: writing to disk is not viewing, whatever the eye
        // shows.
        let open = match self.browser_page() {
            Some(BrowserPage::Html) => open_external::open_page(
                move || {
                    let text = result.response.as_ref().map_or("", |r| r.text());
                    open_external::inert_html(&result.redact(text))
                },
                cx,
            ),
            Some(BrowserPage::Svg) => open_external::open_page(
                move || {
                    let body = result.response.as_ref().map_or(&[][..], |r| r.body());
                    let svg = String::from_utf8_lossy(body);
                    open_external::inert_svg_page(result.redact(&svg).as_bytes())
                },
                cx,
            ),
            None => {
                let content_type = (result.response.iter())
                    .flat_map(|r| r.headers())
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                    .map_or(String::new(), |(_, value)| value.clone());
                open_external::open_bytes(ResultBody(result), &content_type, cx)
            }
        };
        cx.spawn_in(window, async move |_, cx| {
            if let Err(e) = open.await {
                let note = Notification::error(format!("Could not open the body: {e}"));
                cx.update(|window, cx| window.push_notification(note, cx))
                    .ok();
            }
        })
        .detach();
    }

    // The slot the shown body's parse lives in: kept while the response and reveal stay, so
    // every keystroke's job shares one parse.
    fn parsed_slot(&mut self) -> ParsedSlots {
        let reveal = self.reveal;
        let cache = match self.json_cache.take() {
            Some(cache) if cache.reveal == reveal => cache,
            _ => JsonCache {
                reveal,
                slot: ParsedSlots::default(),
            },
        };
        let slot = cache.slot.clone();
        self.json_cache = Some(cache);
        slot
    }

    // A long body costs a filter run per keystroke, so typing on one waits for a pause and
    // each keystroke restarts the wait; a short body refilters on every keystroke.
    fn filter_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let len = self
            .result
            .as_ref()
            .and_then(|r| r.response.as_ref())
            .map_or(0, |r| r.body().len());
        if len <= EDITOR_MAX_BYTES {
            self.refresh(window, cx);
            return;
        }
        let timer = cx.background_executor().timer(SETTLE);
        self.filter_settle = Some(cx.spawn_in(window, async move |this, cx| {
            timer.await;
            this.update_in(cx, |this, window, cx| this.refresh(window, cx))
                .ok();
        }));
    }

    // Prepares the body off the main thread. Each call bumps the generation, and a
    // prepare lands only if no newer show or mode change has started since. It also
    // supersedes any filter still waiting for typing to settle.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_settle = None;
        self.generation += 1;
        let generation = self.generation;
        self.job_cancel.store(true, Ordering::Relaxed);
        self.job_cancel = Arc::default();
        let Some(result) = self.result.clone() else {
            self.task = None;
            self.shown = Shown::Empty;
            self.kind = None;
            self.image = None;
            self.applied_filter = None;
            self.filter_no_match = false;
            self.end_find_without_text(window, cx);
            cx.notify();
            return;
        };
        let basis = self.wrap_basis(window, cx);
        let pretty = self.mode == BodyMode::Pretty;
        let binary_mode = self.binary_mode;
        // With no filter to run, the parsed copy is freed rather than kept for a later one.
        let filter = self
            .filter_expression(cx)
            .map(|expr| (expr, self.parsed_slot()));
        if filter.is_none() {
            self.json_cache = None;
        }
        let (stop, guard) = self.job_signal(&self.job_cancel);
        let parses = Arc::clone(&self.filter_parses);
        let (mask, reveal) = (Arc::clone(&self.mask), self.reveal);
        // An invalid filter keeps the shown text only if it already belongs to this result.
        let keep = self.kind.is_some();
        let job = cx.background_spawn(async move {
            let prepare_job = move || {
                // The kind rides beside the prepared text so the format label needs no
                // second scan of the body on the main thread; a binary body's view is
                // formatted here too, never on the main thread. A filter that ran rides along
                // with whether it matched nothing: an empty JSON array or empty XPath output.
                let Some(r) = &result.response else {
                    return Some((
                        BodyKind::Empty,
                        Some(Prepared::Empty),
                        None,
                        Vec::new(),
                        None,
                    ));
                };
                let kind = body_kind(r.headers(), r.body());
                // Only a text body is redacted; the copy is made here, off the main thread, and
                // once per result.
                let text = match kind {
                    BodyKind::Empty | BodyKind::Binary | BodyKind::Image(_) => r.text(),
                    _ if reveal => r.text(),
                    _ => mask.shown(&result, r.text()),
                };
                let whole = || prepare_body(kind, r.body(), text, pretty, r.truncated());
                // JSON runs JSONPath and XML runs XPath; the matches read the same in Pretty and
                // Raw. XPath matches show as plain text so a text match holding `<` is never
                // re-parsed as markup.
                // A replaced filter job stops here, or at the next value or match the filter writes.
                if filter.is_some() && stop() {
                    return None;
                }
                let filtered = match (kind, filter) {
                    (BodyKind::Json, Some((expr, slot))) => {
                        // The expression is checked first, so its error wins over the body's.
                        let out = JsonFilter::compile(&expr).and_then(|compiled| {
                            let parsed = slot.json.get_or_init(|| {
                                parses.fetch_add(1, Ordering::Relaxed);
                                ParsedJson::parse(text)
                            });
                            compiled.run(parsed.as_ref().map_err(Clone::clone)?, &stop)
                        });
                        if out == Err(FilterError::Cancelled) {
                            return None;
                        }
                        out.map(|out| {
                            let prepared = prepare(kind, &out, out.len() as u64, true);
                            (prepared, (expr, out == "[]"))
                        })
                    }
                    (BodyKind::Xml, Some((expr, slot))) => {
                        // A body over the cache limit is parsed for this run only, so its tree
                        // is freed when the run ends.
                        let out = XmlFilter::compile(&expr).and_then(|compiled| {
                            let parse = || {
                                parses.fetch_add(1, Ordering::Relaxed);
                                ParsedXml::parse(text)
                            };
                            if text.len() > XML_PARSE_CACHE_MAX_BYTES {
                                return compiled.run(&parse()?, &stop);
                            }
                            let parsed = slot.xml.get_or_init(parse);
                            compiled.run(parsed.as_ref().map_err(Clone::clone)?, &stop)
                        });
                        if out == Err(FilterError::Cancelled) {
                            return None;
                        }
                        out.map(|out| {
                            let shown = xpath_shown(&out);
                            let prepared = prepare(BodyKind::Text, shown, shown.len() as u64, true);
                            (prepared, (expr, out.is_empty()))
                        })
                    }
                    _ => {
                        let prepared = whole();
                        let binary = match prepared {
                            Prepared::Binary { .. } => binary_rows(r.body(), binary_mode),
                            _ => Vec::new(),
                        };
                        return Some((kind, Some(prepared), None, binary, None));
                    }
                };
                Some(match filtered {
                    Ok((prepared, applied)) => {
                        (kind, Some(prepared), None, Vec::new(), Some(applied))
                    }
                    Err(e) => (
                        kind,
                        (!keep).then(whole),
                        Some(e.to_string()),
                        Vec::new(),
                        None,
                    ),
                })
            };
            guard.finish(prepare_job())
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            // A replaced job lands nothing; the newer one already bumped the generation.
            let Some((kind, mut prepared, error, binary, applied)) = job.await else {
                return;
            };
            // A long body is cut for the pane it lands in, so it never shows at the fixed width
            // first.
            let mut width = ROW_WIDTH;
            if let (Some(basis), Some(Prepared::Lines(rows))) = (basis, prepared.as_mut()) {
                width = basis.columns(rows.line_count());
                if width != ROW_WIDTH {
                    let whole = std::mem::take(rows);
                    *rows = cx
                        .background_executor()
                        .spawn(async move { whole.rechunk(width) })
                        .await;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation == generation {
                    this.kind = Some(kind);
                    let placeholder = this.filter_placeholder();
                    this.filter_input
                        .update(cx, |i, cx| i.set_placeholder(placeholder, window, cx));
                    this.filter_error = error.map(SharedString::from);
                    // Kept text keeps naming the filter it came from; new text names its own.
                    if prepared.is_some() {
                        (this.applied_filter, this.filter_no_match) = match applied {
                            Some((expr, none)) => (Some(expr.into()), none),
                            None => (None, false),
                        };
                    }
                    match prepared {
                        Some(prepared) => this.apply(prepared, binary, width, window, cx),
                        None => cx.notify(),
                    }
                }
            });
        }));
    }

    // Lands a prepared body; `binary` holds a binary body's formatted rows, else nothing.
    fn apply(
        &mut self,
        prepared: Prepared,
        binary: Vec<SharedString>,
        width: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.image = None;
        self.hovered_row = None;
        self.current_row = None;
        *self.node_pending.borrow_mut() = Default::default();
        self.shown = match prepared {
            Prepared::Empty => Shown::Empty,
            Prepared::Binary { size } => {
                self.clear_row_matches();
                self.chunked = Arc::new(Chunked {
                    rows: binary,
                    line_nos: Vec::new(),
                });
                Shown::Binary(size)
            }
            Prepared::Image(info) => {
                let body = (self.result.as_ref())
                    .and_then(|r| r.response.as_ref())
                    .map_or(&[][..], |r| r.body());
                self.image = Some(Arc::new(Image::from_bytes(
                    image_format(info.kind),
                    body.to_vec(),
                )));
                Shown::Image(info)
            }
            Prepared::Lines(rows) => {
                self.editor_backed = false;
                self.install_rows(rows, width);
                Shown::Lines(self.chunked.len())
            }
            Prepared::Editor { text, language } => {
                self.editor.update(cx, |e, cx| {
                    e.set_highlighter(language, cx);
                    e.set_value(text, window, cx);
                });
                self.prepared_text = self.editor.read(cx).value();
                self.editor_backed = true;
                self.editor_cells = self.prepared_text.lines().map(cells).max().unwrap_or(0);
                Shown::Editor
            }
        };
        if !matches!(self.shown, Shown::Editor | Shown::Lines(_)) {
            self.editor_backed = false;
        }
        self.reshape(false, window, cx);
        self.end_find_without_text(window, cx);
        self.run_find(cx);
        cx.notify();
    }

    // Takes `rows` as the shown rows, cut at `width`.
    fn install_rows(&mut self, rows: Chunked, width: usize) {
        self.clear_row_matches();
        self.current_row = None;
        self.reveal_after_cut = false;
        self.widest = rows.widest();
        self.line_count = rows.line_count();
        self.chunk_width = width;
        self.rechunk = None;
        self.rechunk_generation += 1;
        self.chunked = Arc::new(rows);
    }

    // Shows an editor-backed body as rows while it is unwrapped with a line too wide for the
    // editor, and as the editor otherwise; `keep_place` carries the find match across.
    fn reshape(&mut self, keep_place: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor_backed || !matches!(self.shown, Shown::Editor | Shown::Lines(_)) {
            return;
        }
        let advance = f32::from(mono_advance(Self::font_size(cx), window, cx));
        let wide = !self.wrap
            && !self.edit_on()
            && self.editor_cells as f32 * advance > EDITOR_LINE_MAX_WIDTH;
        match (wide, self.shown == Shown::Editor) {
            (true, true) => self.editor_to_rows(keep_place, cx),
            (false, false) => self.rows_to_editor(keep_place, cx),
            _ => {}
        }
    }

    fn editor_to_rows(&mut self, keep_place: bool, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        let text = editor.value();
        let matcher = &editor.search_session().matcher;
        let at = if self.own_editor_find() {
            (self.editor_hit.and_then(|ix| self.editor_hits.get(ix))).map(|r| r.start)
        } else {
            (matcher.current()).and_then(|ix| matcher.matched_ranges().get(ix).map(|r| r.start))
        }
        .filter(|_| keep_place && self.find_open);
        let top = editor.visible_row_range().map(|r| r.start as u32 + 1);
        self.install_rows(chunk_lines(&text, ROW_WIDTH), ROW_WIDTH);
        self.shown = Shown::Lines(self.chunked.len());
        self.find_anchor = at.map(|offset| {
            let before = &text[..offset];
            let line = before.matches('\n').count() as u32 + 1;
            (line, offset - before.rfind('\n').map_or(0, |n| n + 1))
        });
        self.reveal_after_cut = at.is_some();
        let row = top.and_then(|line| self.chunked.line_nos.iter().position(|n| *n == line));
        if let Some(row) = row.filter(|_| at.is_none()) {
            self.lines_scroll
                .scroll_to_item_strict(row, ScrollStrategy::Top);
        }
        self.run_find(cx);
        cx.notify();
    }

    fn rows_to_editor(&mut self, keep_place: bool, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value();
        let at = (self.row_current.and_then(|ix| self.row_matches.get(ix)))
            .filter(|_| keep_place && self.find_open)
            .map(|(row, range)| {
                let (line, offset) = self.chunked.source_pos(*row, range.start);
                let start: usize = text
                    .split_inclusive('\n')
                    .take(line as usize - 1)
                    .map(str::len)
                    .sum();
                start + offset
            });
        let top = self.top_line();
        self.clear_row_matches();
        self.chunked = Arc::default();
        self.shown = Shown::Editor;
        // The own hits land later at the match's offset; without one the view keeps its line.
        self.editor_place = Some(at.map_or(HitPlace::Keep, HitPlace::At));
        self.run_find(cx);
        self.editor_place = None;
        let kit_at = at.filter(|_| !self.own_editor_find());
        self.editor.update(cx, |e, cx| {
            let ranges = e.search_session().matcher.matched_ranges();
            for _ in 0..kit_at.map_or(0, |at| ranges.partition_point(|r| r.start < at)) {
                e.next_search_match(cx);
            }
            let height = e.line_height().filter(|_| at.is_none());
            if let (Some(line), Some(height)) = (top, height) {
                let y = height * line.saturating_sub(1) as f32;
                e.set_scroll_offset(point(Pixels::ZERO, -y), cx);
            }
        });
        self.reveal_editor.set(kit_at.is_some());
        cx.notify();
    }

    // With nothing left to search the find bar goes; focus in its field moves to the body.
    fn end_find_without_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open && !matches!(self.shown, Shown::Editor | Shown::Lines(_)) {
            self.find_open = false;
            self.editor.update(cx, |e, cx| e.close_search(cx));
            self.clear_editor_hits(cx);
            self.clear_row_matches();
            if self.find_input.read(cx).focus_handle(cx).is_focused(window) {
                self.focus.focus(window, cx);
            }
        }
    }

    // The image centred in the area and shrunk to fit it, never enlarged past its own
    // pixels when they are known, with its caption centred below.
    fn image_preview(&self, info: &ImageInfo, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let content = self.image.clone().map(|image| {
            let picture = img(image)
                .object_fit(ObjectFit::Contain)
                .max_w_full()
                .max_h_full()
                .with_fallback(move || {
                    div()
                        .id("body-image-error")
                        .test_support()
                        .text_color(muted)
                        .child("Could not decode the image.")
                        .into_any_element()
                });
            match info.dimensions {
                // Its own pixels at 100%, following the app zoom like every other size.
                Some((w, h)) => picture.w(style::ui(w as f32)).h(style::ui(h as f32)),
                None => picture.size_full(),
            }
        });
        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(style::SPACE_2)
            .child(
                div()
                    .id("body-image")
                    .test_support()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .children(content),
            )
            .child(
                div()
                    .id("body-image-caption")
                    .test_support()
                    .w_full()
                    .text_center()
                    .text_size(style::TEXT_SMALL)
                    .text_color(muted)
                    .child(image_caption(info)),
            )
            .into_any_element()
    }

    // Cuts the long-body rows at the width this frame wants: the column count the pane holds
    // while wrapping, the fixed row width otherwise. The pane width is quantised, so a resize
    // re-cuts rarely, and a big body is re-cut off the main thread.
    fn sync_chunks(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.in_rows() {
            return;
        }
        let target = match (self.wrap, self.wrap_basis(window, cx)) {
            (true, Some(basis)) => basis.columns(self.line_count),
            (true, None) => return,
            (false, _) => ROW_WIDTH,
        };
        if target == self.chunk_width {
            self.rechunk = None;
            return;
        }
        if self
            .rechunk
            .as_ref()
            .is_some_and(|(width, _)| *width == target)
        {
            return;
        }
        self.rechunk_generation += 1;
        let generation = self.rechunk_generation;
        let current = Arc::clone(&self.chunked);
        // A small body is cut here, so its new rows land with no wait on a worker.
        let inline = (current.rows.iter().map(|r| r.len()).sum::<usize>() <= RECHUNK_INLINE_BYTES)
            .then(|| current.rechunk(target));
        let job = cx.background_spawn(async move {
            let cut = inline.unwrap_or_else(|| current.rechunk(target));
            (cut.widest(), cut)
        });
        let top = self.top_line();
        let visible = self.visible_rows();
        self.reveal_after_cut = (self.row_current.and_then(|m| self.row_matches.get(m)))
            .is_some_and(|(row, _)| visible.is_some_and(|v| v.contains(row)));
        let anchor = (self.row_current.and_then(|m| self.row_matches.get(m)))
            .map(|(row, range)| self.chunked.source_pos(*row, range.start));
        let task = cx.spawn(async move |this, cx| {
            let (widest, cut) = job.await;
            let _ = this.update(cx, |this, cx| {
                if this.rechunk_generation != generation {
                    return;
                }
                // The keyboard row is followed through the cut by where its text starts in
                // the source, read now so a key pressed during the cut is not undone.
                let place = this.current_row.map(|row| this.chunked.source_pos(row, 0));
                this.chunk_width = target;
                this.widest = widest;
                this.chunked = Arc::new(cut);
                this.rechunk = None;
                this.clear_row_matches();
                this.find_anchor = anchor;
                this.run_find(cx);
                this.current_row = place
                    .and_then(|(line, at)| this.chunked.locate(line, at))
                    .map(|(row, _)| row);
                if this.hovered_row.is_some() {
                    this.hovered_row = this.current_row;
                }
                if let Some(row) =
                    top.and_then(|line| this.chunked.line_nos.iter().position(|n| *n == line))
                {
                    this.lines_scroll
                        .scroll_to_item_strict(row, ScrollStrategy::Top);
                }
                cx.notify();
            });
        });
        self.rechunk = Some((target, task));
    }

    // What wrapping needs from the pane: the room beside the gutter's padding and the width of
    // a character, or `None` before the body has been painted.
    fn wrap_basis(&self, window: &Window, cx: &App) -> Option<WrapBasis> {
        if !self.wrap {
            return None;
        }
        let bounds = self.painted.get()?;
        let size = Self::font_size(cx);
        // What the frame's border, the scrollbar and the text's own padding take.
        let margin = window.rem_size() * 2.;
        Some(WrapBasis {
            room: (bounds.size.width - margin).max(Pixels::ZERO),
            advance: mono_advance(size, window, cx),
            pad: style::resolve(style::SPACE_2, window),
        })
    }

    /// The row the keyboard last moved to, if any.
    pub fn current_row(&self) -> Option<usize> {
        self.current_row
    }

    /// The rows on screen from the last layout, or None before one.
    pub fn visible_row_range(&self) -> Option<Range<usize>> {
        self.visible_rows()
    }

    /// How far the rows list can scroll in each direction.
    pub fn rows_max_scroll(&self) -> Point<Pixels> {
        self.lines_scroll.0.borrow().base_handle.max_offset()
    }

    /// How far the rows list is scrolled now (zero or negative in each direction).
    pub fn rows_scroll_offset(&self) -> Point<Pixels> {
        self.lines_scroll.0.borrow().base_handle.offset()
    }

    /// The width the rows list shows, or zero before its first layout.
    pub fn rows_viewport_width(&self) -> Pixels {
        let state = self.lines_scroll.0.borrow();
        state.last_item_size.map_or(Pixels::ZERO, |s| s.item.width)
    }

    /// The source line of the row at the top of the rows list, to keep in view across a re-cut.
    pub fn top_line(&self) -> Option<u32> {
        let state = self.lines_scroll.0.borrow();
        // The list reports its viewport as `item`; every row is equally tall, so the content
        // height over the row count is one row.
        let contents = state.last_item_size?.contents;
        let rows = self.chunked.len();
        if rows == 0 || contents.height <= Pixels::ZERO {
            return None;
        }
        let row = (-state.base_handle.offset().y / (contents.height / rows as f32))
            .floor()
            .max(0.) as usize;
        let row = row.min(self.chunked.len().checked_sub(1)?);
        self.chunked.line_nos[..=row]
            .iter()
            .rev()
            .find(|n| **n != 0)
            .copied()
    }

    // The virtualized rows list in its editor-like frame, with find highlights; `numbered`
    // adds the line-number gutter.
    fn rows_list(&self, numbered: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Every row gets the same gutter width, sized for the widest number.
        let count = self.chunked.len();
        let theme = cx.theme();
        let (border, radius) = (theme.input, theme.radius);
        // The kit's own editor background accessor is crate-private; this is its rule.
        let background = theme
            .highlight_theme
            .style
            .editor_background
            .unwrap_or_else(|| theme.input_background());
        let mono = theme.mono_font_family.clone();
        let mono_size = Self::font_size(cx);
        let muted = theme.muted_foreground;
        // Hex rows are data, not code: ligatures would fuse `<=>` in the ASCII column.
        let hex = !numbered && matches!(self.shown(), Shown::Binary(_));
        let hex_features = hex_font_features();
        // The editor's match colours, so both modes highlight alike.
        let (current_bg, other_bg) = match_colours(cx);
        let marker = theme.list_active;
        let lines = if numbered { self.line_count } else { count };
        let gutter = mono_gutter(gutter_digits(lines), mono_size, window, cx);
        // The numbers keep the same gap from the frame's left border as from the text.
        let inset = style::resolve(style::SPACE_2, window);
        // Unwrapped long lines stay whole, so the list scrolls sideways by its widest row.
        let sideways = numbered && !self.wrap;
        let offered = numbered && self.node_offered();
        let list = uniform_list(
            "body-lines",
            count,
            cx.processor(move |this: &mut Self, range: Range<usize>, _, cx| {
                range
                    .map(|ix| {
                        // Matches are in row order, so this row's are one contiguous run.
                        let lo = this.row_matches.partition_point(|(r, _)| *r < ix);
                        let hi = this.row_matches.partition_point(|(r, _)| *r <= ix);
                        let colour = |m: usize| HighlightStyle {
                            background_color: Some(if this.row_current == Some(m) {
                                current_bg
                            } else {
                                other_bg
                            }),
                            ..Default::default()
                        };
                        // A piece carried over from the row above starts the row.
                        let tail_lo = this.row_tails.partition_point(|(_, r, _)| *r < ix);
                        let tail_hi = this.row_tails.partition_point(|(_, r, _)| *r <= ix);
                        let tails = this.row_tails[tail_lo..tail_hi]
                            .iter()
                            .map(|(m, _, range)| (range.clone(), colour(*m)));
                        let highlights = tails
                            .chain((lo..hi).map(|m| (this.row_matches[m].1.clone(), colour(m))));
                        let text = StyledText::new(this.chunked.rows[ix].clone())
                            .with_highlights(highlights);
                        h_flex()
                            .id(("body-line", ix))
                            .test_support()
                            // A left click acts like a caret: the row becomes the keyboard's
                            // current row. `ix` indexes the cut this frame was built from.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    if ix >= this.chunked.len() {
                                        return;
                                    }
                                    this.current_row = Some(ix);
                                    if !this.value_menu_open.get() {
                                        this.hovered_row = Some(ix);
                                    }
                                    cx.notify();
                                }),
                            )
                            // Resolved on the press, so the menu the frame opens next reads it.
                            .when(offered, |row| {
                                row.on_mouse_move(cx.listener(
                                    move |this, _: &MouseMoveEvent, _, cx| {
                                        if this.hovered_row != Some(ix)
                                            && !this.value_menu_open.get()
                                        {
                                            this.hovered_row = Some(ix);
                                            cx.notify();
                                        }
                                    },
                                ))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, _: &MouseDownEvent, _, _| {
                                        let entries = this.node_copies_at_row(ix);
                                        *this.node_pending.borrow_mut() = (entries, false);
                                    }),
                                )
                            })
                            // Full width, so the empty space after a short row still hovers it.
                            .min_w_full()
                            // The keyboard's current row: a tint on the row already drawn.
                            .when(this.current_row == Some(ix), |row| row.bg(marker))
                            .whitespace_nowrap()
                            .font_family(mono.clone())
                            .when(hex, |row| row.font_features(hex_features.clone()))
                            .text_size(mono_size)
                            .children(numbered.then(|| {
                                // A continuation row has no number. Wrapped, it reads as one
                                // flowing line; unwrapped, the cut is forced, so a mark says so.
                                let number = this.chunked.line_nos.get(ix).copied().unwrap_or(0);
                                div()
                                    .id(("line-number", ix))
                                    .test_support()
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .items_center()
                                    .w(gutter)
                                    .ml(inset)
                                    .pr(style::SPACE_2)
                                    .text_color(muted)
                                    .children((number != 0).then(|| number.to_string()))
                                    .when(number == 0 && sideways, |gutter| {
                                        gutter.child(
                                            div().id(("line-continued", ix)).test_support().child(
                                                Icon::new(IconName::CornerDownRight)
                                                    .size(mono_size)
                                                    .text_color(muted),
                                            ),
                                        )
                                    })
                            }))
                            .child(text)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.lines_scroll);
        if sideways {
            self.reveal_match_sideways(inset + gutter, mono_advance(mono_size, window, cx));
        }
        let list = if sideways {
            list.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .with_width_from_item(Some(self.widest))
        } else {
            list
        }
        .test_support()
        .size_full();
        let max = self.rows_max_scroll();
        let (bar_y, bar_x) = (max.y > px(1.), sideways && max.x > px(1.));
        // The frame matches the editor's border so both modes read as one surface.
        let frame = div()
            .id("body-lines-frame")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .border_1()
            .border_color(border)
            .rounded(radius)
            .bg(background)
            .overflow_hidden()
            // The pane's capture handler focuses the pane first; this then moves focus
            // to the body so its find chord reaches the rows.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.focus.focus(window, cx)),
            )
            // The button stays on the last hovered row until the pointer leaves the whole
            // frame, button included, so it can be reached; an open menu freezes it.
            .when(offered, |frame| {
                frame.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered
                        && this.hovered_row != this.current_row
                        && !this.value_menu_open.get()
                    {
                        this.hovered_row = this.current_row;
                        cx.notify();
                    }
                }))
            })
            .child(list)
            .children(
                offered
                    .then(|| self.rows_button_top(window.rem_size()))
                    .flatten()
                    .map(|top| self.value_button(top, cx)),
            )
            // The rows keep a gutter on the right (and below, when they scroll sideways) for
            // the bars, which overlay it. A bar is drawn only while its axis overflows.
            .pr(style::SCROLLBAR_GUTTER)
            .when(sideways, |frame| frame.pb(style::SCROLLBAR_GUTTER))
            .when(bar_y, |frame| {
                frame.child(
                    div()
                        .id("body-lines-scrollbar-y")
                        .test_support()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .w(style::SCROLLBAR_GUTTER)
                        .child(
                            div().absolute().inset_0().child(
                                Scrollbar::vertical(&self.lines_scroll)
                                    .mode(ScrollbarMode::Always)
                                    .viewport_from_layout(),
                            ),
                        ),
                )
            })
            .when(bar_x, |frame| {
                frame.child(
                    div()
                        .id("body-lines-scrollbar-x")
                        .test_support()
                        .absolute()
                        .left_0()
                        .right(style::SCROLLBAR_GUTTER)
                        .bottom_0()
                        .h(style::SCROLLBAR_GUTTER)
                        .child(
                            div().absolute().inset_0().child(
                                Scrollbar::horizontal(&self.lines_scroll)
                                    .mode(ScrollbarMode::Always)
                                    .viewport_from_layout(),
                            ),
                        ),
                )
            })
            // Layout can disagree with what the bars assumed; ask one more frame when it does.
            .child(
                canvas(
                    {
                        let handle = self.lines_scroll.0.borrow().base_handle.clone();
                        move |_, window, _| {
                            let max = handle.max_offset();
                            if (max.y > px(1.), sideways && max.x > px(1.)) != (bar_y, bar_x) {
                                window.request_animation_frame();
                            }
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0(),
            );
        if !offered {
            return frame.into_any_element();
        }
        let (pending, held, view) = (
            Rc::clone(&self.node_pending),
            Rc::clone(&self.node_held),
            cx.entity().downgrade(),
        );
        frame
            .capture_any_mouse_down({
                let (held, pending) = (Rc::clone(&held), Rc::clone(&pending));
                let handed = Rc::clone(&self.node_handed_focus);
                move |e, window, cx| {
                    if e.button == MouseButton::Right {
                        // Cleared first so a press that hits no row offers nothing stale.
                        *pending.borrow_mut() = Default::default();
                        *held.borrow_mut() = menu_focus::held(window, cx);
                        handed.set(false);
                    }
                }
            })
            .context_menu(move |menu, window, cx| {
                let (held, focus_view) = (Rc::clone(&held), view.clone());
                menu_focus::watch(
                    cx,
                    window,
                    move |_| held.borrow_mut().take().into(),
                    move |window, cx| {
                        focus_view
                            .update(cx, |v, cx| {
                                if !v.node_handed_focus.get() {
                                    v.focus.focus(window, cx)
                                }
                            })
                            .ok();
                    },
                );
                let entries = pending.borrow().0.clone();
                entry_items(menu, entries, &view, cx)
            })
            .into_any_element()
    }
}

// The node menu's entries as items that run through `activate_node`, or the disabled
// "Nothing to copy" when the press hit no node.
fn entry_items(
    mut menu: PopupMenu,
    entries: Vec<NodeCopy>,
    view: &WeakEntity<BodyView>,
    cx: &App,
) -> PopupMenu {
    if entries.is_empty() {
        return menu.item(ui::menu_item("Nothing to copy", true, cx));
    }
    for entry in entries {
        let view = view.clone();
        if let Some(reason) = entry.locked {
            // A locked entry stays listed, dimmed, and says why on hover.
            let label = entry.label;
            menu = menu.item(
                PopupMenuItem::element(move |_, cx| {
                    ui::menu_label_cue(div(), cx)
                        .id(label)
                        .test_support()
                        .aria_label(format!("{label} ({reason})"))
                        .tooltip(move |window, cx| Tooltip::new(reason).build(window, cx))
                        .child(label)
                })
                .disabled(true),
            );
            continue;
        }
        menu = menu.item(
            PopupMenuItem::new(entry.label).on_click(move |_, window, cx| {
                view.update(cx, |v, cx| v.activate_node(&entry, window, cx))
                    .ok();
            }),
        );
    }
    menu
}

// The button's height, for centring it on a line.
const VALUE_BUTTON_HEIGHT: Rems = style::ui(24.);

/// The font features of the Hex view's rows: contextual alternates and standard ligatures off.
pub fn hex_font_features() -> FontFeatures {
    FontFeatures(Arc::new(vec![("calt".into(), 0), ("liga".into(), 0)]))
}

impl Render for BodyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_chunks(window, cx);
        // Each pair of modes is one segmented track; a choice updates this view through a
        // weak handle, since the segment callbacks are plain closures.
        let view = cx.entity().downgrade();
        let edit_toggle = {
            let view = view.clone();
            ui::segmented(
                "body-edit-mode",
                ["View", "Edit"],
                usize::from(self.edit_on()),
                move |ix, window, cx| {
                    view.update(cx, |this, cx| this.set_editing(*ix == 1, window, cx))
                        .ok();
                },
            )
            .ids(["body-view", "body-edit"])
            .tooltips([
                ("body-view-tooltip", "Show the response as received"),
                (
                    "body-edit-tooltip",
                    "Edit a copy; the response stays as received",
                ),
            ])
        };
        let binary_toggle = {
            let view = view.clone();
            ui::segmented(
                "body-binary-mode",
                ["Hex", "Base64"],
                usize::from(self.binary_mode == BinaryMode::Base64),
                move |ix, window, cx| {
                    let mode = [BinaryMode::Hex, BinaryMode::Base64][*ix];
                    view.update(cx, |this, cx| this.set_binary_mode(mode, window, cx))
                        .ok();
                },
            )
            .ids(["body-hex", "body-base64"])
        };
        let toggle = ui::segmented(
            "body-text-mode",
            ["Pretty", "Raw"],
            usize::from(self.mode == BodyMode::Raw),
            move |ix, window, cx| {
                let mode = [BodyMode::Pretty, BodyMode::Raw][*ix];
                view.update(cx, |this, cx| this.set_mode(mode, window, cx))
                    .ok();
            },
        )
        .ids(["body-pretty", "body-raw"]);
        // A pane narrower than the bar wraps it onto more lines, still right-aligned.
        let mut bar = h_flex()
            .w_full()
            .flex_wrap()
            .justify_end()
            .gap(style::SPACE_2);
        let filterable = matches!(self.kind, Some(BodyKind::Json | BodyKind::Xml));
        // One eye for the whole body, when redaction hid something in it.
        if self.maskable() {
            let on = self.reveal;
            let (face, icon, words) = if on {
                ("body-hide", IconName::EyeOff, "Hide secrets")
            } else {
                ("body-reveal", IconName::Eye, "Show secrets")
            };
            // One button across the flip, so a keyboard press keeps focus on it.
            let eye = ui::swap_icon_button("body-eye", face, ui::glyph("body-eye", icon), words);
            bar = bar.child(
                eye.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_reveal(!on, window, cx)
                })),
            );
        } else if self.scrubbed && self.is_text() {
            // A kept body lost its secrets when saved, so the eye stays masked and says why.
            let eye = ui::swap_icon_button(
                "body-eye",
                "body-reveal",
                ui::glyph("body-eye", IconName::Eye),
                SCRUBBED_TIP,
            );
            bar = bar.child(ui::LockButton::lock(eye, true, cx));
        }
        // The filter only ever applies to JSON and XML, so another body stays whole and editable.
        let filtered = filterable && self.filter_expression(cx).is_some();
        // Only the editor can take edits; the long-body rows and filtered matches stay
        // view-only, and an open edit hides the filter.
        if filterable && !self.edit_on() {
            bar = bar.child(
                div().id("body-filter").test_support().child(
                    Button::new("body-filter-button")
                        .ghost()
                        .label("Filter")
                        .choose(self.filter_open, SelectedStyle::Ghost, cx)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.toggle_filter(window, cx)
                        })),
                ),
            );
        }
        if self.editable && self.shown == Shown::Editor && !filtered && !self.masked() {
            bar = bar.child(edit_toggle);
        }
        // An image is shown the same either way, so it offers no toggle; a binary body
        // swaps Pretty and Raw for Hex and Base64.
        if matches!(self.shown, Shown::Binary(_)) {
            bar = bar.child(binary_toggle);
        } else if !matches!(self.shown, Shown::Image(_)) {
            bar = bar.child(toggle);
        }
        if self.wrap_offered() {
            bar = bar.child(
                ui::checkbox("body-wrap-toggle", self.wrap)
                    .label("Wrap")
                    .tooltip("Wrap long lines")
                    .on_click(
                        cx.listener(|this, on: &bool, window, cx| this.set_wrap(*on, window, cx)),
                    ),
            );
        }
        if let Some(label) = self.format_label() {
            bar = bar.child(
                div()
                    .id("body-format")
                    .test_support()
                    .text_size(style::TEXT_SMALL)
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            );
        }
        if !self.compact && self.copy_offered() {
            bar = bar.child(
                div()
                    .id("body-copy")
                    .test_support()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        if this.can_copy() {
                            this.copy(window, cx)
                        }
                    }))
                    .child(
                        ui::icon_button(
                            "body-copy-button",
                            ui::glyph("body-copy-button", IconName::Copy),
                            "Copy body",
                            None,
                        )
                        .lock(!self.can_copy(), cx),
                    ),
            );
        }
        // Unlike Copy and Save it stays in a compact pane, as an icon.
        if self.has_body() {
            let lock = self.schema_test_lock();
            let tip = self.schema_test_tooltip();
            let button = if self.compact {
                let icon = ui::glyph("body-schema-test-button", IconName::Check);
                ui::icon_button("body-schema-test-button", icon, tip, None)
            } else {
                Button::new("body-schema-test-button")
                    .ghost()
                    .label("Schema test")
                    .tooltip(tip)
            };
            bar = bar.child(
                div()
                    .id("body-schema-test")
                    .test_support()
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.request_schema_test(cx)),
                    )
                    .child(button.lock(lock.is_some(), cx)),
            );
        }
        if !self.compact && self.has_body() {
            bar = bar.child(
                div()
                    .id("body-save")
                    .test_support()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.save(window, cx)))
                    .child(
                        ui::icon_button(
                            "body-save-button",
                            ui::glyph("body-save-button", IconName::Download),
                            self.save_lock().unwrap_or("Save body"),
                            None,
                        )
                        .lock(self.save_lock().is_some(), cx),
                    ),
            );
        }
        if !self.compact && self.open_offered(cx) {
            // A cut body would open as a broken file, so it is offered but disabled.
            let truncated = self.truncated();
            let page = self.browser_page().is_some();
            let mut open = Button::new("body-open")
                .label(if page {
                    "Open in browser"
                } else {
                    "Open with system app"
                })
                .lock(truncated, cx)
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.open_external(window, cx)),
                );
            if truncated {
                open = open.tooltip("The body was cut at the size limit, so it cannot be opened");
            } else if page {
                open = open.tooltip(OPEN_PAGE_TIP);
            }
            bar = bar.child(open);
        }
        // The field lets an idle Escape propagate, so the bar's wrapper closes it.
        let find_bar = self.find_open.then(|| {
            h_flex()
                .id("body-find")
                .test_support()
                .w_full()
                .gap(style::SPACE_2)
                .on_action(cx.listener(|this, _: &Escape, window, cx| this.close_find(window, cx)))
                .child(ui::invalid_input(
                    Input::new(&self.find_input).bg(ui::field_fill(false, cx)),
                    div().flex_1(),
                    &self.find_input,
                    self.find_invalid,
                    (
                        "body-find-invalid-ring".into(),
                        "body-find-invalid-icon".into(),
                    ),
                    window,
                    cx,
                ))
                .child(
                    div()
                        .id("body-find-count")
                        .test_support()
                        .text_color(if self.find_invalid {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(self.find_label(cx)),
                )
                .child(self.find_toggle(
                    ("body-find-case", "body-find-case-button"),
                    IconName::CaseSensitive,
                    "Match case".into(),
                    self.find_case_sensitive,
                    |this, cx| this.set_find_case_sensitive(!this.find_case_sensitive, cx),
                    cx,
                ))
                .child(self.find_toggle(
                    ("body-find-word", "body-find-word-button"),
                    IconName::WholeWord,
                    ui::label_with_key("Match whole word", &ToggleFindWholeWord, cx),
                    self.find_whole_word,
                    |this, cx| this.set_find_whole_word(!this.find_whole_word, cx),
                    cx,
                ))
                .child(self.find_toggle(
                    ("body-find-regex", "body-find-regex-button"),
                    IconName::Regex,
                    ui::label_with_key("Use regular expression", &ToggleFindRegex, cx),
                    self.find_regex,
                    |this, cx| this.set_find_regex(!this.find_regex, cx),
                    cx,
                ))
                .child(
                    div()
                        .id("body-find-prev")
                        .test_support()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.find_previous(window, cx);
                            this.refocus_find(window, cx);
                        }))
                        .child(ui::icon_button(
                            "body-find-prev-button",
                            IconName::ChevronUp,
                            &ui::label_with_key("Previous match", &FindPreviousInResponse, cx),
                            None,
                        )),
                )
                .child(
                    div()
                        .id("body-find-next")
                        .test_support()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.find_next(window, cx);
                            this.refocus_find(window, cx);
                        }))
                        .child(ui::icon_button(
                            "body-find-next-button",
                            IconName::ChevronDown,
                            &ui::label_with_key("Next match", &FindNextInResponse, cx),
                            None,
                        )),
                )
                .child(
                    div()
                        .id("body-find-close")
                        .test_support()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close_find(window, cx)
                        }))
                        .child(ui::icon_button(
                            "body-find-close-button",
                            IconName::Close,
                            "Close find",
                            None,
                        )),
                )
        });
        let filter_bar = (self.filter_open && filterable && !self.edit_on()).then(|| {
            v_flex()
                .w_full()
                .gap(style::SPACE_1)
                .child(
                    div()
                        .id("body-filter-input")
                        .test_support()
                        .child(ui::invalid_input(
                            Input::new(&self.filter_input).bg(ui::field_fill(false, cx)),
                            div().w_full(),
                            &self.filter_input,
                            self.filter_error.is_some(),
                            (
                                "body-filter-danger-ring".into(),
                                "body-filter-invalid-icon".into(),
                            ),
                            window,
                            cx,
                        )),
                )
                .children(self.filter_error.clone().map(|error| {
                    div()
                        .id("body-filter-error")
                        .test_support()
                        .text_size(style::TEXT_SMALL)
                        .text_color(cx.theme().danger)
                        .child(error)
                }))
        });
        let filter_chip = self.applied_filter.clone().map(|expr| {
            let text = format!("Filtered by {expr}");
            h_flex()
                .id("body-filter-chip")
                .test_support()
                .aria_label(text.clone())
                .gap(style::SPACE_2)
                .child(
                    // The kit's small tag text is below the app's small size, so the text sets
                    // its own; Clear is a link so it reads as an action.
                    Tag::secondary().small().child(
                        div()
                            .id("body-filter-chip-text")
                            .test_support()
                            .text_size(style::TEXT_SMALL)
                            .child(text),
                    ),
                )
                .child(
                    div().id("body-filter-clear").test_support().child(
                        Button::new("body-filter-clear-button")
                            .link()
                            .small()
                            .label("Clear")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.toggle_filter(window, cx)
                            })),
                    ),
                )
        });
        let no_match = (self.filter_no_match)
            .then(|| self.applied_filter.clone())
            .flatten();
        // A cut binary view carries its own note; any other view gets a row to hold the controls.
        let in_binary_note = matches!(self.shown, Shown::Binary(_)) && self.truncated();
        let cut = self.truncated();
        let cut_row = (!in_binary_note)
            .then(|| self.download_controls(cx))
            .flatten()
            .map(|controls| {
                h_flex()
                    .id("body-cut")
                    .test_support()
                    .gap(style::SPACE_2)
                    .items_center()
                    .children(cut.then(|| {
                        div()
                            .text_size(style::TEXT_SMALL)
                            .text_color(cx.theme().muted_foreground)
                            .child("The body was cut at the size limit.")
                    }))
                    .child(controls)
            });
        let content = match self.shown {
            _ if let Some(expr) = no_match => {
                let text = format!("No match for {expr}");
                div()
                    .id("body-filter-no-match")
                    .test_support()
                    .aria_label(text.clone())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .text_color(cx.theme().muted_foreground)
                    .child(text)
                    .into_any_element()
            }
            Shown::Empty => div()
                .id("body-empty")
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .child("No body.")
                .into_any_element(),
            Shown::Binary(size) => {
                let muted = cx.theme().muted_foreground;
                let note = |id: &'static str, text: String| {
                    div()
                        .id(id)
                        .test_support()
                        .aria_label(text.clone())
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child(text)
                };
                let capped = (size > BINARY_VIEW_CAP as u64).then(|| {
                    let text = format!(
                        "Showing the first {} of {}.",
                        format_size(BINARY_VIEW_CAP as u64),
                        format_size(size)
                    );
                    note("body-binary-cap", text)
                });
                let cut = self.truncated().then(|| {
                    h_flex()
                        .gap(style::SPACE_2)
                        .items_center()
                        .child(note(
                            "body-binary-cut",
                            "The body was cut at the size limit.".into(),
                        ))
                        .children(self.download_controls(cx))
                });
                // The offset column stands in for line numbers, so the list has no gutter.
                v_flex()
                    .id("body-binary")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap(style::SPACE_1)
                    .children(capped)
                    .children(cut)
                    .child(self.rows_list(false, window, cx))
                    .into_any_element()
            }
            Shown::Image(info) => self.image_preview(&info, cx),
            Shown::Editor => div()
                .id("body-editor")
                .test_support()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(ui::ringed_editor(
                    // The kit re-applies `readonly` every render, so the switch is set here.
                    Editor::new(&self.editor)
                        .readonly(!self.edit_on())
                        .h_full()
                        .text_size(Self::font_size(cx))
                        .when(self.node_offered(), |editor| {
                            let pending = Rc::clone(&self.node_pending);
                            editor.context_menu(move |menu, _, cx| {
                                let (entries, selected) = &*pending.borrow();
                                node_menu(menu, entries, *selected, cx)
                            })
                        }),
                    &self.editor,
                ))
                // The editor's own press handler has placed the caret by the time this bubbles
                // up, so the node is resolved here; its menu builder cannot read the editor.
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        let state = this.editor.read(cx);
                        let selected = state.selected_text().chars().next().is_some();
                        let entries = this.node_copies(state.cursor(), cx);
                        *this.node_pending.borrow_mut() = (entries, selected);
                    }),
                )
                .on_action(cx.listener(|this, _: &CopyJsonValue, window, cx| {
                    this.copy_pending_node("Copy value", window, cx)
                }))
                .on_action(cx.listener(|this, _: &CopyJsonPath, window, cx| {
                    this.copy_pending_node("Copy path", window, cx)
                }))
                .on_action(cx.listener(|this, _: &SetJsonVariable, window, cx| {
                    this.copy_pending_node(SET_VARIABLE, window, cx)
                }))
                .on_action(cx.listener(|this, _: &AssertJsonValue, window, cx| {
                    this.copy_pending_node(ASSERT_VALUE, window, cx)
                }))
                .child(self.find_colour_marker())
                .children(
                    self.node_offered()
                        .then(|| self.editor_button_top(window.rem_size(), cx))
                        .flatten()
                        .map(|top| self.value_button(top, cx)),
                )
                .into_any_element(),
            Shown::Lines(_) => self.rows_list(true, window, cx),
        };
        let painted = Rc::clone(&self.painted);
        let pending = Rc::clone(&self.scroll_pending);
        v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .gap_2()
            // With no text to search the chord falls through to the shell's own find.
            .on_action(cx.listener(|this, _: &FindInResponse, window, cx| {
                if matches!(this.shown, Shown::Editor | Shown::Lines(_)) {
                    this.open_find(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // Stepping needs an open bar with a query; otherwise the key passes on.
            .on_action(cx.listener(|this, _: &FindNextInResponse, window, cx| {
                if this.can_step_find(cx) {
                    this.find_next(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // Escape from anywhere in the body closes an open bar; a closed bar lets it pass.
            .on_action(cx.listener(|this, _: &CloseFindInResponse, window, cx| {
                if this.find_open {
                    this.close_find(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // The toggle chords act on an open bar only; otherwise the key passes on.
            .on_action(cx.listener(|this, _: &ToggleFindRegex, _, cx| {
                if this.find_open {
                    this.set_find_regex(!this.find_regex, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleFindWholeWord, _, cx| {
                if this.find_open {
                    this.set_find_whole_word(!this.find_whole_word, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &FindPreviousInResponse, window, cx| {
                if this.can_step_find(cx) {
                    this.find_previous(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|this, _: &RowUp, w, cx| this.move_current(|s| s.before(), w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &RowDown, w, cx| this.move_current(|s| s.after(), w, cx)),
            )
            .on_action(cx.listener(|this, _: &RowPageUp, w, cx| {
                this.move_current(|s| s.from.saturating_sub(s.page), w, cx)
            }))
            .on_action(cx.listener(|this, _: &RowPageDown, w, cx| {
                this.move_current(|s| s.from + s.page, w, cx)
            }))
            .on_action(cx.listener(|this, _: &RowFirst, w, cx| this.move_current(|_| 0, w, cx)))
            .on_action(cx.listener(|this, _: &RowLast, w, cx| this.move_current(|s| s.last, w, cx)))
            // The zoom keys reach this root only with focus inside the body, so they size it.
            .on_action(|_: &ZoomIn, _, cx| zoom::apply_step(Response, ZoomStep::In, cx))
            .on_action(|_: &ZoomOut, _, cx| zoom::apply_step(Response, ZoomStep::Out, cx))
            .on_action(|_: &ResetZoom, _, cx| zoom::apply_step(Response, ZoomStep::Reset, cx))
            .child(bar)
            .children(filter_bar)
            .children(filter_chip)
            .children(find_bar)
            .children(cut_row)
            .child(content)
            .child(
                // Painted before the shell's catch-all listener, so a ctrl+scroll over the
                // body sizes the body rather than the app.
                canvas(
                    move |bounds, _, _| painted.set(Some(bounds)),
                    move |bounds, _, window, _| {
                        let inside = move |p| bounds.contains(&p);
                        zoom::paint_scroll_listener(Response, pending, inside, window);
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

// Where a key press starts from: the row it moves off, the rows on screen, the last row, and
// whether no row was current yet (then `from` is the top visible row).
struct Step {
    from: usize,
    page: usize,
    last: usize,
    fresh: bool,
}

impl Step {
    // One up; the first press only marks the top visible row.
    fn before(&self) -> usize {
        if self.fresh {
            self.from
        } else {
            self.from.saturating_sub(1)
        }
    }

    // One down; the first press only marks the top visible row.
    fn after(&self) -> usize {
        if self.fresh { self.from } else { self.from + 1 }
    }
}

// The gpui decoder format for an image kind.
fn image_format(kind: ImageKind) -> ImageFormat {
    match kind {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Gif => ImageFormat::Gif,
        ImageKind::Webp => ImageFormat::Webp,
        ImageKind::Svg => ImageFormat::Svg,
        ImageKind::Bmp => ImageFormat::Bmp,
    }
}

// A copy of the style gpui-component 0.9.0's `Input` projects onto an editor (input.rs, its
// render), with the find selection in place of the theme's; re-check it when gpui-kit moves.
fn editor_style(cx: &App) -> InputEditorStyle {
    let theme = cx.theme();
    // The fold icon is drawn by the kit without an `App`, so its folded look is read here.
    let fold = selected_look(theme, SelectedStyle::Ghost);
    let style = &theme.highlight_theme.style;
    InputEditorStyle {
        foreground: theme.foreground,
        muted_foreground: theme.muted_foreground,
        background: style
            .editor_background
            .unwrap_or_else(|| theme.input_background()),
        border: theme.border,
        selection: find_selection(cx),
        caret: theme.caret,
        diagnostics: DiagnosticColors {
            error: style.status.error(cx),
            warning: style.status.warning(cx),
            info: style.status.info(cx),
            hint: style.status.hint(cx),
        },
        highlight_styles: Arc::<gpui_kit::component::highlighter::HighlightTheme>::clone(
            &theme.highlight_theme,
        ),
        editor_invisible: style.editor_invisible,
        editor_active_line: style.editor_active_line,
        editor_gutter_background: style.editor_gutter_background,
        fold_icon_renderer: Some(Rc::new(move |ix, is_folded| {
            Button::new(("fold-icon", ix))
                .ghost()
                .icon(if is_folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .rounded(ButtonRounded::Small)
                .size(style::FOLD_ICON)
                .chosen(is_folded, fold)
                .into_any_element()
        })),
    }
}

// The wiring of the download button: the model that runs it, the collection it re-runs in, the
// file the current download writes, and whether the last seen state was Running.
struct DownloadHook {
    model: Entity<DownloadModel>,
    root: PathBuf,
    path: Option<PathBuf>,
    was_running: bool,
    _watch: Subscription,
}

fn download_failed(msg: &str) -> Notification {
    Notification::error(format!("Could not download the response: {msg}"))
}

// The save prompt's default name: the URL's last path segment, percent-decoded. An unsafe decode falls back to the raw segment, then to `response.bin`.
fn download_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let after_scheme = path.split_once("://").map_or(path, |(_, rest)| rest);
    let segment = after_scheme.split_once('/').map_or("", |(_, p)| p);
    let last = segment.rsplit('/').next().unwrap_or("");
    if last.is_empty() {
        return "response.bin".to_string();
    }
    // Prefer the decoded name; an unsafe decode falls back to the raw segment.
    let decoded = percent_encoding::percent_decode_str(last).decode_utf8();
    match decoded {
        Ok(name) if safe_file_name(&name) => name.into_owned(),
        _ if safe_file_name(last) => last.to_string(),
        _ => "response.bin".to_string(),
    }
}

// The Save body prompt's default name: the URL's last segment if it has an extension, else
// the request's name (or `response`) plus an extension from the Content-Type, else `.bin`.
fn save_name(url: Option<&str>, request: &str, content_type: &str) -> String {
    let from_url = url
        .map(download_name)
        .filter(|n| n != "response.bin" && n.rfind('.').is_some_and(|i| i > 0 && i + 1 < n.len()));
    if let Some(name) = from_url {
        return name;
    }
    let media = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let ext = if media == "application/json" || media.ends_with("+json") {
        "json"
    } else if media == "application/xml" || media == "text/xml" || media.ends_with("+xml") {
        "xml"
    } else if media == "text/html" {
        "html"
    } else if media == "text/plain" {
        "txt"
    } else {
        open_external::extension(&media)
    };
    format!("{}.{ext}", file_stem(request))
}

// A request name as a file stem: characters no file system takes become `-`, and a name
// left blank or naming a directory falls back to `response`.
fn file_stem(request: &str) -> String {
    let stem: String = (request.trim().chars())
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c => c,
        })
        .collect();
    if safe_file_name(&stem) {
        stem
    } else {
        "response".to_string()
    }
}

// A name that cannot escape the chosen directory or carry control characters.
fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

// The inert browser copy a text-like body opens as.
#[derive(Clone, Copy)]
enum BrowserPage {
    Html,
    Svg,
}

// A response body borrowed from its shared result, for writers that need owned bytes.
struct ResultBody(Arc<RequestResult>);

impl AsRef<[u8]> for ResultBody {
    fn as_ref(&self) -> &[u8] {
        self.0.response.as_ref().map_or(&[][..], |r| r.body())
    }
}

#[cfg(test)]
mod download_name_tests {
    use super::download_name;

    fn name(path: &str) -> String {
        download_name(&format!("https://h.example/{path}"))
    }

    #[test]
    fn decodes_spaces() {
        assert_eq!(name("files/my%20report.zip"), "my report.zip");
    }

    #[test]
    fn decodes_multibyte_utf8() {
        assert_eq!(name("%E2%82%AC.txt"), "\u{20ac}.txt");
    }

    #[test]
    fn decoded_slash_falls_back_to_raw() {
        assert_eq!(name("a%2Fb.txt"), "a%2Fb.txt");
    }

    #[test]
    fn decoded_backslash_falls_back_to_raw() {
        assert_eq!(name("a%5Cb.txt"), "a%5Cb.txt");
    }

    #[test]
    fn decoded_nul_falls_back_to_raw() {
        assert_eq!(name("%00x"), "%00x");
    }

    #[test]
    fn invalid_utf8_falls_back_to_raw() {
        assert_eq!(name("%FF.bin"), "%FF.bin");
    }

    #[test]
    fn decoded_dot_dot_falls_back_to_raw() {
        assert_eq!(name("%2E%2E"), "%2E%2E");
    }

    #[test]
    fn query_and_fragment_are_stripped() {
        assert_eq!(name("d/x%20y.txt?a=1#f"), "x y.txt");
    }

    #[test]
    fn empty_segment_is_response_bin() {
        assert_eq!(name("dir/"), "response.bin");
        assert_eq!(download_name("https://h.example"), "response.bin");
    }
}

#[cfg(test)]
mod save_name_tests {
    use super::save_name;

    const URL: &str = "https://h.example/api/items";

    #[test]
    fn json_body_gets_json() {
        assert_eq!(
            save_name(Some(URL), "", "application/json"),
            "response.json"
        );
        assert_eq!(
            save_name(Some(URL), "", "Application/JSON; charset=utf-8"),
            "response.json"
        );
        assert_eq!(
            save_name(Some(URL), "", "application/vnd.api+json"),
            "response.json"
        );
    }

    #[test]
    fn xml_html_and_text_bodies() {
        assert_eq!(save_name(Some(URL), "", "application/xml"), "response.xml");
        assert_eq!(save_name(Some(URL), "", "text/xml"), "response.xml");
        assert_eq!(
            save_name(Some(URL), "", "application/atom+xml"),
            "response.xml"
        );
        assert_eq!(
            save_name(Some(URL), "", "text/html; charset=x"),
            "response.html"
        );
        assert_eq!(save_name(Some(URL), "", "text/plain"), "response.txt");
    }

    #[test]
    fn known_binary_type_keeps_its_extension() {
        assert_eq!(save_name(Some(URL), "", "image/png"), "response.png");
    }

    #[test]
    fn unknown_or_missing_type_is_bin() {
        assert_eq!(
            save_name(Some(URL), "", "application/x-weird"),
            "response.bin"
        );
        assert_eq!(save_name(Some(URL), "", ""), "response.bin");
        assert_eq!(save_name(None, "", "application/json"), "response.json");
    }

    #[test]
    fn url_segment_with_extension_wins() {
        assert_eq!(
            save_name(
                Some("https://h.example/files/report.pdf?x=1"),
                "",
                "application/json"
            ),
            "report.pdf"
        );
    }

    #[test]
    fn dotfile_and_trailing_dot_segments_have_no_extension() {
        assert_eq!(
            save_name(Some("https://h.example/.env"), "", "text/plain"),
            "response.txt"
        );
        assert_eq!(
            save_name(Some("https://h.example/file."), "", "application/json"),
            "response.json"
        );
        assert_eq!(
            save_name(Some("https://h.example/dir/"), "", "application/x-weird"),
            "response.bin"
        );
    }

    #[test]
    fn the_request_name_is_the_stem() {
        assert_eq!(
            save_name(Some(URL), "Get items", "application/json"),
            "Get items.json"
        );
        assert_eq!(save_name(None, "Ping", ""), "Ping.bin");
        assert_eq!(
            save_name(
                Some("https://h.example/files/report.pdf"),
                "Get report",
                "application/pdf"
            ),
            "report.pdf"
        );
    }

    #[test]
    fn a_request_name_that_is_no_file_name_is_cleaned_or_dropped() {
        assert_eq!(
            save_name(Some(URL), "  a/b: c?  ", "text/plain"),
            "a-b- c-.txt"
        );
        assert_eq!(save_name(Some(URL), "..", "text/plain"), "response.txt");
        assert_eq!(save_name(Some(URL), " \t ", "text/plain"), "response.txt");
    }

    #[test]
    fn url_segment_without_extension_uses_the_type() {
        assert_eq!(
            save_name(
                Some("https://h.example/files/report"),
                "",
                "application/json"
            ),
            "response.json"
        );
        assert_eq!(
            save_name(Some("https://h.example/"), "", "text/plain"),
            "response.txt"
        );
    }
}
