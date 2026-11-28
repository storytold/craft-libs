//! JPEG 2000 files from OpenJPEG (`tests/fixtures/jp2`) against hayro-jpeg2000, the reader's 8-bit path.

use craft_image_codecs::*;

macro_rules! fixture {
    ($name:literal) => {
        ($name, include_bytes!(concat!("../fixtures/jp2/", $name)).as_slice())
    };
}

/// The sample `make_fixtures.py` writes for channel `channel` of pixel `index` at `depth` bits.
fn value(channel: usize, index: usize, depth: u32) -> u32 {
    let step = 1usize << (depth - 8);
    ((index * step + (index * 37) % step + (channel << (depth - 2))) % (1 << depth)) as u32
}

/// `value` at `depth` bits widened to the full 16-bit range, as the reader widens it.
fn widened(value: u32, depth: u32) -> u16 {
    let max = (1u32 << depth) - 1;
    ((value * 65535 + max / 2) / max) as u16
}

/// hayro-jpeg2000's interleaved 8-bit samples of `bytes`.
fn hayro_8_bit(bytes: &[u8]) -> Vec<u8> {
    let image = hayro_jpeg2000::Image::new(bytes, &hayro_jpeg2000::DecodeSettings::default()).unwrap();
    image.decode().unwrap()
}

/// The largest difference between 16-bit samples brought to 8 bits and hayro-jpeg2000's 8-bit samples.
fn max_diff_from_8_bit(samples: &[u16], eight: &[u8]) -> u32 {
    assert_eq!(samples.len(), eight.len());
    samples.iter().zip(eight).map(|(&s, &e)| ((u32::from(s) + 128) / 257).abs_diff(u32::from(e))).max().unwrap_or(0)
}

#[test]
fn deep_files_in_codestream_order_keep_their_precision_and_hayros_channels() {
    // The file, its layout, its depth and whether it is lossless
    let files = [
        (fixture!("gray-12.jp2"), ChannelLayout::Gray, 12, true),
        (fixture!("gray-12.j2k"), ChannelLayout::Gray, 12, true),
        (fixture!("gray-16.jp2"), ChannelLayout::Gray, 16, true),
        (fixture!("graya-16.jp2"), ChannelLayout::GrayA, 16, true),
        (fixture!("rgb-12.jp2"), ChannelLayout::Rgb, 12, true),
        (fixture!("rgb-16.jp2"), ChannelLayout::Rgb, 16, true),
        (fixture!("rgb-16-no-mct.jp2"), ChannelLayout::Rgb, 16, true),
        (fixture!("rgb-16-icc.jp2"), ChannelLayout::Rgb, 16, true),
        (fixture!("rgb-16-ict.jp2"), ChannelLayout::Rgb, 16, false),
        (fixture!("rgba-16.jp2"), ChannelLayout::Rgba, 16, true),
        (fixture!("rgba-16.j2k"), ChannelLayout::Rgba, 16, true),
        (fixture!("rgba-16-no-cdef.jp2"), ChannelLayout::Rgba, 16, true),
    ];
    for ((name, bytes), layout, depth, lossless) in files {
        let back = decode(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((back.dimensions(), back.layout(), back.sample_type()), ((16, 16), layout, SampleType::U16), "{name}");
        let samples = back.to_u16_samples().unwrap();

        assert!(max_diff_from_8_bit(&samples, &hayro_8_bit(bytes)) <= 1, "{name}: channels or colour differ from hayro-jpeg2000");
        if lossless {
            let channels = layout.channels();
            let expected: Vec<u16> = (0..samples.len()).map(|at| widened(value(at % channels, at / channels, depth), depth)).collect();
            assert!(samples == expected, "{name}: not the encoded {depth}-bit samples");
        }
    }
}

#[test]
fn eight_bit_files_read_as_hayro_reads_them() {
    let files = [
        (fixture!("gray-8.jp2"), ChannelLayout::Gray),
        (fixture!("graya-8.jp2"), ChannelLayout::GrayA),
        (fixture!("rgb-8.jp2"), ChannelLayout::Rgb),
        (fixture!("rgba-8.jp2"), ChannelLayout::Rgba),
    ];
    for ((name, bytes), layout) in files {
        let back = decode(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((back.layout(), back.sample_type()), (layout, SampleType::U8), "{name}");
        assert!(back.data() == hayro_8_bit(bytes), "{name}");
        let channels = layout.channels();
        assert!(back.data().iter().enumerate().all(|(at, &s)| u32::from(s) == value(at % channels, at / channels, 8)), "{name}");
    }
}

#[test]
fn deep_files_hayro_reorders_converts_or_resolves_are_read_at_8_bits_as_before() {
    let files = [
        (fixture!("rgba-16-swapped-cdef.jp2"), ChannelLayout::Rgba),
        (fixture!("rgb-16-sycc.jp2"), ChannelLayout::Rgb),
        (fixture!("palette-10.jp2"), ChannelLayout::Rgb),
        (fixture!("gray-16-signed.jp2"), ChannelLayout::Gray),
        (fixture!("rgb-16-420.jp2"), ChannelLayout::Rgb),
    ];
    for ((name, bytes), layout) in files {
        let back = decode(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((back.layout(), back.sample_type()), (layout, SampleType::U8), "{name}");
        assert!(back.data() == hayro_8_bit(bytes), "{name}");
    }
}

#[test]
fn swapped_colour_associations_put_the_associated_component_in_each_channel() {
    let (_, bytes) = fixture!("rgba-16-swapped-cdef.jp2");
    let back = decode(bytes).unwrap();

    // Codestream component 2 is associated with red and component 0 with blue
    let near = |sample: u8, channel: usize, pixel: usize| (u32::from(sample) * 257).abs_diff(value(channel, pixel, 16)) <= 257;
    for (pixel, rgba) in back.data().as_chunks::<4>().0.iter().enumerate() {
        assert!(near(rgba[0], 2, pixel) && near(rgba[1], 1, pixel) && near(rgba[2], 0, pixel) && near(rgba[3], 3, pixel), "pixel {pixel}");
    }
}

#[test]
fn opacity_defined_before_the_colours_is_still_refused() {
    let (_, bytes) = fixture!("rgba-16-alpha-first.jp2");
    assert!(decode(bytes).is_err());
}
