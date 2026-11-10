//! Synthetic assemblies built word by word (no vendor data in the repository).

use super::*;

/// Builds a file: header, then `section` words at the section base. `root` is the root's section offset.
fn file(section: &[u32], root: u32) -> Vec<u8> {
    let total = (SECTION_BASE + 4 * section.len()) as u32;
    let header = [u32::from_le_bytes(*b"SBAM"), 4 << 16, 11, 22, total, 0x1c, 0, total - 0x1c, 0x10002, 0, 1, root, 2];
    header.iter().chain(section).flat_map(|w| w.to_le_bytes()).collect()
}

/// A graph with one image input copied by one input node (0x20), sized by `$outputsize`.
fn one_input_graph() -> Vec<u8> {
    let w = vec![
        0,           // 0x00
        0x0c,        // 0x04: node table, one entry: node at 0x0c
        0,           // 0x08 (padding)
        0x0019_8820, // 0x0c: tag: op 0x20, variant 0x88, flags 0x0019
        0xaaaa,      // 0x10: image input uid
        0x18,        // 0x14: -> size expression
        0x0a42_0001, // 0x18: expression: 1 instruction, read input (int2)
        0xbbbb,      // 0x1c: $outputsize uid
        8,           // 0x20: defaults: $outputsize = 8,8
        8,           // 0x24
        0x0002_0001, // 0x28: root: 1 output, 2 inputs
        0xcccc,      // output uid
        8,
        0xbbbb, // ($outputsize, int2)
        5,
        0xaaaa, // (image)
        1,      // node count
        0x04,   // node table
        0x08,   // node table end
        0x20,   // defaults
    ];
    file(&w, 0x28)
}

#[test]
fn parses_header_root_and_nodes() {
    let a = Assembly::parse(&one_input_graph()).unwrap();
    assert_eq!((a.version, a.cooker_build, a.asm_uid), (4, 11, 22));
    assert_eq!(a.outputs, vec![0xcccc]);
    assert_eq!(a.inputs, vec![AsmInput { ty: 8, uid: 0xbbbb }, AsmInput { ty: 5, uid: 0xaaaa }]);
    assert_eq!(a.nodes.len(), 1);
    let n = &a.nodes[0];
    assert_eq!((n.opcode, n.variant, n.flags), (0x20, 0x88, 0x0019));
    assert_eq!(n.body, vec![0xaaaa, 0x18, 0x0a42_0001, 0xbbbb]);
    let size = Expr::parse(&a.section(), n.word(1).unwrap()).unwrap();
    assert_eq!(size.as_input(), Some(0xbbbb));
    assert_eq!(size.end, 0x20);
}

#[test]
fn rejects_wrong_magic_and_truncation() {
    assert!(matches!(Assembly::parse(b"NOPE"), Err(DecodeError::BadMagic { .. })));
    let mut f = one_input_graph();
    f.truncate(0x50);
    assert!(Assembly::parse(&f).is_err());
}

#[test]
fn decodes_select_with_aligned_constants() {
    // (invert ? 1.0 : 0.0): read int input, two float constants, select.
    let words = [0x0842_0004u32, 0x1234, 0x0000_0d00, 0x3f80_0000, 0x0000_0d00, 0, 0x0000_0d09, 0x0002_0001];
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let s = Section::new(&bytes, 0);
    let e = Expr::parse(&s, 0).unwrap();
    assert_eq!(e.instrs.len(), 4);
    assert_eq!(e.instrs[0].operands, Operands::Input(0x1234));
    assert_eq!(e.instrs[1].operands, Operands::Constant(vec![0x3f80_0000]));
    assert_eq!(e.instrs[3].operands, Operands::Refs(vec![0, 1, 2]));
    assert_eq!(e.instrs[3].name(), "ifelse");
    assert_eq!(e.end, 32);
}

