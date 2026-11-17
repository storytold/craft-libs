use craft_image_codecs::{exr_stream::Decoder, *};
use std::io::Cursor;

fn fixture(layout: ChannelLayout, sample: SampleType, compression: ExrCompression) -> Vec<u8> {
    let n = 31 * 35 * layout.channels();
    let values: Vec<f32> = (0..n)
        .map(|i| match i % 5 {
            0 => -0.25,
            1 => 3.5,
            2 => 0.0,
            3 => 0.0001,
            _ => 0.5,
        })
        .collect();
    let image = Image::from_f32(31, 35, layout, &values).unwrap().convert(layout, sample);
    encode(&image, Format::OpenExr, &EncodeOptions { exr_compression: compression, ..Default::default() }).unwrap()
}

#[test]
fn scanline_blocks_match_whole_decoder_at_each_layout_depth_and_compression() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::F16, SampleType::F32] {
            for compression in [ExrCompression::None, ExrCompression::Rle, ExrCompression::Zip1, ExrCompression::Zip16, ExrCompression::Piz] {
                let bytes = fixture(layout, sample, compression);
                let reference = decode(&bytes).unwrap();
                let target = if layout.is_gray() { ChannelLayout::GrayA } else { ChannelLayout::Rgba };
                let reference = reference.convert(target, SampleType::F32);
                let mut stream = Decoder::new(Cursor::new(&bytes), &|| false).unwrap();
                assert_eq!(stream.info().all_half, sample == SampleType::F16);
                let row = 31 * target.channels() * 4;
                let mut actual = vec![0; reference.data().len()];
                let mut blocks = 0;
                while let Some(band) = stream.next_band(&|| false).unwrap() {
                    actual[band.y as usize * row..(band.y + band.height) as usize * row].copy_from_slice(&band.data);
                    blocks += 1;
                }
                assert_eq!(blocks, stream.info().blocks);
                assert_eq!(actual, reference.data(), "{layout:?}/{sample:?}/{compression:?}");
            }
        }
    }
}

fn change_window(bytes: &mut [u8], width: i32, height: i32) {
    let needle = b"dataWindow\0box2i\0\x10\0\0\0";
    let at = bytes.windows(needle.len()).position(|v| v == needle).unwrap() + needle.len();
    bytes[at + 8..at + 12].copy_from_slice(&(width - 1).to_le_bytes());
    bytes[at + 12..at + 16].copy_from_slice(&(height - 1).to_le_bytes());
}

#[test]
fn enormous_header_is_not_a_whole_image_allocation() {
    let mut bytes = fixture(ChannelLayout::Rgb, SampleType::F16, ExrCompression::Zip16);
    change_window(&mut bytes, 65536, 32768);
    assert!(matches!(decode(&bytes), Err(CodecError::LimitExceeded(ref message)) if message.contains("2147483648 pixels")));
    // Streaming gets as far as the now-truncated offset table, without a pixel allocation.
    assert!(matches!(Decoder::new(Cursor::new(bytes), &|| false), Err(CodecError::Malformed { .. })));
}

#[test]
fn cancellation_truncation_duplicate_offsets_and_forged_lengths_fail() {
    let bytes = fixture(ChannelLayout::Rgb, SampleType::F16, ExrCompression::Zip16);
    assert!(Decoder::new(Cursor::new(&bytes), &|| true).is_err());
    let mut decoder = Decoder::new(Cursor::new(&bytes), &|| false).unwrap();
    assert!(decoder.next_band(&|| true).is_err());
    for end in [0, 8, bytes.len() / 2] {
        assert!(Decoder::new(Cursor::new(&bytes[..end]), &|| false).and_then(|mut r| r.next_band(&|| false)).is_err());
    }
    // Locate the offset table by finding the validated header end.
    let mut input = Cursor::new(&bytes);
    let _ = exr::meta::MetaData::read_from_buffered(&mut input, false).unwrap();
    let at = input.position() as usize;
    let offset = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    let mut corrupt = bytes.clone();
    corrupt[offset + 4..offset + 8].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(Decoder::new(Cursor::new(corrupt), &|| false).unwrap().next_band(&|| false).is_err());
    let mut duplicate = bytes.clone();
    duplicate[at + 8..at + 16].copy_from_slice(&bytes[at..at + 8]);
    let mut decoder = Decoder::new(Cursor::new(duplicate), &|| false).unwrap();
    assert!(decoder.next_band(&|| false).unwrap().is_some());
    assert!(decoder.next_band(&|| false).is_err());
}

