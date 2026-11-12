use super::transcode::*;
use super::*;
use crate::compression::Compression;
use crate::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use crate::header::ColorMode;
use crate::testgen;

/// Keys the generator plants that the transcoder (rightly) does not know.
const UNKNOWN: [&[u8; 4]; 3] = [b"zOdd", b"zNop", b"Txt2"];

/// photocraft#6607: moving the layers out of a PSD writes the bytes copying them does, and leaves the
/// rest of the file (its merged image) as it was. The big-endian body is reserved at its exact
/// length, so the buffer is never grown (a copy of every layer's pixels).
#[test]
fn taking_the_layers_writes_the_same_bytes_as_copying_them() {
    for case in testgen::all_cases() {
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let copied = ImageSourceData::from_psd(&case.file).to_bytes(order);
            let mut file = case.file.clone();
            let taken = ImageSourceData::take_from_psd(&mut file).into_bytes(order);
            match (copied, taken) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{} {order:?}", case.name);
                    if !order.is_little() {
                        assert_eq!(b.0.capacity(), b.0.len(), "{}: the body grew", case.name);
                    }
                }
                (a, b) => assert_eq!(a.is_err(), b.is_err(), "{} {order:?}", case.name),
            }
            assert!(file.layer_info.is_none() && file.global_blocks.is_empty() && file.global_layer_mask.is_none(), "{}", case.name);
            assert_eq!(file.image_data, case.file.image_data, "{}", case.name);
            assert_eq!(file.header, case.file.header, "{}", case.name);
        }
    }
}

/// `d` without the blocks a little-endian round trip drops, and without the layer-info
/// padding (which follows the body length, so it changes when blocks are dropped).
fn known(d: &ImageSourceData) -> ImageSourceData {
    let mut d = d.clone();
    d.byte_order = ByteOrder::Big;
    d.global_blocks.retain(|b| !UNKNOWN.contains(&&b.key));
    if d.layer_info.is_none() {
        d.layer_info_key = *b"Layr";
    }
    if let Some(info) = &mut d.layer_info {
        info.padding = None;
        for l in &mut info.layers {
            l.blocks.retain(|b| !UNKNOWN.contains(&&b.key));
        }
    }
    d
}

fn roundtrip(d: &ImageSourceData) -> ImageSourceData {
    let (big, w) = d.to_bytes(ByteOrder::Big).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let (little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert!(w.iter().all(|w| UNKNOWN.iter().any(|k| w.starts_with(std::str::from_utf8(*k).unwrap()))), "{w:?}");
    if w.is_empty() {
        assert_eq!(big.len(), little.len());
    }
    if big.len() > 36 {
        assert_ne!(big, little);
        assert!(little[36..].starts_with(b"MIB8"));
    }
    let (back_big, w) = ImageSourceData::from_bytes(&big).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let (back_little, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    if little.len() > 36 {
        assert_eq!(back_little.byte_order, ByteOrder::Little);
    }
    assert_eq!(known(&back_little), known(&back_big));
    // The little-endian bytes are a fixed point (from the second generation on when blocks
    // were dropped, since the layer-info padding then follows the shorter body).
    let (little2, _) = back_little.to_bytes(ByteOrder::Little).unwrap();
    let little3 = ImageSourceData::from_bytes(&little2).unwrap().0.to_bytes(ByteOrder::Little).unwrap().0;
    assert_eq!(little3, little2);
    back_big
}

#[test]
fn generated_files_survive_both_orders() {
    for case in testgen::all_cases() {
        let d = ImageSourceData::from_psd(&case.file);
        let back = roundtrip(&d);
        assert_eq!(back.version, d.version, "{}", case.name);
        assert_eq!(known(&back), known(&d), "{}", case.name);
        // Nothing but the planted unknown blocks is lost on the little-endian path.
        let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
        let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
        assert_eq!(known(&back), known(&d), "{}", case.name);
    }
}

#[test]
fn same_order_transcoding_is_the_identity() {
    for case in testgen::all_cases() {
        let d = ImageSourceData::from_psd(&case.file);
        let (big, _) = d.to_bytes(ByteOrder::Big).unwrap();
        let (same, w) = transcode_blocks(&big[36..], d.version, ByteOrder::Big, ByteOrder::Big).unwrap();
        assert!(w.is_empty());
        assert_eq!(same, big[36..].to_vec(), "{}", case.name);
    }
}

fn sample_layers() -> ImageSourceData {
    let file = testgen::layered(Version::Psd, ColorMode::Rgb, 8, Compression::Rle);
    ImageSourceData::from_psd(&file)
}

#[test]
fn signature_selects_the_version() {
    let mut d = sample_layers();
    let (b, _) = d.to_bytes(ByteOrder::Big).unwrap();
    assert!(b.starts_with(SIGNATURE));
    d.version = Version::Psb;
    let (b, _) = d.to_bytes(ByteOrder::Big).unwrap();
    assert!(b.starts_with(SIGNATURE_PSB));
    assert_eq!(ImageSourceData::from_bytes(&b).unwrap().0.version, Version::Psb);
    roundtrip(&d);
}

#[test]
fn layer_key_follows_depth() {
    assert_eq!(layer_info_key(8), *b"Layr");
    assert_eq!(layer_info_key(16), *b"Lr16");
    assert_eq!(layer_info_key(32), *b"Lr32");
    for (depth, comp) in [(16, Compression::ZipPrediction), (32, Compression::ZipPrediction), (16, Compression::Rle)] {
        let file = testgen::layered(Version::Psd, ColorMode::Rgb, depth, comp);
        let d = ImageSourceData::from_psd(&file);
        assert_eq!(d.layer_info_key, layer_info_key(depth));
        let back = roundtrip(&d);
        assert_eq!(back.layer_info_key, d.layer_info_key);
        // The 16/32-bit model goes back into a PSD as a global block, like Photoshop's.
        let psd = back.into_psd(file.header.clone(), file.resources.clone(), file.image_data.clone());
        assert!(matches!(psd.layer_info_placement, LayerInfoPlacement::GlobalBlock { .. }));
        let again = PsdFile::from_bytes(&psd.to_bytes().unwrap()).unwrap();
        assert_eq!(known(&ImageSourceData::from_psd(&again)).layer_info, known(&d).layer_info);
        let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
        let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
        assert_eq!(known(&back).layer_info, known(&d).layer_info, "depth {depth}");
    }
}

#[test]
fn blocks_are_padded_to_four() {
    let d = sample_layers();
    let (b, _) = d.to_bytes(ByteOrder::Big).unwrap();
    let mut at = 36;
    while at < b.len() {
        assert_eq!(&b[at..at + 4], b"8BIM");
        let len = u32::from_be_bytes(b[at + 8..at + 12].try_into().unwrap()) as usize;
        at += 12 + len;
        at += (4 - at % 4) % 4;
    }
    assert_eq!(at, b.len());
}

fn block_rt(key: [u8; 4], data: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    let d = ImageSourceData { global_blocks: vec![TaggedBlock::new(key, data.clone())], ..Default::default() };
    let (big, _) = d.to_bytes(ByteOrder::Big).unwrap();
    let (little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let (back, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(back.global_blocks[0].data, data);
    assert_eq!(back.to_bytes(ByteOrder::Big).unwrap().0, big);
    (big, little)
}

#[test]
fn descriptor_blocks_swap_keys_and_numbers() {
    let mut desc = Descriptor::new("null");
    desc.items.push((Id::new("Opct"), Value::UnitFloat { unit: *b"#Prc", value: 75.5 }));
    desc.items.push((Id::new("Nm  "), Value::Text(UnicodeString::new_nul("Drop Shadow"))));
    desc.items.push((Id::new("enab"), Value::Boolean(true)));
    desc.items.push((Id::new("Md  "), Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new("Mltp") }));
    desc.items.push((Id::new("list"), Value::List(vec![Value::Integer(-3), Value::Double(1.5), Value::LargeInteger(7)])));
    desc.items.push((Id::new("longKeyName"), Value::Descriptor(Descriptor::new("innr"))));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor::new(desc).to_bytes());
    let (big, little) = block_rt(*b"lfx2", data);
    // Keys are reversed and the version field swapped in the little-endian bytes.
    assert!(little[36..].starts_with(b"MIB82xfl"));
    let body_at = 48;
    assert_eq!(&little[body_at..body_at + 4], &[0, 0, 0, 0]);
    assert_eq!(&little[body_at + 4..body_at + 8], &[16, 0, 0, 0]);
    assert_eq!(&big[body_at + 4..body_at + 8], &[0, 0, 0, 16]);
    assert!(little.windows(4).any(|w| w == b"crP#"), "unit key reversed");
    assert!(little.windows(11).any(|w| w == b"longKeyName"), "string ids are not reversed");
}

#[test]
fn descriptor_references_by_identifier_and_index_keep_their_class() {
    use crate::descriptor::{Class, ReferenceItem};
    let class = || Class { name: UnicodeString::new(""), class_id: Id::new("Lyr ") };
    let desc = Descriptor::new("null")
        .with("null", Value::Reference(vec![ReferenceItem::Identifier { class: class(), id: 7 }, ReferenceItem::Index { class: class(), index: 2 }]));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor::new(desc).to_bytes());
    block_rt(*b"lfx2", data);
}

fn type_layer_data() -> Vec<u8> {
    let mut data = Vec::new();
    data.put_u16(1);
    for value in [1.5, 0.0, -0.25, 1.75, 12.5, -3.25] {
        data.put_f64(value);
    }
    data.put_u16(50);
    let text = Descriptor::new("TxLr").with("Txt ", Value::Text(UnicodeString::new_nul("Synthetic Ω 🦀")));
    data.extend(VersionedDescriptor::new(text).to_bytes());
    data.put_u16(1);
    data.extend(VersionedDescriptor::new(Descriptor::new("warp")).to_bytes());
    for value in [11, -22, 333, 4_444] {
        data.put_i32(value);
    }
    data
}

fn tagged_payload<'a>(bytes: &'a [u8], signature_and_key: &[u8; 8], little: bool) -> &'a [u8] {
    let block_at = bytes.windows(8).position(|window| window == signature_and_key).expect("tagged block");
    let length_at = block_at + 8;
    let length_bytes: [u8; 4] = bytes.get(length_at..length_at + 4).expect("tagged block length").try_into().expect("four-byte length");
    let length = if little { u32::from_le_bytes(length_bytes) } else { u32::from_be_bytes(length_bytes) };
    let data_at = length_at + 4;
    let data_end = data_at + usize::try_from(length).expect("test block length fits usize");
    bytes.get(data_at..data_end).expect("tagged block payload")
}

