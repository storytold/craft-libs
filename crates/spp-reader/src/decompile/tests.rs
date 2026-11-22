//! Synthetic expressions and descriptions (no data from real files in the repository).
//!
//! These tests check the graph text the decompiler writes. That the graph runtime runs it as intended is checked where
//! both crates meet (the importer's tests), since this crate does not depend on the runtime.

use super::functions::Functions;
use crate::decode::graph_package::compiled::{Expr, Section};
use crate::decode::graph_package::{Description, root_xml};

fn expr(words: &[u32]) -> Expr {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Expr::parse(&Section::new(&bytes, 0), 0).unwrap()
}

fn description() -> Description {
    Description::parse(
        root_xml(
            r#"<ROOT><graphs><graph pkgurl="pkg://t" label="T"><inputs>
        <input uid="4660" identifier="invert" type="4" default="0"/></inputs></graph></graphs></ROOT>"#,
        )
        .as_bytes(),
    )
    .unwrap()
}

/// A synthetic 8x8 grayscale JPEG, a ramp from 0 to 252.
const JPEG_RAMP: &str = "ffd8ffe000104a46494600010100000100010000ffdb0043000201010101010201010102020202020403020202020504040304060506060605060606070908060709070606080b08090a0a0a0a0a06080b0c0b0a0c090a0a0affc0000b080008000801011100ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b5100002010303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffda0008010100003f00aff0dbfe0db8ff0057ff001417a7fcbb7ff5abffd9";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

#[test]
fn an_embedded_jpeg_becomes_a_grey_png() {
    let png = super::bitmaps::jpeg_to_png(&hex(JPEG_RAMP)).unwrap();
    let (w, h, depth, color, grey) = crate::png_out::decode(&png);
    assert_eq!((w, h, depth, color), (8, 8, 8, 0), "grayscale stays grayscale");
    assert!(grey[0] < 13, "{}", grey[0]);
    assert!(grey[63] > 230, "{}", grey[63]);
}

#[test]
fn an_embedded_colour_image_takes_its_second_jpeg_as_alpha() {
    let ramp = hex(JPEG_RAMP);
    let png = super::bitmaps::jpeg_with_alpha_to_png(&ramp, &ramp).unwrap();
    let (w, h, depth, color, rgba) = crate::png_out::decode(&png);
    assert_eq!((w, h, depth, color), (8, 8, 8, 6));
    // the alpha follows the ramp, dark and transparent at the start, bright and opaque at the end
    assert!(rgba[3] < 13, "{:?}", &rgba[..4]);
    assert!(rgba[63 * 4 + 3] > 230, "{:?}", &rgba[63 * 4..]);
    // an alpha of another size is refused
    assert!(super::bitmaps::jpeg_with_alpha_to_png(&ramp, &[0xff, 0xd8, 0xff, 0xe0, 0, 1]).is_err());
}

#[test]
fn a_broken_jpeg_is_an_error_not_a_panic() {
    assert!(super::bitmaps::jpeg_to_png(&[0xff, 0xd8, 0xff, 0xe0, 0, 1]).is_err());
}

#[test]
fn a_node_without_a_size_expression_has_the_size_in_its_tag() {
    let asm = Assembly::parse(&one_uniform()).unwrap();
    let d = super::decompile(&description(), &asm, 64);
    assert!(d.text.contains("node 1 color") && d.text.contains("osize_log2_x=8 osize_log2_y=4"), "{}", d.text);
    assert!(d.text.starts_with("format 3\nsize 64\n"), "{}", d.text);
    assert_eq!(super::GRAPH_FORMAT, 3);
}

use crate::decode::graph_package::compiled::Assembly;

/// One uniform, no size expression: tag op 0x0c, variant 0x48 (height 2^4, width 2^8), flags 0x0108 (inline value).
fn one_uniform() -> Vec<u8> {
    let words: [u32; 13] = [0, 0x0c, 0, 0x0108_480c, 0x3f00_0000, 8, 8, 0x0001_0001, 0xcccc, 8, 0xbbbb, 1, 0x04];
    let mut section: Vec<u32> = words.to_vec();
    section.extend([0x08, 0x14]);
    let total = (0x34 + 4 * section.len()) as u32;
    let header = [u32::from_le_bytes(*b"SBAM"), 4 << 16, 11, 22, total, 0x1c, 0, total - 0x1c, 0x10002, 0, 1, 0x1c, 2];
    header.iter().chain(&section).flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn hostile_assemblies_and_descriptions_decompile_without_panicking() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let good = one_uniform();
    let desc = description();
    let mut rng = 0x5eed_u64;
    let mut next = || {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (rng >> 33) as usize
    };
    let mut inputs: Vec<Vec<u8>> = (0..good.len()).map(|n| good[..n].to_vec()).collect();
    for _ in 0..2000 {
        let mut v = good.clone();
        for _ in 0..1 + next() % 4 {
            let at = next() % v.len();
            v[at] = match next() % 4 {
                0 => 0xff,
                1 => 0x00,
                2 => v[at].wrapping_add(1),
                _ => next() as u8,
            };
        }
        inputs.push(v);
    }
    for input in inputs {
        let run = || {
            if let Ok(asm) = Assembly::parse(&input) {
                let _ = super::decompile(&desc, &asm, 64);
            }
        };
        assert!(catch_unwind(AssertUnwindSafe(run)).is_ok(), "panicked on {:02x?}", input);
    }
}

