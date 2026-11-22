//! Value expressions as engine function graphs (`meta kind "function"` subgraphs), one `f_*` node per
//! instruction. Identical expressions share one function.

use crate::decode::graph_package::Description;
use crate::decode::graph_package::compiled::{Expr, Instr, NO_OPERAND, Operands, ValueType, swizzle_source};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Collects the functions of one package.
#[derive(Debug, Default)]
pub struct Functions {
    /// function text (body) → name
    by_body: BTreeMap<String, String>,
    /// name → full subgraph text
    pub subgraphs: Vec<(String, String)>,
    pub warnings: Vec<String>,
    /// Expressions of the node being translated that run first and set variables its other expressions read (a
    /// node's size expression does this). Every function of the node starts with them.
    pub init: Vec<Expr>,
    /// What the graph's value nodes leave in its value memory, read with the `inputat` instruction.
    pub values: Vec<ValueNode>,
    /// How `rand` draws in the node being translated.
    pub rand: RandContext,
}

/// What a pixel processor stores instead of making an image: words `offset..offset + words` of the value memory, the
/// result of instruction `source` of `expr`.
#[derive(Debug, Clone)]
pub struct ValueNode {
    pub offset: u32,
    pub words: u32,
    pub expr: Expr,
    pub source: u16,
    /// The node's seed expression.
    pub seed: Option<Expr>,
    /// It reads images, which a function of another node can read only through an image slot of its own (see [`Host`]).
    pub reads_images: bool,
    /// The value node's image inputs (node indices), what its samplers read.
    pub inputs: Vec<usize>,
}

/// A pixel processor whose function is being built: a value node that reads images is computed in its function, which
/// samples the value node's images through slots added after the processor's own `own` inputs. `extra` collects the
/// image nodes those slots take, in slot order.
#[derive(Debug, Default)]
pub struct Host {
    pub own: usize,
    pub extra: Vec<usize>,
}

thread_local! {
    static HOST: std::cell::RefCell<Option<Host>> = const { std::cell::RefCell::new(None) };
    /// While a value node that reads images is built: the slot each of its images is read through.
    static REMAP: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Builds a pixel processor's function with `own` inputs; returns the function and the image nodes of the extra slots.
pub fn with_host<T>(own: usize, f: impl FnOnce() -> T) -> (T, Vec<usize>) {
    HOST.with(|h| *h.borrow_mut() = Some(Host { own, extra: Vec::new() }));
    let r = f();
    let extra = HOST.with(|h| h.borrow_mut().take()).map(|h| h.extra).unwrap_or_default();
    (r, extra)
}

/// The slots of a value node's images in the host being built, `None` outside a host.
fn host_slots(inputs: &[usize]) -> Option<Vec<String>> {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        let host = h.as_mut()?;
        Some(
            inputs
                .iter()
                .map(|i| {
                    let k = host.extra.iter().position(|e| e == i).unwrap_or_else(|| {
                        host.extra.push(*i);
                        host.extra.len() - 1
                    });
                    slot((host.own + k) as u16)
                })
                .collect(),
        )
    })
}

/// The slot a sampler reads: its own image's, or inside a value node being computed in a host, the host's slot for it.
fn sample_slot(img: u16) -> String {
    REMAP.with(|r| r.borrow().as_ref().and_then(|m| m.get(usize::from(img)).cloned())).unwrap_or_else(|| slot(img))
}

/// Which of the compiled-graph engine's random generators an expression draws from, written on a `rand` node as
/// `lcg=<site>` ([`site_name`](rand_mode::site_name)). The numbers are the engine's own (`lcg_rand`, `func/vm.rs` in the graph engine); the generators behind them
/// (multiplier, increment, start state) were measured: "Random numbers: solved, from rendered
/// images only".
pub mod rand_mode {
    /// No generator: functions outside any node.
    pub const NONE: u8 = 0;
    /// A pixel processor, per pixel.
    pub const PIXEL: u8 = 1;
    /// A value node, once (draw number `k`).
    pub const VALUE: u8 = 2;
    /// A node's parameter expression.
    pub const PARAMETER: u8 = 3;
    /// A pixel processor's size expression (draw number `k`).
    pub const SIZE: u8 = 5;
    /// Fx-map expressions, by what they compute (the engine's `FX_SITES` in this order).
    pub const FX_LUMINANCE: u8 = 10;
    pub const FX_BRANCH_OFFSET: u8 = 11;
    pub const FX_PATTERN_OFFSET: u8 = 12;
    pub const FX_SIZE: u8 = 13;
    pub const FX_ROTATION: u8 = 14;
    pub const FX_VARIATION: u8 = 15;
    pub const FX_SWITCH_CONDITION: u8 = 16;
    pub const FX_ITERATE_COUNT: u8 = 17;
    pub const FX_IMAGE_INDEX: u8 = 18;

    /// The graph language's name of a generator (`lcg="<site>"` on `f_random`); `None` for one without a name.
    pub fn site_name(mode: u8) -> Option<&'static str> {
        Some(match mode {
            PIXEL => "pixel",
            VALUE => "value",
            PARAMETER => "parameter",
            SIZE => "size",
            FX_LUMINANCE => "fx_luminance",
            FX_BRANCH_OFFSET => "fx_branch_offset",
            FX_PATTERN_OFFSET => "fx_pattern_offset",
            FX_SIZE => "fx_size",
            FX_ROTATION => "fx_rotation",
            FX_VARIATION => "fx_variation",
            FX_SWITCH_CONDITION => "fx_switch_condition",
            FX_ITERATE_COUNT => "fx_iterate_count",
            FX_IMAGE_INDEX => "fx_image_index",
            _ => return None,
        })
    }
}

