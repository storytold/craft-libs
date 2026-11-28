//! Lower the position-dependent part of a pure single-precision pixel program to compute shader expressions.

use std::fmt::Write;

use super::code::{Code, FLOOR_ENV, NONE, indexed_lcg_draw};
use super::{B, FnEnv, Op, Prog, U, Value};
use crate::accelerator::{GreyInput, PixelImageInput};
use crate::graph::Pin;
use crate::tex::{GreyIn, ImgRef};

pub struct PixelShader {
    pub source: String,
    /// Position-independent registers, evaluated by the same VM as the CPU path.
    pub uniforms: Vec<[f32; 4]>,
    /// Indices into the function's bound images, in texture binding order.
    pub images: Vec<usize>,
    pub grey: bool,
}

impl Prog {
    pub fn pixel_shader(&self, env: &FnEnv<'_>) -> Option<PixelShader> {
        let images: Vec<_> = env.images.iter().map(|&(name, image)| (name, SampleImage::Host(image))).collect();
        self.emit_pixel_shader(env, &images)
    }

    pub(crate) fn packed_pixel_shader(&self, env: &FnEnv<'_>, images: &[(&str, GreyInput<'_>)]) -> Option<PixelShader> {
        if !self.can_sample_resident_images() {
            return None;
        }
        let images: Vec<_> = images.iter().map(|&(name, image)| (name, SampleImage::Packed(image))).collect();
        self.emit_pixel_shader(env, &images).filter(|shader| shader.grey)
    }

    pub(crate) fn textured_pixel_shader(&self, env: &FnEnv<'_>, images: &[(&str, PixelImageInput<'_>)]) -> Option<PixelShader> {
        if !self.can_sample_resident_images() {
            return None;
        }
        let images = images
            .iter()
            .map(|&(name, image)| {
                let sampled = match image {
                    PixelImageInput::Host(image) => SampleImage::Host(image),
                    PixelImageInput::Float(_) => SampleImage::Float,
                };
                (name, sampled)
            })
            .collect::<Vec<_>>();
        self.emit_pixel_shader(env, &images).filter(|shader| shader.grey)
    }

    fn can_sample_resident_images(&self) -> bool {
        let code = self.code();
        // These values are evaluated on the CPU while binding a program. They cannot read GPU images.
        !self.uses_stats
            && !code.insts.iter().zip(&code.floor).any(|(inst, &floor)| floor <= FLOOR_ENV && matches!(inst.op, Op::SampleColor | Op::SampleGray))
    }

    fn emit_pixel_shader(&self, env: &FnEnv<'_>, inputs: &[(&str, SampleImage<'_>)]) -> Option<PixelShader> {
        if !self.single {
            return None;
        }
        let code = self.code();
        let count = code.insts.len();
        let mut frame = self.frame();
        self.bind(env, &mut frame);
        let root_index = code.out as usize;
        let root = code.insts.get(root_index)?;
        // Some compiled functions initialise locals before the expression that varies per pixel.
        // Run that initialiser once in the ordinary VM and bind subsequent reads as uniforms.
        let (out, initialised) = if root.op == Op::Seq && *code.floor.get(root_index)? > FLOOR_ENV {
            let [initialiser, out, ..] = root.ins;
            if !uniform_initialiser(code, initialiser) {
                return None;
            }
            self.uniform_value(env, &mut frame, initialiser as usize)?;
            (out, true)
        } else {
            (code.out, false)
        };
        let uniform_register = |index: usize| {
            code.floor.get(index).is_some_and(|&floor| floor <= FLOOR_ENV)
                || (initialised && code.insts.get(index).is_some_and(|inst| inst.op == Op::Get))
        };
        let mut need = vec![false; count];
        let mut todo = vec![out];
        while let Some(index) = todo.pop() {
            let index = index as usize;
            let visit = need.get_mut(index)?;
            if std::mem::replace(visit, true) || uniform_register(index) {
                continue;
            }
            todo.extend(code.insts.get(index)?.ins.into_iter().filter(|&i| i != NONE));
        }

        let mut shader = PixelShader { source: String::new(), uniforms: Vec::new(), images: Vec::new(), grey: false };
        let mut bindings = Vec::new();
        let mut types = vec![None; count];
        let result = self.type_of(out as usize, &mut types).ok()?;
        let mut body = String::new();
        for (index, &used) in need.iter().enumerate() {
            if !used {
                continue;
            }
            let pin = self.type_of(index, &mut types).ok()?;
            let expression = if uniform_register(index) {
                let value = self.uniform_value(env, &mut frame, index)?;
                if code.insts.get(index)?.op == Op::Get && value.pin() != pin {
                    return None;
                }
                let slot = shader.uniforms.len();
                shader.uniforms.push(uniform(value)?);
                let read = format!("parameters[{}]", slot + 1);
                if pin == Pin::Bool { format!("{read}.x != 0.0") } else { format!("{read}{}", suffix(pin.lanes())?) }
            } else {
                if matches!(pin, Pin::Int | Pin::Int2 | Pin::Int3 | Pin::Int4 | Pin::Text) {
                    return None;
                }
                expression(code, index, &types, inputs, &mut shader.images, &mut bindings)?
            };
            writeln!(body, "    let r{index} = {expression};").ok()?;
        }
        for (slot, &image) in shader.images.iter().enumerate() {
            let (_, image) = *inputs.get(image)?;
            if let SampleImage::Packed(image) = image {
                let (w, h, div, alpha) = image.info();
                let param = shader.uniforms.len() + 1;
                shader.uniforms.push([w as f32, h as f32, div, alpha]);
                writeln!(shader.source, "@group(0) @binding({}) var<storage, read> input_{slot}: array<u32>;", slot + 2).ok()?;
                writeln!(shader.source, "{}", packed_sampler(slot, param)).ok()?;
                continue;
            }
            let (sample_type, sample) = match image {
                SampleImage::Host(ImgRef::Grey(GreyIn::Q(grey))) => {
                    let div = shader.uniforms.len() + 1;
                    shader.uniforms.push([grey.div, 0.0, 0.0, 0.0]);
                    let alpha = grey.alpha;
                    ("u32", format!("vec4f(vec3f(tc_sample_steps(input_{slot}, uv, mode, parameters[{div}].x)), {alpha:.1})"))
                }
                SampleImage::Host(ImgRef::Grey(grey)) => ("f32", format!("vec4f(vec3f(tc_sample(input_{slot}, uv, mode).x), {:.1})", grey.alpha())),
                SampleImage::Host(ImgRef::Rgba(_)) => ("f32", format!("tc_sample(input_{slot}, uv, mode)")),
                SampleImage::Float => ("f32", format!("vec4f(vec3f(tc_sample(input_{slot}, uv, mode).x), 1.0)")),
                SampleImage::Packed(_) => return None,
            };
            writeln!(shader.source, "@group(0) @binding({}) var input_{slot}: texture_2d<{sample_type}>;", slot + 2).ok()?;
            writeln!(shader.source, "fn sample_{slot}(uv: vec2f, mode: u32) -> vec4f {{ return {sample}; }}").ok()?;
        }
        let last = format!("r{out}");
        let (ty, expression) = match result {
            Pin::Bool => ("f32", format!("select(0.0, 1.0, {last})")),
            Pin::Float | Pin::Int => ("f32", last),
            Pin::Float2 | Pin::Int2 => ("vec4f", format!("vec4f({last}, 0.0, 1.0)")),
            Pin::Float3 | Pin::Int3 => ("vec4f", format!("vec4f({last}, 1.0)")),
            Pin::Float4 | Pin::Int4 => ("vec4f", last),
            _ => return None,
        };
        shader.grey = ty == "f32";
        writeln!(shader.source, "fn evaluate_pixel(pos: vec2f) -> {ty} {{\n{body}    return {expression};\n}}").ok()?;
        Some(shader)
    }
}

