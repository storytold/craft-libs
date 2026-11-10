//! Pattern programs of fx-map nodes as the graph engine's pattern programs (`meta kind "patterns"`, `pp_*` nodes).

use super::functions::{Functions, rand_mode};
use crate::decode::graph_package::Description;
use crate::decode::graph_package::compiled::fxmap::{Children, Count, Record, Seed};
use crate::decode::graph_package::compiled::{Expr, Value};
use std::fmt::Write;

/// A quadrant's parameter from the decoder's slot number. The tag holds one bit per form, so a slot's inline value and
/// its expression belong to neighbouring parameters: slot k's expression is parameter k−1, its inline value parameter k.
/// Measured against the reference app with generated graphs.
fn param_name(slot: u8, expression: bool) -> Option<(&'static str, usize)> {
    // the extra expression after the array picks which input an image pattern draws
    if slot == 8 {
        return Some(("image_index", 1));
    }
    Some(match slot - u8::from(expression) {
        1 => ("luminance", 1),
        2 => ("branch_offset", 2),
        3 => ("offset", 2),
        4 => ("size", 2),
        5 => ("rotation", 1),
        6 => ("variation", 1),
        _ => return None,
    })
}

/// A built-in pattern: the compiled format's code to the graph language's shape name (`pattern="<shape>"` on `pp_quadrant`).
pub(super) fn pattern_shape(code: u8) -> Option<&'static str> {
    Some(match code {
        0 => "square",
        1 => "disc",
        2 => "paraboloid",
        3 => "bell",
        4 => "gaussian",
        5 => "thorn",
        6 => "pyramid",
        7 => "brick",
        8 => "gradation",
        9 => "waves",
        10 => "half_bell",
        11 => "ridged_bell",
        12 => "crescent",
        13 => "capsule",
        14 => "cone",
        15 => "gradation_offset",
        16 => "hemisphere",
        _ => return None,
    })
}

/// Text of the program subgraph body for `root`; `init` are the host node's expressions (size, parameters),
/// which set variables the program reads, run before it.
pub fn program(root: &Record, init: &[(&str, &Expr)], desc: &Description, fns: &mut Functions) -> String {
    let mut b = Builder { text: String::new(), next: 1 };
    let top = b.record(root, desc, fns);
    // the node's own expressions draw as parameters
    fns.rand.mode = rand_mode::PARAMETER;
    let out = b.id();
    let mut line = format!("node {out} pp_out in=@{top}");
    for (key, e) in init {
        line += &format!(" fn.{key}=\"{}\"", fns.function(e, desc, None));
    }
    let _ = writeln!(b.text, "{line}");
    format!("meta kind \"patterns\"\n{}", b.text)
}

struct Builder {
    text: String,
    next: u32,
}

impl Builder {
    fn id(&mut self) -> u32 {
        let i = self.next;
        self.next += 1;
        i
    }

    /// A record's seed, which adds to the state it gets from above.
    fn seed(&mut self, seed: &Option<Seed>, desc: &Description, fns: &mut Functions) -> String {
        match seed {
            Some(Seed::Const(w)) => format!(" seed={w}"),
            Some(Seed::Expr(e)) => format!(" fn.seed=\"{}\"", fns.function(e, desc, None)),
            None => String::new(),
        }
    }

    fn count(&mut self, c: &Count, key: &str, desc: &Description, fns: &mut Functions) -> String {
        match c {
            Count::Const(n) => format!(" {key}={n}"),
            Count::Expr(e) => format!(" fn.{key}=\"{}\"", fns.function(e, desc, None)),
        }
    }