fn legacy_little_endian_type_block() -> Vec<u8> {
    let mut tysh = Vec::new();
    tysh.extend_from_slice(&1u16.to_le_bytes());
    for value in [1.5f64, 0.0, -0.25, 1.75, 12.5, -3.25] {
        tysh.extend_from_slice(&value.to_le_bytes());
    }
    tysh.extend_from_slice(&50u16.to_le_bytes());
    // Independently encode a little-endian `VersionedDescriptor` with a single TEXT value.
    tysh.extend_from_slice(&16u32.to_le_bytes());
    tysh.extend_from_slice(&0u32.to_le_bytes()); // empty descriptor name
    tysh.extend_from_slice(&0u32.to_le_bytes()); // four-character class id
    tysh.extend_from_slice(b"rLxT"); // little-endian four-character class id
    tysh.extend_from_slice(&1u32.to_le_bytes()); // one item
    tysh.extend_from_slice(&0u32.to_le_bytes()); // four-character key
    tysh.extend_from_slice(b" txT"); // little-endian four-character key
    tysh.extend_from_slice(b"TXET"); // little-endian OSType
    let mut units: Vec<u16> = "Synthetic Ω 🦀".encode_utf16().collect();
    units.push(0); // `new_nul`, matching the canonical fixture
    tysh.extend_from_slice(&u32::try_from(units.len()).expect("short test string").to_le_bytes());
    for unit in units {
        tysh.extend_from_slice(&unit.to_le_bytes());
    }
    tysh.extend_from_slice(&1u16.to_le_bytes());
    // Empty warp descriptor, also encoded independently in little-endian order.
    tysh.extend_from_slice(&16u32.to_le_bytes());
    tysh.extend_from_slice(&0u32.to_le_bytes()); // empty descriptor name
    tysh.extend_from_slice(&4u32.to_le_bytes()); // `warp` is a four-byte string id
    tysh.extend_from_slice(b"warp");
    tysh.extend_from_slice(&0u32.to_le_bytes()); // no items
    for value in [11i32, -22, 333, 4_444] {
        tysh.extend_from_slice(&value.to_le_bytes());
    }

    let mut image_source = super::SIGNATURE.to_vec();
    image_source.extend_from_slice(b"MIB8hSyT");
    image_source.extend_from_slice(&u32::try_from(tysh.len()).expect("short test block").to_le_bytes());
    image_source.extend_from_slice(&tysh);
    let padding = (4 - tysh.len() % 4) % 4;
    image_source.resize(image_source.len() + padding, 0);
    image_source
}

