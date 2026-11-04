use apinata_app::tag_input::TagChips;

fn chips(tags: &[&str]) -> TagChips {
    let mut c = TagChips::default();
    c.load(tags.iter().map(|t| (*t).to_owned()).collect());
    c
}

#[test]
fn enter_commits_the_typed_text_as_a_chip() {
    let mut c = TagChips::default();
    assert!(!c.type_text("smoke"));
    assert_eq!(c.pending(), "smoke");
    assert!(c.enter());
    assert_eq!(c.chips(), ["smoke"]);
    assert_eq!(c.pending(), "");
}

#[test]
fn a_comma_commits_the_text_before_it_and_keeps_the_rest_pending() {
    let mut c = TagChips::default();
    assert!(c.type_text("a, b ,c"));
    assert_eq!(c.chips(), ["a", "b"]);
    assert_eq!(c.pending(), "c");
}

#[test]
fn backspace_on_empty_input_removes_the_last_chip() {
    let mut c = chips(&["a", "b"]);
    assert!(c.backspace());
    assert_eq!(c.chips(), ["a"]);
}

#[test]
fn backspace_with_pending_text_removes_no_chip() {
    let mut c = chips(&["a"]);
    c.type_text("x");
    assert!(!c.backspace());
    assert_eq!(c.chips(), ["a"]);
}

#[test]
fn backspace_with_no_chips_changes_nothing() {
    assert!(!TagChips::default().backspace());
}

#[test]
fn remove_drops_the_chip_at_an_index() {
    let mut c = chips(&["a", "b", "c"]);
    assert!(c.remove(1));
    assert_eq!(c.chips(), ["a", "c"]);
    assert!(!c.remove(9));
}

#[test]
fn blur_commits_pending_text() {
    let mut c = TagChips::default();
    c.type_text("late");
    assert!(c.blur());
    assert_eq!(c.chips(), ["late"]);
    assert!(!c.blur());
}

#[test]
fn blanks_and_repeats_are_not_chips() {
    let mut c = chips(&["a"]);
    c.type_text("   ");
    assert!(!c.enter());
    c.type_text(" a ");
    assert!(!c.enter(), "a repeat changes nothing");
    assert_eq!(c.chips(), ["a"]);
    assert_eq!(c.pending(), "");
}

#[test]
fn loading_normalises_and_leaves_pending_text_alone() {
    let mut c = TagChips::default();
    c.type_text("typing");
    c.load(vec![" x ".into(), "x".into(), "".into(), "y".into()]);
    assert_eq!(c.chips(), ["x", "y"]);
    assert_eq!(c.pending(), "typing");
}