    fn record(&mut self, r: &Record, desc: &Description, fns: &mut Functions) -> u32 {
        match r {
            Record::Iterate { seed, count, first, body } => {
                // with two records the one stored first repeats, and the other runs once before it (measured with variables)
                let (once, repeated) = if first.is_some() { (body, first) } else { (first, body) };
                let f = once.as_ref().map(|c| self.record(c, desc, fns));
                let bd = repeated.as_ref().map(|c| self.record(c, desc, fns));
                let me = self.id();
                let mut line = format!("node {me} pp_iterate");
                line += &self.seed(seed, desc, fns);
                fns.rand.mode = rand_mode::FX_ITERATE_COUNT;
                line += &self.count(count, "count", desc, fns);
                if let Some(f) = f {
                    line += &format!(" first=@{f}");
                }
                if let Some(bd) = bd {
                    line += &format!(" body=@{bd}");
                }
                let _ = writeln!(self.text, "{line}");
                me
            }
            Record::Switch { seed, condition, branches } => {
                let ids: Vec<Option<u32>> = branches.iter().map(|c| c.as_ref().map(|c| self.record(c, desc, fns))).collect();
                let me = self.id();
                let mut line = format!("node {me} pp_switch");
                line += &self.seed(seed, desc, fns);
                fns.rand.mode = rand_mode::FX_SWITCH_CONDITION;
                line += &self.count(condition, "cond", desc, fns);
                for (slot, id) in ["a", "b"].iter().zip(ids) {
                    if let Some(id) = id {
                        line += &format!(" {slot}=@{id}");
                    }
                }
                let _ = writeln!(self.text, "{line}");
                me
            }
            Record::Quadrant { seed, pattern, draws, image, image_word, flag1, params, children, .. } => {
                let kids: Vec<(usize, u32)> = match children {
                    Children::None => Vec::new(),
                    // one child drawn in all four quadrants
                    Children::Shared(c) => {
                        let id = self.record(c, desc, fns);
                        (0..4).map(|q| (q, id)).collect()
                    }
                    Children::Four(cs) => cs.iter().enumerate().filter_map(|(q, c)| c.as_ref().map(|c| (q, self.record(c, desc, fns)))).collect(),
                };
                let me = self.id();
                let mut line = format!("node {me} pp_quadrant");
                // an image quadrant's `pattern` is an index into the node's inputs, the others' a built-in shape
                match pattern_shape(*pattern) {
                    _ if *image => {
                        let _ = write!(line, " image_index={pattern}");
                    }
                    Some(shape) => {
                        let _ = write!(line, " pattern=\"{shape}\"");
                    }
                    None => {
                        let _ = write!(line, " pattern={pattern}");
                    }
                }
                let _ = write!(line, " draws={}", u8::from(*draws));
                line += &self.seed(seed, desc, fns);
                if *image {
                    line += " image=1";
                    // the image word's bit 2 reads the image itself rather than a pyramid level, bit 3 also without filtering
                    let filter = image_word.map_or(0, |w| if w & 8 != 0 { 2 } else { u32::from(w & 4 != 0) });
                    if filter != 0 {
                        let _ = write!(line, " image_filter={filter}");
                    }
                }
                if *flag1 {
                    line += " max=1";
                }
                for (slot, v) in params {
                    let Some((name, lanes)) = param_name(*slot, matches!(v, Value::Expr(_))) else { continue };
                    // the generator of each kind of expression
                    fns.rand.mode = match name {
                        "image_index" => rand_mode::FX_IMAGE_INDEX,
                        "branch_offset" => rand_mode::FX_BRANCH_OFFSET,
                        "offset" => rand_mode::FX_PATTERN_OFFSET,
                        "size" => rand_mode::FX_SIZE,
                        "rotation" => rand_mode::FX_ROTATION,
                        "variation" => rand_mode::FX_VARIATION,
                        _ => rand_mode::FX_LUMINANCE,
                    };
                    match v {
                        Value::Const(w) => {
                            let f: Vec<f32> = w.iter().map(|x| f32::from_bits(*x)).collect();
                            if lanes == 1 {
                                line += &format!(" {name}={}", f.first().copied().unwrap_or(0.0));
                            } else {
                                line += &format!(" {name}_x={} {name}_y={}", f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0));
                            }
                        }
                        Value::Expr(e) => line += &format!(" fn.{name}=\"{}\"", fns.function(e, desc, None)),
                        _ => {}
                    }
                }
                for (q, id) in kids {
                    line += &format!(" q{q}=@{id}");
                }
                let _ = writeln!(self.text, "{line}");
                me
            }
        }
    }
}