#[test]
fn type_block_write_keeps_text_descriptor_in_psd_order() {
    let canonical = type_layer_data();
    let source = ImageSourceData { global_blocks: vec![TaggedBlock::new(*b"TySh", canonical.clone())], ..Default::default() };

    let (little, warnings) = source.to_bytes(ByteOrder::Little).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let payload = tagged_payload(&little, b"MIB8hSyT", true);
    assert_eq!(payload.get(..2), Some(&[1, 0][..]), "TySh header stays in TIFF order");
    for (index, expected) in [1.5f64, 0.0, -0.25, 1.75, 12.5, -3.25].into_iter().enumerate() {
        let start = 2 + index * 8;
        let bytes: [u8; 8] = payload.get(start..start + 8).unwrap().try_into().unwrap();
        assert_eq!(f64::from_le_bytes(bytes), expected, "matrix element {index}");
    }

    let (text_descriptor, descriptor_len) = VersionedDescriptor::parse_prefix(canonical.get(52..).unwrap()).unwrap();
    let text_section_len = 2 + descriptor_len;
    assert_eq!(payload.get(50..52), Some(&50u16.to_be_bytes()[..]));
    assert_eq!(payload.get(52..56), Some(&16u32.to_be_bytes()[..]));
    assert_eq!(payload.get(50..50 + text_section_len), canonical.get(50..50 + text_section_len));
    assert!(matches!(text_descriptor.descriptor.get("Txt "), Some(Value::Text(value)) if value.to_string_lossy() == "Synthetic Ω 🦀"));
    let text_descriptor_from_output = VersionedDescriptor::parse_prefix(payload.get(52..).unwrap()).unwrap().0;
    assert!(matches!(text_descriptor_from_output.descriptor.get("Txt "), Some(Value::Text(value)) if value.to_string_lossy() == "Synthetic Ω 🦀"));

    let warp_at = 50 + text_section_len;
    assert_eq!(payload.get(warp_at..warp_at + 2), Some(&1u16.to_le_bytes()[..]));
    let (_, warp_len) = VersionedDescriptor::parse_prefix(canonical.get(warp_at + 2..).unwrap()).unwrap();
    let warp_descriptor_at = warp_at + 2;
    assert_eq!(payload.get(warp_descriptor_at..warp_descriptor_at + 4), Some(&16u32.to_le_bytes()[..]));
    assert_eq!(payload.get(warp_descriptor_at + 4..warp_descriptor_at + 8), Some(&0u32.to_le_bytes()[..]));
    assert_eq!(payload.get(warp_descriptor_at + 8..warp_descriptor_at + 12), Some(&4u32.to_le_bytes()[..]));
    assert_eq!(payload.get(warp_descriptor_at + 12..warp_descriptor_at + 16), Some(b"warp".as_slice()));
    assert_eq!(payload.get(warp_descriptor_at + 16..warp_descriptor_at + 20), Some(&0u32.to_le_bytes()[..]));
    let bounds_at = warp_descriptor_at + warp_len;
    for (index, expected) in [11i32, -22, 333, 4_444].into_iter().enumerate() {
        let start = bounds_at + index * 4;
        assert_eq!(payload.get(start..start + 4), Some(&expected.to_le_bytes()[..]), "bound {index}");
    }

    let (big, warnings) = source.to_bytes(ByteOrder::Big).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(tagged_payload(&big, b"8BIMTySh", false), canonical);

    let (reopened, warnings) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(reopened.global_blocks[0].data, canonical);
    assert_eq!(reopened.to_bytes(ByteOrder::Big).unwrap().0, big);
}

#[test]
fn legacy_little_endian_type_descriptor_still_imports() {
    let input = legacy_little_endian_type_block();
    let (reopened, warnings) = ImageSourceData::from_bytes(&input).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(reopened.global_blocks[0].data, type_layer_data());
}

#[test]
fn type_block_layout() {
    let mut data = Vec::new();
    data.put_u16(1);
    for v in [1.0, 0.0, 0.0, 1.0, 12.5, -3.0] {
        data.put_f64(v);
    }
    data.put_u16(50);
    data.extend(VersionedDescriptor::new(Descriptor::new("TxLr")).to_bytes());
    data.put_u16(1);
    data.extend(VersionedDescriptor::new(Descriptor::new("warp")).to_bytes());
    for v in [0.0, 0.0, 100.0, 20.0] {
        data.put_f64(v);
    }
    let (_, little) = block_rt(*b"TySh", data);
    assert!(little[48..].starts_with(&[1, 0]));
    assert_eq!(&little[50..58], &1.0f64.to_le_bytes());
}

#[test]
fn photoshop_type_block_switches_to_big_endian_after_its_matrix() {
    let mut data = Vec::new();
    data.put_u16(1);
    for v in [1.8054202887117468, 0.0, -0.143, 1.8063853899082987, 247.25, 2864.5] {
        data.put_f64(v);
    }
    data.put_u16(50);
    let text = Descriptor::new("TxLr").with("Txt ", Value::Text(UnicodeString::new_nul("Synthetic headline")));
    data.extend(VersionedDescriptor::new(text).to_bytes());
    data.put_u16(1);
    data.extend(VersionedDescriptor::new(Descriptor::new("warp")).to_bytes());
    for v in [0, 0, 120, 30] {
        data.put_i32(v);
    }
    let (_, mut little) = block_rt(*b"TySh", data.clone());
    // Photoshop uses big-endian text but returns to little-endian for warp/bounds.
    let (_, used) = VersionedDescriptor::parse_prefix(&data[52..]).unwrap();
    little[98..100 + used].copy_from_slice(&data[50..52 + used]);
    let (back, warnings) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!back.global_blocks[0].is_foreign_order());
    assert_eq!(back.global_blocks[0].data, data);
    // The normalized model remains writable in either order.
    for order in [ByteOrder::Big, ByteOrder::Little] {
        let (bytes, warnings) = back.to_bytes(order).unwrap();
        assert!(warnings.is_empty());
        let (again, warnings) = ImageSourceData::from_bytes(&bytes).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(again.global_blocks[0].data, data);
    }
}

