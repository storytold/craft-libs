//! The engine of compiled graphs computes one lane at a time, on demand, each lane of each instruction once per run (measured:
//! a random number behind a vector lane nobody reads isn't drawn, and the lane read first draws first, through vectors,
//! swizzles, arithmetic and `ifelse`). What needs a whole vector, a `set` or a `dot`, asks for lane x of everything below it, then
//! lane y: for `set(mul(vec(p, q), vec(r, s)))` the draws are p, r, q, s (measured; `dot` too). That decides which random numbers
//! go where, so an expression that draws is rewritten here into one lane per instruction, which then runs in the same order.

use super::functions::has_effect;
use crate::decode::graph_package::compiled::{Expr, Instr, NO_OPERAND, Operands, VALUE_TYPE_BITS, ValueType, swizzle_source};
use std::collections::HashMap;

/// In the type of a `sequence` written here: its first operand runs although it sets nothing.
pub const KEEP: u8 = 0x80;

// Format data: the expression opcodes this rewrite emits (the compiled graph's numbering, `asm::expr` `OPS`).
pub(super) const OP_SEQUENCE: u8 = 0x0c;
const OP_VECTOR: u8 = 0x0d;
const OP_SWIZZLE: u8 = 0x10;

struct Rewrite<'a> {
    e: &'a Expr,
    effect: Vec<bool>,
    /// Whether a `rand` is at or below each instruction.
    draws: Vec<bool>,
    out: Vec<Instr>,
    lanes: HashMap<(u16, u8), u16>,
    whole: HashMap<u16, u16>,
}

/// Whether an operation works lane by lane on its operands.
fn lane_wise(name: &str) -> bool {
    matches!(
        name,
        "add"
            | "sub"
            | "mul"
            | "div"
            | "mod"
            | "neg"
            | "abs"
            | "floor?"
            | "ceil?"
            | "cos"
            | "sin"
            | "tan"
            | "sqrt?"
            | "log?"
            | "exp?"
            | "log2"
            | "pow2"
            | "pow"
            | "lerp"
            | "min"
            | "max"
            | "cast"
            | "ifelse"
    )
}

