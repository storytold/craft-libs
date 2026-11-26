// SPDX-License-Identifier: MIT OR Apache-2.0
//! The form a function graph runs in: a flat table of instructions, one register per node, plus what is known about each
//! node without running it. A node's *class* says what its value can depend on:
//!
//! * `Const`: nothing (literals and what is computed from them only). Computed once for a [`super::vm::Frame`].
//! * `Env`: the caller's environment (size, seed, inputs, images, but not the position). Once per binding.
//! * `Run`: the position. Once per run.
//! * variables (`f_get`), or an `f_set`, `f_random` or `f_loop` (`impure`): once per run and again in each pass of a loop.
//!
//! The first three cannot be told apart from running them every time, except by the time it takes.

use super::{Op, Prog, Value, coerce, loop_passes, type_pin};
use crate::graph::Pin;
use std::collections::HashMap;

/// "No such operand / table entry".
pub const NONE: u32 = u32::MAX;

/// Indices into `Frame::floors`: a register is valid when its stamp is at or above the floor its instruction names.
pub const FLOOR_CONST: u32 = 0;
pub const FLOOR_ENV: u32 = 1;
pub const FLOOR_RUN: u32 = 2;
pub const FLOOR_PASS: u32 = 3;
pub const FLOORS: usize = 4;

/// The compiled value and size sites restart from the given seed for each indexed draw.
/// They neither read the pixel position nor change the caller's generator state.
pub(super) fn indexed_lcg_draw(site: i32) -> bool {
    matches!(site, 2 | 5)
}

pub struct Inst {
    pub op: Op,
    pub impure: bool,
    /// operands in slot order, already past any `f_pin`
    pub ins: [u32; 4],
    /// variable of an `f_set` / `f_get`, or index into the table of its kind
    pub x: u32,
    /// the type an `f_get` declares ([`Code::get_pins`])
    pub y: u32,
}

/// An `f_input` / `f_graph_input`.
pub struct InputInfo {
    pub pin: Pin,
    pub default: Value,
    /// for `f_input`, where its value is bound in `Frame::args`
    pub arg: u32,
}

/// The image slot and sampling mode of an image read.
pub struct SampleInfo {
    pub slot: String,
    pub filter: Option<u32>,
}

pub struct Code {
    pub insts: Vec<Inst>,
    /// Initial validity class of each register.
    pub floor: Vec<u32>,
    pub out: u32,
    pub literals: Vec<(u32, Value)>,
    pub vec_defaults: Vec<[f64; 4]>,
    /// reads of a swizzle: `None` when the mask is not plain (the slow path then reports the problem)
    pub swizzles: Vec<Option<([u8; 4], u8)>>,
    pub seeds: Vec<u32>,
    pub samples: Vec<SampleInfo>,
    pub inputs: Vec<InputInfo>,
    pub passes: Vec<usize>,
    /// the site and draw number `k` of a random number drawn as compiled graphs draw it (see `lcg_rand`); `None` for the plain sequence
    pub lcg_draws: Vec<Option<(i32, u32)>>,
    pub get_pins: Vec<Option<Pin>>,
    pub var_names: Vec<String>,
    pub arg_names: Vec<String>,
    /// where each of `Prog::inputs` is bound
    pub input_args: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Const,
    Env,
    Run,
    Vars,
    Impure,
}