/// The whole prefix has to be independent of the current pixel and leave only local values for the shader body.
/// The shader expression emitter separately rejects writes and other effects in that body.
fn uniform_initialiser(code: &Code, root: u32) -> bool {
    let mut seen = vec![false; code.insts.len()];
    let mut todo = vec![root];
    while let Some(index) = todo.pop() {
        let Some(visited) = seen.get_mut(index as usize) else { return false };
        if std::mem::replace(visited, true) {
            continue;
        }
        let Some(inst) = code.insts.get(index as usize) else { return false };
        if matches!(inst.op, Op::Pos | Op::SampleGray | Op::SampleColor | Op::Stat(_) | Op::Loop | Op::Call) {
            return false;
        }
        if inst.op == Op::Random && !code.lcg_draws.get(inst.x as usize).copied().flatten().is_some_and(|(site, _)| indexed_lcg_draw(site)) {
            return false;
        }
        todo.extend(inst.ins.into_iter().filter(|&i| i != NONE));
    }
    true
}

#[derive(Clone, Copy)]
enum SampleImage<'a> {
    Host(ImgRef<'a>),
    Packed(GreyInput<'a>),
    Float,
}

fn packed_sampler(slot: usize, param: usize) -> String {
    format!(
        "fn load_{slot}(p: vec2i, width: i32, div: f32) -> f32 {{
    let i = u32(p.y * width + p.x);
    return f32((input_{slot}[i / 2u] >> ((i & 1u) * 16u)) & 0xffffu) / div;
}}
fn sample_{slot}(uv: vec2f, mode: u32) -> vec4f {{
    let p = parameters[{param}];
    let n = vec2i(p.xy);
    let position = tc_sample_position(uv);
    if ((mode & 1u) == 0u) {{
        return vec4f(vec3f(load_{slot}(tc_nearest(position, n, mode), n.x, p.z)), p.w);
    }}
    let sample = tc_bilinear(position, n, mode);
    let p00 = load_{slot}(sample.a, n.x, p.z);
    let p10 = load_{slot}(vec2i(sample.b.x, sample.a.y), n.x, p.z);
    let p01 = load_{slot}(vec2i(sample.a.x, sample.b.y), n.x, p.z);
    let p11 = load_{slot}(sample.b, n.x, p.z);
    let top = p00 + (p10 - p00) * sample.t.x;
    let bottom = p01 + (p11 - p01) * sample.t.x;
    return vec4f(vec3f(top + (bottom - top) * sample.t.y), p.w);
}}"
    )
}

