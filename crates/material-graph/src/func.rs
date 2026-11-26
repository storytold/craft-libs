// SPDX-License-Identifier: MIT OR Apache-2.0
//! Function graphs: small value-computing graphs that drive parameters, pixel processors and pattern programs.
//!
//! A function graph is an ordinary [`Graph`] stored next to the texture graphs of a package (it carries the meta
//! entry `kind "function"`). Its nodes are the `f_*` kinds below. Links carry values instead of textures: floats and
//! float vectors, integers and integer vectors, booleans and strings. One `f_out` node returns the result.
//!
//! Rules that keep results predictable:
//! * integers only widen to floats; a float needs `f_to_int` to become an integer;
//! * a scalar combines with a vector of any size, two vectors need the same size;
//! * no operation produces NaN or infinity: division by zero, `sqrt` of a negative number, `log` of zero and the like
//!   give 0, inverse trigonometric inputs are clamped;
//! * nodes are evaluated on demand and once per run; each pass of a loop starts anew (an `f_set` does not: what was
//!   computed from a variable before it keeps its value, as measured against the engine).

mod code;
#[cfg(test)]
mod differential;
#[cfg(test)]
mod reference;
mod vm;
mod wgsl;

pub use vm::Frame as FnFrame;
pub use wgsl::PixelShader;

use crate::graph::{Graph, Pin, Spec};
use crate::math;
use crate::noise::rnd;
use crate::tex::{ImgRef, get, lum};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Stands in for an infinity (values stay finite, so later operations can't make NaN); any result limited to 0..1 ends up
/// at the limit.
const HUGE: f64 = 1e30;

/// A pixel of `t` at (u, v): `filter` bit 0 bilinear, else nearest; bit 1 keep the edge, else repeat.
fn sample_filtered(t: ImgRef, u: f32, v: f32, filter: u32) -> [f32; 4] {
    let (u, v) = if filter & 2 != 0 { (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)) } else { (u, v) };
    if filter & 1 == 0 {
        let at = |a: f32, n: usize| {
            let i = (a * n as f32).floor() as i64;
            if filter & 2 != 0 { i.clamp(0, n as i64 - 1) } else { i }
        };
        return t.at(at(u, t.w()), at(v, t.h()));
    }
    if filter & 2 != 0 {
        let (hx, hy) = (0.5 / t.w() as f32, 0.5 / t.h() as f32);
        return t.sample(u.clamp(hx, 1.0 - hx), v.clamp(hy, 1.0 - hy));
    }
    t.sample(u, v)
}

// ------------------------------------------------------------------------------------------------ values

/// A numeric value: one to four lanes, integer or float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    pub lanes: u8,
    pub int: bool,
    pub v: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(Num),
    Bool(bool),
    Str(Arc<str>),
}

