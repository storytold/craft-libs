//! Tests of [`crate::seek`]: files are built with the `exr` crate and read back through the
//! seekable reader; the whole-part decoder ([`crate::decode`]) is the oracle for what a channel
//! should hold.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::io::{Cursor, Read, Seek, SeekFrom};

use exr::math::Vec2;
use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, ImageAttributes, IntegerBounds, Layer, LayerAttributes, LineOrder,
    WritableImage,
};

use super::decode::{Limits, decode_display_part};
use super::seek::*;

fn value(i: usize, k: usize) -> f32 {
    (i * 7 + k * 1000) as f32 / 4.0
}

/// A layer of `w × h` at `at` in the display window, with these channels (f32 ramps, each its own).
fn layer(
    name: Option<&str>,
    at: (i32, i32),
    (w, h): (usize, usize),
    channels: &[&str],
    tiles: bool,
    compression: Compression,
) -> Layer<AnyChannels<FlatSamples>> {
    let mut attrs = name.map_or_else(LayerAttributes::default, LayerAttributes::named);
    attrs.layer_position = Vec2(at.0, at.1);
    let list = channels.iter().enumerate().map(|(k, n)| AnyChannel::new(*n, FlatSamples::F32((0..w * h).map(|i| value(i, k)).collect()))).collect();
    let blocks = if tiles { Blocks::Tiles(Vec2(4, 4)) } else { Blocks::ScanLines };
    Layer::new((w, h), attrs, Encoding { compression, blocks, line_order: LineOrder::Increasing }, AnyChannels::sort(list))
}

fn file(layers: Vec<Layer<AnyChannels<FlatSamples>>>, display: (usize, usize)) -> Vec<u8> {
    let image = exr::image::Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), display)), layers);
    let mut out = Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    out.into_inner()
}

fn never() -> bool {
    false
}

fn indexed(bytes: &[u8]) -> Index {
    Index::build(&mut Cursor::new(bytes), &Limits::default(), &never).unwrap()
}

/// What the whole-part decoder places in the display window for each channel of part `part`.
fn oracle(bytes: &[u8], part: usize, fill: f32) -> Vec<Vec<f32>> {
    decode_display_part(bytes, part, &|_| true, &|_| fill, &Limits::default()).unwrap().channels.into_iter().map(|c| c.samples).collect()
}

fn channels_of(index: &Index, part: usize) -> Vec<(usize, usize)> {
    (0..index.parts()[part].channels.len()).map(|c| (part, c)).collect()
}

#[test]
fn every_compression_scan_lines_and_tiles_read_like_the_whole_part_decoder() {
    for compression in
        [Compression::Uncompressed, Compression::RLE, Compression::ZIP1, Compression::ZIP16, Compression::PIZ, Compression::PXR24, Compression::B44]
    {
        for tiles in [false, true] {
            let bytes = file(vec![layer(None, (0, 0), (9, 7), &["R", "G", "B", "A"], tiles, compression)], (9, 7));
            let index = indexed(&bytes);
            let channels = channels_of(&index, 0);
            let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never).unwrap();
            assert_eq!((read.width, read.height), (9, 7));
            assert_eq!(read.planes, oracle(&bytes, 0, 0.0), "{compression:?} tiles {tiles}");
        }
    }
}

#[test]
fn only_the_requested_channels_are_returned_in_the_requested_order() {
    let bytes = file(vec![layer(None, (0, 0), (8, 4), &["R", "G", "B", "A"], false, Compression::ZIP1)], (8, 4));
    let index = indexed(&bytes);
    let names: Vec<&str> = index.parts()[0].channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "G", "R"]);
    let whole = oracle(&bytes, 0, 0.0);
    // R, then A (stored first), the others not read.
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 3), (0, 0)], usize::MAX), &never).unwrap();
    assert_eq!(read.planes, [whole[3].clone(), whole[0].clone()]);
}

#[test]
fn a_part_with_no_requested_channel_is_not_read() {
    let bytes = file(
        vec![
            layer(Some("depth"), (0, 0), (16, 16), &["Z"], false, Compression::ZIP1),
            layer(Some("beauty"), (0, 0), (16, 16), &["R", "G", "B", "A"], true, Compression::PIZ),
        ],
        (16, 16),
    );
    let index = indexed(&bytes);
    assert_eq!(index.parts().len(), 2);
    let depth = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never).unwrap();
    assert_eq!(depth.planes, oracle(&bytes, 0, 0.0));
    assert!(depth.stats.chunks[0] > 0);
    assert_eq!(depth.stats.chunks[1], 0, "the beauty part was never touched");
    // Channels of both parts in one read, in request order.
    let both = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(1, 3), (0, 0)], usize::MAX), &never).unwrap();
    assert_eq!(both.planes, [oracle(&bytes, 1, 0.0)[3].clone(), oracle(&bytes, 0, 0.0)[0].clone()]);
}

