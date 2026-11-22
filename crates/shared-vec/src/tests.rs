//! The tree's shape after every operation, against `Vec` as the model.

use proptest::prelude::*;

use crate::SharedVec;
use crate::node::{self, MAX};

#[derive(Clone, Debug)]
enum Op {
    Push(u32),
    Pop,
    Insert(usize, u32),
    Remove(usize),
    Set(usize, u32),
    SplitOffAndAppend(usize),
    Truncate(usize),
    Splice(usize, usize, Vec<u32>),
    Retain(u32),
    /// Removes the element at an index through `retain`.
    RetainOne(usize),
    /// Splits off a few elements from the front (`true`) or the back, and appends the rest again.
    SplitOffFew(usize, bool),
    /// Truncates a few elements.
    TruncateFew(usize),
    AppendClone,
    Snapshot,
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => any::<u32>().prop_map(Op::Push),
        2 => Just(Op::Pop),
        6 => (any::<usize>(), any::<u32>()).prop_map(|(at, value)| Op::Insert(at, value)),
        6 => any::<usize>().prop_map(Op::Remove),
        3 => (any::<usize>(), any::<u32>()).prop_map(|(at, value)| Op::Set(at, value)),
        1 => any::<usize>().prop_map(Op::SplitOffAndAppend),
        1 => any::<usize>().prop_map(Op::Truncate),
        2 => (any::<usize>(), 0..200usize, prop::collection::vec(any::<u32>(), 0..200)).prop_map(|(at, len, with)| Op::Splice(at, len, with)),
        1 => (2..40u32).prop_map(Op::Retain),
        // Range edits of a few elements, which a large enough list makes as single edits.
        2 => (any::<usize>(), 0..3usize, prop::collection::vec(any::<u32>(), 0..3)).prop_map(|(at, len, with)| Op::Splice(at, len, with)),
        2 => any::<usize>().prop_map(Op::RetainOne),
        2 => (0..3usize, any::<bool>()).prop_map(|(count, front)| Op::SplitOffFew(count, front)),
        1 => (0..3usize).prop_map(Op::TruncateFew),
        1 => Just(Op::AppendClone),
        2 => Just(Op::Snapshot),
        1 => Just(Op::Clear),
    ]
}