pub fn build(p: &Prog) -> Code {
    let n = p.nodes.len();
    // an `f_pin` / `f_out` is its source: both finish together, so both are valid together
    let mut alias: Vec<u32> = (0..n as u32).collect();
    for (i, node) in p.nodes.iter().enumerate() {
        // (a node's inputs come before it)
        if matches!(node.op, Op::Out | Op::Pin)
            && let Some(j) = node.ins[0]
            && let Some(a) = alias.get(j).copied()
            && let Some(d) = alias.get_mut(i)
        {
            *d = a;
        }
    }
    let mut c = Code {
        insts: Vec::with_capacity(n),
        floor: vec![FLOOR_CONST; n],
        out: alias.get(p.out).copied().unwrap_or(NONE),
        literals: Vec::new(),
        vec_defaults: Vec::new(),
        swizzles: Vec::new(),
        seeds: Vec::new(),
        samples: Vec::new(),
        inputs: Vec::new(),
        passes: Vec::new(),
        lcg_draws: Vec::new(),
        get_pins: Vec::new(),
        var_names: Vec::new(),
        arg_names: Vec::new(),
        input_args: Vec::new(),
    };
    let mut var_ids: HashMap<&str, u32> = HashMap::new();
    let mut arg_ids: HashMap<&str, u32> = HashMap::new();
    let mut class = vec![Class::Const; n];
    for (name, _, _) in &p.inputs {
        let next = arg_ids.len() as u32;
        let a = *arg_ids.entry(name.as_str()).or_insert(next);
        if a == next {
            c.arg_names.push(name.clone());
        }
        c.input_args.push(a);
    }
    for (i, node) in p.nodes.iter().enumerate() {
        let num = |k: &str| node.num.get(k).copied().unwrap_or(0.0);
        let mut inst = Inst { op: node.op, impure: false, ins: [NONE; 4], x: NONE, y: NONE };
        for (d, j) in inst.ins.iter_mut().zip(node.ins) {
            if let Some(j) = j {
                *d = alias.get(j).copied().unwrap_or(NONE);
            }
        }
        let var_of = |name: &str, c: &mut Code| -> u32 {
            if let Some(v) = var_ids.get(name) {
                return *v;
            }
            let v = c.var_names.len() as u32;
            c.var_names.push(name.to_string());
            v
        };
        let mut own = match node.op {
            Op::Float
            | Op::Int
            | Op::Bool
            | Op::Str
            | Op::Out
            | Op::Pin
            | Op::Vec(..)
            | Op::Swizzle
            | Op::Un(_)
            | Op::Bin(_)
            | Op::Clamp
            | Op::Lerp
            | Op::Smooth
            | Op::If
            | Op::Seq => Class::Const,
            Op::ToFloat | Op::ToInt | Op::ToBool | Op::ToStr | Op::Concat => Class::Const,
            Op::Stat(_)
            | Op::Size
            | Op::OwnSize
            | Op::CellSize
            | Op::CellDepth
            | Op::Index
            | Op::Count
            | Op::Seed
            | Op::Input
            | Op::GraphInput
            | Op::SampleGray
            | Op::SampleColor => Class::Env,
            Op::Pos | Op::Call => Class::Run,
            Op::Get => Class::Vars,
            Op::Set | Op::Loop | Op::Random => Class::Impure,
        };
        match node.op {
            Op::Float => c.literals.push((i as u32, Value::float(num("v")))),
            Op::Int => c.literals.push((i as u32, Value::int(num("v").trunc() as i64))),
            Op::Bool => c.literals.push((i as u32, Value::Bool(num("v") != 0.0))),
            Op::Str => c.literals.push((i as u32, Value::text(&node.text))),
            Op::Vec(..) => {
                inst.x = c.vec_defaults.len() as u32;
                c.vec_defaults.push([num("x"), num("y"), num("z"), num("w")]);
            }
            Op::Swizzle => {
                inst.x = c.swizzles.len() as u32;
                c.swizzles.push(plain_mask(&node.text));
            }
            Op::Random => {
                inst.x = c.seeds.len() as u32;
                c.seeds.push(num("seed") as u32);
                c.lcg_draws.push(node.num.contains_key("lcg").then(|| (num("lcg") as i32, num("k") as u32)));
            }
            Op::SampleGray | Op::SampleColor => {
                inst.x = c.samples.len() as u32;
                let slot = if node.name.is_empty() { "in".to_string() } else { node.name.clone() };
                c.samples.push(SampleInfo { slot, filter: node.num.get("filter").map(|f| *f as u32) });
            }
            Op::Input | Op::GraphInput => {
                let pin = type_pin(num("type").round() as i32);
                let default = match pin {
                    Pin::Bool => Value::Bool(num("v") != 0.0),
                    Pin::Text => Value::text(&node.text),
                    q if q.lanes() == 1 => coerce(&Value::float(num("v")), q),
                    q => coerce(&Value::vec(q.lanes() as u8, [num("x"), num("y"), num("z"), num("w")]), q),
                };
                let mut arg = NONE;
                if node.op == Op::Input {
                    let next = arg_ids.len() as u32;
                    arg = *arg_ids.entry(node.name.as_str()).or_insert(next);
                    if arg == next {
                        c.arg_names.push(node.name.clone());
                    }
                }
                inst.x = c.inputs.len() as u32;
                c.inputs.push(InputInfo { pin, default, arg });
            }
            Op::Loop => {
                inst.x = c.passes.len() as u32;
                c.passes.push(loop_passes(&node.num));
            }
            Op::Set | Op::Get => {
                let v = var_of(&node.name, &mut c);
                var_ids.insert(node.name.as_str(), v);
                inst.x = v;
                if node.op == Op::Get {
                    inst.y = c.get_pins.len() as u32;
                    // a declared type shapes what is read; the value's own shape otherwise
                    c.get_pins.push(if node.num.contains_key("type") { Some(type_pin(num("type").round() as i32)) } else { None });
                }
            }
            _ => {}
        }
        // a call starts with variables and random numbers of its own, so nothing in it is visible here but the result, which
        // depends at most on the position
        for j in inst.ins.iter().copied().filter(|j| *j != NONE) {
            own = own.max(class.get(j as usize).copied().unwrap_or(Class::Const));
        }
        if let Some(d) = class.get_mut(i) {
            *d = own;
        }
        inst.impure = own == Class::Impure;
        if let Some(d) = c.floor.get_mut(i) {
            *d = match own {
                Class::Const => FLOOR_CONST,
                Class::Env => FLOOR_ENV,
                Class::Run => FLOOR_RUN,
                Class::Impure | Class::Vars => FLOOR_PASS,
            };
        }
        c.insts.push(inst);
    }

    c
}

fn plain_mask(mask: &str) -> Option<([u8; 4], u8)> {
    let (mut idx, mut len) = ([0u8; 4], 0usize);
    for ch in mask.chars() {
        if len == 4 {
            return None;
        }
        let slot = idx.get_mut(len)?;
        *slot = match ch {
            'x' | 'r' => 0,
            'y' | 'g' => 1,
            'z' | 'b' => 2,
            'w' | 'a' => 3,
            _ => return None,
        };
        len += 1;
    }
    (len > 0).then_some((idx, len as u8))
}
