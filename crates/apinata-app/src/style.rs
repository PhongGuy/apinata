//! Spacing and size constants: every size the views use, so no view holds a literal.

use gpui_kit::component::Size;
use gpui_kit::{Pixels, Rems, Window, px, rems};

use crate::zoom;

// Layout sizes are rems at the 13 px UI base, so the kit's per-frame rem (13 px times the
// zoom) scales spacing exactly like text; hairlines, radii and `PANE_MIN` stay px.

/// `px` at the 100% UI size as rems, so the size follows the app zoom.
pub const fn ui(px: f32) -> Rems {
    rems(px / zoom::UI_FONT_SIZE)
}

/// `len` in pixels at `window`'s current rem size.
pub fn resolve(len: Rems, window: &Window) -> Pixels {
    snap(len.to_pixels(window.rem_size()))
}

/// `len` in pixels at `zoom` (clamped like the zoom itself); for code without a window.
pub fn at_zoom(len: Rems, zoom: f32) -> Pixels {
    snap(len.to_pixels(zoom::ui_font_size(zoom)))
}

// A px size stored as rems comes back with f32 noise (28 / 13 * 13 is 28.000002); a 1/64 px
// grid is exact in f32 and removes it, so 100% gives back the px value the layout rounds to.
pub(crate) fn snap(len: Pixels) -> Pixels {
    px((f32::from(len) * 64.).round() / 64.)
}

/// The kit `Size` for tab strips at `zoom`. Kit tab heights are fixed px per `Size`, so
/// the strip steps around the default (Medium at 100%) to keep its rem-sized labels inside.
pub fn kit_size(zoom: f32) -> Size {
    match zoom {
        z if z < 0.75 => Size::XSmall,
        z if z < 1.25 => Size::Medium,
        _ => Size::Large,
    }
}

/// Line height of tab labels, a little under the kit's 1.25 so a Large tab's text_base line
/// fits its 32 px clipped inner box at 200%.
pub const TAB_LINE_HEIGHT: f32 = 1.2;
/// Tightest padding: the vertical inset of a compact label such as the drag preview.
pub const SPACE_0: Rems = ui(2.);
/// Smallest gap: icon to label, inline padding.
pub const SPACE_1: Rems = ui(4.);
/// Default gap between related controls.
pub const SPACE_2: Rems = ui(8.);
/// Padding inside panels and rows.
pub const SPACE_3: Rems = ui(12.);
/// Gap between sections.
pub const SPACE_4: Rems = ui(16.);
/// Gap around large blocks such as empty states.
pub const SPACE_5: Rems = ui(24.);
/// Height of a list or table row.
pub const ROW_HEIGHT: Rems = ui(28.);
/// The kit's Medium input height, so a form label lines up with its field.
pub const INPUT_HEIGHT: Rems = rems(2.);
/// Width of a header table's name column; in a read-only table, the most it takes.
pub const HEADER_NAME_WIDTH: Rems = ui(220.);
/// The part of a narrow read-only header table's row its name column takes, so a cut name
/// (shown whole on hover) gives way before the value wraps.
pub const HEADER_NAME_SHARE: f32 = 1. / 3.;
/// Width of the wide fixed columns (Name, Domain) in the cookie table; the value column
/// takes the rest.
pub const COOKIE_COL_WIDTH: Rems = ui(120.);
/// The cookie jar row's name column, on its first line, so every value starts at one x.
pub const JAR_NAME_WIDTH: Rems = ui(120.);
/// The cookie jar row's path column, first on its second line.
pub const JAR_PATH_WIDTH: Rems = ui(80.);
/// The cookie jar row's expiry column, after the path, so every row's flags start at one x.
pub const JAR_EXPIRY_WIDTH: Rems = ui(80.);
/// Width of the cookie table's fixed columns, in order: Name, Domain, Path, Expires,
/// HttpOnly, Secure. Narrow ones keep the table inside a common pane width.
pub const COOKIE_FIXED_WIDTHS: [Rems; 6] =
    [ui(120.), ui(120.), ui(80.), ui(100.), ui(80.), ui(72.)];