impl Rewrite<'_> {
    fn emit(&mut self, i: Instr) -> u16 {
        self.out.push(i);
        (self.out.len() - 1) as u16
    }

    fn count(&self, r: u16) -> u8 {
        match self.e.instrs.get(usize::from(r)) {
            Some(i) if i.value_type() != ValueType::Bool => i.components,
            _ => 1,
        }
    }

    /// The new instruction giving lane `l` of old instruction `r`. A scalar has one lane, whichever is asked for.
    fn lane(&mut self, r: u16, l: u8) -> u16 {
        if r == NO_OPERAND {
            return r;
        }
        let n = self.count(r);
        let l = if n == 1 { 0 } else { l.min(n - 1) };
        if let Some(x) = self.lanes.get(&(r, l)) {
            return *x;
        }
        let Some(ins) = self.e.instrs.get(usize::from(r)) else { return NO_OPERAND };
        let new = match (ins.name(), &ins.operands) {
            ("const", Operands::Constant(w)) if n > 1 => {
                self.emit(Instr { components: 1, operands: Operands::Constant(vec![w.get(usize::from(l)).copied().unwrap_or(0)]), ..ins.clone() })
            }
            ("vector" | "vector3?" | "vector4", Operands::Refs(refs)) => {
                let (mut at, mut found) = (0u8, None);
                for x in refs {
                    let c = self.count(*x);
                    if l < at + c {
                        found = Some(self.lane(*x, l - at));
                        break;
                    }
                    at += c;
                }
                found.unwrap_or(NO_OPERAND)
            }
            ("swizzle", Operands::RefSlot(a, m)) => {
                let from = if ins.value_type() == ValueType::Bool || n == 1 && self.count(*a) == 1 { 0 } else { swizzle_source(*m, u16::from(l)) };
                self.lane(*a, from)
            }
            ("sequence", Operands::Refs(refs)) if let &[a, b] = refs.as_slice() => {
                // a first operand without a `set` doesn't run, but what it shares with others does (`arms`), so it stays when it draws
                if self.effect.get(usize::from(a)).copied().unwrap_or(false) || self.draws.get(usize::from(a)).copied().unwrap_or(false) {
                    let first = self.all(a);
                    let then = self.lane(b, l);
                    let ty = self.out.get(usize::from(then)).map_or(ins.ty, |t| t.ty);
                    self.emit(Instr { op: ins.op, components: 1, ty, operands: Operands::Refs(vec![first, then]) })
                } else {
                    self.lane(b, l)
                }
            }
            (name, Operands::Refs(refs)) if lane_wise(name) => {
                let new: Vec<u16> = refs.iter().map(|x| self.lane(*x, l)).collect();
                self.emit(Instr { components: 1, operands: Operands::Refs(new), ..ins.clone() })
            }
            _ => {
                let w = self.run_whole(r);
                if n == 1 { w } else { self.emit(Instr { op: OP_SWIZZLE, components: 1, ty: ins.ty, operands: Operands::RefSlot(w, u16::from(l)) }) }
            }
        };
        self.lanes.insert((r, l), new);
        new
    }

    /// All lanes of old instruction `r` as one value.
    fn all(&mut self, r: u16) -> u16 {
        if r == NO_OPERAND {
            return r;
        }
        let n = self.count(r);
        if n == 1 {
            return self.lane(r, 0);
        }
        let ty = self.e.instrs.get(usize::from(r)).map_or(1, |i| i.ty);
        let refs: Vec<u16> = (0..n).map(|l| self.lane(r, l)).collect();
        self.emit(Instr { op: OP_VECTOR, components: n, ty, operands: Operands::Refs(refs) })
    }

    /// An instruction that needs its operands whole, as it stands.
    fn run_whole(&mut self, r: u16) -> u16 {
        if let Some(x) = self.whole.get(&r) {
            return *x;
        }
        let Some(ins) = self.e.instrs.get(usize::from(r)) else { return NO_OPERAND };
        // several vectors at once: lane x of each, then lane y (measured on `dot`), before the operation has them whole
        let mut before = Vec::new();
        if let Operands::Refs(refs) = &ins.operands
            && refs.iter().filter(|x| self.draws.get(usize::from(**x)).copied().unwrap_or(false)).count() > 0
            && refs.iter().any(|x| self.count(*x) > 1)
        {
            let most = refs.iter().map(|x| self.count(*x)).max().unwrap_or(1);
            for l in 0..most {
                for x in refs {
                    if l < self.count(*x) && self.draws.get(usize::from(*x)).copied().unwrap_or(false) {
                        before.push(self.lane(*x, l));
                    }
                }
            }
        }
        let operands = match &ins.operands {
            Operands::Refs(refs) => Operands::Refs(refs.iter().map(|x| self.all(*x)).collect()),
            Operands::RefSlot(a, k) => Operands::RefSlot(self.all(*a), *k),
            Operands::Loop(refs, passes) => Operands::Loop(refs.iter().map(|x| self.all(*x)).collect(), *passes),
            Operands::Sample(p, image, filter) => Operands::Sample(self.all(*p), *image, *filter),
            other => other.clone(),
        };
        let mut new = self.emit(Instr { operands, ..ins.clone() });
        for d in before.into_iter().rev() {
            new = self.emit(Instr {
                op: OP_SEQUENCE,
                components: ins.components,
                ty: (ins.ty & VALUE_TYPE_BITS) | KEEP,
                operands: Operands::Refs(vec![d, new]),
            });
        }
        self.whole.insert(r, new);
        new
    }
}

/// `e` lane by lane if it draws random numbers and has anything with more than one lane; `None` if that changes nothing.
pub fn scalarize(e: &Expr) -> Option<Expr> {
    let draws = e.instrs.iter().any(|i| i.name() == "rand");
    if !draws || !e.instrs.iter().any(|i| i.value_type() != ValueType::Bool && i.components > 1) {
        return None;
    }
    let root = e.instrs.len().checked_sub(1)? as u16;
    let mut below = vec![false; e.instrs.len()];
    for (i, ins) in e.instrs.iter().enumerate() {
        let at = |r: &u16| below.get(usize::from(*r)).copied().unwrap_or(false);
        let v = ins.name() == "rand"
            || match &ins.operands {
                Operands::Refs(r) | Operands::Loop(r, _) => r.iter().any(at),
                Operands::RefSlot(a, _) => at(a),
                Operands::Sample(p, ..) => at(p),
                _ => false,
            };
        if let Some(b) = below.get_mut(i) {
            *b = v;
        }
    }
    let mut w = Rewrite { e, effect: has_effect(e), draws: below, out: Vec::new(), lanes: HashMap::new(), whole: HashMap::new() };
    let top = w.all(root);
    // the last instruction is the result
    if usize::from(top) + 1 != w.out.len() {
        let ins = e.instrs.get(usize::from(root))?;
        let ty = w.out.get(usize::from(top)).map_or(ins.ty, |t| t.ty);
        w.emit(Instr { op: OP_SEQUENCE, components: ins.components, ty, operands: Operands::Refs(vec![top, top]) });
    }
    Some(Expr { offset: e.offset, end: e.end, instrs: w.out })
}

