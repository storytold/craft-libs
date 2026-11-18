//! Animatable properties and their keyframes.
//!
//! A Lottie keyframe holds the start value `s` of a segment and the segment's normalised cubic
//! Bezier ease: `o` (out tangent of this key) and `i` (in tangent of the next key), each with `x`
//! (time fraction) and `y` (value progress fraction), per dimension. After Effects' temporal ease
//! is speed (units per second) and influence (fraction of the segment): for a segment of duration
//! `d` and value change `Δv`,
//!
//! ```text
//! o.x = influence_out          o.y = speed_out · influence_out · d / Δv
//! i.x = 1 − influence_in       i.y = 1 − speed_in · influence_in · d / Δv
//! ```
//!
//! Spatial properties ease along the motion path (`Δv` is the arc length, one dimension) and carry
//! `to`/`ti` spatial tangents. Hold keys set `h: 1`.

use serde_json::json;

use crate::{Json, round6};

/// Whether a property value is keyframed: `a` is 1, or `k` is a list of keys (files without `a`).
pub fn is_animated(j: &Json) -> bool {
    if j.get("a").and_then(Json::as_i64) == Some(1) {
        return true;
    }
    j.get("k").and_then(Json::as_array).and_then(|a| a.first()).is_some_and(|f| f.get("t").is_some())
}

/// Component `d` of an ease tangent field (`x` or `y` of an `o` or `i`), which may be a number
/// or a list; a list shorter than `d` gives its first.
pub fn tangent(j: &Json, field: &str, d: usize) -> Option<f64> {
    match j.get(field)? {
        Json::Number(n) => n.as_f64(),
        Json::Array(a) => a.get(d).or(a.first()).and_then(Json::as_f64),
        _ => None,
    }
}

/// One keyframe as the file has it. Values stay JSON: their type depends on the property.
#[derive(Debug, Clone, PartialEq)]
pub struct Key<'a> {
    /// `t`: the frame; 0 when missing.
    pub t: f64,
    /// The value at the key: its `s`, or the end value `e` of the key before (files before 5.0
    /// end a segment with `e` and give the last key no value).
    pub s: Option<&'a Json>,
    /// `h` is 1: the value holds until the next key.
    pub hold: bool,
    /// `o`: the out tangent of the segment that starts here.
    pub o: Option<&'a Json>,
    /// `i`: the in tangent of the segment that starts here (it arrives at the next key).
    pub i: Option<&'a Json>,
    /// `to`: the spatial out tangent, zeros when missing.
    pub to: [f64; 3],
    /// `ti`: the spatial in tangent at the next key, zeros when missing.
    pub ti: [f64; 3],
}

/// The value of a property.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue<'a> {
    /// No `k`, or a keyframed property whose `k` is not a list.
    None,
    /// One value.
    Static(&'a Json),
    /// Keyframes in file order.
    Keys(Vec<Key<'a>>),
}

/// An animatable property.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop<'a> {
    /// `x`: the expression, when it is not blank.
    pub expression: Option<&'a str>,
    pub value: PropValue<'a>,
}

fn vec3_of(j: Option<&Json>) -> [f64; 3] {
    let a = j.and_then(Json::as_array);
    let g = |i: usize| a.and_then(|a| a.get(i)).and_then(Json::as_f64).unwrap_or(0.0);
    [g(0), g(1), g(2)]
}

/// Reads an animatable property. A value that is not an object is the static value itself (some
/// files write `"o": 100`).
pub fn read_prop(j: &Json) -> Prop<'_> {
    if !j.is_object() {
        return Prop { expression: None, value: PropValue::Static(j) };
    }
    let expression = j.get("x").and_then(Json::as_str).filter(|x| !x.trim().is_empty());
    let Some(k) = j.get("k") else { return Prop { expression, value: PropValue::None } };
    if !is_animated(j) {
        return Prop { expression, value: PropValue::Static(k) };
    }
    let Some(list) = k.as_array() else { return Prop { expression, value: PropValue::None } };
    let mut keys = Vec::with_capacity(list.len());
    let mut prev_end: Option<&Json> = None;
    for kj in list {
        keys.push(Key {
            t: kj.get("t").and_then(Json::as_f64).unwrap_or(0.0),
            s: kj.get("s").or(prev_end),
            hold: kj.get("h").and_then(Json::as_i64) == Some(1),
            o: kj.get("o"),
            i: kj.get("i"),
            to: vec3_of(kj.get("to")),
            ti: vec3_of(kj.get("ti")),
        });
        prev_end = kj.get("e");
    }
    Prop { expression, value: PropValue::Keys(keys) }
}

/// One side of a keyframe's temporal ease, as After Effects has it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ease {
    /// Units per second.
    pub speed: f64,
    /// Fraction of the segment, 0..=1.
    pub influence: f64,
}