/// The cookie table's least width: the 572 px of fixed columns, a 160 px Value, eight 8 px
/// gaps, the 8 px right gutter and two 28 px action slots. A narrower pane scrolls the table
/// sideways.
pub const COOKIE_TABLE_MIN_WIDTH: Rems = ui(864.);
/// The This response cookie table's fixed column widths: Name, Hop and Result. Result fits
/// its longest phrase ("Domain does not match host") whole at the default zoom.
pub const SET_COOKIE_FIXED_WIDTHS: [Rems; 3] = [ui(120.), ui(240.), ui(216.)];
/// The This response table's Value width while no shown value is revealed: the masked text
/// "[redacted]" whole at the default zoom beside its 28 px eye.
pub const SET_COOKIE_MASKED_VALUE_WIDTH: Rems = ui(120.);
/// The This response table's least width: the 576 px of fixed columns, a 160 px Value with its
/// 28 px eye, four 8 px gaps, the 8 px right gutter and the 28 px copy slot. A narrower pane
/// scrolls the table sideways.
pub const SET_COOKIE_TABLE_MIN_WIDTH: Rems = ui(832.);
/// Width of the slot that holds Send, or the spinner and Cancel while sending; Run is at
/// least this wide.
pub const SEND_SLOT_WIDTH: Rems = ui(96.);
/// Widest a History dropdown row grows before its target truncates.
pub const HISTORY_ROW_WIDTH: Rems = ui(480.);
/// The "Previous runs" menu's widest: a full row plus the menu's padding and check column.
pub const HISTORY_MENU_WIDTH: Rems = ui(528.);
/// The History palette's status and age column: room for "404 · 11 months ago".
pub const HISTORY_DETAIL_WIDTH: Rems = ui(144.);
/// The History palette's tag column: room for the widest origin tag.
pub const HISTORY_TAG_WIDTH: Rems = ui(56.);

/// The column of a search hit's field label.
pub const SEARCH_FIELD_WIDTH: Rems = ui(120.);
/// Width of a form's label column; longer labels wrap.
pub const FORM_LABEL_WIDTH: Rems = ui(200.);
/// Below this row width a form row puts its label above its control.
pub const FORM_STACK_WIDTH: Rems = ui(520.);
/// Line height of a form label: 18 px at the 13 px UI size, so a label line plus `SPACE_1`
/// above and below is exactly `INPUT_HEIGHT`.
pub const FORM_LABEL_LINE_HEIGHT: f32 = 18. / 13.;
/// A short form label column, for a form whose labels are single words.
pub const SHORT_FORM_LABEL_WIDTH: Rems = ui(80.);
/// The Import dialog's label column, wide enough for "Link or file" on one line.
pub const IMPORT_FORM_LABEL_WIDTH: Rems = ui(100.);
/// Width shared by a form's text inputs and selects.
pub const FORM_INPUT_WIDTH: Rems = ui(160.);
/// Body width of a dialog that sets no width of its own: the kit's 448 px default less its
/// 16 px side paddings, which stay px.
pub const DIALOG_BODY_WIDTH: Rems = ui(416.);
/// The Generate Code dialog's body width, so a typical URL fits on one code line.
pub const CODE_DIALOG_WIDTH: Rems = ui(720.);
/// The Generate Code preview's height, before the window's room caps it.
pub const CODE_PREVIEW_HEIGHT: Rems = ui(320.);
/// The kit dialog's px padding on each edge; it does not follow the zoom.
pub const DIALOG_PADDING: Pixels = px(16.);
/// How far inside the dialog panel's top and right edges the close button sits, as the kit's.
pub const DIALOG_CLOSE_INSET: Pixels = px(8.);
/// The kit dialog's px side paddings, 16 on each side; they do not follow the zoom.
pub const DIALOG_SIDE_PADDINGS: Pixels = px(32.);
/// The kit dialog's px height around its content: 16 px paddings above and below and a
/// 1 px border.
pub const DIALOG_BOX_CHROME: Pixels = px(34.);
/// Width of the Preferences section list, including its gap to the rows.
pub const PREFS_NAV_WIDTH: Rems = ui(136.);
/// Width of the Preferences dialog: the section list beside a 600 px rows column.
pub const PREFS_DIALOG_WIDTH: Rems = ui(736.);
/// Height of the Preferences dialog, kept while the search filters rows out; at 100% every
/// row fits without scrolling.
pub const PREFS_DIALOG_HEIGHT: Rems = ui(676.);
/// Below this title row width the Preferences search drops its shortcut hint.
pub const PREFS_SEARCH_HINT_ROOM: Rems = ui(320.);
/// The Preferences dialog's height around its body at 50%, which the kit's px paddings keep
/// from shrinking with the zoom.
pub const PREFS_DIALOG_CHROME: Pixels = px(55.);
/// Below this much window room Preferences drops its rows' top padding and empty status line.
pub const PREFS_SHORT_ROOM: Rems = ui(320.);
/// Height of the window's title bar.
pub const TITLE_BAR_HEIGHT: Rems = ui(36.);
/// Height of the status bar.
pub const STATUS_BAR_HEIGHT: Rems = ui(24.);
/// The response timing bar: wide enough to read the proportions, thin enough to sit in a text row.
pub const TIMING_BAR_WIDTH: Pixels = px(72.);
/// Height of the response timing bar.
pub const TIMING_BAR_HEIGHT: Pixels = px(4.);