#[test]
fn legacy_effects_and_adjustments() {
    // lrFX with a drop shadow and a solid fill.
    let mut fx = Vec::new();
    fx.put_u16(0);
    fx.put_u16(3);
    fx.put(b"8BIMcmnS");
    fx.put_u32(7);
    fx.put_u32(0);
    fx.put_u8(1);
    fx.put_u16(0);
    fx.put(b"8BIMdsdw");
    let mut sh = Vec::new();
    sh.put_u32(2);
    for v in [5 << 16, 75 << 16, 120 << 16, 7 << 16] {
        sh.put_i32(v);
    }
    for _ in 0..5 {
        sh.put_u16(0);
    }
    sh.put(b"8BIMmul ");
    sh.put_u8(1);
    sh.put_u8(1);
    sh.put_u8(191);
    sh.put_u16(0);
    sh.put_u16(0x1234);
    sh.put_u16(0);
    sh.put_u16(0);
    sh.put_u16(0);
    fx.put_u32(sh.len() as u32);
    fx.put(&sh);
    fx.put(b"8BIMsofi");
    let mut so = Vec::new();
    so.put_u32(2);
    so.put(b"8BIMnorm");
    so.put_u16(0);
    so.put_u16(0xffff);
    so.put_u16(0);
    so.put_u16(0);
    so.put_u16(0);
    so.put_u8(255);
    so.put_u8(1);
    so.put_u16(0);
    so.put_u16(0xffff);
    so.put_u16(0);
    so.put_u16(0);
    so.put_u16(0);
    fx.put_u32(so.len() as u32);
    fx.put(&so);
    let (_, little) = block_rt(*b"lrFX", fx);
    assert!(little.windows(8).any(|w| w == b"MIB8wdsd"), "effect keys reversed");
    assert!(little.windows(2).any(|w| w == [0x34, 0x12]), "colour component swapped");

    // levl: version + 29 records + the Lvls extension.
    let mut lv = Vec::new();
    lv.put_u16(2);
    for i in 0..29 * 5 {
        lv.put_u16(i as u16);
    }
    lv.put(b"Lvls");
    lv.put_u16(3);
    lv.put_u16(33);
    for i in 0..4 * 5 {
        lv.put_u16(1000 + i as u16);
    }
    let (_, little) = block_rt(*b"levl", lv);
    assert!(little.windows(4).any(|w| w == b"slvL"));

    // curv with the Crv extension.
    let mut cv = vec![0u8];
    cv.put_u16(1);
    cv.put_u32(0b101);
    cv.put_u16(2);
    cv.put_u16(0);
    cv.put_u16(0);
    cv.put_u16(255);
    cv.put_u16(255);
    cv.put_u16(1);
    cv.put_u16(128);
    cv.put_u16(100);
    cv.put(b"Crv ");
    cv.put_u16(4);
    cv.put_u32(1);
    cv.put_u16(2);
    cv.put_u16(1);
    cv.put_u16(7);
    cv.put_u16(9);
    block_rt(*b"curv", cv);

    for (key, data) in [
        (*b"hue2", vec![0, 2, 0, 0, 0, 10, 0, 20, 0, 30, 0, 40, 0, 50, 0, 60]),
        (*b"brit", vec![0, 10, 0, 20, 0, 30, 1, 0]),
        (*b"thrs", vec![0, 128, 0, 0]),
        (*b"post", vec![0, 4, 0, 0]),
        (*b"nvrt", vec![]),
        (
            *b"expA",
            [1u16.to_be_bytes().to_vec(), 0.5f32.to_be_bytes().to_vec(), 0.0f32.to_be_bytes().to_vec(), 1.0f32.to_be_bytes().to_vec()].concat(),
        ),
        (*b"blnc", vec![0; 20]),
        (*b"mixr", vec![0, 1, 0, 0, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0]),
        (*b"selc", vec![0, 1, 0, 0, 0, 5, 0, 6, 0, 7, 0, 8]),
        (*b"phfl", vec![0, 3, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 25, 1, 0]),
        (*b"phfl", vec![0, 2, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 0, 0, 25, 1, 0]),
        (*b"lclr", vec![0, 1, 0, 0, 0, 0, 0, 0]),
        (*b"fxrp", [1.5f64.to_be_bytes().to_vec(), 2.5f64.to_be_bytes().to_vec()].concat()),
        (*b"brst", vec![0, 0, 0, 1, 0, 0, 0, 2]),
        (*b"FMsk", vec![0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 50]),
        (*b"lsct", [3u32.to_be_bytes().to_vec(), b"8BIMpass".to_vec()].concat()),
        (*b"lspf", vec![0, 0, 0, 5]),
        (*b"lnsr", b"layr".to_vec()),
        (*b"knko", vec![1, 0, 0, 0]),
        (*b"knko", vec![2, 0, 0, 0]),
        (*b"infx", vec![1, 0, 0, 0]),
        (*b"clbl", vec![0, 0, 0, 0]),
        (*b"tsly", vec![0, 0, 0, 0]),
        (*b"lmgm", vec![1, 0, 0, 0]),
        (*b"vmgm", vec![1, 0, 0, 0]),
        (*b"iOpa", vec![128, 0, 0, 0]),
    ] {
        let (big, little) = block_rt(key, data.clone());
        if data.len() >= 2 && key != *b"nvrt" {
            assert_ne!(big, little, "{}", String::from_utf8_lossy(&key));
        }
    }
}

#[test]
fn global_layer_mask_block() {
    let d = ImageSourceData {
        global_layer_mask: Some(GlobalLayerMask { data: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 128, 0] }),
        ..Default::default()
    };
    let (little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert!(w.is_empty());
    assert!(little[36..].starts_with(b"MIB8ksML"));
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    assert_eq!(back.global_layer_mask, d.global_layer_mask);
    assert_eq!(back.global_layer_mask.unwrap().opacity(), Some(50));
}

#[test]
fn gradient_map_layout() {
    let mut g = Vec::new();
    g.put_u16(3);
    g.put_u8(0);
    g.put_u8(1);
    g.put(b"Lnr ");
    g.put_u32(2);
    g.put_u16(u16::from(b'a'));
    g.put_u16(0);
    g.put_u16(2);
    for (loc, mid) in [(0u32, 50u32), (4096, 50)] {
        g.put_u32(loc);
        g.put_u32(mid);
        g.put_u16(0);
        g.put_u16(0x1111);
        g.put_u16(0x2222);
        g.put_u16(0x3333);
        g.put_u16(0);
        g.put_u16(0);
    }
    g.put_u16(2);
    for loc in [0u32, 4096] {
        g.put_u32(loc);
        g.put_u32(50);
        g.put_u16(100);
    }
    g.put_u16(2);
    g.put_u16(1);
    g.put_u16(32);
    g.put_u16(0);
    g.put_u32(0xdead_beef);
    for _ in 0..12 {
        g.put_u16(0);
    }
    let (_, little) = block_rt(*b"grdm", g);
    assert!(little.windows(4).any(|w| w == 0xdead_beefu32.to_le_bytes()));
    assert!(little.windows(4).any(|w| w == b" rnL"));
}

#[test]
fn vector_mask_records() {
    let mut v = Vec::new();
    v.put_u32(3);
    v.put_u32(0);
    // Closed length record, then two linked knots.
    v.put_u16(0);
    v.put_u16(2);
    v.put_i16(1);
    v.put(&[0; 20]);
    for sel in [1u16, 1] {
        v.put_u16(sel);
        for i in 0..6 {
            v.put_i32((i + 1) * 0x0010_0000);
        }
    }
    let (_, little) = block_rt(*b"vmsk", v);
    assert!(little.windows(4).any(|w| w == [0, 0, 0x10, 0]), "fixed-point coordinate swapped");
}