impl Value {
    pub fn float(x: f64) -> Value {
        Value::Num(Num { lanes: 1, int: false, v: [x, 0.0, 0.0, 0.0] })
    }
    pub fn int(x: i64) -> Value {
        Value::Num(Num { lanes: 1, int: true, v: [x as f64, 0.0, 0.0, 0.0] })
    }
    pub fn vec(lanes: u8, v: [f64; 4]) -> Value {
        Value::Num(Num { lanes: lanes.clamp(1, 4), int: false, v })
    }
    pub fn text(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    /// The pin type of this value.
    pub fn pin(&self) -> Pin {
        match self {
            Value::Bool(_) => Pin::Bool,
            Value::Str(_) => Pin::Text,
            Value::Num(n) => match (n.lanes, n.int) {
                (1, false) => Pin::Float,
                (2, false) => Pin::Float2,
                (3, false) => Pin::Float3,
                (_, false) => Pin::Float4,
                (1, true) => Pin::Int,
                (2, true) => Pin::Int2,
                (3, true) => Pin::Int3,
                (_, true) => Pin::Int4,
            },
        }
    }

    /// First lane as f32 (booleans are 0 / 1, strings 0): how a value drives a numeric parameter.
    pub fn as_f32(&self) -> f32 {
        match self {
            Value::Num(n) => n.v[0] as f32,
            Value::Bool(b) => *b as i32 as f32,
            Value::Str(_) => 0.0,
        }
    }

    fn num(&self) -> Result<Num, String> {
        match self {
            Value::Num(n) => Ok(*n),
            Value::Bool(b) => Ok(Num { lanes: 1, int: true, v: [*b as i32 as f64, 0.0, 0.0, 0.0] }),
            Value::Str(_) => Err("a string cannot be used as a number".into()),
        }
    }

    /// Text shown in the editor and produced by `f_to_string`.
    pub fn show(&self) -> String {
        fn f(x: f64, int: bool) -> String {
            if int {
                format!("{}", x as i64)
            } else if x == x.trunc() && x.abs() < 1e9 {
                format!("{x:.1}")
            } else {
                format!("{x:.4}").trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Num(n) if n.lanes == 1 => f(n.v[0], n.int),
            Value::Num(n) => format!("({})", n.v.iter().take(n.lanes as usize).map(|x| f(*x, n.int)).collect::<Vec<_>>().join(", ")),
        }
    }
}

/// A value of the given type with every lane 1: used to find the result type of a node without running it.
fn dummy(p: Pin) -> Value {
    match p {
        Pin::Bool => Value::Bool(true),
        Pin::Text => Value::text("x"),
        Pin::Float | Pin::Float2 | Pin::Float3 | Pin::Float4 => Value::Num(Num { lanes: p.lanes() as u8, int: false, v: [1.0; 4] }),
        Pin::Int | Pin::Int2 | Pin::Int3 | Pin::Int4 => Value::Num(Num { lanes: p.lanes() as u8, int: true, v: [1.0; 4] }),
        _ => Value::float(1.0),
    }
}

/// Pin type of a declared input / constant (`type` number): 0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4,
/// 6 int2, 7 int3, 8 int4, 9 string.
pub fn type_pin(t: i32) -> Pin {
    match t {
        1 => Pin::Int,
        2 => Pin::Bool,
        3 => Pin::Float2,
        4 => Pin::Float3,
        5 => Pin::Float4,
        6 => Pin::Int2,
        7 => Pin::Int3,
        8 => Pin::Int4,
        9 => Pin::Text,
        _ => Pin::Float,
    }
}

/// Convert `v` to the declared type `p` the way an input does (a number fills the lanes, a vector is cut or padded).
pub fn coerce(v: &Value, p: Pin) -> Value {
    match (p, v) {
        (Pin::Bool, _) => Value::Bool(v.as_f32() != 0.0),
        (Pin::Text, _) => Value::text(&v.show()),
        (_, Value::Str(_)) => dummy(p).clone_zero(),
        (_, _) => {
            let n = v.num().unwrap_or(Num { lanes: 1, int: false, v: [0.0; 4] });
            let lanes = p.lanes().max(1) as u8;
            let int = matches!(p, Pin::Int | Pin::Int2 | Pin::Int3 | Pin::Int4);
            let mut out = [0.0; 4];
            for (i, o) in out.iter_mut().enumerate().take(lanes as usize) {
                let x = if n.lanes == 1 {
                    n.v[0]
                } else if i < n.lanes as usize {
                    get(&n.v, i)
                } else {
                    0.0
                };
                *o = if int { x.trunc() } else { x };
            }
            Value::Num(Num { lanes, int, v: out })
        }
    }
}

impl Value {
    fn clone_zero(&self) -> Value {
        match self {
            Value::Num(n) => Value::Num(Num { v: [0.0; 4], ..*n }),
            Value::Bool(_) => Value::Bool(false),
            Value::Str(_) => Value::text(""),
        }
    }
}

// ------------------------------------------------------------------------------------------------ node table

macro_rules! f {
    ($k:expr, [$($r:expr),*], [$($o:expr),*], $d:expr) => {
        Spec { kind: $k, req: &[$($r),*], opt: &[$($o),*], doc: $d }
    };
}

/// Node kinds of function graphs. Slots: `a`, `b`, `c` for operands, `x y z w` for vector parts, `cond` for `f_if`.
pub const FUNC_SPECS: &[Spec] = &[
    f!("f_out", ["in"], [], "result of the function"),
    f!("f_pin", ["a"], [], "re-route a value link"),
    f!("f_float", [], [], "constant float: v"),
    f!("f_int", [], [], "constant integer: v"),
    f!("f_bool", [], [], "constant boolean: v (0 or 1)"),
    f!("f_string", [], [], "constant string: v"),
    f!("f_vec2", [], ["x", "y"], "float2 from parts (x y, linked parts win over the typed values)"),
    f!("f_vec3", [], ["x", "y", "z"], "float3 from parts"),
    f!("f_vec4", [], ["x", "y", "z", "w"], "float4 from parts"),
    f!("f_ivec2", [], ["x", "y"], "integer2 from parts"),
    f!("f_ivec3", [], ["x", "y", "z"], "integer3 from parts"),
    f!("f_ivec4", [], ["x", "y", "z", "w"], "integer4 from parts"),
    f!("f_swizzle", ["a"], [], "pick or reorder components: mask (xyzw or rgba, e.g. \"zyx\")"),
    f!("f_neg", ["a"], [], "negate"),
    f!("f_abs", ["a"], [], "absolute value"),
    f!("f_sqrt", ["a"], [], "square root (0 for negative input)"),
    f!("f_exp", ["a"], [], "e to the power a"),
    f!("f_log", ["a"], [], "natural logarithm (0 for input <= 0)"),
    f!("f_log_floor", ["a"], [], "natural logarithm, very low for input <= 0 (the engine of compiled graphs)"),
    f!("f_log2", ["a"], [], "base-2 logarithm (0 for input <= 0)"),
    f!("f_sin", ["a"], [], "sine (radians)"),
    f!("f_cos", ["a"], [], "cosine (radians)"),
    f!("f_sin_poly", ["a"], [], "sine by the polynomial of the engine of compiled graphs' fx-maps and node parameters"),
    f!("f_cos_poly", ["a"], [], "cosine by the polynomial of the engine of compiled graphs' fx-maps and node parameters"),
    f!("f_sin_f32turns", ["a"], [], "sine as a compiled graph's pixel processor has it on a GPU: the turn count truncated in single precision"),
    f!("f_cos_f32turns", ["a"], [], "cosine as a compiled graph's pixel processor has it on a GPU: the turn count truncated in single precision"),
    f!("f_tan", ["a"], [], "tangent (radians)"),
    f!("f_asin", ["a"], [], "inverse sine (input clamped to -1..1)"),
    f!("f_acos", ["a"], [], "inverse cosine (input clamped to -1..1)"),
    f!("f_atan", ["a"], [], "inverse tangent"),
    f!("f_radians", ["a"], [], "degrees to radians"),
    f!("f_degrees", ["a"], [], "radians to degrees"),
    f!("f_floor", ["a"], [], "round down"),
    f!("f_ceil", ["a"], [], "round up"),
    f!("f_round", ["a"], [], "round to nearest"),
    f!("f_fract", ["a"], [], "fractional part"),
    f!("f_sign", ["a"], [], "-1, 0 or 1"),
    f!("f_saturate", ["a"], [], "clamp to 0..1"),
    f!("f_not", ["a"], [], "boolean not"),
    f!("f_length", ["a"], [], "length of a vector"),
    f!("f_normalize", ["a"], [], "vector of length 1 (zero stays zero)"),
    f!("f_add", ["a", "b"], [], "a + b"),
    f!("f_sub", ["a", "b"], [], "a - b"),
    f!("f_mul", ["a", "b"], [], "a * b"),
    f!("f_div", ["a", "b"], [], "a / b (0 when b is 0; integer division truncates)"),
    f!("f_div_large", ["a", "b"], [], "a / b, very large when b is 0 and a is not (the engine of compiled graphs)"),
    f!("f_mod", ["a", "b"], [], "remainder of a / b with the sign of b (0 when b is 0)"),
    f!("f_pow", ["a", "b"], [], "a to the power b"),
    f!("f_min", ["a", "b"], [], "smaller value"),
    f!("f_max", ["a", "b"], [], "larger value"),
    f!("f_atan2", ["a", "b"], [], "angle of the point (b, a)"),
    f!("f_atan2_approx", ["a", "b"], [], "angle of the point (b, a), by the approximation of the engine of compiled graphs"),
    f!("f_step", ["a", "b"], [], "0 when b < a, else 1"),
    f!("f_dot", ["a", "b"], [], "dot product"),
    f!("f_cross", ["a", "b"], [], "cross product of two float3"),
    f!("f_distance", ["a", "b"], [], "distance between two points"),
    f!("f_clamp", ["a", "b", "c"], [], "clamp a between b (low) and c (high)"),
    f!("f_lerp", ["a", "b", "c"], [], "a + (b - a) * c"),
    f!("f_smoothstep", ["a", "b", "c"], [], "smooth 0..1 ramp of c between edges a and b"),
    f!("f_eq", ["a", "b"], [], "a == b"),
    f!("f_ne", ["a", "b"], [], "a != b"),
    f!("f_lt", ["a", "b"], [], "a < b"),
    f!("f_le", ["a", "b"], [], "a <= b"),
    f!("f_gt", ["a", "b"], [], "a > b"),
    f!("f_ge", ["a", "b"], [], "a >= b"),
    f!("f_and", ["a", "b"], [], "boolean and"),
    f!("f_or", ["a", "b"], [], "boolean or"),
    f!("f_xor", ["a", "b"], [], "boolean exclusive or"),
    f!("f_if", ["cond", "a", "b"], [], "a when cond is true, else b (only the chosen branch runs)"),
    f!("f_sequence", ["a", "b"], [], "run a, then b; returns b"),
    f!("f_loop", ["a", "c"], ["b"], "run a, then c again and again: until b is true, or `passes` times; returns 0"),
    f!("f_set", ["a"], [], "store a in the variable `name`; returns a"),
    f!("f_get", [], [], "read the variable `name` (0 until it is set); type: what it holds, for type checks (default float)"),
    f!(
        "f_random",
        [],
        ["a", "b", "s"],
        "random float between a (default 0) and b (default 1); `seed` shifts the sequence; `lcg` draws from a 24-bit generator as compiled graphs do, by where the function runs (pixel, value, parameter, size, fx_luminance ... fx_image_index; see names::LCG_SITES and lcg_rand), with seed s and draw number k"
    ),
    f!(
        "f_sample_gray",
        ["a"],
        [],
        "brightness of input image `slot` (default in) at the position a (float2, 0..1); filter: bit 0 bilinear, else nearest, bit 1 keep the edge, else repeat (default bilinear, repeat)"
    ),
    f!("f_sample_color", ["a"], [], "colour (float4) of input image `slot` at the position a; filter as for f_sample_gray"),
    f!("f_image_avg", [], [], "average colour (float4) of input image `slot`"),
    f!("f_image_min", [], [], "smallest channel values (float4) of input image `slot`"),
    f!("f_image_max", [], [], "largest channel values (float4) of input image `slot`"),
    f!("f_pos", [], [], "position being evaluated (float2, 0..1)"),
    f!("f_size", [], [], "output size in pixels (float2)"),
    f!("f_own_size", [], [], "size in pixels (float2) of the node whose parameter this computes; the output size elsewhere"),
    f!("f_cell_size", [], [], "size of the cell a pattern program is walking, in uv (float; 1 elsewhere)"),
    f!("f_cell_depth", [], [], "how many quadrants deep the cell of a pattern program is (float; 0 elsewhere)"),
    f!("f_index", [], [], "instance / iteration number (integer)"),
    f!("f_count", [], [], "number of instances / iterations (integer)"),
    f!("f_seed", [], [], "random seed of the caller (integer)"),
    f!(
        "f_input",
        [],
        [],
        "input of this function: name, type (0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4, 6 int2, 7 int3, 8 int4, 9 string), default v / x y z w"
    ),
    f!("f_graph_input", [], [], "value of an Input Value node of the graph the caller lives in: name, type, default v / x y z w"),
    f!("f_to_float", ["a"], [], "convert to float"),
    f!("f_to_int", ["a"], [], "convert to integer (drops the fraction)"),
    f!("f_to_bool", ["a"], [], "non-zero is true"),
    f!("f_to_string", ["a"], [], "text of a value"),
    f!("f_concat", ["a", "b"], [], "join two strings"),
    f!("f_call", [], ["a", "b", "c", "d"], "run another function (`fn`); a b c d fill its inputs in the order of their node numbers"),
];

/// Is this kind a function-graph node?
pub fn is_func_kind(kind: &str) -> bool {
    kind.starts_with("f_")
}

/// True when a graph is a function graph (meta `kind "function"`).
pub fn is_function(g: &Graph) -> bool {
    g.meta.get("kind").map(|k| k == "function").unwrap_or(false)
}

// ------------------------------------------------------------------------------------------------ operations

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Out,
    Pin,
    Float,
    Int,
    Bool,
    Str,
    Vec(u8, bool),
    Swizzle,
    Un(U),
    Bin(B),
    Clamp,
    Lerp,
    Smooth,
    If,
    Seq,
    Loop,
    Set,
    Get,
    Random,
    SampleGray,
    SampleColor,
    Stat(u8),
    Pos,
    Size,
    OwnSize,
    CellSize,
    CellDepth,
    Index,
    Count,
    Seed,
    Input,
    GraphInput,
    ToFloat,
    ToInt,
    ToBool,
    ToStr,
    Concat,
    Call,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum U {
    Neg,
    Abs,
    Sqrt,
    Exp,
    Log,
    LogFloor,
    Log2,
    Sin,
    Cos,
    SinPoly,
    CosPoly,
    SinF32Turns,
    CosF32Turns,
    Tan,
    Asin,
    Acos,
    Atan,
    Rad,
    Deg,
    Floor,
    Ceil,
    Round,
    Fract,
    Sign,
    Sat,
    Not,
    Length,
    Normalize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum B {
    Add,
    Sub,
    Mul,
    Div,
    DivLarge,
    Mod,
    Pow,
    Min,
    Max,
    Atan2,
    Atan2Approx,
    Step,
    Dot,
    Cross,
    Distance,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
}

fn op_of(kind: &str) -> Option<Op> {
    use B::*;
    use U::*;
    Some(match kind {
        "f_out" => Op::Out,
        "f_pin" => Op::Pin,
        "f_float" => Op::Float,
        "f_int" => Op::Int,
        "f_bool" => Op::Bool,
        "f_string" => Op::Str,
        "f_vec2" => Op::Vec(2, false),
        "f_vec3" => Op::Vec(3, false),
        "f_vec4" => Op::Vec(4, false),
        "f_ivec2" => Op::Vec(2, true),
        "f_ivec3" => Op::Vec(3, true),
        "f_ivec4" => Op::Vec(4, true),
        "f_swizzle" => Op::Swizzle,
        "f_neg" => Op::Un(Neg),
        "f_abs" => Op::Un(Abs),
        "f_sqrt" => Op::Un(Sqrt),
        "f_exp" => Op::Un(Exp),
        "f_log" => Op::Un(Log),
        "f_log_floor" => Op::Un(LogFloor),
        "f_log2" => Op::Un(Log2),
        "f_sin" => Op::Un(Sin),
        "f_cos" => Op::Un(Cos),
        "f_sin_poly" => Op::Un(SinPoly),
        "f_cos_poly" => Op::Un(CosPoly),
        "f_sin_f32turns" => Op::Un(SinF32Turns),
        "f_cos_f32turns" => Op::Un(CosF32Turns),
        "f_tan" => Op::Un(Tan),
        "f_asin" => Op::Un(Asin),
        "f_acos" => Op::Un(Acos),
        "f_atan" => Op::Un(Atan),
        "f_radians" => Op::Un(Rad),
        "f_degrees" => Op::Un(Deg),
        "f_floor" => Op::Un(Floor),
        "f_ceil" => Op::Un(Ceil),
        "f_round" => Op::Un(Round),
        "f_fract" => Op::Un(Fract),
        "f_sign" => Op::Un(Sign),
        "f_saturate" => Op::Un(Sat),
        "f_not" => Op::Un(Not),
        "f_length" => Op::Un(Length),
        "f_normalize" => Op::Un(Normalize),
        "f_add" => Op::Bin(Add),
        "f_sub" => Op::Bin(Sub),
        "f_mul" => Op::Bin(Mul),
        "f_div" => Op::Bin(Div),
        "f_div_large" => Op::Bin(DivLarge),
        "f_mod" => Op::Bin(Mod),
        "f_pow" => Op::Bin(Pow),
        "f_min" => Op::Bin(Min),
        "f_max" => Op::Bin(Max),
        "f_atan2" => Op::Bin(Atan2),
        "f_atan2_approx" => Op::Bin(Atan2Approx),
        "f_step" => Op::Bin(Step),
        "f_dot" => Op::Bin(Dot),
        "f_cross" => Op::Bin(Cross),
        "f_distance" => Op::Bin(Distance),
        "f_eq" => Op::Bin(Eq),
        "f_ne" => Op::Bin(Ne),
        "f_lt" => Op::Bin(Lt),
        "f_le" => Op::Bin(Le),
        "f_gt" => Op::Bin(Gt),
        "f_ge" => Op::Bin(Ge),
        "f_and" => Op::Bin(And),
        "f_or" => Op::Bin(Or),
        "f_xor" => Op::Bin(Xor),
        "f_clamp" => Op::Clamp,
        "f_lerp" => Op::Lerp,
        "f_smoothstep" => Op::Smooth,
        "f_if" => Op::If,
        "f_sequence" => Op::Seq,
        "f_loop" => Op::Loop,
        "f_set" => Op::Set,
        "f_get" => Op::Get,
        "f_random" => Op::Random,
        "f_sample_gray" => Op::SampleGray,
        "f_sample_color" => Op::SampleColor,
        "f_image_avg" => Op::Stat(0),
        "f_image_min" => Op::Stat(1),
        "f_image_max" => Op::Stat(2),
        "f_pos" => Op::Pos,
        "f_size" => Op::Size,
        "f_own_size" => Op::OwnSize,
        "f_cell_size" => Op::CellSize,
        "f_cell_depth" => Op::CellDepth,
        "f_index" => Op::Index,
        "f_count" => Op::Count,
        "f_seed" => Op::Seed,
        "f_input" => Op::Input,
        "f_graph_input" => Op::GraphInput,
        "f_to_float" => Op::ToFloat,
        "f_to_int" => Op::ToInt,
        "f_to_bool" => Op::ToBool,
        "f_to_string" => Op::ToStr,
        "f_concat" => Op::Concat,
        "f_call" => Op::Call,
        _ => return None,
    })
}

fn clean(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

fn lanes_of(a: &Num, b: &Num) -> Result<u8, String> {
    if a.lanes == b.lanes {
        Ok(a.lanes)
    } else if a.lanes == 1 {
        Ok(b.lanes)
    } else if b.lanes == 1 {
        Ok(a.lanes)
    } else {
        Err(format!("cannot combine {} lanes with {} lanes", a.lanes, b.lanes))
    }
}

fn lane(n: &Num, i: usize) -> f64 {
    if n.lanes == 1 { n.v[0] } else { get(&n.v, i) }
}

fn map1(a: &Num, int_out: bool, f: impl Fn(f64) -> f64) -> Value {
    let mut v = [0.0; 4];
    for (o, x) in v.iter_mut().zip(a.v).take(a.lanes as usize) {
        let r = clean(f(x));
        *o = if int_out { r.trunc() } else { r };
    }
    Value::Num(Num { lanes: a.lanes, int: int_out, v })
}

/// sin in fx-maps and node parameters of compiled graphs (measured to 2^-22 through luminance on 4096 angles, and in uniforms' parameters):
/// degree 7 on -pi/2..pi/2, u - u^3/6 + u^5/120 - 0.000191765 u^7, folded, up to 1.5e-5 from the true value near +-1; residual 3e-7.
/// cos(x) is sin(pi/2 - x). A pixel processor's functions don't do this.
fn engine_sin(x: f64) -> f64 {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let x = x as f32;
    let u = x - (x / TAU).round() * TAU;
    let (sign, u) = if u < 0.0 { (-1.0f32, -u) } else { (1.0, u) };
    let u = if u > FRAC_PI_2 { PI - u } else { u };
    let q = u * u;
    f64::from(sign * u * (1.0 + q * (-1.0 / 6.0 + q * (1.0 / 120.0 + q * -0.000_191_765))))
}

/// The part of a turn that sin and cos read in a pixel processor of a compiled graph: `x / 2 pi` with the product rounded toward zero in
/// single precision, less its floor; the sine or cosine of that is then accurate. Read to about 2^-28 through the reference's GPU engine
/// (fit error 7e-8 at x up to 10,000, where an accurate cos of x is up to 7e-4 off). It matters for hashes such as
/// `fract(cos(..) * 12345.7)`, which spread that difference over the whole range.
///
/// LIMIT: measured on one GPU only, an NVIDIA RTX 5080, where the reference's Vulkan and OpenGL engines agree. Its CPU engine differs here
/// (mean 35 to 50 on these nodes), and another maker's GPU may too, so this matches that card's output, not every machine's. Chosen knowingly.
fn gpu_turn(x: f64) -> f64 {
    let product = f64::from(x as f32) * f64::from(std::f32::consts::FRAC_1_PI / 2.0);
    let near = product as f32;
    let t = if f64::from(near).abs() > product.abs() { if near > 0.0 { near.next_down() } else { near.next_up() } } else { near };
    f64::from(t - t.floor())
}

/// atan2 as compiled graphs compute it (measured, every bit of a float on a full circle of points): a polynomial of degree 4 in
/// min(|x|, |y|) / max(|x|, |y|), up to 0.00024 away from the true angle, folded by octant.
fn engine_atan2(y: f64, x: f64) -> f64 {
    let (ax, ay) = (x.abs() as f32, y.abs() as f32);
    if ax == 0.0 && ay == 0.0 {
        return 0.0;
    }
    let t = ax.min(ay) / ax.max(ay);
    let p = t * (1.002_489_8 + t * (-0.012_381_5 + t * (-0.347_314_36 + t * 0.142_604_62)));
    let r = if ay > ax { std::f32::consts::FRAC_PI_2 - p } else { p };
    let r = if x < 0.0 { std::f32::consts::PI - r } else { r };
    f64::from(if y < 0.0 { -r } else { r })
}

fn map2(a: &Num, b: &Num, int_out: bool, f: impl Fn(f64, f64) -> f64) -> Result<Value, String> {
    let lanes = lanes_of(a, b)?;
    let mut v = [0.0; 4];
    for (i, o) in v.iter_mut().enumerate().take(lanes as usize) {
        let r = clean(f(lane(a, i), lane(b, i)));
        *o = if int_out { r.trunc() } else { r };
    }
    Ok(Value::Num(Num { lanes, int: int_out, v }))
}

fn unary(op: U, x: &Value) -> Result<Value, String> {
    if op == U::Not {
        return match x {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            _ => Err("f_not needs a boolean".into()),
        };
    }
    let a = x.num()?;
    let keep_int = a.int;
    Ok(match op {
        U::Neg => map1(&a, keep_int, |x| -x),
        U::Abs => map1(&a, keep_int, f64::abs),
        U::Sign => map1(&a, keep_int, |x| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }),
        U::Floor => map1(&a, keep_int, f64::floor),
        U::Ceil => map1(&a, keep_int, f64::ceil),
        U::Round => map1(&a, keep_int, f64::round),
        U::Sqrt => map1(&a, false, |x| x.max(0.0).sqrt()),
        U::Exp => map1(&a, false, |x| math::exp(x.min(80.0))),
        U::Log => map1(&a, false, |x| if x > 0.0 { math::ln(x) } else { 0.0 }),
        U::LogFloor => map1(&a, false, |x| if x > 0.0 { math::ln(x) } else { -HUGE }),
        U::Log2 => map1(&a, false, |x| if x > 0.0 { math::log2(x) } else { 0.0 }),
        U::Sin => map1(&a, false, math::sin),
        U::Cos => map1(&a, false, math::cos),
        U::SinPoly => map1(&a, false, engine_sin),
        U::CosPoly => map1(&a, false, |x| engine_sin(std::f64::consts::FRAC_PI_2 - x)),
        U::SinF32Turns => map1(&a, false, |x| math::sin(std::f64::consts::TAU * gpu_turn(x))),
        U::CosF32Turns => map1(&a, false, |x| math::cos(std::f64::consts::TAU * gpu_turn(x))),
        U::Tan => map1(&a, false, math::tan),
        U::Asin => map1(&a, false, |x| math::asin(x.clamp(-1.0, 1.0))),
        U::Acos => map1(&a, false, |x| math::acos(x.clamp(-1.0, 1.0))),
        U::Atan => map1(&a, false, math::atan),
        U::Rad => map1(&a, false, f64::to_radians),
        U::Deg => map1(&a, false, f64::to_degrees),
        U::Fract => map1(&a, false, |x| x - x.floor()),
        U::Sat => map1(&a, false, |x| x.clamp(0.0, 1.0)),
        U::Length => {
            let s: f64 = a.v.iter().take(a.lanes as usize).map(|x| x * x).sum();
            Value::float(clean(s.sqrt()))
        }
        U::Normalize => {
            let s: f64 = a.v.iter().take(a.lanes as usize).map(|x| x * x).sum::<f64>().sqrt();
            if s > 1e-12 { map1(&Num { int: false, ..a }, false, |x| x / s) } else { map1(&Num { int: false, ..a }, false, |_| 0.0) }
        }
        // answered above, before the operand is read as a number
        U::Not => return Err("f_not needs a boolean".into()),
    })
}

#[inline]
fn div_large(p: f64, q: f64) -> f64 {
    if q != 0.0 {
        p / q
    } else if p == 0.0 {
        0.0
    } else {
        HUGE.copysign(p)
    }
}

fn binary(op: B, x: &Value, y: &Value) -> Result<Value, String> {
    match op {
        B::And | B::Or | B::Xor => {
            return match (x, y) {
                (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(match op {
                    B::And => *a && *b,
                    B::Or => *a || *b,
                    _ => a != b,
                })),
                _ => Err("boolean operators need two booleans".into()),
            };
        }
        B::Eq | B::Ne => {
            let same = match (x, y) {
                (Value::Str(a), Value::Str(b)) => a == b,
                (Value::Bool(a), Value::Bool(b)) => a == b,
                (Value::Str(_), _) | (_, Value::Str(_)) => return Err("cannot compare a string with a number".into()),
                _ => {
                    let (a, b) = (x.num()?, y.num()?);
                    let lanes = lanes_of(&a, &b)?;
                    (0..lanes as usize).all(|i| lane(&a, i) == lane(&b, i))
                }
            };
            return Ok(Value::Bool(if op == B::Eq { same } else { !same }));
        }
        _ => {}
    }
    let (a, b) = (x.num()?, y.num()?);
    let both_int = a.int && b.int;
    match op {
        B::Add => map2(&a, &b, both_int, |p, q| p + q),
        B::Sub => map2(&a, &b, both_int, |p, q| p - q),
        B::Mul => map2(&a, &b, both_int, |p, q| p * q),
        B::DivLarge if !both_int => map2(&a, &b, false, div_large),
        B::Div | B::DivLarge => map2(&a, &b, both_int, |p, q| {
            if q == 0.0 {
                0.0
            } else if both_int {
                (p / q).trunc()
            } else {
                p / q
            }
        }),
        // in single precision for floats: 1.0 mod 0.2 is then 0, and not nearly 0.2 (0.2 is a little above 1/5)
        B::Mod => map2(&a, &b, both_int, |p, q| {
            if q == 0.0 {
                0.0
            } else if both_int {
                p - q * (p / q).floor()
            } else {
                let (p, q) = (p as f32, q as f32);
                f64::from(p - q * (p / q).floor())
            }
        }),
        B::Pow => map2(&a, &b, false, |p, q| {
            let r = math::powf(p, q);
            if p < 0.0 && q.fract() != 0.0 { 0.0 } else { r }
        }),
        B::Min => map2(&a, &b, both_int, f64::min),
        B::Max => map2(&a, &b, both_int, f64::max),
        B::Atan2 => map2(&a, &b, false, math::atan2),
        B::Atan2Approx => map2(&a, &b, false, engine_atan2),
        B::Step => map2(&a, &b, false, |e, v| if v < e { 0.0 } else { 1.0 }),
        B::Lt | B::Le | B::Gt | B::Ge => {
            if a.lanes != 1 || b.lanes != 1 {
                return Err("ordering comparisons need scalars".into());
            }
            let (p, q) = (a.v[0], b.v[0]);
            Ok(Value::Bool(match op {
                B::Lt => p < q,
                B::Le => p <= q,
                B::Gt => p > q,
                _ => p >= q,
            }))
        }
        B::Dot => {
            let lanes = lanes_of(&a, &b)?;
            Ok(Value::float(clean((0..lanes as usize).map(|i| lane(&a, i) * lane(&b, i)).sum())))
        }
        B::Distance => {
            let lanes = lanes_of(&a, &b)?;
            Ok(Value::float(clean((0..lanes as usize).map(|i| math::powi(lane(&a, i) - lane(&b, i), 2)).sum::<f64>().sqrt())))
        }
        B::Cross => {
            if a.lanes != 3 || b.lanes != 3 {
                return Err("f_cross needs two float3 values".into());
            }
            let (p, q) = (a.v, b.v);
            Ok(Value::vec(3, [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0], 0.0]))
        }
        // answered above, before the operands are read as numbers
        B::And | B::Or | B::Xor | B::Eq | B::Ne => Err("internal: a logical operator reached arithmetic".into()),
    }
}

fn clamp3(x: &Value, lo: &Value, hi: &Value) -> Result<Value, String> {
    let (a, b, c) = (x.num()?, lo.num()?, hi.num()?);
    let lanes = lanes_of(&a, &lanes_num(&b, &c)?)?;
    let int = a.int && b.int && c.int;
    let mut v = [0.0; 4];
    for (i, o) in v.iter_mut().enumerate().take(lanes as usize) {
        let (l, h) = (lane(&b, i), lane(&c, i));
        *o = clean(lane(&a, i).max(l.min(h)).min(l.max(h)));
    }
    Ok(Value::Num(Num { lanes, int, v }))
}

fn lanes_num(a: &Num, b: &Num) -> Result<Num, String> {
    Ok(Num { lanes: lanes_of(a, b)?, int: false, v: [0.0; 4] })
}

fn lerp3(a: &Value, b: &Value, t: &Value) -> Result<Value, String> {
    if let (
        Value::Num(Num { lanes: 1, v: [p, ..], .. }),
        Value::Num(Num { lanes: 1, v: [q, ..], .. }),
        Value::Num(Num { lanes: 1, v: [t, ..], .. }),
    ) = (a, b, t)
    {
        return Ok(Value::float(clean(p + (q - p) * t)));
    }
    let (a, b, t) = (a.num()?, b.num()?, t.num()?);
    let lanes = lanes_of(&lanes_num(&a, &b)?, &t)?;
    let mut v = [0.0; 4];
    for (i, o) in v.iter_mut().enumerate().take(lanes as usize) {
        let (p, q) = (lane(&a, i), lane(&b, i));
        *o = clean(p + (q - p) * lane(&t, i));
    }
    Ok(Value::Num(Num { lanes, int: false, v }))
}

fn smooth3(e0: &Value, e1: &Value, x: &Value) -> Result<Value, String> {
    let (a, b, x) = (e0.num()?, e1.num()?, x.num()?);
    let lanes = lanes_of(&lanes_num(&a, &b)?, &x)?;
    let mut v = [0.0; 4];
    for (i, o) in v.iter_mut().enumerate().take(lanes as usize) {
        let (lo, hi) = (lane(&a, i), lane(&b, i));
        let t = if (hi - lo).abs() < 1e-12 { if lane(&x, i) < lo { 0.0 } else { 1.0 } } else { ((lane(&x, i) - lo) / (hi - lo)).clamp(0.0, 1.0) };
        *o = t * t * (3.0 - 2.0 * t);
    }
    Ok(Value::Num(Num { lanes, int: false, v }))
}

fn swizzle(a: &Value, mask: &str) -> Result<Value, String> {
    let n = a.num()?;
    if mask.is_empty() || mask.chars().count() > 4 {
        return Err("a swizzle mask has 1 to 4 letters".into());
    }
    let mut v = [0.0; 4];
    let mut lanes = 0u8;
    for c in mask.chars() {
        let i = match c {
            'x' | 'r' => 0,
            'y' | 'g' => 1,
            'z' | 'b' => 2,
            'w' | 'a' => 3,
            _ => return Err(format!("bad swizzle letter '{c}'")),
        };
        if i >= n.lanes as usize && !(n.lanes == 1 && i == 0) {
            return Err(format!("component '{c}' does not exist in a value with {} lane(s)", n.lanes));
        }
        if let Some(o) = v.get_mut(lanes as usize) {
            *o = get(&n.v, i);
        }
        lanes += 1;
    }
    Ok(Value::Num(Num { lanes, int: n.int, v }))
}

/// A vector from its parts: what is not linked comes from `defaults`.
#[inline]
fn build_vec_ref(lanes: u8, int: bool, parts: [Option<&Value>; 4], defaults: [f64; 4]) -> Result<Value, String> {
    let mut v = [0.0; 4];
    let mut at = 0usize;
    let mut slot = 0usize;
    // a linked part may itself be a vector: its lanes fill the following components
    while at < lanes as usize && slot < 4 {
        match parts.get(slot).copied().flatten() {
            Some(p) => {
                let n = p.num()?;
                for x in n.v.iter().take(n.lanes as usize) {
                    if at < lanes as usize {
                        if let Some(o) = v.get_mut(at) {
                            *o = *x;
                        }
                        at += 1;
                    }
                }
            }
            None => {
                if let Some(o) = v.get_mut(at) {
                    *o = get(&defaults, at);
                }
                at += 1;
            }
        }
        slot += 1;
    }
    while at < lanes as usize {
        if let Some(o) = v.get_mut(at) {
            *o = get(&defaults, at);
        }
        at += 1;
    }
    if int {
        for x in v.iter_mut() {
            *x = x.trunc();
        }
    }
    Ok(Value::Num(Num { lanes, int, v }))
}

/// Result type of applying a pure operation to inputs of the given types (used by the editor and the checker).
fn infer_pure(op: Op, args: &[Option<Pin>], mask: &str) -> Result<Pin, String> {
    let d: Vec<Option<Value>> = args.iter().map(|p| p.map(dummy)).collect();
    let get = |i: usize| -> Result<&Value, String> { d.get(i).and_then(|v| v.as_ref()).ok_or_else(|| "an input is not connected".to_string()) };
    let r = match op {
        Op::Un(u) => unary(u, get(0)?)?,
        Op::Bin(b) => binary(b, get(0)?, get(1)?)?,
        Op::Clamp => clamp3(get(0)?, get(1)?, get(2)?)?,
        Op::Lerp => lerp3(get(0)?, get(1)?, get(2)?)?,
        Op::Smooth => smooth3(get(0)?, get(1)?, get(2)?)?,
        Op::Swizzle => swizzle(get(0)?, mask)?,
        Op::ToFloat => coerce(&Value::Num(get(0)?.num()?), Pin::Float),
        Op::ToInt => {
            let n = get(0)?.num()?;
            Value::Num(Num { int: true, ..n })
        }
        Op::ToBool => Value::Bool(true),
        Op::ToStr => Value::text(""),
        Op::Concat => match (get(0)?, get(1)?) {
            (Value::Str(_), Value::Str(_)) => Value::text(""),
            _ => return Err("f_concat needs two strings".into()),
        },
        _ => return Err("not a pure operation".into()),
    };
    Ok(r.pin())
}

// ------------------------------------------------------------------------------------------------ programs

/// What the caller provides to a running function.
pub struct FnEnv<'a> {
    pub size: [f64; 2],
    pub index: i64,
    pub count: i64,
    pub seed: u32,
    /// the node whose parameter is computed, its size in pixels
    pub own: Option<[f64; 2]>,
    /// the cell a pattern program is walking: its size in uv and how many quadrants deep it is
    pub cell: [f64; 2],
    /// input images by slot name (`in`, `in2`, ...), read by the sampling nodes
    pub images: Vec<(&'a str, ImgRef<'a>)>,
    /// values of the Input Value nodes of the graph the caller lives in
    pub graph_inputs: &'a BTreeMap<String, Value>,
    /// the function's own inputs (`f_input`)
    pub args: BTreeMap<String, Value>,
    stats: Vec<ImageStats>,
}

/// An image's name and its per-channel statistics (three of them), as function graphs read them.
type ImageStats = (String, [f32; 4], [f32; 4], [f32; 4]);

static NO_INPUTS: BTreeMap<String, Value> = BTreeMap::new();

impl<'a> FnEnv<'a> {
    pub fn new(size: [f64; 2]) -> FnEnv<'a> {
        FnEnv {
            size,
            index: 0,
            count: 1,
            seed: 0,
            own: None,
            cell: [1.0, 0.0],
            images: Vec::new(),
            graph_inputs: &NO_INPUTS,
            args: BTreeMap::new(),
            stats: Vec::new(),
        }
    }

