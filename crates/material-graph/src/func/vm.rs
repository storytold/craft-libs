// SPDX-License-Identifier: MIT OR Apache-2.0
//! Runs a [`Code`]. The rule of the graph language, which the tree walk in `reference.rs` states and the tests compare with:
//!
//! > A node runs on demand, once per run; each step of a loop starts anew, so what a pass demands runs again in it. An
//! > `f_set` starts nothing anew: what was computed from a variable before it keeps its value for the rest of the pass.
//!
//! So what remains to save is what can't change from run to run: nodes that depend on nothing, on this binding or on this run
//! keep their value that long (the classes in [`super::code`]). Whatever reads a variable or has a side effect lasts one
//! pass. That can't be stretched: a pass fixes what a node reads at its first demand, for it and everything below it,
//! so a node would have to be visited in a pass even when its variables haven't changed.

use super::code::{Code, FLOOR_ENV, FLOOR_PASS, FLOOR_RUN, FLOORS, Inst, NONE, build};
use super::{
    B, FnEnv, FnState, Num, Op, Prog, Value, binary, build_vec_ref, clamp3, clean, coerce, div_large, kind_of, lerp3, lum, map1, rnd,
    sample_filtered, smooth3, swizzle,
};
use crate::graph::Pin;
use crate::lcg_rand as sr;
use crate::tex::get;

type Fail = Box<String>;
type R = Result<(), Fail>;

/// The two low bits of a register's stamp hold its validity class, fixed during compilation.
const CLOCK_STEP: u64 = 4;
const FLOOR_MASK: u64 = CLOCK_STEP - 1;

/// What a register the program doesn't have reads as. `code::build` gives every operand a register, so only a broken
/// table could ask for one; reading this keeps that from being a panic.
static NO_VALUE: Value = Value::Bool(false);

/// The value in register `i`.
#[inline(always)]
fn reg(fr: &Frame, i: usize) -> &Value {
    fr.values.get(i).unwrap_or(&NO_VALUE)
}

/// Entry `i` of one of the code's tables.
#[inline(always)]
fn entry<T: Copy>(table: &[T], i: u32) -> Option<T> {
    table.get(i as usize).copied()
}

#[cold]
fn no_instruction(i: usize) -> Fail {
    Box::new(format!("no instruction {i}"))
}

/// What a worker needs to run a [`Prog`]: registers and what is known to be valid. One per thread; made for one program,
/// and kept over many runs because that is what makes values last.
pub struct Frame {
    values: Vec<Value>,
    /// Each stamp combines its clock value with the register's validity class. A cache hit needs only this
    /// compact table and the frame's floors, without also fetching the code or the register's value.
    stamps: Vec<u64>,
    floors: [u64; FLOORS],
    clock: u64,
    vars: Vec<Value>,
    wrote: Vec<u64>,
    rng: u32,
    pos: [f64; 2],
    /// the compiled-graph generator's state (`lcg_rand`), once it has started
    lcg_state: Option<u32>,
    args: Vec<Option<Value>>,
    /// Image positions in the bound environment; None as a whole in a frame for a nested function call.
    images: Option<Vec<Option<usize>>>,
    kids: Vec<Option<Box<Frame>>>,
    run_start: u64,
    #[cfg(test)]
    pub(super) counts: Counts,
}

/// How much work a run did, for tests that want to see that it stays small.
#[cfg(test)]
#[derive(Default, Debug, Clone, Copy)]
pub(super) struct Counts {
    pub computed: u64,
}

impl Frame {
    fn new(c: &Code, kids: usize) -> Frame {
        let mut values = vec![NO_VALUE.clone(); c.insts.len()];
        let mut stamps: Vec<_> = c.floor.iter().copied().map(u64::from).collect();
        for (i, v) in &c.literals {
            if let (Some(value), Some(stamp)) = (values.get_mut(*i as usize), stamps.get_mut(*i as usize)) {
                *value = v.clone();
                *stamp |= CLOCK_STEP;
            }
        }
        Frame {
            values,
            stamps,
            floors: [CLOCK_STEP; FLOORS],
            clock: 2 * CLOCK_STEP,
            vars: vec![Value::float(0.0); c.var_names.len()],
            wrote: vec![0; c.var_names.len()],
            rng: 0,
            pos: [0.0; 2],
            lcg_state: None,
            args: vec![None; c.arg_names.len()],
            images: None,
            kids: (0..kids).map(|_| None).collect(),
            run_start: 0,
            #[cfg(test)]
            counts: Counts::default(),
        }
    }