#[test]
fn only_the_chunks_holding_the_requested_rows_are_read() {
    // One line per chunk (ZIPS-like layout through ZIP1): rows 4..8 are four chunks.
    let bytes = file(vec![layer(None, (0, 0), (8, 16), &["Y"], false, Compression::ZIP1)], (8, 16));
    let index = indexed(&bytes);
    let whole = oracle(&bytes, 0, 0.0)[0].clone();
    let request = Request { rows: Some(4..8), fill: &[-1.0], ..Request::new(&[(0, 0)], usize::MAX) };
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
    assert_eq!(read.stats.chunks[0], 4);
    for row in 0..16 {
        let got = &read.planes[0][row * 8..row * 8 + 8];
        if (4..8).contains(&row) {
            assert_eq!(got, &whole[row * 8..row * 8 + 8], "row {row}");
        } else {
            assert_eq!(got, [-1.0; 8], "row {row} keeps the fill value");
        }
    }
    // Tiles: rows 5..6 touch the one tile row (4..8) only.
    let tiled = file(vec![layer(None, (0, 0), (8, 16), &["Y"], true, Compression::ZIP1)], (8, 16));
    let tiled_index = indexed(&tiled);
    let request = Request { rows: Some(5..6), ..Request::new(&[(0, 0)], usize::MAX) };
    let read = read_planes(&mut Cursor::new(&tiled[..]), &tiled_index, &request, &never).unwrap();
    assert_eq!(read.stats.chunks[0], 2, "two 4-wide tiles of the one tile row");
    assert_eq!(&read.planes[0][5 * 8..6 * 8], &oracle(&tiled, 0, 0.0)[0][5 * 8..6 * 8]);
}

#[test]
fn a_reduced_level_keeps_every_nth_row_and_column() {
    // 9 × 7: level 1 is 5 × 4, level 2 is 3 × 2.
    for tiles in [false, true] {
        let bytes = file(vec![layer(None, (0, 0), (9, 7), &["Y"], tiles, Compression::ZIP1)], (9, 7));
        let index = indexed(&bytes);
        let whole = oracle(&bytes, 0, 0.0)[0].clone();
        for level in 0..=3u32 {
            let stride = 1usize << level;
            let request = Request { level, ..Request::new(&[(0, 0)], usize::MAX) };
            let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
            assert_eq!((read.width, read.height), level_size((9, 7), level));
            let expected: Vec<f32> =
                (0..7).step_by(stride).flat_map(|y| (0..9).step_by(stride).map(move |x| (y, x))).map(|(y, x)| whole[y * 9 + x]).collect();
            assert_eq!(read.planes[0], expected, "level {level} tiles {tiles}");
        }
    }
}

#[test]
fn a_reduced_level_reads_only_the_chunks_with_a_row_on_its_grid() {
    // One line per chunk: level 2 needs every 4th row of 16.
    let bytes = file(vec![layer(None, (0, 0), (8, 16), &["Y"], false, Compression::ZIP1)], (8, 16));
    let index = indexed(&bytes);
    let request = Request { level: 2, ..Request::new(&[(0, 0)], usize::MAX) };
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
    assert_eq!(read.stats.chunks[0], 4);
}

#[test]
fn a_part_inside_a_larger_display_window_is_placed_and_clipped_like_the_whole_part_decoder() {
    // Data window 3 × 2 at (2, 1) in a 6 × 4 display window; another at (-1, -1) cut by the edge.
    for (at, size) in [((2, 1), (3, 2)), ((-1, -1), (4, 3)), ((5, 3), (4, 4)), ((-9, 0), (2, 2))] {
        let bytes = file(vec![layer(None, at, size, &["R", "G"], false, Compression::ZIP1)], (6, 4));
        let index = indexed(&bytes);
        let channels = channels_of(&index, 0);
        let request = Request { fill: &[9.0, 9.0], ..Request::new(&channels, usize::MAX) };
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
        assert_eq!(read.planes, oracle(&bytes, 0, 9.0), "data window {size:?} at {at:?}");
    }
}

#[test]
fn bands_cover_the_planes_exactly_and_arrive_in_row_order() {
    for tiles in [false, true] {
        let bytes = file(vec![layer(None, (0, 0), (12, 10), &["R", "G"], tiles, Compression::ZIP16)], (12, 10));
        let index = indexed(&bytes);
        let channels = channels_of(&index, 0);
        let mut planes = [vec![f32::NAN; 120], vec![f32::NAN; 120]];
        let mut seen = 0usize;
        // Per slot, the (row, column) of the last band: file order is block row by block row,
        // and left to right within one (tiles), so it only grows.
        let mut last: [Option<(usize, usize)>; 2] = [None, None];
        let stats = read_bands(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never, &mut |band: Band| {
            assert!(last[band.slot] < Some((band.y, band.x)), "slot {}: {:?} after {:?}", band.slot, (band.y, band.x), last[band.slot]);
            last[band.slot] = Some((band.y, band.x));
            assert_eq!(band.samples.len(), band.width * band.height);
            assert!(band.x + band.width <= 12 && band.y + band.height <= 10);
            for r in 0..band.height {
                for c in 0..band.width {
                    planes[band.slot][(band.y + r) * 12 + band.x + c] = band.samples[r * band.width + c];
                    seen += 1;
                }
            }
        })
        .unwrap();
        assert_eq!(seen, 240, "every sample once");
        assert!(stats.chunks[0] > 0 && stats.lines > 0);
        assert_eq!(planes.to_vec(), oracle(&bytes, 0, 0.0), "tiles {tiles}");
    }
}

