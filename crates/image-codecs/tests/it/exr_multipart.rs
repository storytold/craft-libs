//! Multi-part EXR (a Maya/Arnold render writes one part per AOV): the `exr` crate
//! writes synthetic files with several parts, the info/decode tests check the part
//! listing, the colour-ranked auto-pick and part-by-index decoding, and malformed
//! inputs must error, never panic.

use craft_image_codecs::f16;
use craft_image_codecs::{
    ChannelLayout, DecodeOptions, DecodeWarning, Format, Limits, SampleType, decode, decode_as_with, decode_exr_part, exr_info, probe_size,
};
use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Image, ImageAttributes, IntegerBounds, Layer, LayerAttributes, LineOrder,
    WritableImage,
};

fn layer_f32(name: &str, comp: Compression, chans: &[(&str, Vec<f32>)]) -> Layer<AnyChannels<FlatSamples>> {
    let n = chans.iter().find(|(_, v)| !v.is_empty()).map_or(0, |(_, v)| v.len());
    let list = chans
        .iter()
        .map(|(cn, v)| {
            assert_eq!(v.len(), n, "channel lengths must match");
            AnyChannel::new(*cn, FlatSamples::F32(v.clone()))
        })
        .collect::<Vec<_>>();
    let h = (n as f32).sqrt().round() as usize; // tests use square images
    Layer::new(
        (h, h),
        LayerAttributes::named(name),
        Encoding { compression: comp, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
        AnyChannels::sort(list.into_iter().collect()),
    )
}

/// Three parts in the order an AOV-heavy render might use: a depth-only `Z` part first
/// (which used to make the whole file fail with "no R/G/B or Y channels"), the `beauty`
/// RGBA part second and a `diffuse` RGB part third, each with its own compression.
fn aov_file() -> Vec<u8> {
    let n = 16;
    let z: Vec<f32> = (0..n).map(|i| i as f32).collect();
    let beauty: [Vec<f32>; 4] = std::array::from_fn(|c| {
        (0..n)
            .map(|i| match c {
                0 => i as f32 / 15.0,
                1 => 1.0 - i as f32 / 15.0,
                2 => 0.25,
                _ => 1.0,
            })
            .collect()
    });
    let diffuse: [Vec<f32>; 3] = std::array::from_fn(|c| (0..n).map(|_| [0.3, 0.6, 0.9][c]).collect());
    let layers = vec![
        layer_f32("Z", Compression::Uncompressed, &[("Z", z)]),
        layer_f32(
            "beauty",
            Compression::ZIP1,
            &[("R", beauty[0].clone()), ("G", beauty[1].clone()), ("B", beauty[2].clone()), ("A", beauty[3].clone())],
        ),
        layer_f32("diffuse", Compression::RLE, &[("R", diffuse[0].clone()), ("G", diffuse[1].clone()), ("B", diffuse[2].clone())]),
    ];
    write_image(layers)
}

fn write_image(layers: Vec<Layer<AnyChannels<FlatSamples>>>) -> Vec<u8> {
    let size: (usize, usize) = layers.first().map_or((1, 1), |l| l.size.into());
    let bounds = IntegerBounds::new((0, 0), size);
    let image = Image::from_layers(ImageAttributes::new(bounds), layers);
    let mut out = std::io::Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).expect("synthetic exr writes");
    out.into_inner()
}

