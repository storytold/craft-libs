//! What runs although the arm holding it isn't taken. Measured on the engine of compiled graphs, in fx-maps and pixel processors
//! alike: an `ifelse` skips only what belongs to the arm alone. An instruction read from anywhere else too belongs to the
//! place all its readers share, so it runs there even when every reader is in an arm not taken.
//!
//! * Each arm of an `ifelse` is a place inside the place of the `ifelse`. So is the first operand of a `sequence` that doesn't run
//!   (no `set` below it): it never runs, but reading from there still counts as reading.
//! * An instruction's place is the innermost one holding all readers that the result can reach. A reader nothing reaches doesn't
//!   count, and neither does the same reader reading twice.
//! * It runs in operand order: condition, first arm, second arm, with what belongs to an arm not taken skipped and the rest run
//!   where it stands. So what leaves the first arm runs before the second is taken, and what leaves the second after the first.
//!
//! Places go by lane, since the engine runs lanes one by one ([`super::scalar`] has split them by the time this reads them).

use crate::decode::graph_package::compiled::{Expr, Operands};

const NO_PLACE: u32 = u32::MAX;

/// The places of one expression. Only expressions that draw or set anything have work to do here.
pub(super) struct Arms<'a> {
    e: &'a Expr,
    /// place of each instruction; `NO_PLACE` if the result doesn't reach it
    place: Vec<u32>,
    /// parent of each place; place 0 is the expression itself
    parent: Vec<u32>,
    depth: Vec<u32>,
    /// first and second arm of an instruction, as places
    arm_at: Vec<[u32; 2]>,
    /// a `rand` or a `set` at or below each instruction: running it can be told apart from not
    matters: Vec<bool>,
}

fn operands(e: &Expr, i: usize) -> Vec<u16> {
    match e.instrs.get(i).map(|ins| &ins.operands) {
        Some(Operands::Refs(r) | Operands::Loop(r, _)) => r.iter().copied().filter(|x| usize::from(*x) < e.instrs.len()).collect(),
        Some(Operands::RefSlot(a, _) | Operands::Sample(a, ..)) => vec![*a],
        _ => Vec::new(),
    }
}