/// How `rand` draws in the expressions being translated (see the graph engine's `lcg_rand`).
#[derive(Debug, Clone, Default)]
pub struct RandContext {
    /// One of [`rand_mode`].
    pub mode: u8,
    /// The node's seed expression.
    pub seed: Option<Expr>,
}

/// What `build` writes for a `rand`.
struct RandUse {
    mode: u8,
    seed: Option<u32>,
    /// A value node's draws in the order they run.
    draws: BTreeMap<usize, u32>,
}

impl RandUse {
    /// The node names of cos and sin here. A pixel processor has the GPU's; a value node, its near relative (not measured), and functions
    /// outside any node the accurate ones; fx-maps and node parameters the engine's polynomial.
    fn trig(&self) -> (&'static str, &'static str) {
        match self.mode {
            rand_mode::PIXEL => ("f_cos_f32turns", "f_sin_f32turns"),
            rand_mode::NONE | rand_mode::VALUE => ("f_cos", "f_sin"),
            _ => ("f_cos_poly", "f_sin_poly"),
        }
    }
}

/// For each instruction of `e`, whether running it changes something besides giving a value: a `set`, anywhere below it. Measured:
/// the first operand of a `sequence` without one doesn't run at all, so its random numbers aren't drawn there.
pub(super) fn has_effect(e: &Expr) -> Vec<bool> {
    let mut out = vec![false; e.instrs.len()];
    for (i, ins) in e.instrs.iter().enumerate() {
        let below = |r: &u16| out.get(usize::from(*r)).copied().unwrap_or(false);
        let v = match &ins.operands {
            Operands::Refs(_) if ins.op == super::scalar::OP_SEQUENCE && ins.ty & super::scalar::KEEP != 0 => true,
            Operands::Refs(r) => r.iter().any(below),
            Operands::RefSlot(a, _) => matches!(ins.name(), "set" | "store") || below(a),
            Operands::Loop(r, _) => r.iter().any(below),
            Operands::Sample(p, ..) => below(p),
            _ => false,
        };
        if let Some(o) = out.get_mut(i) {
            *o = v;
        }
    }
    out
}

/// The `rand` instructions `e` runs, in order, when it computes instruction `root`: operands left to right.
fn draw_order(e: &Expr, root: usize) -> BTreeMap<usize, u32> {
    fn walk(e: &Expr, effect: &[bool], i: usize, seen: &mut Vec<bool>, order: &mut BTreeMap<usize, u32>) {
        let (Some(ins), Some(s)) = (e.instrs.get(i), seen.get_mut(i)) else { return };
        if *s {
            return;
        }
        *s = true;
        match &ins.operands {
            Operands::Refs(r) if ins.name() == "sequence" => {
                for (k, x) in r.iter().enumerate() {
                    if *x != NO_OPERAND && (k > 0 || ins.ty & super::scalar::KEEP != 0 || effect.get(usize::from(*x)).copied().unwrap_or(false)) {
                        walk(e, effect, usize::from(*x), seen, order);
                    }
                }
            }
            Operands::Refs(r) => r.iter().filter(|x| **x != NO_OPERAND).for_each(|x| walk(e, effect, usize::from(*x), seen, order)),
            Operands::RefSlot(a, _) => walk(e, effect, usize::from(*a), seen, order),
            _ => {}
        }
        if ins.name() == "rand" {
            let n = order.len() as u32 + 1;
            order.insert(i, n);
        }
    }
    let mut order = BTreeMap::new();
    walk(e, &has_effect(e), root, &mut vec![false; e.instrs.len()], &mut order);
    order
}

/// How a graph input is read inside functions.
fn input_node(uid: u32, desc: &Description) -> String {
    match desc.input(uid) {
        Some(i) if i.identifier == "$outputsize" => "OUTPUTSIZE".into(),
        Some(i) if i.identifier == "$randomseed" => "f_seed".into(),
        Some(i) => format!("f_graph_input name=\"{}\" type={}", i.identifier, value_type(i.ty)),
        None => format!("f_float v=0 # unknown input {uid:#x}"),
    }
}

/// The `type` number of `f_graph_input` / `input_value` for an XML input type.
pub fn value_type(xml_ty: u32) -> u32 {
    match xml_ty {
        0 => 0,
        1 => 3,
        2 => 4,
        3 => 5,
        4 => 1,
        6 => TEXT_TYPE,
        8 => 6,
        9 => 7,
        10 => 8,
        _ => 0,
    }
}

/// The `type` number of a text input.
pub const TEXT_TYPE: u32 = 9;

const LANES: [&str; 4] = ["x", "y", "z", "w"];

/// What stops a loop the engine would run until its condition holds, whatever that takes.
const LOOP_NO_CAP: u32 = 1 << 24;

impl Functions {
    /// The name of a function returning graph input `uid` as it is (a text input's text), created on first use.
    pub fn input_function(&mut self, uid: u32, desc: &Description) -> String {
        let body = format!("node 2 {}\nnode 1 f_out in=@2\n", input_node(uid, desc));
        if let Some(name) = self.by_body.get(&body) {
            return name.clone();
        }
        let name = format!("fn{}", self.subgraphs.len() + 1);
        self.subgraphs.push((name.clone(), format!("meta kind \"function\"\n{body}")));
        self.by_body.insert(body, name.clone());
        name
    }