    /// Precompute image statistics for `f_image_avg` / `f_image_min` / `f_image_max` (call once before many runs).
    pub fn with_stats(mut self) -> FnEnv<'a> {
        self.stats = self
            .images
            .iter()
            .map(|(name, t)| {
                let (mut sum, mut mn, mut mx) = ([0.0f64; 4], [f32::MAX; 4], [f32::MIN; 4]);
                t.for_each_px(|p| {
                    for (((sum, mn), mx), p) in sum.iter_mut().zip(mn.iter_mut()).zip(mx.iter_mut()).zip(p) {
                        *sum += p as f64;
                        *mn = mn.min(p);
                        *mx = mx.max(p);
                    }
                });
                let n = t.len().max(1) as f64;
                (name.to_string(), [(sum[0] / n) as f32, (sum[1] / n) as f32, (sum[2] / n) as f32, (sum[3] / n) as f32], mn, mx)
            })
            .collect();
        self
    }

    fn image(&self, slot: &str) -> Option<ImgRef<'a>> {
        self.images.iter().find(|(n, _)| *n == slot).map(|(_, t)| *t)
    }
}

struct PNode {
    id: u32,
    op: Op,
    /// input node indices by slot position (the order of the spec: required slots, then optional ones)
    ins: [Option<usize>; 4],
    num: BTreeMap<String, f64>,
    text: String,
    name: String,
    callee: Option<Arc<Prog>>,
}