/// Corner radius of controls; matches the theme's `radius`.
pub const RADIUS: Pixels = px(4.);

/// Room for the kit's keyboard ring, a fixed 3 px outside a button at every zoom.
pub const FOCUS_RING_ROOM: Pixels = px(3.);

/// How far a keyboard list in Preferences reaches past its rows on each side: its ring's room
/// and the highlight's padding, so the rows line up with every other section.
pub fn list_gutter(window: &Window) -> Pixels {
    FOCUS_RING_ROOM + resolve(SPACE_1, window)
}

/// Where every tab's content and the first tab start from the pane's left and top: the pane
/// inset plus the ring room the request and response panes keep inside their clipping edge.
pub fn tab_content_inset(window: &Window) -> Pixels {
    resolve(PANE_INSET, window) + FOCUS_RING_ROOM
}
/// Room for a name line and three lines of data, so the Events rows are uniform.
pub const EVENT_ROW_HEIGHT: Rems = ui(86.);
/// The kit editor's fold chevron button, at the fixed size the kit draws it.
pub const FOLD_ICON: Pixels = px(14.);
/// Right padding that keeps scrolled rows clear of the kit's vertical scrollbar, which is
/// 16 px wide at any zoom.
pub const SCROLLBAR_GUTTER: Pixels = px(16.);
/// Height of the fade over a scroll area's bottom edge while more rows are below.
pub const SCROLL_FADE_HEIGHT: Rems = ui(32.);
/// Width of the fade over a sideways strip's edge while items lie beyond it.
pub const STRIP_FADE_WIDTH: Rems = ui(24.);
/// What a tab strip's overflow menu button takes beside the tabs while they overflow.
pub const TAB_OVERFLOW_WIDTH: Rems = ui(32.);
/// The widest a comparison's source menu grows before its label truncates.
pub const COMPARE_SOURCE_MAX_WIDTH: Rems = ui(240.);
/// Side of a checkbox's box; the kit's medium indicator.
pub const CHECKBOX_SIZE: Rems = rems(1.);
/// Opacity of a disabled control that has no muted colour of its own.
pub const DISABLED_OPACITY: f32 = 0.5;
/// Corner radius of popovers and menus; matches the theme's `radius.lg`.
pub const POPOVER_RADIUS: Pixels = px(6.);
// Text sizes are rems of the 13 px theme `font.size`, so the app zoom scales them.
/// Body text size; matches the theme's `font.size`.
pub const TEXT_UI: Rems = rems(1.);
/// Width of the recent collections list in the no-collection state.
pub const RECENT_LIST_WIDTH: Rems = ui(360.);
/// Secondary text size for hints, badges and status text.
pub const TEXT_SMALL: Rems = rems(12. / 13.);
/// Text beside a default-size kit button; matches the kit's `text_sm` button label.
pub const TEXT_CONTROL: Rems = rems(0.875);
/// A dialog title.
pub const TEXT_TITLE: Rems = rems(16. / 13.);
/// Width of the method tag column in the sidebar and tabs.
pub const METHOD_COLUMN: Rems = ui(32.);
/// Diameter of the unsaved-changes dot.
pub const DOT: Rems = ui(6.);
/// Height of the active environment's colour stripe along the window's top edge.
pub const ENV_STRIPE_HEIGHT: Rems = ui(2.);
/// Width of the title bar's environment control.
pub const ENV_CONTROL_WIDTH: Rems = ui(180.);
/// The narrowest a title-bar environment select shrinks to, so 800 px at 200% fits.
pub const ENV_CONTROL_MIN: Rems = ui(96.);
/// Squeezed narrower than this, a title-bar environment select is too small to read, so it
/// hides and leaves the Tab order.
pub const ENV_CONTROL_SHOWN_FROM: Rems = ui(40.);
/// Below this window width the title bar's environment selects shrink past their minimum.
pub const TITLE_BAR_CRAMPED_BELOW: Rems = ui(320.);
/// Below this window width the global select has no usable room left, so the title bar shows
/// a button to its dialog in its place.
pub const TITLE_GLOBAL_BUTTON_BELOW: Rems = ui(240.);
/// Below this window width the title bar drops its variables button; the palette still lists it.
pub const VARIABLES_BUTTON_FROM: Rems = ui(480.);
/// The narrowest the collection name button gets in a cramped title bar, before the selects
/// give way: its caret and a few characters.
pub const TITLE_NAME_FLOOR: Rems = ui(48.);
/// The method select's fixed width.
pub const METHOD_SELECT: Rems = ui(96.);
/// The custom method field, which opens in the method select's place.
pub const CUSTOM_METHOD: Rems = ui(104.);
/// The URL input's floor: 16 rem. A side-by-side request pane too narrow to hold it beside the
/// method and the buttons gives way to the stacked layout.
pub const URL_FIELD_MIN: Rems = rems(16.);
/// What Save, Send and Generate code take side by side, with the gap between them.
pub const REQUEST_BUTTONS_WIDTH: Rems = ui(190.);
/// The method select's and the buttons' gaps, the focus-ring inset and the pane gutter around
/// the URL row.
pub const URL_ROW_SLACK: Rems = ui(28.);
/// The narrowest the response pane stays wide; it folds sooner when its header's floor (the
/// facts with the address cut, beside the tab strip's floor) is wider.
pub const RESPONSE_COMPACT_BELOW: Rems = ui(600.);
/// The widest the response's address grows; `proxy 10.0.0.1:3128` fits uncut.
pub const RESPONSE_ADDRESS_MAX: Rems = ui(160.);
/// The width of the collapsed response column shown side by side before a tab first sends.
pub const RESPONSE_STRIP_WIDTH: Rems = ui(160.);
/// The smallest height (or width) the request or the response panel can be dragged to.
pub const PANE_MIN: Pixels = px(120.);
/// The ratio an untouched request/response split starts at.
pub const DEFAULT_SPLIT_RATIO: f32 = 0.5;
/// The request pane height an untouched split reserves on a short room: toolbar, section bar
/// and the three NTLM rows.
pub const REQUEST_PANE_NEEDED: Rems = ui(232.);
/// The response height a stacked request pane's row minimum leaves before it gives way; px,
/// so it does not scale with zoom.
pub const RESPONSE_KEPT: Pixels = px(240.);
/// The response height an untouched split keeps when it favours the request pane.
pub const RESPONSE_FLOOR: Pixels = px(72.);

