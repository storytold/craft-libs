//! The preview check is PhotoCraft's test; the rest builds files with the `exr` crate, except the
//! files that mix flat and deep parts, which are built by hand.
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

#[test]
fn decoded_display_part_places_selected_channels_and_checks_total_budget() {
    let bytes = file(vec![layer(None, 2, 2, &["R", "G"], false, exr::prelude::Compression::ZIP1)], (3, 2));
    let part = decode_display_part(&bytes, 0, &|name| name == "R", &|_| 9.0, &Limits::default()).unwrap();
    assert_eq!(part.channels.len(), 1);
    assert_eq!(part.channels[0].name, "R");
    assert_eq!(part.channels[0].samples, [0.0, 1.0, 9.0, 2.0, 3.0, 9.0]);

    let limits = Limits { max_bytes: 6 * 4 - 1, ..Limits::default() };
    assert!(matches!(decode_display_part(&bytes, 0, &all, &|_| 0.0, &limits), Err(Error::LimitExceeded(_))));

    // One selected display plane fits exactly. Both data channels together (2x2x2 f32, 32 bytes)
    // don't, but they are decompressed one scan-line block (2 pixels x 2 channels) at a time.
    let block_limits = Limits { max_bytes: 6 * 4, ..Limits::default() };
    let part = decode_display_part(&bytes, 0, &|name| name == "R", &|_| 9.0, &block_limits).unwrap();
    assert_eq!(part.channels[0].samples, [0.0, 1.0, 9.0, 2.0, 3.0, 9.0]);
}

#[test]
fn decoded_display_part_bounds_each_all_channel_block() {
    // The kept R plane is 4 pixels of f32 (16 bytes), but each scan-line block carries all four
    // channels (64 bytes) before the ignored ones are discarded.
    let bytes = file(vec![layer(None, 4, 1, &["R", "G", "B", "A"], false, exr::prelude::Compression::ZIP1)], (4, 1));
    let r = |name: &str| name == "R";
    assert!(matches!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &Limits { max_bytes: 16, ..Limits::default() }), Err(Error::LimitExceeded(_))));
    assert_eq!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &Limits { max_bytes: 64, ..Limits::default() }).unwrap().channels.len(), 1);
    // ZIP groups 16 scan lines, but a one-row part's block is that row: still 64 bytes, not 1024
    let zip16 = file(vec![layer(None, 4, 1, &["R", "G", "B", "A"], false, exr::prelude::Compression::ZIP16)], (4, 1));
    assert!(matches!(decode_display_part(&zip16, 0, &r, &|_| 0.0, &Limits { max_bytes: 63, ..Limits::default() }), Err(Error::LimitExceeded(_))));
    assert_eq!(decode_display_part(&zip16, 0, &r, &|_| 0.0, &Limits { max_bytes: 64, ..Limits::default() }).unwrap().channels.len(), 1);
    // A tile is the block of a tiled part: a 4x4 tile of four f32 channels is 256 bytes, while
    // the whole 8x8 part is 1024 and the kept R plane of the 2x2 display window only 16.
    let tiled = file(vec![layer(None, 8, 8, &["R", "G", "B", "A"], true, exr::prelude::Compression::ZIP1)], (2, 2));
    assert!(matches!(decode_display_part(&tiled, 0, &r, &|_| 0.0, &Limits { max_bytes: 255, ..Limits::default() }), Err(Error::LimitExceeded(_))));
    assert_eq!(decode_display_part(&tiled, 0, &r, &|_| 0.0, &Limits { max_bytes: 256, ..Limits::default() }).unwrap().channels[0].samples.len(), 4);
}

#[test]
fn decoded_display_part_clips_a_wide_data_window_to_the_small_display() {
    let width = (Limits::default().max_width as usize) + 1;
    let bytes = file(vec![layer(None, width, 1, &["R"], false, exr::prelude::Compression::ZIP1)], (1, 1));
    let part = decode_display_part(&bytes, 0, &all, &|_| 9.0, &Limits::default()).unwrap();
    assert_eq!(part.channels.len(), 1);
    assert_eq!(part.channels[0].samples, [0.0]);
}

#[test]
fn decoded_display_part_rejects_kept_subsampled_channels() {
    let mut bytes = file(vec![layer(None, 4, 1, &["R"], false, exr::prelude::Compression::Uncompressed)], (4, 1));
    // exr's writer doesn't emit subsampling yet, so patch the standard chlist sampling field in a
    // valid uncompressed file. The reader must reject it before looking at the block payload.
    let tag = b"channels\0chlist\0";
    let attr = bytes.windows(tag.len()).position(|window| window == tag).unwrap();
    let payload = attr + tag.len() + 4;
    let name_end = payload + bytes[payload..].iter().position(|byte| *byte == 0).unwrap();
    let x_sampling = name_end + 1 + 4 + 1 + 3;
    bytes[x_sampling..x_sampling + 4].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(decode_display_part(&bytes, 0, &|name| name == "R", &|_| 0.0, &Limits::default()), Err(Error::Unsupported(_))));
}