/// A compiled function graph.
pub struct Prog {
    nodes: Vec<PNode>,
    out: usize,
    /// declared inputs: (name, type, default), in node-number order
    pub inputs: Vec<(String, Pin, Value)>,
    pub uses_stats: bool,
    /// Every float result is rounded to single precision (meta `float32`): the engine of compiled graphs computes in it, and
    /// hashes made of large multiplications and `mod` give other numbers otherwise.
    single: bool,
    /// the form it runs in, made the first time it runs (compiling alone is also how the editor checks types)
    code: std::sync::OnceLock<code::Code>,
    /// the most nodes a run goes through one inside the other (a call counts its function's), see [`MAX_RUN_DEPTH`]
    depth: usize,
}

/// Variables and random-number position that outlive one run: a pattern program evaluates many functions
/// in sequence, and what one sets (`f_set`) the next reads (`f_get`).
#[derive(Debug, Clone, Default)]
pub struct FnState {
    vars: Vec<(String, Value)>,
    rng: u32,
    /// The compiled-graph generator's state: a fx-map sets it to the quadrant's.
    pub lcg_state: Option<u32>,
}

const MAX_CALL_DEPTH: usize = 8;

/// The most nodes a function may have one behind the other, the nodes of the functions it calls included. A run works
/// out a node by working out its inputs first, on the call stack (it is per pixel and must be quick), so this bounds the
/// stack a run takes: this deep fits in the stack of an ordinary thread (2 MB) in an optimised build, with room to spare
/// (measured: 2040 nodes in a row under 1 MB; unoptimised builds take about fifteen times that). A deeper function is
/// refused when it is compiled. (The deepest of the decompiled library has about 460.)
pub const MAX_RUN_DEPTH: usize = 2048;
/// `v` with float lanes in single precision. Double-precision results of + - * / round to exactly what single precision gives.
fn to_single(v: Value) -> Value {
    match v {
        Value::Num(mut n) if !n.int => {
            for x in n.v.iter_mut().take(n.lanes as usize) {
                *x = f64::from(*x as f32);
            }
            Value::Num(n)
        }
        other => other,
    }
}

