//! The shared key-value grid: column descriptions and the heading row's colours.

use crate::support;

use apinata_app::kv_grid::{KvColumn, KvWidth, Reloaded, after_reload, head_colors};
use apinata_app::kv_table::KvColumns;
use apinata_app::style::TAG_WIDTH;
use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::{ElementId, TestAppContext};

fn heads(columns: &[KvColumn]) -> Vec<String> {
    columns.iter().map(|c| c.heading.to_string()).collect()
}

#[test]
fn the_presets_describe_their_columns_in_order() {
    let labels = ("Key", "Value");
    assert_eq!(heads(&KvColumns::PLAIN.describe(labels)), ["Key", "Value"]);
    assert_eq!(
        heads(&KvColumns::DESCRIBED.describe(labels)),
        ["Key", "Value", "Description"]
    );
    assert_eq!(
        heads(&KvColumns::MULTIPART.describe(labels)),
        ["Key", "Value", "Content type", "Description"]
    );
    assert_eq!(
        heads(&KvColumns::FILE_BODY.describe(labels)),
        ["Files", "Content type"]
    );
    assert_eq!(
        heads(&KvColumns::PLAIN.describe(("Name", "Expression"))),
        ["Name", "Expression"]
    );
}

#[test]
fn described_columns_keep_the_heading_and_cell_ids() {
    let cols = KvColumns::MULTIPART.describe(("Key", "Value"));
    let ids: Vec<ElementId> = cols.iter().map(|c| c.head_id.clone()).collect();
    let want: Vec<ElementId> = [
        "kv-head-name",
        "kv-head-value",
        "kv-head-ctype",
        "kv-head-description",
    ]
    .map(ElementId::from)
    .to_vec();
    assert_eq!(ids, want);
    let cells: Vec<&str> = cols.iter().map(|c| c.cell_id).collect();
    assert_eq!(cells, ["kv-name", "kv-value", "kv-ctype", "kv-description"]);
    assert!(cols.iter().all(|c| c.width == KvWidth::Flex));
}

#[test]
fn a_fixed_column_keeps_its_width() {
    assert_eq!(
        KvColumn::fixed("h", "c", "Tag", TAG_WIDTH).width,
        KvWidth::Fixed(TAG_WIDTH)
    );
    assert_eq!(KvColumn::flex("h", "c", "Name").width, KvWidth::Flex);
}

#[gpui_kit::test]
fn the_heading_row_takes_the_table_head_tokens(cx: &mut TestAppContext) {
    support::init(cx);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        cx.update(|cx| {
            let t = cx.theme();
            assert_eq!(
                head_colors(cx),
                (t.table_head, t.muted_foreground),
                "{mode:?}"
            );
        });
    }
}

// Where control focus on row `ix` goes when a reload replaces `old` by `new`, rows compared
// by their letter and nothing pinned.
fn reloaded(old: &str, new: &str, ix: usize) -> Reloaded {
    let (old, new): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    after_reload(&old, &new, ix, |_| false, |a, b| a == b)
}

#[test]
fn a_reload_dropping_a_row_above_keeps_the_row_one_slot_up() {
    assert_eq!(reloaded("abc", "ac", 2), Reloaded::Kept(1));
    assert_eq!(reloaded("abc", "ac", 0), Reloaded::Kept(0));
}

#[test]
fn a_reload_dropping_the_row_itself_drops_it() {
    assert_eq!(reloaded("abc", "ac", 1), Reloaded::Dropped(1));
}

#[test]
fn a_reload_inserting_a_row_above_keeps_the_row_one_slot_down() {
    assert_eq!(reloaded("ac", "abc", 1), Reloaded::Kept(2));
    assert_eq!(reloaded("ac", "abc", 0), Reloaded::Kept(0));
    assert_eq!(reloaded("bc", "abc", 0), Reloaded::Kept(1));
}

#[test]
fn a_longer_reload_that_is_no_insert_keeps_the_index() {
    assert_eq!(reloaded("ab", "xyz", 1), Reloaded::Kept(1));
}

#[test]
fn a_reload_moving_the_row_keeps_it_at_its_new_place() {
    assert_eq!(reloaded("abc", "bca", 0), Reloaded::Kept(2));
    assert_eq!(reloaded("abc", "cab", 2), Reloaded::Kept(0));
}

// The focused row did not move itself, but another row's move shifted it.
#[test]
fn a_reload_moving_another_row_keeps_the_row_where_it_shifted() {
    assert_eq!(reloaded("abc", "bca", 1), Reloaded::Kept(0));
    assert_eq!(reloaded("abc", "acb", 0), Reloaded::Kept(0));
}

#[test]
fn a_same_length_reload_that_is_no_move_keeps_the_index() {
    assert_eq!(reloaded("abc", "aXc", 2), Reloaded::Kept(2));
    assert_eq!(reloaded("abc", "abc", 1), Reloaded::Kept(1));
}

// A pinned row keeps its slot in a move, as the table moves rows around it.
#[test]
fn a_reload_moving_rows_around_a_pinned_row_follows_the_row() {
    let old: Vec<char> = "aPb".chars().collect();
    let new: Vec<char> = "bPa".chars().collect();
    let pinned = |c: &char| *c == 'P';
    assert_eq!(
        after_reload(&old, &new, 0, pinned, |a, b| a == b),
        Reloaded::Kept(2)
    );
}

// An insert above and an edit to another row in the same reload: the focused row follows
// by content, whichever side the edit is on.
#[test]
fn a_reload_inserting_above_and_editing_another_row_follows_the_row() {
    assert_eq!(reloaded("ambc", "amnbC", 2), Reloaded::Kept(3));
    assert_eq!(reloaded("ambc", "aMnbc", 2), Reloaded::Kept(3));
}

#[test]
fn a_reload_dropping_above_and_editing_below_follows_the_row() {
    assert_eq!(reloaded("abcd", "acD", 2), Reloaded::Kept(1));
}

// The focused row itself edited in place (an undone toggle) keeps its slot.
#[test]
fn a_reload_editing_the_row_itself_in_place_keeps_it() {
    assert_eq!(reloaded("abc", "aXc", 1), Reloaded::Kept(1));
}

// Twin rows: of the alignments that explain a reload, the one leaving focus nearest its
// old place wins.
#[test]
fn twin_rows_keep_focus_nearest_its_old_place() {
    assert_eq!(reloaded("AAB", "ABA", 0), Reloaded::Kept(0));
}
