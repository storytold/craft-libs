//! The WGSL text of the shading functions: it parses and validates, alone and joined with an app's
//! bindings and entry points; every Rust shading function has its WGSL counterpart; and the GPU
//! layouts write the bytes where the WGSL structs read them.

use craft_pbr::{GpuMaterial, GpuSh9, Limits, MAX_RADIANCE, Material, PbrError, SHADE_WGSL, sh_basis, try_sh9};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{Block, Module, Statement, TypeInner};

/// Each Rust shading function and its WGSL counterpart. The README's table lists the same pairs.
const COUNTERPARTS: [(&str, &str); 12] = [
    ("Material::try_new", "pbr_material_clamped"),
    ("Material::f0", "pbr_f0"),
    ("Material::try_direct", "pbr_direct"),
    ("fresnel_schlick", "pbr_fresnel_schlick"),
    ("smith_schlick_ggx", "pbr_smith_schlick_ggx"),
    ("ggx_distribution", "pbr_ggx_distribution"),
    ("try_environment_brdf", "pbr_environment_brdf"),
    ("dir_of", "pbr_dir_of"),
    ("uv_of", "pbr_uv_of"),
    ("sh_basis", "pbr_sh_basis"),
    ("irradiance", "pbr_irradiance"),
    ("unit", "pbr_unit"),
];

/// App-side declarations and entry points in the shape EffectCraft's Advanced 3D shader has: a
/// uniform block, material and SH buffers, an environment texture, and `vs` and `fs` that shade
/// one light and the environment with the shared functions.
const APP_DECLARATIONS: &str = "
struct Globals {
    clip: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    env: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> materials: array<PbrMaterial>;
@group(0) @binding(2) var<uniform> sh: PbrSh9;
@group(0) @binding(3) var env_tex: texture_2d<f32>;
@group(0) @binding(4) var env_smp: sampler;
";

const APP_ENTRY_POINTS: &str = "
struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) material: u32,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) material: u32,
};
@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.clip = g.clip * vec4<f32>(v.pos, 1.0);
    o.pos = v.pos;
    o.normal = v.normal;
    o.material = v.material;
    return o;
}
@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let m = materials[i.material];
    let n = pbr_unit(i.normal);
    let v = pbr_unit(g.eye.xyz - i.pos);
    var c = pbr_direct(m, n, v, g.light.xyz);
    let lv = pbr_specular_levels(m.roughness, u32(g.env.x));
    let uv = pbr_uv_of(reflect(-v, n));
    let a = textureSampleLevel(env_tex, env_smp, uv, lv.x).rgb;
    let b = textureSampleLevel(env_tex, env_smp, uv, lv.y).rgb;
    c = c + pbr_environment_light(m, dot(n, v), pbr_irradiance(sh, n), mix(a, b, lv.z));
    return vec4<f32>(c, 1.0);
}
";

/// Parses and validates `source`, failing the test with naga's report.
fn module(source: &str) -> Module {
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    if let Err(e) = Validator::new(ValidationFlags::all(), Capabilities::default()).validate(&module) {
        panic!("{}", e.emit_to_string(source));
    }
    module
}

fn function_names(m: &Module) -> Vec<&str> {
    m.functions.iter().filter_map(|(_, f)| f.name.as_deref()).collect()
}

fn has_loop(block: &Block) -> bool {
    block.iter().any(|s| match s {
        Statement::Loop { .. } => true,
        Statement::Block(b) => has_loop(b),
        Statement::If { accept, reject, .. } => has_loop(accept) || has_loop(reject),
        Statement::Switch { cases, .. } => cases.iter().any(|c| has_loop(&c.body)),
        _ => false,
    })
}

/// The members of the struct named `name` as (member name, offset, type), and its size.
fn struct_layout<'m>(m: &'m Module, name: &str) -> (Vec<(&'m str, u32, &'m TypeInner)>, u32) {
    let ty = m.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no struct {name}")).1;
    let TypeInner::Struct { members, span } = &ty.inner else { panic!("{name} is not a struct") };
    let fields = members.iter().map(|mb| (mb.name.as_deref().unwrap_or(""), mb.offset, &m.types[mb.ty].inner)).collect();
    (fields, *span)
}