/// A loop stops after this many passes, whatever its condition says.
const MAX_LOOP_PASSES: usize = 1 << 12;

/// How many passes a loop node may run: `passes`, at most [`MAX_LOOP_PASSES`] unless it says `long` (compiled graphs, whose
/// loops run the count they hold).
fn loop_passes(num: &BTreeMap<String, f64>) -> usize {
    let limit = if num.get("long").is_some_and(|v| *v > 0.5) { f64::from(u32::MAX) } else { MAX_LOOP_PASSES as f64 };
    num.get("passes").copied().unwrap_or(MAX_LOOP_PASSES as f64).clamp(0.0, limit) as usize
}

fn declared(n: &crate::graph::Node) -> (Pin, Value) {
    let pin = type_pin(n.nums.get("type").copied().unwrap_or(0.0).round() as i32);
    let g = |k: &str| n.nums.get(k).copied().unwrap_or(0.0) as f64;
    let v = match pin {
        Pin::Bool => Value::Bool(g("v") != 0.0),
        Pin::Text => Value::text(n.strs.get("v").map(String::as_str).unwrap_or("")),
        p if p.lanes() == 1 => coerce(&Value::float(g("v")), p),
        p => coerce(&Value::vec(p.lanes() as u8, [g("x"), g("y"), g("z"), g("w")]), p),
    };
    (pin, v)
}