    fn set_floor(&mut self, floor: u32, t: u64) {
        if let Some(f) = self.floors.get_mut(floor as usize) {
            *f = t;
        }
    }

    /// Everything computed from the caller's environment stops counting.
    fn new_binding(&mut self) {
        self.clock += CLOCK_STEP;
        self.set_floor(FLOOR_ENV, self.clock);
    }

    fn new_run(&mut self, pos: [f64; 2]) {
        self.clock += CLOCK_STEP;
        let t = self.clock;
        self.set_floor(FLOOR_RUN, t);
        self.set_floor(FLOOR_PASS, t);
        for v in &mut self.vars {
            *v = Value::float(0.0);
        }
        self.rng = 0;
        self.lcg_state = None;
        self.pos = pos;
        self.run_start = t;
    }

    /// A pass of a loop begins or ends: nodes with side effects and what reads variables run again.
    #[inline]
    fn new_pass(&mut self) {
        self.clock += CLOCK_STEP;
        self.set_floor(FLOOR_PASS, self.clock);
    }
}

impl Prog {
    pub(super) fn code(&self) -> &Code {
        self.code.get_or_init(|| build(self))
    }

    pub(super) fn uniform_value(&self, env: &FnEnv, frame: &mut Frame, index: usize) -> Option<Value> {
        ensure(self, self.code(), index, env, frame).ok()?;
        frame.values.get(index).cloned()
    }

    /// Registers for running this program; reuse them over many runs (and bind again when the environment changes).
    pub fn frame(&self) -> Frame {
        let c = self.code();
        Frame::new(c, self.nodes.len())
    }

    /// Tell `frame` about a new environment: what was computed from the old one is not used again.
    pub fn bind(&self, env: &FnEnv, frame: &mut Frame) {
        let c = self.code();
        for (slot, name) in frame.args.iter_mut().zip(&c.arg_names) {
            *slot = env.args.get(name).cloned();
        }
        if !c.samples.is_empty() {
            let images = frame.images.get_or_insert_with(Vec::new);
            images.clear();
            images.extend(c.samples.iter().map(|sample| env.images.iter().position(|(name, _)| *name == sample.slot)));
        }
        frame.new_binding();
    }

    /// Run the function at `pos` (uv, 0..1).
    pub fn run(&self, env: &FnEnv, pos: [f64; 2]) -> Result<Value, String> {
        let mut f = self.frame();
        self.bind(env, &mut f);
        self.run_bound(env, pos, &mut f)
    }

    /// As [`Prog::run`], in a frame that was last bound to `env`. Anything computed for an earlier run that still holds is
    /// not computed again.
    pub fn run_bound(&self, env: &FnEnv, pos: [f64; 2], fr: &mut Frame) -> Result<Value, String> {
        fr.new_run(pos);
        let c = self.code();
        self.go(c, env, fr)
    }

    /// Run the function with variables and random sequence carried in `state` (read before, updated after).
    pub fn run_in(&self, env: &FnEnv, pos: [f64; 2], state: &mut FnState) -> Result<Value, String> {
        let mut f = self.frame();
        self.bind(env, &mut f);
        self.run_in_bound(env, pos, state, &mut f)
    }

    /// As [`Prog::run_in`], in a frame that was last bound to `env`.
    pub fn run_in_bound(&self, env: &FnEnv, pos: [f64; 2], state: &mut FnState, fr: &mut Frame) -> Result<Value, String> {
        fr.new_run(pos);
        let c = self.code();
        fr.rng = state.rng;
        fr.lcg_state = state.lcg_state;
        for (k, name) in c.var_names.iter().enumerate() {
            if let Some((_, v)) = state.vars.iter().find(|(n, _)| n == name)
                && let Some(d) = fr.vars.get_mut(k)
            {
                *d = v.clone();
            }
        }
        let r = self.go(c, env, fr);
        for (k, name) in c.var_names.iter().enumerate() {
            if fr.wrote.get(k).is_some_and(|w| *w >= fr.run_start)
                && let Some(v) = fr.vars.get(k)
            {
                match state.vars.iter_mut().find(|(n, _)| n == name) {
                    Some(slot) => slot.1 = v.clone(),
                    None => state.vars.push((name.clone(), v.clone())),
                }
            }
        }
        state.rng = fr.rng;
        state.lcg_state = fr.lcg_state;
        r
    }