    /// The name of a function computing `e`, created on first use. `lane` picks one component of a vector
    /// result (for parameters stored per channel).
    pub fn function(&mut self, e: &Expr, desc: &Description, lane: Option<usize>) -> String {
        let lanewise = super::scalar::scalarize(e);
        let e = lanewise.as_ref().unwrap_or(e);
        let mut body = String::new();
        let mut warn = Vec::new();
        // node ids: instructions from 2 on, one expression after the other; helpers above them all
        let draws = |x: &Expr| x.instrs.iter().any(|i| i.name() == "rand");
        let seed = self
            .rand
            .seed
            .as_ref()
            .filter(|_| matches!(self.rand.mode, rand_mode::PIXEL | rand_mode::PARAMETER) && (draws(e) || self.init.iter().any(draws)));
        let total: usize = self.init.iter().map(|i| i.instrs.len()).sum::<usize>() + e.instrs.len() + seed.map_or(0, |s| s.instrs.len());
        let helper = Cell::new(1000u32.max(total as u32 + 2));
        let next = || {
            helper.set(helper.get() + 1);
            helper.get()
        };
        let mut memo = BTreeMap::new();
        let mut base = 2u32;
        let mut rand = RandUse { mode: self.rand.mode, seed: None, draws: BTreeMap::new() };
        if let Some(s) = seed {
            rand.seed = Some(build(s, desc, &self.values, &mut memo, &mut body, &mut warn, base, &helper, &rand));
            base += s.instrs.len() as u32;
        }
        let mut first = Vec::new();
        for i in &self.init {
            // a size expression runs once, drawing as a parameter does, even when it lands in a function that runs per pixel
            let once = RandUse { mode: rand_mode::SIZE, seed: rand.seed, draws: draw_order(i, i.instrs.len().wrapping_sub(1)) };
            first.push(build(i, desc, &self.values, &mut memo, &mut body, &mut warn, base, &helper, &once));
            base += i.instrs.len() as u32;
        }
        let mut out = build(e, desc, &self.values, &mut memo, &mut body, &mut warn, base, &helper, &rand);
        if let Some(k) = lane {
            let h = next();
            let _ = writeln!(body, "node {h} f_swizzle a=@{out} mask=\"{}\"", LANES.get(k).copied().unwrap_or("x"));
            out = h;
        }
        for f in first.into_iter().rev() {
            let h = next();
            let _ = writeln!(body, "node {h} f_sequence a=@{f} b=@{out}");
            out = h;
        }
        let _ = writeln!(body, "node 1 f_out in=@{out}");
        if let Some(name) = self.by_body.get(&body) {
            return name.clone();
        }
        let name = format!("fn{}", self.subgraphs.len() + 1);
        self.warnings.extend(warn.into_iter().map(|w| format!("{name}: {w}")));
        self.subgraphs.push((name.clone(), format!("meta kind \"function\"\nmeta float32 \"1\"\n{body}")));
        self.by_body.insert(body, name.clone());
        name
    }
}

/// The `type` number of a value with `components` lanes of the instruction type `ty`, as `f_get` declares it.
fn pin_type(ty: ValueType, components: u8) -> u32 {
    match (ty, components) {
        (ValueType::Bool, _) => 2,
        (ValueType::Int, 1) => 1,
        (ValueType::Int, n) => 4 + u32::from(n),
        (_, 1) => 0,
        (_, n) => 1 + u32::from(n),
    }
}

/// Writes the nodes for `e` into `out` (instruction i is node base + i, helpers above every instruction) and returns
/// the id of the result node.
#[allow(clippy::too_many_arguments)]
fn build(
    e: &Expr,
    desc: &Description,
    values: &[ValueNode],
    memo: &mut BTreeMap<usize, u32>,
    out: &mut String,
    warn: &mut Vec<String>,
    base: u32,
    helper: &Cell<u32>,
    rand: &RandUse,
) -> u32 {
    let left_out = e.instrs.iter().any(|i| matches!(&i.operands, Operands::Refs(r) if r.contains(&NO_OPERAND)));
    if left_out {
        let _ = writeln!(out, "node {} f_float v=0", u32::from(NO_OPERAND) + base);
    }
    let effect = has_effect(e);
    let mut b = Builder {
        e,
        desc,
        values,
        memo,
        out,
        warn,
        base,
        helper,
        rand,
        lanes: e.instrs.iter().map(|i| i.components).collect(),
        arms: super::arms::Arms::new(e, &effect),
        effect,
    };
    for (i, ins) in e.instrs.iter().enumerate() {
        let line = b.instruction(i, ins);
        let _ = writeln!(b.out, "node {} {line}", i as u32 + base);
    }
    e.instrs.len().saturating_sub(1) as u32 + base
}

/// One expression being written by [`build`]: what it reads, where it writes, and what is known of its instructions.
struct Builder<'a, 'o> {
    e: &'a Expr,
    desc: &'a Description,
    values: &'a [ValueNode],
    /// Value nodes already computed in this function: value node index → node id of the value it stores.
    memo: &'o mut BTreeMap<usize, u32>,
    out: &'o mut String,
    warn: &'o mut Vec<String>,
    /// Node id of instruction 0.
    base: u32,
    /// The last helper node id given out.
    helper: &'a Cell<u32>,
    rand: &'a RandUse,
    /// How many lanes each instruction's value has.
    lanes: Vec<u8>,
    /// See [`has_effect`].
    effect: Vec<bool>,
    arms: super::arms::Arms<'a>,
}