/// A source that counts the bytes read from it.
struct Counting<'a> {
    inner: Cursor<&'a [u8]>,
    read: &'a Cell<u64>,
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read.set(self.read.get() + n as u64);
        Ok(n)
    }
}

impl Seek for Counting<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn the_first_bands_arrive_before_the_whole_request_has_been_read() {
    // Uncompressed lines of 1024 f32 (one chunk each): about 5 MiB in all, more than the first
    // two batches.
    let bytes = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let read = Cell::new(0u64);
    let mut at_first_band = None;
    let mut source = Counting { inner: Cursor::new(&bytes[..]), read: &read };
    let before = read.get();
    // The rows each channel's bands have reached: they must keep growing (file order).
    let mut reached = [0usize; 2];
    let mut bands = 0usize;
    read_bands(&mut source, &index, &Request::new(&channels, usize::MAX), &never, &mut |band: Band| {
        at_first_band.get_or_insert(read.get() - before);
        assert_eq!(band.y, reached[band.slot], "slot {} out of order", band.slot);
        reached[band.slot] = band.y + band.height;
        bands += 1;
    })
    .unwrap();
    let total = read.get() - before;
    let first = at_first_band.unwrap();
    assert!(first < total / 2, "first band after {first} of {total} bytes");
    assert_eq!((reached, bands), ([600, 600], 1200), "every row of both channels, one band per chunk and channel");
}

#[test]
fn without_threads_the_chunks_are_decoded_on_the_calling_thread_with_the_same_result() {
    // (where threads can't be started, as in WebAssembly, no worker runs)
    let bytes = file(vec![layer(None, (0, 0), (1024, 300), &["R", "G"], false, Compression::Uncompressed)], (1024, 300));
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let whole = oracle(&bytes, 0, 0.0);
    let read = Cell::new(0u64);
    let mut source = Counting { inner: Cursor::new(&bytes[..]), read: &read };
    let mut planes = vec![vec![f32::NAN; 1024 * 300]; 2];
    let mut reached = [0usize; 2];
    let mut at_first_band = None;
    let request = Request { workers: Some(0), ..Request::new(&channels, usize::MAX) };
    let stats = read_bands(&mut source, &index, &request, &never, &mut |band: Band| {
        at_first_band.get_or_insert(read.get());
        assert_eq!(band.y, reached[band.slot], "file order");
        reached[band.slot] = band.y + band.height;
        let plane = &mut planes[band.slot];
        plane[band.y * 1024..(band.y + band.height) * 1024].copy_from_slice(&band.samples);
    })
    .unwrap();
    assert_eq!(planes, whole);
    assert_eq!(stats.chunks[0], 300);
    assert!(at_first_band.unwrap() < stats.bytes / 2, "delivered piece by piece");
}

#[test]
fn a_read_can_be_called_off() {
    let bytes = file(vec![layer(None, (0, 0), (16, 16), &["R"], false, Compression::ZIP1)], (16, 16));
    let index = indexed(&bytes);
    let count = Cell::new(0usize);
    let cancel = || {
        count.set(count.get() + 1);
        count.get() > 1
    };
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &cancel);
    assert_eq!(got.err(), Some(Error::Cancelled));
    let building = Index::build(&mut Cursor::new(&bytes[..]), &Limits::default(), &|| true);
    assert_eq!(building.err(), Some(Error::Cancelled));
}

#[test]
fn a_request_over_the_byte_limit_is_refused_before_anything_is_read() {
    let bytes = file(vec![layer(None, (0, 0), (16, 16), &["R", "G"], false, Compression::ZIP1)], (16, 16));
    let index = indexed(&bytes);
    let read = Cell::new(0u64);
    let mut source = Counting { inner: Cursor::new(&bytes[..]), read: &read };
    // Two planes of 16 × 16 f32 are 2048 bytes.
    let got = read_planes(&mut source, &index, &Request::new(&[(0, 0), (0, 1)], 2047), &never);
    assert!(matches!(got, Err(Error::LimitExceeded(_))), "{got:?}");
    assert_eq!(read.get(), 0, "no chunk was fetched");
    let got = read_planes(&mut source, &index, &Request::new(&[(0, 0), (0, 1)], 2048), &never).unwrap();
    assert_eq!(got.planes.len(), 2);
    // A reduced level counts its smaller planes.
    let request = Request { level: 1, ..Request::new(&[(0, 0), (0, 1)], 512) };
    assert_eq!(read_planes(&mut source, &index, &request, &never).unwrap().width, 8);
}

#[test]
fn requests_for_channels_that_do_not_exist_are_refused() {
    let bytes = file(vec![layer(None, (0, 0), (4, 4), &["R"], false, Compression::ZIP1)], (4, 4));
    let index = indexed(&bytes);
    for channels in [[(1, 0)], [(0, 1)]] {
        let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never);
        assert!(matches!(got, Err(Error::Malformed(_))), "{channels:?}: {got:?}");
    }
    // Nothing requested is an empty read.
    let none = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[], usize::MAX), &never).unwrap();
    assert!(none.planes.is_empty() && none.stats.bytes == 0);
}