fn stereo_file() -> Vec<u8> {
    let n = 4;
    let px = |k: f32| -> [Vec<f32>; 4] { std::array::from_fn(|c| (0..n).map(|i| if c == 0 { i as f32 / 3.0 * k } else { k }).collect()) };
    let left = px(0.1);
    let right = px(0.9);
    let with_view = |name: &str, view: &str, p: [Vec<f32>; 4]| {
        let attrs = LayerAttributes { view_name: Some(view.into()), ..LayerAttributes::named(name) };
        let list: Vec<_> = ["R", "G", "B", "A"].iter().enumerate().map(|(c, cn)| AnyChannel::new(*cn, FlatSamples::F32(p[c].clone()))).collect();
        Layer::new(
            (2, 2),
            attrs,
            Encoding { compression: Compression::Uncompressed, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
            AnyChannels::sort(list.into_iter().collect()),
        )
    };
    // The `exr` crate refuses duplicate part names when writing, so a stereo pair is
    // emulated with distinct names that still carry the view attribute.
    write_image(vec![with_view("beauty_left", "left", left), with_view("beauty_right", "right", right)])
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn info_lists_every_part() {
    let parts = exr_info(&aov_file(), &Limits::default()).unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts.iter().map(|p| p.name.as_deref()).collect::<Vec<_>>(), [Some("Z"), Some("beauty"), Some("diffuse")]);
    let z = &parts[0];
    assert_eq!((z.width, z.height), (4, 4));
    assert!(!z.deep && !z.tiled);
    assert_eq!(z.channels.len(), 1);
    assert_eq!(z.channels[0].name, "Z");
    assert_eq!(z.channels[0].sample, SampleType::F32);
    let beauty = &parts[1];
    // OpenEXR stores the channel list in the header alphabetically; the info shows the file as-is.
    assert_eq!(beauty.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A", "B", "G", "R"]);
    assert_eq!(beauty.channels.iter().map(|c| c.sample).collect::<Vec<_>>(), [SampleType::F32; 4]);
}

#[test]
fn auto_pick_skips_the_depth_part() {
    let img = decode(&aov_file()).unwrap();
    assert_eq!(img.layout(), ChannelLayout::Rgba);
    assert!(img.warnings.contains(&DecodeWarning::MoreParts { total: Some(3) }), "{:?}", img.warnings);
    for i in 0..16 {
        let (x, y) = (i % 4, i / 4);
        assert!(near(img.get(x, y, 0), i as f32 / 15.0), "pixel {i}");
        assert!(near(img.get(x, y, 1), 1.0 - i as f32 / 15.0), "pixel {i}");
        assert!(near(img.get(x, y, 2), 0.25), "pixel {i}");
        assert!(near(img.get(x, y, 3), 1.0), "pixel {i}");
    }
}

#[test]
fn parts_decode_by_index() {
    let bytes = aov_file();
    // The depth part opens as a grayscale image instead of failing the file.
    let z = decode_exr_part(&bytes, 0, &DecodeOptions::default()).unwrap();
    assert_eq!(z.layout(), ChannelLayout::Gray);
    for i in 0..16 {
        assert!(near(z.get(i % 4, i / 4, 0), i as f32), "pixel {i}");
    }
    let diffuse = decode_exr_part(&bytes, 2, &DecodeOptions::default()).unwrap();
    assert_eq!(diffuse.layout(), ChannelLayout::Rgb);
    assert!(near(diffuse.get(1, 2, 2), 0.9));
    // An explicit part choice does not warn about the parts left out.
    assert!(!diffuse.warnings.contains(&DecodeWarning::MoreParts { total: Some(3) }));
}

#[test]
fn part_index_out_of_range_errors() {
    let bytes = aov_file();
    let e = decode_exr_part(&bytes, 3, &DecodeOptions::default()).unwrap_err().to_string();
    assert!(e.contains("out of range") && e.contains("3 parts"), "{e}");
}

#[test]
fn views_are_listed_and_left_wins_the_auto_pick() {
    let bytes = stereo_file();
    let parts = exr_info(&bytes, &Limits::default()).unwrap();
    assert_eq!(parts.iter().map(|p| p.view.as_deref()).collect::<Vec<_>>(), [Some("left"), Some("right")]);
    let img = decode(&bytes).unwrap();
    assert!(near(img.get(0, 0, 0), 0.0) && near(img.get(3, 0, 0), 0.1), "the left view is the first part");
    assert!(near(img.get(1, 1, 1), 0.1), "left's G is 0.1 everywhere");
    let right = decode_exr_part(&bytes, 1, &DecodeOptions::default()).unwrap();
    assert!(near(right.get(1, 1, 1), 0.9), "right's G is 0.9 everywhere");
}

#[test]
fn single_part_files_do_not_warn() {
    let n = 4;
    let v: Vec<f32> = (0..n).map(|i| i as f32 / 3.0).collect();
    let bytes = write_image(vec![layer_f32("main", Compression::ZIP1, &[("Y", v)])]);
    let img = decode(&bytes).unwrap();
    assert_eq!(img.layout(), ChannelLayout::Gray);
    assert!(img.warnings.is_empty(), "{:?}", img.warnings);
}

#[test]
fn half_float_parts_stay_half() {
    let n = 4;
    let chans: [Vec<f16>; 3] = std::array::from_fn(|c| (0..n).map(|_| f16::from_f32([0.5, 0.25, 1.0][c])).collect());
    let list: Vec<_> = ["R", "G", "B"].iter().enumerate().map(|(c, cn)| AnyChannel::new(*cn, FlatSamples::F16(chans[c].clone()))).collect();
    let layer = Layer::new(
        (2, 2),
        LayerAttributes::named("half"),
        Encoding { compression: Compression::Uncompressed, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
        AnyChannels::sort(list.into_iter().collect()),
    );
    let bytes = write_image(vec![layer]);
    let img = decode_exr_part(&bytes, 0, &DecodeOptions::default()).unwrap();
    assert_eq!(img.sample_type(), SampleType::F16);
    assert!(near(img.get(1, 1, 0), 0.5));
}

#[test]
fn truncated_files_error_never_panic() {
    let bytes = aov_file();
    for end in (8..bytes.len()).step_by(5) {
        let cut = &bytes[..end];
        let _ = exr_info(cut, &Limits::default());
        let _ = decode_as_with(Format::OpenExr, cut, &DecodeOptions::default());
        let _ = decode_exr_part(cut, 1, &DecodeOptions::default());
    }
}

#[test]
fn parts_are_capped_together() {
    // Each part fits alone (largest 256 bytes) but the three together (512) do not.
    let limits = Limits { max_alloc: 300, ..Limits::default() };
    let e = exr_info(&aov_file(), &limits).unwrap_err().to_string();
    assert!(e.contains("across 3 parts"), "{e}");
}

/// Two ZIP1 parts of 4x4 RGBA; every chunk of part 0 is then overwritten with garbage, so
/// inflating it fails.
fn file_with_corrupt_first_part() -> Vec<u8> {
    let px = |k: f32| -> Vec<(&'static str, Vec<f32>)> { ["R", "G", "B", "A"].iter().map(|c| (*c, vec![k; 16])).collect() };
    let mut bytes = write_image(vec![layer_f32("first", Compression::ZIP1, &px(0.25)), layer_f32("second", Compression::ZIP1, &px(0.75))]);
    let mut cursor = std::io::Cursor::new(&bytes[..]);
    exr::meta::MetaData::read_from_buffered(&mut cursor, false).unwrap();
    let table = cursor.position() as usize;
    // Part 0 owns the first four of the eight chunks (one scan line per ZIP1 block).
    for i in 0..4 {
        let at = table + i * 8;
        let offset = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        bytes[offset + 12..offset + 12 + size].fill(0xA5);
    }
    bytes
}

#[test]
fn decoding_one_part_does_not_decode_the_others() {
    let bytes = file_with_corrupt_first_part();
    // Part 0 is unreadable ...
    assert!(decode_exr_part(&bytes, 0, &DecodeOptions::default()).is_err());
    // ... but asking for part 1 must not touch it.
    let img = decode_exr_part(&bytes, 1, &DecodeOptions::default()).expect("part 1 decodes on its own");
    assert_eq!((img.width(), img.height()), (4, 4));
    assert!(img.to_normalized().iter().all(|v| near(*v, 0.75)), "wrong pixels");
}

fn channel_group_file(channels: &[(&str, f32)]) -> Vec<u8> {
    let channels: Vec<_> = channels.iter().map(|(name, value)| (*name, vec![*value; 4])).collect();
    write_image(vec![layer_f32("groups", Compression::ZIP1, &channels)])
}

fn assert_channel_group(bytes: &[u8], layout: ChannelLayout, expected: &[f32]) {
    assert_eq!(exr_info(bytes, &Limits::default()).unwrap()[0].color_rank(), expected.len() as u8);
    assert_eq!(probe_size(bytes).unwrap(), (2, 2));
    for img in [decode(bytes).unwrap(), decode_exr_part(bytes, 0, &DecodeOptions::default()).unwrap()] {
        assert_eq!(img.layout(), layout);
        assert_eq!(img.to_normalized(), expected.repeat(4));
    }
}

#[test]
fn flat_decode_keeps_rgb_channels_in_one_group() {
    // A partial group sorts before the nested beauty layer, but cannot donate its R.
    let bytes = channel_group_file(&[("aaa.R", 9.0), ("beauty.diffuse.R", 0.25), ("beauty.diffuse.G", 0.5), ("beauty.diffuse.B", 2.0)]);
    assert_channel_group(&bytes, ChannelLayout::Rgb, &[0.25, 0.5, 2.0]);
}

#[test]
fn flat_decode_does_not_borrow_another_groups_alpha() {
    for alpha in ["A", "aaa.A", "beauty.other.A"] {
        let bytes = channel_group_file(&[(alpha, 0.125), ("beauty.R", 0.25), ("beauty.G", 0.5), ("beauty.B", 2.0)]);
        assert_channel_group(&bytes, ChannelLayout::Rgb, &[0.25, 0.5, 2.0]);
        let bytes = channel_group_file(&[(alpha, 0.125), ("beauty.Y", 0.5)]);
        assert_channel_group(&bytes, ChannelLayout::Gray, &[0.5]);
    }
}

#[test]
fn flat_decode_rejects_rgb_split_between_groups() {
    for channels in [
        [("aaa.R", 0.25), ("bbb.G", 0.5), ("ccc.B", 2.0)],
        [("beauty.R", 0.25), ("beauty.diffuse.G", 0.5), ("beauty.diffuse.B", 2.0)],
        [(".R", 0.25), ("G", 0.5), ("B", 2.0)],
    ] {
        let bytes = channel_group_file(&channels);
        assert_eq!(exr_info(&bytes, &Limits::default()).unwrap()[0].color_rank(), 0);
        assert!(probe_size(&bytes).is_err());
        assert!(decode(&bytes).is_err());
        assert!(decode_exr_part(&bytes, 0, &DecodeOptions::default()).is_err());
    }
}

#[test]
fn auto_pick_and_probe_skip_incoherent_rgb_parts() {
    let bytes = write_image(vec![
        layer_f32("split", Compression::ZIP1, &[("aaa.R", vec![9.0; 4]), ("bbb.G", vec![8.0; 4]), ("ccc.B", vec![7.0; 4])]),
        layer_f32("luminance", Compression::ZIP1, &[("Y", vec![0.5; 16]), ("A", vec![0.75; 16])]),
    ]);
    let parts = exr_info(&bytes, &Limits::default()).unwrap();
    assert_eq!(parts.iter().map(|part| part.color_rank()).collect::<Vec<_>>(), [0, 2]);
    assert_eq!(probe_size(&bytes).unwrap(), (4, 4));
    let img = decode(&bytes).unwrap();
    assert_eq!((img.width(), img.height(), img.layout()), (4, 4, ChannelLayout::GrayA));
    assert_eq!(img.to_f32_samples().unwrap(), [0.5, 0.75].repeat(16));
}

#[test]
fn flat_decode_prefers_the_highest_ranked_complete_group() {
    let bytes = channel_group_file(&[
        ("aaa.R", 9.0),
        ("aaa.G", 8.0),
        ("aaa.B", 7.0),
        ("beauty.R", 0.25),
        ("beauty.G", 0.5),
        ("beauty.B", 2.0),
        ("beauty.A", 0.75),
    ]);
    assert_channel_group(&bytes, ChannelLayout::Rgba, &[0.25, 0.5, 2.0, 0.75]);
    let bytes = channel_group_file(&[("aaa.Y", 9.0), ("beauty.Y", 0.5), ("beauty.A", 0.75)]);
    assert_channel_group(&bytes, ChannelLayout::GrayA, &[0.5, 0.75]);
}

#[test]
fn flat_decode_breaks_group_ties_by_prefix() {
    let bytes = channel_group_file(&[("z.R", 9.0), ("z.G", 8.0), ("z.B", 7.0), ("beauty.R", 0.25), ("beauty.G", 0.5), ("beauty.B", 2.0)]);
    assert_channel_group(&bytes, ChannelLayout::Rgb, &[0.25, 0.5, 2.0]);
    // Even a prefix that sorts before the unprefixed channel names loses a rank tie.
    let bytes = channel_group_file(&[("0.R", 9.0), ("0.G", 8.0), ("0.B", 7.0), ("R", 0.25), ("G", 0.5), ("B", 2.0)]);
    assert_channel_group(&bytes, ChannelLayout::Rgb, &[0.25, 0.5, 2.0]);
}

#[test]
fn flat_decode_preserves_luminance_and_lone_channel_fallbacks() {
    for prefix in ["", "beauty."] {
        let y = format!("{prefix}Y");
        let a = format!("{prefix}A");
        assert_channel_group(&channel_group_file(&[(&y, 2.0), (&a, 0.75)]), ChannelLayout::GrayA, &[2.0, 0.75]);
        assert_channel_group(&channel_group_file(&[(&y, 2.0)]), ChannelLayout::Gray, &[2.0]);
    }
    for name in ["depth.Z", "A", "beauty.A"] {
        // A lone alpha is one grayscale channel, not the same samples duplicated as YA.
        assert_channel_group(&channel_group_file(&[(name, 9.0)]), ChannelLayout::Gray, &[9.0]);
    }
}

#[test]
fn half_float_group_does_not_use_unrelated_float_channels() {
    let mut layer = layer_f32("mixed", Compression::ZIP1, &[("aaa.R", vec![9.0; 4]), ("aaa.A", vec![0.125; 4])]);
    for (name, value) in [("beauty.R", 0.25), ("beauty.G", 0.5), ("beauty.B", 2.0), ("beauty.A", 0.75)] {
        layer.channel_data.list.push(AnyChannel::new(name, FlatSamples::F16(vec![f16::from_f32(value); 4])));
    }
    layer.channel_data = AnyChannels::sort(layer.channel_data.list);
    let bytes = write_image(vec![layer]);
    assert_channel_group(&bytes, ChannelLayout::Rgba, &[0.25, 0.5, 2.0, 0.75]);
    assert_eq!(decode(&bytes).unwrap().sample_type(), SampleType::F16);
}