fn f32_at(bytes: &[u8], at: u32) -> f32 {
    let at = at as usize;
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn shading_text_validates_with_an_apps_bindings_and_entry_points() {
    let source = [APP_DECLARATIONS, SHADE_WGSL, APP_ENTRY_POINTS].join("\n");
    let m = module(&source);
    let entry: Vec<&str> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(entry, ["vs", "fs"]);
}

#[test]
fn shading_text_alone_has_no_bindings_entry_points_or_loops() {
    let m = module(SHADE_WGSL);
    assert!(m.global_variables.is_empty(), "the text declares a binding");
    assert!(m.entry_points.is_empty(), "the text declares an entry point");
    for (_, f) in m.functions.iter() {
        assert!(!has_loop(&f.body), "{:?} loops", f.name);
    }
    let names = m
        .functions
        .iter()
        .filter_map(|(_, f)| f.name.clone())
        .chain(m.constants.iter().filter_map(|(_, c)| c.name.clone()))
        .chain(m.types.iter().filter_map(|(_, t)| t.name.clone()).filter(|n| n.chars().next().is_some_and(char::is_uppercase)));
    for name in names {
        assert!(name.starts_with("pbr_") || name.starts_with("PBR_") || name.starts_with("Pbr"), "{name} has no pbr prefix");
    }
}

#[test]
fn every_rust_shading_function_has_a_wgsl_counterpart_listed_in_the_readme() {
    let m = module(SHADE_WGSL);
    let names = function_names(&m);
    let readme = include_str!("../../README.md");
    for (rust, wgsl) in COUNTERPARTS {
        assert!(names.contains(&wgsl), "{wgsl}, the counterpart of {rust}, is missing from SHADE_WGSL");
        let row = format!("| `{rust}` | `{wgsl}` |");
        assert!(readme.lines().any(|l| l.starts_with(&row)), "the README has no row for {rust} and {wgsl}");
    }
}

#[test]
fn gpu_material_bytes_sit_where_pbr_material_reads_them() {
    let m = module(SHADE_WGSL);
    let (fields, span) = struct_layout(&m, "PbrMaterial");
    assert_eq!(span as usize, GpuMaterial::SIZE);
    let mat = Material::try_new([0.1, 0.2, 0.3], 0.4, 0.5).unwrap().try_with_weights(6.0, 0.7).unwrap();
    let bytes = mat.gpu().to_bytes();
    assert_eq!(bytes.len(), span as usize);
    let names: Vec<&str> = fields.iter().map(|f| f.0).collect();
    assert_eq!(names, ["base_color", "metallic", "roughness", "specular", "diffuse"]);
    for (name, offset, _) in fields {
        let read = match name {
            "base_color" => vec![f32_at(&bytes, offset), f32_at(&bytes, offset + 4), f32_at(&bytes, offset + 8)],
            _ => vec![f32_at(&bytes, offset)],
        };
        let want = match name {
            "base_color" => mat.base_color().to_vec(),
            "metallic" => vec![mat.metallic()],
            "roughness" => vec![mat.roughness()],
            "specular" => vec![mat.specular()],
            _ => vec![mat.diffuse()],
        };
        assert_eq!(read, want, "{name} at byte {offset}");
    }
}

#[test]
fn gpu_sh9_bytes_sit_where_pbr_sh9_reads_them() {
    let m = module(SHADE_WGSL);
    let (fields, span) = struct_layout(&m, "PbrSh9");
    assert_eq!(span as usize, GpuSh9::SIZE);
    let &[(_, offset, &TypeInner::Array { stride, .. })] = fields.as_slice() else { panic!("PbrSh9 is not one array") };
    let coefficients: [[f32; 3]; 9] = std::array::from_fn(|k| std::array::from_fn(|c| (k * 3 + c) as f32 - 4.5));
    let bytes = GpuSh9::try_new(&coefficients).unwrap().to_bytes();
    assert_eq!(bytes.len(), span as usize);
    for (k, rgb) in coefficients.iter().enumerate() {
        for (c, want) in rgb.iter().enumerate() {
            assert_eq!(f32_at(&bytes, offset + stride * k as u32 + 4 * c as u32), *want, "coefficient {k} channel {c}");
        }
    }
}

#[test]
fn gpu_sh9_rejects_coefficients_that_are_not_finite_or_too_large() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, -1.5 * GpuSh9::MAX_COEFFICIENT] {
        let mut c = [[0.5f32; 3]; 9];
        c[8][2] = bad;
        assert!(matches!(GpuSh9::try_new(&c), Err(PbrError::BadOption(_))), "{bad}");
    }
    let mut c = [[0.5f32; 3]; 9];
    c[0][0] = -GpuSh9::MAX_COEFFICIENT;
    assert!(GpuSh9::try_new(&c).is_ok());
}

#[test]
fn gpu_sh9_accepts_the_brightest_environments_and_keeps_the_wgsl_sum_finite() {
    // A uniform environment and one lit only above the horizon, at the largest radiance.
    for lit_rows in [8, 4] {
        let env = crate::env_from(16, 8, |_, y| if y < lit_rows { [MAX_RADIANCE; 3] } else { [0.0; 3] });
        let sh = try_sh9(&env, &Limits::default()).unwrap();
        assert!(GpuSh9::try_new(&sh).is_ok(), "{lit_rows} lit rows: {sh:?}");
    }
    // pbr_irradiance sums nine coefficients of at most MAX_COEFFICIENT times the basis values.
    let mut largest = 0.0f32;
    for i in 0..4096u32 {
        let [u, v] = craft_pbr::hammersley(i, 4096).unwrap();
        let b = sh_basis(craft_pbr::dir_of(u, v)).unwrap();
        largest = largest.max(b.iter().map(|y| y.abs()).sum());
    }
    assert!(largest < 5.0, "basis sum {largest}");
    assert!((5.0 * GpuSh9::MAX_COEFFICIENT).is_finite());
}
