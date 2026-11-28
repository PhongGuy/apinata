//! The Vim key map's Normal-mode state machine, fed keys over plain text.

use apinata_app::vim::{VimMode, VimOp, VimState};

// Feeds `keys` (one key per char) from `cursor`, following each move, and returns the ops of
// the last key.
fn feed(text: &str, cursor: usize, keys: &str) -> (Vec<VimOp>, VimState) {
    let mut vim = VimState::new();
    let mut at = cursor;
    let mut last = Vec::new();
    for key in keys.chars() {
        last = vim.feed(&key.to_string(), text, at);
        for op in &last {
            match op {
                VimOp::MoveTo(c) | VimOp::Edit { cursor: c, .. } => at = *c,
                _ => {}
            }
        }
    }
    (last, vim)
}

fn moved(text: &str, cursor: usize, keys: &str) -> usize {
    match feed(text, cursor, keys).0.as_slice() {
        [VimOp::MoveTo(c)] => *c,
        other => panic!("{keys:?} on {text:?} gave {other:?}"),
    }
}

fn edit(range: std::ops::Range<usize>, text: &str, cursor: usize) -> VimOp {
    VimOp::Edit {
        range,
        text: text.into(),
        cursor,
    }
}

#[test]
fn starts_in_normal() {
    assert_eq!(VimState::new().mode(), VimMode::Normal);
}

#[test]
fn h_and_l_stay_on_the_line_and_take_a_count() {
    assert_eq!(moved("hello", 0, "3l"), 3);
    assert_eq!(moved("hello", 4, "l"), 4);
    assert_eq!(moved("ab\ncd", 3, "h"), 3);
    assert_eq!(moved("hello", 4, "2h"), 2);
}

#[test]
fn l_steps_over_multibyte_characters() {
    assert_eq!(moved("héllo", 1, "l"), 3);
    assert_eq!(moved("héllo", 3, "h"), 1);
}

#[test]
fn line_ends_and_first_non_blank() {
    assert_eq!(moved("  ab cd", 6, "0"), 0);
    assert_eq!(moved("  ab cd", 6, "^"), 2);
    assert_eq!(moved("  ab cd\nx", 0, "$"), 6);
}

#[test]
fn j_and_k_keep_the_column_within_the_line() {
    let text = "abc\nde\nfgh";
    assert_eq!(moved(text, 2, "j"), 5);
    assert_eq!(moved(text, 1, "2j"), 8);
    assert_eq!(moved(text, 8, "k"), 5);
    assert_eq!(moved(text, 1, "k"), 1);
}

#[test]
fn word_motions() {
    let text = "foo bar.baz";
    assert_eq!(moved(text, 0, "w"), 4);
    assert_eq!(moved(text, 4, "w"), 7);
    assert_eq!(moved(text, 7, "w"), 8);
    assert_eq!(moved(text, 0, "2w"), 7);
    assert_eq!(moved(text, 8, "b"), 7);
    assert_eq!(moved(text, 7, "b"), 4);
    assert_eq!(moved(text, 4, "b"), 0);
    assert_eq!(moved(text, 0, "e"), 2);
    assert_eq!(moved(text, 2, "e"), 6);
    assert_eq!(moved("a\n  b", 0, "w"), 4);
}

#[test]
fn gg_and_g_go_to_lines() {
    let text = "a\n  b\nc";
    assert_eq!(moved(text, 6, "gg"), 0);
    assert_eq!(moved(text, 0, "G"), 6);
    assert_eq!(moved(text, 0, "2G"), 4);
    assert_eq!(moved(text, 0, "2gg"), 4);
}

#[test]
fn x_deletes_characters_on_the_line() {
    assert_eq!(feed("hello", 1, "x").0, vec![edit(1..2, "", 1)]);
    assert_eq!(feed("hello", 1, "2x").0, vec![edit(1..3, "", 1)]);
    assert_eq!(feed("ab\nc", 1, "5x").0, vec![edit(1..2, "", 0)]);
    assert_eq!(feed("\nc", 0, "x").0, vec![]);
}

#[test]
fn dd_deletes_whole_lines() {
    assert_eq!(feed("a\n  b\nc", 2, "dd").0, vec![edit(2..6, "", 2)]);
    assert_eq!(feed("a\nb", 2, "dd").0, vec![edit(1..3, "", 0)]);
    assert_eq!(feed("abc", 1, "dd").0, vec![edit(0..3, "", 0)]);
    assert_eq!(feed("a\nb\nc", 0, "2dd").0, vec![edit(0..4, "", 0)]);
}

#[test]
fn d_dollar_style_d_deletes_to_line_end() {
    assert_eq!(feed("hello\nx", 2, "D").0, vec![edit(2..5, "", 1)]);
}

#[test]
fn insert_commands_enter_insert_mode() {
    let (ops, vim) = feed("ab", 0, "i");
    assert_eq!((ops, vim.mode()), (vec![], VimMode::Insert));
    let (ops, vim) = feed("ab", 0, "a");
    assert_eq!((ops, vim.mode()), (vec![VimOp::MoveTo(1)], VimMode::Insert));
    assert_eq!(feed("ab\nc", 0, "A").0, vec![VimOp::MoveTo(2)]);
    assert_eq!(feed("  ab", 3, "I").0, vec![VimOp::MoveTo(2)]);
    assert_eq!(feed("ab\ncd", 0, "o").0, vec![edit(2..2, "\n", 3)]);
    assert_eq!(feed("ab\ncd", 4, "O").0, vec![edit(3..3, "\n", 3)]);
}

#[test]
fn u_undoes_count_times() {
    assert_eq!(feed("a", 0, "2u").0, vec![VimOp::Undo, VimOp::Undo]);
}

#[test]
fn unknown_keys_do_nothing_and_escape_drops_a_pending_operator() {
    let (ops, vim) = feed("abc", 0, "z");
    assert_eq!((ops, vim.mode()), (vec![], VimMode::Normal));
    let mut vim = VimState::new();
    assert_eq!(vim.feed("d", "a\nb", 0), vec![]);
    assert_eq!(vim.feed("escape", "a\nb", 0), vec![]);
    assert_eq!(vim.feed("d", "a\nb", 0), vec![]);
}

#[test]
fn leaving_insert_steps_back_one_character_but_not_off_the_line() {
    let mut vim = VimState::new();
    vim.feed("i", "ab\ncd", 0);
    assert_eq!(vim.leave_insert("ab\ncd", 2), vec![VimOp::MoveTo(1)]);
    assert_eq!(vim.mode(), VimMode::Normal);
    vim.feed("i", "ab\ncd", 0);
    assert_eq!(vim.leave_insert("ab\ncd", 3), vec![VimOp::MoveTo(3)]);
}