#[test]
fn headers_declaring_more_than_the_limits_are_refused_before_any_allocation() {
    // An 8 × 8 data window, but the limits allow 4 × 4 only.
    let bytes = file(vec![layer(None, (0, 0), (8, 8), &["R"], false, Compression::ZIP1)], (8, 8));
    let limits = Limits { max_width: 4, max_height: 4, ..Limits::default() };
    let got = Index::build(&mut Cursor::new(&bytes[..]), &limits, &never);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("part 0")), "{got:?}");
    // A 2 × 2 data window (within the limit) in an 8 × 8 display window over it.
    let small = file(vec![layer(None, (0, 0), (2, 2), &["R"], false, Compression::ZIP1)], (8, 8));
    let limits = Limits { max_pixels: 16, ..Limits::default() };
    let got = Index::build(&mut Cursor::new(&small[..]), &limits, &never);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("display window")), "{got:?}");
    assert!(Index::build(&mut Cursor::new(&small[..]), &Limits { max_pixels: 64, ..Limits::default() }, &never).is_ok());
}

#[test]
fn hostile_files_are_refused_and_never_panic() {
    let bytes = file(
        vec![
            layer(Some("a"), (0, 0), (9, 7), &["R", "G"], false, Compression::ZIP1),
            layer(Some("b"), (1, 1), (5, 5), &["Z"], true, Compression::PIZ),
        ],
        (9, 7),
    );
    let try_read = |data: &[u8]| {
        let Ok(index) = Index::build(&mut Cursor::new(data), &Limits::default(), &never) else { return };
        let channels: Vec<(usize, usize)> = (0..index.parts().len()).flat_map(|p| channels_of(&index, p)).collect();
        let _ = read_planes(&mut Cursor::new(data), &index, &Request::new(&channels, 1 << 24), &never);
    };
    // Every truncation.
    for n in 0..bytes.len() {
        try_read(&bytes[..n]);
    }
    // A flipped byte at every position (headers, tables and chunks).
    for at in 0..bytes.len() {
        let mut broken = bytes.clone();
        broken[at] ^= 0xFF;
        try_read(&broken);
    }
    // Garbage and an empty source.
    try_read(&[]);
    try_read(&[0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0]);
    try_read(&vec![0xAB; 4096]);
}

#[test]
fn headers_over_the_attribute_and_size_bounds_are_refused() {
    let mut bytes = vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0];
    // An attribute that claims 100 MB.
    bytes.extend_from_slice(b"comments\0string\0");
    bytes.extend_from_slice(&100_000_000i32.to_le_bytes());
    bytes.extend_from_slice(&[0; 64]);
    assert!(matches!(Index::build(&mut Cursor::new(&bytes[..]), &Limits::default(), &never), Err(Error::LimitExceeded(_))));
    // A name that never ends.
    let mut bytes = vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0];
    bytes.extend_from_slice(&[b'a'; 1000]);
    assert!(matches!(Index::build(&mut Cursor::new(&bytes[..]), &Limits::default(), &never), Err(Error::LimitExceeded(_))));
}

#[test]
fn a_tiled_file_with_mip_levels_is_read_from_its_largest_level_only() {
    use exr::math::RoundingMode;
    use exr::prelude::Levels;
    // 9 × 7 in 4 × 4 tiles: level 0 is 3 × 2 tiles, then 4 × 3, 2 × 1 and 1 × 1 pixels in one
    // tile each. The smaller levels hold a value no full-resolution pixel has, so a wrong level
    // would show in the planes.
    let (w, h) = (9usize, 7usize);
    let sizes: Vec<usize> = exr::meta::mip_map_levels(RoundingMode::Down, Vec2(w, h)).map(|(_, size)| size.area()).collect();
    assert!(sizes.len() > 1);
    let levels = |k: usize| Levels::Mip {
        rounding_mode: RoundingMode::Down,
        level_data: sizes
            .iter()
            .enumerate()
            .map(|(level, n)| FlatSamples::F32(if level == 0 { (0..*n).map(|i| value(i, k)).collect() } else { vec![1e9; *n] }))
            .collect(),
    };
    let channels = AnyChannels::sort(vec![AnyChannel::new("R", levels(0)), AnyChannel::new("G", levels(1))].into());
    let encoding = Encoding { compression: Compression::ZIP1, blocks: Blocks::Tiles(Vec2(4, 4)), line_order: LineOrder::Increasing };
    let image = exr::image::Image::from_layers(
        ImageAttributes::new(IntegerBounds::new((0, 0), (w, h))),
        vec![Layer::new((w, h), LayerAttributes::default(), encoding, channels)],
    );
    let mut out = Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    let bytes = out.into_inner();
    let index = indexed(&bytes);
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0), (0, 1)], usize::MAX), &never).unwrap();
    // (the part stores G before R)
    let expected = |k: usize| (0..w * h).map(|i| value(i, k)).collect::<Vec<f32>>();
    assert_eq!(read.planes, [expected(1), expected(0)]);
    assert_eq!(read.stats.chunks[0], 6, "the six tiles of level 0, none of the smaller levels");
    // A reduced read takes its rows from the largest level too.
    let half = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request { level: 1, ..Request::new(&[(0, 1)], usize::MAX) }, &never).unwrap();
    let every_other: Vec<f32> = (0..h).step_by(2).flat_map(|y| (0..w).step_by(2).map(move |x| value(y * w + x, 0))).collect();
    assert_eq!(half.planes[0], every_other);
}