    fn go(&self, c: &Code, env: &FnEnv, fr: &mut Frame) -> Result<Value, String> {
        ensure(self, c, c.out as usize, env, fr).map_err(|e| *e)?;
        Ok(reg(fr, c.out as usize).clone())
    }
}

#[inline(always)]
fn ensure(p: &Prog, c: &Code, i: usize, env: &FnEnv, fr: &mut Frame) -> R {
    if let Some(&stamp) = fr.stamps.get(i)
        && stamp >= fr.floors.get((stamp & FLOOR_MASK) as usize).copied().unwrap_or(u64::MAX)
    {
        return Ok(());
    }
    compute(p, c, i, env, fr)
}

fn compute(p: &Prog, c: &Code, i: usize, env: &FnEnv, fr: &mut Frame) -> R {
    let Some(inst) = c.insts.get(i) else { return Err(no_instruction(i)) };
    if let Err(e) = body_inner(p, c, i, inst, env, fr) {
        return Err(wrap(p, i, e));
    }
    if let Some(stamp) = fr.stamps.get_mut(i) {
        *stamp = fr.clock | (*stamp & FLOOR_MASK);
    }
    #[cfg(test)]
    {
        fr.counts.computed += 1;
    }
    Ok(())
}

#[cold]
fn wrap(p: &Prog, i: usize, e: Fail) -> Fail {
    if e.starts_with("node ") {
        return e;
    }
    let Some(n) = p.nodes.get(i) else { return e };
    Box::new(format!("node {} ({}): {e}", n.id, kind_of(n.op)))
}

#[inline(always)]
fn scalar_float(v: &Value) -> Option<f64> {
    match v {
        Value::Num(Num { lanes: 1, int: false, v }) => Some(v[0]),
        _ => None,
    }
}

#[inline(always)]
fn float1(x: f64) -> Value {
    Value::Num(Num { lanes: 1, int: false, v: [clean(x), 0.0, 0.0, 0.0] })
}

/// Binary operation with the common case (two float scalars) handled directly; everything else as [`binary`] has it.
#[inline(always)]
fn fast_binary(op: B, x: &Value, y: &Value) -> Result<Value, String> {
    if let (Some(p), Some(q)) = (scalar_float(x), scalar_float(y)) {
        match op {
            B::Add => return Ok(float1(p + q)),
            B::Sub => return Ok(float1(p - q)),
            B::Mul => return Ok(float1(p * q)),
            B::Div => return Ok(float1(if q != 0.0 { p / q } else { 0.0 })),
            B::DivLarge => return Ok(float1(div_large(p, q))),
            B::Step => return Ok(float1(if q < p { 0.0 } else { 1.0 })),
            B::Min => return Ok(float1(p.min(q))),
            B::Max => return Ok(float1(p.max(q))),
            B::Lt => return Ok(Value::Bool(p < q)),
            B::Le => return Ok(Value::Bool(p <= q)),
            B::Gt => return Ok(Value::Bool(p > q)),
            B::Ge => return Ok(Value::Bool(p >= q)),
            _ => {}
        }
    }
    // float vectors, or a vector and a scalar: the same lane by lane
    if let (Value::Num(a), Value::Num(b)) = (x, y)
        && !a.int
        && !b.int
        && (a.lanes == b.lanes || a.lanes == 1 || b.lanes == 1)
        && matches!(op, B::Add | B::Sub | B::Mul | B::DivLarge | B::Min | B::Max)
    {
        let lanes = a.lanes.max(b.lanes);
        let mut v = [0.0; 4];
        for (i, o) in v.iter_mut().enumerate().take(lanes as usize) {
            let (p, q) = (if a.lanes == 1 { a.v[0] } else { get(&a.v, i) }, if b.lanes == 1 { b.v[0] } else { get(&b.v, i) });
            *o = clean(match op {
                B::Add => p + q,
                B::Sub => p - q,
                B::DivLarge => div_large(p, q),
                B::Min => p.min(q),
                B::Max => p.max(q),
                _ => p * q,
            });
        }
        return Ok(Value::Num(Num { lanes, int: false, v }));
    }
    binary(op, x, y)
}