/// The response pane height the Events tab asks for: its header and two full rows.
pub const EVENTS_PANE_WANTED: Pixels = px(260.);

/// The response pane height a started comparison asks for, so its list is usable.
pub const COMPARE_PANE_WANTED: Pixels = px(220.);
/// The least height of the request section body: a padded key-value heading, one row and the
/// add row, with slack for the heading, which does not scale exactly with zoom.
pub const SECTION_BODY_MIN: Rems = ui(96.);
/// The command palette dialog's width.
pub const PALETTE_WIDTH: Rems = ui(560.);
/// The widest a tab's label grows before its name is cut with an ellipsis.
pub const TAB_MAX_WIDTH: Rems = ui(220.);
/// The widest a select button grows before its value is cut with an ellipsis.
pub const SELECT_MAX_WIDTH: Rems = ui(240.);
/// The select button's chevron.
pub const SELECT_CHEVRON: Rems = ui(14.);
/// Inset of the tab strip and the request editor from their pane's edges.
pub const PANE_INSET: Rems = SPACE_3;
/// Space between a dialog's body and its button row.
pub const DIALOG_FOOTER_GAP: Rems = SPACE_3;
/// Width of a text input that holds a long value: an auth field or a host.
pub const WIDE_INPUT_WIDTH: Rems = ui(320.);
/// Width of an input that holds a short value, such as a color.
pub const SHORT_INPUT_WIDTH: Rems = ui(120.);
/// Narrowest the runner table's Name column gets; a narrower pane scrolls the table sideways.
pub const RUN_NAME_MIN_WIDTH: Rems = ui(120.);
/// Width of the runner table's Result column, wide enough for "Retrying, attempt 2 of 3".
pub const RUN_RESULT_WIDTH: Rems = ui(200.);
/// The runner table's least width: three short columns, the Result column, the Name minimum,
/// gaps and row padding.
pub const RUN_TABLE_MIN_WIDTH: Rems = ui(3. * 80. + 200. + 120. + 4. * 8. + 2. * 12.);
/// The runner options' label column, wide enough for "Retry delay (ms)".
pub const RUN_OPTION_LABEL_WIDTH: Rems = ui(120.);
/// Most the runner's results or plan list keep when the pane is short; the options give way
/// only after the list has shrunk to this.
pub const RUN_LIST_FLOOR: Rems = ui(120.);
/// A deliberately low per-row height for that floor, so a short list is never padded.
pub const RUN_ROW_FLOOR: Rems = ui(16.);
/// How much harder the results or plan list shrinks than the options above it.
pub const RUN_LIST_SHRINK: f32 = 1e4;
/// Width of a section's mode select (auth mode, body mode, API key placement).
pub const MODE_SELECT_WIDTH: Rems = ui(200.);
/// Width of a key-value row's tag column (`query`, `path`), the same for every tag.
pub const TAG_WIDTH: Rems = ui(44.);
/// Width of a key-value table's checkbox column.
pub const KV_CHECK_WIDTH: Rems = ui(20.);
/// Minimum height of a breadcrumb folder link, its hit target.
pub const CRUMB_MIN_HEIGHT: Rems = ui(20.);
/// Width of the drag-handle slot at the left of a table row.
pub const KV_GRIP_WIDTH: Rems = ui(12.);
/// Width of a key-value table's trailing remove or bulk control.
pub const KV_ACTION_WIDTH: Rems = ROW_HEIGHT;
/// Height of an edit grid's heading band: the bulk toggle's 2 rem plus `SPACE_0` above and below.
pub const KV_HEAD_HEIGHT: Rems = Rems(INPUT_HEIGHT.0 + 2. * SPACE_0.0);
/// Line height of a read-only key-value view's text.
pub const KV_VIEW_LINE_HEIGHT: Rems = ui(20.);
/// Vertical padding of a read-only key-value cell, centring its first line in `ROW_HEIGHT`.
pub const KV_VIEW_PAD_Y: Rems = Rems((ROW_HEIGHT.0 - KV_VIEW_LINE_HEIGHT.0) / 2.);
/// Width of an assertion row's operator select.
pub const OPERATOR_SELECT_WIDTH: Rems = ui(180.);
/// The parts of an assertion row's free width its value takes to the expression's one.
pub const VALUE_GROW: f32 = 1.5;
/// Width of the operator list, room for the plain words and the raw operator beside them.
pub const OPERATOR_MENU_WIDTH: Rems = ui(260.);
/// Width of an oauth2 parameter row's stage select.
pub const PARAM_STAGE_WIDTH: Rems = ui(140.);
/// Width of an oauth2 parameter row's placement select.
pub const PARAM_PLACEMENT_WIDTH: Rems = ui(100.);
/// The least width of an environment variable's name input and of its value input.
pub const ENV_CELL_MIN: Rems = ui(90.);
/// The least width of the editor pane: two minimum cells plus the table's drag handle, checkbox, secret
/// toggle, delete button and gaps. The list shrinks to make room for it.
pub const ENV_PANE_MIN: Rems = ui(310.);
/// Width of the global environment dialog's list of environments.
pub const ENV_LIST_WIDTH: Rems = ui(180.);
/// Width of the environment editor's list column.
pub const ENV_EDITOR_LIST_WIDTH: Rems = ui(200.);
/// The least width the editor's list shrinks to when the pane needs room; low enough that
/// a 420 px window still fits the variable table's minimum beside it after the tab content
/// inset.
pub const ENV_EDITOR_LIST_MIN: Rems = ui(77.);
/// The global environment editor dialog's width.
pub const GLOBAL_ENV_DIALOG_WIDTH: Rems = ui(720.);
/// The least height of the global environment editor's two-pane row.
pub const GLOBAL_ENV_BODY_MIN: Rems = ui(200.);
/// The Changes dialog's width: the file list beside the diff pane.
pub const CHANGES_DIALOG_WIDTH: Rems = ui(760.);
/// The Changes dialog's panes' height, before the window's room caps it.
pub const CHANGES_DIALOG_HEIGHT: Rems = ui(440.);
/// Width of the Changes dialog's file list.
pub const CHANGES_FILES_WIDTH: Rems = ui(220.);
/// Below this width the Changes dialog stacks its file list over the diff pane.
pub const CHANGES_STACK_WIDTH: Rems = ui(520.);
/// Width of the status word column in the Changes dialog's file list, room for "conflicted".
pub const CHANGES_STATUS_WIDTH: Rems = ui(72.);
/// The tallest the Changes dialog's commit failure text grows before it scrolls.
pub const CHANGES_NOTICE_MAX_HEIGHT: Rems = ui(120.);
/// The smallest height of the Compare dialog's diff list.
pub const COMPARE_DIFF_MIN_HEIGHT: Rems = ui(120.);
/// The smallest height of a code editor; above it the request section's editors fill the pane.
pub const EDITOR_MIN_HEIGHT: Rems = ui(200.);
/// The smallest height of a script or tests editor. Its phase switch, snippet row and Wrap row
/// stand taller than the body's mode row, so a lower floor keeps the section fitting wherever
/// the body does.
pub const SCRIPT_EDITOR_MIN_HEIGHT: Rems = ui(120.);
/// The height of the bulk key-value editor, and the cap on an example's request text.
pub const EDITOR_HEIGHT: Rems = ui(300.);
/// The tallest the decoded JWT block in the auth section grows before its content scrolls.
pub const JWT_BLOCK_MAX_HEIGHT: Rems = ui(240.);
/// Size of the app icon in the no-collection state.
pub const APP_ICON_SIZE: Rems = ui(64.);
/// The sidebar width when the session has none; it follows the zoom, while a dragged width
/// is saved and restored as px.
pub const SIDEBAR_WIDTH: Rems = ui(240.);
/// The most of the window's width the sidebar shows; a wider preferred width is kept but
/// not shown.
pub const SIDEBAR_MAX_SHARE: f32 = 0.4;
/// The part of the narrowest sidebar that follows the zoom, unless the cap is narrower
/// still: the filter row's three buttons, gaps and padding (98), the search icon (11.5) and
/// a text area of about seven characters (48), measured at 100%.
pub const SIDEBAR_MIN: Rems = ui(158.);
/// The filter input's side paddings (11 each) and icon gap (6), fixed px at every zoom,
/// which the narrowest sidebar adds to [`SIDEBAR_MIN`].
pub const SIDEBAR_MIN_FIXED: Pixels = px(28.);

