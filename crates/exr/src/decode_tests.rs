//! The preview check is PhotoCraft's test; the rest builds files with the `exr` crate.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use exr::meta::attribute::{Preview, Text};
use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Encoding, FlatSamples, ImageAttributes, IntegerBounds, Layer, LayerAttributes, LineOrder, WritableImage,
};

use super::decode::*;

fn ramp(n: usize, k: f32) -> Vec<f32> {
    (0..n).map(|i| i as f32 * k).collect()
}

/// A layer of `w × h` with these channels of f32 ramps, scan lines or 4 × 4 tiles.
fn layer(
    name: Option<&str>,
    w: usize,
    h: usize,
    channels: &[&str],
    tiles: bool,
    compression: exr::prelude::Compression,
) -> Layer<AnyChannels<FlatSamples>> {
    let mut attrs = name.map_or_else(LayerAttributes::default, LayerAttributes::named);
    attrs.other.insert(Text::from("cryptomatte/abc1234/name"), exr::meta::attribute::AttributeValue::Text(Text::from("CryptoObject")));
    let list = channels.iter().enumerate().map(|(i, n)| AnyChannel::new(*n, FlatSamples::F32(ramp(w * h, i as f32 + 1.0)))).collect();
    let blocks = if tiles { Blocks::Tiles(exr::math::Vec2(4, 4)) } else { Blocks::ScanLines };
    Layer::new((w, h), attrs, Encoding { compression, blocks, line_order: LineOrder::Increasing }, AnyChannels::sort(list))
}

fn file(layers: Vec<Layer<AnyChannels<FlatSamples>>>, display: (usize, usize)) -> Vec<u8> {
    let image = exr::image::Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), display)), layers);
    let mut out = std::io::Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    out.into_inner()
}

fn all(_: &str) -> bool {
    true
}

#[test]
fn every_compression_scan_lines_and_tiles_decode_exactly() {
    use exr::prelude::Compression::*;
    for compression in [Uncompressed, RLE, ZIP1, ZIP16, PIZ] {
        for tiles in [false, true] {
            let bytes = file(vec![layer(None, 9, 7, &["R", "G", "B"], tiles, compression)], (9, 7));
            let part = decode_part(&bytes, 0, &all, &Limits::default()).unwrap();
            assert_eq!((part.info.tiled, part.info.data_window.width, part.info.data_window.height), (tiles, 9, 7));
            // Each channel kept its own ramp, whatever order the file stores them in.
            for (name, k) in [("R", 1.0), ("G", 2.0), ("B", 3.0)] {
                assert_eq!(part.channel(name).unwrap().samples, Samples::F32(ramp(63, k)), "{compression:?} tiles {tiles} {name}");
            }
        }
    }
    // Lossy ones decode too (values not exact).
    for compression in [PXR24, B44, B44A, DWAA(None), DWAB(None)] {
        let bytes = file(vec![layer(None, 16, 16, &["R"], false, compression)], (16, 16));
        assert_eq!(decode_part(&bytes, 0, &all, &Limits::default()).unwrap().channels[0].samples.len(), 256, "{compression:?}");
    }
}