impl Builder<'_, '_> {
    /// Node id of instruction `i`.
    fn id(&self, i: u16) -> u32 {
        u32::from(i) + self.base
    }

    /// A new helper node id, above every instruction.
    fn next_helper(&self) -> u32 {
        self.helper.set(self.helper.get() + 1);
        self.helper.get()
    }

    /// Node id of operand `k` (0 when there is none).
    fn operand(&self, refs: &[u16], k: usize) -> u32 {
        refs.get(k).copied().map_or(0, |i| self.id(i))
    }

    /// Runs `first` in order, then gives `value`.
    fn after(&mut self, first: &[u16], value: u32) -> u32 {
        let mut v = value;
        for x in first.iter().rev() {
            let h = self.next_helper();
            let _ = writeln!(self.out, "node {h} f_sequence a=@{} b=@{v}", self.id(*x));
            v = h;
        }
        v
    }

    /// The node line of instruction `i`, by its operation; the helper nodes it needs are written before it.
    fn instruction(&mut self, i: usize, ins: &Instr) -> String {
        let refs = match &ins.operands {
            Operands::Refs(r) => r.clone(),
            _ => Vec::new(),
        };
        let r = |k: usize| self.operand(&refs, k);
        match (ins.name(), &ins.operands) {
            ("const", Operands::Constant(w)) => constant(ins, w),
            ("input", Operands::Input(uid)) => self.graph_input(ins, *uid),
            ("inputat", Operands::Slot(k)) => self.value_memory(ins, u32::from(*k)),
            ("builtin", Operands::Slot(k)) => self.builtin(*k),
            ("loop", Operands::Loop(l, passes)) => self.loop_node(l, *passes),
            // a variable read as a boolean: variables hold numbers
            ("get", Operands::Slot(k)) if ins.value_type() == ValueType::Bool => {
                let h = self.next_helper();
                let _ = writeln!(self.out, "node {h} f_get name=\"v{k}\" type=2");
                format!("f_to_bool a=@{h}")
            }
            ("get", Operands::Slot(k)) => format!("f_get name=\"v{k}\" type={}", pin_type(ins.value_type(), ins.components)),
            // storing passes the value on; whoever reads the memory computes it itself
            ("store", Operands::RefSlot(a, _)) => format!("f_sequence a=@{0} b=@{0}", self.id(*a)),
            ("set", Operands::RefSlot(a, k)) => format!("f_set a=@{} name=\"v{k}\"", self.id(*a)),
            ("swizzle", Operands::RefSlot(a, m)) => self.swizzle(ins, *a, *m),
            ("ifelse", _) => self.if_else(i, ins, &refs),
            // a left-out operand reads as 0, but these operators only take booleans
            ("and" | "or", Operands::Refs(refs)) if refs.contains(&NO_OPERAND) => self.bool_operands(ins, refs),
            // in a pixel processor every product and sum of a dot rounds to single precision on its own, which a hash shows
            // (measured by evaluating real nodes step by step: noise_anisotropic_noise #0, feather_stylized #0)
            ("dot", Operands::Refs(x))
                if let &[x0, x1] = x.as_slice()
                    && self.rand.mode == rand_mode::PIXEL =>
            {
                self.dot_rounded(&refs, x0, x1)
            }
            // of unequal length: the missing lanes read as 0, so the longer one's extra lanes count for nothing (probable: follows
            // from the rule for lanes, not measured for a dot itself)
            ("dot", Operands::Refs(x))
                if let &[x0, x1] = x.as_slice()
                    && self.lanes.get(usize::from(x0)) != self.lanes.get(usize::from(x1)) =>
            {
                self.dot_unequal(&refs, x0, x1)
            }
            ("sequence", _)
                if ins.ty & super::scalar::KEEP != 0 || refs.first().is_some_and(|a| self.effect.get(usize::from(*a)).copied().unwrap_or(false)) =>
            {
                format!("f_sequence a=@{} b=@{}", r(0), r(1))
            }
            // the first operand doesn't run, though what it shares with others does
            ("sequence", _) => {
                let (first, second) = (self.arms.leaving(i, 0), r(1));
                format!("f_pin a=@{}", self.after(&first, second))
            }
            ("vector" | "vector3?" | "vector4", _) => self.vector(ins, &refs),
            ("cast", _) => self.cast(ins, &refs),
            ("log2", _) => self.log2(&refs),
            ("pow2", _) => self.pow2(&refs),
            ("atan2", _) => self.atan2(&refs),
            ("cartesian", _) => self.cartesian(&refs),
            ("rand", _) => self.random(i, &refs),
            ("samplelum" | "samplecol", Operands::Sample(p, img, f)) => self.sample(ins, *p, *img, *f),
            (name, _) => self.simple_op(ins, name, &refs),
        }
    }