/// The narrowest a drag takes the sidebar at `window`'s zoom.
pub fn sidebar_min(window: &Window) -> Pixels {
    resolve(SIDEBAR_MIN, window) + SIDEBAR_MIN_FIXED
}

/// The narrowest a drag takes the sidebar at `zoom`; for code without a window.
pub fn sidebar_min_at(zoom: f32) -> Pixels {
    at_zoom(SIDEBAR_MIN, zoom) + SIDEBAR_MIN_FIXED
}
/// The part of the narrowest filter field that shows its key hint which follows the zoom:
/// icon and gap (15), placeholder (41) and hint (42), measured at 100%.
pub const FILTER_HINT_MIN_CONTENT: Rems = ui(98.);
/// The kit input's two side paddings, 11 px each at every zoom.
pub const INPUT_SIDE_PADDINGS: Pixels = px(22.);

/// The narrowest filter field that shows its key hint, at `window`'s zoom.
pub fn filter_hint_min_field(window: &Window) -> Pixels {
    resolve(FILTER_HINT_MIN_CONTENT, window) + INPUT_SIDE_PADDINGS
}
/// The console dock's height each time it opens; neither it nor a dragged height is kept.
pub const DOCK_HEIGHT: Rems = ui(240.);
/// The least height of a settings or runner view; a shorter tab area scrolls it.
pub const TAB_VIEW_MIN: Rems = ui(160.);
/// The least height the tab area keeps with the dock open, room for a request's toolbar,
/// an editor section and a response strip.
pub const TAB_AREA_MIN: Rems = ui(380.);
/// The least height the open dock keeps: its header strip and a filter row.
pub const DOCK_MIN: Rems = ui(84.);
/// The space above and below the dock header's tab strip.
pub const DOCK_HEADER_PADDING: Rems = ui(3.);

