//! `bytes_not_in` counts the blocks one list holds and another does not, and never less.

use std::collections::HashSet;
use std::mem::size_of;
use std::sync::Arc;

use craft_shared_vec::{BlockId, SharedVec};

/// The bytes of the blocks of `after` that `before` does not hold, from the full block lists.
fn expected<T>(before: &SharedVec<T>, after: &SharedVec<T>) -> usize {
    let old: HashSet<BlockId> = before.blocks().into_iter().map(|block| block.id).collect();
    let mut seen = HashSet::new();
    after.blocks().into_iter().filter(|block| !old.contains(&block.id) && seen.insert(block.id)).map(|block| block.bytes).sum()
}

#[test]
fn the_count_matches_the_blocks_each_edit_copied() {
    let base: SharedVec<u64> = (0..100_000).collect();
    type Edit = fn(&mut SharedVec<u64>);
    let edits: [(&str, Edit); 10] = [
        ("set", |list| assert!(list.set(77_777, 1).is_ok())),
        ("insert", |list| assert!(list.insert(3, 1).is_ok())),
        ("remove", |list| assert!(list.remove(64 * 40).is_some())),
        ("push", |list| list.push(1)),
        ("pop", |list| assert!(list.pop().is_some())),
        ("splice", |list| assert_eq!(list.splice(10..5_000, [1, 2, 3]).len(), 4_990)),
        ("retain", |list| list.retain(|value| value % 1_000 != 0)),
        ("append", |list| list.append(&mut (0..500).collect())),
        ("append itself", |list| list.append(&mut list.clone())),
        ("truncate", |list| list.truncate(60_001)),
    ];
    for (name, edit) in edits {
        let mut after = base.clone();
        edit(&mut after);
        assert_eq!(after.bytes_not_in(&base), expected(&base, &after), "{name}");
        // The other direction: what keeping the old list costs once the new one is kept.
        assert_eq!(base.bytes_not_in(&after), expected(&after, &base), "{name}, reversed");
    }
}

#[test]
fn a_list_costs_nothing_beside_its_clone_and_everything_beside_an_unrelated_list() {
    let list: SharedVec<u64> = (0..10_000).collect();
    assert_eq!(list.bytes_not_in(&list.clone()), 0);
    let unrelated: SharedVec<u64> = (0..10_000).collect();
    let all: usize = list.blocks().iter().map(|block| block.bytes).sum();
    assert_eq!(list.bytes_not_in(&unrelated), all);
    assert_eq!(list.bytes_not_in(&SharedVec::new()), all);
    assert!(all >= 10_000 * size_of::<u64>());
}

#[test]
fn a_range_edit_of_one_element_costs_about_as_much_as_one_removal() {
    let base: SharedVec<u64> = (0..100_000).collect();
    let mut removed = base.clone();
    assert!(removed.remove(50_000).is_some());
    let single = removed.bytes_not_in(&base);
    type Edit = fn(&mut SharedVec<u64>);
    let edits: [(&str, Edit); 6] = [
        ("splice out", |list| assert_eq!(list.splice(50_000..50_001, []).len(), 1)),
        ("splice in", |list| assert!(list.splice(50_000..50_000, [7]).is_empty())),
        ("retain", |list| list.retain(|value| *value != 50_000)),
        ("truncate", |list| list.truncate(99_999)),
        ("split_off", |list| assert_eq!(list.split_off(99_999).len(), 1)),
        ("append", |list| list.append(&mut SharedVec::from(vec![7]))),
    ];
    for (name, edit) in edits {
        let mut after = base.clone();
        edit(&mut after);
        let cost = after.bytes_not_in(&base);
        assert!(cost <= 2 * single, "{name}: {cost} bytes against {single} for one removal");
    }
}