#[cfg(test)]
mod tests {
    use super::scalarize;
    use crate::decode::graph_package::compiled::{Expr, Instr, Operands};

    const FLOAT: u8 = 9;
    fn ins(op: u8, components: u8, operands: Operands) -> Instr {
        Instr { op, components, ty: FLOAT, operands }
    }
    fn refs(r: &[u16]) -> Operands {
        Operands::Refs(r.to_vec())
    }
    fn expr(instrs: Vec<Instr>) -> Expr {
        Expr { offset: 0, end: 0, instrs }
    }
    fn draws(e: &Expr) -> usize {
        e.instrs.iter().filter(|i| i.name() == "rand").count()
    }

    #[test]
    fn a_random_number_behind_a_lane_nobody_reads_is_gone() {
        // %0 const, %1 rand, %2 rand, %3 vector(%1, %2), %4 swizzle y
        let e = expr(vec![
            ins(0x00, 1, Operands::Constant(vec![0x3f80_0000])),
            ins(0x32, 1, refs(&[0])),
            ins(0x32, 1, refs(&[0])),
            ins(0x0d, 2, refs(&[1, 2])),
            ins(0x10, 1, Operands::RefSlot(3, 1)),
        ]);
        let s = scalarize(&e).unwrap();
        assert_eq!(draws(&s), 1);
    }

    #[test]
    fn lanes_follow_arithmetic_and_a_set_keeps_them_all() {
        let through = |last: Instr| {
            expr(vec![
                ins(0x00, 1, Operands::Constant(vec![0x3f80_0000])),
                ins(0x32, 1, refs(&[0])),
                ins(0x32, 1, refs(&[0])),
                ins(0x0d, 2, refs(&[1, 2])),
                ins(0x17, 2, refs(&[3])),
                last,
                ins(0x10, 1, Operands::RefSlot(5, 0)),
            ])
        };
        let neg = scalarize(&through(ins(0x17, 2, refs(&[4])))).unwrap();
        assert_eq!(draws(&neg), 1);
        let set = scalarize(&through(ins(0x07, 2, Operands::RefSlot(4, 0)))).unwrap();
        assert_eq!(draws(&set), 2);
    }

    /// `rand`s of the expression by what they read as their bound, in the order the expression runs them.
    fn run_order(e: &Expr) -> Vec<u32> {
        fn walk(e: &Expr, i: u16, seen: &mut Vec<u16>, out: &mut Vec<u32>) {
            if seen.contains(&i) {
                return;
            }
            seen.push(i);
            let ins = &e.instrs[usize::from(i)];
            match &ins.operands {
                Operands::Refs(r) => r.iter().for_each(|x| walk(e, *x, seen, out)),
                Operands::RefSlot(a, _) => walk(e, *a, seen, out),
                _ => {}
            }
            if ins.op == 0x32
                && let Operands::Refs(r) = &ins.operands
                && let Operands::Constant(w) = &e.instrs[usize::from(r[0])].operands
            {
                out.push(w[0]);
            }
        }
        let (mut seen, mut out) = (Vec::new(), Vec::new());
        walk(e, (e.instrs.len() - 1) as u16, &mut seen, &mut out);
        out
    }

    /// %0..%3 bounds, %4..%7 rands, %8 vec(%4, %5), %9 vec(%6, %7)
    fn four_draws(mut rest: Vec<Instr>) -> Expr {
        let mut v: Vec<Instr> = (1..=4).map(|k| ins(0x00, 1, Operands::Constant(vec![k]))).collect();
        v.extend((0..4).map(|k| ins(0x32, 1, refs(&[k]))));
        v.push(ins(0x0d, 2, refs(&[4, 5])));
        v.push(ins(0x0d, 2, refs(&[6, 7])));
        v.append(&mut rest);
        expr(v)
    }

    #[test]
    fn a_set_of_a_vector_runs_lane_by_lane() {
        let e = four_draws(vec![ins(0x14, 2, refs(&[8, 9])), ins(0x07, 2, Operands::RefSlot(10, 0))]);
        assert_eq!(run_order(&scalarize(&e).unwrap()), vec![1, 3, 2, 4]);
    }

    #[test]
    fn a_dot_runs_lane_by_lane() {
        let e = four_draws(vec![ins(0x18, 1, refs(&[8, 9]))]);
        assert_eq!(run_order(&scalarize(&e).unwrap()), vec![1, 3, 2, 4]);
    }

    #[test]
    fn an_expression_without_random_numbers_stays() {
        let e = expr(vec![ins(0x00, 2, Operands::Constant(vec![0, 0])), ins(0x10, 1, Operands::RefSlot(0, 0))]);
        assert!(scalarize(&e).is_none());
    }
}