/// Runs `ops` on a list and on a `Vec`, checking the shape and the contents after each, and that
/// every snapshot taken along the way still holds what it held.
fn run(start: usize, ops: &[Op]) -> Result<(), TestCaseError> {
    let mut model: Vec<u32> = (0..start as u32).collect();
    let mut list: SharedVec<u32> = model.iter().copied().collect();
    let mut snapshots: Vec<(SharedVec<u32>, Vec<u32>)> = Vec::new();
    for op in ops {
        let len = model.len();
        match op.clone() {
            Op::Push(value) => {
                model.push(value);
                list.push(value);
            }
            Op::Pop => prop_assert_eq!(list.pop(), model.pop()),
            Op::Insert(at, value) => {
                let at = at % (len + 1);
                model.insert(at, value);
                prop_assert_eq!(list.insert(at, value), Ok(()));
                prop_assert_eq!(list.insert(len + 2, value), Err(value));
            }
            Op::Remove(at) => {
                if len > 0 {
                    let at = at % len;
                    prop_assert_eq!(list.remove(at), Some(model.remove(at)));
                }
                prop_assert_eq!(list.remove(model.len()), None);
            }
            Op::Set(at, value) => {
                if len > 0 {
                    let at = at % len;
                    let old = std::mem::replace(&mut model[at], value);
                    prop_assert_eq!(list.set(at, value), Ok(old));
                }
                prop_assert_eq!(list.set(len, value), Err(value));
            }
            Op::SplitOffAndAppend(at) => {
                let at = at % (len + 1);
                let mut back = list.split_off(at);
                node::check(&list.root).map_err(TestCaseError::fail)?;
                node::check(&back.root).map_err(TestCaseError::fail)?;
                prop_assert_eq!(list.to_vec(), model[..at].to_vec());
                prop_assert_eq!(back.to_vec(), model[at..].to_vec());
                list.append(&mut back);
                prop_assert!(back.is_empty());
            }
            Op::Truncate(to) => {
                let to = to % (len + 1);
                model.truncate(to);
                list.truncate(to);
            }
            Op::Splice(at, count, with) => {
                let start = at % (len + 1);
                let end = (start + count).min(len);
                let expected: Vec<u32> = model.splice(start..end, with.iter().copied()).collect();
                prop_assert_eq!(list.splice(start..end, with), expected);
            }
            Op::Retain(modulus) => {
                model.retain(|value| value % modulus != 0);
                list.retain(|value| value % modulus != 0);
            }
            Op::RetainOne(at) => {
                if len > 0 {
                    let at = at % len;
                    model.remove(at);
                    let mut index = 0;
                    list.retain(|_| {
                        index += 1;
                        index != at + 1
                    });
                }
            }
            Op::SplitOffFew(count, front) => {
                let at = if front { count.min(len) } else { len.saturating_sub(count) };
                let mut back = list.split_off(at);
                node::check(&list.root).map_err(TestCaseError::fail)?;
                node::check(&back.root).map_err(TestCaseError::fail)?;
                prop_assert_eq!(list.to_vec(), model[..at].to_vec());
                prop_assert_eq!(back.to_vec(), model[at..].to_vec());
                list.append(&mut back);
                prop_assert!(back.is_empty());
            }
            Op::TruncateFew(count) => {
                model.truncate(len.saturating_sub(count));
                list.truncate(len.saturating_sub(count));
            }
            Op::AppendClone if len < 20_000 => {
                let mut copy = list.clone();
                list.append(&mut copy);
                model.extend_from_within(..);
            }
            Op::AppendClone => {}
            Op::Snapshot => snapshots.push((list.clone(), model.clone())),
            Op::Clear => {
                model.clear();
                list.clear();
            }
        }
        node::check(&list.root).map_err(TestCaseError::fail)?;
        prop_assert_eq!(list.len(), model.len());
        prop_assert!(list.iter().eq(model.iter()));
        prop_assert!(list.iter().rev().eq(model.iter().rev()));
    }
    for (snapshot, expected) in &snapshots {
        node::check(&snapshot.root).map_err(TestCaseError::fail)?;
        prop_assert_eq!(&snapshot.to_vec(), expected);
    }
    prop_assert_eq!(Vec::from(list), model);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn small_lists_match_vec(start in 0..300usize, ops in prop::collection::vec(op(), 1..120)) {
        run(start, &ops)?;
    }

    // Past 64 * 64 elements the tree has two levels of branches.
    #[test]
    fn two_level_lists_match_vec(start in 4_000..9_000usize, ops in prop::collection::vec(op(), 1..60)) {
        run(start, &ops)?;
    }

    #[test]
    fn mixed_ends_of_the_iterator_meet_once(start in 0..3_000usize, picks in prop::collection::vec(any::<bool>(), 0..3_000)) {
        let model: Vec<u32> = (0..start as u32).collect();
        let list: SharedVec<u32> = model.iter().copied().collect();
        let (mut ours, mut theirs) = (list.iter(), model.iter());
        for from_back in picks {
            let (a, b) = if from_back { (ours.next_back(), theirs.next_back()) } else { (ours.next(), theirs.next()) };
            prop_assert_eq!(a, b);
            prop_assert_eq!(ours.len(), theirs.len());
        }
        let collect = |acc: Vec<u32>, value: &u32| { let mut acc = acc; acc.push(*value); acc };
        prop_assert_eq!(ours.clone().fold(Vec::new(), collect), theirs.clone().fold(Vec::new(), collect));
        prop_assert_eq!(ours.clone().rfold(Vec::new(), collect), theirs.clone().rfold(Vec::new(), collect));
        prop_assert!(ours.eq(theirs));
    }
}

#[test]
fn a_three_level_tree_keeps_its_shape_through_removals_and_insertions() -> Result<(), String> {
    let count = MAX * MAX * MAX + 5;
    let mut model: Vec<u32> = (0..count as u32).collect();
    let mut list: SharedVec<u32> = model.iter().copied().collect();
    let before = list.clone();
    let mut seed = 0x2545_f491_u64;
    for step in 0..30_000u32 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let at = (seed % model.len() as u64) as usize;
        if step % 3 == 0 {
            model.insert(at, step);
            list.insert(at, step).map_err(|_| "insert refused")?;
        } else {
            let removed = list.remove(at);
            if removed != Some(model.remove(at)) {
                return Err(format!("step {step}: removed {removed:?}"));
            }
        }
    }
    node::check(&list.root)?;
    node::check(&before.root)?;
    if !list.iter().eq(model.iter()) {
        return Err("contents differ from the model".into());
    }
    if !before.iter().copied().eq(0..count as u32) {
        return Err("the clone changed".into());
    }
    Ok(())
}