#[test]
fn an_empty_list_holds_no_blocks() {
    let mut emptied: SharedVec<u64> = (0..5_000).collect();
    emptied.retain(|_| false);
    let read: SharedVec<u64> = serde_json::from_str("[]").unwrap_or_else(|error| panic!("{error}"));
    for list in [&SharedVec::new(), &SharedVec::default(), &read, &emptied] {
        assert!(list.blocks().is_empty());
        assert_eq!(list.bytes_not_in(&SharedVec::new()), 0);
        assert!(list.ptr_eq(&SharedVec::new()));
    }
    let mut popped: SharedVec<u64> = (0..3).collect();
    while popped.pop().is_some() {}
    assert!(popped.blocks().is_empty());
}

#[test]
fn one_edit_on_a_large_list_costs_a_small_fraction_of_a_vector_copy() {
    for count in [1_000usize, 10_000, 100_000] {
        let before: SharedVec<u64> = (0..count as u64).collect();
        let mut after = before.clone();
        assert!(after.set(count / 3, 0).is_ok());
        let cost = after.bytes_not_in(&before);
        let vector_copy = count * size_of::<u64>();
        assert!(cost > 64 * size_of::<u64>(), "the copied leaf is counted");
        if count >= 10_000 {
            assert!(cost * 20 < vector_copy, "{count}: {cost} bytes against {vector_copy}");
        }
    }
}

type Payload = Arc<Vec<u8>>;

fn key(value: &Payload) -> *const Vec<u8> {
    Arc::as_ptr(value)
}

fn payload(value: &Payload) -> usize {
    value.capacity()
}

#[test]
fn a_large_new_payload_is_counted_in_full() {
    let before: SharedVec<Payload> = (0..10_000).map(|_| Arc::new(vec![0u8; 16])).collect();
    let mut after = before.clone();
    assert!(after.set(4_321, Arc::new(vec![0u8; 10 << 20])).is_ok());
    let charged = after.bytes_not_in_with(&before, key, payload);
    assert!(charged >= 10 << 20);
    assert!(charged < (10 << 20) + 4_096, "{charged}");

    let fresh: SharedVec<Payload> = (0..100).map(|_| Arc::new(vec![0u8; 100 << 10])).collect();
    assert!(fresh.bytes_not_in_with(&SharedVec::new(), key, payload) >= 100 * (100 << 10));
    // The same allocation held twice is counted once.
    let twice: SharedVec<Payload> = [Arc::new(vec![0u8; 1 << 20])].iter().cycle().take(2).cloned().collect();
    assert!(twice.bytes_not_in_with(&SharedVec::new(), key, payload) < 2 << 20);
}

#[test]
fn unchanged_elements_of_a_copied_or_rebuilt_leaf_are_not_counted() {
    let size = 16 << 10;
    let before: SharedVec<Payload> = (0..1_000).map(|_| Arc::new(vec![0u8; size])).collect();

    // A set copies a leaf of 64 elements; only the new element's payload is new.
    let mut after = before.clone();
    assert!(after.set(500, Arc::new(vec![0u8; size])).is_ok());
    let charged = after.bytes_not_in_with(&before, key, payload);
    assert!(charged >= size && charged < 2 * size, "{charged}");

    // A removal moves elements between leaves without allocating any payload.
    let mut after = before.clone();
    after.retain(|value| !Arc::ptr_eq(value, &before.get(500).cloned().unwrap_or_default()));
    after.truncate(900);
    let removed = after.splice(10..20, []);
    assert_eq!(removed.len(), 10);
    let charged = after.bytes_not_in_with(&before, key, payload);
    assert!(charged < size, "{charged}");
    assert_eq!(charged, after.bytes_not_in(&before));
}

#[test]
fn many_edits_to_one_list_stay_far_below_vector_copies() {
    // One edit per element of a 10,000-element list, each kept beside the one before it, the way an
    // undo history keeps snapshots. A vector copied per edit would cost 10,000 * 80,000 bytes.
    let count = 10_000usize;
    let mut list: SharedVec<u64> = (0..count as u64).collect();
    let mut charged = 0usize;
    for index in 0..count {
        let before = list.clone();
        assert!(list.set(index, 0).is_ok());
        charged += list.bytes_not_in(&before);
    }
    let vector_copies = count * count * size_of::<u64>();
    assert!(charged * 20 < vector_copies, "{charged} bytes against {vector_copies}");
}