#[test]
fn a_flat_chunk_s_packed_length_is_checked_before_the_crate_reads_it() {
    // One 9 × 7 tile of a 1024 × 1024 tile size: the crate would reserve up to the nominal tile
    // (1024 × 1024 pixels of three f32 channels, 12 MiB) for the chunk's declared length.
    let list = ["B", "G", "R"].iter().map(|n| AnyChannel::new(*n, FlatSamples::F32(ramp(63, 1.0)))).collect();
    let encoding = Encoding {
        compression: exr::prelude::Compression::Uncompressed,
        blocks: Blocks::Tiles(exr::math::Vec2(1024, 1024)),
        line_order: LineOrder::Increasing,
    };
    let mut bytes = file(vec![Layer::new((9, 7), LayerAttributes::default(), encoding, AnyChannels::sort(list))], (9, 7));
    assert_eq!(decode_part(&bytes, 0, &all, &Limits::default()).unwrap().channels.len(), 3);
    let mut cursor = std::io::Cursor::new(&bytes[..]);
    exr::meta::MetaData::read_from_buffered(&mut cursor, false).unwrap();
    let table = cursor.position() as usize;
    let chunk = u64::from_le_bytes(bytes[table..table + 8].try_into().unwrap()) as usize;
    // The packed length follows the four tile coordinates.
    let size_at = chunk + 16;
    bytes.extend_from_slice(&[0; 2000]);
    let limits = Limits { max_bytes: 1000, ..Limits::default() };
    let r = |name: &str| name == "R";
    // More bytes than the file has left after the length.
    bytes[size_at..size_at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(matches!(decode_part(&bytes, 0, &all, &limits), Err(Error::Malformed(m)) if m.contains("remain in the file")));
    assert!(matches!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &limits), Err(Error::Malformed(m)) if m.contains("remain in the file")));
    // Inside the file, but over the byte budget.
    bytes[size_at..size_at + 4].copy_from_slice(&1001i32.to_le_bytes());
    assert!(matches!(decode_part(&bytes, 0, &all, &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &limits), Err(Error::LimitExceeded(_))));
    // A decode that keeps no channel reads no chunk.
    let none = |_: &str| false;
    assert!(decode_part(&bytes, 0, &none, &limits).unwrap().channels.is_empty());
    assert!(decode_display_part(&bytes, 0, &none, &|_| 0.0, &limits).unwrap().channels.is_empty());
}

#[test]
fn a_skipped_flat_chunk_s_packed_length_is_not_held_to_the_budget() {
    use exr::math::RoundingMode;
    use exr::prelude::Levels;
    // 9 × 7 in 4 × 4 tiles with mip levels. The reader reads only the six tiles of level 0, so a
    // smaller level's chunk may declare more than the budget while it fits in the file.
    let sizes: Vec<usize> = exr::meta::mip_map_levels(RoundingMode::Down, exr::math::Vec2(9, 7)).map(|(_, size)| size.area()).collect();
    let levels = Levels::Mip { rounding_mode: RoundingMode::Down, level_data: sizes.iter().map(|n| FlatSamples::F32(ramp(*n, 1.0))).collect() };
    let encoding = Encoding {
        compression: exr::prelude::Compression::Uncompressed,
        blocks: Blocks::Tiles(exr::math::Vec2(4, 4)),
        line_order: LineOrder::Increasing,
    };
    let channels = AnyChannels::sort(vec![AnyChannel::new("R", levels)].into());
    let layers = vec![Layer::new((9, 7), LayerAttributes::default(), encoding, channels)];
    let image = exr::image::Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), (9, 7))), layers);
    let mut out = std::io::Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    let mut bytes = out.into_inner();
    let mut cursor = std::io::Cursor::new(&bytes[..]);
    let meta = exr::meta::MetaData::read_from_buffered(&mut cursor, false).unwrap();
    let table = cursor.position() as usize;
    let count = meta.headers[0].chunk_count;
    assert!(count > 6, "the part has chunks beyond level 0");
    // The last chunk in the table holds the smallest level: its tile coordinates, then its level.
    let last = table + (count - 1) * 8;
    let chunk = u64::from_le_bytes(bytes[last..last + 8].try_into().unwrap()) as usize;
    let level_x = i32::from_le_bytes(bytes[chunk + 8..chunk + 12].try_into().unwrap());
    let level_y = i32::from_le_bytes(bytes[chunk + 12..chunk + 16].try_into().unwrap());
    assert_ne!((level_x, level_y), (0, 0));
    bytes.extend_from_slice(&[0; 2000]);
    bytes[chunk + 16..chunk + 20].copy_from_slice(&1500i32.to_le_bytes());
    let limits = Limits { max_bytes: 1000, ..Limits::default() };
    let r = |name: &str| name == "R";
    assert_eq!(decode_part(&bytes, 0, &all, &limits).unwrap().channels.len(), 1);
    assert_eq!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &limits).unwrap().channels.len(), 1);
    // The reader picks chunks by their place in the table, so a level-0 entry that points at the
    // smaller level's chunk is read, and that chunk's length is held to the budget.
    let entry = bytes[last..last + 8].to_vec();
    bytes[table..table + 8].copy_from_slice(&entry);
    assert!(matches!(decode_part(&bytes, 0, &all, &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &limits), Err(Error::LimitExceeded(_))));
}

