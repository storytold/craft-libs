//! Named values of the graph language: the words a graph text (format 3 on) writes where formats 1 and 2 wrote numbers,
//! and the old spellings it still reads.
//!
//! A node keeps these values as numbers in memory (`Node::nums`), so the evaluator and the editor see the same thing they
//! always did; only the text changes. Reading accepts both a name and the old number, writing gives the name whenever
//! the number has one (a number without a name is written as the number, which every version reads).
//!
//! | node kind | text key | in memory | values |
//! |---|---|---|---|
//! | every kind | `format` | `ofmt` | [`OUTPUT_FORMATS`] |
//! | `blend` | `mode` | `mode` | [`BLEND_MODES`] |
//! | `pp_quadrant` | `pattern` | `pattern` | [`PATTERN_SHAPES`] |
//! | `f_random` | `lcg` | `lcg` | [`LCG_SITES`] |
//!
//! Older spellings read as well: the key `ofmt=<n>`, numbers in place of the names, and the function ops of
//! [`RENAMED_OPS`] under their old names. The numbers of the compiled format are not here: the decompiler has its own
//! table from them to these names.

use crate::graph::Node;

/// Function ops renamed in format 3: (old name, name).
pub const RENAMED_OPS: &[(&str, &str)] = &[
    ("f_fsin", "f_sin_poly"),
    ("f_fcos", "f_cos_poly"),
    ("f_flog", "f_log_floor"),
    ("f_fdiv", "f_div_large"),
    ("f_fatan2", "f_atan2_approx"),
    ("f_gsin", "f_sin_f32turns"),
    ("f_gcos", "f_cos_f32turns"),
];

/// A node's Output Format: 0 the graph's own, 1 to 4 plain quantising (4 forces 32-bit float), 5 to 7 quantising as
/// compiled graphs do it (rounding of their own, and blurs and levels that hold the format on the way).
pub const OUTPUT_FORMATS: &[(u32, &str)] =
    &[(0, "graph"), (1, "u8"), (2, "u16"), (3, "f16"), (4, "f32"), (5, "u8_compiled"), (6, "u16_compiled"), (7, "f16_compiled")];

/// Modes of the `blend` node (`ops::blend_ex`). 32 to 36 are compiled graphs' own versions, whose opacity acts before
/// the result is limited (soft light: a square root above the middle).
pub const BLEND_MODES: &[(u32, &str)] = &[
    (0, "copy"),
    (1, "add"),
    (2, "subtract"),
    (3, "multiply"),
    (4, "screen"),
    (5, "overlay"),
    (6, "max"),
    (7, "min"),
    (8, "difference"),
    (9, "soft_light"),
    (10, "color_dodge"),
    (11, "color_burn"),
    (12, "linear_burn"),
    (13, "hard_light"),
    (14, "exclusion"),
    (15, "divide"),
    (16, "linear_light"),
    (17, "color"),
    (18, "luminosity"),
    (19, "dissolve"),
    (20, "add_sub"),
    (21, "switch"),
    (32, "add_compiled"),
    (33, "subtract_compiled"),
    (34, "add_sub_compiled"),
    (35, "divide_compiled"),
    (36, "soft_light_compiled"),
];

/// Shapes a pattern program's quadrant draws (`patterns::shape`).
pub const PATTERN_SHAPES: &[(u32, &str)] = &[
    (0, "square"),
    (1, "disc"),
    (2, "paraboloid"),
    (3, "bell"),
    (4, "gaussian"),
    (5, "thorn"),
    (6, "pyramid"),
    (7, "brick"),
    (8, "gradation"),
    (9, "waves"),
    (10, "half_bell"),
    (11, "ridged_bell"),
    (12, "crescent"),
    (13, "capsule"),
    (14, "cone"),
    (15, "gradation_offset"),
    (16, "hemisphere"),
];

/// Which 24-bit generator `f_random` draws from, by where the function runs (`lcg_rand`): a pixel processor per pixel,
/// a value node once (draw `k`), a node's parameter expression, a pixel processor's size expression (draw `k`), and
/// the expressions of a fx-map's quadrant by what they compute (`lcg_rand::FX_SITES` in this order).
pub const LCG_SITES: &[(u32, &str)] = &[
    (1, "pixel"),
    (2, "value"),
    (3, "parameter"),
    (5, "size"),
    (10, "fx_luminance"),
    (11, "fx_branch_offset"),
    (12, "fx_pattern_offset"),
    (13, "fx_size"),
    (14, "fx_rotation"),
    (15, "fx_variation"),
    (16, "fx_switch_condition"),
    (17, "fx_iterate_count"),
    (18, "fx_image_index"),
];