impl Prog {
    /// Compile function graph `g`; `lib` supplies the functions that `f_call` nodes run.
    pub fn compile(g: &Graph, lib: &BTreeMap<String, Graph>) -> Result<Prog, String> {
        Prog::compile_depth(g, lib, 0)
    }

    fn compile_depth(g: &Graph, lib: &BTreeMap<String, Graph>, depth: usize) -> Result<Prog, String> {
        if depth >= MAX_CALL_DEPTH {
            return Err("functions call each other too deeply (a function calls itself?)".into());
        }
        let outs: Vec<u32> = g.nodes.iter().filter(|(_, n)| n.kind == "f_out").map(|(i, _)| *i).collect();
        let out_id = match outs.as_slice() {
            [one] => *one,
            [] => return Err("the function has no Output node".into()),
            _ => return Err("the function has more than one Output node".into()),
        };
        let mut prog = Prog {
            nodes: Vec::new(),
            out: 0,
            inputs: Vec::new(),
            uses_stats: false,
            single: g.meta.contains_key("float32"),
            code: std::sync::OnceLock::new(),
            depth: 0,
        };
        let mut index: HashMap<u32, usize> = HashMap::new();
        let mut visiting: HashSet<u32> = HashSet::new();
        prog.out = prog.add(g, lib, out_id, depth, &mut index, &mut visiting)?;
        // the nodes come after their inputs, so one pass finds how deep each is
        let mut deep: Vec<usize> = Vec::with_capacity(prog.nodes.len());
        for n in &prog.nodes {
            let below = n.ins.iter().flatten().filter_map(|j| deep.get(*j)).copied().max().unwrap_or(0);
            deep.push(below.saturating_add(1).saturating_add(n.callee.as_ref().map_or(0, |c| c.depth)));
        }
        prog.depth = deep.iter().copied().max().unwrap_or(0);
        if prog.depth > MAX_RUN_DEPTH {
            return Err(format!("the function is too deep to run: {} nodes one behind the other, at most {MAX_RUN_DEPTH}", prog.depth));
        }
        // declared inputs, including ones the result does not use (so a caller can still bind them)
        let mut ids: Vec<u32> = g.nodes.iter().filter(|(_, n)| n.kind == "f_input").map(|(i, _)| *i).collect();
        ids.sort();
        for n in ids.iter().filter_map(|id| g.nodes.get(id)) {
            let (pin, def) = declared(n);
            prog.inputs.push((n.strs.get("name").cloned().unwrap_or_default(), pin, def));
        }
        Ok(prog)
    }

