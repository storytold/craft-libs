//! Diagnostic capture-gain record, measured from original Sony files without external decoder code.
//! The same tag has different offsets on other bodies/signatures. Never search for a plausible
//! array in product decoding or treat these values as automatic correction opcodes.

use crate::lens_metadata::{SonyCaptureGains, SonyCaptureGainsQ11, SonyNativeShading};
use craft_tiff::{Ifd, Value};

mod older;
pub(super) use older::{read as read_q12, read_early_rx1, read_slt_nominal};

fn encoded_record(note: &Ifd) -> Option<&[u8]> {
    let mut fields = note.entries.iter().filter(|e| e.tag == 0x9416);
    let Value::Undefined(encoded) = &fields.next()?.value else { return None };
    // Exact observed layout only. Duplicates and truncated/extended blocks cannot borrow offsets.
    (fields.next().is_none() && encoded.len() == 4416).then_some(encoded)
}

pub(super) fn read(note: &Ifd) -> Option<SonyCaptureGains> {
    let encoded = encoded_record(note)?;
    // Fixed, bounded offsets measured from original files. Never scan for plausible gains:
    // the 16-word layouts put unrelated controls where newer records have more native words.
    let plain: Vec<u8> = encoded[..2272].iter().map(|&b| crate::arw::DECIPHER[b as usize]).collect();
    let (start, native, stored_samples) = match plain.get(..4)? {
        [2, 0, 212, 14] => (1553, 2191, 16),
        [3, 0, 214, 14] => (1553, 2193, 16),
        [6, 0, 22, 15] => (1563, 2203, 32),
        [7, 0, 24, 15] => (1564, 2204, 32),
        _ => return None,
    };
    let u16_at = |p: usize| u16::from_le_bytes([plain[p], plain[p + 1]]);
    let gains_q16: [[u32; 32]; 4] = std::array::from_fn(|plane| {
        std::array::from_fn(|knot| {
            let p = start + 128 * plane + 4 * knot;
            u32::from_le_bytes([plain[p], plain[p + 1], plain[p + 2], plain[p + 3]])
        })
    });
    let radius_parameters: [u16; 4] = std::array::from_fn(|i| u16_at(start + 512 + 2 * i));
    let centers: [[u16; 2]; 4] = std::array::from_fn(|i| [u16_at(start + 520 + 2 * i), u16_at(start + 528 + 2 * i)]);
    let marker = native + if stored_samples == 32 { 66 } else { 32 };
    let marker = [plain[marker], plain[marker + 1]];
    // Native 6192 × 4128 APS-C files use a sentinel in place of
    // the numeric radius parameter. Retain it unchanged for validation.
    let aps_c_radius =
        stored_samples == 32 && radius_parameters == [0xfffa; 4] && centers == [[3122, 2084]; 4] && matches!(marker, [1, 1] | [2, 2] | [2, 3]);
    // Restrict diagnostics to the observed envelope. Unknown layouts stay absent rather than
    // presenting arbitrary bytes as usable gains. These bounds do not authorize pixel changes.
    if gains_q16.iter().any(|g| g[0] != 65536 || g.iter().any(|&v| !(65536..=1048576).contains(&v)))
        || (!aps_c_radius && radius_parameters.iter().any(|&v| !(8..=1024).contains(&v)))
        || centers.iter().flatten().any(|&v| !(256..=16000).contains(&v))
    {
        return None;
    }
    let native_shading = Some(SonyNativeShading {
        stored_samples,
        values: std::array::from_fn(|i| if i < usize::from(stored_samples) { u16_at(native + 2 * i) as i16 } else { 0 }),
        endpoint: if stored_samples == 32 { u16_at(native + 64) as i16 } else { 0 },
        marker,
    });
    Some(SonyCaptureGains { gains_q16, radius_parameters, centers, native_shading })
}