/// The ease of one segment in one dimension: the two inner control points of a cubic Bezier from
/// (0, 0) to (1, 1), time across and value progress up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tangents {
    pub ox: f64,
    pub oy: f64,
    pub ix: f64,
    pub iy: f64,
}

impl Tangents {
    /// Reads dimension `d` of a key's `o` and `i`. A missing `x` is a third of the way (1/3 and
    /// 2/3), `x` is held to 0..=1, and a missing `y` equals its `x` (linear).
    pub fn read(o: &Json, i: &Json, d: usize) -> Self {
        let ox = tangent(o, "x", d).unwrap_or(1.0 / 3.0).clamp(0.0, 1.0);
        let oy = tangent(o, "y", d).unwrap_or(ox);
        let ix = tangent(i, "x", d).unwrap_or(2.0 / 3.0).clamp(0.0, 1.0);
        let iy = tangent(i, "y", d).unwrap_or(ix);
        Self { ox, oy, ix, iy }
    }

    /// Whether both control points are on the diagonal: constant speed.
    pub fn is_linear(&self) -> bool {
        (self.ox - self.oy).abs() <= 1e-4 && (self.ix - self.iy).abs() <= 1e-4
    }

    /// The speed and influence of the segment's two sides (out of its first key, into its
    /// second), for a value change `dv` over `dur` seconds. An influence is at least 0.001.
    pub fn to_speed(&self, dv: f64, dur: f64) -> (Ease, Ease) {
        let inf_o = self.ox.max(0.001);
        let inf_i = (1.0 - self.ix).max(0.001);
        (Ease { speed: self.oy * dv / (inf_o * dur), influence: inf_o }, Ease { speed: (1.0 - self.iy) * dv / (inf_i * dur), influence: inf_i })
    }

    /// The tangents of a segment from the ease out of its first key and into its second, for a
    /// value change `dv` over `dur` seconds, rounded to six decimals. Influences are held to
    /// 0..=1 and kept as they are even when the handles cross in time. Without a value change
    /// the progress is 0 and 1.
    pub fn from_speed(out: Ease, into: Ease, dv: f64, dur: f64) -> Self {
        let (i0, i1) = (out.influence.clamp(0.0, 1.0), into.influence.clamp(0.0, 1.0));
        let (y0, y1) = if dv.abs() > 1e-12 { (out.speed * i0 * dur / dv, 1.0 - into.speed * i1 * dur / dv) } else { (0.0, 1.0) };
        Self { ox: round6(i0), oy: round6(y0), ix: round6(1.0 - i1), iy: round6(y1) }
    }
}

/// One keyframe to write.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyOut {
    /// The frame.
    pub t: f64,
    /// The value at the key.
    pub s: Json,
    /// Hold until the next key (`h: 1`); `ease` and `spatial` are then not written.
    pub hold: bool,
    /// The ease of the segment that starts here, one entry per dimension.
    pub ease: Vec<Tangents>,
    /// `(to, ti)`: the spatial tangents out of this key and into the next.
    pub spatial: Option<(Vec<f64>, Vec<f64>)>,
}

/// Keys as the `k` list of a keyframed property. A value that is not a list is wrapped in one
/// (`s: [5]`). The last key is written without tangents: no segment starts there.
pub fn write_keys(keys: &[KeyOut]) -> Vec<Json> {
    let last = keys.len().saturating_sub(1);
    keys.iter()
        .enumerate()
        .map(|(n, k)| {
            let s = if k.s.is_array() { k.s.clone() } else { json!([k.s]) };
            let mut o = json!({"t": round6(k.t), "s": s});
            if k.hold {
                o["h"] = json!(1);
            } else if n < last {
                let col = |f: fn(&Tangents) -> f64| k.ease.iter().map(f).collect::<Vec<_>>();
                o["o"] = json!({"x": col(|t| t.ox), "y": col(|t| t.oy)});
                o["i"] = json!({"x": col(|t| t.ix), "y": col(|t| t.iy)});
                if let Some((to, ti)) = &k.spatial {
                    o["to"] = json!(to.iter().map(|v| round6(*v)).collect::<Vec<_>>());
                    o["ti"] = json!(ti.iter().map(|v| round6(*v)).collect::<Vec<_>>());
                }
            }
            o
        })
        .collect()
}

/// An animatable property: `{a: 0, k: value}` without keys, `{a: 1, k: [keys]}` with, and `x`
/// for an expression.
pub fn write_prop(value: Json, keys: &[KeyOut], expression: Option<&str>) -> Json {
    let mut out = if keys.is_empty() { json!({"a": 0, "k": value}) } else { json!({"a": 1, "k": write_keys(keys)}) };
    if let Some(x) = expression {
        out["x"] = json!(x);
    }
    out
}