/// Swizzle with a mask known to be plain letters.
#[inline(always)]
fn fast_swizzle(x: &Value, mask: Option<([u8; 4], u8)>, text: &str) -> Result<Value, String> {
    if let (Value::Num(n), Some((idx, len))) = (x, mask) {
        let mut v = [0.0; 4];
        let lanes = n.lanes as usize;
        for (o, at) in v.iter_mut().zip(idx).take(len as usize) {
            let at = at as usize;
            if at >= lanes && !(lanes == 1 && at == 0) {
                return swizzle(x, text);
            }
            *o = get(&n.v, at);
        }
        return Ok(Value::Num(Num { lanes: len, int: n.int, v }));
    }
    swizzle(x, text)
}

/// Clones an operand: what impure nodes do, since a later operand can run this one again.
#[inline(always)]
fn val(p: &Prog, c: &Code, j: u32, env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    ensure(p, c, j as usize, env, fr)?;
    Ok(reg(fr, j as usize).clone())
}

#[inline(always)]
fn opt_val(p: &Prog, c: &Code, j: u32, env: &FnEnv, fr: &mut Frame) -> Result<Option<Value>, Fail> {
    if j == NONE { Ok(None) } else { val(p, c, j, env, fr).map(Some) }
}

#[inline(always)]
fn body_inner(p: &Prog, c: &Code, i: usize, inst: &Inst, env: &FnEnv, fr: &mut Frame) -> R {
    let [a, b, ..] = inst.ins;
    let (au, bu) = (a as usize, b as usize);
    let r: Value = match inst.op {
        // a literal holds its value from the start; `f_pin` and `f_out` are read through
        Op::Float | Op::Int | Op::Bool | Op::Str => return Ok(()),
        Op::Out | Op::Pin => {
            ensure(p, c, au, env, fr)?;
            reg(fr, au).clone()
        }
        Op::Swizzle => {
            ensure(p, c, au, env, fr)?;
            fast_swizzle(reg(fr, au), entry(&c.swizzles, inst.x).flatten(), p.nodes.get(i).map_or("", |n| n.text.as_str()))?
        }
        Op::Un(u) => {
            ensure(p, c, au, env, fr)?;
            super::unary(u, reg(fr, au))?
        }
        Op::Bin(o) => {
            if inst.impure {
                let x = val(p, c, a, env, fr)?;
                ensure(p, c, bu, env, fr)?;
                fast_binary(o, &x, reg(fr, bu))?
            } else {
                ensure(p, c, au, env, fr)?;
                ensure(p, c, bu, env, fr)?;
                fast_binary(o, reg(fr, au), reg(fr, bu))?
            }
        }
        Op::Seq => {
            ensure(p, c, au, env, fr)?;
            ensure(p, c, bu, env, fr)?;
            reg(fr, bu).clone()
        }
        // the rest in a function of its own: a chain of the operations above recurses through this frame only, so it
        // stays small (a deep function's run takes this frame once per node one behind the other)
        _ => body_rest(p, c, i, inst, env, fr)?,
    };
    if let Some(value) = fr.values.get_mut(i) {
        *value = if p.single { super::to_single(r) } else { r };
    }
    Ok(())
}