    /// Adds node `id` and everything it reads (each before the nodes that read it), returning its index. The nodes
    /// waiting for their inputs are kept in a list, not on the call stack, so a long chain needs no more stack than a
    /// short one; the order is that of a depth-first walk (inputs in slot order).
    fn add(
        &mut self,
        g: &Graph,
        lib: &BTreeMap<String, Graph>,
        id: u32,
        depth: usize,
        index: &mut HashMap<u32, usize>,
        visiting: &mut HashSet<u32>,
    ) -> Result<usize, String> {
        /// A node whose inputs are being added: the slots it reads and how far along they are.
        struct Adding<'g> {
            id: u32,
            n: &'g crate::graph::Node,
            op: Op,
            slots: Vec<&'static str>,
            req: usize,
            pos: usize,
            ins: [Option<usize>; 4],
        }
        // the first look at a node: its index when it is in already, else the node to add
        let start = |id: u32, index: &HashMap<u32, usize>, visiting: &mut HashSet<u32>| -> Result<Result<usize, Adding>, String> {
            if let Some(i) = index.get(&id) {
                return Ok(Ok(*i));
            }
            if !visiting.insert(id) {
                return Err("the function graph contains a cycle".into());
            }
            let n = g.nodes.get(&id).ok_or_else(|| format!("node {id} is missing"))?;
            let op = op_of(&n.kind).ok_or_else(|| format!("node {id} ({}) is not a function node", n.kind))?;
            let sp = crate::graph::spec(&n.kind).ok_or_else(|| format!("unknown node {}", n.kind))?;
            let slots: Vec<&'static str> = sp.req.iter().chain(sp.opt.iter()).copied().take(4).collect();
            Ok(Err(Adding { id, n, op, slots, req: sp.req.len(), pos: 0, ins: [None; 4] }))
        };
        let mut stack = match start(id, index, visiting)? {
            Ok(i) => return Ok(i),
            Err(a) => vec![a],
        };
        // the index of the node finished last, for the node that asked for it
        let mut finished: Option<usize> = None;
        loop {
            let Some(top) = stack.last_mut() else { return Err(format!("node {id} is missing")) };
            if let Some(i) = finished.take()
                && let Some(slot) = top.pos.checked_sub(1).and_then(|p| top.ins.get_mut(p))
            {
                *slot = Some(i);
            }
            if let Some(slot) = top.slots.get(top.pos).copied() {
                let pos = top.pos;
                top.pos += 1;
                if let Some(src) = top.n.ins.get(slot) {
                    match start(*src, index, visiting)? {
                        Ok(i) => {
                            if let Some(s) = top.ins.get_mut(pos) {
                                *s = Some(i);
                            }
                        }
                        Err(a) => stack.push(a),
                    }
                } else if pos < top.req {
                    return Err(format!("node {} ({}) is missing its '{slot}' input", top.id, top.n.kind));
                }
                continue;
            }
            let Some(Adding { id, n, op, ins, .. }) = stack.pop() else { return Err(format!("node {id} is missing")) };
            let callee = if op == Op::Call {
                let name = n.strs.get("fn").ok_or_else(|| format!("node {id} (f_call) needs fn=\"name\""))?;
                let fg = lib.get(name).ok_or_else(|| format!("node {id}: unknown function '{name}'"))?;
                if !is_function(fg) {
                    return Err(format!("node {id}: '{name}' is not a function graph"));
                }
                Some(Arc::new(Prog::compile_depth(fg, lib, depth + 1).map_err(|e| format!("in function '{name}': {e}"))?))
            } else {
                None
            };
            if matches!(op, Op::Stat(_)) {
                self.uses_stats = true;
            }
            if let Some(c) = &callee
                && c.uses_stats
            {
                self.uses_stats = true;
            }
            let i = self.nodes.len();
            // integer constants take their exact digits; a float constant stays the single it reads as (a hash's 34359740000 is
            // 34359738368: metal_galvanized came out 3.2 off at its first fx-map with the digits)
            let integer = matches!(op, Op::Int | Op::Vec(_, true));
            self.nodes.push(PNode {
                id,
                op,
                ins,
                num: n
                    .nums
                    .iter()
                    .map(|(k, v)| {
                        let exact =
                            integer.then(|| n.strs.get(&format!("{k}{}", crate::graph::EXACT_SUFFIX))).flatten().and_then(|t| t.parse::<i64>().ok());
                        (k.clone(), exact.map_or(f64::from(*v), |i| i as f64))
                    })
                    .collect(),
                text: n.strs.get("v").or_else(|| n.strs.get("mask")).cloned().unwrap_or_default(),
                name: n.strs.get("name").or_else(|| n.strs.get("slot")).cloned().unwrap_or_default(),
                callee,
            });
            index.insert(id, i);
            visiting.remove(&id);
            if stack.is_empty() {
                return Ok(i);
            }
            finished = Some(i);
        }
    }

    /// Check types without running the function. `Ok` carries the type of the result.
    pub fn result_type(&self) -> Result<Pin, String> {
        let mut memo: Vec<Option<Pin>> = vec![None; self.nodes.len()];
        self.type_of(self.out, &mut memo)
    }

    fn type_of(&self, i: usize, memo: &mut Vec<Option<Pin>>) -> Result<Pin, String> {
        if let Some(p) = memo.get(i).copied().flatten() {
            return Ok(p);
        }
        let n = self.nodes.get(i).ok_or_else(|| format!("node index {i} is missing"))?;
        let r = self.type_node(n, memo).map_err(|e| if e.starts_with("node ") { e } else { format!("node {} ({}): {e}", n.id, kind_of(n.op)) })?;
        if let Some(m) = memo.get_mut(i) {
            *m = Some(r);
        }
        Ok(r)
    }

    fn type_node(&self, n: &PNode, memo: &mut Vec<Option<Pin>>) -> Result<Pin, String> {
        let t = |k: usize, memo: &mut Vec<Option<Pin>>| -> Result<Option<Pin>, String> {
            match n.ins.get(k).copied().flatten() {
                Some(j) => self.type_of(j, memo).map(Some),
                None => Ok(None),
            }
        };
        Ok(match n.op {
            Op::Out | Op::Pin => t(0, memo)?.ok_or("an input is not connected")?,
            Op::Float | Op::Random | Op::SampleGray | Op::CellSize | Op::CellDepth => Pin::Float,
            Op::Int | Op::Index | Op::Count | Op::Seed => Pin::Int,
            Op::Bool => Pin::Bool,
            Op::Str => Pin::Text,
            Op::Vec(lanes, int) => {
                for k in 0..4 {
                    t(k, memo)?;
                }
                type_pin(if int { 5 + lanes as i32 - 1 } else { 3 + lanes as i32 - 2 })
            }
            Op::Seq => {
                t(0, memo)?;
                t(1, memo)?.ok_or("an input is not connected")?
            }
            Op::Loop => {
                for k in 0..3 {
                    t(k, memo)?;
                }
                Pin::Float
            }
            Op::Set => t(0, memo)?.ok_or("an input is not connected")?,
            // a variable's type is what it was declared with: Float unless the node says otherwise
            Op::Get => type_pin(n.num.get("type").copied().unwrap_or(0.0).round() as i32),
            Op::If => {
                let c = t(0, memo)?.ok_or("an input is not connected")?;
                if c != Pin::Bool {
                    return Err("the condition must be a boolean".into());
                }
                let (a, b) = (t(1, memo)?.ok_or("an input is not connected")?, t(2, memo)?.ok_or("an input is not connected")?);
                if a != b && !(b.accepts(a) || a.accepts(b)) {
                    return Err(format!("the branches have different types ({} and {})", a.name(), b.name()));
                }
                if b.accepts(a) { b } else { a }
            }
            Op::SampleColor | Op::Stat(_) => Pin::Float4,
            Op::OwnSize | Op::Pos | Op::Size => Pin::Float2,
            Op::Input | Op::GraphInput => type_pin(n.num.get("type").copied().unwrap_or(0.0).round() as i32),
            Op::Call => n.callee.as_ref().ok_or("call without a function")?.result_type()?,
            Op::ToFloat
            | Op::ToInt
            | Op::ToBool
            | Op::ToStr
            | Op::Concat
            | Op::Un(_)
            | Op::Bin(_)
            | Op::Clamp
            | Op::Lerp
            | Op::Smooth
            | Op::Swizzle => {
                let args: Vec<Option<Pin>> = (0..3).map(|k| t(k, memo)).collect::<Result<_, _>>()?;
                infer_pure(n.op, &args, &n.text)?
            }
        })
    }
}

fn kind_of(op: Op) -> &'static str {
    match op {
        Op::Bin(_) | Op::Un(_) => "math",
        Op::If => "if",
        Op::Call => "call",
        Op::Loop => "loop",
        Op::Swizzle => "swizzle",
        _ => "function node",
    }
}

// ------------------------------------------------------------------------------------------------ helpers for callers

/// Values of the Input Value nodes of a texture graph, by name (the editor's graph parameters).
pub fn graph_inputs(g: &Graph) -> BTreeMap<String, Value> {
    graph_inputs_from(g.nodes.values())
}

pub(crate) fn graph_inputs_from<'a>(nodes: impl Iterator<Item = &'a crate::graph::Node>) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    for n in nodes.filter(|n| n.kind == "input_value") {
        let name = n.strs.get("name").cloned().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let (_, v) = declared(n);
        m.insert(name, v);
    }
    m
}

/// Compile the function called `name` from `lib`.
pub fn compile_named(lib: &BTreeMap<String, Graph>, name: &str) -> Result<Prog, String> {
    let g = lib.get(name).ok_or_else(|| format!("unknown function '{name}'"))?;
    if !is_function(g) {
        return Err(format!("'{name}' is not a function graph"));
    }
    Prog::compile(g, lib).map_err(|e| format!("function '{name}': {e}"))
}

/// Static types of the value nodes of a function graph, for the editor: node id to type or problem.
pub fn node_types(g: &Graph, lib: &BTreeMap<String, Graph>) -> HashMap<u32, Result<Pin, String>> {
    let mut out: HashMap<u32, Result<Pin, String>> = HashMap::new();
    // compile each node as if it were the output: cheap for the sizes of function graphs
    for (&id, n) in &g.nodes {
        if !is_func_kind(&n.kind) || n.kind == "f_out" {
            if n.kind == "f_out"
                && let Some(src) = n.ins.get("in")
                && let Some(r) = out.get(src).cloned().or_else(|| probe(g, lib, *src))
            {
                out.insert(id, r);
            }
            continue;
        }
        if let Some(r) = probe(g, lib, id) {
            out.insert(id, r);
        }
    }
    out
}

fn probe(g: &Graph, lib: &BTreeMap<String, Graph>, id: u32) -> Option<Result<Pin, String>> {
    // build a throw-away graph whose output reads node `id`
    let mut h = Graph { nodes: g.nodes.clone(), ..Graph::default() };
    h.nodes.retain(|_, n| n.kind != "f_out");
    let mut out = crate::graph::Node { kind: "f_out".into(), ..Default::default() };
    out.ins.insert("in".into(), id);
    let oid = h.nodes.keys().max().copied().unwrap_or(0) + 1;
    h.nodes.insert(oid, out);
    h.meta.insert("kind".into(), "function".into());
    Some(Prog::compile(&h, lib).and_then(|p| p.result_type()))
}

/// Run function `name` once (no position, no images): the common case of a function-driven parameter.
pub fn eval_once(lib: &BTreeMap<String, Graph>, name: &str, env: &FnEnv) -> Result<Value, String> {
    compile_named(lib, name)?.run(env, [0.5, 0.5])
}

/// Run the function graph `g` as if `id` were its result, at the centre of the image: the value an editor shows
/// next to a node.
pub fn probe_value(g: &Graph, lib: &BTreeMap<String, Graph>, id: u32, graph_inputs: &BTreeMap<String, Value>) -> Result<Value, String> {
    let mut h = Graph { nodes: g.nodes.clone(), ..Graph::default() };
    h.nodes.retain(|_, n| n.kind != "f_out");
    let mut out = crate::graph::Node { kind: "f_out".into(), ..Default::default() };
    out.ins.insert("in".into(), id);
    let oid = h.nodes.keys().max().copied().unwrap_or(0) + 1;
    h.nodes.insert(oid, out);
    h.meta.insert("kind".into(), "function".into());
    let prog = Prog::compile(&h, lib)?;
    let mut env = FnEnv::new([512.0, 512.0]);
    env.graph_inputs = graph_inputs;
    prog.run(&env, [0.5, 0.5])
}

#[cfg(test)]
mod single_tests {
    use super::{engine_atan2, engine_sin, gpu_turn};

