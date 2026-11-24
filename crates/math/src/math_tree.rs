//! Structural access to an equation tree: the arguments of a node, and every run in a tree.

use crate::math::{Arg, MAX_DEPTH, MNode, MRun};

/// Number of child arguments of a node.
pub fn child_count(n: &MNode) -> usize {
    match n {
        MNode::Run(_) => 0,
        MNode::Frac { .. } | MNode::Func { .. } | MNode::Lim { .. } => 2,
        MNode::Rad { .. } => 2,
        MNode::Script { .. } | MNode::Nary { .. } => 3,
        MNode::Delim { elems, .. } => elems.len(),
        MNode::EqArr { rows } => rows.len(),
        MNode::Matrix { rows, .. } => rows.iter().map(|r| r.len()).sum(),
        MNode::Acc { .. } | MNode::Bar { .. } | MNode::BorderBox { .. } | MNode::Boxed { .. } | MNode::GroupChr { .. } | MNode::Phant { .. } => 1,
    }
}

/// Child argument `i` of a node (see [`child_count`] for the numbering: fraction num 0 / den 1;
/// script base 0 / sub 1 / sup 2; radical degree 0 / base 1; n-ary sub 0 / sup 1 / operand 2;
/// function name 0 / argument 1; limit base 0 / limit 1; matrices row by row).
pub fn child(n: &MNode, i: usize) -> Option<&Arg> {
    match (n, i) {
        (MNode::Frac { num, .. }, 0) => Some(num),
        (MNode::Frac { den, .. }, 1) => Some(den),
        (MNode::Script { base, .. }, 0) => Some(base),
        (MNode::Script { sub, .. }, 1) => Some(sub),
        (MNode::Script { sup, .. }, 2) => Some(sup),
        (MNode::Rad { deg, .. }, 0) => Some(deg),
        (MNode::Rad { e, .. }, 1) => Some(e),
        (MNode::Nary { sub, .. }, 0) => Some(sub),
        (MNode::Nary { sup, .. }, 1) => Some(sup),
        (MNode::Nary { e, .. }, 2) => Some(e),
        (MNode::Delim { elems, .. }, i) => elems.get(i),
        (MNode::Func { name, .. }, 0) => Some(name),
        (MNode::Func { e, .. }, 1) => Some(e),
        (MNode::Lim { e, .. }, 0) => Some(e),
        (MNode::Lim { lim, .. }, 1) => Some(lim),
        (
            MNode::Acc { e, .. }
            | MNode::Bar { e, .. }
            | MNode::BorderBox { e, .. }
            | MNode::Boxed { e }
            | MNode::GroupChr { e, .. }
            | MNode::Phant { e, .. },
            0,
        ) => Some(e),
        (MNode::EqArr { rows }, i) => rows.get(i),
        (MNode::Matrix { rows, .. }, i) => rows.iter().flatten().nth(i),
        _ => None,
    }
}

pub fn child_mut(n: &mut MNode, i: usize) -> Option<&mut Arg> {
    match (n, i) {
        (MNode::Frac { num, .. }, 0) => Some(num),
        (MNode::Frac { den, .. }, 1) => Some(den),
        (MNode::Script { base, .. }, 0) => Some(base),
        (MNode::Script { sub, .. }, 1) => Some(sub),
        (MNode::Script { sup, .. }, 2) => Some(sup),
        (MNode::Rad { deg, .. }, 0) => Some(deg),
        (MNode::Rad { e, .. }, 1) => Some(e),
        (MNode::Nary { sub, .. }, 0) => Some(sub),
        (MNode::Nary { sup, .. }, 1) => Some(sup),
        (MNode::Nary { e, .. }, 2) => Some(e),
        (MNode::Delim { elems, .. }, i) => elems.get_mut(i),
        (MNode::Func { name, .. }, 0) => Some(name),
        (MNode::Func { e, .. }, 1) => Some(e),
        (MNode::Lim { e, .. }, 0) => Some(e),
        (MNode::Lim { lim, .. }, 1) => Some(lim),
        (
            MNode::Acc { e, .. }
            | MNode::Bar { e, .. }
            | MNode::BorderBox { e, .. }
            | MNode::Boxed { e }
            | MNode::GroupChr { e, .. }
            | MNode::Phant { e, .. },
            0,
        ) => Some(e),
        (MNode::EqArr { rows }, i) => rows.get_mut(i),
        (MNode::Matrix { rows, .. }, i) => rows.iter_mut().flatten().nth(i),
        _ => None,
    }
}

/// Call `f` on every run in the tree.
pub fn each_run_mut(root: &mut Arg, f: &mut dyn FnMut(&mut MRun)) {
    fn go(a: &mut Arg, f: &mut dyn FnMut(&mut MRun), depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for n in a.iter_mut() {
            if let MNode::Run(r) = n {
                f(r);
            }
            for c in 0..child_count(n) {
                if let Some(ch) = child_mut(n, c) {
                    go(ch, f, depth + 1);
                }
            }
        }
    }
    go(root, f, 0);
}