#[test]
fn pattern_block() {
    let plane: Vec<u8> = (0..16u8).collect();
    let p = crate::patterns::PsdPattern {
        mode: 3,
        width: 4,
        height: 4,
        name: "dots".into(),
        id: "abc".into(),
        palette: None,
        depth: 8,
        channels: vec![plane.clone(), plane.clone(), plane.clone()],
        alpha: Some(plane),
    };
    let data = crate::patterns::write_pattern_block(std::slice::from_ref(&p)).unwrap();
    block_rt(*b"Patt", data.clone());
    // Little-endian pattern data parses back into the same pattern.
    let d = ImageSourceData { global_blocks: vec![TaggedBlock::new(*b"Patt", data)], ..Default::default() };
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    assert_eq!(crate::patterns::parse_pattern_block(&back.global_blocks[0].data).unwrap(), vec![p]);
}

#[test]
fn filter_effects_block() {
    use crate::filter_effects::{EffectsPlane, FilterEffects, FilterEffectsItem};
    let plane = EffectsPlane::encode(Compression::Rle, &[1, 2, 3, 4, 5, 6], 3, 2, 8).unwrap();
    let mask = EffectsPlane::encode(Compression::Raw, &[9; 6], 3, 2, 8).unwrap();
    let rect = crate::layer::Rect { top: 0, left: 0, bottom: 2, right: 3 };
    let fx = FilterEffects {
        version: 1,
        items: vec![FilterEffectsItem {
            id: "x".into(),
            version: 1,
            rect,
            depth: 8,
            max_channels: 1,
            slots: vec![Some(plane.clone()), None, Some(plane)],
            mask: Some((rect, mask)),
        }],
    };
    let data = fx.to_bytes();
    block_rt(*b"FEid", data.clone());
    let d = ImageSourceData { global_blocks: vec![TaggedBlock::new(*b"FEid", data)], ..Default::default() };
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    assert_eq!(FilterEffects::parse(&back.global_blocks[0].data).unwrap(), fx);
}

/// An external linked file (`liFE`) holds the descriptor of its link, and from version 3 its
/// bytes: both are transcoded, so the record survives a little-endian save.
#[test]
fn external_linked_files_keep_their_link_and_bytes() {
    use crate::io::{WriteExt, write_pascal};
    let link = Descriptor::new("ExternalFileLink").with("fullPath", Value::Text(crate::descriptor::UnicodeString::new_nul("/art/logo.psd")));
    let record = |version: u32| {
        let mut item = b"liFE".to_vec();
        item.put_u32(version);
        write_pascal(&mut item, b"uuid-1", 1);
        crate::descriptor::UnicodeString::new("logo.psd").write(&mut item);
        item.put(b"8BPS");
        item.put(b"8BIM");
        item.put_u64(5);
        item.put_u8(0);
        crate::descriptor::VersionedDescriptor::new(link.clone()).write(&mut item);
        if version > 3 {
            item.put_i32(2026);
            item.put(&[11, 12, 9, 30]);
            item.put_f64(15.5);
        }
        item.put_u64(5);
        item.put(b"BYTES");
        if version >= 5 {
            crate::descriptor::UnicodeString::new("child").write(&mut item);
        }
        if version >= 6 {
            item.put_f64(1.0);
        }
        if version >= 7 {
            item.put_u8(0);
        }
        let mut out = Vec::new();
        out.put_u64(item.len() as u64);
        out.put(&item);
        out.resize(out.len().div_ceil(4) * 4, 0);
        out
    };
    // Converted without a warning, and read back to the same bytes (`block_rt`).
    for version in [3, 4, 7] {
        block_rt(*b"lnkE", record(version));
    }
}

/// `FELS` (the filter masks of linked smart objects) has the `FEid` layout: it is checked and
/// byte-swapped the same way.
#[test]
fn linked_filter_masks_are_filter_effects() {
    use crate::filter_effects::{EffectsPlane, FilterEffects, FilterEffectsItem};
    let rect = crate::layer::Rect { top: 0, left: 0, bottom: 2, right: 3 };
    let mask = EffectsPlane::encode(Compression::Raw, &[9; 6], 3, 2, 8).unwrap();
    let item = FilterEffectsItem {
        id: "linked".into(),
        version: 1,
        rect,
        depth: 8,
        max_channels: 1,
        slots: vec![None, None, None],
        mask: Some((rect, mask)),
    };
    let fx = FilterEffects { version: 1, items: vec![item] };
    let data = fx.to_bytes();
    let little = |key: [u8; 4]| {
        let d = ImageSourceData { global_blocks: vec![TaggedBlock::new(key, data.clone())], ..Default::default() };
        let (bytes, w) = d.to_bytes(ByteOrder::Little).unwrap();
        assert!(w.is_empty(), "{w:?}");
        // After the 36-byte signature, the signature, key and length: the block data.
        bytes[36 + 12..].to_vec()
    };
    assert_eq!(little(*b"FELS"), little(*b"FEid"));
    block_rt(*b"FELS", data.clone());
    assert!(TaggedBlock::new(*b"FELS", data.clone()).check_structure().is_ok());
    assert!(TaggedBlock::new(*b"FELS", data[..data.len() - 1].to_vec()).check_structure().is_err());
}