    #[test]
    fn a_gpu_turn_truncates_its_product_in_single_precision() {
        let k = f64::from(std::f32::consts::FRAC_1_PI / 2.0);
        for x in [12000.0f32, 7321.5, 100.25, 0.75] {
            let exact = f64::from(x) * k;
            let t = gpu_turn(f64::from(x));
            // what is dropped is toward zero, and less than one float's step at that size
            let dropped = exact - exact.floor() - t;
            assert!((0.0..2.0 * f64::from(exact as f32) * f64::from(f32::EPSILON)).contains(&dropped), "{x}: {t} of {exact}");
        }
        // a negative product truncates toward zero too, which is upward
        let exact = -9001.0 * k;
        assert!(gpu_turn(-9001.0) >= exact - exact.floor());
        assert_eq!(gpu_turn(0.0), 0.0);
        assert!((gpu_turn(std::f64::consts::FRAC_PI_2) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn engine_sin_is_the_measured_polynomial_on_every_side() {
        let cos = |x: f64| engine_sin(std::f64::consts::FRAC_PI_2 - x);
        // measured offsets from the true value
        for (x, e) in [(0.2f64, 1.42e-5f64), (0.3, 1.22e-5), (0.4, 8.81e-6), (2.9, -1.36e-5), (3.3, -1.39e-5), (6.1, 1.43e-5)] {
            assert!((cos(x) - crate::math::cos(x) - e).abs() < 5e-7, "cos {x}: {}", cos(x) - crate::math::cos(x));
        }
        assert!((engine_sin(1.2) - crate::math::sin(1.2f64) - 9.75e-6).abs() < 5e-7);
        for x in [0.2f64, 2.9, 3.3, 6.1] {
            assert!((engine_sin(x) - crate::math::sin(x)).abs() < 5e-7, "sin {x}");
        }
        assert!((engine_sin(-1.2) + engine_sin(1.2)).abs() < 1e-7);
    }

    #[test]
    fn engine_atan2_folds_a_polynomial_by_octant() {
        // the measured offsets from the true angle, at t = 0.083 and t = 0.913 (radians)
        let e = |t: f64| engine_atan2(t, 1.0) - crate::math::atan(t);
        assert!((e(0.083) - 119.6e-6).abs() < 1e-6, "{}", e(0.083));
        assert!((e(0.913) + 235.1e-6).abs() < 1e-6, "{}", e(0.913));
        // rotating by a quarter turn keeps the offset, reflecting in an axis negates it
        let t = 0.083f64;
        assert!((engine_atan2(1.0, -t) - (std::f64::consts::FRAC_PI_2 + crate::math::atan(t)) - e(t)).abs() < 1e-6);
        assert!((engine_atan2(1.0, t) - (std::f64::consts::FRAC_PI_2 - crate::math::atan(t)) + e(t)).abs() < 1e-6);
        assert!((engine_atan2(-t, 1.0) + crate::math::atan(t) + e(t)).abs() < 1e-6);
        assert!((engine_atan2(t, -1.0) - (std::f64::consts::PI - crate::math::atan(t)) + e(t)).abs() < 1e-6);
    }

    #[test]
    fn a_float32_function_computes_in_single_precision() {
        // 16777216 + 1 is 16777216 in single precision, so the difference is 0 with the meta entry and 1 without
        let text = |meta: &str| {
            format!(
                "size 4\nsubgraph f\nmeta kind \"function\"\n{meta}node 2 f_float v=16777216\nnode 3 f_float v=1\nnode 4 f_add a=@2 b=@3\nnode 5 f_sub a=@4 b=@2\nnode 1 f_out in=@5\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1\n"
            )
        };
        let run = |m: &str| crate::graph::Graph::parse(&text(m)).unwrap().evaluate(Some(4)).unwrap()["o"].px[0][0];
        assert_eq!(run(""), 1.0);
        assert_eq!(run("meta float32 \"1\"\n"), 0.0);
    }

    #[test]
    fn an_integer_too_large_for_single_precision_keeps_its_digits() {
        // a seed of -1331112700 + 1 is -1331112699, where the nearest single is -1331112704; the low digits come out as 1
        let text = "size 4\nsubgraph f\nmeta kind \"function\"\nmeta float32 \"1\"\nnode 2 f_int v=-1331112700\nnode 3 f_int v=1\nnode 4 f_add a=@2 b=@3\nnode 5 f_int v=-1331112710\nnode 6 f_sub a=@4 b=@5\nnode 7 f_to_float a=@6\nnode 8 f_float v=11\nnode 9 f_div_large a=@7 b=@8\nnode 1 f_out in=@9\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1\n";
        let g = crate::graph::Graph::parse(text).unwrap();
        assert_eq!(g.evaluate(Some(4)).unwrap()["o"].px[0][0], 1.0);
        // and it survives being written out and read again
        let again = crate::graph::Graph::parse(&g.to_text()).unwrap();
        assert_eq!(again.evaluate(Some(4)).unwrap()["o"].px[0][0], 1.0);
        // a float constant stays a single: 34359740000 reads as 34359738368
        let text = "size 4\nsubgraph f\nmeta kind \"function\"\nmeta float32 \"1\"\nnode 2 f_float v=34359740000\nnode 3 f_float v=34359738000\nnode 4 f_sub a=@2 b=@3\nnode 1 f_out in=@4\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1\n";
        assert_eq!(crate::graph::Graph::parse(text).unwrap().evaluate(Some(4)).unwrap()["o"].px[0][0], 0.0);
    }
}

#[cfg(test)]
mod loop_tests {
    use crate::graph::Graph;

    #[test]
    fn a_loop_runs_until_its_condition_holds_and_is_read_through_its_variables() {
        // i = 0; until i > 4: i = i + 1
        let g = "size 4\nsubgraph f\nmeta kind \"function\"\nnode 2 f_float v=0\nnode 3 f_set a=@2 name=\"i\"\nnode 4 f_get name=\"i\"\nnode 5 f_float v=4\nnode 6 f_gt a=@4 b=@5\nnode 7 f_float v=1\nnode 8 f_add a=@4 b=@7\nnode 9 f_set a=@8 name=\"i\"\nnode 10 f_loop a=@3 b=@6 c=@9\nnode 11 f_get name=\"i\"\nnode 12 f_sequence a=@10 b=@11\nnode 13 f_float v=10\nnode 14 f_div a=@12 b=@13\nnode 1 f_out in=@14\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1\n";
        let t = Graph::parse(g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(4)).unwrap();
        assert!((t["o"].px[0][0] - 0.5).abs() < 1e-5, "{:?}", t["o"].px[0]);
    }

    #[test]
    fn a_long_loop_runs_past_the_usual_limit_and_a_count_caps_a_condition() {
        // v counts the bodies; condition v > limit
        let run = |args: &str, limit: f32| {
            let g = format!(
                "size 4\nsubgraph f\nmeta kind \"function\"\nnode 2 f_float v=0\nnode 3 f_set a=@2 name=\"i\"\nnode 4 f_get name=\"i\"\nnode 5 f_float v={limit}\nnode 6 f_gt a=@4 b=@5\nnode 7 f_float v=1\nnode 8 f_add a=@4 b=@7\nnode 9 f_set a=@8 name=\"i\"\nnode 10 f_loop a=@3 b=@6 c=@9 {args}\nnode 11 f_get name=\"i\"\nnode 12 f_sequence a=@10 b=@11\nnode 13 f_float v=8192\nnode 14 f_div a=@12 b=@13\nnode 1 f_out in=@14\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1\n"
            );
            (crate::graph::Graph::parse(&g).unwrap().evaluate(Some(4)).unwrap()["o"].px[0][0] * 8192.0).round() as u32
        };
        assert_eq!(run("passes=3 long=1", 1000.0), 3);
        assert_eq!(run("passes=16777216 long=1", 5.5), 6);
        assert_eq!(run("passes=8000 long=1", 7000.5), 7001);
        assert_eq!(run("passes=8000", 7000.5), 4096, "ordinary graphs keep their limit");
    }

    #[test]
    fn a_loop_without_a_condition_runs_its_passes() {
        let g = "size 4\nsubgraph f\nmeta kind \"function\"\nnode 2 f_float v=0\nnode 3 f_set a=@2 name=\"i\"\nnode 4 f_get name=\"i\"\nnode 7 f_float v=1\nnode 8 f_add a=@4 b=@7\nnode 9 f_set a=@8 name=\"i\"\nnode 10 f_loop a=@3 c=@9 passes=3\nnode 11 f_get name=\"i\"\nnode 12 f_sequence a=@10 b=@11\nnode 13 f_float v=10\nnode 14 f_div a=@12 b=@13\nnode 1 f_out in=@14\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1\n";
        let t = Graph::parse(g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(4)).unwrap();
        assert!((t["o"].px[0][0] - 0.3).abs() < 1e-5, "{:?}", t["o"].px[0]);
    }
}
