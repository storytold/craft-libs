// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mini expression language for the Pixel Processor / function nodes.
use crate::math;
use crate::noise;
use crate::tex::{Tex, lum};

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
    Lt,
    Gt,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Func {
    Sin,
    Cos,
    Abs,
    Sqrt,
    Min,
    Max,
    Pow,
    Floor,
    Fract,
    Clamp,
    Mix,
    Step,
    Smoothstep,
    Noise,
    Rand,
    Tex,
    Atan2,
    Exp,
    Log,
}

const FUNCS: &[(&str, Func, usize)] = &[
    ("sin", Func::Sin, 1),
    ("cos", Func::Cos, 1),
    ("abs", Func::Abs, 1),
    ("sqrt", Func::Sqrt, 1),
    ("min", Func::Min, 2),
    ("max", Func::Max, 2),
    ("pow", Func::Pow, 2),
    ("floor", Func::Floor, 1),
    ("fract", Func::Fract, 1),
    ("clamp", Func::Clamp, 3),
    ("mix", Func::Mix, 3),
    ("step", Func::Step, 2),
    ("smoothstep", Func::Smoothstep, 3),
    ("noise", Func::Noise, 3),
    ("rand", Func::Rand, 2),
    ("tex", Func::Tex, 2),
    ("atan2", Func::Atan2, 2),
    ("exp", Func::Exp, 1),
    ("log", Func::Log, 1),
];

/// Variables: u v (0..1), x y (pixel), w h (size), r g b (input colour), l (input luminance)
pub const VARS: [&str; 10] = ["u", "v", "x", "y", "w", "h", "r", "g", "b", "l"];

#[derive(Debug, Clone)]
pub enum Ex {
    Num(f32),
    Var(usize),
    Neg(Box<Ex>),
    Bin(Op, Box<Ex>, Box<Ex>),
    Call(Func, Vec<Ex>),
}

pub struct Env<'a> {
    pub vars: [f32; 10],
    pub tex: Option<&'a Tex>,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32),
    Id(String),
    Sym(char),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let at = |i: usize| cs.get(i).copied().unwrap_or('\0');
    while let Some(c) = cs.get(i).copied() {
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let st = i;
            while i < cs.len() && (at(i).is_ascii_digit() || at(i) == '.') {
                i += 1;
            }
            let t: String = cs.iter().take(i).skip(st).collect();
            out.push(Tok::Num(t.parse::<f32>().map_err(|_| format!("bad number '{t}'"))?));
        } else if c.is_alphabetic() || c == '_' {
            let st = i;
            while i < cs.len() && (at(i).is_alphanumeric() || at(i) == '_') {
                i += 1;
            }
            out.push(Tok::Id(cs.iter().take(i).skip(st).collect()));
        } else if "+-*/^%<>(),".contains(c) {
            out.push(Tok::Sym(c));
            i += 1;
        } else {
            return Err(format!("unexpected character '{c}'"));
        }
    }
    Ok(out)
}

struct P {
    t: Vec<Tok>,
    i: usize,
    /// how many brackets, signs and powers the parser is inside now (each is a few calls deeper)
    nest: usize,
}

