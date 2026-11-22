// SPDX-License-Identifier: MIT OR Apache-2.0
//! The tree walk that defines what function graphs mean, node by node with a memo that every pass of a loop clears. It stays
//! here so the tests can hold [`super::vm`] to it.

use super::*;

struct State {
    memo: Vec<Option<(Value, u64)>>,
    epoch: u64,
    vars: Vec<(String, Value)>,
    rng: u32,
    /// The state of the compiled-graph generator (`lcg_rand`), once it has started.
    lcg_state: Option<u32>,
    steps: u64,
    /// steps this run may take before it gives up with [`OVER_BUDGET`]: some real functions would take longer than the universe has
    budget: u64,
}

pub(super) const OVER_BUDGET: &str = "node over budget";

impl Prog {
    /// Run the function at `pos` (uv, 0..1).
    pub(super) fn ref_run(&self, env: &FnEnv, pos: [f64; 2]) -> Result<Value, String> {
        self.ref_run_budget(env, pos, u64::MAX)
    }

    /// [`Self::ref_run`] that gives up after `budget` steps.
    pub(super) fn ref_run_budget(&self, env: &FnEnv, pos: [f64; 2], budget: u64) -> Result<Value, String> {
        let mut st = State { memo: vec![None; self.nodes.len()], epoch: 0, vars: Vec::new(), rng: 0, lcg_state: None, steps: 0, budget };
        self.eval(self.out, env, pos, &mut st)
    }

    /// Run the function with variables and random sequence carried in `state` (read before, updated after).
    pub(super) fn ref_run_in(&self, env: &FnEnv, pos: [f64; 2], state: &mut FnState) -> Result<Value, String> {
        let mut st = State {
            memo: vec![None; self.nodes.len()],
            epoch: 0,
            vars: std::mem::take(&mut state.vars),
            rng: state.rng,
            lcg_state: state.lcg_state,
            steps: 0,
            budget: u64::MAX,
        };
        let r = self.eval(self.out, env, pos, &mut st);
        state.vars = st.vars;
        state.rng = st.rng;
        state.lcg_state = st.lcg_state;
        r
    }

    fn eval(&self, i: usize, env: &FnEnv, pos: [f64; 2], st: &mut State) -> Result<Value, String> {
        if let Some((v, e)) = &st.memo[i]
            && *e == st.epoch
        {
            return Ok(v.clone());
        }
        st.steps += 1;
        if st.steps > st.budget {
            return Err(OVER_BUDGET.into());
        }
        let n = &self.nodes[i];
        let r = self
            .eval_node(n, env, pos, st)
            .map_err(|e| if e.starts_with("node ") { e } else { format!("node {} ({}): {e}", n.id, kind_of(n.op)) })?;
        let r = if self.single { super::to_single(r) } else { r };
        st.memo[i] = Some((r.clone(), st.epoch));
        Ok(r)
    }

    fn arg(&self, n: &PNode, k: usize, env: &FnEnv, pos: [f64; 2], st: &mut State) -> Result<Value, String> {
        match n.ins[k] {
            Some(j) => self.eval(j, env, pos, st),
            None => Err("an input is not connected".into()),
        }
    }

    fn opt_arg(&self, n: &PNode, k: usize, env: &FnEnv, pos: [f64; 2], st: &mut State) -> Result<Option<Value>, String> {
        match n.ins[k] {
            Some(j) => self.eval(j, env, pos, st).map(Some),
            None => Ok(None),
        }
    }

    fn eval_loop(&self, n: &PNode, env: &FnEnv, pos: [f64; 2], st: &mut State) -> Result<Value, String> {
        self.arg(n, 0, env, pos, st)?;
        let passes = super::loop_passes(&n.num);
        // every pass reads the variables anew, so nothing computed in the pass before counts
        for _ in 0..passes {
            st.epoch += 1;
            // a condition read back from a variable comes as a number
            match self.opt_arg(n, 2, env, pos, st)? {
                Some(Value::Bool(true)) => break,
                Some(Value::Num(x)) if x.v[0] != 0.0 => break,
                Some(Value::Str(_)) => return Err("the loop's condition must be a boolean".into()),
                _ => {}
            }
            st.epoch += 1;
            self.arg(n, 1, env, pos, st)?;
        }
        st.epoch += 1;
        Ok(Value::float(0.0))
    }

    fn eval_if(&self, n: &PNode, env: &FnEnv, pos: [f64; 2], st: &mut State) -> Result<Value, String> {
        match self.arg(n, 0, env, pos, st)? {
            Value::Bool(true) => self.arg(n, 1, env, pos, st),
            Value::Bool(false) => self.arg(n, 2, env, pos, st),
            _ => Err("the condition must be a boolean".into()),
        }
    }