    /// A graph input: `$outputsize` as the log2 of the size in pixels, a toggle read as a boolean, the rest as
    /// [`input_node`] reads it.
    fn graph_input(&mut self, ins: &Instr, uid: u32) -> String {
        match input_node(uid, self.desc).as_str() {
            "OUTPUTSIZE" => {
                // $outputsize is the log2 of the size in pixels
                let (a, b) = (self.next_helper(), self.next_helper());
                let _ = writeln!(self.out, "node {a} f_size");
                let _ = writeln!(self.out, "node {b} f_log2 a=@{a}");
                format!("f_to_int a=@{b}")
            }
            // a toggle read as a boolean: the input is an integer
            s if ins.value_type() == ValueType::Bool && s.starts_with("f_graph_input") => {
                let h = self.next_helper();
                let _ = writeln!(self.out, "node {h} {s}");
                format!("f_to_bool a=@{h}")
            }
            s => s.to_string(),
        }
    }

    /// Word `k` of the value memory (`inputat`): the value node that stores it is computed in place.
    fn value_memory(&mut self, ins: &Instr, k: u32) -> String {
        let values = self.values;
        let found = values.iter().enumerate().find(|(_, v)| v.offset <= k && k < v.offset + v.words);
        let slots = found.filter(|(_, value)| value.reads_images).and_then(|(_, value)| host_slots(&value.inputs));
        match found {
            Some((v, value)) if !value.reads_images || slots.is_some() => {
                let src = match self.memo.get(&v) {
                    Some(id) => *id,
                    None => self.value_node(v, value, slots),
                };
                if ins.value_type() == ValueType::Bool {
                    format!("f_to_bool a=@{src}")
                } else {
                    let first = (k - value.offset) as usize;
                    let mask: String = (first..first + usize::from(ins.components)).map(|l| LANES.get(l).copied().unwrap_or("x")).collect();
                    format!("f_swizzle a=@{src} mask=\"{mask}\"")
                }
            }
            Some(_) => {
                self.warn.push(format!("value {k} is computed from an image: read as 0"));
                zero(ins)
            }
            None => {
                self.warn.push(format!("value {k} has no value node: read as 0"));
                zero(ins)
            }
        }
    }

    /// Computes value node `v` (index `v` of the values) inside this function, once, and gives the node id of what it
    /// stores. `slots`: the host's image slots its samplers read through, when it reads images.
    fn value_node(&mut self, v: usize, value: &ValueNode, slots: Option<Vec<String>>) -> u32 {
        let helper = self.helper;
        // a value node that reads images samples them through the host's extra slots
        let outer = REMAP.with(|r| r.replace(slots));
        let (n, b) = (value.expr.instrs.len() as u32, helper.get() + 1);
        helper.set(helper.get() + n + 1);
        // the value node's draws: its own seed, each from the start of its generator
        let mut own = RandUse { mode: rand_mode::VALUE, seed: None, draws: draw_order(&value.expr, value.expr.instrs.len().wrapping_sub(1)) };
        if let Some(s) = &value.seed {
            let sb = helper.get() + 1;
            helper.set(helper.get() + s.instrs.len() as u32 + 1);
            own.seed = Some(build(s, self.desc, self.values, self.memo, self.out, self.warn, sb, helper, &own));
        }
        build(&value.expr, self.desc, self.values, self.memo, self.out, self.warn, b, helper, &own);
        REMAP.with(|r| r.replace(outer));
        let r = b + u32::from(value.source);
        self.memo.insert(v, r);
        r
    }

    /// Built-in variable `k`: sizes, the position, the index, and what a fx-map's cell holds.
    fn builtin(&mut self, k: u16) -> String {
        match k {
            // the node's own size in its parameters (measured), not the graph's as $outputsize is
            // (a size expression runs before the node has a size: it reads 1, measured)
            1 if self.rand.mode == rand_mode::SIZE => "f_vec2 x=1 y=1".into(),
            // a value node computed inside another node's function: the value node's size, which is the graph's
            // (topstitches #7 stores $sizelog2 for #9, a 2048 x 16 node: with #9's own size its lengths came out 128 times too long)
            1 if self.rand.mode == rand_mode::VALUE => "f_size".into(),
            3 if self.rand.mode == rand_mode::VALUE => {
                let a = self.next_helper();
                let _ = writeln!(self.out, "node {a} f_size");
                format!("f_log2 a=@{a}")
            }
            1 => "f_own_size".into(),
            8 => "f_pos".into(),
            10 => "f_index".into(),
            // of the cell a fx-map is walking (measured)
            11 => "f_cell_size".into(),
            12 => "f_cell_depth".into(),
            // $sizelog2 goes with $size: the node's own (quantize's prefix sums on a 1024 x 2048 node run their last steps only
            // where it reads 11, in a 512 graph: the reference renderer's palette needs them)
            3 => {
                let a = self.next_helper();
                let _ = writeln!(self.out, "node {a} f_own_size");
                format!("f_log2 a=@{a}")
            }
            _ => {
                self.warn.push(format!("built-in variable #{k} read as 0"));
                "f_float v=0".into()
            }
        }
    }

    /// A loop: start, stop condition and body, with its count.
    fn loop_node(&self, l: &[u16], passes: u32) -> String {
        let at = |k: usize| l.get(k).copied().map_or(0, |i| self.id(i));
        // measured: with a condition the count is a cap, 0 meaning none; without one it is the count, at least 1
        match l.get(1) {
            Some(&NO_OPERAND) => format!("f_loop a=@{} c=@{} passes={} long=1", at(0), at(2), passes.max(1)),
            _ => format!("f_loop a=@{} b=@{} c=@{} passes={} long=1", at(0), at(1), at(2), if passes == 0 { LOOP_NO_CAP } else { passes }),
        }
    }