/// The operations [`body_inner`] doesn't do itself.
#[inline(never)]
fn body_rest(p: &Prog, c: &Code, i: usize, inst: &Inst, env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    let [a, b, d, e] = inst.ins;
    let (au, bu, du) = (a as usize, b as usize, d as usize);
    Ok(match inst.op {
        Op::Vec(lanes, int) => {
            let defaults = entry(&c.vec_defaults, inst.x).unwrap_or([0.0; 4]);
            if inst.impure {
                impure_vec(p, c, inst, lanes, int, defaults, env, fr)?
            } else {
                for j in inst.ins {
                    if j != NONE {
                        ensure(p, c, j as usize, env, fr)?;
                    }
                }
                let part = |j: u32| if j == NONE { None } else { Some(reg(fr, j as usize)) };
                build_vec_ref(lanes, int, [part(a), part(b), part(d), part(e)], defaults)?
            }
        }
        Op::Clamp | Op::Lerp | Op::Smooth => {
            let f = match inst.op {
                Op::Clamp => clamp3,
                Op::Lerp => lerp3,
                _ => smooth3,
            };
            if inst.impure {
                let (x, y) = (val(p, c, a, env, fr)?, val(p, c, b, env, fr)?);
                ensure(p, c, du, env, fr)?;
                f(&x, &y, reg(fr, du))?
            } else {
                ensure(p, c, au, env, fr)?;
                ensure(p, c, bu, env, fr)?;
                ensure(p, c, du, env, fr)?;
                f(reg(fr, au), reg(fr, bu), reg(fr, du))?
            }
        }
        Op::If => {
            ensure(p, c, au, env, fr)?;
            let pick = match reg(fr, au) {
                Value::Bool(true) => bu,
                Value::Bool(false) => du,
                _ => return Err(Box::new("the condition must be a boolean".to_string())),
            };
            ensure(p, c, pick, env, fr)?;
            reg(fr, pick).clone()
        }
        Op::Loop => {
            ensure(p, c, au, env, fr)?;
            // every pass reads the variables anew, so nothing computed in the pass before counts
            for pass in 0..entry(&c.passes, inst.x).unwrap_or(0) {
                // a compiled graph's loop can run billions of passes: it stops when the evaluation is cancelled
                if pass & 0xfff == 0xfff && crate::context::cancelled() {
                    return Err(Box::new("cancelled".to_string()));
                }
                fr.new_pass();
                if d != NONE {
                    ensure(p, c, du, env, fr)?;
                    // a condition read back from a variable comes as a number
                    match reg(fr, du) {
                        Value::Bool(true) => break,
                        Value::Num(x) if x.v[0] != 0.0 => break,
                        Value::Str(_) => return Err(Box::new("the loop's condition must be a boolean".to_string())),
                        _ => {}
                    }
                }
                fr.new_pass();
                ensure(p, c, bu, env, fr)?;
            }
            fr.new_pass();
            Value::float(0.0)
        }
        Op::Set => {
            ensure(p, c, au, env, fr)?;
            let v = reg(fr, au).clone();
            if let Some(w) = fr.wrote.get_mut(inst.x as usize) {
                *w = fr.clock;
            }
            if let Some(d) = fr.vars.get_mut(inst.x as usize) {
                *d = v.clone();
            }
            v
        }
        // with a declared type the value takes that shape: reading two lanes of a variable set with four gives the first two
        Op::Get => {
            let v = fr.vars.get(inst.x as usize).unwrap_or(&NO_VALUE);
            match entry(&c.get_pins, inst.y).flatten() {
                Some(pin) if matches!(v, Value::Num(_)) && !matches!(pin, Pin::Bool | Pin::Text) => coerce(v, pin),
                _ => v.clone(),
            }
        }
        Op::Random => random(p, c, i, env, fr)?,
        Op::SampleGray | Op::SampleColor => sample(p, c, inst, env, fr)?,
        Op::Stat(k) => stat(p, i, k, env)?,
        Op::Pos => Value::vec(2, [fr.pos[0], fr.pos[1], 0.0, 0.0]),
        Op::Size => Value::vec(2, [env.size[0], env.size[1], 0.0, 0.0]),
        Op::OwnSize => {
            let o = env.own.unwrap_or(env.size);
            Value::vec(2, [o[0], o[1], 0.0, 0.0])
        }
        Op::CellSize => Value::float(env.cell[0]),
        Op::CellDepth => Value::float(env.cell[1]),
        Op::Index => Value::int(env.index),
        Op::Count => Value::int(env.count),
        Op::Seed => Value::int(env.seed as i64),
        Op::Input | Op::GraphInput => input(p, c, i, env, fr),
        Op::ToFloat => {
            ensure(p, c, au, env, fr)?;
            let n = reg(fr, au).num()?;
            Value::Num(Num { int: false, ..n })
        }
        Op::ToInt => {
            ensure(p, c, au, env, fr)?;
            let n = reg(fr, au).num()?;
            map1(&n, true, |x| x.trunc())
        }
        Op::ToBool => {
            ensure(p, c, au, env, fr)?;
            Value::Bool(match reg(fr, au) {
                Value::Bool(t) => *t,
                Value::Str(s) => !s.is_empty(),
                Value::Num(n) => n.v.iter().take(n.lanes as usize).any(|x| *x != 0.0),
            })
        }
        Op::ToStr | Op::Concat => text_op(p, c, i, env, fr)?,
        Op::Call => call(p, c, i, env, fr)?,
        // done by body_inner
        Op::Float | Op::Int | Op::Bool | Op::Str | Op::Out | Op::Pin | Op::Swizzle | Op::Un(_) | Op::Bin(_) | Op::Seq => reg(fr, i).clone(),
    })
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn impure_vec(p: &Prog, c: &Code, inst: &Inst, lanes: u8, int: bool, defaults: [f64; 4], env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    let [a, b, d, e] = inst.ins;
    let parts = [opt_val(p, c, a, env, fr)?, opt_val(p, c, b, env, fr)?, opt_val(p, c, d, env, fr)?, opt_val(p, c, e, env, fr)?];
    let refs = [parts[0].as_ref(), parts[1].as_ref(), parts[2].as_ref(), parts[3].as_ref()];
    Ok(build_vec_ref(lanes, int, refs, defaults)?)
}

#[inline(never)]
fn random(p: &Prog, c: &Code, i: usize, env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    let Some(inst) = c.insts.get(i) else { return Err(no_instruction(i)) };
    let [a, b, s, _] = inst.ins;
    if let Some((mode, k)) = entry(&c.lcg_draws, inst.x).flatten() {
        let hi = if b == NONE { 1.0 } else { val(p, c, b, env, fr)?.as_f32() as f64 };
        let seed = match opt_val(p, c, s, env, fr)? {
            Some(Value::Num(x)) => x.v[0] as i64 as u32,
            _ => 0,
        };
        let state = match mode {
            // a value processor's, computed where it is read: draw number `k` from its own start
            2 => (0..k).fold(sr::pixel_state(seed, 0.0, 0.0), |s, _| sr::step(sr::FUNCTION, s)),
            // a size expression's: runs once, so the same draw at every pixel
            5 => (0..k).fold(seed & 0xff_ffff, |s, _| sr::step(sr::PARAMETER, s)),
            1 => {
                let s = fr.lcg_state.unwrap_or_else(|| sr::pixel_state(seed, fr.pos[0] as f32, fr.pos[1] as f32));
                sr::step(sr::FUNCTION, s)
            }
            3 => sr::step(sr::PARAMETER, fr.lcg_state.unwrap_or(seed & 0xff_ffff)),
            // a fx-map's expression: the program set the state
            m => {
                let site = sr::FX_SITES.get((m - 10).clamp(0, sr::FX_SITES.len() as i32 - 1) as usize).copied().unwrap_or(sr::FUNCTION);
                sr::step(site, fr.lcg_state.unwrap_or(0))
            }
        };
        if !super::code::indexed_lcg_draw(mode) {
            fr.lcg_state = Some(state);
        }
        return Ok(Value::float(hi * sr::output(state)));
    }
    let lo = if a == NONE { 0.0 } else { val(p, c, a, env, fr)?.as_f32() as f64 };
    let hi = if b == NONE { 1.0 } else { val(p, c, b, env, fr)?.as_f32() as f64 };
    fr.rng = fr.rng.wrapping_add(1);
    let seed = env.seed.wrapping_add(entry(&c.seeds, inst.x).unwrap_or(0)).wrapping_add(fr.rng.wrapping_mul(7919));
    let pos = fr.pos;
    let id = p.nodes.get(i).map_or(0, |n| n.id);
    let r = rnd((pos[0] * 4096.0) as i32 ^ (id as i32).wrapping_mul(131), (pos[1] * 4096.0) as i32 ^ env.index as i32, seed) as f64;
    Ok(Value::float(lo + (hi - lo) * r))
}

#[inline(never)]
fn sample(p: &Prog, c: &Code, inst: &Inst, env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    let au = inst.ins[0] as usize;
    ensure(p, c, au, env, fr)?;
    let (u, v) = match reg(fr, au) {
        Value::Num(Num { lanes: 2, v, .. }) => (v[0] as f32, v[1] as f32),
        Value::Str(_) => return Err(Box::new("a string cannot be used as a number".to_string())),
        _ => return Err(Box::new("the sample position must be a float2".to_string())),
    };
    let info = c.samples.get(inst.x as usize);
    let image = match &fr.images {
        Some(bound) => entry(bound, inst.x).flatten().and_then(|index| env.images.get(index).map(|(_, image)| *image)),
        None => info.and_then(|info| env.image(&info.slot)),
    };
    // A slot without an image reads zero.
    let s = match (image, info.and_then(|info| info.filter)) {
        (None, _) => [0.0; 4],
        (Some(t), Some(f)) => sample_filtered(t, u, v, f),
        (Some(t), None) => t.sample(u, v),
    };
    Ok(if inst.op == Op::SampleGray { Value::float(lum(s) as f64) } else { Value::vec(4, [s[0] as f64, s[1] as f64, s[2] as f64, s[3] as f64]) })
}

#[inline(never)]
fn stat(p: &Prog, i: usize, k: u8, env: &FnEnv) -> Result<Value, Fail> {
    let name = p.nodes.get(i).map_or("", |n| n.name.as_str());
    let slot = if name.is_empty() { "in" } else { name };
    let s = env.stats.iter().find(|s| s.0 == slot).ok_or_else(|| format!("no input image '{slot}' for the statistic"))?;
    let q = match k {
        0 => s.1,
        1 => s.2,
        _ => s.3,
    };
    Ok(Value::vec(4, [q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64]))
}

#[inline(never)]
fn input(p: &Prog, c: &Code, i: usize, env: &FnEnv, fr: &Frame) -> Value {
    let Some((inst, info)) = c.insts.get(i).and_then(|inst| Some((inst, c.inputs.get(inst.x as usize)?))) else { return NO_VALUE.clone() };
    let from = if inst.op == Op::Input {
        fr.args.get(info.arg as usize).and_then(Option::as_ref)
    } else {
        env.graph_inputs.get(p.nodes.get(i).map_or("", |n| n.name.as_str()))
    };
    match from {
        Some(v) => coerce(v, info.pin),
        None => info.default.clone(),
    }
}

#[inline(never)]
fn text_op(p: &Prog, c: &Code, i: usize, env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    let Some(inst) = c.insts.get(i) else { return Err(no_instruction(i)) };
    let [a, b, ..] = inst.ins;
    if inst.op == Op::ToStr {
        ensure(p, c, a as usize, env, fr)?;
        return Ok(Value::text(&reg(fr, a as usize).show()));
    }
    let x = val(p, c, a, env, fr)?;
    ensure(p, c, b as usize, env, fr)?;
    match (&x, reg(fr, b as usize)) {
        (Value::Str(s), Value::Str(t)) => Ok(Value::text(&format!("{s}{t}"))),
        _ => Err(Box::new("f_concat needs two strings".to_string())),
    }
}

/// `f_call`: the callee gets variables and a random sequence of its own, so all the caller sees is the result.
#[inline(never)]
fn call(p: &Prog, c: &Code, i: usize, env: &FnEnv, fr: &mut Frame) -> Result<Value, Fail> {
    let callee = p.nodes.get(i).and_then(|n| n.callee.as_ref()).ok_or_else(|| "call without a function".to_string())?;
    let cc = callee.code();
    let mut kid = fr.kids.get_mut(i).and_then(Option::take).unwrap_or_else(|| Box::new(callee.frame()));
    kid.args.fill(None);
    let Some(inst) = c.insts.get(i) else { return Err(no_instruction(i)) };
    let mut failed = None;
    for (k, ((_, pin, _), operand)) in callee.inputs.iter().zip(inst.ins).enumerate() {
        match opt_val(p, c, operand, env, fr) {
            Ok(Some(v)) => {
                if let Some(arg) = cc.input_args.get(k).and_then(|a| kid.args.get_mut(*a as usize)) {
                    *arg = Some(coerce(&v, *pin));
                }
            }
            Ok(None) => {}
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failed {
        if let Some(k) = fr.kids.get_mut(i) {
            *k = Some(kid);
        }
        return Err(e);
    }
    kid.new_binding();
    kid.new_run(fr.pos);
    let r = callee.go(cc, env, &mut kid);
    if let Some(k) = fr.kids.get_mut(i) {
        *k = Some(kid);
    }
    Ok(r?)
}