fn uniform(value: Value) -> Option<[f32; 4]> {
    match value {
        Value::Bool(value) => Some([f32::from(value), 0.0, 0.0, 0.0]),
        Value::Num(value) => {
            let floats = value.v.map(|v| v as f32);
            floats.iter().zip(value.v).all(|(&f, v)| f.is_finite() && (!value.int || f64::from(f) == v)).then_some(floats)
        }
        Value::Str(_) => None,
    }
}

fn suffix(lanes: usize) -> Option<&'static str> {
    Some(match lanes {
        1 => ".x",
        2 => ".xy",
        3 => ".xyz",
        4 => "",
        _ => return None,
    })
}

fn sample_binding(
    sample: u32,
    input_name: &str,
    inputs: &[(&str, SampleImage<'_>)],
    images: &mut Vec<usize>,
    bindings: &mut Vec<(u32, Option<usize>)>,
) -> Option<usize> {
    if let Some((_, binding)) = bindings.iter().find(|(i, _)| *i == sample) {
        *binding
    } else {
        let binding = inputs.iter().position(|(name, _)| *name == input_name).map(|image| {
            if let Some(slot) = images.iter().position(|&source| source == image) {
                slot
            } else {
                let slot = images.len();
                images.push(image);
                slot
            }
        });
        bindings.push((sample, binding));
        binding
    }
}

fn swizzle_expression(v: &str, input: usize, components: &[u8; 4], n: u8) -> Option<String> {
    let parts = components
        .iter()
        .take(n as usize)
        .map(|&component| match (input, component) {
            (1, 0) => v.to_owned(),
            (l, c) if usize::from(c) < l => format!("{v}[{c}u]"),
            _ => "0.0".to_owned(),
        })
        .collect::<Vec<_>>();
    if n == 1 { parts.into_iter().next() } else { Some(format!("vec{n}f({})", parts.join(", "))) }
}

fn float_expression(src: u32, src_pin: Pin, lanes: usize) -> String {
    let mut expr = format!("r{src}");
    if src_pin == Pin::Bool {
        expr = format!("select(0.0, 1.0, {expr})");
    }
    if src_pin.lanes() == 1 && lanes > 1 {
        expr = format!("vec{lanes}f({expr})");
    }
    expr
}

fn sample_expression(binding: Option<usize>, filter: u32, grey: bool, coordinates: impl FnOnce() -> Option<String>) -> Option<String> {
    let read = match binding {
        Some(binding) => format!("sample_{binding}({}, {filter}u)", coordinates()?),
        None => "vec4f(0.0)".into(),
    };
    Some(if grey { format!("tc_lum({read})") } else { read })
}

/// A `vecNf(..)` of the scalar parts of a vector node (`defaults` stand in for the parts that are not linked). A linked
/// vector fills several lanes on the CPU, which this cannot say, so it is left to the CPU (`None`).
fn vec_expression(
    n: u8,
    defaults: &[f64; 4],
    operand: &dyn Fn(usize) -> Option<u32>,
    pin: &dyn Fn(u32) -> Option<Pin>,
    float: &dyn Fn(usize, usize) -> Option<String>,
) -> Option<String> {
    if has_vector_part(operand, pin) {
        return None;
    }
    let values = defaults
        .iter()
        .take(n as usize)
        .enumerate()
        .map(|(i, &d)| if operand(i).is_none() { Some(format!("bitcast<f32>({}u)", (d as f32).to_bits())) } else { float(i, 1) })
        .collect::<Option<Vec<_>>>()?;
    Some(format!("vec{n}f({})", values.join(", ")))
}

/// WGSL for "`value` is not zero": a vector is true when one of its lanes is.
fn non_zero(value: &str, lanes: usize) -> String {
    if lanes > 1 { format!("any({value} != vec{lanes}f(0.0))") } else { format!("{value} != 0.0") }
}

/// Whether any of an instruction's four operands is a vector (more than one lane).
fn has_vector_part(operand: &dyn Fn(usize) -> Option<u32>, pin: &dyn Fn(u32) -> Option<Pin>) -> bool {
    (0..4).any(|slot| operand(slot).and_then(pin).is_some_and(|p| p.lanes() > 1))
}

fn expression(
    code: &Code,
    index: usize,
    pins: &[Option<Pin>],
    inputs: &[(&str, SampleImage<'_>)],
    images: &mut Vec<usize>,
    bindings: &mut Vec<(u32, Option<usize>)>,
) -> Option<String> {
    let instruction = code.insts.get(index)?;
    let operand = |slot: usize| instruction.ins.get(slot).copied().filter(|&i| i != NONE);
    let pin = |id: u32| pins.get(id as usize).copied().flatten();
    let value = |slot: usize| Some(format!("r{}", operand(slot)?));
    let float = |slot: usize, lanes: usize| {
        let src = operand(slot)?;
        Some(float_expression(src, pin(src)?, lanes))
    };
    let lanes = pins.get(index)?.as_ref()?.lanes();
    let input_lanes = |slot| operand(slot).and_then(pin).map_or(1, Pin::lanes);
    Some(match instruction.op {
        Op::Pos => "pos".into(),
        Op::Un(U::Not) => format!("!{}", value(0)?),
        Op::Un(op) => unary(op, &float(0, input_lanes(0))?, input_lanes(0))?,
        Op::Bin(B::And) => format!("{} && {}", value(0)?, value(1)?),
        Op::Bin(B::Or) => format!("{} || {}", value(0)?, value(1)?),
        Op::Bin(B::Eq | B::Ne | B::Xor) if operand(0).and_then(pin) == Some(Pin::Bool) && operand(1).and_then(pin) == Some(Pin::Bool) => {
            format!("{} {} {}", value(0)?, if instruction.op == Op::Bin(B::Eq) { "==" } else { "!=" }, value(1)?)
        }
        Op::Bin(op) => {
            let n = input_lanes(0).max(input_lanes(1));
            binary(op, &float(0, n)?, &float(1, n)?, n)?
        }
        Op::Vec(n, false) => vec_expression(n, code.vec_defaults.get(instruction.x as usize)?, &operand, &pin, &float)?,
        Op::Swizzle => {
            let (components, n) = code.swizzles.get(instruction.x as usize)?.as_ref()?;
            swizzle_expression(&float(0, 1)?, input_lanes(0), components, *n)?
        }
        Op::SampleColor | Op::SampleGray => {
            let info = code.samples.get(instruction.x as usize)?;
            let binding = sample_binding(instruction.x, &info.slot, inputs, images, bindings);
            sample_expression(binding, info.filter.unwrap_or(1), instruction.op == Op::SampleGray, || float(0, 2))?
        }
        Op::Clamp => {
            let (a, lo, hi) = (float(0, lanes)?, float(1, lanes)?, float(2, lanes)?);
            format!("clamp({a}, min({lo}, {hi}), max({lo}, {hi}))")
        }
        Op::Lerp => {
            let (a, b, t) = (float(0, lanes)?, float(1, lanes)?, float(2, lanes)?);
            format!("{a} + ({b} - {a}) * {t}")
        }
        Op::Smooth => call_lanes("tc_smooth", &[float(0, lanes)?, float(1, lanes)?, float(2, lanes)?], lanes),
        Op::If => {
            let condition = value(0)?;
            let read =
                if pins.get(index).copied().flatten() == Some(Pin::Bool) { [value(1)?, value(2)?] } else { [float(1, lanes)?, float(2, lanes)?] };
            let [on_true, on_false] = read;
            format!("select({on_false}, {on_true}, {condition})")
        }
        Op::ToFloat => float(0, lanes)?,
        Op::ToBool => {
            let n = input_lanes(0);
            non_zero(&float(0, if n > 1 { n } else { 1 })?, n)
        }
        _ => return None,
    })
}

fn call_lanes(function: &str, args: &[String], n: usize) -> String {
    if n == 1 {
        return format!("{function}({})", args.join(", "));
    }
    let lanes = (0..n)
        .map(|i| {
            let fields = args.iter().map(|v| format!("{v}[{i}u]")).collect::<Vec<_>>();
            format!("{function}({})", fields.join(", "))
        })
        .collect::<Vec<_>>();
    format!("vec{n}f({})", lanes.join(", "))
}

fn unary(op: U, a: &str, n: usize) -> Option<String> {
    let broadcast = |v: &str| if n == 1 { v.to_owned() } else { format!("vec{n}f({v})") };
    let z = broadcast("0.0");
    let one = broadcast("1.0");
    let call = |f| format!("{f}({a})");
    Some(match op {
        U::Neg => format!("-{a}"),
        U::Abs => call("abs"),
        U::Sqrt => format!("sqrt(max({a}, {z}))"),
        U::Exp => format!("exp(min({a}, {}))", broadcast("80.0")),
        U::Log | U::LogFloor | U::Log2 => format!(
            "select({}, {f}({a}), {a} > {z})",
            if op == U::LogFloor { broadcast("-1e30") } else { z.clone() },
            f = if op == U::Log2 { "log2" } else { "log" }
        ),
        U::Sin => call("sin"),
        U::Cos => call("cos"),
        U::SinPoly | U::CosPoly | U::SinF32Turns | U::CosF32Turns => {
            let f = match op {
                U::SinPoly => "tc_sin_poly",
                U::CosPoly => "tc_cos_poly",
                U::SinF32Turns => "tc_sin_turns",
                _ => "tc_cos_turns",
            };
            call_lanes(f, &[a.to_owned()], n)
        }
        U::Tan => call("tan"),
        U::Asin | U::Acos => format!("{}(clamp({a}, {}, {one}))", if op == U::Asin { "asin" } else { "acos" }, broadcast("-1.0")),
        U::Atan => call("atan"),
        U::Rad => call("radians"),
        U::Deg => call("degrees"),
        U::Floor => call("floor"),
        U::Ceil => call("ceil"),
        U::Round => format!("sign({a}) * floor(abs({a}) + {})", broadcast("0.5")),
        U::Fract => call("fract"),
        U::Sign => call("sign"),
        U::Sat => call("saturate"),
        U::Length => call(if n == 1 { "abs" } else { "length" }),
        U::Normalize if n == 1 => format!("select(0.0, sign({a}), abs({a}) > 1e-12)"),
        U::Normalize => format!("select({z}, {a} / length({a}), length({a}) > 1e-12)"),
        U::Not => return None,
    })
}

fn binary(op: B, a: &str, b: &str, n: usize) -> Option<String> {
    let zero = if n == 1 { "0.0".to_owned() } else { format!("vec{n}f(0.0)") };
    let call = |f| format!("{f}({a}, {b})");
    let map = |f| call_lanes(f, &[a.to_owned(), b.to_owned()], n);
    Some(match op {
        B::Add => format!("{a} + {b}"),
        B::Sub => format!("{a} - {b}"),
        B::Mul => format!("{a} * {b}"),
        B::Div => format!("select({zero}, {a} / {b}, {b} != {zero})"),
        B::DivLarge => map("tc_div_large"),
        B::Mod => format!("select({zero}, {a} - {b} * floor({a} / {b}), {b} != {zero})"),
        B::Pow => map("tc_pow"),
        B::Min => call("min"),
        B::Max => call("max"),
        B::Atan2 => call("atan2"),
        B::Atan2Approx => map("tc_atan2"),
        B::Step => call("step"),
        B::Dot if n == 1 => format!("{a} * {b}"),
        B::Dot => call("dot"),
        B::Cross => call("cross"),
        B::Distance if n == 1 => format!("abs({a} - {b})"),
        B::Distance => call("distance"),
        B::Eq if n == 1 => format!("{a} == {b}"),
        B::Ne if n == 1 => format!("{a} != {b}"),
        B::Eq => format!("all({a} == {b})"),
        B::Ne => format!("any({a} != {b})"),
        B::Lt => format!("{a} < {b}"),
        B::Le => format!("{a} <= {b}"),
        B::Gt => format!("{a} > {b}"),
        B::Ge => format!("{a} >= {b}"),
        B::And | B::Or | B::Xor => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use std::collections::BTreeMap;

    /// The pixel shader of a single-precision function made of `body`, if it can be lowered.
    fn shader(body: &str) -> Option<PixelShader> {
        let g = Graph::parse_unchecked(&format!("meta kind \"function\"\nmeta float32 \"1\"\n{body}")).unwrap();
        Prog::compile(&g, &BTreeMap::new()).unwrap().pixel_shader(&FnEnv::new([8.0, 8.0]))
    }

    /// A float2 is true when one of its lanes is: comparing a `vec2f` with the scalar `0.0` is not valid WGSL.
    #[test]
    fn a_vector_is_reduced_to_a_bool_with_any() {
        let s = shader("node 1 f_pos\nnode 2 f_to_bool a=@1\nnode 3 f_out in=@2\n").expect("a shader for the position test");
        assert!(s.source.contains("any(") && s.source.contains("!= vec2f(0.0))"), "{}", s.source);
        let s =
            shader("node 1 f_pos\nnode 4 f_swizzle a=@1 mask=\"x\"\nnode 2 f_to_bool a=@4\nnode 3 f_out in=@2\n").expect("a shader for the scalar");
        assert!(!s.source.contains("any("), "a scalar is compared as it is: {}", s.source);
    }

    /// A linked vector fills several lanes of the vector on the CPU, which `vec2f(r1, 0.5)` in WGSL would not say.
    #[test]
    fn a_vector_part_of_a_vector_is_left_to_the_cpu() {
        assert!(shader("node 1 f_pos\nnode 2 f_vec4 x=@1 y=0.5\nnode 3 f_out in=@2\n").is_none());
        // scalar parts still lower
        assert!(shader("node 1 f_pos\nnode 4 f_swizzle a=@1 mask=\"x\"\nnode 2 f_vec4 x=@4 y=0.5\nnode 3 f_out in=@2\n").is_some());
    }
}