/// Photoshop writes one `FEid` section per smart object: every section's lengths and
/// numbers are swapped, not only the first one's.
#[test]
fn filter_effects_block_swaps_every_section() {
    use crate::filter_effects::{EffectsPlane, FilterEffects, FilterEffectsItem};
    let rect = crate::layer::Rect { top: 0, left: 0, bottom: 2, right: 3 };
    let item = |id: &str, v: u8| FilterEffectsItem {
        id: id.into(),
        version: 1,
        rect,
        depth: 8,
        max_channels: 1,
        slots: vec![Some(EffectsPlane::encode(Compression::Raw, &[v; 6], 3, 2, 8).unwrap()), None, None],
        mask: None,
    };
    let fx = FilterEffects { version: 1, items: vec![item("first", 1), item("second-object", 2)] };
    let data = fx.to_bytes();
    let len1 = u64::from_be_bytes(data[4..12].try_into().unwrap()) as usize;
    let second = 12 + len1 + (4 - len1 % 4) % 4;
    let len2 = &data[second..second + 8];
    block_rt(*b"FEid", data.clone());
    let d = ImageSourceData { global_blocks: vec![TaggedBlock::new(*b"FEid", data.clone())], ..Default::default() };
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    // 36-byte signature, then `MIB8`, the reversed key and a u32 length: the block data.
    let body = &little[36 + 12..];
    let mut swapped = len2.to_vec();
    swapped.reverse();
    assert_eq!(&body[second..second + 8], &swapped[..], "second section's length is little-endian");
    let (back, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(FilterEffects::parse(&back.global_blocks[0].data).unwrap(), fx);
}

#[test]
fn metadata_setting_items() {
    let desc = VersionedDescriptor::new(Descriptor::new("null")).to_bytes();
    let items = vec![crate::metadata::MetadataItem::new(*b"cmls", desc)];
    block_rt(*b"shmd", crate::metadata::write_shmd(&items));
    // An unknown item cannot be converted: it is dropped on its own, the rest stays.
    let items = vec![
        crate::metadata::MetadataItem::new(*b"zzzz", vec![1, 2, 3, 4, 5, 6, 7, 8]),
        crate::metadata::MetadataItem::new(*b"mdyn", vec![0, 0, 0, 1]),
        crate::metadata::MetadataItem::new(*b"cmls", VersionedDescriptor::new(Descriptor::new("null")).to_bytes()),
    ];
    let d = ImageSourceData { global_blocks: vec![TaggedBlock::new(*b"shmd", crate::metadata::write_shmd(&items))], ..Default::default() };
    let (little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].starts_with("shmd item zzzz dropped"), "{}", w[0]);
    let (back, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(w.is_empty());
    let back_items = crate::metadata::parse_shmd(&back.global_blocks[0].data).unwrap();
    assert_eq!(back_items, items[1..].to_vec());
}

#[test]
fn unknown_descriptor_shaped_blocks_are_converted() {
    // A key the transcoder has never heard of, holding a versioned descriptor (as every
    // block Photoshop added since CS6 does): converted, not dropped.
    let mut desc = Descriptor::new("null");
    desc.items.push((Id::new("Nm  "), Value::Text(UnicodeString::new_nul("x"))));
    let data = VersionedDescriptor::new(desc.clone()).to_bytes();
    block_rt(*b"Qqqq", data);
    let mut with_version = 3u32.to_be_bytes().to_vec();
    with_version.extend(VersionedDescriptor::new(desc).to_bytes());
    block_rt(*b"Qqqq", with_version);
    // Empty data needs no conversion either.
    block_rt(*b"Mt32", Vec::new());
}

#[test]
fn linked_layer_items() {
    // One embedded file (liFD, version 7) as PhotoCraft writes it.
    let mut item = Vec::new();
    item.put(b"liFD");
    item.put_u32(7);
    item.put_u8(4);
    item.put(b"uuid");
    let units: Vec<u16> = "a.png\0".encode_utf16().collect();
    item.put_u32(units.len() as u32);
    for u in units {
        item.put_u16(u);
    }
    item.put(b"png ");
    item.put(b"8BIM");
    let bytes = [0x89, b'P', b'N', b'G', 1, 2, 3];
    item.put_u64(bytes.len() as u64);
    item.put_u8(0);
    item.put(&bytes);
    item.put_u32(0);
    item.put_f64(0.0);
    item.put_u8(0);
    let mut data = (item.len() as u64).to_be_bytes().to_vec();
    data.put(&item);
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let (_, little) = block_rt(*b"lnk2", data);
    assert!(little.windows(4).any(|w| w == b"DFil"), "item type key reversed");
    assert!(little.windows(7).any(|w| w == bytes), "file bytes untouched");
}

#[test]
fn annotation_block() {
    let mut a = Vec::new();
    a.put_u16(2);
    a.put_u16(1);
    a.put_u32(1);
    let mut item = Vec::new();
    item.put(b"txtA");
    item.put_u8(0);
    item.put_u8(1);
    item.put_u16(0);
    for v in [10, 20, 30, 40, 50, 60, 70, 80] {
        item.put_i32(v);
    }
    item.put_u16(0);
    for v in [0xffff, 0, 0, 0] {
        item.put_u16(v);
    }
    for s in [&b"me"[..], b"note", b"2026"] {
        item.put_u8(s.len() as u8);
        item.put(s);
        if (1 + s.len()) % 2 == 1 {
            item.put_u8(0);
        }
    }
    let text = [0xfe, 0xff, 0, b'h', 0, b'i'];
    let mut d = Vec::new();
    d.put(b"txtC");
    d.put_u32(text.len() as u32);
    d.put(&text);
    item.put_u32(d.len() as u32 + 4);
    item.put(&d);
    a.put_u32(item.len() as u32 + 4);
    a.put(&item);
    let (_, little) = block_rt(*b"Anno", a);
    assert!(little.windows(4).any(|w| w == b"Atxt"));
    assert!(little.windows(6).any(|w| w == text), "text kept as stored");
}

#[test]
fn unknown_blocks_kept_in_big_dropped_in_little() {
    let mut d = sample_layers();
    d.global_blocks.push(TaggedBlock::new(*b"Zzzz", vec![1, 2, 3, 4, 5]));
    let (big, w) = d.to_bytes(ByteOrder::Big).unwrap();
    assert!(w.is_empty());
    let (back, w) = ImageSourceData::from_bytes(&big).unwrap();
    assert!(w.is_empty());
    assert_eq!(back.global_blocks, d.global_blocks);
    // Big-endian data of unknown layout cannot go into a little-endian file.
    let (little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert_eq!(w.iter().filter(|w| w.starts_with("Zzzz block (5 bytes) dropped")).count(), 1, "{w:?}");
    let (back, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(w.is_empty());
    assert_eq!(back.global_blocks.iter().map(|b| b.key).collect::<Vec<_>>(), vec![*b"Patt", *b"FMsk"]);
    assert_eq!(known(&back).layer_info, known(&d).layer_info);
}

#[test]
fn unknown_blocks_from_a_little_endian_file_are_kept_with_their_byte_order() {
    let d = sample_layers();
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    // A little-endian file with a block of unknown layout, among known ones.
    let mut raw = little.clone();
    raw.extend_from_slice(b"MIB8zzzZ");
    raw.extend_from_slice(&6u32.to_le_bytes());
    raw.extend_from_slice(&[9, 8, 7, 6, 5, 4, 0, 0]);
    raw.extend_from_slice(b"MIB8diyl");
    raw.extend_from_slice(&4u32.to_le_bytes());
    raw.extend_from_slice(&7u32.to_le_bytes());
    let (back, w) = ImageSourceData::from_bytes(&raw).unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(known(&back).layer_info, known(&d).layer_info);
    let kept = back.global_blocks.iter().find(|b| &b.key == b"Zzzz").expect("kept");
    assert!(kept.is_foreign_order());
    assert_eq!(kept.signature, *b"MIB8");
    assert_eq!(kept.data, [9, 8, 7, 6, 5, 4]);
    // The known block after it was still converted.
    assert_eq!(back.global_blocks.iter().find(|b| &b.key == b"lyid").unwrap().data, 7u32.to_be_bytes());
    // Written back to a little-endian TIFF, the bytes are exactly what was read.
    let (again, w) = back.to_bytes(ByteOrder::Little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let tail = &raw[raw.len() - 18 - 16..];
    assert!(again.windows(tail.len()).any(|x| x == tail), "foreign-order block restored verbatim");
    assert_eq!(ImageSourceData::from_bytes(&again).unwrap().0.global_blocks, back.global_blocks);
    // The big-endian (PSD) model keeps the marker, so a PSD writer can tell it apart.
    let (big, _) = back.to_bytes(ByteOrder::Big).unwrap();
    assert!(big.windows(8).any(|x| x == b"MIB8Zzzz"));
    assert_eq!(ImageSourceData::from_bytes(&big).unwrap().0.global_blocks, back.global_blocks);
}

/// A PSB keeps `Mt16`/`Mt32`/`Mtrn`/`Alph` blocks of unknown layout verbatim, and their
/// length field is 8 bytes (a PSB long key). Writing them into a little-endian TIFF must
/// read that width; the old code looked the key up after reversing it for the target
/// order, which never matches `PSB_LONG_KEYS`.
#[test]
fn psb_foreign_order_long_key_roundtrips() {
    let d = ImageSourceData {
        version: Version::Psb,
        byte_order: ByteOrder::Little,
        global_blocks: vec![TaggedBlock { signature: *b"MIB8", key: *b"Mt16", data: vec![9, 8, 7, 6, 5, 4], padding: None, stored: None }],
        ..Default::default()
    };
    let (little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let (back, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(back.global_blocks, d.global_blocks, "the 8-byte length and the bytes survive");
}

#[test]
fn rle_counts_follow_the_container_and_samples_do_not() {
    let file = testgen::layered(Version::Psd, ColorMode::Rgb, 16, Compression::Rle);
    let d = ImageSourceData::from_psd(&file);
    let (big, _) = d.to_bytes(ByteOrder::Big).unwrap();
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    let d = known(&d);
    let (a, b) = (d.layer_info.as_ref().unwrap(), back.layer_info.as_ref().unwrap());
    for (la, lb) in a.layers.iter().zip(&b.layers) {
        for (ca, cb) in la.channels.iter().zip(&lb.channels) {
            assert_eq!(ca, cb);
            let (w, h) = la.channel_rect(ca.id).size().unwrap();
            assert_eq!(ca.decode(w, h, 16, Version::Psd).unwrap(), cb.decode(w, h, 16, Version::Psd).unwrap());
        }
    }
    // The encoded samples (after the count table) are identical bytes in both files, while
    // the count table itself is swapped.
    let (rec, ch) = a
        .layers
        .iter()
        .flat_map(|l| l.channels.iter().filter(|c| c.id >= -1 && c.compression == Some(Compression::Rle)).map(move |c| (l, c)))
        .find(|(l, c)| l.rect.height() > 1 && c.data.len() > l.rect.height() as usize * 2 + 8)
        .unwrap();
    let table = rec.rect.height() as usize * 2;
    let samples = &ch.data[table..];
    assert!(big.windows(samples.len()).any(|w| w == samples));
    assert!(little.windows(samples.len()).any(|w| w == samples));
    let counts = &ch.data[..table];
    let swapped: Vec<u8> = counts.chunks(2).flat_map(|c| [c[1], c[0]]).collect();
    assert!(little.windows(table + samples.len()).any(|w| w[..table] == swapped[..] && w[table..] == *samples));
}

#[test]
fn blending_ranges_are_bytes_in_either_order() {
    let mut file = testgen::layered(Version::Psd, ColorMode::Rgb, 8, Compression::Rle);
    let ranges: Vec<u8> = [0u8, 0, 255, 255].repeat(10);
    let marked: Vec<u8> = (1u8..=40).collect();
    for (i, l) in file.layer_info.as_mut().unwrap().layers.iter_mut().enumerate() {
        l.blending_ranges.data = if i == 0 { marked.clone() } else { ranges.clone() };
    }
    let d = ImageSourceData::from_psd(&file);
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    // Written verbatim (what Photoshop writes in an `II` TIFF), not swapped per u32.
    assert!(little.windows(marked.len()).any(|w| w == marked));
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    for (a, b) in d.layer_info.as_ref().unwrap().layers.iter().zip(&back.layer_info.as_ref().unwrap().layers) {
        assert_eq!(a.blending_ranges, b.blending_ranges);
    }
}

#[test]
fn psb_sized_data_uses_long_lengths_and_counts() {
    let file = testgen::layered(Version::Psb, ColorMode::Cmyk, 8, Compression::Rle);
    let d = ImageSourceData::from_psd(&file);
    assert_eq!(d.version, Version::Psb);
    let back = roundtrip(&d);
    assert_eq!(back.layer_info, d.layer_info);
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    assert!(little[36..].starts_with(b"MIB8"));
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    assert_eq!(known(&back).layer_info, known(&d).layer_info);
}

#[test]
fn resources_both_orders() {
    let res = vec![
        ImageResource::new(1005, vec![0, 72, 0, 0, 0, 1, 0, 1, 0, 72, 0, 0, 0, 1, 0, 1]),
        ImageResource::new(1037, vec![0, 0, 0, 120]),
        ImageResource::new(1039, vec![1, 2, 3]),
        ImageResource { signature: *b"8BIM", id: 1026, name: b"grp".to_vec(), data: vec![0, 1, 0, 1, 0, 0] },
    ];
    let big = resources_to_bytes(&res).unwrap();
    let (back, w) = resources_from_bytes(&big).unwrap();
    assert!(w.is_empty());
    assert_eq!(back, res);
    let (little, w) = transcode_resources(&big, ByteOrder::Big, ByteOrder::Little).unwrap();
    assert!(w.is_empty());
    assert!(little.starts_with(b"MIB8"));
    assert_eq!(little.len(), big.len());
    assert_eq!(&little[4..6], &[0xed, 3]);
    let (back, w) = resources_from_bytes(&little).unwrap();
    assert!(w.is_empty());
    assert_eq!(back, res);
    // The ICC blob is untouched by the swap.
    assert!(little.windows(3).any(|w| w == [1, 2, 3]));
    // An odd trailing pad byte is tolerated.
    let mut padded = big.clone();
    padded.push(0);
    assert_eq!(resources_from_bytes(&padded).unwrap().0, res);
}

#[test]
fn malformed_input_errors_without_panicking() {
    assert!(ImageSourceData::from_bytes(b"").is_err());
    assert!(ImageSourceData::from_bytes(b"Adobe Photoshop Document Data Block").is_err());
    let mut bad = SIGNATURE.to_vec();
    bad.extend_from_slice(b"XXXXLayr");
    assert!(ImageSourceData::from_bytes(&bad).is_err());
    let d = sample_layers();
    for order in [ByteOrder::Big, ByteOrder::Little] {
        let (bytes, _) = d.to_bytes(order).unwrap();
        for cut in (0..bytes.len()).step_by(7).chain(bytes.len().saturating_sub(8)..bytes.len()) {
            let _ = ImageSourceData::from_bytes(&bytes[..cut]);
        }
        // Bit flips in the structure (not the signature).
        for i in (36..bytes.len()).step_by(13) {
            let mut b = bytes.clone();
            b[i] ^= 0x5a;
            let _ = ImageSourceData::from_bytes(&b);
        }
    }
    // A block that claims more data than exists.
    let mut huge = SIGNATURE.to_vec();
    huge.extend_from_slice(b"8BIMluni");
    huge.extend_from_slice(&u32::MAX.to_be_bytes());
    assert!(ImageSourceData::from_bytes(&huge).is_err());
    assert!(resources_from_bytes(b"8BIM\x03\xed\x00").is_err());
}

#[test]
fn into_psd_roundtrips_through_the_psd_writer() {
    let file = testgen::layered(Version::Psd, ColorMode::Grayscale, 8, Compression::Zip);
    let d = ImageSourceData::from_psd(&file);
    let (little, _) = d.to_bytes(ByteOrder::Little).unwrap();
    let (back, _) = ImageSourceData::from_bytes(&little).unwrap();
    let psd = back.into_psd(file.header.clone(), file.resources.clone(), file.image_data.clone());
    let bytes = psd.to_bytes().unwrap();
    let again = PsdFile::from_bytes(&bytes).unwrap();
    let expected = known(&ImageSourceData::from_psd(&file));
    assert_eq!(known(&ImageSourceData::from_psd(&again)).layer_info, expected.layer_info);
    assert_eq!(again.global_blocks, expected.global_blocks);
    assert_eq!(again.layer_tree().len(), file.layer_tree().len());
}

/// Global blocks are padded as they are written, not on a copy: the bytes are those of the
/// former path (each block cloned with explicit padding to 4, written with `write_blocks`).
#[test]
fn global_blocks_written_without_copies_keep_their_bytes() {
    for version in [Version::Psd, Version::Psb] {
        let mut d =
            ImageSourceData { version, global_layer_mask: Some(GlobalLayerMask { data: vec![0, 1, 0, 2, 0, 3, 0, 4, 9] }), ..Default::default() };
        for n in 0..8u8 {
            d.global_blocks.push(TaggedBlock::new(*b"FMsk", (0..n).collect()));
        }
        let mut explicit = TaggedBlock::new(*b"FMsk", vec![1, 2, 3]);
        explicit.padding = Some(vec![]);
        d.global_blocks.push(explicit);
        let mut odd = TaggedBlock::new(*b"FMsk", vec![1, 2]);
        odd.padding = Some(vec![0, 0, 0, 0, 0]);
        d.global_blocks.push(odd);
        let mut copies = vec![padded_block(*b"LMsk", d.global_layer_mask.as_ref().unwrap().data.clone())];
        for b in &d.global_blocks {
            let mut b = b.clone();
            if b.padding.is_none() {
                b.padding = Some(vec![0; (4 - b.data.len() % 4) % 4]);
            }
            copies.push(b);
        }
        let mut body = Vec::new();
        crate::tagged::write_blocks(&mut body, &copies, version, 2).unwrap();
        let signature = if version.is_psb() { SIGNATURE_PSB } else { SIGNATURE };
        assert_eq!(d.to_bytes(ByteOrder::Big).unwrap().0, [signature, &body[..]].concat());
        let (little, _) = transcode_blocks(&body, version, ByteOrder::Big, ByteOrder::Little).unwrap();
        assert_eq!(d.to_bytes(ByteOrder::Little).unwrap().0, [signature, &little[..]].concat());
        // Converted behind the signature in the buffer it starts with: never grown (growing
        // would at least double it), so the body is not moved to make room.
        let (after, _) = transcode_blocks_after(signature, &body, version, ByteOrder::Big, ByteOrder::Little).unwrap();
        assert_eq!(after, [signature, &little[..]].concat());
        assert_eq!(after.capacity(), signature.len() + body.len());
    }
}

/// Nested structures are transcoded into one shared buffer. A block whose body fails
/// part-way leaves no partial bytes: dropped on the way out (with the enclosing lengths
/// patched as if it had never been there), or kept exactly as stored on the way in.
#[test]
fn a_nested_block_that_fails_part_way_leaves_nothing_behind() {
    let clean = sample_layers();
    let mut fx = Vec::new();
    fx.put_u32(0);
    fx.extend_from_slice(&VersionedDescriptor::new(Descriptor::new("null").with("Opct", Value::Double(1.0))).to_bytes());
    // A whole number of 4-byte units, so dropping it cannot change any padding.
    fx.resize(fx.len().next_multiple_of(4), 0);
    let mut d = clean.clone();
    d.layer_info.as_mut().unwrap().layers[0].blocks.insert(0, TaggedBlock::new(*b"lfx2", fx.clone()));
    let (mut little, w) = d.to_bytes(ByteOrder::Little).unwrap();
    assert!(!w.iter().any(|w| w.starts_with("lfx2")), "{w:?}");
    // An unknown value type after the descriptor's header: parsing fails part-way.
    let at = little.windows(4).position(|w| w == b"buod").unwrap();
    little[at..at + 4].copy_from_slice(b"????");
    let (back, w) = ImageSourceData::from_bytes(&little).unwrap();
    assert_eq!(w.iter().filter(|w| w.starts_with("lfx2 block")).count(), 1, "{w:?}");
    let kept = &back.layer_info.as_ref().unwrap().layers[0].blocks[0];
    assert!(kept.is_foreign_order());
    let stored = little.windows(4).position(|w| w == b"2xfl").unwrap() + 8;
    assert_eq!(kept.data, little[stored..stored + fx.len()]);
    // Big-endian data that fails part-way cannot go into a little-endian file: dropped.
    let mut big_bad = d.clone();
    let mut broken = fx.clone();
    let at = broken.windows(4).position(|w| w == b"doub").unwrap();
    broken[at..at + 4].copy_from_slice(b"????");
    big_bad.layer_info.as_mut().unwrap().layers[0].blocks[0].data = broken;
    let (little, w) = big_bad.to_bytes(ByteOrder::Little).unwrap();
    assert_eq!(w.iter().filter(|w| w.starts_with("lfx2 block")).count(), 1, "{w:?}");
    assert_eq!(little, clean.to_bytes(ByteOrder::Little).unwrap().0);
}