#[test]
fn a_flat_part_beside_a_deep_part_is_read_and_the_deep_part_is_refused() {
    use super::decode_tests::{mixed_file, mixed_flat};
    for (deep_first, tiled) in [(false, false), (true, false), (false, true), (true, true)] {
        let (bytes, _) = mixed_file(deep_first, tiled);
        let (flat, deep) = if deep_first { (1, 0) } else { (0, 1) };
        let index = indexed(&bytes);
        assert_eq!((index.parts()[flat].deep, index.parts()[deep].deep), (false, true));
        let names: Vec<&str> = index.parts()[flat].channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["B", "G", "R"]);
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(flat, 2), (flat, 0)], usize::MAX), &never).unwrap();
        let expected = |channel: f32| (0..6).map(|i| mixed_flat(channel, i)).collect::<Vec<f32>>();
        assert_eq!(read.planes, [expected(0.0), expected(100.0)], "deep first {deep_first} tiled {tiled}");
        assert_eq!((read.stats.chunks[flat], read.stats.chunks[deep]), (2, 0), "the deep part's chunks are never read");
        for channels in [vec![(deep, 0)], vec![(flat, 0), (deep, 0)]] {
            let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never);
            assert!(matches!(got, Err(Error::Unsupported(_))), "{channels:?}: {got:?}");
        }
    }
}

#[test]
fn a_subsampled_channel_is_refused() {
    let mut bytes = file(vec![layer(None, (0, 0), (4, 2), &["R"], false, Compression::Uncompressed)], (4, 2));
    // The `exr` crate's writer emits no subsampling: the x sampling of the one channel in the
    // channel list is patched to 2 (the 4 × 2 window is a multiple of it).
    let tag = b"channels\0chlist\0";
    let attr = bytes.windows(tag.len()).position(|window| window == tag).unwrap();
    let payload = attr + tag.len() + 4;
    let name_end = payload + bytes[payload..].iter().position(|byte| *byte == 0).unwrap();
    let x_sampling = name_end + 1 + 4 + 1 + 3;
    bytes[x_sampling..x_sampling + 4].copy_from_slice(&2u32.to_le_bytes());
    let index = indexed(&bytes);
    assert_eq!(index.parts()[0].channels[0].sampling, (2, 1));
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
    assert!(matches!(got, Err(Error::Unsupported(_))), "{got:?}");
}

#[test]
fn half_float_and_integer_channels_convert_like_the_whole_part_decoder() {
    use exr::prelude::f16;
    let (w, h) = (7usize, 5usize);
    let halves: Vec<f16> = (0..w * h).map(|i| f16::from_f32(i as f32 / 8.0 - 2.0)).collect();
    // Integers beyond f32's exact range too (object ids): they round like the oracle's.
    let ids: Vec<u32> = (0..w * h).map(|i| (i as u32).wrapping_mul(2_654_435_761)).collect();
    for tiles in [false, true] {
        let list = vec![
            AnyChannel::new("Y", FlatSamples::F16(halves.clone())),
            AnyChannel::new("id", FlatSamples::U32(ids.clone())),
            AnyChannel::new("Z", FlatSamples::F32(vec![0.5; w * h])),
        ];
        let blocks = if tiles { Blocks::Tiles(Vec2(4, 4)) } else { Blocks::ScanLines };
        let layer = Layer::new(
            (w, h),
            LayerAttributes::default(),
            Encoding { compression: Compression::ZIP16, blocks, line_order: LineOrder::Increasing },
            AnyChannels::sort(list.into()),
        );
        // Placed partly outside the display window, so the conversion and the clipping meet.
        let mut layer = layer;
        layer.attributes.layer_position = Vec2(-2, 1);
        let bytes = file(vec![layer], (6, 5));
        let index = indexed(&bytes);
        let channels = channels_of(&index, 0);
        let request = Request { fill: &[7.0, 7.0, 7.0], ..Request::new(&channels, usize::MAX) };
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
        assert_eq!(read.planes, oracle(&bytes, 0, 7.0), "tiles {tiles}");
        assert!(read.planes.iter().all(|p| p.iter().any(|v| *v != 7.0)));
    }
}

#[test]
fn the_index_reports_its_parts_and_a_size() {
    let bytes = file(vec![layer(Some("beauty"), (0, 0), (8, 8), &["R", "G", "B"], false, Compression::ZIP16)], (8, 8));
    let index = indexed(&bytes);
    assert_eq!(index.parts()[0].name.as_deref(), Some("beauty"));
    assert_eq!((index.display().width, index.display().height), (8, 8));
    assert_eq!(index.file_len(), bytes.len() as u64);
    assert!(index.bytes() > 0 && index.bytes() < 64 << 10, "{}", index.bytes());
}

/// Writes `layers` into a file whose display window is `display` (absolute position and size).
fn file_at(layers: Vec<Layer<AnyChannels<FlatSamples>>>, display: ((i32, i32), (usize, usize))) -> Vec<u8> {
    let image = exr::image::Image::from_layers(ImageAttributes::new(IntegerBounds::new(display.0, display.1)), layers);
    let mut out = Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    out.into_inner()
}