/// The most tokens an expression may have: evaluating it recurses along its tree, which a chain of terms makes as deep
/// as it is long.
const MAX_TOKENS: usize = 4096;
/// The most brackets, signs and powers one inside the other.
const MAX_NESTING: usize = 256;

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn sym(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Sym(c)) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn cmp(&mut self) -> Result<Ex, String> {
        let mut l = self.add()?;
        loop {
            let op = if self.sym('<') {
                Op::Lt
            } else if self.sym('>') {
                Op::Gt
            } else {
                return Ok(l);
            };
            let r = self.add()?;
            l = Ex::Bin(op, Box::new(l), Box::new(r));
        }
    }
    fn add(&mut self) -> Result<Ex, String> {
        let mut l = self.mul()?;
        loop {
            let op = if self.sym('+') {
                Op::Add
            } else if self.sym('-') {
                Op::Sub
            } else {
                return Ok(l);
            };
            let r = self.mul()?;
            l = Ex::Bin(op, Box::new(l), Box::new(r));
        }
    }
    fn mul(&mut self) -> Result<Ex, String> {
        let mut l = self.unary()?;
        loop {
            let op = if self.sym('*') {
                Op::Mul
            } else if self.sym('/') {
                Op::Div
            } else if self.sym('%') {
                Op::Mod
            } else {
                return Ok(l);
            };
            let r = self.unary()?;
            l = Ex::Bin(op, Box::new(l), Box::new(r));
        }
    }
    fn unary(&mut self) -> Result<Ex, String> {
        if self.nest >= MAX_NESTING {
            return Err(format!("expression nested more than {MAX_NESTING} deep"));
        }
        self.nest += 1;
        let r = self.unary_inner();
        self.nest -= 1;
        r
    }
    fn unary_inner(&mut self) -> Result<Ex, String> {
        if self.sym('-') {
            return Ok(Ex::Neg(Box::new(self.unary()?)));
        }
        let b = self.primary()?;
        if self.sym('^') {
            let e = self.unary()?;
            return Ok(Ex::Bin(Op::Pow, Box::new(b), Box::new(e)));
        }
        Ok(b)
    }
    fn primary(&mut self) -> Result<Ex, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Ex::Num(n))
            }
            Some(Tok::Sym('(')) => {
                self.i += 1;
                let e = self.cmp()?;
                if !self.sym(')') {
                    return Err("expected ')'".into());
                }
                Ok(e)
            }
            Some(Tok::Id(name)) => {
                self.i += 1;
                if self.sym('(') {
                    let (f, arity) =
                        FUNCS.iter().find(|(n, _, _)| *n == name).map(|(_, f, a)| (*f, *a)).ok_or_else(|| format!("unknown function '{name}'"))?;
                    let mut args = Vec::new();
                    if !self.sym(')') {
                        loop {
                            args.push(self.cmp()?);
                            if self.sym(')') {
                                break;
                            }
                            if !self.sym(',') {
                                return Err("expected ',' or ')'".into());
                            }
                        }
                    }
                    if args.len() != arity {
                        return Err(format!("{name}() takes {arity} argument(s), got {}", args.len()));
                    }
                    Ok(Ex::Call(f, args))
                } else if name == "pi" {
                    Ok(Ex::Num(std::f32::consts::PI))
                } else {
                    VARS.iter().position(|v| *v == name).map(Ex::Var).ok_or_else(|| format!("unknown variable '{name}'"))
                }
            }
            other => Err(format!("unexpected token {:?}", other)),
        }
    }
}

pub fn parse(src: &str) -> Result<Ex, String> {
    let t = tokenize(src)?;
    if t.len() > MAX_TOKENS {
        return Err(format!("expression longer than {MAX_TOKENS} tokens"));
    }
    let mut p = P { t, i: 0, nest: 0 };
    let e = p.cmp()?;
    if p.i != p.t.len() {
        return Err("trailing tokens in expression".into());
    }
    Ok(e)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let d = b - a;
    let t = if d.abs() < 1e-9 { 0.0 } else { ((x - a) / d).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

pub fn eval(e: &Ex, env: &Env) -> f32 {
    let r = match e {
        Ex::Num(n) => *n,
        Ex::Var(i) => env.vars.get(*i).copied().unwrap_or(0.0),
        Ex::Neg(a) => -eval(a, env),
        Ex::Bin(op, a, b) => {
            let (x, y) = (eval(a, env), eval(b, env));
            match op {
                Op::Add => x + y,
                Op::Sub => x - y,
                Op::Mul => x * y,
                Op::Div => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x / y
                    }
                }
                Op::Pow => math::powf(x, y),
                Op::Mod => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x.rem_euclid(y)
                    }
                }
                Op::Lt => (x < y) as i32 as f32,
                Op::Gt => (x > y) as i32 as f32,
            }
        }
        Ex::Call(f, a) => {
            // (the parser checked the number of arguments; none takes more than three)
            let mut v = [0.0f32; 3];
            for (v, x) in v.iter_mut().zip(a) {
                *v = eval(x, env);
            }
            match f {
                Func::Sin => math::sin(v[0]),
                Func::Cos => math::cos(v[0]),
                Func::Abs => v[0].abs(),
                Func::Sqrt => v[0].max(0.0).sqrt(),
                Func::Min => v[0].min(v[1]),
                Func::Max => v[0].max(v[1]),
                Func::Pow => math::powf(v[0], v[1]),
                Func::Floor => v[0].floor(),
                Func::Fract => v[0] - v[0].floor(),
                Func::Clamp => v[0].max(v[1].min(v[2])).min(v[1].max(v[2])),
                Func::Mix => v[0] + (v[1] - v[0]) * v[2],
                Func::Step => (v[1] >= v[0]) as i32 as f32,
                Func::Smoothstep => smooth(v[0], v[1], v[2]),
                Func::Noise => {
                    let p = v[2].round().clamp(1.0, 4096.0) as i32;
                    noise::perlin(v[0], v[1], p, p, 0)
                }
                Func::Rand => noise::rnd(v[0].floor() as i32, v[1].floor() as i32, 1),
                Func::Tex => env.tex.map(|t| lum(t.sample(v[0], v[1]))).unwrap_or(0.0),
                Func::Atan2 => math::atan2(v[0], v[1]),
                Func::Exp => math::exp(v[0].min(80.0)),
                Func::Log => math::ln(v[0].max(1e-12)),
            }
        }
    };
    if r.is_finite() { r } else { 0.0 }
}