mod values {
    use super::super::functions::{Functions, ValueNode};
    use super::*;

    fn body<'a>(fns: &'a Functions, name: &str) -> &'a str {
        fns.subgraphs.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_str()).unwrap()
    }

    /// const2 (0.25, 0.75): 1 instruction
    fn pair() -> Expr {
        expr(&[0x1140_0001, 0x3e80_0000, 0x3f40_0000])
    }

    #[test]
    fn a_value_is_computed_where_it_is_read() {
        let mut fns = Functions::default();
        fns.values = vec![ValueNode { offset: 3, words: 2, expr: pair(), source: 0, seed: None, reads_images: false, inputs: Vec::new() }];
        // inputat1 #4 (token 0x0503): the second word of the value
        let name = fns.function(&expr(&[0x0503_0001, 4]), &description(), None);
        assert!(body(&fns, &name).contains("0.75"), "{}", body(&fns, &name));
        assert!(fns.warnings.is_empty(), "{:?}", fns.warnings);
    }

    #[test]
    fn a_value_from_an_image_is_computed_in_a_host_through_slots_after_its_own() {
        let mut fns = Functions::default();
        fns.values = vec![ValueNode { offset: 3, words: 2, expr: pair(), source: 0, seed: None, reads_images: true, inputs: vec![7, 9] }];
        let (_, extra) = super::super::functions::with_host(2, || fns.function(&expr(&[0x0503_0001, 4]), &description(), None));
        assert_eq!(extra, vec![7, 9], "the value node's images, in the slots after the host's two");
        assert!(fns.warnings.is_empty(), "{:?}", fns.warnings);
    }

    #[test]
    fn a_value_from_an_image_outside_a_host_or_outside_every_node_warns() {
        let mut fns = Functions::default();
        fns.values = vec![ValueNode { offset: 0, words: 2, expr: pair(), source: 0, seed: None, reads_images: true, inputs: vec![7] }];
        fns.function(&expr(&[0x0503_0001, 0]), &description(), None);
        assert_eq!(fns.warnings.len(), 1);
        let mut fns = Functions::default();
        fns.function(&expr(&[0x0503_0001, 9]), &description(), None);
        assert_eq!(fns.warnings.len(), 1);
    }

    #[test]
    fn init_expressions_are_written_before_the_function_reads_them() {
        let mut fns = Functions::default();
        // set1 (const 0.5, #2)
        fns.init = vec![expr(&[0x0900_0002, 0x3f00_0000, 0x0000_0907, 0x0002])];
        // get1 #2 [05]
        let name = fns.function(&expr(&[0x0504_0001, 2]), &description(), None);
        assert!(body(&fns, &name).contains("0.5"), "{}", body(&fns, &name));
    }
}

#[test]
fn the_working_colour_space_is_read_from_the_usertag() {
    assert!(super::working_space("colorspace=$working;padding=extend;alpha=straight"));
    assert!(!super::working_space("colorspace=$standardsrgb"));
    assert!(!super::working_space("defaultcolor=(128,128,255)"));
}

#[test]
fn declared_colours_are_read_from_the_usertag() {
    let tag = "defaultcolor=(128,128,255);Padding=Extend;ColorSpace=NormalXYZLeft";
    assert_eq!(super::default_color(tag), Some([128.0 / 255.0, 128.0 / 255.0, 1.0]));
    assert_eq!(super::default_color("Padding=Extend"), None);
    assert_eq!(super::default_color("defaultcolor=(1,2"), None);
}

#[test]
fn the_first_operand_of_a_sequence_runs_only_for_its_sets() {
    use crate::decode::graph_package::compiled::{Instr, Operands};
    let ins = |op, operands| Instr { op, components: 1, ty: 9, operands };
    let body = |instrs: Vec<Instr>| {
        let mut fns = Functions::default();
        let e = Expr { offset: 0, end: 0, instrs };
        fns.function(&e, &description(), None);
        fns.subgraphs[0].1.clone()
    };
    let one = ins(0x00, Operands::Constant(vec![0x3f80_0000]));
    let pure = body(vec![one.clone(), ins(0x32, Operands::Refs(vec![0])), ins(0x32, Operands::Refs(vec![0])), ins(0x0c, Operands::Refs(vec![1, 2]))]);
    assert!(pure.contains("f_pin") && !pure.contains("f_sequence"), "{pure}");
    let set = body(vec![
        one,
        ins(0x32, Operands::Refs(vec![0])),
        ins(0x07, Operands::RefSlot(1, 0)),
        ins(0x32, Operands::Refs(vec![0])),
        ins(0x0c, Operands::Refs(vec![2, 3])),
    ]);
    assert!(set.contains("f_sequence"), "{set}");
}