/// Where the offset table of a single-part file with `count` chunks starts: the position whose
/// smallest entry is where the table ends.
fn table_at(bytes: &[u8], count: usize) -> usize {
    let entry = |p: usize| u64::from_le_bytes(bytes[p..p + 8].try_into().unwrap()) as usize;
    (8..bytes.len() - count * 8).find(|p| (0..count).map(|i| entry(p + i * 8)).min() == Some(p + count * 8)).unwrap()
}

/// Sets the box2i attribute `name` (`dataWindow`, `displayWindow`) of a file to `bounds`.
fn patch_box(bytes: &mut [u8], name: &str, bounds: [i32; 4]) {
    let tag = format!("{name}\0box2i\0");
    let at = bytes.windows(tag.len()).position(|w| w == tag.as_bytes()).unwrap() + tag.len() + 4;
    for (i, v) in bounds.iter().enumerate() {
        bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// A tiled 9 × 7 file in 4 × 4 tiles with these levels of one channel `R`.
fn tiled_levels(levels: exr::prelude::Levels<FlatSamples>) -> Vec<u8> {
    let encoding = Encoding { compression: Compression::ZIP1, blocks: Blocks::Tiles(Vec2(4, 4)), line_order: LineOrder::Increasing };
    let channels = AnyChannels::sort(vec![AnyChannel::new("R", levels)].into());
    let image = exr::image::Image::from_layers(
        ImageAttributes::new(IntegerBounds::new((0, 0), (9, 7))),
        vec![Layer::new((9, 7), LayerAttributes::default(), encoding, channels)],
    );
    let mut out = Cursor::new(Vec::new());
    image.write().to_buffered(&mut out).unwrap();
    out.into_inner()
}

#[test]
fn a_channel_requested_twice_is_delivered_in_both_slots() {
    for tiles in [false, true] {
        let bytes = file(vec![layer(None, (0, 0), (9, 7), &["R", "G"], tiles, Compression::ZIP1)], (9, 7));
        let index = indexed(&bytes);
        let whole = oracle(&bytes, 0, 0.0);
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 1), (0, 0), (0, 1)], usize::MAX), &never).unwrap();
        assert_eq!(read.planes, [whole[1].clone(), whole[0].clone(), whole[1].clone()], "tiles {tiles}");
    }
}

#[test]
fn a_block_is_bounded_by_the_size_the_exr_crate_reads_for_it() {
    // One row 4 Mi pixels wide of one f32 channel: 16 MiB a row. PIZ holds 32 rows a block, and
    // the `exr` crate sizes what it reads for a chunk by that (512 MiB) even when the part has one
    // row; ZIP1 holds one row a block (16 MiB).
    for (compression, accepted) in [(Compression::ZIP1, true), (Compression::PIZ, false)] {
        let mut bytes = file(vec![layer(None, (0, 0), (1, 1), &["R"], false, compression)], (1, 1));
        patch_box(&mut bytes, "dataWindow", [0, 0, (4 << 20) - 1, 0]);
        let got = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never);
        if accepted {
            assert!(got.is_ok(), "{compression:?}: {got:?}");
        } else {
            assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("block")), "{compression:?}: {got:?}");
        }
    }
}

#[test]
fn rows_and_band_positions_count_from_the_display_window_s_corner() {
    // Display window 12 × 9 at (10, 20); the data window 8 × 5 at (11, 22), i.e. (1, 2) inside it.
    let bytes = file_at(vec![layer(None, (11, 22), (8, 5), &["Y"], false, Compression::ZIP1)], ((10, 20), (12, 9)));
    let index = indexed(&bytes);
    assert_eq!((index.display().x, index.display().y), (10, 20));
    let whole = oracle(&bytes, 0, -1.0)[0].clone();
    let request = Request { rows: Some(3..6), fill: &[-1.0], ..Request::new(&[(0, 0)], usize::MAX) };
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
    for row in 0..9 {
        let got = &read.planes[0][row * 12..row * 12 + 12];
        if (3..6).contains(&row) {
            assert_eq!(got, &whole[row * 12..row * 12 + 12], "row {row}");
            assert!(got.iter().any(|v| *v != -1.0), "row {row} has samples");
        } else {
            assert_eq!(got, [-1.0; 12], "row {row}");
        }
    }
    let mut placed = Vec::new();
    read_bands(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut |b: Band| placed.push((b.x, b.y, b.width, b.height))).unwrap();
    assert_eq!(placed, [(1, 3, 8, 1), (1, 4, 8, 1), (1, 5, 8, 1)]);
}

#[test]
fn bands_are_bounded_by_the_rows_they_deliver_and_planes_by_the_display_window() {
    let bytes = file(vec![layer(None, (0, 0), (16, 16), &["R", "G"], false, Compression::ZIP1)], (16, 16));
    let index = indexed(&bytes);
    // Rows 4..8 of two channels: 4 × 16 × 2 f32, 512 bytes; the whole planes are 2048.
    let rows = |max_bytes| Request { rows: Some(4..8), ..Request::new(&[(0, 0), (0, 1)], max_bytes) };
    let mut count = 0;
    read_bands(&mut Cursor::new(&bytes[..]), &index, &rows(512), &never, &mut |_| count += 1).unwrap();
    assert_eq!(count, 8);
    let got = read_bands(&mut Cursor::new(&bytes[..]), &index, &rows(511), &never, &mut |_| {});
    assert!(matches!(got, Err(Error::LimitExceeded(_))), "{got:?}");
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &rows(512), &never);
    assert!(matches!(got, Err(Error::LimitExceeded(_))), "read_planes allocates the whole planes: {got:?}");
    // At level 2 rows 4..8 hold one row on the grid (row 4): 4 × 2 f32.
    let reduced = |max_bytes| Request { level: 2, ..rows(max_bytes) };
    read_bands(&mut Cursor::new(&bytes[..]), &index, &reduced(32), &never, &mut |_| {}).unwrap();
    assert!(read_bands(&mut Cursor::new(&bytes[..]), &index, &reduced(31), &never, &mut |_| {}).is_err());
}