    /// A swizzle of instruction `a` by mask `m`.
    fn swizzle(&mut self, ins: &Instr, a: u16, m: u16) -> String {
        let e = self.e;
        // booleans are one value here, whatever their lanes field says
        if e.instrs.get(usize::from(a)).is_some_and(|s| s.value_type() == ValueType::Bool) {
            return format!("f_to_bool a=@{}", self.id(a));
        }
        // a scalar's only lane, also when the mask names another
        if ins.components == 1 && self.lanes.get(usize::from(a)).copied() == Some(1) {
            return format!("f_swizzle a=@{} mask=\"x\"", self.id(a));
        }
        // lanes a value doesn't have read as 0 (measured on a scalar)
        if (0..u16::from(ins.components)).any(|l| swizzle_source(m, l) >= self.lanes.get(usize::from(a)).copied().unwrap_or(4)) {
            let n = self.lanes.get(usize::from(a)).copied().unwrap_or(4);
            let mut parts = Vec::new();
            for l in 0..u16::from(ins.components) {
                let from = swizzle_source(m, l);
                let h = self.next_helper();
                if from < n {
                    let _ =
                        writeln!(self.out, "node {h} f_swizzle a=@{} mask=\"{}\"", self.id(a), LANES.get(usize::from(from)).copied().unwrap_or("x"));
                } else {
                    let _ = writeln!(self.out, "node {h} f_float v=0");
                }
                parts.push(h);
            }
            let args: Vec<String> = parts.iter().zip(LANES).map(|(p, l)| format!("{l}=@{p}")).collect();
            return format!("f_vec{} {}", parts.len(), args.join(" "));
        }
        let mask: String =
            (0..usize::from(ins.components)).map(|l| LANES.get(usize::from(swizzle_source(m, l as u16))).copied().unwrap_or("x")).collect();
        format!("f_swizzle a=@{} mask=\"{mask}\"", self.id(a))
    }

    /// An `ifelse`, with what must run though its arm is not taken.
    fn if_else(&mut self, i: usize, ins: &Instr, refs: &[u16]) -> String {
        let e = self.e;
        // toggles are stored as integers: a condition that is not boolean is converted
        let c = refs.first().copied().unwrap_or(0);
        let cond = if e.instrs.get(usize::from(c)).is_some_and(|ci| ci.value_type() != ValueType::Bool) {
            let h = self.next_helper();
            let _ = writeln!(self.out, "node {h} f_to_bool a=@{}", self.id(c));
            h
        } else {
            self.id(c)
        };
        // what leaves the arm not taken runs all the same, in operand order (see `arms`)
        let (first, second) = (self.arms.leaving(i, 0), self.arms.leaving(i, 1));
        let (arm1, arm2) = (self.arm(ins, refs, 1), self.arm(ins, refs, 2));
        let mut a = arm1;
        if !second.is_empty() {
            let then = self.after(&second, a);
            let h = self.next_helper();
            let _ = writeln!(self.out, "node {h} f_sequence a=@{a} b=@{then}");
            a = h;
        }
        let b = self.after(&first, arm2);
        format!("f_if cond=@{cond} a=@{a} b=@{b}")
    }

    /// Arm `k` of an `ifelse`: a left-out arm is a zero of the result's shape, not the scalar one (topstitches' size: vec2 arms).
    fn arm(&mut self, ins: &Instr, refs: &[u16], k: usize) -> u32 {
        match refs.get(k) {
            Some(&NO_OPERAND) if ins.components > 1 => {
                let h = self.next_helper();
                let _ = writeln!(self.out, "node {h} {}", zero(ins));
                h
            }
            _ => self.operand(refs, k),
        }
    }

    /// An `and` or `or` with a left-out operand, which reads as a false boolean.
    fn bool_operands(&mut self, ins: &Instr, refs: &[u16]) -> String {
        let mut operand = |k: usize| {
            if refs.get(k) == Some(&NO_OPERAND) {
                let h = self.next_helper();
                let _ = writeln!(self.out, "node {h} f_to_bool a=@{}", self.id(NO_OPERAND));
                h
            } else {
                self.operand(refs, k)
            }
        };
        let (x, y) = (operand(0), operand(1));
        format!("f_{} a=@{x} b=@{y}", ins.name())
    }

    /// A dot product in a pixel processor: each product and each sum rounded on its own.
    fn dot_rounded(&mut self, refs: &[u16], x0: u16, x1: u16) -> String {
        let n = self.lanes.get(usize::from(x0)).copied().unwrap_or(1).min(self.lanes.get(usize::from(x1)).copied().unwrap_or(1));
        let mut sum = None;
        for l in LANES.iter().take(usize::from(n)) {
            let (a, b, m) = (self.next_helper(), self.next_helper(), self.next_helper());
            let _ = writeln!(self.out, "node {a} f_swizzle a=@{} mask=\"{l}\"", self.operand(refs, 0));
            let _ = writeln!(self.out, "node {b} f_swizzle a=@{} mask=\"{l}\"", self.operand(refs, 1));
            let _ = writeln!(self.out, "node {m} f_mul a=@{a} b=@{b}");
            sum = Some(match sum {
                None => m,
                Some(s) => {
                    let h = self.next_helper();
                    let _ = writeln!(self.out, "node {h} f_add a=@{s} b=@{m}");
                    h
                }
            });
        }
        format!("f_pin a=@{}", sum.unwrap_or(0))
    }