impl<'a> Arms<'a> {
    pub(super) fn new(e: &'a Expr, effect: &[bool]) -> Arms<'a> {
        let n = e.instrs.len();
        let mut matters = vec![false; n];
        for (i, ins) in e.instrs.iter().enumerate() {
            let own = matches!(ins.name(), "rand" | "set");
            let v = own || operands(e, i).iter().any(|x| matters.get(usize::from(*x)).copied().unwrap_or(false));
            if let Some(m) = matters.get_mut(i) {
                *m = v;
            }
        }
        let mut a = Arms { e, place: vec![NO_PLACE; n], parent: vec![0], depth: vec![0], arm_at: vec![[NO_PLACE; 2]; n], matters };
        if n == 0 {
            return a;
        }
        if let Some(last) = a.place.last_mut() {
            *last = 0;
        }
        for (i, ins) in e.instrs.iter().enumerate().rev() {
            let here = a.place_of(i);
            if here == NO_PLACE {
                continue;
            }
            let refs = operands(e, i);
            let direct = match &ins.operands {
                Operands::Refs(r) => Some(r.as_slice()),
                _ => None,
            };
            match (ins.name(), direct) {
                ("ifelse", Some(&[condition, first_ref, second_ref])) => {
                    let (first, second) = (a.child(here), a.child(here));
                    a.set_arms(i, [first, second]);
                    a.read(condition, here);
                    a.read(first_ref, first);
                    a.read(second_ref, second);
                }
                ("sequence", Some(&[skipped, kept]))
                    if !effect.get(usize::from(skipped)).copied().unwrap_or(false) && ins.ty & super::scalar::KEEP == 0 =>
                {
                    let dead = a.child(here);
                    a.set_arms(i, [dead, NO_PLACE]);
                    a.read(skipped, dead);
                    a.read(kept, here);
                }
                _ => refs.iter().for_each(|x| a.read(*x, here)),
            }
        }
        a
    }

    /// Place of instruction `i` (`NO_PLACE` past the end).
    fn place_of(&self, i: usize) -> u32 {
        self.place.get(i).copied().unwrap_or(NO_PLACE)
    }

    fn set_arms(&mut self, i: usize, arms: [u32; 2]) {
        if let Some(a) = self.arm_at.get_mut(i) {
            *a = arms;
        }
    }

    /// Depth of place `p`; every place this module hands out exists, so the fallback is never taken.
    fn depth_of(&self, p: u32) -> u32 {
        self.depth.get(p as usize).copied().unwrap_or(0)
    }

    /// Parent of place `p` (the expression itself for place 0, and for a place that doesn't exist).
    fn parent_of(&self, p: u32) -> u32 {
        self.parent.get(p as usize).copied().unwrap_or(0)
    }

    fn child(&mut self, of: u32) -> u32 {
        self.parent.push(of);
        self.depth.push(self.depth_of(of) + 1);
        (self.parent.len() - 1) as u32
    }

    /// `r` is read from `from`: its place is now what holds both.
    fn read(&mut self, r: u16, from: u32) {
        let Some(at) = self.place.get(usize::from(r)).copied() else { return };
        let place = if at == NO_PLACE { from } else { self.common(at, from) };
        if let Some(p) = self.place.get_mut(usize::from(r)) {
            *p = place;
        }
    }

    fn common(&self, mut a: u32, mut b: u32) -> u32 {
        while a != b {
            if self.depth_of(a) >= self.depth_of(b) {
                a = self.parent_of(a);
            } else {
                b = self.parent_of(b);
            }
        }
        a
    }

    fn inside(&self, mut p: u32, of: u32) -> bool {
        while self.depth_of(p) > self.depth_of(of) {
            p = self.parent_of(p);
        }
        p == of
    }

    /// What must run though arm `k` of instruction `i` is skipped, in operand order.
    pub(super) fn leaving(&self, i: usize, k: usize) -> Vec<u16> {
        let arm = self.arm_at.get(i).and_then(|a| a.get(k)).copied().unwrap_or(NO_PLACE);
        if arm == NO_PLACE {
            return Vec::new();
        }
        let Some(ins) = self.e.instrs.get(i) else { return Vec::new() };
        let root = match &ins.operands {
            // the operand after the condition, or a sequence's first
            Operands::Refs(r) if ins.name() == "ifelse" => r.get(k + 1),
            Operands::Refs(r) => r.first(),
            _ => None,
        };
        let Some(&root) = root else { return Vec::new() };
        let mut out = Vec::new();
        let mut seen = vec![false; self.e.instrs.len()];
        self.walk(root, arm, &mut seen, &mut out);
        out
    }

    fn walk(&self, r: u16, arm: u32, seen: &mut [bool], out: &mut Vec<u16>) {
        let at = usize::from(r);
        match seen.get_mut(at) {
            Some(s) if !*s && self.matters.get(at).copied().unwrap_or(false) => *s = true,
            _ => return,
        }
        if !self.inside(self.place_of(at), arm) {
            out.push(r);
            return;
        }
        for x in operands(self.e, at) {
            self.walk(x, arm, seen, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Arms;
    use crate::decode::graph_package::compiled::{Expr, Instr, Operands};

    const FLOAT: u8 = 9;
    const BOOL: u8 = 0;
    fn refs(op: u8, r: &[u16]) -> Instr {
        Instr { op, components: 1, ty: FLOAT, operands: Operands::Refs(r.to_vec()) }
    }
    fn konst() -> Instr {
        Instr { op: 0x00, components: 1, ty: FLOAT, operands: Operands::Constant(vec![0x3f80_0000]) }
    }
    fn truth() -> Instr {
        Instr { op: 0x00, components: 1, ty: BOOL, operands: Operands::Constant(vec![1]) }
    }
    const RAND: u8 = 0x32;
    const IF: u8 = 0x09;
    const ADD: u8 = 0x12;
    const SEQ: u8 = 0x0c;
    fn leaving(e: &Expr, i: usize, k: usize) -> Vec<u16> {
        let effect = vec![false; e.instrs.len()];
        Arms::new(e, &effect).leaving(i, k)
    }

    #[test]
    fn read_from_two_ifs_it_leaves_both_arms() {
        // %3 rand; %4 if(%1, %0, add %3); %6 if(%1, %0, add %3); %8 add
        let e = Expr {
            offset: 0,
            end: 0,
            instrs: vec![
                konst(),
                truth(),
                refs(RAND, &[0]),
                refs(ADD, &[2, 0]),
                refs(IF, &[1, 0, 3]),
                refs(ADD, &[2, 2]),
                refs(IF, &[1, 0, 5]),
                refs(ADD, &[4, 6]),
            ],
        };
        assert_eq!(leaving(&e, 4, 1), vec![2]);
        assert_eq!(leaving(&e, 6, 1), vec![2]);
    }

    #[test]
    fn read_only_from_one_arm_it_stays_there() {
        let e = Expr {
            offset: 0,
            end: 0,
            instrs: vec![konst(), truth(), refs(RAND, &[0]), refs(ADD, &[2, 2]), refs(ADD, &[3, 2]), refs(IF, &[1, 0, 4])],
        };
        assert!(leaving(&e, 5, 1).is_empty());
    }

    #[test]
    fn under_an_outer_arm_it_stays_in_that_arm() {
        // the two readers share the outer if's second arm: it leaves the inner arms, not the outer
        let e = Expr {
            offset: 0,
            end: 0,
            instrs: vec![
                konst(),
                truth(),
                refs(RAND, &[0]),
                refs(ADD, &[2, 0]),
                refs(IF, &[1, 0, 3]),
                refs(ADD, &[2, 2]),
                refs(IF, &[1, 0, 5]),
                refs(ADD, &[4, 6]),
                refs(IF, &[1, 0, 7]),
            ],
        };
        assert_eq!(leaving(&e, 4, 1), vec![2]);
        assert!(leaving(&e, 8, 1).is_empty());
    }

    #[test]
    fn a_reader_nothing_reaches_does_not_count() {
        let e = Expr {
            offset: 0,
            end: 0,
            instrs: vec![konst(), truth(), refs(RAND, &[0]), refs(ADD, &[2, 0]), refs(ADD, &[2, 2]), refs(IF, &[1, 0, 4])],
        };
        assert!(leaving(&e, 5, 1).is_empty());
    }

    #[test]
    fn a_sequence_that_skips_its_first_operand_still_reads_it() {
        // sequence(%2, if(c, 0, add %2)): the rand belongs to the sequence's place, so it runs
        let e = Expr {
            offset: 0,
            end: 0,
            instrs: vec![konst(), truth(), refs(RAND, &[0]), refs(ADD, &[2, 0]), refs(IF, &[1, 0, 3]), refs(SEQ, &[2, 4])],
        };
        assert_eq!(leaving(&e, 5, 0), vec![2]);
        assert_eq!(leaving(&e, 4, 1), vec![2]);
    }

    #[test]
    fn operand_order_in_what_leaves() {
        // arm reads %3 then %2, both read elsewhere too
        let e = Expr {
            offset: 0,
            end: 0,
            instrs: vec![
                konst(),
                truth(),
                refs(RAND, &[0]),
                refs(RAND, &[0]),
                refs(ADD, &[3, 2]),
                refs(IF, &[1, 0, 4]),
                refs(ADD, &[2, 3]),
                refs(ADD, &[5, 6]),
            ],
        };
        assert_eq!(leaving(&e, 5, 1), vec![3, 2]);
    }
}