/// Height of the kit's underline tab strip, which steps with the kit size rather than scaling.
pub fn tab_strip_height(zoom: f32) -> Pixels {
    match kit_size(zoom) {
        Size::XSmall => px(26.),
        Size::Small => px(30.),
        Size::Large => px(44.),
        _ => px(36.),
    }
}

/// The dock's header, all a collapsed dock shows: its tab strip and the padding around it.
pub fn dock_header(zoom: f32) -> Pixels {
    tab_strip_height(zoom) + at_zoom(DOCK_HEADER_PADDING, zoom) * 2.
}

/// The dock's height between the bars: the preferred height, at most 40% of the room and
/// leaving the tab area its minimum, but never under the dock's own minimum, which wins in
/// a window too short for both.
pub fn dock_height(
    preferred: Pixels,
    between_bars: Pixels,
    tab_min: Pixels,
    dock_min: Pixels,
) -> Pixels {
    preferred
        .min(between_bars * 0.4)
        .min(between_bars - tab_min)
        .max(dock_min)
}

/// Whether the open dock shows only its header: it does once the dock at its minimum would
/// leave the tab area less than a strip, counted as one dock header, over a view's minimum.
pub fn dock_collapses(
    between_bars: Pixels,
    view_min: Pixels,
    header: Pixels,
    dock_min: Pixels,
) -> bool {
    between_bars < view_min + header + dock_min
}