#[test]
fn multi_part_files_list_and_decode_one_part() {
    let bytes = file(
        vec![
            layer(Some("depth"), 4, 2, &["Z"], false, exr::prelude::Compression::ZIP1),
            layer(Some("beauty"), 4, 2, &["A", "B", "G", "R"], true, exr::prelude::Compression::PIZ),
        ],
        (4, 2),
    );
    let parts = parts(&bytes, &Limits::default()).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!((parts[0].name.as_deref(), parts[1].name.as_deref(), parts[1].tiled), (Some("depth"), Some("beauty"), true));
    assert_eq!(parts[0].channels, [ChannelInfo { name: "Z".into(), sample: SampleType::F32, sampling: (1, 1) }]);
    assert_eq!((parts[0].color_rank(), parts[1].color_rank(), parts[1].compression.as_str()), (1, 4, "PIZ"));
    assert_eq!(parts[1].text("cryptomatte/abc1234/name"), Some("CryptoObject"));
    assert_eq!(pick_part(&parts), Some(1));
    // Only the kept channels of the chosen part come back.
    let part = decode_part(&bytes, 1, &|n| n == "R" || n == "A", &Limits::default()).unwrap();
    assert_eq!(part.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A", "R"]);
    let z = decode_part(&bytes, 0, &all, &Limits::default()).unwrap();
    assert_eq!(z.channels[0].samples.to_f32(), ramp(8, 1.0));
    assert!(matches!(decode_part(&bytes, 2, &all, &Limits::default()), Err(Error::Malformed(_))));
    assert_eq!(pick_part(&[]), None);
}

#[test]
fn written_files_read_back() {
    let h: Vec<f16> = (0..6).map(|i| f16::from_f32(i as f32 / 4.0)).collect();
    let channels = vec![
        Channel { name: "Y".into(), samples: Samples::F16(h.clone()) },
        Channel { name: "A".into(), samples: Samples::F32(ramp(6, 0.5)) },
        Channel { name: "id".into(), samples: Samples::U32((0..6).collect()) },
    ];
    for c in [Compression::None, Compression::Rle, Compression::Zip1, Compression::Zip16, Compression::Piz] {
        let bytes = write(3, 2, &channels, c).unwrap();
        let part = decode_part(&bytes, 0, &all, &Limits::default()).unwrap();
        assert_eq!(part.channel("Y").unwrap().samples, Samples::F16(h.clone()));
        assert_eq!(part.channel("A").unwrap().samples, Samples::F32(ramp(6, 0.5)));
        assert_eq!(part.channel("id").unwrap().samples, Samples::U32((0..6).collect()));
        assert_eq!(part.channel("id").unwrap().samples.get_f32(5), 5.0);
    }
    assert!(matches!(write(0, 2, &channels, Compression::None), Err(Error::Encode(_))));
    assert!(matches!(write(2, 2, &channels, Compression::None), Err(Error::Encode(_))));
    assert!(matches!(write(3, 2, &[], Compression::None), Err(Error::Encode(_))));
}

#[test]
fn display_window_placement() {
    let data = Window { x: 1, y: -1, width: 2, height: 2 };
    let display = Window { x: 0, y: 0, width: 3, height: 2 };
    // Row -1 is cut; row 0 lands at display row 0 from column 1.
    assert_eq!(to_display_window(&[1.0, 2.0, 3.0, 4.0], data, display, 9.0, 100).unwrap(), [9.0, 3.0, 4.0, 9.0, 9.0, 9.0]);
    assert_eq!(to_display_window(&[1.0], data, display, 0.0, 100), None);
    assert_eq!(to_display_window(&[1.0, 2.0, 3.0, 4.0], data, display, 0.0, 5), None);
    let far = Window { x: -10, y: 0, width: 2, height: 2 };
    assert_eq!(to_display_window(&[1.0, 2.0, 3.0, 4.0], far, display, 0.0, 100).unwrap(), [0.0; 6]);
}

/// A 2x2 single-part file whose layer carries a 2x1 preview.
fn with_preview() -> Vec<u8> {
    let mut attrs = LayerAttributes::named("rgb");
    attrs.preview = Some(Preview { size: exr::math::Vec2(2, 1), pixel_data: vec![7; 8] });
    let list = ["R", "G", "B"].into_iter().map(|n| AnyChannel::new(n, FlatSamples::F32(vec![0.5; 4]))).collect();
    let layer = Layer::new(
        (2, 2),
        attrs,
        Encoding { compression: exr::prelude::Compression::Uncompressed, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
        AnyChannels::sort(list),
    );
    file(vec![layer], (2, 2))
}

#[test]
fn a_preview_declaring_more_pixels_than_it_holds_is_refused_before_the_crate_reads_it() {
    let good = with_preview();
    assert_eq!(decode_part(&good, 0, &all, &Limits::default()).unwrap().channels.len(), 3);
    // Patch the preview to 1024x1024 (4 MiB) while its value still holds 8 bytes.
    let tag = b"preview\0preview\0";
    let at = good.windows(tag.len()).position(|w| w == tag).unwrap() + tag.len() + 4;
    let mut bad = good.clone();
    bad[at..at + 4].copy_from_slice(&1024u32.to_le_bytes());
    bad[at + 4..at + 8].copy_from_slice(&1024u32.to_le_bytes());
    for r in [parts(&bad, &Limits::default()).map(|_| ()), decode_part(&bad, 0, &all, &Limits::default()).map(|_| ())] {
        assert!(matches!(&r, Err(Error::Malformed(m)) if m.contains("preview")), "{r:?}");
    }
    // A header cut off mid-attribute is left to the crate to report.
    assert!(parts(&good[..at], &Limits::default()).is_err());
}

#[test]
fn limits_are_checked_from_the_headers() {
    let bytes = file(vec![layer(None, 8, 4, &["R", "G"], false, exr::prelude::Compression::ZIP16)], (8, 4));
    let ok = |l: Limits| parts(&bytes, &l).is_ok();
    let d = Limits::default();
    assert!(ok(Limits { max_width: 8, max_height: 4, max_pixels: 32, max_bytes: 8 * 4 * 2 * 4, ..d }));
    assert!(matches!(parts(&bytes, &Limits { max_width: 7, ..d }), Err(Error::LimitExceeded(_))));
    assert!(matches!(parts(&bytes, &Limits { max_height: 3, ..d }), Err(Error::LimitExceeded(_))));
    assert!(matches!(parts(&bytes, &Limits { max_pixels: 31, ..d }), Err(Error::LimitExceeded(_))));
    assert!(matches!(decode_part(&bytes, 0, &all, &Limits { max_bytes: 255, ..d }), Err(Error::LimitExceeded(_))));
    // A file too small for what it declares: 256 bytes of samples against a ratio of 0 and no slack
    // is over; the 1 MiB slack always admits small pictures.
    assert!(ok(Limits { max_ratio: 0, ..d }));
    // A header claiming a huge data window over a few bytes of file is refused by the ratio.
    let big = file(vec![layer(None, 4096, 4096, &["R"], false, exr::prelude::Compression::ZIP16)], (4096, 4096));
    assert!(ok_big(&big));
    assert!(matches!(parts(&big, &Limits { max_ratio: 1, ..d }), Err(Error::LimitExceeded(_))));
}

fn ok_big(b: &[u8]) -> bool {
    parts(b, &Limits::default()).is_ok()
}

#[test]
fn hostile_files_never_panic() {
    let good = file(
        vec![
            layer(Some("a"), 5, 3, &["R", "G", "B"], false, exr::prelude::Compression::ZIP1),
            layer(Some("b"), 5, 3, &["Y"], true, exr::prelude::Compression::RLE),
        ],
        (5, 3),
    );
    let limits = Limits { max_bytes: 1 << 24, ..Limits::default() };
    for n in 0..good.len() {
        let _ = parts(&good[..n], &limits);
        let _ = decode_part(&good[..n], 1, &all, &limits);
    }
    for i in 0..good.len() {
        for v in [0u8, 1, 0x7F, 0xFF] {
            let mut b = good.clone();
            b[i] = v;
            if let Ok(p) = decode_part(&b, 0, &all, &limits) {
                let n = p.info.data_window.width as usize * p.info.data_window.height as usize;
                assert!(p.channels.iter().all(|c| c.samples.len() == n));
            }
        }
    }
    assert!(parts(b"not exr", &limits).is_err());
}
