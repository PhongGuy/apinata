//! Bruno's sibling ordering and renumbering at `1301ea17`: `sortByNameThenSequence`
//! (bruno-common), `resetSequencesInFolder` and the two `getReordered*` helpers (bruno-app
//! `utils/collections`), over display names and seqs.

use crate::collection::name_order;

/// One sibling as Bruno's numbering sees it: its display name and Bruno's seq for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeqItem {
    pub(crate) name: String,
    pub(crate) seq: Option<u64>,
}

/// Sort by name, then splice items with a positive seq in at index `seq - 1`. Equal seqs share
/// one slot, so a group counts as a single position for every later splice.
pub(crate) fn sort_by_name_then_seq<T>(
    mut items: Vec<T>,
    name: impl Fn(&T) -> &str,
    seq: impl Fn(&T) -> Option<u64>,
) -> Vec<T> {
    items.sort_by(|a, b| name_order(name(a), name(b)));
    let (mut sequenced, rest): (Vec<T>, Vec<T>) = items
        .into_iter()
        .partition(|i| seq(i).is_some_and(|s| s > 0));
    sequenced.sort_by_key(&seq);
    let mut slots: Vec<Vec<T>> = rest.into_iter().map(|i| vec![i]).collect();
    for item in sequenced {
        let s = seq(&item);
        let index = s
            .and_then(|s| usize::try_from(s - 1).ok())
            .unwrap_or(usize::MAX)
            .min(slots.len());
        match slots.get_mut(index) {
            Some(slot) if slot.first().is_some_and(|f| seq(f) == s) => slot.push(item),
            _ => slots.insert(index, vec![item]),
        }
    }
    slots.into_iter().flatten().collect()
}

/// `resetSequencesInFolder`: indices into `items` in Bruno's 1..n order.
pub(crate) fn reset_order(items: &[SeqItem]) -> Vec<usize> {
    let indexed: Vec<(usize, &SeqItem)> = items.iter().enumerate().collect();
    sort_by_name_then_seq(indexed, |(_, i)| i.name.as_str(), |(_, i)| i.seq)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

/// Number `order` 1..n and keep what differs from each item's stored seq, by index.
fn changed(stored: &[Option<u64>], order: &[usize]) -> Vec<(usize, u64)> {
    let mut out: Vec<(usize, u64)> = order
        .iter()
        .zip(1u64..)
        .filter(|&(&i, seq)| stored[i] != Some(seq))
        .map(|(&i, seq)| (i, seq))
        .collect();
    out.sort_unstable();
    out
}

/// Merge restored items into `list`, the siblings still there, in order. `restored` runs newest
/// delete first, each item with its tail: the siblings after it at delete time, in the order the
/// folder listed them. An item goes just before the first tail member still listed, or last.
pub(crate) fn place_restored<T: PartialEq + Clone>(
    mut list: Vec<T>,
    restored: &[(T, Vec<T>)],
) -> Vec<T> {
    for (item, tail) in restored {
        let at = tail
            .iter()
            .find_map(|path| list.iter().position(|p| p == path))
            .unwrap_or(list.len());
        list.insert(at, item.clone());
    }
    list
}

/// Send each `ranked` member of `tail`, the siblings after a deleted item in their old order, back
/// behind the members it followed, where the delete pulled it ahead of them. A member already
/// behind them stays put.
pub(crate) fn return_ranked<T: PartialEq>(
    list: &mut Vec<T>,
    tail: &[&T],
    ranked: impl Fn(&T) -> bool,
) {
    for (index, member) in tail.iter().enumerate().filter(|(_, m)| ranked(m)) {
        let position = |path: &T| list.iter().position(|p| p == path);
        let Some(from) = position(member) else {
            continue;
        };
        let behind = tail[..index].iter().filter_map(|p| position(p)).max();
        if let Some(behind) = behind.filter(|&b| b > from) {
            let moved = list.remove(from);
            list.insert(behind, moved);
        }
    }
}

/// Case-insensitive natural order: digit runs compare by value, so "item 2" precedes "item 10".
/// Names equal under that order compare raw, as [`name_order`] does for ties.
pub(crate) fn natural_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for ch in s.to_lowercase().chars() {
            let digit = ch.is_ascii_digit();
            match out.last_mut() {
                Some((d, run)) if *d == digit => run.push(ch),
                _ => out.push((digit, ch.to_string())),
            }
        }
        out
    }
    let (x, y) = (chunks(a), chunks(b));
    for ((xd, xs), (yd, ys)) in x.iter().zip(&y) {
        let ord = if *xd && *yd {
            let (xs, ys) = (xs.trim_start_matches('0'), ys.trim_start_matches('0'));
            xs.len().cmp(&ys.len()).then_with(|| xs.cmp(ys))
        } else {
            xs.cmp(ys)
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    x.len().cmp(&y.len()).then_with(|| a.cmp(b))
}

/// Renumber 1..n by natural name order alone, ignoring stored seqs, and report what changed.
/// Equal names keep their listed order.
pub(crate) fn sort_names(items: &[SeqItem]) -> Vec<(usize, u64)> {
    let stored: Vec<Option<u64>> = items.iter().map(|i| i.seq).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&items[a], &items[b]);
        // Equal names stay in their current seq order, so a re-sort is a no-op.
        natural_order(&x.name, &y.name).then_with(|| x.seq.cmp(&y.seq))
    });
    changed(&stored, &order)
}

/// `getReorderedItemsInSourceDirectory`: renumber 1..n and report what changed.
pub(crate) fn reorder_source(items: &[SeqItem]) -> Vec<(usize, u64)> {
    let stored: Vec<Option<u64>> = items.iter().map(|i| i.seq).collect();
    changed(&stored, &reset_order(items))
}