#[test]
fn rejects_forward_operands_and_unknown_operations() {
    // count 2: a constant, then add(%5, %0) where %5 does not exist yet
    let fwd = [0x0d00_0002u32, 0x3f80_0000, 0x0005_0912, 0];
    let bytes: Vec<u8> = fwd.iter().flat_map(|w| w.to_le_bytes()).collect();
    assert!(Expr::parse(&Section::new(&bytes, 0), 0).is_err());
    let unknown = [0x003f_0001u32];
    let bytes: Vec<u8> = unknown.iter().flat_map(|w| w.to_le_bytes()).collect();
    assert!(Expr::parse(&Section::new(&bytes, 0), 0).is_err());
}

#[test]
fn boolean_constants_are_one_unaligned_u16() {
    // 2 instructions: bool2 constant (bits 0xffff), then set(%0, #1)
    let words = [0x0440_0002u32, 0x0847_ffff, 0x0001_0000];
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let e = Expr::parse(&Section::new(&bytes, 0), 0).unwrap();
    assert_eq!(e.instrs[0].operands, Operands::Constant(vec![0xffff]));
    assert_eq!(e.instrs[1].operands, Operands::RefSlot(0, 1));
    assert_eq!(e.end, 12);
}

#[test]
fn embedded_jpeg_needs_a_length_that_fits_and_a_jpeg_signature() {
    let jpeg = u32::from_le_bytes([0xff, 0xd8, 0xff, 0xe0]);
    // section: unused word, {length 4, jpeg}, the other embedded form (no length, no JPEG, flags 0x0518),
    // then the root: no outputs, no inputs, an empty node table
    let root = 20;
    let mut section = vec![0, 4, jpeg, 0x1a40_f340, 0xce40_1d80];
    section.extend([0, 0, root + 24, root + 24, root, 0]);
    let asm = Assembly::parse(&file(&section, root)).unwrap();
    assert_eq!(asm.embedded_jpeg(4), Some(&jpeg.to_le_bytes()[..]));
    assert_eq!(asm.embedded_jpeg(12), None, "a huge length runs past the end");
    assert_eq!(asm.embedded_jpeg(0), None, "length 0 has no signature");
    assert_eq!(asm.embedded_jpeg(u32::MAX - 1), None, "offsets past the end are refused");
}