    fn eval_node(&self, n: &PNode, env: &FnEnv, pos: [f64; 2], st: &mut State) -> Result<Value, String> {
        let num = |k: &str| n.num.get(k).copied().unwrap_or(0.0);
        Ok(match n.op {
            Op::Out | Op::Pin => self.arg(n, 0, env, pos, st)?,
            Op::Float => Value::float(num("v")),
            Op::Int => Value::int(num("v").trunc() as i64),
            Op::Bool => Value::Bool(num("v") != 0.0),
            Op::Str => Value::text(&n.text),
            Op::Vec(lanes, int) => {
                let mut parts: [Option<Value>; 4] = [None, None, None, None];
                for (k, p) in parts.iter_mut().enumerate() {
                    *p = self.opt_arg(n, k, env, pos, st)?;
                }
                build_vec_ref(
                    lanes,
                    int,
                    [parts[0].as_ref(), parts[1].as_ref(), parts[2].as_ref(), parts[3].as_ref()],
                    [num("x"), num("y"), num("z"), num("w")],
                )?
            }
            Op::Swizzle => swizzle(&self.arg(n, 0, env, pos, st)?, &n.text)?,
            Op::Un(u) => unary(u, &self.arg(n, 0, env, pos, st)?)?,
            Op::Bin(b) => {
                let (x, y) = (self.arg(n, 0, env, pos, st)?, self.arg(n, 1, env, pos, st)?);
                binary(b, &x, &y)?
            }
            Op::Clamp => {
                let (x, y, z) = (self.arg(n, 0, env, pos, st)?, self.arg(n, 1, env, pos, st)?, self.arg(n, 2, env, pos, st)?);
                clamp3(&x, &y, &z)?
            }
            Op::Lerp => {
                let (x, y, z) = (self.arg(n, 0, env, pos, st)?, self.arg(n, 1, env, pos, st)?, self.arg(n, 2, env, pos, st)?);
                lerp3(&x, &y, &z)?
            }
            Op::Smooth => {
                let (x, y, z) = (self.arg(n, 0, env, pos, st)?, self.arg(n, 1, env, pos, st)?, self.arg(n, 2, env, pos, st)?);
                smooth3(&x, &y, &z)?
            }
            Op::If => self.eval_if(n, env, pos, st)?,
            Op::Seq => {
                self.arg(n, 0, env, pos, st)?;
                self.arg(n, 1, env, pos, st)?
            }
            Op::Loop => self.eval_loop(n, env, pos, st)?,
            Op::Set => {
                let v = self.arg(n, 0, env, pos, st)?;
                match st.vars.iter_mut().find(|(k, _)| *k == n.name) {
                    Some(slot) => slot.1 = v.clone(),
                    None => st.vars.push((n.name.clone(), v.clone())),
                }
                // nothing runs again for this: outside a loop every instruction runs once per run (measured)
                v
            }
            // with a declared type the value takes that shape: reading two lanes of a variable set with four gives the first two
            Op::Get => {
                let v = st.vars.iter().find(|(k, _)| *k == n.name).map(|(_, v)| v.clone()).unwrap_or_else(|| Value::float(0.0));
                match num("type").round() as i32 {
                    0 if !n.num.contains_key("type") => v,
                    t if matches!(v, Value::Num(_)) && !matches!(type_pin(t), Pin::Bool | Pin::Text) => coerce(&v, type_pin(t)),
                    _ => v,
                }
            }
            Op::Random if n.num.contains_key("lcg") => {
                let hi = self.opt_arg(n, 1, env, pos, st)?.map(|v| v.as_f32() as f64).unwrap_or(1.0);
                let seed = match self.opt_arg(n, 2, env, pos, st)? {
                    Some(Value::Num(x)) => x.v[0] as i64 as u32,
                    _ => 0,
                };
                use crate::lcg_rand as sr;
                let state = match num("lcg") as i32 {
                    // a value processor's, computed where it is read: draw number `k` from its own start
                    2 => (0..num("k") as u32).fold(sr::pixel_state(seed, 0.0, 0.0), |s, _| sr::step(sr::FUNCTION, s)),
                    // a size expression's: runs once, so the same draw at every pixel
                    5 => (0..num("k") as u32).fold(seed & 0xff_ffff, |s, _| sr::step(sr::PARAMETER, s)),
                    1 => {
                        let s = st.lcg_state.unwrap_or_else(|| sr::pixel_state(seed, pos[0] as f32, pos[1] as f32));
                        sr::step(sr::FUNCTION, s)
                    }
                    3 => sr::step(sr::PARAMETER, st.lcg_state.unwrap_or(seed & 0xff_ffff)),
                    // a fx-map's expression: the program set the state
                    m => sr::step(sr::FX_SITES[(m - 10).clamp(0, sr::FX_SITES.len() as i32 - 1) as usize], st.lcg_state.unwrap_or(0)),
                };
                if !matches!(num("lcg") as i32, 2 | 5) {
                    st.lcg_state = Some(state);
                }
                Value::float(hi * sr::output(state))
            }
            Op::Random => {
                let lo = self.opt_arg(n, 0, env, pos, st)?.map(|v| v.as_f32() as f64).unwrap_or(0.0);
                let hi = self.opt_arg(n, 1, env, pos, st)?.map(|v| v.as_f32() as f64).unwrap_or(1.0);
                st.rng = st.rng.wrapping_add(1);
                let seed = env.seed.wrapping_add(num("seed") as u32).wrapping_add(st.rng.wrapping_mul(7919));
                let r = rnd((pos[0] * 4096.0) as i32 ^ (n.id as i32).wrapping_mul(131), (pos[1] * 4096.0) as i32 ^ env.index as i32, seed) as f64;
                Value::float(lo + (hi - lo) * r)
            }
            Op::SampleGray | Op::SampleColor => {
                let p = self.arg(n, 0, env, pos, st)?.num()?;
                if p.lanes != 2 {
                    return Err("the sample position must be a float2".into());
                }
                let slot = if n.name.is_empty() { "in" } else { n.name.as_str() };
                // a slot nothing is connected to reads 0, as in `vm`
                let (u, v) = (p.v[0] as f32, p.v[1] as f32);
                let c = match (env.image(slot), n.num.get("filter")) {
                    (None, _) => [0.0; 4],
                    (Some(t), Some(f)) => sample_filtered(t, u, v, *f as u32),
                    (Some(t), None) => t.sample(u, v),
                };
                if n.op == Op::SampleGray { Value::float(lum(c) as f64) } else { Value::vec(4, [c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64]) }
            }
            Op::Stat(k) => {
                let slot = if n.name.is_empty() { "in" } else { n.name.as_str() };
                let s = env.stats.iter().find(|s| s.0 == slot).ok_or_else(|| format!("no input image '{slot}' for the statistic"))?;
                let a = match k {
                    0 => s.1,
                    1 => s.2,
                    _ => s.3,
                };
                Value::vec(4, [a[0] as f64, a[1] as f64, a[2] as f64, a[3] as f64])
            }
            Op::Pos => Value::vec(2, [pos[0], pos[1], 0.0, 0.0]),
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
            Op::Input | Op::GraphInput => input_value(n, env),
            Op::ToFloat => {
                let v = self.arg(n, 0, env, pos, st)?;
                let nn = v.num()?;
                Value::Num(Num { int: false, ..nn })
            }
            Op::ToInt => {
                let nn = self.arg(n, 0, env, pos, st)?.num()?;
                map1(&nn, true, |x| x.trunc())
            }
            Op::ToBool => {
                let v = self.arg(n, 0, env, pos, st)?;
                Value::Bool(match &v {
                    Value::Bool(b) => *b,
                    Value::Str(s) => !s.is_empty(),
                    Value::Num(nn) => (0..nn.lanes as usize).any(|i| nn.v[i] != 0.0),
                })
            }
            Op::ToStr => Value::text(&self.arg(n, 0, env, pos, st)?.show()),
            Op::Concat => match (self.arg(n, 0, env, pos, st)?, self.arg(n, 1, env, pos, st)?) {
                (Value::Str(a), Value::Str(b)) => Value::text(&format!("{a}{b}")),
                _ => return Err("f_concat needs two strings".into()),
            },
            Op::Call => {
                let callee = n.callee.as_ref().ok_or("call without a function")?;
                let mut sub_env = FnEnv {
                    size: env.size,
                    index: env.index,
                    own: env.own,
                    cell: env.cell,
                    count: env.count,
                    seed: env.seed,
                    images: env.images.clone(),
                    graph_inputs: env.graph_inputs,
                    args: BTreeMap::new(),
                    stats: env.stats.clone(),
                };
                for (k, (name, pin, _)) in callee.inputs.iter().enumerate().take(4) {
                    if let Some(v) = self.opt_arg(n, k, env, pos, st)? {
                        sub_env.args.insert(name.clone(), coerce(&v, *pin));
                    }
                }
                callee.ref_run_budget(&sub_env, pos, st.budget)?
            }
        })
    }
}

fn input_value(n: &PNode, env: &FnEnv) -> Value {
    let num = |k: &str| n.num.get(k).copied().unwrap_or(0.0);
    let pin = type_pin(num("type").round() as i32);
    let default = {
        let g = |k: &str| num(k);
        match pin {
            Pin::Bool => Value::Bool(g("v") != 0.0),
            Pin::Text => Value::text(&n.text),
            p if p.lanes() == 1 => coerce(&Value::float(g("v")), p),
            p => coerce(&Value::vec(p.lanes() as u8, [g("x"), g("y"), g("z"), g("w")]), p),
        }
    };
    let from = if n.op == Op::Input { env.args.get(&n.name) } else { env.graph_inputs.get(&n.name) };
    match from {
        Some(v) => coerce(v, pin),
        None => default,
    }
}