/// One named value: on which kinds (`None` = all), its key in the text, its key in memory and its names.
struct Named {
    kind: Option<&'static str>,
    key: &'static str,
    memory: &'static str,
    table: &'static [(u32, &'static str)],
}

const NAMED: &[Named] = &[
    Named { kind: None, key: "format", memory: "ofmt", table: OUTPUT_FORMATS },
    Named { kind: Some("blend"), key: "mode", memory: "mode", table: BLEND_MODES },
    Named { kind: Some("pp_quadrant"), key: "pattern", memory: "pattern", table: PATTERN_SHAPES },
    Named { kind: Some("f_random"), key: "lcg", memory: "lcg", table: LCG_SITES },
];

fn applies(n: &Named, kind: &str) -> bool {
    n.kind.is_none_or(|k| k == kind)
}

/// The name of `code` in a table.
pub fn name_of(table: &[(u32, &'static str)], code: u32) -> Option<&'static str> {
    table.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// The number of `name` in a table.
pub fn code_of(table: &[(u32, &str)], name: &str) -> Option<u32> {
    table.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

/// The op's current name, for one written under an old name.
pub fn current_op(kind: &str) -> Option<&'static str> {
    RENAMED_OPS.iter().find(|(old, _)| *old == kind).map(|(_, new)| *new)
}

/// A node just read from a text, to its form in memory: old op names and keys to the current ones, names to numbers.
pub fn read_node(n: &mut Node) -> Result<(), String> {
    if let Some(new) = current_op(&n.kind) {
        n.kind = new.to_string();
    }
    for named in NAMED.iter().filter(|x| applies(x, &n.kind)) {
        if let Some(name) = n.strs.remove(named.key) {
            let code = code_of(named.table, &name).ok_or_else(|| format!("unknown {} '{name}' (known: {})", named.key, names(named.table)))?;
            n.nums.insert(named.memory.to_string(), code as f32);
        } else if named.key != named.memory
            && let Some(v) = n.nums.remove(named.key)
        {
            // the new key with a number
            n.nums.insert(named.memory.to_string(), v);
        }
    }
    Ok(())
}

fn names(table: &[(u32, &str)]) -> String {
    table.iter().map(|(_, n)| *n).collect::<Vec<_>>().join(", ")
}

/// How a number of a node of `kind` is written: `Some(key, name)` when it has a name.
pub fn written_name(kind: &str, key: &str, v: f32) -> Option<(&'static str, &'static str)> {
    let named = NAMED.iter().find(|x| x.memory == key && applies(x, kind))?;
    if v < 0.0 || v.fract() != 0.0 || v > u32::MAX as f32 {
        return None;
    }
    name_of(named.table, v as u32).map(|name| (named.key, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_is_one_to_one() {
        for t in [OUTPUT_FORMATS, BLEND_MODES, PATTERN_SHAPES, LCG_SITES] {
            for (i, (c, n)) in t.iter().enumerate() {
                assert!(t.iter().skip(i + 1).all(|(c2, n2)| c2 != c && n2 != n), "{n} twice");
                assert!(n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'), "{n} is one word");
            }
        }
        for (i, (old, new)) in RENAMED_OPS.iter().enumerate() {
            assert!(RENAMED_OPS.iter().skip(i + 1).all(|(o, n)| o != old && n != new));
            assert!(crate::func::FUNC_SPECS.iter().any(|s| s.kind == *new), "{new} is an op");
        }
    }

    /// A graph written in format 2: every number and old name this module renames (`f_random`'s generator as a number).
    const OLD: &str = "size 16
meta compiled \"1\"
subgraph px
meta kind \"function\"
node 2 f_seed
node 3 f_float v=1
node 4 f_random b=@3 lcg=1 s=@2
node 5 f_fsin a=@4
node 6 f_fcos a=@4
node 7 f_gsin a=@4
node 8 f_gcos a=@4
node 9 f_flog a=@4
node 10 f_fdiv a=@5 b=@6
node 11 f_fatan2 a=@7 b=@8
node 12 f_add a=@10 b=@11
node 13 f_add a=@12 b=@9
node 14 f_random b=@3 lcg=5 s=@2 k=2
node 15 f_add a=@13 b=@14
node 1 f_out in=@15
end
subgraph lum
meta kind \"function\"
node 2 f_float v=1
node 3 f_random b=@2 lcg=10
node 1 f_out in=@3
end
subgraph p
meta kind \"patterns\"
node 1 pp_quadrant pattern=4 draws=1 fn.luminance=\"lum\" size_x=0.5 size_y=0.5
node 2 pp_quadrant draws=0 q0=@1 q1=@1 q2=@1 q3=@1
node 3 pp_out in=@2
end
node 1 pixel_processor fn=\"px\" ofmt=5
node 2 pattern_program program=\"p\" ofmt=7
node 3 blend mode=21 alpha_mode=1 bg=@1 fg=@2 opacity=0.5
node 4 blend mode=34 alpha_mode=1 bg=@3 fg=@1 opacity=0.25 ofmt=6
output o 4
";

    #[test]
    fn old_spellings_read_as_the_new_names_and_give_the_same_image() {
        let old = crate::graph::Graph::parse(OLD).unwrap_or_else(|e| panic!("{e}"));
        let text = old.to_text();
        assert!(text.starts_with(&format!("format {}\n", crate::graph::FORMAT_VERSION)));
        for gone in ["ofmt", "f_fsin", "f_fcos", "f_gsin", "f_gcos", "f_flog", "f_fdiv", "f_fatan2", "mode=21", "pattern=4"] {
            assert!(!text.contains(gone), "{gone} still written:\n{text}");
        }
        for name in [
            "lcg=\"pixel\"",
            "lcg=\"size\"",
            "lcg=\"fx_luminance\"",
            "f_sin_poly",
            "f_cos_poly",
            "f_sin_f32turns",
            "f_cos_f32turns",
            "f_log_floor",
            "f_div_large",
            "f_atan2_approx",
            "format=\"u8_compiled\"",
            "format=\"u16_compiled\"",
            "format=\"f16_compiled\"",
            "mode=\"switch\"",
            "mode=\"add_sub_compiled\"",
            "pattern=\"gaussian\"",
        ] {
            assert!(text.contains(name), "{name} not written:\n{text}");
        }
        let new = crate::graph::Graph::parse(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(new.to_text(), text, "writing again changes nothing");
        let (a, b) = (&old.evaluate(Some(16)).unwrap()["o"], &new.evaluate(Some(16)).unwrap()["o"]);
        let bits = |t: &crate::tex::Tex| t.px.iter().flat_map(|p| p.map(f32::to_bits)).collect::<Vec<_>>();
        assert_eq!(bits(a), bits(b));
        assert!(a.px.iter().any(|p| p[0] != a.px[0][0]), "the test graph draws something");
    }

    #[test]
    fn a_number_without_a_name_stays_a_number_and_an_unknown_name_is_refused() {
        let g = crate::graph::Graph::parse("size 4\nnode 1 color r=1 g=1 b=1\nnode 2 blend mode=25 bg=@1 fg=@1 ofmt=9\noutput o 2\n").unwrap();
        let text = g.to_text();
        assert!(text.contains(" mode=25") && text.contains(" ofmt=9"), "{text}");
        assert_eq!(crate::graph::Graph::parse(&text).unwrap().to_text(), text);
        let e = crate::graph::Graph::parse("size 4\nnode 1 color r=1 g=1 b=1\nnode 2 blend mode=\"sideways\" bg=@1 fg=@1\noutput o 2\n").unwrap_err();
        assert!(e.to_string().contains("unknown mode 'sideways'"), "{e}");
    }

    #[test]
    fn an_image_quadrant_reads_its_input_from_image_index_or_the_old_pattern_number() {
        let text = |q: &str| {
            format!(
                "size 8\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant {q} draws=1 image=1 image_filter=2\nnode 2 pp_out in=@1\nend\nnode 1 color r=1 g=0 b=0\nnode 2 color r=0 g=1 b=0\nnode 3 pattern_program program=\"p\" color=1 in=@1 in2=@2\noutput o 3\n"
            )
        };
        let render = |q: &str| crate::graph::Graph::parse(&text(q)).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(8)).unwrap()["o"].px.clone();
        assert_eq!(render("image_index=1"), render("pattern=1"));
        assert_ne!(render("image_index=1"), render("image_index=0"));
    }
}