    /// A dot product of operands with different lane counts, over the lanes both have.
    fn dot_unequal(&mut self, refs: &[u16], x0: u16, x1: u16) -> String {
        let (la, lb) = (self.lanes.get(usize::from(x0)).copied().unwrap_or(1), self.lanes.get(usize::from(x1)).copied().unwrap_or(1));
        let n = usize::from(la.min(lb));
        let mask: String = LANES.iter().take(n).copied().collect();
        let (a, b) = (self.next_helper(), self.next_helper());
        let _ = writeln!(self.out, "node {a} f_swizzle a=@{} mask=\"{mask}\"", self.operand(refs, 0));
        let _ = writeln!(self.out, "node {b} f_swizzle a=@{} mask=\"{mask}\"", self.operand(refs, 1));
        format!("f_dot a=@{a} b=@{b}")
    }

    /// A vector built from the lanes of its operands, one after the other.
    fn vector(&mut self, ins: &Instr, refs: &[u16]) -> String {
        let e = self.e;
        // concatenate the lanes of the operands
        let mut parts = Vec::new();
        for &src in refs {
            let bool_value = e.instrs.get(usize::from(src)).is_some_and(|s| s.value_type() == ValueType::Bool);
            let n = self.lanes.get(usize::from(src)).copied().unwrap_or(1);
            if n == 1 || bool_value {
                parts.push(self.id(src));
            } else {
                for l in 0..n {
                    let h = self.next_helper();
                    let _ =
                        writeln!(self.out, "node {h} f_swizzle a=@{} mask=\"{}\"", self.id(src), LANES.get(usize::from(l)).copied().unwrap_or("x"));
                    parts.push(h);
                }
            }
        }
        let n = parts.len().clamp(2, 4);
        let int = ins.value_type() == ValueType::Int;
        let kind = format!("f_{}vec{n}", if int { "i" } else { "" });
        let args: Vec<String> = parts.iter().take(4).zip(LANES).map(|(p, l)| format!("{l}=@{p}")).collect();
        format!("{kind} {}", args.join(" "))
    }

    /// A cast to an integer (a float becomes the integer below it) or to a float.
    fn cast(&mut self, ins: &Instr, refs: &[u16]) -> String {
        let e = self.e;
        if ins.value_type() == ValueType::Int {
            // a float becomes the integer below it
            let from_float = refs.first().and_then(|s| e.instrs.get(usize::from(*s))).is_some_and(|s| s.value_type() == ValueType::Float);
            if from_float {
                let h = self.next_helper();
                let _ = writeln!(self.out, "node {h} f_floor a=@{}", self.operand(refs, 0));
                format!("f_to_int a=@{h}")
            } else {
                format!("f_to_int a=@{}", self.operand(refs, 0))
            }
        } else {
            format!("f_to_float a=@{}", self.operand(refs, 0))
        }
    }

    /// A base-2 logarithm, as the natural one times log2(e).
    fn log2(&mut self, refs: &[u16]) -> String {
        let (l, k) = (self.next_helper(), self.next_helper());
        let _ = writeln!(self.out, "node {l} f_log_floor a=@{}", self.operand(refs, 0));
        let _ = writeln!(self.out, "node {k} f_float v={}", std::f64::consts::LOG2_E);
        format!("f_mul a=@{l} b=@{k}")
    }

    /// Two to a power.
    fn pow2(&mut self, refs: &[u16]) -> String {
        let two = self.next_helper();
        let _ = writeln!(self.out, "node {two} f_float v=2");
        format!("f_pow a=@{two} b=@{}", self.operand(refs, 0))
    }

    /// The angle of a 2D vector.
    fn atan2(&mut self, refs: &[u16]) -> String {
        let (x, y) = (self.next_helper(), self.next_helper());
        let _ = writeln!(self.out, "node {x} f_swizzle a=@{} mask=\"x\"", self.operand(refs, 0));
        let _ = writeln!(self.out, "node {y} f_swizzle a=@{} mask=\"y\"", self.operand(refs, 0));
        format!("f_atan2_approx a=@{y} b=@{x}")
    }

    /// Polar to cartesian: a length and an angle as a 2D vector, with this expression's cos and sin.
    fn cartesian(&mut self, refs: &[u16]) -> String {
        let (c, s, x, y) = (self.next_helper(), self.next_helper(), self.next_helper(), self.next_helper());
        let (fc, fs) = self.rand.trig();
        let _ = writeln!(self.out, "node {c} {fc} a=@{}", self.operand(refs, 1));
        let _ = writeln!(self.out, "node {s} {fs} a=@{}", self.operand(refs, 1));
        let _ = writeln!(self.out, "node {x} f_mul a=@{} b=@{c}", self.operand(refs, 0));
        let _ = writeln!(self.out, "node {y} f_mul a=@{} b=@{s}", self.operand(refs, 0));
        format!("f_vec2 x=@{x} y=@{y}")
    }

    /// A random number (instruction `i`), from the generator, seed and draw number this expression uses.
    fn random(&self, i: usize, refs: &[u16]) -> String {
        let rand = self.rand;
        let mut l = format!("f_random b=@{}", self.operand(refs, 0));
        if rand.mode != rand_mode::NONE {
            match rand_mode::site_name(rand.mode) {
                Some(site) => {
                    let _ = write!(l, " lcg=\"{site}\"");
                }
                None => {
                    let _ = write!(l, " lcg={}", rand.mode);
                }
            }
        }
        if let Some(s) = rand.seed {
            let _ = write!(l, " s=@{s}");
        }
        if matches!(rand.mode, rand_mode::VALUE | rand_mode::SIZE) {
            let _ = write!(l, " k={}", rand.draws.get(&i).copied().unwrap_or(1));
        }
        l
    }