/// The code tables are total over the codes the format defines, and no two codes share a name.
#[test]
fn the_code_tables_name_every_code_once() {
    use super::functions::rand_mode;
    use crate::decode::graph_package::compiled::SampleDepth;
    let blends: Vec<&str> = (1..=11).map(super::blend_mode).collect();
    let mut unique = blends.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), blends.len(), "{blends:?}");
    assert_eq!((super::blend_mode(0), super::blend_mode(12)), ("copy", "copy"));
    assert_eq!(super::blend_mode(7), "switch");
    assert_eq!(super::blend_mode(9), "overlay");
    let formats: Vec<_> = [SampleDepth::Int8, SampleDepth::Int16, SampleDepth::Float16].into_iter().map(super::output_format).collect();
    assert!(formats.iter().all(Option::is_some), "{formats:?}");
    assert_eq!(super::output_format(SampleDepth::Float32), None);
    let shapes: Vec<_> = (0..=16u8).map(|c| super::programs::pattern_shape(c).unwrap()).collect();
    let mut unique = shapes.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 17);
    assert_eq!(super::programs::pattern_shape(2), Some("paraboloid"));
    assert_eq!(super::programs::pattern_shape(17), None);
    for mode in 0..=20u8 {
        assert_eq!(rand_mode::site_name(mode).is_some(), matches!(mode, 1 | 2 | 3 | 5 | 10..=18), "generator {mode}");
    }
}

/// One text node (0x22) whose font starts with `sfnt` (the layout of the font graphs; see the decoder's test).
fn one_text(sfnt: [u8; 4]) -> Vec<u8> {
    let section: Vec<u32> = vec![
        0,
        0,
        4,
        u32::from_le_bytes(sfnt),
        0x18,
        0,
        0x0001_8822,
        0x3282,
        0,
        0xdddd,
        0x04,
        0x3c,
        0x44,
        0x4c,
        0x54,
        0x0a42_0001,
        0xbbbb,
        0x0942_0001,
        0xeeee,
        0x0902_0001,
        0xffff,
        0x0a02_0001,
        0x1111,
        8,
        8,
        0x0005_0001,
        0xcccc,
        8,
        0xbbbb,
        6,
        0xdddd,
        1,
        0xeeee,
        0,
        0xffff,
        4,
        0x1111,
        1,
        0x10,
        0x14,
        0x5c,
    ];
    let total = (0x34 + 4 * section.len()) as u32;
    let header = [u32::from_le_bytes(*b"SBAM"), 4 << 16, 11, 22, total, 0x1c, 0, total - 0x1c, 0x10002, 0, 1, 0x64, 2];
    header.iter().chain(&section).flat_map(|w| w.to_le_bytes()).collect()
}

fn text_description() -> Description {
    Description::parse(
        root_xml(
            r#"<ROOT><graphs><graph pkgurl="pkg://font_t" label="Font T"><outputs><output uid="52428" identifier="output"/></outputs><inputs>
        <input uid="48059" identifier="$outputsize" type="8" default="8,8"/>
        <input uid="56797" identifier="text" type="6" default="Ab"/>
        <input uid="61166" identifier="position" type="1" default="0,0.05"/>
        <input uid="65535" identifier="fontsize" type="0" default="0.2"/>
        <input uid="4369" identifier="alignment" type="4" default="1"/></inputs></graph></graphs></ROOT>"#,
        )
        .as_bytes(),
    )
    .unwrap()
}

#[test]
fn a_text_node_draws_its_text_input_with_the_stored_truetype_font_or_else_the_built_in_face() {
    let d = super::decompile(&text_description(), &Assembly::parse(&one_text([0, 1, 0, 0])).unwrap(), 64);
    assert!(d.text.contains("node 1 text y_up=1 font=\"font_4.ttf\" fn.text="), "{}", d.text);
    assert!(d.text.contains("resource font_4.ttf ") && d.text.contains("input_value name=\"text\" type=9 v=\"Ab\""), "{}", d.text);
    assert!(!d.text.contains("expose \"text\""), "a text has no slider");
    // OpenType (CFF) outlines, which the runtime cannot read: the built-in face, with a warning, instead of a failed render
    let d = super::decompile(&text_description(), &Assembly::parse(&one_text(*b"OTTO")).unwrap(), 64);
    assert!(d.text.contains("node 1 text y_up=1 fn.text=") && !d.text.contains("resource font_"), "{}", d.text);
    assert!(d.warnings.iter().any(|w| w.contains("CFF")), "{:?}", d.warnings);
}