/// A section whose root (no outputs, inputs or nodes) follows `words`, for reading records and expressions from.
fn section_of(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn sample_has_a_filter_only_when_bit_2_of_the_type_is_set() {
    // constant, sample(%0, image 3) without a filter, constant the next token reads as its own instruction
    let no_filter = [0x0d00_0003u32, 0x3f80_0000, 0x0000_0933, 0x0003, 0x0000_0d00, 0];
    let e = Expr::parse(&Section::new(&section_of(&no_filter), 0), 0).unwrap();
    assert_eq!(e.instrs[1].operands, Operands::Sample(0, 3, 0));
    assert_eq!(e.instrs.len(), 3);
    // constant, sample(%0, image 3, filter 1), then an add of the two
    let with_filter = [0x0d00_0003u32, 0x3f80_0000, 0x0000_0d33, 0x0001_0003, 0x0000_0912, 0x0000_0001];
    let e = Expr::parse(&Section::new(&section_of(&with_filter), 0), 0).unwrap();
    assert_eq!(e.instrs[1].operands, Operands::Sample(0, 3, 1));
    assert_eq!(e.instrs[2].operands, Operands::Refs(vec![0, 1]));
}

mod quadrants {
    use super::super::fxmap::{Children, Record};
    use super::super::{Section, Value};
    use super::section_of;

    fn floats(v: &[f32]) -> Vec<u32> {
        v.iter().map(|f| f.to_bits()).collect()
    }

    #[test]
    fn inline_slots_have_their_measured_sizes() {
        // slot 1 one word, 2 two, 3 two, 4 two, 5 one, 6 one, each with code 2
        let tag = 0x2aa8_0048 | (0x0a << 8);
        let mut w = vec![tag, 8];
        w.extend(floats(&[3.0, 0.25, 0.5, 0.1, 0.2, 1.5, 2.5, 0.125, 0.75]));
        let r = Record::parse(&Section::new(&section_of(&w), 0), 0).unwrap();
        let Record::Quadrant { pattern, draws, image, params, children, .. } = r else { panic!() };
        assert_eq!((pattern, draws, image), (10, true, false));
        assert_eq!(children, Children::None);
        let got: Vec<(u8, Vec<f32>)> = params.iter().map(|(k, v)| (*k, v.floats().unwrap())).collect();
        let want = vec![(1, vec![3.0]), (2, vec![0.25, 0.5]), (3, vec![0.1, 0.2]), (4, vec![1.5, 2.5]), (5, vec![0.125]), (6, vec![0.75])];
        assert_eq!(got, want);
    }

    #[test]
    fn slot_1_code_1_is_a_flag_without_data() {
        // slot 1 code 1, slot 6 code 2: the array holds only slot 6's word
        let w = vec![0x2004_0048, 8, 0.5f32.to_bits()];
        let r = Record::parse(&Section::new(&section_of(&w), 0), 0).unwrap();
        let Record::Quadrant { flag1, params, .. } = r else { panic!() };
        assert!(flag1);
        assert_eq!(params, vec![(6, Value::Const(vec![0.5f32.to_bits()]))]);
    }

    #[test]
    fn an_image_quadrant_has_a_word_before_its_array() {
        // bit 7, input 2, slot 6 code 2, slot 7 code 2: image word, array pointer, array
        let w = vec![0xa000_0288, 4, 12, 0.5f32.to_bits()];
        let r = Record::parse(&Section::new(&section_of(&w), 0), 0).unwrap();
        let Record::Quadrant { pattern, image, image_word, params, .. } = r else { panic!() };
        assert_eq!((pattern, image, image_word), (2, true, Some(4)));
        assert_eq!(params, vec![(6, Value::Const(vec![0.5f32.to_bits()]))]);
    }

    #[test]
    fn slot_7_code_3_and_image_word_bit_1_each_add_a_pointer() {
        // expression (one float constant), shared by both pointers
        let e = [0x0d00_0001u32, 0x3f80_0000];
        // word 0: tag; 1: image word 2; 2: array pointer; 3, 4: array; 5, 6: the expression
        let w = vec![0xc000_0088, 2, 12, 20, 20, e[0], e[1]];
        let r = Record::parse(&Section::new(&section_of(&w), 0), 0).unwrap();
        let Record::Quadrant { params, .. } = r else { panic!() };
        let slots: Vec<u8> = params.iter().map(|(k, _)| *k).collect();
        assert_eq!(slots, vec![7, 8]);
    }

    #[test]
    fn children_follow_the_image_word_and_the_array_pointer() {
        // shared child at word 5; slot 7 code 2 with no other parameters still has an array pointer
        let w = vec![0x8000_0088 | (2 << 16), 4, 16, 20, 0, 0x0000_0048];
        let r = Record::parse(&Section::new(&section_of(&w), 0), 0).unwrap();
        let Record::Quadrant { children, .. } = r else { panic!() };
        assert!(matches!(children, Children::Shared(_)));
    }
}

mod operations {
    use super::super::atomic::{Kind, Value, decode};
    use super::super::node::NodeRecord;
    use super::super::section::Section;
    use super::section_of;

    fn record(opcode: u8, flags: u16, body: &[u32]) -> NodeRecord {
        NodeRecord { index: 0, offset: 0, end: 4 + 4 * body.len() as u32, opcode, variant: 0x77, flags, body: body.to_vec() }
    }

    #[test]
    fn distance_has_a_source_input_when_bit_0_of_the_mode_is_set() {
        let s = section_of(&[]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x2a, 0x18, &[6, 3, 0x4200_0000]), &s).unwrap();
        assert_eq!((n.kind, n.inputs.clone(), n.variant), (Kind::Distance, vec![3], 0x77));
        assert_eq!(n.param("distance").and_then(Value::floats), Some(vec![32.0]));
        let n = decode(&record(0x2b, 0x18, &[5, 3, 4, 0x4380_0000]), &s).unwrap();
        assert_eq!((n.color, n.inputs.clone()), (true, vec![3, 4]));
        assert_eq!(decode(&record(0x2a, 0x18, &[2, 3]), &s).unwrap().param("distance"), Some(&Value::Default));
    }

    #[test]
    fn only_a_colour_jpeg_with_bit_8_has_an_alpha_word() {
        let s = section_of(&[]);
        let s = Section::new(&s, 0);
        // JPEG (bit 11) with alpha (bit 8): the image and its alpha (ivy_branch)
        let n = decode(&record(0x21, 0x908, &[4, 0x6994]), &s).unwrap();
        assert_eq!((n.kind, n.mode, n.param("alpha")), (Kind::Input, Some(4), Some(&Value::Const(vec![0x6994]))));
        // uncompressed RGBA 16 bits (0x07xx): one word, the alpha is in the samples (Swirls' normal)
        let n = decode(&record(0x21, 0x718, &[0x140_0004]), &s).unwrap();
        assert_eq!((n.kind, n.color, n.mode, n.param("alpha")), (Kind::Input, true, Some(0x140_0004), None));
    }

    #[test]
    fn a_curve_reads_its_keys_through_the_pointer() {
        let one = 1.0f32.to_bits();
        // the count, then point, left and right of two keys
        let s = section_of(&[2, 0, 0, 0, 0, 0, one, one, one, 0, one, one, one, one]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x2c, 0x218, &[7, 2, 0]), &s).unwrap();
        assert_eq!((n.kind, n.inputs.clone(), n.mode), (Kind::Curve, vec![7], Some(1)));
        assert_eq!((n.curve[1].point, n.curve[1].left), ([1.0, 1.0], [0.0, 1.0]));
    }

    #[test]
    fn normal_codes_intensity_and_y_direction_separately() {
        let s = section_of(&[]);
        let s = Section::new(&s, 0);
        // code 1 for the direction has no data of its own
        let n = decode(&record(0x25, 0x18, &[0x15, 3, 0x4000_0000]), &s).unwrap();
        assert_eq!(n.param("intensity").and_then(Value::floats), Some(vec![2.0]));
        assert_eq!(n.mode, Some(0x15));
    }

    #[test]
    fn emboss_codes_start_at_bit_1() {
        let s = section_of(&[]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x10, 0x18, &[0xa, 3, 4, 0x4000_0000, 0x3e80_0000]), &s).unwrap();
        assert_eq!(n.inputs, vec![3, 4]);
        assert_eq!(n.param("angle").and_then(Value::floats), Some(vec![0.25]));
    }

    #[test]
    fn gradient_dynamic_keeps_its_axis_from_the_flags() {
        let s = section_of(&[]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x27, 0x0a18, &[1, 2, 0x3e80_0000]), &s).unwrap();
        assert_eq!((n.kind, n.mode, n.color), (Kind::GradientDynamic, Some(1), true));
        assert_eq!(n.param("position").and_then(Value::floats), Some(vec![0.25]));
    }

    #[test]
    fn a_gradient_warp_may_start_with_a_word_of_zero() {
        let s = section_of(&[]);
        let s = Section::new(&s, 0);
        let mut r = record(0x0e, 0x18, &[0, 3, 4]);
        r.index = 9;
        assert_eq!(decode(&r, &s).unwrap().inputs, vec![3, 4]);
        // two inputs that are node 0, then the next word is no node index
        let mut r = record(0x0e, 0x18, &[0, 0, 0x3a83_126f]);
        r.index = 9;
        assert_eq!(decode(&r, &s).unwrap().inputs, vec![0, 0]);
    }

    #[test]
    fn gradient_keys_are_floats_when_flags_bits_9_and_10_are_set() {
        let (zero, half, one) = (0u32, 0.5f32.to_bits(), 1.0f32.to_bits());
        let s = section_of(&[zero, one, half, half, one, one, half, zero, zero, one]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x01, 0x618, &[3, 2, half, 0]), &s).unwrap();
        assert_eq!(n.keys.len(), 2);
        assert_eq!((n.keys[0].position, n.keys[0].color), (0.0, [1.0, 0.5, 0.5, 1.0]));
        assert_eq!((n.keys[1].position, n.keys[1].color), (1.0, [0.5, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn float_keys_with_flags_bit_9_alone_follow_at_once_and_carry_a_float_midpoint() {
        // flags bits 8 and 9: no extra word before the keys pointer; grey keys are position, value, midpoint (-1)
        let (zero, half, one, minus) = (0u32, 0.5f32.to_bits(), 1.0f32.to_bits(), (-1.0f32).to_bits());
        let s = section_of(&[zero, one, minus, one, half, minus]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x00, 0x318, &[3, 2, 0]), &s).unwrap();
        assert_eq!(n.keys.len(), 2);
        assert_eq!((n.keys[0].position, n.keys[0].color), (0.0, [1.0, 1.0, 1.0, 1.0]));
        assert_eq!((n.keys[1].position, n.keys[1].color), (1.0, [0.5, 0.5, 0.5, 1.0]));
        // a colour key is position, RGBA, midpoint
        let s = section_of(&[zero, half, half, half, one, minus, one, one, zero, zero, one, minus]);
        let s = Section::new(&s, 0);
        let n = decode(&record(0x01, 0x318, &[3, 2, 0]), &s).unwrap();
        assert_eq!((n.keys[1].position, n.keys[1].color), (1.0, [1.0, 0.0, 0.0, 1.0]));
    }
}

mod records {
    use super::super::atomic::{self, Kind};
    use super::super::node::NodeRecord;
    use super::*;

    fn decode(opcode: u8, flags: u16, body: Vec<u32>, data: &[u32]) -> atomic::AtomicNode {
        let bytes: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
        let rec = NodeRecord { index: 0, offset: 0, end: 1000, opcode, variant: 0x88, flags, body };
        atomic::decode(&rec, &Section::new(&bytes, 0)).unwrap()
    }

    #[test]
    fn a_switch_without_an_expression_has_its_condition_in_bit_6() {
        use super::super::fxmap::{Count, Record};
        for (tag, want) in [(0x09u32, 0), (0x49, 1)] {
            let bytes: Vec<u8> = [tag, 0, 0].iter().flat_map(|w| w.to_le_bytes()).collect();
            let Record::Switch { condition, .. } = Record::parse(&Section::new(&bytes, 0), 0).unwrap() else { panic!() };
            assert_eq!(condition, Count::Const(want));
        }
    }

    #[test]
    fn a_colour_fx_map_background_is_four_words() {
        // mode: p0 code 1, p1 code 1: four words, then one
        let n = decode(0x09, 0x0000, vec![5, 0, 1, 2, 3, 4, 5], &[0x0000_0008]);
        assert_eq!(n.param("p0"), Some(&atomic::Value::Const(vec![1, 2, 3, 4])));
        assert_eq!(n.param("p1"), Some(&atomic::Value::Const(vec![5])));
    }

    #[test]
    fn a_fx_map_background_with_code_3_is_a_node_to_draw_on() {
        // mode 3: p0 has code 3, its word is a node index. Program: an empty quadrant right after the record.
        let n = decode(0x08, 0x0000, vec![3, 0, 7], &[0x0000_0008]);
        assert!(matches!(n.param("p0"), Some(atomic::Value::Image(7))), "{:?}", n.param("p0"));
    }

    #[test]
    fn the_high_half_of_a_pixel_processors_count_is_what_it_leaves_as_a_value() {
        // no images, 4 words of values, then the function
        let function = [0x0900_0001u32, 0x3f80_0000];
        let n = decode(0x28, 0x0000, vec![0x0004_0000, 0], &function);
        assert_eq!(n.kind, Kind::PixelProcessor);
        assert_eq!((n.inputs.len(), n.value_words), (0, 4));
        assert!(n.function.is_some());
        // one image, no values
        let n = decode(0x28, 0x0000, vec![1, 7, 0], &function);
        assert_eq!((n.inputs.as_slice(), n.value_words), ([7].as_slice(), 0));
    }

    #[test]
    fn a_format_word_comes_before_seed_and_size_on_any_operation() {
        // a transform with flags bit 6: mode word, input, format word, then the matrix inline
        let n = decode(0x04, 0x0040, vec![0x40 | 0x3f, 3, 2, 1, 2, 3, 4], &[]);
        assert_eq!(n.mode, Some(0x7f), "the mode word stays");
        assert_eq!(n.seed.as_ref().and_then(|e| e.as_constant()), Some([2u32].as_slice()), "the word is the seed");
        assert_eq!(n.param("matrix"), Some(&atomic::Value::Const(vec![1, 2, 3, 4])));
    }

    #[test]
    fn gradient_keys_have_midpoints_only_with_flags_bit_8() {
        // (pos, value) pairs: 0x0000,0xffff and 0x8000,0x0000
        let n = decode(0x00, 0x0000, vec![5, 2, 0], &[0xffff_0000, 0x0000_8000]);
        assert_eq!(n.keys.len(), 2);
        assert!((n.keys[1].position - 0.5).abs() < 1e-3);
        assert_eq!(n.keys[1].color[0], 0.0);
        // (pos, value, mid): the second key starts 6 bytes in
        let n = decode(0x00, 0x0100, vec![5, 2, 0], &[0xffff_0000, 0x8000_1234, 0x0000_ffff]);
        assert!((n.keys[1].position - 0x8000 as f32 / 65535.0).abs() < 1e-5, "{}", n.keys[1].position);
        assert_eq!(n.keys[1].color[0], 1.0);
    }

    #[test]
    fn colour_gradient_keys_are_six_or_eight_bytes() {
        // position 0, RGBA 1 2 3 4 and position 0x8000, RGBA 255 0 0 255 (six bytes each)
        let n = decode(0x01, 0x0000, vec![5, 2, 0], &[0x0201_0000, 0x8000_0403, 0xff00_00ff]);
        assert_eq!(n.keys[1].color, [1.0, 0.0, 0.0, 1.0]);
        assert!((n.keys[1].position - 0x8000 as f32 / 65535.0).abs() < 1e-5);
    }

    #[test]
    fn sampling_a_colour_can_carry_a_filter_like_sampling_a_luminance() {
        // %0 pos, %1 samplecol (2 operands), %2 samplecol with a filter (3), %3 const 1: all four must parse in step
        let mut b = Vec::new();
        for h in [4u16, 0x0541, 8, 0x0934, 0, 0, 0x0d34, 0, 0, 1] {
            b.extend(h.to_le_bytes());
        }
        let mut b2 = b[..b.len()].to_vec();
        b2.extend(0x0900u16.to_le_bytes());
        while b2.len() % 4 != 0 {
            b2.push(0);
        }
        b2.extend(1.0f32.to_le_bytes());
        let e = Expr::parse(&Section::new(&b2, 0), 0).unwrap();
        assert_eq!(e.instrs[2].operands, Operands::Sample(0, 0, 1));
        assert_eq!(e.instrs[3].operands, Operands::Constant(vec![0x3f80_0000]));
    }
}