#[test]
fn hostile_metadata_is_bounded_and_mutated_inputs_never_panic() {
    let bytes = fixture(ChannelLayout::Rgb, SampleType::F16, ExrCompression::Zip16);
    let mut large = bytes.clone();
    let needle = b"channels\0chlist\0";
    let length = bytes.windows(needle.len()).position(|v| v == needle).unwrap() + needle.len();
    large[length..length + 4].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(matches!(Decoder::new(Cursor::new(large), &|| false), Err(CodecError::LimitExceeded(_))));
    let mut many = bytes[..8].to_vec();
    for _ in 0..4097 {
        many.extend_from_slice(b"z\0string\0\0\0\0\0");
    }
    many.push(0);
    assert!(matches!(Decoder::new(Cursor::new(many), &|| false), Err(CodecError::LimitExceeded(_))));
    // Small deterministic mutations cover headers, offsets and compressed payloads.
    for i in 0..256 {
        let mut corrupt = bytes.clone();
        let at = i * (bytes.len() - 1) / 255;
        corrupt[at] ^= 0xff;
        assert!(
            std::panic::catch_unwind(|| {
                if let Ok(mut decoder) = Decoder::new(Cursor::new(corrupt), &|| false) {
                    while let Ok(Some(_)) = decoder.next_band(&|| false) {}
                }
            })
            .is_ok(),
            "mutation at byte {at}"
        );
    }
}

#[test]
fn mixed_samples_and_offset_data_window_keep_coordinates_and_values() {
    use exr::prelude::{AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Layer, LayerAttributes, LineOrder, WritableImage};
    let channels = AnyChannels::sort(
        vec![
            AnyChannel::new("R", FlatSamples::F16(vec![f16::from_f32(-0.25); 31 * 35])),
            AnyChannel::new("G", FlatSamples::F32(vec![3.5; 31 * 35])),
            AnyChannel::new("B", FlatSamples::U32(vec![42; 31 * 35])),
        ]
        .into_iter()
        .collect(),
    );
    let attributes = LayerAttributes { layer_position: exr::math::Vec2(-15, 40), ..Default::default() };
    let layer = Layer::new(
        (31, 35),
        attributes,
        Encoding { compression: Compression::ZIP16, blocks: Blocks::ScanLines, line_order: LineOrder::Decreasing },
        channels,
    );
    let mut bytes = Cursor::new(Vec::new());
    exr::image::Image::from_layer(layer).write().to_buffered(&mut bytes).unwrap();
    let mut decoder = Decoder::new(bytes, &|| false).unwrap();
    assert!(!decoder.info().all_half);
    let mut rows = 0;
    while let Some(band) = decoder.next_band(&|| false).unwrap() {
        assert_eq!(band.y, rows);
        rows += band.height;
        for p in band.data.as_chunks::<16>().0 {
            let v: Vec<f32> = p.as_chunks::<4>().0.iter().map(|s| f32::from_le_bytes(*s)).collect();
            assert_eq!(v, [-0.25, 3.5, 42.0, 1.0]);
        }
    }
    assert_eq!(rows, 35);
}

#[test]
fn unsupported_variants_stay_on_the_guarded_legacy_path() {
    let bytes = fixture(ChannelLayout::Rgb, SampleType::F16, ExrCompression::Zip16);
    for flag in [0x200_u32, 0x800, 0x1000] {
        let mut changed = bytes.clone();
        changed[4..8].copy_from_slice(&(2 | flag).to_le_bytes());
        assert!(matches!(Decoder::new(Cursor::new(changed), &|| false), Err(CodecError::Unsupported { .. })));
    }
    let mut custom = bytes[..8].to_vec();
    custom.extend_from_slice(b"chromaticities\0chromaticities\0\x20\0\0\0");
    for v in [0.64_f32, 0.33, 0.3, 0.6, 0.15, 0.06, 0.3127, 0.329] {
        custom.extend_from_slice(&v.to_le_bytes());
    }
    custom.extend_from_slice(&bytes[8..]);
    assert!(matches!(Decoder::new(Cursor::new(custom), &|| false), Err(CodecError::Unsupported { .. })));
}