/// `getReorderedItemsInTargetDirectory`: renumber, move `dragged` above or below `target`,
/// renumber again. `items[dragged].seq` places the item (the target's seq for a
/// cross-directory drop, as Bruno lists it); `dragged_stored`, its seq on disk, decides
/// whether it is rewritten.
pub(crate) fn reorder_target(
    items: &[SeqItem],
    target: usize,
    dragged: usize,
    below: bool,
    dragged_stored: Option<u64>,
) -> Vec<(usize, u64)> {
    let mut stored: Vec<Option<u64>> = items.iter().map(|i| i.seq).collect();
    stored[dragged] = dragged_stored;
    let mut order = reset_order(items);
    let position = |i| order.iter().position(|&x| x == i).expect("listed");
    let (t, d) = (position(target), position(dragged));
    let mut at = if below { t + 1 } else { t };
    if d < at {
        at -= 1;
    }
    let moved = order.remove(d);
    order.insert(at, moved);
    changed(&stored, &order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(rows: &[(&str, Option<u64>)]) -> Vec<SeqItem> {
        rows.iter()
            .map(|(n, s)| SeqItem {
                name: (*n).to_owned(),
                seq: *s,
            })
            .collect()
    }

    fn sorted(rows: &[(&str, Option<u64>)]) -> Vec<String> {
        let out = sort_by_name_then_seq(items(rows), |i| i.name.as_str(), |i| i.seq);
        out.into_iter().map(|i| i.name).collect()
    }

    #[test]
    fn names_order_items_without_a_seq() {
        assert_eq!(
            sorted(&[("b", None), ("a", None), ("c", None)]),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn a_seq_splices_the_item_into_place() {
        assert_eq!(
            sorted(&[("a", None), ("b", Some(1)), ("c", None)]),
            ["b", "a", "c"]
        );
    }

    #[test]
    fn equal_seqs_share_one_slot() {
        assert_eq!(
            sorted(&[("y", Some(1)), ("x", Some(1)), ("z", None)]),
            ["x", "y", "z"]
        );
    }

    #[test]
    fn a_seq_past_the_end_appends_and_zero_counts_as_none() {
        assert_eq!(sorted(&[("a", Some(9)), ("b", None)]), ["b", "a"]);
        assert_eq!(sorted(&[("b", Some(0)), ("a", Some(2))]), ["b", "a"]);
    }

    #[test]
    fn the_source_directory_is_renumbered_without_gaps() {
        assert_eq!(
            reorder_source(&items(&[("a", Some(1)), ("c", Some(3))])),
            [(1, 2)]
        );
        assert!(reorder_source(&items(&[("a", Some(1)), ("b", Some(2))])).is_empty());
    }

    #[test]
    fn a_drop_above_moves_the_item_before_the_target() {
        let list = items(&[("a", Some(1)), ("b", Some(2)), ("c", Some(3))]);
        assert_eq!(
            reorder_target(&list, 0, 2, false, Some(3)),
            [(0, 2), (1, 3), (2, 1)]
        );
    }

    #[test]
    fn a_drop_below_moves_the_item_after_the_target() {
        let list = items(&[("a", Some(1)), ("b", Some(2)), ("c", Some(3))]);
        assert_eq!(
            reorder_target(&list, 2, 0, true, Some(1)),
            [(0, 3), (1, 1), (2, 2)]
        );
    }

    #[test]
    fn a_drop_also_closes_gaps_around_it() {
        let list = items(&[("a", Some(2)), ("b", Some(5)), ("c", None)]);
        assert_eq!(
            reorder_target(&list, 1, 2, true, None),
            [(0, 1), (1, 2), (2, 3)]
        );
    }

    #[test]
    fn a_dragged_item_is_compared_with_its_stored_seq_not_the_targets() {
        // A cross-directory drop above `y`: `a` is placed with y's seq but stored as 7.
        let list = items(&[("x", Some(1)), ("y", Some(2)), ("a", Some(2))]);
        assert_eq!(
            reorder_target(&list, 1, 2, false, Some(7)),
            [(1, 3), (2, 2)]
        );
    }
    /// One restored item and its tail, in its old listed order.
    fn back(item: &'static str, tail: &[&'static str]) -> (&'static str, Vec<&'static str>) {
        (item, tail.to_vec())
    }

    #[test]
    fn items_pruned_from_a_folder_return_to_their_old_order() {
        // [a, b, c, keep], pruned c, then b, then a: each tail is only `keep`.
        let order = place_restored(
            vec!["keep"],
            &[
                back("a", &["keep"]),
                back("b", &["keep"]),
                back("c", &["keep"]),
            ],
        );
        assert_eq!(order, ["a", "b", "c", "keep"]);
    }

    #[test]
    fn an_item_goes_before_an_item_restored_with_it() {
        // [a, b, c], pruned a, then b, then c: the later tails shrink as the folder does.
        let order = place_restored(
            Vec::new(),
            &[back("c", &[]), back("b", &["c"]), back("a", &["b", "c"])],
        );
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn a_tail_member_that_left_the_folder_is_passed_over() {
        let order = place_restored(vec!["x", "z"], &[back("y", &["gone", "z"])]);
        assert_eq!(order, ["x", "y", "z"]);
    }

    #[test]
    fn an_item_with_no_tail_left_goes_last() {
        let order = place_restored(vec!["x", "z"], &[back("y", &["gone"])]);
        assert_eq!(order, ["x", "z", "y"]);
    }

    #[test]
    fn the_first_tail_member_still_listed_wins_wherever_it_now_stands() {
        // y followed n most closely; x moved ahead of it since, so n stays behind x.
        let order = place_restored(vec!["x", "y"], &[back("n", &["y", "x"])]);
        assert_eq!(order, ["x", "n", "y"]);
    }
}