pub(super) fn read_q11(note: &Ifd) -> Option<SonyCaptureGainsQ11> {
    let encoded = encoded_record(note)?;
    let decode = |v: u8| crate::arw::DECIPHER[v as usize];
    let layout_offset = match (*encoded.first_chunk::<4>()?).map(decode) {
        [8, 128, 90, 13] => 0, // A7 V
        [9, 128, 98, 13] => 8, // A7R VI
        _ => return None,
    };
    let encoded = &encoded[layout_offset..];
    let word = |p: usize| u16::from_le_bytes([decode(encoded[p]), decode(encoded[p + 1])]);
    let recorded: [[u16; 34]; 3] = std::array::from_fn(|plane| std::array::from_fn(|i| word(1483 + 68 * plane + 2 * i)));
    let center = [word(1687), word(1699)];
    let marker = [decode(encoded[1861]), decode(encoded[1862])];
    if !matches!(marker, [1, 1] | [2, 2])
        || center.iter().any(|&v| !(256..=16000).contains(&v))
        || (0..6).any(|i| word(1687 + 2 * i) != center[0])
        || (0..3).any(|i| word(1699 + 2 * i) != center[1])
        || recorded.iter().any(|g| g[0] != 2048 || g[32..] != [g[31]; 2] || g.iter().any(|&v| !(2048..=32768).contains(&v)))
    {
        return None;
    }
    let gains_q11 = recorded.map(|plane| std::array::from_fn(|i| plane[i]));
    let native_shading =
        SonyNativeShading { stored_samples: 32, values: std::array::from_fn(|i| word(1795 + 2 * i) as i16), endpoint: word(1859) as i16, marker };
    Some(SonyCaptureGainsQ11 { gains_q11, center, native_shading })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::Entry;

    fn plain() -> Vec<u8> {
        let mut b = vec![0; 4416];
        b[..4].copy_from_slice(&[6, 0, 22, 15]);
        for plane in 0..4 {
            for knot in 0..32 {
                let gain = 65536 + (plane as u32 + 1) * knot as u32 * 100;
                let p = 1563 + 128 * plane + 4 * knot;
                b[p..p + 4].copy_from_slice(&gain.to_le_bytes());
            }
            for (start, value) in [(2075, 68u16), (2083, 4332), (2091, 2892)] {
                let p = start + 2 * plane;
                b[p..p + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
        b
    }

    fn encode(b: &[u8]) -> Vec<u8> {
        b.iter().map(|&v| if v < 249 { (u32::from(v).pow(3) % 249) as u8 } else { v }).collect()
    }

    fn note(b: &[u8]) -> Ifd {
        Ifd { entries: vec![Entry { tag: 0x9416, value: Value::Undefined(encode(b)), offset: 0 }], ..Default::default() }
    }

    #[test]
    fn a7_v_q11_capture_record_keeps_three_curves_and_the_optical_slot() {
        let evidence: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-a7v-q11-optics-2026-11.json"))).unwrap();
        let fixtures = evidence["fixtures"].as_array().unwrap();
        assert_eq!(fixtures.len(), 9);
        for row in fixtures {
            let mut encoded = vec![0; 4416];
            for field in row["encoded_fields"].as_array().unwrap() {
                let start = field["offset"].as_u64().unwrap() as usize;
                for (i, pair) in field["hex"].as_str().unwrap().as_bytes().chunks_exact(2).enumerate() {
                    encoded[start + i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
                }
            }
            let expected: SonyCaptureGainsQ11 = serde_json::from_value(row["header"]["lens_metadata"]["sony_capture_gains_q11"].clone()).unwrap();
            let make_note = |bytes| Ifd { entries: vec![Entry { tag: 0x9416, value: Value::Undefined(bytes), offset: 0 }], ..Default::default() };
            let mut mn = make_note(encoded.clone());
            assert_eq!(read_q11(&mn), Some(expected));
            assert!(read(&mn).is_none());
            mn.entries.push(mn.entries[0].clone());
            assert!(read_q11(&mn).is_none(), "duplicate gain block");
            encoded[1691] ^= 1;
            assert!(read_q11(&make_note(encoded)).is_none(), "disagreeing center copies");
        }
    }

    #[test]
    fn a7r_vi_q11_layout_reads_the_shifted_curves_and_centers() {
        // Full-frame lossless, RawDB ILCE-7RM6_FF_land_RAW.ARW. The sensor
        // center follows three gain planes, each with two repeated end words.
        let gains: [u16; 32] = [
            2048, 2048, 2056, 2070, 2084, 2102, 2121, 2145, 2173, 2203, 2236, 2272, 2312, 2352, 2399, 2450, 2507, 2566, 2628, 2696, 2768, 2845, 2926,
            3012, 3104, 3198, 3304, 3412, 3542, 3685, 3850, 3850,
        ];
        let optical: [i16; 32] = [
            0, 0, 80, 208, 336, 496, 672, 880, 1120, 1376, 1648, 1936, 2240, 2544, 2880, 3232, 3600, 3968, 4336, 4720, 5104, 5488, 5872, 6256, 6640,
            7008, 7392, 7760, 8160, 8560, 8976, 9392,
        ];
        let mut bytes = vec![0; 4416];
        bytes[..4].copy_from_slice(&[9, 128, 98, 13]);
        let words = [gains; 3].into_iter().flat_map(|plane| plane.into_iter().chain([3850; 2]));
        let center = [5004u16, 3336];
        for (chunk, value) in bytes[1491..1713].chunks_exact_mut(2).zip(words.chain([center[0]; 6]).chain([center[1]; 3])) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        for (chunk, value) in bytes[1803..1869].chunks_exact_mut(2).zip(optical.into_iter().chain([9808])) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        bytes[1869..1871].copy_from_slice(&[2, 2]);
        let expected = SonyCaptureGainsQ11 {
            gains_q11: [gains; 3],
            center,
            native_shading: SonyNativeShading { stored_samples: 32, values: optical, endpoint: 9808, marker: [2, 2] },
        };
        assert_eq!(read_q11(&note(&bytes)), Some(expected));
        assert!(read(&note(&bytes)).is_none());
        bytes[1697] ^= 1;
        assert!(read_q11(&note(&bytes)).is_none(), "disagreeing center copies");
    }

    #[test]
    fn aps_c_radius_sentinel_retains_the_observed_gain_fields() {
        let evidence: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-a6700-aps-c-shading-2026-11.json")))
                .unwrap();
        let fixtures = evidence["capture_reader_fixtures"].as_array().unwrap();
        assert_eq!(fixtures.len(), 3);
        for row in fixtures {
            let mut encoded = vec![0; 4416];
            for block in row["encoded_blocks"].as_array().unwrap() {
                let start = block["offset"].as_u64().unwrap() as usize;
                for (i, digits) in block["hex"].as_str().unwrap().as_bytes().chunks_exact(2).enumerate() {
                    encoded[start + i] = u8::from_str_radix(std::str::from_utf8(digits).unwrap(), 16).unwrap();
                }
            }
            let expected: SonyCaptureGains = serde_json::from_value(row["expected"].clone()).unwrap();
            let original = Ifd { entries: vec![Entry { tag: 0x9416, value: Value::Undefined(encoded.clone()), offset: 0 }], ..Default::default() };
            assert_eq!(read(&original), Some(expected));
            // Keep the sentinel scoped to the independently observed crop center.
            encoded[2083] = 0;
            let bad = Ifd { entries: vec![Entry { tag: 0x9416, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() };
            assert!(read(&bad).is_none());
        }
    }

    #[test]
    fn original_numeric_fields_select_layout_without_a_body_allowlist() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-capture-layouts-2026-11.json")))
                .unwrap();
        for row in fixtures["records"].as_array().unwrap() {
            // Only observed numeric fields are included; no photograph or proprietary decoder code.
            let mut encoded = vec![0; 4416];
            for field in row["encoded_fields"].as_array().unwrap() {
                let at = field["offset"].as_u64().unwrap() as usize;
                for (i, pair) in field["hex"].as_str().unwrap().as_bytes().chunks_exact(2).enumerate() {
                    encoded[at + i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
                }
            }
            let note = Ifd { entries: vec![Entry { tag: 0x9416, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() };
            let expected: SonyCaptureGains = serde_json::from_value(row["expected"].clone()).unwrap();
            assert_eq!(read(&note), Some(expected), "{}", row["sample"]);
        }
    }

    #[test]
    fn keeps_each_plane_and_native_coordinates() {
        let got = read(&note(&plain())).unwrap();
        assert_eq!(got.radius_parameters, [68; 4]);
        assert_eq!(got.centers, [[4332, 2892]; 4]);
        for plane in 0..4 {
            for knot in 0..32 {
                assert_eq!(got.gains_q16[plane][knot], 65536 + (plane as u32 + 1) * knot as u32 * 100);
            }
        }
    }

    #[test]
    fn each_signature_bounds_its_native_slot_and_ignores_following_controls() {
        for (signature, start, native, count) in
            [([2, 0, 212, 14], 1553, 2191, 16), ([3, 0, 214, 14], 1553, 2193, 16), ([6, 0, 22, 15], 1563, 2203, 32), ([7, 0, 24, 15], 1564, 2204, 32)]
        {
            let mut b = vec![0; 4416];
            b[..4].copy_from_slice(&signature);
            b[start..start + 536].copy_from_slice(&plain()[1563..2099]);
            for i in 0..count {
                b[native + 2 * i..native + 2 * i + 2].copy_from_slice(&(i as i16 * 137).to_le_bytes());
            }
            let marker = native + if count == 32 { 66 } else { 32 };
            b[marker..marker + 2].copy_from_slice(&[1, 1]);
            b[marker + 2..].fill(255);
            let got = read(&note(&b)).unwrap();
            assert_eq!(got.gains_q16, read(&note(&plain())).unwrap().gains_q16);
            let n = got.native_shading.unwrap();
            assert_eq!(usize::from(n.stored_samples), count);
            assert_eq!(n.values, std::array::from_fn(|i| if i < count { i as i16 * 137 } else { 0 }));
            assert_eq!(n.endpoint, 0);
            assert_eq!(n.marker, [1, 1]);
            for len in 0..b.len() {
                assert!(read(&note(&b[..len])).is_none(), "{signature:?}, length {len}");
            }
            for offset in [start, start + 128, start + 512, start + 520, start + 528] {
                let mut bad = b.clone();
                bad[offset..offset + 4].fill(255);
                assert!(read(&note(&bad)).is_none(), "{signature:?}, offset {offset}");
            }
        }
    }

    #[test]
    fn retains_native_words_and_unknown_markers_without_guessing() {
        let mut b = plain();
        for marker in [[1, 1], [2, 2], [9, 7]] {
            for i in 0..32 {
                b[2203 + i * 2..2205 + i * 2].copy_from_slice(&(i as i16 * 137 - 20).to_le_bytes());
            }
            b[2267..2269].copy_from_slice(&(-123i16).to_le_bytes());
            b[2269..2271].copy_from_slice(&marker);
            let got = read(&note(&b)).unwrap().native_shading.unwrap();
            assert_eq!(got.values, std::array::from_fn(|i| i as i16 * 137 - 20));
            assert_eq!(got.endpoint, -123);
            assert_eq!(got.marker, marker);
        }
    }

    #[test]
    fn old_serialized_capture_records_keep_absent_native_copy() {
        let record = read(&note(&plain())).unwrap();
        let mut json = serde_json::to_value(&record).unwrap();
        assert!(json["native_shading"].get("stored_samples").is_none());
        let compatible: SonyCaptureGains = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(compatible.native_shading.unwrap().stored_samples, 32);
        json.as_object_mut().unwrap().remove("native_shading");
        let old: SonyCaptureGains = serde_json::from_value(json).unwrap();
        assert!(old.native_shading.is_none());
        assert_eq!(old.gains_q16, record.gains_q16);
        assert_eq!(serde_json::from_str::<SonyCaptureGains>(&serde_json::to_string(&record).unwrap()).unwrap(), record);
    }

    #[test]
    fn rejects_unmeasured_layouts_types_and_duplicates() {
        let b = plain();
        let mut bad = b.clone();
        bad[0] = 2;
        assert!(read(&note(&bad)).is_none());
        let mut n = note(&b);
        n.entries[0].value = Value::Byte(encode(&b));
        assert!(read(&n).is_none());
        let mut n = note(&b);
        n.entries.push(n.entries[0].clone());
        assert!(read(&n).is_none());
        assert!(read(&Ifd::default()).is_none());
    }

    #[test]
    fn rejects_every_truncation_and_out_of_envelope_values() {
        let b = plain();
        for len in 0..b.len() {
            assert!(read(&note(&b[..len])).is_none(), "length {len}");
        }
        let mut long = b.clone();
        long.push(0);
        assert!(read(&note(&long)).is_none());
        for (p, bytes) in [(1563, 0u32.to_le_bytes()), (1687, u32::MAX.to_le_bytes()), (2075, [0; 4]), (2083, [255; 4])] {
            let mut bad = b.clone();
            bad[p..p + 4].copy_from_slice(&bytes);
            assert!(read(&note(&bad)).is_none(), "offset {p}");
        }
    }

    fn fixture(order: craft_tiff::ByteOrder, include: bool) -> Vec<u8> {
        use craft_tiff::{IfdBuilder, ImageData, Tiff, TiffWriter, tags as t};
        let mut raw = IfdBuilder::new();
        raw.set(t::MAKE, Value::Ascii("SONY".into()));
        raw.set(t::MODEL, Value::Ascii("ILCE-1M2".into()));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![32]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![24]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        let mut maker = Vec::new();
        order.put_u16(&mut maker, 1);
        order.put_u16(&mut maker, if include { 0x9416 } else { 0xffff });
        order.put_u16(&mut maker, 7);
        order.put_u32(&mut maker, 4416);
        order.put_u32(&mut maker, 0); // Patch to an absolute offset once the writer locates the note.
        order.put_u32(&mut maker, 0);
        maker.extend(encode(&plain()));
        raw.set_child(t::EXIF_IFD, IfdBuilder::new().with(t::MAKER_NOTE, Value::Undefined(maker)));
        raw.set_image(ImageData::Strips { rows_per_strip: 24, strips: vec![vec![0; 32 * 24 * 2]] });
        let mut bytes = TiffWriter::new(order, false).write(&[raw]).unwrap();
        let tiff = Tiff::parse(&bytes).unwrap();
        let start = tiff.exif().unwrap().get(t::MAKER_NOTE).unwrap().offset as usize;
        let mut address = Vec::new();
        order.put_u32(&mut address, (start + 18) as u32);
        bytes[start + 10..start + 14].copy_from_slice(&address);
        bytes
    }

    #[test]
    fn header_and_full_decode_keep_diagnostics_without_changing_pixels() {
        for order in [craft_tiff::ByteOrder::Little, craft_tiff::ByteOrder::Big] {
            let file = fixture(order, true);
            let full = crate::decode(&file).unwrap();
            let header = crate::probe_info(&file).unwrap();
            assert_eq!(header.lens_metadata, full.lens_metadata);
            assert!(full.lens_metadata.sony_capture_gains.is_some());
            assert_eq!(full.data, crate::decode(&fixture(order, false)).unwrap().data);
            assert!(full.opcodes.list1.is_empty() && full.opcodes.list2.is_empty() && full.opcodes.list3.is_empty());
        }
    }
}