/// Below this window width the sidebar steps aside: three default sidebars, so it never
/// takes more than a third of the window at any zoom.
pub const SIDEBAR_AUTO_HIDE_BELOW: Rems = ui(720.);
/// The widest a variable tooltip grows before its text wraps.
pub const VAR_TIP_MAX_WIDTH: Rems = ui(360.);
/// Width of the placeholder completion list.
pub const VAR_MENU_WIDTH: Rems = ui(320.);
/// Height of the placeholder completion list before it scrolls.
pub const VAR_MENU_MAX_HEIGHT: Rems = ui(240.);

#[cfg(test)]
mod tests {
    use super::*;

    fn at(height: f32) -> f32 {
        // 36 px title bar and 24 px status bar at zoom 1, the default dock.
        let bb = px(height - 60.);
        dock_height(
            at_zoom(DOCK_HEIGHT, 1.),
            bb,
            at_zoom(TAB_AREA_MIN, 1.),
            at_zoom(DOCK_MIN, 1.),
        )
        .into()
    }

    #[test]
    fn the_dock_leaves_the_tab_area_its_minimum_where_it_can() {
        assert_eq!(at(400.), 84., "the dock minimum wins");
        assert_eq!(at(500.), 84., "the dock minimum wins");
        assert_eq!(at(600.), 160., "room minus the tab minimum");
        assert_eq!(at(800.), 240., "the preferred height");
        assert_eq!(at(1080.), 240., "the preferred height");
    }

    fn collapses(height: f32, zoom: f32) -> bool {
        let bars = at_zoom(TITLE_BAR_HEIGHT, zoom) + at_zoom(STATUS_BAR_HEIGHT, zoom);
        dock_collapses(
            px(height) - bars,
            at_zoom(TAB_VIEW_MIN, zoom),
            dock_header(zoom),
            at_zoom(DOCK_MIN, zoom),
        )
    }

    #[test]
    fn the_dock_collapses_below_346_px_at_100_percent() {
        assert!(collapses(300., 1.));
        assert!(collapses(345., 1.));
        assert!(!collapses(346., 1.));
        assert!(!collapses(800., 1.));
        assert!(collapses(500., 1.5), "the threshold follows the zoom");
    }
}