#[test]
fn the_packed_length_check_finds_the_end_of_level_0_among_many_small_tiles() {
    use exr::math::RoundingMode;
    use exr::prelude::Levels;
    // 64 × 48 in 1 × 1 tiles with mip levels: the 3,072 tiles of level 0 fill the first places of
    // the offset table, and the smaller levels follow.
    let (w, h) = (64, 48);
    let sizes: Vec<usize> = exr::meta::mip_map_levels(RoundingMode::Down, exr::math::Vec2(w, h)).map(|(_, size)| size.area()).collect();
    let levels = Levels::Mip { rounding_mode: RoundingMode::Down, level_data: sizes.iter().map(|n| FlatSamples::F32(ramp(*n, 1.0))).collect() };
    let encoding = Encoding {
        compression: exr::prelude::Compression::Uncompressed,
        blocks: Blocks::Tiles(exr::math::Vec2(1, 1)),
        line_order: LineOrder::Increasing,
    };
    let channels = AnyChannels::sort(vec![AnyChannel::new("R", levels)].into());
    let layers = vec![Layer::new((w, h), LayerAttributes::default(), encoding, channels)];
    let image = exr::image::Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), (w, h))), layers);
    let mut out = std::io::Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    let mut bytes = out.into_inner();
    let mut cursor = std::io::Cursor::new(&bytes[..]);
    let meta = exr::meta::MetaData::read_from_buffered(&mut cursor, false).unwrap();
    let table = cursor.position() as usize;
    let level_0 = w * h;
    assert!(meta.headers[0].chunk_count > level_0, "the part has chunks beyond level 0");
    let chunk_at = |bytes: &[u8], place: usize| u64::from_le_bytes(bytes[table + place * 8..table + place * 8 + 8].try_into().unwrap()) as usize;
    let level_of = |bytes: &[u8], chunk: usize| {
        let x = i32::from_le_bytes(bytes[chunk + 8..chunk + 12].try_into().unwrap());
        let y = i32::from_le_bytes(bytes[chunk + 12..chunk + 16].try_into().unwrap());
        (x, y)
    };
    let (last_read, first_skipped) = (chunk_at(&bytes, level_0 - 1), chunk_at(&bytes, level_0));
    assert_eq!(level_of(&bytes, last_read), (0, 0));
    assert_ne!(level_of(&bytes, first_skipped), (0, 0));
    // The decoded part takes 12,288 bytes, inside the budget; a declared length of 30,000 is not.
    bytes.extend_from_slice(&[0; 40_000]);
    let limits = Limits { max_bytes: 20_000, ..Limits::default() };
    let r = |name: &str| name == "R";
    // The first chunk past level 0 is not read, so its length is not held to the budget.
    bytes[first_skipped + 16..first_skipped + 20].copy_from_slice(&30_000i32.to_le_bytes());
    assert_eq!(decode_part(&bytes, 0, &all, &limits).unwrap().channels.len(), 1);
    assert_eq!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &limits).unwrap().channels.len(), 1);
    // The last chunk of level 0 is read, so its length is.
    bytes[last_read + 16..last_read + 20].copy_from_slice(&30_000i32.to_le_bytes());
    assert!(matches!(decode_part(&bytes, 0, &all, &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(decode_display_part(&bytes, 0, &r, &|_| 0.0, &limits), Err(Error::LimitExceeded(_))));
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
    // Only headers are read here: keep the large declared windows without encoding their pixels.
    let mut big = file(vec![layer(None, 8, 4, &["R"], false, exr::prelude::Compression::ZIP16)], (8, 4));
    let window: Vec<u8> = [0i32, 0, 4095, 4095].iter().flat_map(|v| v.to_le_bytes()).collect();
    for tag in ["dataWindow\0box2i\0", "displayWindow\0box2i\0"] {
        let tag = tag.as_bytes();
        let at = big.windows(tag.len()).position(|w| w == tag).unwrap() + tag.len() + 4;
        big[at..at + 16].copy_from_slice(&window);
    }
    // At 1024 file bytes the default ratio admits 64 MiB, but ratio 1 plus 1 MiB slack does not.
    big.resize(1024, 0);
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
    // The gate runs every divisor-th truncation and byte; nightly runs every one.
    let step = craft_testkit::hostile::divisor();
    for n in (0..good.len()).step_by(step) {
        let _ = parts(&good[..n], &limits);
        let _ = decode_part(&good[..n], 1, &all, &limits);
    }
    for i in (0..good.len()).step_by(step) {
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

// A multi-part file with a flat part and a deep scan-line part, built by hand per the OpenEXR file
// layout (the `exr` crate's writer refuses deep headers): version 2 with the deep and multi-part
// flags, the headers and an empty header, one offset table per part, then chunks that each start
// with their part number. Both parts are uncompressed, two chunks a part: the deep part and a
// scan-line flat part hold one scan line a chunk, and a tiled flat part one 2 × 2 tile a chunk.

fn mixed_attr(out: &mut Vec<u8>, name: &str, ty: &str, payload: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(ty.as_bytes());
    out.push(0);
    out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
    out.extend_from_slice(payload);
}

/// The version field and the headers of a mixed file whose flat part is `w × h` with float
/// `B`, `G`, `R` channels, in scan lines or (with `tiled`) in one level of 2 × 2 tiles, and whose
/// deep part is 3 × 2 with one float `A` channel. Both parts are uncompressed.
fn mixed_headers(deep_first: bool, tiled: bool, (w, h): (usize, usize)) -> Vec<u8> {
    mixed_headers_with(deep_first, tiled, (w, h), 0, &["B", "G", "R"], 2)
}

/// [`mixed_headers`] with the flat part's compression code (0 for none, or 3 for ZIP, which
/// holds 16 scan lines a block), its float channels, and the width and height of its tiles.
fn mixed_headers_with(deep_first: bool, tiled: bool, (w, h): (usize, usize), compression: u8, flat_channels: &[&str], tile: u32) -> Vec<u8> {
    mixed_headers_with_deep_size(deep_first, tiled, (w, h), compression, flat_channels, tile, (3, 2))
}

fn mixed_headers_with_deep_size(
    deep_first: bool,
    tiled: bool,
    (w, h): (usize, usize),
    compression: u8,
    flat_channels: &[&str],
    tile: u32,
    deep_size: (usize, usize),
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0131_2f76u32.to_le_bytes());
    out.extend_from_slice(&(2u32 | 0x800 | 0x1000).to_le_bytes());
    for deep in if deep_first { [true, false] } else { [false, true] } {
        let (w, h, channels): (usize, usize, &[&str]) = if deep { (deep_size.0, deep_size.1, &["A"]) } else { (w, h, flat_channels) };
        let mut chlist = Vec::new();
        for name in channels {
            chlist.extend_from_slice(name.as_bytes());
            chlist.push(0);
            chlist.extend_from_slice(&2i32.to_le_bytes()); // float
            chlist.extend_from_slice(&[0u8; 4]); // pLinear and reserved
            chlist.extend_from_slice(&1i32.to_le_bytes());
            chlist.extend_from_slice(&1i32.to_le_bytes());
        }
        chlist.push(0);
        let window: Vec<u8> = [0i32, 0, w as i32 - 1, h as i32 - 1].iter().flat_map(|v| v.to_le_bytes()).collect();
        let tiled = tiled && !deep;
        let compression = if deep { 0 } else { compression };
        let lines = if compression == 3 { 16 } else { 1 };
        let chunks = if tiled { w.div_ceil(tile as usize) * h.div_ceil(tile as usize) } else { h.div_ceil(lines) };
        mixed_attr(&mut out, "channels", "chlist", &chlist);
        mixed_attr(&mut out, "chunkCount", "int", &(chunks as i32).to_le_bytes());
        mixed_attr(&mut out, "compression", "compression", &[compression]);
        mixed_attr(&mut out, "dataWindow", "box2i", &window);
        mixed_attr(&mut out, "displayWindow", "box2i", &window);
        mixed_attr(&mut out, "lineOrder", "lineOrder", &[0]);
        if deep {
            mixed_attr(&mut out, "maxSamplesPerPixel", "int", &1i32.to_le_bytes());
        }
        mixed_attr(&mut out, "name", "string", if deep { b"deep" } else { b"flat" });
        mixed_attr(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
        mixed_attr(&mut out, "screenWindowCenter", "v2f", &[0u8; 8]);
        mixed_attr(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
        if tiled {
            // `tile` × `tile` tiles, one level, rounded down.
            let mut desc: Vec<u8> = [tile, tile].iter().flat_map(|v| v.to_le_bytes()).collect();
            desc.push(0);
            mixed_attr(&mut out, "tiles", "tiledesc", &desc);
        }
        let ty: &[u8] = if deep {
            b"deepscanline"
        } else if tiled {
            b"tiledimage"
        } else {
            b"scanlineimage"
        };
        mixed_attr(&mut out, "type", "string", ty);
        if deep {
            mixed_attr(&mut out, "version", "int", &1i32.to_le_bytes());
        }
        out.push(0); // end of this header
    }
    out.push(0); // end of the header list
    out
}

/// Sample `i` of a flat channel: `R` is 1, 2, 3, …, `G` 10 more and `B` 100 more, top row first.
pub(super) fn mixed_flat(channel: f32, i: usize) -> f32 {
    channel + 1.0 + i as f32
}

/// A mixed file with a 3 × 2 flat part, and its chunk offsets as `offsets[part][chunk]`. The
/// tiled flat part's chunks hold the tiles at (0, 0) and (1, 0).
pub(super) fn mixed_file(deep_first: bool, tiled: bool) -> (Vec<u8>, [[usize; 2]; 2]) {
    let (w, h) = (3usize, 2usize);
    let mut out = mixed_headers(deep_first, tiled, (w, h));
    let chunks: Vec<Vec<Vec<u8>>> = (0..2i32)
        .map(|part| {
            let deep = (part == 0) == deep_first;
            (0..h)
                .map(|y| {
                    let mut c = Vec::new();
                    c.extend_from_slice(&part.to_le_bytes());
                    if tiled && !deep {
                        // Tile (y, 0) at level (0, 0): columns 2y and 2y + 1 that lie inside.
                        [y as i32, 0, 0, 0].iter().for_each(|v| c.extend_from_slice(&v.to_le_bytes()));
                        let xs = 2 * y..(2 * y + 2).min(w);
                        c.extend_from_slice(&((xs.len() * h * 3 * 4) as i32).to_le_bytes());
                        for row in 0..h {
                            for channel in [100.0, 10.0, 0.0] {
                                xs.clone().for_each(|x| c.extend_from_slice(&mixed_flat(channel, row * w + x).to_le_bytes()));
                            }
                        }
                        return c;
                    }
                    c.extend_from_slice(&(y as i32).to_le_bytes());
                    if deep {
                        // Packed count table, packed and unpacked sample data: one sample a pixel.
                        for size in [w * 4, w * 4, w * 4] {
                            c.extend_from_slice(&(size as u64).to_le_bytes());
                        }
                        (1..=w as u32).for_each(|n| c.extend_from_slice(&n.to_le_bytes()));
                        (0..w).for_each(|_| c.extend_from_slice(&0.5f32.to_le_bytes()));
                    } else {
                        c.extend_from_slice(&((w * 3 * 4) as i32).to_le_bytes());
                        for channel in [100.0, 10.0, 0.0] {
                            (0..w).for_each(|x| c.extend_from_slice(&mixed_flat(channel, y * w + x).to_le_bytes()));
                        }
                    }
                    c
                })
                .collect()
        })
        .collect();
    let mut offsets = [[0usize; 2]; 2];
    let mut at = out.len() + 2 * h * 8;
    for (part, lines) in chunks.iter().enumerate() {
        for (y, c) in lines.iter().enumerate() {
            offsets[part][y] = at;
            out.extend_from_slice(&(at as u64).to_le_bytes());
            at += c.len();
        }
    }
    chunks.into_iter().flatten().for_each(|c| out.extend_from_slice(&c));
    (out, offsets)
}

#[test]
fn a_flat_part_beside_a_deep_part_decodes() {
    let limits = Limits::default();
    for (deep_first, tiled) in [(false, false), (true, false), (false, true), (true, true)] {
        let (bytes, _) = mixed_file(deep_first, tiled);
        let (flat, deep) = if deep_first { (1, 0) } else { (0, 1) };
        let parts = parts(&bytes, &limits).unwrap();
        assert_eq!((parts[flat].deep, parts[deep].deep, parts[flat].name.as_deref()), (false, true, Some("flat")));

        let part = decode_part(&bytes, flat, &all, &limits).unwrap();
        for (name, channel) in [("R", 0.0), ("G", 10.0), ("B", 100.0)] {
            let expected: Vec<f32> = (0..6).map(|i| mixed_flat(channel, i)).collect();
            assert_eq!(part.channel(name).unwrap().samples, Samples::F32(expected), "deep first {deep_first} tiled {tiled} {name}");
        }
        let only_g = decode_part(&bytes, flat, &|n| n == "G", &limits).unwrap();
        assert_eq!(only_g.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["G"]);

        let display = decode_display_part(&bytes, flat, &|n| n == "flat.R", &|_| 9.0, &limits).unwrap();
        assert_eq!(display.channels.len(), 1);
        assert_eq!((display.channels[0].name.as_str(), display.channels[0].samples.as_slice()), ("flat.R", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0][..]));

        assert!(matches!(decode_part(&bytes, deep, &all, &limits), Err(Error::Unsupported(_))));
        assert!(matches!(decode_display_part(&bytes, deep, &all, &|_| 0.0, &limits), Err(Error::Unsupported(_))));
        assert!(matches!(decode_part(&bytes, 2, &all, &limits), Err(Error::Malformed(_))));
        let deep_image = crate::deep::decode_deep(&bytes, &limits).unwrap();
        assert_eq!((deep_image.width, deep_image.height), (3, 2));
    }
}

#[test]
fn a_flat_part_s_chunks_beside_a_deep_part_are_checked_before_they_are_read() {
    let limits = Limits::default();
    for deep_first in [false, true] {
        let (good, offsets) = mixed_file(deep_first, false);
        let (flat, deep) = if deep_first { (1, 0) } else { (0, 1) };
        let tables = mixed_headers(deep_first, false, (3, 2)).len();
        let entry = tables + flat * 2 * 8;
        let with_offset = |slot: usize, value: u64| {
            let mut b = good.clone();
            b[entry + slot * 8..entry + slot * 8 + 8].copy_from_slice(&value.to_le_bytes());
            b
        };
        let refused = |b: &[u8]| {
            for r in [decode_part(b, flat, &all, &limits).map(|_| ()), decode_display_part(b, flat, &all, &|_| 0.0, &limits).map(|_| ())] {
                assert!(matches!(&r, Err(Error::Malformed(_))), "deep first {deep_first}: {r:?}");
            }
        };
        // The deep part's chunk, the end of the file and past it, the offset tables, the same
        // chunk twice.
        refused(&with_offset(0, offsets[deep][0] as u64));
        refused(&with_offset(0, good.len() as u64));
        refused(&with_offset(0, u64::MAX));
        refused(&with_offset(0, (tables + 8) as u64));
        refused(&with_offset(1, offsets[flat][0] as u64));
        // A chunk naming another part, and one declaring more bytes than a block holds.
        let mut b = good.clone();
        b[offsets[flat][1]..offsets[flat][1] + 4].copy_from_slice(&(deep as i32).to_le_bytes());
        refused(&b);
        let mut b = good.clone();
        b[offsets[flat][0] + 8..offsets[flat][0] + 12].copy_from_slice(&i32::MAX.to_le_bytes());
        refused(&b);
        // Offset tables longer than the file.
        refused(&good[..tables + 20]);

        for n in 0..good.len() {
            let _ = decode_part(&good[..n], flat, &all, &limits);
            let _ = decode_display_part(&good[..n], flat, &all, &|_| 0.0, &limits);
        }
        for i in 0..good.len() {
            for v in [0u8, 1, 0x7F, 0xFF] {
                let mut b = good.clone();
                b[i] = v;
                if let Ok(p) = decode_part(&b, flat, &all, &limits) {
                    let n = p.info.data_window.width as usize * p.info.data_window.height as usize;
                    assert!(p.channels.iter().all(|c| c.samples.len() == n));
                }
                let _ = decode_display_part(&b, flat, &all, &|_| 0.0, &limits);
            }
        }
    }
}

#[test]
fn a_tiled_flat_part_beside_a_deep_part_must_hold_each_of_its_tiles() {
    let limits = Limits::default();
    for deep_first in [false, true] {
        let (good, offsets) = mixed_file(deep_first, true);
        let flat = usize::from(deep_first);
        // The second chunk names tile (0, 0) at level (1, 1), which a one-level part does not
        // have, or tile (0, 0) again, so tile (1, 0) has no chunk.
        for coordinates in [[0i32, 0, 1, 1], [0, 0, 0, 0]] {
            let mut b = good.clone();
            let at = offsets[flat][1] + 4;
            coordinates.iter().enumerate().for_each(|(i, v)| b[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes()));
            for r in [decode_part(&b, flat, &all, &limits).map(|_| ()), decode_display_part(&b, flat, &all, &|_| 0.0, &limits).map(|_| ())] {
                assert!(matches!(&r, Err(Error::Malformed(_))), "deep first {deep_first} {coordinates:?}: {r:?}");
            }
        }
    }
}

#[test]
fn a_scan_line_chunk_beside_a_deep_part_must_start_at_its_block_s_first_line() {
    // A ZIP flat part (16 lines a block) holds its 3 × 2 pixels in one chunk, stored raw, as a
    // block is when compressing it does not make it smaller; the deep part follows.
    let limits = Limits::default();
    let (w, h) = (3usize, 2usize);
    let (plain, offsets) = mixed_file(false, false);
    let deep = &plain[offsets[1][0]..];
    let with_y = |y: i32| {
        let mut out = mixed_headers_with(false, false, (w, h), 3, &["B", "G", "R"], 2);
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&0i32.to_le_bytes());
        chunk.extend_from_slice(&y.to_le_bytes());
        chunk.extend_from_slice(&((w * h * 3 * 4) as i32).to_le_bytes());
        for row in 0..h {
            for channel in [100.0, 10.0, 0.0] {
                (0..w).for_each(|x| chunk.extend_from_slice(&mixed_flat(channel, row * w + x).to_le_bytes()));
            }
        }
        let flat_at = out.len() + 3 * 8;
        let deep_at = flat_at + chunk.len();
        for at in [flat_at, deep_at, deep_at + offsets[1][1] - offsets[1][0]] {
            out.extend_from_slice(&(at as u64).to_le_bytes());
        }
        out.extend_from_slice(&chunk);
        out.extend_from_slice(deep);
        out
    };
    let part = decode_part(&with_y(0), 0, &all, &limits).unwrap();
    assert_eq!(part.channel("R").unwrap().samples, Samples::F32((0..6).map(|i| mixed_flat(0.0, i)).collect()));
    // Each of these y values rounds to block 0, but none is that block's first line.
    for y in [-15, -1, 1, 15] {
        let b = with_y(y);
        for r in [decode_part(&b, 0, &all, &limits).map(|_| ()), decode_display_part(&b, 0, &all, &|_| 0.0, &limits).map(|_| ())] {
            assert!(matches!(&r, Err(Error::Malformed(_))), "y {y}: {r:?}");
        }
    }
}

/// A mixed file whose flat part, first, is one pixel of these float channels (each 1.0) in one
/// `tile` × `tile` tile, held by a chunk that declares `size` payload bytes.
fn mixed_one_tile(channels: &[&str], tile: u32, size: i32) -> Vec<u8> {
    let (plain, offsets) = mixed_file(false, false);
    let mut out = mixed_headers_with(false, true, (1, 1), 0, channels, tile);
    let mut chunk = Vec::new();
    [0i32, 0, 0, 0, 0, size].iter().for_each(|v| chunk.extend_from_slice(&v.to_le_bytes()));
    channels.iter().for_each(|_| chunk.extend_from_slice(&1f32.to_le_bytes()));
    let flat_at = out.len() + 3 * 8;
    let deep_at = flat_at + chunk.len();
    for at in [flat_at, deep_at, deep_at + offsets[1][1] - offsets[1][0]] {
        out.extend_from_slice(&(at as u64).to_le_bytes());
    }
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&plain[offsets[1][0]..]);
    out
}

#[test]
fn a_chunk_beside_a_deep_part_must_fit_the_rest_of_the_file() {
    // 2^14 × 2^14 tiles of three floats let a chunk hold up to 3 GiB (also on 32-bit), so the crate would
    // allocate the 2 GiB this chunk declares before it found the file too short.
    let limits = Limits::default();
    let channels = ["B", "G", "R"];
    let good = mixed_one_tile(&channels, 1 << 14, 12);
    assert_eq!(decode_part(&good, 0, &all, &limits).unwrap().channel("R").unwrap().samples, Samples::F32(vec![1.0]));
    let b = mixed_one_tile(&channels, 1 << 14, i32::MAX);
    for r in [decode_part(&b, 0, &all, &limits).map(|_| ()), decode_display_part(&b, 0, &all, &|_| 0.0, &limits).map(|_| ())] {
        assert!(matches!(&r, Err(Error::Malformed(m)) if m.contains("remain in the file")), "{r:?}");
    }
}

#[test]
fn a_tile_too_large_to_address_is_refused_before_a_chunk_is_read() {
    // Five float channels in 10^9 × 10^9 tiles: 20 bytes a pixel times 10^18 pixels does not fit
    // in a 64-bit size, though the 1 × 1 part passes every size limit.
    let limits = Limits::default();
    let channels = ["A", "B", "C", "D", "E"];
    let tile = 1_000_000_000u32;
    let mixed = mixed_one_tile(&channels, tile, 20);

    // A file of flat parts only, read by the crate's own reader, with its 4 × 4 tiles widened.
    let mut flat_only = file(vec![layer(None, 1, 1, &channels, true, exr::prelude::Compression::Uncompressed)], (1, 1));
    let at = flat_only.windows(9).position(|w| w == b"tiledesc\0").unwrap() + 9 + 4;
    [tile, tile].iter().enumerate().for_each(|(i, v)| flat_only[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes()));

    for (name, b) in [("mixed", &mixed), ("flat only", &flat_only)] {
        let results = [
            parts(b, &limits).map(|_| ()),
            decode_part(b, 0, &all, &limits).map(|_| ()),
            decode_display_part(b, 0, &all, &|_| 0.0, &limits).map(|_| ()),
        ];
        for r in results {
            assert!(matches!(&r, Err(Error::LimitExceeded(_))), "{name}: {r:?}");
        }
    }
}

#[test]
fn the_flat_parts_of_a_mixed_file_must_fit_the_file() {
    // 1024 × 1024 × 3 float samples (12 MiB) declared in a file of a few hundred bytes: over a
    // ratio of 1 plus the 1 MiB slack, though another part is deep.
    let headers = mixed_headers(false, false, (1024, 1024));
    assert!(parts(&headers, &Limits::default()).is_ok());
    let tight = Limits { max_ratio: 1, ..Limits::default() };
    for r in [parts(&headers, &tight).map(|_| ()), decode_part(&headers, 0, &all, &tight).map(|_| ())] {
        assert!(matches!(&r, Err(Error::LimitExceeded(_))), "{r:?}");
    }
    // The deep part's samples are not counted, and a small flat part passes the same ratio.
    assert!(parts(&mixed_file(false, false).0, &tight).is_ok());
}

#[test]
fn deep_samples_do_not_count_against_the_flat_expansion_ratio() {
    // The 4 MiB deep plane would exceed max_ratio=1 plus the 1 MiB slack if counted.
    // Header inspection does not allocate those samples; deep decoding bounds its own counts.
    let headers = mixed_headers_with_deep_size(false, false, (1, 1), 0, &["R"], 2, (1024, 1024));
    let limits = Limits { max_ratio: 1, max_bytes: 8 * 1024 * 1024, ..Limits::default() };
    assert!(parts(&headers, &limits).is_ok());
}

#[test]
fn mixed_display_decode_bounds_each_block_not_the_whole_flat_part() {
    for (tiled, block_bytes) in [(false, 36), (true, 48)] {
        for deep_first in [false, true] {
            let (bytes, _) = mixed_file(deep_first, tiled);
            let flat = usize::from(deep_first);
            let keep = |name: &str| name == "flat.R";
            // The full flat part is 72 bytes; the selected plane is 24 bytes.
            let limits = Limits { max_bytes: block_bytes, ..Limits::default() };
            let part = decode_display_part(&bytes, flat, &keep, &|_| 0.0, &limits).unwrap();
            assert_eq!(part.channels[0].samples, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
            let tight = Limits { max_bytes: block_bytes - 1, ..limits };
            assert!(matches!(decode_display_part(&bytes, flat, &keep, &|_| 0.0, &tight), Err(Error::LimitExceeded(_))));
        }
    }
}

#[test]
fn present_packed_payload_is_bounded_before_allocation() {
    let packed = 1024 * 1024;
    let mut bytes = mixed_one_tile(&["R"], 1 << 15, packed);
    // Make the declared payload present, not merely a truncated-file error. The nominal
    // huge tile allows this size in exr::Chunk::read, but the actual flat image is 1x1.
    bytes.resize(bytes.len() + packed as usize, 0);
    let limits = Limits { max_bytes: 4, ..Limits::default() };
    assert!(matches!(
        decode_display_part(&bytes, 0, &|_| true, &|_| 0.0, &limits),
        Err(Error::LimitExceeded(message)) if message.contains("packed flat block")
    ));
    // decode_part also includes the deep header's 24 bytes in its aggregate header budget.
    let limits = Limits { max_bytes: 32, ..limits };
    assert!(matches!(decode_part(&bytes, 0, &all, &limits), Err(Error::LimitExceeded(message)) if message.contains("packed flat block")));
}

#[test]
fn cropped_mixed_display_bounds_block_tracking() {
    let mut bytes = mixed_headers_with(false, true, (100, 100), 0, &["R"], 1);
    // Both display windows are 1x1; the flat data window still has 10,000 one-pixel tiles.
    let needle = b"displayWindow\0box2i\0";
    let positions: Vec<usize> = bytes.windows(needle.len()).enumerate().filter_map(|(i, w)| (w == needle).then_some(i + needle.len() + 4)).collect();
    for at in positions {
        bytes[at..at + 16].fill(0);
    }
    // Supply all offset-table bytes so validation reaches bookkeeping, not truncation.
    bytes.resize(bytes.len() + (10_000 + 2) * 8, 0);
    let limits = Limits { max_bytes: 4, ..Limits::default() };
    assert!(matches!(
        decode_display_part(&bytes, 0, &|_| true, &|_| 0.0, &limits),
        Err(Error::LimitExceeded(message)) if message.contains("block tracking")
    ));
}

#[test]
fn dense_block_ordinals_cover_scan_lines_and_all_tile_levels() {
    use exr::block::chunk::TileCoordinates;
    use exr::meta::attribute::LevelMode;
    for tiled in [false, true] {
        let bytes = file(vec![layer(None, 9, 7, &["R"], tiled, exr::prelude::Compression::ZIP1)], (9, 7));
        let mut meta = read_meta(&bytes, &Limits::default()).unwrap();
        let header = &mut meta.headers[0];
        for mode in [LevelMode::Singular, LevelMode::MipMap, LevelMode::RipMap] {
            if let exr::meta::BlockDescription::Tiles(tiles) = &mut header.blocks {
                tiles.level_mode = mode;
            }
            for (i, block) in header.blocks_increasing_y_order().enumerate() {
                assert_eq!(flat_block_ordinal(header, block.location).unwrap(), i);
            }
            let absent = TileCoordinates { tile_index: exr::math::Vec2(0, 0), level_index: exr::math::Vec2(100, 100) };
            assert!(flat_block_ordinal(header, absent).is_err());
        }
    }
}

#[test]
fn write_rgba_compressions_and_chromaticities() {
    let compressions = [
        Compression::None,
        Compression::Rle,
        Compression::Zip1,
        Compression::Zip16,
        Compression::Piz,
        Compression::Pxr24,
        Compression::B44,
        Compression::B44a,
        Compression::Dwaa,
        Compression::Dwab,
    ];
    let chroma = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]];
    let data = vec![0.25_f32, 0.5, 0.75, 1.0, 0.1, 0.2, 0.3, 0.4, 0.9, 0.8, 0.7, 0.6, 0.0, 0.0, 0.0, 1.0];
    for comp in compressions {
        for depth in [SampleDepth::Float, SampleDepth::Half] {
            let mut buf = std::io::Cursor::new(Vec::new());
            write_rgba(&mut buf, &data, 2, 2, depth, comp, Some(chroma)).unwrap();
            let bytes = buf.into_inner();
            let meta = exr::meta::MetaData::read_from_buffered(std::io::Cursor::new(&bytes), false).unwrap();
            let c = meta.headers[0].shared_attributes.chromaticities.unwrap();
            assert_eq!([c.red.0, c.red.1], chroma[0]);
            assert_eq!([c.green.0, c.green.1], chroma[1]);
            assert_eq!([c.blue.0, c.blue.1], chroma[2]);
            assert_eq!([c.white.0, c.white.1], chroma[3]);
            let part = decode_part(&bytes, 0, &|_| true, &Limits::default()).unwrap();
            assert_eq!(part.info.data_window.width, 2);
            assert_eq!(part.info.data_window.height, 2);
            assert_eq!(part.channels.len(), 4);
        }
    }
}

#[test]
fn write_rgba_half_clamps_overflow() {
    let data = vec![100_000.0_f32, -100_000.0, 0.5, 1.0];
    let mut buf = std::io::Cursor::new(Vec::new());
    write_rgba(&mut buf, &data, 1, 1, SampleDepth::Half, Compression::None, None).unwrap();
    let bytes = buf.into_inner();
    let part = decode_part(&bytes, 0, &|_| true, &Limits::default()).unwrap();
    let r = part.channels.iter().find(|c| c.name == "R").unwrap();
    let g = part.channels.iter().find(|c| c.name == "G").unwrap();
    match (&r.samples, &g.samples) {
        (Samples::F16(rv), Samples::F16(gv)) => {
            assert_eq!(rv[0].to_f32(), 65_504.0);
            assert_eq!(gv[0].to_f32(), -65_504.0);
        }
        _ => panic!("expected F16 samples"),
    }
}