#[test]
fn a_header_declaring_more_chunks_than_the_bound_is_refused_before_the_tables_are_read() {
    // One column of 2^24 + 1 rows, one row a chunk: the file can't hold its table, but the count
    // alone is refused first.
    let mut bytes = file(vec![layer(None, (0, 0), (1, 1), &["R"], false, Compression::Uncompressed)], (1, 1));
    patch_box(&mut bytes, "dataWindow", [0, 0, 0, 1 << 24]);
    patch_box(&mut bytes, "displayWindow", [0, 0, 0, 1 << 24]);
    let got = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("chunks")), "{got:?}");
}

/// A source that fails with `kind` once `after` bytes have been read.
struct Failing<'a> {
    inner: Cursor<&'a [u8]>,
    after: u64,
    kind: std::io::ErrorKind,
}

impl Read for Failing<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.inner.position() >= self.after {
            return Err(std::io::Error::new(self.kind, "the share went away"));
        }
        let room = (self.after - self.inner.position()) as usize;
        let n = buf.len().min(room);
        self.inner.read(&mut buf[..n])
    }
}

impl Seek for Failing<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn a_failing_source_is_an_io_error_and_a_short_file_is_malformed() {
    let bytes = file(vec![layer(None, (0, 0), (8, 8), &["R"], false, Compression::ZIP1)], (8, 8));
    for after in [4u64, 40, 200] {
        let mut source = Failing { inner: Cursor::new(&bytes[..]), after, kind: std::io::ErrorKind::Other };
        let got = Index::build(&mut source, &Limits::default(), &never);
        assert!(matches!(got, Err(Error::Io(_))), "after {after}: {got:?}");
        let got = Index::build(&mut Cursor::new(&bytes[..after as usize]), &Limits::default(), &never);
        assert!(matches!(got, Err(Error::Malformed(_))), "cut at {after}: {got:?}");
    }
}

#[test]
fn a_read_called_off_while_workers_hold_chunks_stops() {
    let bytes = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let stop = Cell::new(false);
    let mut bands = 0usize;
    let request = Request { workers: Some(2), ..Request::new(&channels, usize::MAX) };
    let got = read_bands(&mut Cursor::new(&bytes[..]), &index, &request, &|| stop.get(), &mut |_| {
        bands += 1;
        stop.set(true);
    });
    assert_eq!(got.err(), Some(Error::Cancelled));
    assert!(bands > 0 && bands < 1200, "{bands} bands");
}

#[test]
fn a_part_written_bottom_up_arrives_bottom_up() {
    let mut part = layer(None, (0, 0), (8, 12), &["Y"], false, Compression::ZIP1);
    part.encoding.line_order = LineOrder::Decreasing;
    let bytes = file(vec![part], (8, 12));
    let index = indexed(&bytes);
    let mut rows = Vec::new();
    let mut plane = vec![f32::NAN; 96];
    read_bands(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never, &mut |b: Band| {
        rows.push(b.y);
        plane[b.y * 8..(b.y + b.height) * 8].copy_from_slice(&b.samples);
    })
    .unwrap();
    assert_eq!(rows, (0..12).rev().collect::<Vec<_>>(), "file order");
    assert_eq!(vec![plane], oracle(&bytes, 0, 0.0));
}

#[test]
fn a_tiled_file_with_rip_levels_is_read_from_its_largest_level_only() {
    use exr::image::RipMaps;
    use exr::math::RoundingMode;
    use exr::prelude::Levels;
    let (w, h) = (9usize, 7usize);
    let sizes: Vec<usize> = exr::meta::rip_map_levels(RoundingMode::Down, Vec2(w, h)).map(|(_, size)| size.area()).collect();
    let level_count = Vec2(exr::meta::compute_level_count(RoundingMode::Down, w), exr::meta::compute_level_count(RoundingMode::Down, h));
    assert!(sizes.len() > 1);
    let map_data = sizes
        .iter()
        .enumerate()
        .map(|(level, n)| FlatSamples::F32(if level == 0 { (0..*n).map(|i| value(i, 0)).collect() } else { vec![1e9; *n] }))
        .collect();
    let bytes = tiled_levels(Levels::Rip { rounding_mode: RoundingMode::Down, level_data: RipMaps { map_data, level_count } });
    let index = indexed(&bytes);
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never).unwrap();
    assert_eq!(read.planes[0], (0..w * h).map(|i| value(i, 0)).collect::<Vec<f32>>());
    assert_eq!(read.stats.chunks[0], 6);
}

