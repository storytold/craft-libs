//! An edit to a clone copies only the blocks it changes.

use std::collections::HashSet;

use craft_shared_vec::{BlockId, SharedVec};

fn ids(list: &SharedVec<u64>, leaves_only: bool) -> HashSet<BlockId> {
    list.blocks().into_iter().filter(|block| block.is_leaf || !leaves_only).map(|block| block.id).collect()
}

/// The blocks of `after`, and of its leaves, that `before` does not hold.
fn new_blocks(before: &SharedVec<u64>, after: &SharedVec<u64>) -> (usize, usize) {
    let old = ids(before, false);
    let blocks = after.blocks();
    let new: Vec<_> = blocks.iter().filter(|block| !old.contains(&block.id)).collect();
    (new.len(), new.iter().filter(|block| block.is_leaf).count())
}

#[test]
fn a_set_copies_one_leaf_and_the_branches_above_it() {
    for count in [1_000u64, 10_000, 100_000] {
        let before: SharedVec<u64> = (0..count).collect();
        let mut after = before.clone();
        assert!(after.ptr_eq(&before));
        assert_eq!(after.set(count as usize / 2, u64::MAX), Ok(count / 2));
        assert!(!after.ptr_eq(&before));
        let (blocks, leaves) = new_blocks(&before, &after);
        assert_eq!(leaves, 1, "{count} elements");
        // Up to 64 * 64 * 64 elements, a leaf has at most two branches above it.
        assert!(blocks <= 3, "{count} elements: {blocks} new blocks");
        assert_eq!(before.get(count as usize / 2), Some(&(count / 2)));
    }
}

#[test]
fn a_get_mut_copies_the_same_path_as_a_set() {
    let before: SharedVec<u64> = (0..50_000).collect();
    let mut after = before.clone();
    if let Some(value) = after.get_mut(31_000) {
        *value += 1;
    }
    let (_, leaves) = new_blocks(&before, &after);
    assert_eq!(leaves, 1);
    assert_eq!(after.get(31_000), Some(&31_001));
    assert_eq!(before.get(31_000), Some(&31_000));
}

#[test]
fn single_removals_at_the_front_middle_and_end_copy_at_most_two_leaves() {
    let before: SharedVec<u64> = (0..100_000).collect();
    for at in [0, 50_000, 99_999] {
        let mut after = before.clone();
        assert_eq!(after.remove(at), Some(at as u64));
        let (_, leaves) = new_blocks(&before, &after);
        // The leaf that lost the element, and a neighbor it merged with or took elements from.
        assert!(leaves <= 2, "removal at {at} copied {leaves} leaves");
        assert_eq!(after.len(), 99_999);
    }
}

#[test]
fn removing_from_the_end_one_at_a_time_keeps_the_earlier_leaves() {
    let start: SharedVec<u64> = (0..20_000).collect();
    let mut list = start.clone();
    for _ in 0..5_000 {
        let before = list.clone();
        list.pop();
        let (_, leaves) = new_blocks(&before, &list);
        assert!(leaves <= 2);
    }
    let kept = ids(&start, true);
    let shared = ids(&list, true).intersection(&kept).count();
    assert!(shared * 64 >= 14_000, "only {shared} of the first leaves are still shared");
}

#[test]
fn an_insertion_shares_the_leaves_it_does_not_touch() {
    let before: SharedVec<u64> = (0..100_000).collect();
    let mut after = before.clone();
    assert_eq!(after.insert(0, 7), Ok(()));
    let (_, leaves) = new_blocks(&before, &after);
    // The leaf that took the element, and the second half when it split.
    assert!(leaves <= 2);
    assert_eq!(after.first(), Some(&7));
    assert_eq!(after.get(1), Some(&0));
}

#[test]
fn retain_shares_every_leaf_it_keeps_whole() {
    let before: SharedVec<u64> = (0..100_000).collect();
    let mut after = before.clone();
    after.retain(|&value| value != 40_000);
    assert_eq!(after.len(), 99_999);
    let total = ids(&after, true).len();
    let (_, leaves) = new_blocks(&before, &after);
    assert!(leaves <= 2, "{leaves} of {total} leaves are new");

    // Nothing removed: the list is the same tree.
    let mut unchanged = before.clone();
    unchanged.retain(|_| true);
    assert!(unchanged.ptr_eq(&before));
}

#[test]
fn append_and_split_off_keep_the_leaves_away_from_the_cut() {
    let left: SharedVec<u64> = (0..10_000).collect();
    let right: SharedVec<u64> = (10_000..20_000).collect();
    let mut joined = left.clone();
    joined.append(&mut right.clone());
    assert!(joined.iter().copied().eq(0..20_000));
    let original: HashSet<BlockId> = ids(&left, true).union(&ids(&right, true)).copied().collect();
    let fresh = ids(&joined, true).difference(&original).count();
    assert!(fresh <= 2, "{fresh} leaves were copied at the join");

    let mut front = joined.clone();
    let back = front.split_off(12_345);
    assert!(front.iter().copied().eq(0..12_345));
    assert!(back.iter().copied().eq(12_345..20_000));
    let from_joined = ids(&joined, true);
    let fresh = ids(&front, true).union(&ids(&back, true)).filter(|id| !from_joined.contains(id)).count();
    assert!(fresh <= 4, "{fresh} leaves were copied at the cut");
}

#[test]
fn splice_replaces_a_range_and_keeps_the_rest_shared() {
    let before: SharedVec<u64> = (0..10_000).collect();
    let mut after = before.clone();
    let removed = after.splice(5_000..5_010, [1, 2, 3]);
    assert!(removed.iter().copied().eq(5_000..5_010));
    assert_eq!(after.len(), 9_993);
    assert!(after.iter().skip(5_000).take(3).copied().eq([1, 2, 3]));
    let (_, leaves) = new_blocks(&before, &after);
    assert!(leaves <= 4);
}

#[test]
fn a_splice_whose_replacement_panics_leaves_the_list_as_it_was() {
    let mut list: SharedVec<u64> = (0..10_000).collect();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        list.splice(100..200, (0..10u64).map(|value| if value == 5 { panic!("replacement failed") } else { value }))
    }));
    assert!(result.is_err());
    assert!(list.iter().copied().eq(0..10_000));
}

#[test]
fn a_list_holding_nan_is_not_equal_to_its_clone() {
    let list: SharedVec<f64> = [1.0, f64::NAN].into_iter().collect();
    assert!(list != list.clone());
    assert_eq!(list.partial_cmp(&list.clone()), None);
    let numbers: SharedVec<f64> = [1.0, 2.0].into_iter().collect();
    assert!(numbers == numbers.clone());
}

#[test]
fn chunks_cover_every_element_once_in_order() {
    let list: SharedVec<u64> = (0..10_007).collect();
    let flat: Vec<u64> = list.chunks().flatten().copied().collect();
    assert!(flat.iter().copied().eq(0..10_007));
    assert!(list.chunks().all(|chunk| !chunk.is_empty()));
    assert_eq!(SharedVec::<u64>::new().chunks().count(), 0);
}

#[test]
fn out_of_range_edits_leave_the_list_unchanged() {
    let mut list: SharedVec<u64> = (0..100).collect();
    let before = list.clone();
    assert_eq!(list.set(100, 1), Err(1));
    assert_eq!(list.insert(101, 1), Err(1));
    assert_eq!(list.remove(100), None);
    assert!(list.split_off(100).is_empty());
    list.truncate(500);
    assert!(list.get_mut(100).is_none());
    assert!(list.ptr_eq(&before));
}