    /// A grey or colour sample of image input `img` at position `p` with filter `f`; without a filter word the sample
    /// is nearest and repeats.
    fn sample(&mut self, ins: &Instr, p: u16, img: u16, f: u16) -> String {
        // a left-out position samples where the pixel is (wood_slice #1005, marble_veined, fabric_wool_herringbone: an RGB
        // to HSL over the input; measured against the reference renderer on wood_slice)
        let at = if p == NO_OPERAND {
            let h = self.next_helper();
            let _ = writeln!(self.out, "node {h} f_pos");
            h
        } else {
            self.id(p)
        };
        let kind = if ins.name() == "samplelum" { "f_sample_gray" } else { "f_sample_color" };
        format!("{kind} a=@{at} slot=\"{}\" filter={f}", sample_slot(img))
    }

    /// An operation that is one function node ([`simple`]); one not known reads as 0.
    fn simple_op(&mut self, ins: &Instr, name: &str, refs: &[u16]) -> String {
        let r = |k: usize| self.operand(refs, k);
        match simple(name) {
            Some(("f_cos", _)) => format!("{} a=@{}", self.rand.trig().0, r(0)),
            Some(("f_sin", _)) => format!("{} a=@{}", self.rand.trig().1, r(0)),
            Some((kind, 1)) => format!("{kind} a=@{}", r(0)),
            Some((kind, 2)) => format!("{kind} a=@{} b=@{}", r(0), r(1)),
            Some((kind, _)) => format!("{kind} a=@{} b=@{} c=@{}", r(0), r(1), r(2)),
            None => {
                self.warn.push(format!("operation {name} ({:#04x}) read as 0", ins.op));
                "f_float v=0".into()
            }
        }
    }
}

/// A zero of an instruction's shape.
fn zero(ins: &Instr) -> String {
    match (ins.value_type(), ins.components) {
        (ValueType::Bool, _) => "f_bool v=0".into(),
        (ValueType::Int, 1) => "f_int v=0".into(),
        (ValueType::Int, n) => format!("f_ivec{n} {}", LANES.iter().take(usize::from(n)).map(|l| format!("{l}=0")).collect::<Vec<_>>().join(" ")),
        (_, 1) => "f_float v=0".into(),
        (_, n) => format!("f_vec{n} {}", LANES.iter().take(usize::from(n)).map(|l| format!("{l}=0")).collect::<Vec<_>>().join(" ")),
    }
}

/// Pixel-processor image slot names: image 0 is `in`, 1 `in2`, …
pub fn slot(img: u16) -> String {
    if img == 0 { "in".into() } else { format!("in{}", img + 1) }
}

fn constant(ins: &Instr, w: &[u32]) -> String {
    match (ins.value_type(), w) {
        (ValueType::Bool, [bits, ..]) => format!("f_bool v={}", bits & 1),
        (ValueType::Int, [v]) => format!("f_int v={}", *v as i32),
        (ValueType::Int, v) => {
            let n = v.len().clamp(2, 4);
            let parts: Vec<String> = v.iter().zip(LANES).map(|(x, l)| format!("{l}={}", *x as i32)).collect();
            format!("f_ivec{n} {}", parts.join(" "))
        }
        (_, [v]) => format!("f_float v={}", f32::from_bits(*v)),
        (_, v) => {
            let n = v.len().clamp(2, 4);
            let parts: Vec<String> = v.iter().zip(LANES).map(|(x, l)| format!("{l}={}", f32::from_bits(*x))).collect();
            format!("f_vec{n} {}", parts.join(" "))
        }
    }
}

/// Operations that map onto one function node: (kind, operand count).
fn simple(name: &str) -> Option<(&'static str, usize)> {
    Some(match name {
        "add" => ("f_add", 2),
        "sub" => ("f_sub", 2),
        "mul" => ("f_mul", 2),
        "div" => ("f_div_large", 2),
        "mod" => ("f_mod", 2),
        "neg" => ("f_neg", 1),
        "dot" => ("f_dot", 2),
        "cross" => ("f_cross", 2),
        "and" => ("f_and", 2),
        "or" => ("f_or", 2),
        "not" => ("f_not", 1),
        "eq" => ("f_eq", 2),
        "noteq" => ("f_ne", 2),
        "gt" => ("f_gt", 2),
        "gteq" => ("f_ge", 2),
        "lt" => ("f_lt", 2),
        "lteq" => ("f_le", 2),
        "abs" => ("f_abs", 1),
        "floor?" => ("f_floor", 1),
        "ceil?" => ("f_ceil", 1),
        "cos" => ("f_cos", 1),
        "sin" => ("f_sin", 1),
        "sqrt?" => ("f_sqrt", 1),
        "log?" => ("f_log_floor", 1),
        "exp?" => ("f_exp", 1),
        "pow" => ("f_pow", 2),
        "tan" => ("f_tan", 1),
        "lerp" => ("f_lerp", 3),
        "min" => ("f_min", 2),
        "max" => ("f_max", 2),
        _ => return None,
    })
}