#[test]
fn rows_an_odd_data_window_and_a_level_together_on_several_threads() {
    // A 37 × 29 data window at (3, 5) in a 50 × 40 display window, one row a chunk.
    let bytes = file(vec![layer(None, (3, 5), (37, 29), &["R", "G"], false, Compression::ZIP1)], (50, 40));
    let index = indexed(&bytes);
    let whole = oracle(&bytes, 0, 9.0);
    for level in [0u32, 1, 2] {
        let stride = 1usize << level;
        let request = Request { rows: Some(7..33), level, fill: &[9.0, 9.0], workers: Some(4), ..Request::new(&[(0, 1), (0, 0)], usize::MAX) };
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
        for (slot, channel) in [(0, 1), (1, 0)] {
            let expected: Vec<f32> = (0..40)
                .step_by(stride)
                .flat_map(|y| (0..50).step_by(stride).map(move |x| (y, x)))
                .map(|(y, x)| if (7..33).contains(&y) { whole[channel][y * 50 + x] } else { 9.0 })
                .collect();
            assert_eq!(read.planes[slot], expected, "level {level} slot {slot}");
        }
    }
}

#[test]
fn a_damaged_chunk_in_the_middle_ends_the_read_after_the_bands_before_it() {
    // 600 uncompressed chunks of 8200 bytes (y, size, 2 × 1024 f32), the last at the file's end:
    // row 400's chunk claims another row. The read spans several pieces and threads.
    let mut bytes = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let at = bytes.len() - (600 - 400) * 8200;
    assert_eq!(&bytes[at..at + 4], &400i32.to_le_bytes());
    bytes[at..at + 4].copy_from_slice(&7i32.to_le_bytes());
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let mut reached = [0usize; 2];
    let request = Request { workers: Some(4), ..Request::new(&channels, usize::MAX) };
    let got = read_bands(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut |b: Band| {
        assert_eq!(b.y, reached[b.slot]);
        reached[b.slot] = b.y + b.height;
    });
    assert!(matches!(got, Err(Error::Malformed(_))), "{got:?}");
    assert_eq!(reached, [400, 400], "every row before the damaged chunk, nothing after it");
}

#[test]
fn a_chunk_that_is_not_the_block_its_table_entry_stands_for_is_refused() {
    // Scan lines: the entries of rows 0 and 1 swapped; reading row 0 fetches row 1's chunk.
    let mut bytes = file(vec![layer(None, (0, 0), (8, 16), &["Y"], false, Compression::ZIP1)], (8, 16));
    let table = table_at(&bytes, 16);
    let (a, b) = (bytes[table..table + 8].to_vec(), bytes[table + 8..table + 16].to_vec());
    bytes[table..table + 8].copy_from_slice(&b);
    bytes[table + 8..table + 16].copy_from_slice(&a);
    let index = indexed(&bytes);
    let request = Request { rows: Some(0..1), ..Request::new(&[(0, 0)], usize::MAX) };
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never);
    assert!(matches!(got, Err(Error::Malformed(_))), "{got:?}");
    // Mip-mapped tiles: the first full-resolution tile's entry points at a smaller level's tile.
    use exr::math::RoundingMode;
    use exr::prelude::Levels;
    let sizes: Vec<usize> = exr::meta::mip_map_levels(RoundingMode::Down, Vec2(9, 7)).map(|(_, size)| size.area()).collect();
    let mut mip =
        tiled_levels(Levels::Mip { rounding_mode: RoundingMode::Down, level_data: sizes.iter().map(|n| FlatSamples::F32(vec![1.0; *n])).collect() });
    let table = table_at(&mip, 6 + sizes.len() - 1);
    let smaller = mip[table + 6 * 8..table + 7 * 8].to_vec();
    mip[table..table + 8].copy_from_slice(&smaller);
    let index = indexed(&mip);
    let got = read_planes(&mut Cursor::new(&mip[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
    assert!(matches!(got, Err(Error::Malformed(_))), "{got:?}");
}

#[test]
fn the_decoded_bytes_in_flight_stay_within_the_budget_however_well_the_file_compresses() {
    // A flat colour in ZIP1 lines: each 1024-pixel line of two f32 channels compresses to a few
    // dozen bytes but decodes to 8 KiB (and 8 KiB of bands), so a 256 KiB piece would hold
    // thousands of them.
    let (w, h) = (1024usize, 600usize);
    let list = vec![AnyChannel::new("R", FlatSamples::F32(vec![0.25; w * h])), AnyChannel::new("G", FlatSamples::F32(vec![0.5; w * h]))];
    let encoding = Encoding { compression: Compression::ZIP1, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    let bytes = file(vec![Layer::new((w, h), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (w, h));
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let budget = 64 << 10;
    let mut rows = 0usize;
    let request = Request { workers: Some(4), ..Request::new(&channels, usize::MAX) };
    let mut sink = |b: Band| {
        assert!(b.samples.iter().all(|v| *v == 0.25 || *v == 0.5));
        rows += b.height;
    };
    let stats = super::seek::read_bands_within(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut sink, budget).unwrap();
    assert_eq!(rows, 2 * h);
    assert!(stats.bytes < 64 << 10, "{} bytes: well compressed", stats.bytes);
    assert!(stats.in_flight > 0 && stats.in_flight <= budget, "{} decoded bytes in flight", stats.in_flight);
}
