//! Tests of [`crate::seek`]: files are built with the `exr` crate and read back through the
//! seekable reader; the whole-part decoder ([`crate::decode`]) is the oracle for what a channel
//! should hold.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// (lists of channel groups with one group are arrays of one range on purpose)
#![allow(clippy::single_range_in_vec_init)]

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
fn a_block_is_bounded_by_the_bytes_the_decompressor_allocates_for_it() {
    // One row 4 Mi pixels wide of one f32 channel: 16 MiB. PIZ holds 32 rows a block, but a
    // one-row part's block is that row; 16 such rows of four channels in one ZIP16 block are 1 GiB.
    // Both index; a read refuses the 1 GiB block unless its `max_bytes` allows that much.
    // (`cancel` stops each read that passes the check before a chunk is fetched.)
    let stop = || true;
    for (compression, (w, h), channels, big) in
        [(Compression::PIZ, (4 << 20, 1), &["R"][..], false), (Compression::ZIP16, (4 << 20, 16), &["R", "G", "B", "A"][..], true)]
    {
        let mut bytes = file(vec![layer(None, (0, 0), (1, h), channels, false, compression)], (1, h));
        patch_box(&mut bytes, "dataWindow", [0, 0, w - 1, h as i32 - 1]);
        let index = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never).unwrap();
        let read = |max_bytes| read_bands(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], max_bytes), &stop, &mut |_| {});
        let got = read(256 << 20);
        if big {
            assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("block")), "{compression:?}: {got:?}");
            assert_eq!(read(1 << 30).err(), Some(Error::Cancelled), "{compression:?}: allowed with max_bytes 1 GiB");
        } else {
            assert_eq!(got.err(), Some(Error::Cancelled), "{compression:?}");
        }
    }
}

#[test]
fn a_wide_dwab_part_with_half_a_gigabyte_blocks_reads_only_with_a_budget_that_allows_them() {
    // 16K wide, 256 lines (one DWAB block), 64 half channels: 128 bytes a pixel, 512 MiB a block.
    let mut bytes = many_channels(1, 1, 64, true, Compression::DWAB(None));
    patch_box(&mut bytes, "dataWindow", [0, 0, 16383, 255]);
    patch_box(&mut bytes, "displayWindow", [0, 0, 16383, 255]);
    let index = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never).unwrap();
    assert_eq!(index.chunks(0).map(|c| (c.width, c.height)).collect::<Vec<_>>(), [(16384, 256)]);
    let stop = || true;
    let read = |max_bytes| read_bands(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], max_bytes), &stop, &mut |_| {});
    let got = read(256 << 20);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("block")), "{got:?}");
    assert_eq!(read((512 << 20) - 1).err().map(|e| matches!(e, Error::LimitExceeded(_))), Some(true));
    assert_eq!(read(512 << 20).err(), Some(Error::Cancelled), "a 512 MiB block within max_bytes passes the check");
    assert_eq!(read(1 << 30).err(), Some(Error::Cancelled));
}

// (budgets over 4 GiB: a 64-bit usize)
#[cfg(target_pointer_width = "64")]
#[test]
fn a_block_is_held_to_max_block_bytes_whatever_the_total_budget() {
    // The same 512 MiB DWAB block, read with a 64 GiB budget: `max_block_bytes` decides. The
    // source panics if a chunk is fetched, so a missing check fails at once.
    let mut bytes = many_channels(1, 1, 64, true, Compression::DWAB(None));
    patch_box(&mut bytes, "dataWindow", [0, 0, 16383, 255]);
    patch_box(&mut bytes, "displayWindow", [0, 0, 16383, 255]);
    let index = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never).unwrap();
    let chunks = index.chunks(0).next().unwrap().offset;
    let read = |max_block_bytes, cancel: &dyn Fn() -> bool| {
        let request = Request { max_block_bytes, ..Request::new(&[(0, 0)], 64 << 30) };
        read_bands(&mut NoChunks { inner: Cursor::new(&bytes[..]), chunks }, &index, &request, cancel, &mut |_| {})
    };
    let got = read(Some(256 << 20), &never);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("one block")), "{got:?}");
    // (cancelled right after the check, before a chunk is fetched)
    assert_eq!(read(Some(1 << 30), &|| true).err(), Some(Error::Cancelled), "a 1 GiB block limit passes the check");
    // `None`: max(256 MiB, max_bytes), here 64 GiB.
    assert_eq!(read(None, &|| true).err(), Some(Error::Cancelled));
}

#[test]
fn a_chunk_over_the_in_flight_budget_is_decoded_alone() {
    // 64 rows of one f32 channel, 16 wide, one row a chunk: each chunk holds a 64-byte block and
    // 64 bytes of bands, 128 bytes in flight; with a budget of 100 no two may be in flight at
    // once, however many workers there are.
    let bytes = file(vec![layer(None, (0, 0), (16, 64), &["R"], false, Compression::ZIP1)], (16, 64));
    let index = indexed(&bytes);
    let whole = oracle(&bytes, 0, 0.0);
    for _ in 0..20 {
        let mut plane = vec![f32::NAN; 16 * 64];
        let request = Request { workers: Some(8), ..Request::new(&[(0, 0)], usize::MAX) };
        let mut sink = |b: Band| plane[b.y * 16..(b.y + b.height) * 16].copy_from_slice(&b.samples);
        let stats = super::seek::read_bands_within(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut sink, 100).unwrap();
        assert_eq!(stats.in_flight, 128, "one chunk at a time");
        assert_eq!(vec![plane], whole);
    }
}

/// A file of `channels` channels named `c0`, `c1`, … of `w × h` pixels, f32 or half.
fn many_channels(w: usize, h: usize, channels: usize, half: bool, compression: Compression) -> Vec<u8> {
    use exr::prelude::f16;
    let list: Vec<AnyChannel<FlatSamples>> = (0..channels)
        .map(|k| {
            let samples = (0..w * h).map(|i| ((i * 3 + k * 7) % 50) as f32 / 8.0);
            let samples = if half { FlatSamples::F16(samples.map(f16::from_f32).collect()) } else { FlatSamples::F32(samples.collect()) };
            AnyChannel::new(format!("c{k:03}").as_str(), samples)
        })
        .collect();
    let encoding = Encoding { compression, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    file(vec![Layer::new((w, h), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (w, h))
}

#[test]
fn wide_dwab_parts_with_many_channels_are_accepted() {
    // DWAB holds 256 scan lines a block, and the `exr` crate's bound on a chunk counts all 256
    // even in a part with fewer rows: over 256 MiB for these, though each block is a few MiB.
    let dwab = Compression::DWAB(None);
    for (w, h, channels, half) in [(65536, 1, 16, false), (4096, 8, 70, false), (8192, 4, 65, true)] {
        let mut bytes = many_channels(1, 1, channels, half, dwab);
        patch_box(&mut bytes, "dataWindow", [0, 0, w - 1, h - 1]);
        let got = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never);
        assert!(got.is_ok(), "{w} × {h}, {channels} channels: {got:?}");
    }
    // A real one: one row 4096 wide of 65 f32 channels (the crate's bound: 256 × 1 MiB).
    let bytes = many_channels(4096, 1, 65, false, dwab);
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never).unwrap();
    assert_eq!(read.planes, oracle(&bytes, 0, 0.0));
}

#[test]
fn a_chunk_declaring_more_bytes_than_it_holds_is_refused_before_the_crate_reads_it() {
    // A one-pixel-wide part in 4096 × 16 tiles: each chunk holds 16 pixels, but the `exr` crate
    // would allocate whatever size up to a whole tile (256 KiB) a chunk declares.
    let part = layer(None, (0, 0), (1, 32), &["R"], false, Compression::ZIP1);
    let part =
        Layer::new((1, 32), LayerAttributes::default(), Encoding { blocks: Blocks::Tiles(Vec2(4096, 16)), ..part.encoding }, part.channel_data);
    let good = file(vec![part], (1, 32));
    let table = table_at(&good, 2);
    let chunk = u64::from_le_bytes(good[table..table + 8].try_into().unwrap()) as usize;
    assert!(indexed(&good).chunks(0).all(|c| (c.width, c.height) == (1, 16)));
    for size in [4096 * 16 * 4 - 1, -1i32] {
        let mut bytes = good.clone();
        // (tile column, row, level x and y, then the size)
        bytes[chunk + 16..chunk + 20].copy_from_slice(&size.to_le_bytes());
        let index = indexed(&bytes);
        let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
        assert!(matches!(&got, Err(Error::Malformed(m)) if m.starts_with("a chunk declaring")), "size {size}: {got:?}");
    }
}

#[test]
fn a_chunk_naming_another_part_is_refused() {
    use super::decode_tests::mixed_file;
    // The flat part's first chunk names the deep part.
    let (mut bytes, offsets) = mixed_file(false, false);
    bytes[offsets[0][0]..offsets[0][0] + 4].copy_from_slice(&1i32.to_le_bytes());
    let index = indexed(&bytes);
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
    assert!(matches!(&got, Err(Error::Malformed(m)) if m.contains("another part")), "{got:?}");
}

#[test]
fn a_failed_or_cancelled_read_still_reports_how_far_it_got() {
    // Row 400's chunk of 600 claims another row (see the damaged-chunk test).
    let mut bytes = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let at = bytes.len() - (600 - 400) * 8200;
    bytes[at..at + 4].copy_from_slice(&7i32.to_le_bytes());
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let request = Request { workers: Some(4), ..Request::new(&channels, usize::MAX) };
    let mut stats = Stats::default();
    let got = read_bands_with_stats(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut |_| {}, &mut stats);
    assert!(matches!(got, Err(Error::Malformed(_))), "{got:?}");
    assert_eq!(stats.lines, 400, "the lines delivered before the damaged chunk");
    assert!(stats.chunks[0] > 400 && stats.bytes >= 400 * 8200, "{stats:?}");
    // Called off after the first band.
    let stop = Cell::new(false);
    let got = read_bands_with_stats(&mut Cursor::new(&bytes[..]), &index, &request, &|| stop.get(), &mut |_| stop.set(true), &mut stats);
    assert_eq!(got.err(), Some(Error::Cancelled));
    assert!(stats.lines > 0 && stats.lines < 600 && stats.chunks[0] > 0, "{stats:?}");
}

#[test]
fn the_index_lists_the_chunks_of_the_full_resolution_level() {
    use super::decode_tests::mixed_file;
    use exr::math::RoundingMode;
    use exr::prelude::Levels;
    let sizes: Vec<usize> = exr::meta::mip_map_levels(RoundingMode::Down, Vec2(9, 7)).map(|(_, size)| size.area()).collect();
    let bytes =
        tiled_levels(Levels::Mip { rounding_mode: RoundingMode::Down, level_data: sizes.iter().map(|n| FlatSamples::F32(vec![1.0; *n])).collect() });
    let index = indexed(&bytes);
    let chunks: Vec<ChunkInfo> = index.chunks(0).collect();
    let tiles: Vec<(usize, usize, usize, usize)> = chunks.iter().map(|c| (c.x, c.y, c.width, c.height)).collect();
    assert_eq!(tiles, [(0, 0, 4, 4), (4, 0, 4, 4), (8, 0, 1, 4), (0, 4, 4, 3), (4, 4, 4, 3), (8, 4, 1, 3)]);
    assert!(chunks.iter().all(|c| c.len > 0 && c.offset > 0 && c.offset + c.len <= bytes.len() as u64));
    assert_eq!(index.chunks(1).count(), 0, "no such part");
    let (mixed, offsets) = mixed_file(true, false);
    let index = indexed(&mixed);
    assert_eq!(index.chunks(0).count(), 0, "a deep part");
    assert_eq!(index.chunks(1).map(|c| (c.offset as usize, c.y, c.height)).collect::<Vec<_>>(), [(offsets[1][0], 0, 1), (offsets[1][1], 1, 1)]);
}

#[test]
fn htj2k_compression_is_its_own_error() {
    let mut bytes = file(vec![layer(None, (0, 0), (4, 4), &["R"], false, Compression::ZIP1)], (4, 4));
    let tag = b"compression\0compression\0";
    let at = bytes.windows(tag.len()).position(|w| w == tag).unwrap() + tag.len() + 4;
    for code in [10u8, 11] {
        bytes[at] = code;
        let got = Index::build(&mut Cursor::new(&bytes[..]), &Limits::default(), &never);
        assert_eq!(got.err(), Some(Error::UnsupportedCompression("HTJ2K".into())), "code {code}");
    }
}

#[test]
fn the_display_window_may_have_its_own_pixel_limit() {
    // A 16 × 16 data window (overscan) over an 8 × 8 display window.
    let bytes = file(vec![layer(None, (-4, -4), (16, 16), &["R"], false, Compression::ZIP1)], (8, 8));
    let limits = Limits { max_pixels: 256, ..Limits::default() };
    let build = |display| Index::build_with_display_limit(&mut Cursor::new(&bytes[..]), &limits, display, &never);
    assert!(build(64).is_ok());
    assert!(matches!(&build(63), Err(Error::LimitExceeded(m)) if m.contains("display window")), "{:?}", build(63));
    // The data windows stay bound by `max_pixels`.
    let tight = Limits { max_pixels: 255, ..Limits::default() };
    let got = Index::build_with_display_limit(&mut Cursor::new(&bytes[..]), &tight, 1 << 20, &never);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("part 0")), "{got:?}");
    // `build` holds both to `max_pixels`.
    assert!(Index::build(&mut Cursor::new(&bytes[..]), &Limits { max_pixels: 63, ..Limits::default() }, &never).is_err());
    assert!(Index::build(&mut Cursor::new(&bytes[..]), &limits, &never).is_ok());
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

#[test]
fn cancel_is_polled_before_each_chunk_reaches_the_sink() {
    // Decoded on the calling thread, the first piece's ~30 chunks are ready at once; a cancel
    // the sink raises after the first chunk's two bands stops the rest.
    let bytes = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let stop = Cell::new(false);
    let mut bands = 0usize;
    let request = Request { workers: Some(0), ..Request::new(&channels, usize::MAX) };
    let got = read_bands(&mut Cursor::new(&bytes[..]), &index, &request, &|| stop.get(), &mut |_| {
        bands += 1;
        stop.set(true);
    });
    assert_eq!(got.err(), Some(Error::Cancelled));
    assert_eq!(bands, 2, "one chunk (both channels), nothing after the cancel");
}

#[test]
fn a_chunk_offset_into_the_headers_is_outside_the_chunk_area() {
    let mut bytes = file(vec![layer(None, (0, 0), (8, 16), &["Y"], false, Compression::ZIP1)], (8, 16));
    let table = table_at(&bytes, 16);
    bytes[table..table + 8].copy_from_slice(&8u64.to_le_bytes());
    let index = indexed(&bytes);
    assert_eq!(index.chunks(0).next().map(|c| c.len), Some(0));
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
    assert!(matches!(&got, Err(Error::Malformed(m)) if m.contains("outside the chunk area")), "{got:?}");
}

/// A single-part scan-line DWA file of `w × h` with these half channels (a ramp each).
fn dwa_file(compression: Compression, (w, h): (usize, usize), names: &[&str], tiles: bool) -> Vec<u8> {
    use exr::prelude::f16;
    let list: Vec<AnyChannel<FlatSamples>> = names
        .iter()
        .enumerate()
        .map(|(k, n)| AnyChannel::new(*n, FlatSamples::F16((0..w * h).map(|i| f16::from_f32(((i % w) + k) as f32 / 64.0)).collect())))
        .collect();
    let blocks = if tiles { Blocks::Tiles(Vec2(16, 16)) } else { Blocks::ScanLines };
    let encoding = Encoding { compression, blocks, line_order: LineOrder::Increasing };
    file(vec![Layer::new((w, h), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (w, h))
}

/// `bytes` with DWA counter `field` (0: version … 9: DC count) of the first chunk set to `value`.
fn with_dwa_counter(bytes: &[u8], field: usize, value: u64) -> Vec<u8> {
    let index = indexed(bytes);
    let chunk = index.chunks(0).next().unwrap();
    let at = chunk.offset as usize;
    // (y, then the payload size: compressed, so it's no raw copy of the 64 × 32 half block)
    assert_ne!(i32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()), 64 * 32 * 2);
    let mut out = bytes.to_vec();
    out[at + 8 + 8 * field..at + 16 + 8 * field].copy_from_slice(&value.to_le_bytes());
    out
}

/// Every reader of the crate refuses `bytes` as damaged, and none aborts.
fn refused_by_every_reader(bytes: &[u8], what: &str) {
    use crate::decode::{Error as DecodeError, decode_display_part, decode_part};
    let index = indexed(bytes);
    let got = read_planes(&mut Cursor::new(bytes), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
    assert!(matches!(&got, Err(Error::Malformed(m)) if m.contains("DWA")), "seek, {what}: {got:?}");
    let got = decode_display_part(bytes, 0, &|_| true, &|_| 0.0, &Limits::default()).map(|_| ());
    assert!(matches!(&got, Err(DecodeError::Malformed(m)) if m.contains("DWA")), "decode_display_part, {what}: {got:?}");
    let got = decode_part(bytes, 0, &|_| true, &Limits::default()).map(|_| ());
    assert!(matches!(&got, Err(DecodeError::Malformed(m)) if m.contains("DWA")), "decode_part, {what}: {got:?}");
    let got = crate::stream::Decoder::new(Cursor::new(bytes), &never).and_then(|mut d| {
        while d.next_band(&never)?.is_some() {}
        Ok(())
    });
    assert!(matches!(&got, Err(DecodeError::Malformed(m)) if m.contains("DWA")), "stream, {what}: {got:?}");
}

#[test]
fn a_dwa_chunk_declaring_2_to_the_44_dc_values_is_refused_not_an_abort() {
    // The case that aborted the process: DWAA 64 × 32, one half channel R, DC count 2^44.
    let bytes = with_dwa_counter(&dwa_file(Compression::DWAA(None), (64, 32), &["R"], false), 9, 1 << 44);
    refused_by_every_reader(&bytes, "DC count 2^44");
}

#[test]
fn a_dwa_chunk_just_above_the_data_window_is_checked_as_the_block_the_decoder_rounds_it_into() {
    // The `exr` crate divides a scan line's offset from the data window toward zero, so a chunk
    // at y = -1 (DWAA, 32 lines) or -255 (DWAB, 256 lines) becomes block 0 and is decompressed.
    use crate::decode::{Error as DecodeError, decode_display_part, decode_part};
    for (compression, y) in [(Compression::DWAA(None), -1i32), (Compression::DWAA(None), -31), (Compression::DWAB(None), -255)] {
        let mut bytes = with_dwa_counter(&dwa_file(compression, (64, 32), &["R"], false), 9, 1 << 40);
        let at = indexed(&bytes).chunks(0).next().unwrap().offset as usize;
        bytes[at..at + 4].copy_from_slice(&y.to_le_bytes());
        let got = decode_part(&bytes, 0, &|_| true, &Limits::default()).map(|_| ());
        assert!(matches!(&got, Err(DecodeError::Malformed(m)) if m.contains("DWA")), "decode_part, {compression:?} y {y}: {got:?}");
        let got = decode_display_part(&bytes, 0, &|_| true, &|_| 0.0, &Limits::default()).map(|_| ());
        assert!(matches!(&got, Err(DecodeError::Malformed(m)) if m.contains("DWA")), "decode_display_part, {compression:?} y {y}: {got:?}");
    }
}

#[test]
fn a_dwa_chunk_the_scan_cannot_read_is_refused_not_left_to_the_decoder() {
    use crate::decode::{Error as DecodeError, decode_part};
    let good = dwa_file(Compression::DWAA(None), (64, 32), &["R"], false);
    let table = table_at(&good, 1);
    // The entry points past the end of the file, and into the last bytes of it.
    for offset in [good.len() as u64 + 100, good.len() as u64 - 6] {
        let mut bytes = good.clone();
        bytes[table..table + 8].copy_from_slice(&offset.to_le_bytes());
        let got = decode_part(&bytes, 0, &|_| true, &Limits::default()).map(|_| ());
        assert!(matches!(&got, Err(DecodeError::Malformed(m)) if m.contains("DWA")), "offset {offset}: {got:?}");
    }
}

#[test]
fn a_chunk_declaring_a_small_payload_before_padding_is_held_to_what_the_payload_can_hold() {
    // A 1 × 1 PIZ file patched to 16 Mi pixels wide (a 64 MiB block) with 100 KiB of padding
    // after its one chunk: the gap to the file's end could hold the block at 1024 : 1, the few
    // dozen bytes the chunk declares can't.
    let mut bytes = file(vec![layer(None, (0, 0), (1, 1), &["R"], false, Compression::PIZ)], (1, 1));
    patch_box(&mut bytes, "dataWindow", [0, 0, (16 << 20) - 1, 0]);
    bytes.extend(std::iter::repeat_n(0u8, 100 << 10));
    let limits = Limits { max_ratio: 1024, ..Limits::none() };
    let index = Index::build(&mut Cursor::new(&bytes[..]), &limits, &never).unwrap();
    assert_eq!(index.check_blocks(), Ok(()), "the gap is enough");
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], 1 << 30), &never);
    assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("can hold")), "{got:?}");
}

#[test]
fn every_dwa_counter_the_inflater_sizes_by_is_bounded_by_the_block() {
    for compression in [Compression::DWAA(None), Compression::DWAB(None)] {
        // (fields: 1 unknown uncompressed size, 6 RLE uncompressed size, 7 RLE raw size, 8 AC
        // count, 9 DC count; a value just over each bound, and a huge one)
        let good = dwa_file(compression, (64, 32), &["R"], false);
        // The block: 64 × 32 half, 4096 bytes; 8 × 4 DCT blocks of one channel.
        for (field, over) in [(1, 4097u64), (6, 8193), (7, 4097), (8, 32 * 63 + 1), (9, 33)] {
            for value in [over, 1 << 44, i64::MAX as u64] {
                refused_by_every_reader(&with_dwa_counter(&good, field, value), &format!("{compression:?} counter {field} = {value}"));
            }
        }
    }
}

#[test]
fn valid_dwa_files_pass_the_guard() {
    // Odd sizes (blocks padded to 8 × 8 for the DCT), lossy and lossless channels, one-row and
    // one-column parts, scan lines and tiles.
    for compression in [Compression::DWAA(None), Compression::DWAB(None)] {
        for (size, names, tiles) in [
            ((1, 1), &["R"][..], false),
            ((9, 7), &["B", "G", "R"][..], false),
            ((1, 300), &["Y", "A"][..], false),
            ((300, 1), &["R", "G", "B", "A", "Z"][..], false),
            ((64, 32), &["R"][..], false),
            ((37, 41), &["R", "G", "B", "A"][..], true),
        ] {
            let bytes = dwa_file(compression, size, names, tiles);
            let index = indexed(&bytes);
            assert_eq!(index.check_blocks(), Ok(()));
            let channels = channels_of(&index, 0);
            let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never);
            assert_eq!(read.map(|r| r.planes).ok(), Some(oracle(&bytes, 0, 0.0)), "{compression:?} {size:?} {names:?} tiles {tiles}");
            assert!(crate::decode::decode_part(&bytes, 0, &|_| true, &Limits::default()).is_ok());
            if !tiles {
                let mut stream = crate::stream::Decoder::new(Cursor::new(&bytes[..]), &never);
                if let Ok(d) = stream.as_mut() {
                    while d.next_band(&never).unwrap().is_some() {}
                }
            }
        }
    }
}

/// A source that panics when anything at or past `chunks` (where the chunks start) is read: a
/// read that a check should have refused fails fast instead of allocating.
#[cfg(target_pointer_width = "64")]
struct NoChunks<'a> {
    inner: Cursor<&'a [u8]>,
    chunks: u64,
}

#[cfg(target_pointer_width = "64")]
impl Read for NoChunks<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        assert!(self.inner.position() < self.chunks, "a chunk was read");
        self.inner.read(buf)
    }
}

#[cfg(target_pointer_width = "64")]
impl Seek for NoChunks<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

// (budgets over 4 GiB: a 64-bit usize)
#[cfg(target_pointer_width = "64")]
#[test]
fn a_tiny_chunk_declaring_a_gigabyte_block_is_refused_before_it_is_read() {
    // A 1 × 1 file whose data window is patched to 2^30 - 2 pixels wide: one row of one f32
    // channel is a 4 GiB block over a chunk of a few dozen bytes. Within an app's 8 GiB budget
    // the block cap allows it; the bytes can't hold it.
    let limits = Limits { max_ratio: Limits::default().max_ratio, ..Limits::none() };
    for compression in [Compression::PIZ, Compression::DWAB(None), Compression::ZIP16] {
        let mut bytes = file(vec![layer(None, (0, 0), (1, 1), &["R"], false, compression)], (1, 1));
        patch_box(&mut bytes, "dataWindow", [0, 0, (1 << 30) - 3, 0]);
        let index = Index::build(&mut Cursor::new(&bytes[..]), &limits, &never).unwrap();
        let chunk = index.chunks(0).next().unwrap();
        assert!(chunk.len < 200 && chunk.width == (1 << 30) - 2, "{chunk:?}");
        let got = index.check_blocks();
        assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("can hold")), "{compression:?}: {got:?}");
        let mut source = NoChunks { inner: Cursor::new(&bytes[..]), chunks: chunk.offset };
        let got = read_planes(&mut source, &index, &Request::new(&[(0, 0)], 8 << 30), &never);
        assert!(matches!(&got, Err(Error::LimitExceeded(m)) if m.contains("can hold")), "{compression:?}: {got:?}");
    }
}

#[test]
fn black_frames_that_compress_extremely_well_still_read() {
    // 4096 × 256 × 4 half channels of zero in DWAB (one 8 MiB block) and of f32 in ZIP16.
    use exr::prelude::f16;
    let (w, h) = (4096usize, 256usize);
    for (compression, half) in [(Compression::DWAB(None), true), (Compression::ZIP16, false)] {
        let list: Vec<AnyChannel<FlatSamples>> = ["R", "G", "B", "A"]
            .iter()
            .map(|n| AnyChannel::new(*n, if half { FlatSamples::F16(vec![f16::ZERO; w * h]) } else { FlatSamples::F32(vec![0.0; w * h]) }))
            .collect();
        let encoding = Encoding { compression, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
        let bytes = file(vec![Layer::new((w, h), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (w, h));
        let index = indexed(&bytes);
        let smallest = index.chunks(0).map(|c| c.len).min().unwrap();
        assert_eq!(index.check_blocks(), Ok(()), "{compression:?}, chunks from {smallest} bytes");
        let channels = channels_of(&index, 0);
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never).unwrap();
        assert!(read.planes.iter().all(|p| p.len() == w * h && p.iter().all(|v| *v == 0.0)), "{compression:?}");
    }
}

#[test]
fn the_ratio_bound_allows_a_mebibyte_more_and_four_times_as_much_for_dwa() {
    use exr::prelude::f16;
    let build = |bytes: &[u8], max_ratio| Index::build(&mut Cursor::new(bytes), &Limits { max_ratio, ..Limits::default() }, &never).unwrap();
    // A black ZIP16 block of 512 × 16 f32 RGBA is 128 KiB over a few hundred bytes: refused at
    // one byte per byte but for the 1 MiB every chunk is allowed besides.
    let zip = file(vec![layer(None, (0, 0), (512, 16), &["R", "G", "B", "A"], false, Compression::ZIP16)], (512, 16));
    let list: Vec<AnyChannel<FlatSamples>> =
        ["R", "G", "B", "A"].iter().map(|n| AnyChannel::new(*n, FlatSamples::F32(vec![0.0; 512 * 16]))).collect();
    let encoding = Encoding { compression: Compression::ZIP16, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    let black = file(vec![Layer::new((512, 16), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (512, 16));
    for bytes in [&zip, &black] {
        let index = build(bytes, 1);
        assert!(index.chunks(0).all(|c| c.len < 128 << 10));
        assert_eq!(index.check_blocks(), Ok(()));
    }
    // A black DWAB block of 4096 × 256 half RGBA (8 MiB): with a ratio that lets its bytes hold
    // between a quarter of the block and the block (less the 1 MiB), only the fourfold DWA
    // allowance admits it.
    let list: Vec<AnyChannel<FlatSamples>> =
        ["R", "G", "B", "A"].iter().map(|n| AnyChannel::new(*n, FlatSamples::F16(vec![f16::ZERO; 4096 * 256]))).collect();
    let encoding = Encoding { compression: Compression::DWAB(None), blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    let dwab = file(vec![Layer::new((4096, 256), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (4096, 256));
    let len = indexed(&dwab).chunks(0).next().unwrap().len;
    let ratio = (5u64 << 20) / len; // len × ratio ≈ 5 MiB: 8 MiB > that + 1 MiB, but ≤ 4 × that + 1 MiB
    let index = build(&dwab, ratio);
    assert_eq!(index.check_blocks(), Ok(()), "{len}-byte chunk, ratio {ratio}");
    assert!(matches!(build(&dwab, ratio / 8).check_blocks(), Err(Error::LimitExceeded(_))));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn memory_the_system_cannot_give_is_out_of_memory_not_a_limit() {
    // A display window of (2^30 - 2)^2 pixels: one f32 plane is 4 EiB, which no system maps, so
    // the reservation fails at once (nothing is touched).
    let mut bytes = file(vec![layer(None, (0, 0), (1, 1), &["R"], false, Compression::ZIP1)], (1, 1));
    patch_box(&mut bytes, "displayWindow", [0, 0, (1 << 30) - 3, (1 << 30) - 3]);
    let index = Index::build(&mut Cursor::new(&bytes[..]), &Limits::none(), &never).unwrap();
    let got = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request::new(&[(0, 0)], usize::MAX), &never);
    assert!(matches!(got, Err(Error::OutOfMemory(_))), "{got:?}");
}

#[test]
#[allow(clippy::reversed_empty_ranges)] // (reversed ranges are what is tested)
fn empty_reversed_and_extreme_row_ranges_read_without_overflow() {
    // (alignment to a level's grid near the ends of i64 overflowed before the rows were clamped)
    let bytes = file(vec![layer(None, (0, 0), (9, 7), &["R"], false, Compression::ZIP1)], (9, 7));
    let index = indexed(&bytes);
    let whole = oracle(&bytes, 0, 0.0)[0].clone();
    for level in 0..=16u32 {
        for rows in [i64::MAX..i64::MAX, i64::MIN..i64::MIN, i64::MAX - 1..i64::MAX, 5..2, i64::MAX..i64::MIN, -3..-1, 7..9] {
            let request = Request { rows: Some(rows.clone()), level, ..Request::new(&[(0, 0)], usize::MAX) };
            let mut bands = 0;
            read_bands(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut |_| bands += 1).unwrap();
            assert_eq!(bands, 0, "{rows:?} at level {level} holds no row");
            assert_eq!(index.estimate(&request).unwrap().chunks, 0);
        }
        // Everything, from one end of i64 to the other.
        let request = Request { rows: Some(i64::MIN..i64::MAX), level, ..Request::new(&[(0, 0)], usize::MAX) };
        let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
        let stride = 1usize << level;
        let expected: Vec<f32> =
            (0..7).step_by(stride).flat_map(|y| (0..9).step_by(stride).map(move |x| (y, x))).map(|(y, x)| whole[y * 9 + x]).collect();
        assert_eq!(read.planes[0], expected, "level {level}");
    }
}

#[test]
fn a_sink_can_stop_the_read_with_its_own_error() {
    // 600 chunks on 4 threads: the sink refuses its third band. Nothing reaches it after that, the
    // read returns (no hang) with the sink's error unchanged, and the counts say how far it got.
    let bytes = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let index = indexed(&bytes);
    let channels = channels_of(&index, 0);
    let request = Request { workers: Some(4), ..Request::new(&channels, usize::MAX) };
    let mut calls = 0usize;
    let mut stats = Stats::default();
    let got = read_bands_try(
        &mut Cursor::new(&bytes[..]),
        &index,
        &request,
        &never,
        &mut |_: Band| {
            calls += 1;
            if calls >= 3 { Err("the tile store is full") } else { Ok(()) }
        },
        &mut stats,
    );
    assert_eq!(got, Err(Stop::Sink("the tile store is full")));
    assert_eq!(calls, 3, "no band after the sink's error");
    assert!(stats.lines >= 1 && stats.lines < 600 && stats.bytes > 0, "{stats:?}");
    // A sink that never fails reads everything, as `read_bands` does.
    let mut bands = 0usize;
    let mut sink = |_: Band| -> std::result::Result<(), ()> {
        bands += 1;
        Ok(())
    };
    read_bands_try(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut sink, &mut stats).unwrap();
    assert_eq!((bands, stats.lines), (1200, 600));
    // The read's own errors come back as such.
    let mut damaged = bytes.clone();
    let at = damaged.len() - 200 * 8200;
    damaged[at..at + 4].copy_from_slice(&7i32.to_le_bytes());
    let index = indexed(&damaged);
    let got = read_bands_try(&mut Cursor::new(&damaged[..]), &index, &request, &never, &mut |_| Ok::<(), ()>(()), &mut stats);
    assert!(matches!(got, Err(Stop::Read(Error::Malformed(_)))), "{got:?}");
}

/// The planes of `bands` (grouped or not) for `slots` slots of `w × h`, de-interleaved.
fn planes_of(bands: &[Band], slots: usize, (w, h): (usize, usize)) -> Vec<Vec<f32>> {
    let mut planes = vec![vec![-1e30; w * h]; slots];
    for b in bands {
        assert_eq!(b.samples.len(), b.width * b.height * b.values_per_pixel);
        for r in 0..b.height {
            for c in 0..b.width {
                for k in 0..b.values_per_pixel {
                    planes[b.slot + k][(b.y + r) * w + b.x + c] = b.samples[(r * b.width + c) * b.values_per_pixel + k];
                }
            }
        }
    }
    planes
}

#[test]
fn grouped_slots_arrive_interleaved_as_one_band_per_block() {
    for tiles in [false, true] {
        let bytes = file(vec![layer(None, (0, 0), (13, 11), &["R", "G", "B", "A", "Z"], tiles, Compression::ZIP1)], (13, 11));
        let index = indexed(&bytes);
        // Stored A B G R Z: ask for R G B A as a group, Z alone, and G again in a group of one.
        let names: Vec<&str> = index.parts()[0].channels.iter().map(|c| c.name.as_str()).collect();
        let at = |n: &str| (0, names.iter().position(|m| *m == n).unwrap());
        let channels = [at("R"), at("G"), at("B"), at("A"), at("Z"), at("G")];
        for (rows, level) in [(None, 0u32), (Some(3..9), 0), (None, 1)] {
            let base = Request { rows: rows.clone(), level, ..Request::new(&channels, usize::MAX) };
            let mut single = Vec::new();
            read_bands(&mut Cursor::new(&bytes[..]), &index, &base, &never, &mut |b| single.push(b)).unwrap();
            let groups = [0..4, 5..6];
            let mut grouped = Vec::new();
            read_bands(&mut Cursor::new(&bytes[..]), &index, &Request { groups: &groups, ..base.clone() }, &never, &mut |b| grouped.push(b)).unwrap();
            assert!(single.iter().all(|b| b.values_per_pixel == 1));
            assert!(grouped.iter().all(|b| b.values_per_pixel == if b.slot == 0 { 4 } else { 1 }), "{tiles} {rows:?} {level}");
            assert_eq!(grouped.iter().filter(|b| b.slot == 0).count(), single.iter().filter(|b| b.slot == 0).count(), "one band per block");
            let size = level_size((13, 11), level);
            assert_eq!(planes_of(&grouped, 6, size), planes_of(&single, 6, size), "tiles {tiles} rows {rows:?} level {level}");
            // `read_planes` ignores groups.
            let with = read_planes(&mut Cursor::new(&bytes[..]), &index, &Request { groups: &groups, ..base.clone() }, &never).unwrap();
            assert_eq!(with, read_planes(&mut Cursor::new(&bytes[..]), &index, &base, &never).unwrap());
        }
    }
}

#[test]
fn groups_must_be_slots_of_one_part_each_in_one_group() {
    let bytes = file(
        vec![
            layer(Some("a"), (0, 0), (8, 8), &["R", "G"], false, Compression::ZIP1),
            layer(Some("b"), (0, 0), (8, 8), &["Z"], false, Compression::ZIP1),
        ],
        (8, 8),
    );
    let index = indexed(&bytes);
    let channels = [(0, 0), (0, 1), (1, 0)];
    let read = |groups: &[std::ops::Range<usize>]| {
        read_bands(&mut Cursor::new(&bytes[..]), &index, &Request { groups, ..Request::new(&channels, usize::MAX) }, &never, &mut |_| {})
    };
    assert!(matches!(read(&[1..3]), Err(Error::Unsupported(m)) if m.contains("spans parts")));
    for bad in [&[0..2, 1..2][..], &[2..4], &[1..1]] {
        assert!(matches!(read(bad), Err(Error::Malformed(_))), "{bad:?}");
    }
    assert!(read(&[0..2]).is_ok());
}

#[test]
fn the_estimate_covers_what_the_read_does() {
    // A flat colour in ZIP16 (decodes to far more than it reads), lines of a ramp, and tiles.
    let (w, h) = (1024usize, 600usize);
    let list = vec![AnyChannel::new("R", FlatSamples::F32(vec![0.25; w * h])), AnyChannel::new("G", FlatSamples::F32(vec![0.5; w * h]))];
    let encoding = Encoding { compression: Compression::ZIP16, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    let flat = file(vec![Layer::new((w, h), LayerAttributes::default(), encoding, AnyChannels::sort(list.into()))], (w, h));
    let ramp = file(vec![layer(None, (0, 0), (1024, 600), &["R", "G"], false, Compression::Uncompressed)], (1024, 600));
    let tiled = file(vec![layer(None, (0, 0), (300, 200), &["R", "G", "B"], true, Compression::PIZ)], (300, 200));
    for bytes in [&flat, &ramp, &tiled] {
        let index = indexed(bytes);
        let channels = channels_of(&index, 0);
        for (rows, level, workers) in [(None, 0u32, None), (Some(50..120), 0, Some(4)), (None, 1, Some(0))] {
            let request = Request { rows: rows.clone(), level, workers, ..Request::new(&channels, usize::MAX) };
            let estimate = index.estimate(&request).unwrap();
            let read = read_planes(&mut Cursor::new(&bytes[..]), &index, &request, &never).unwrap();
            let what = format!("{rows:?} level {level}: {estimate:?} {:?}", read.stats);
            assert_eq!(estimate.chunks, read.stats.chunks.iter().sum::<usize>(), "{what}");
            assert_eq!(estimate.compressed_bytes, read.stats.bytes, "{what}");
            assert!(estimate.working_bytes >= read.stats.in_flight, "{what}");
            let (ow, oh) = level_size((index.display().width as usize, index.display().height as usize), level);
            assert_eq!(estimate.planes_bytes, (ow * oh * 4 * channels.len()) as u64, "{what}");
            // Measured from what is decoded: the largest chunk's block and bands (decoded, and
            // as much again for scratch) fit in the working bytes.
            let largest = decoded_per_chunk(bytes, &index, &request).into_iter().max().unwrap_or(0);
            assert!(estimate.working_bytes >= 2 * largest, "{what}, largest chunk {largest}");
        }
    }
    // It refuses what the read refuses.
    let index = indexed(&ramp);
    assert!(matches!(index.estimate(&Request::new(&[(0, 0)], 1024)), Err(Error::LimitExceeded(_))));
    assert!(matches!(index.estimate(&Request::new(&[(0, 9)], usize::MAX)), Err(Error::Malformed(_))));
}

/// Per chunk of a read of part 0 of `bytes`: its decompressed block's bytes (all of the part's
/// channels over the chunk's pixels) plus the bytes of the bands it gave, as decoded.
fn decoded_per_chunk(bytes: &[u8], index: &Index, request: &Request) -> Vec<u64> {
    let pixel_bytes: u64 = index.parts()[0].channels.iter().map(|c| if c.sample == crate::decode::SampleType::F16 { 2 } else { 4 }).sum();
    let mut bands_of: std::collections::BTreeMap<usize, u64> = std::collections::BTreeMap::new();
    read_bands(&mut Cursor::new(bytes), index, request, &never, &mut |b: Band| {
        assert_eq!(b.samples.len(), b.width * b.height * b.values_per_pixel, "exactly sized");
        assert_eq!(b.samples.capacity(), b.samples.len(), "no slack");
        *bands_of.entry(b.chunk).or_default() += 4 * b.samples.len() as u64;
    })
    .unwrap();
    // The chunks a read fetches, in file order, as the bands number them.
    let mut chunks: Vec<ChunkInfo> = index.chunks(0).collect();
    chunks.sort_by_key(|c| c.offset);
    let estimate = index.estimate(request).unwrap();
    let needed: Vec<ChunkInfo> = chunks
        .into_iter()
        .filter(|c| {
            let rows = request.rows.clone().unwrap_or(0..i64::MAX);
            let stride = 1i64 << request.level;
            (c.y as i64..(c.y + c.height) as i64).any(|r| rows.contains(&r) && r % stride == 0)
        })
        .collect();
    assert_eq!(needed.len(), estimate.chunks);
    needed.iter().enumerate().map(|(i, c)| (c.width * c.height) as u64 * pixel_bytes + bands_of.get(&i).copied().unwrap_or(0)).collect()
}

#[test]
fn a_grouped_read_with_duplicates_keeps_one_chunk_in_flight_at_its_decoded_size() {
    // 1024 × 64 RGBA f32 in ZIP16: 16-row blocks of 256 KiB. R, G, B, A and R again as one group:
    // each block's band is 16 × 1024 × 5 values (320 KiB), so a chunk is 576 KiB decoded, over
    // a 64 KiB budget: one at a time.
    let bytes = file(vec![layer(None, (0, 0), (1024, 64), &["R", "G", "B", "A"], false, Compression::ZIP16)], (1024, 64));
    let index = indexed(&bytes);
    let names: Vec<&str> = index.parts()[0].channels.iter().map(|c| c.name.as_str()).collect();
    let at = |n: &str| (0, names.iter().position(|m| *m == n).unwrap());
    let channels = [at("R"), at("G"), at("B"), at("A"), at("R")];
    let groups = [0..5];
    let request = Request { groups: &groups, workers: Some(4), ..Request::new(&channels, usize::MAX) };
    let decoded = decoded_per_chunk(&bytes, &index, &request);
    assert_eq!(decoded, vec![(256 + 320) << 10; 4], "block and band of each chunk, as decoded");
    let budget = 64 << 10;
    let mut bands = Vec::new();
    let stats = super::seek::read_bands_within(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut |b| bands.push(b), budget).unwrap();
    assert_eq!(stats.in_flight, (256 + 320) << 10, "one chunk at a time, counted at its decoded size");
    let estimate = super::seek::estimate_within(&index, &request, budget).unwrap();
    assert!(estimate.working_bytes >= 2 * ((256 + 320) << 10), "{estimate:?}");
    // The interleaved values, duplicates included, are those of per-channel reads.
    let mut single = Vec::new();
    read_bands(&mut Cursor::new(&bytes[..]), &index, &Request::new(&channels, usize::MAX), &never, &mut |b| single.push(b)).unwrap();
    assert_eq!(planes_of(&bands, 5, (1024, 64)), planes_of(&single, 5, (1024, 64)));
}

#[test]
fn bands_say_which_chunk_they_came_from_for_progress() {
    // Tiles of 4 × 4 over 9 × 7 (six chunks), and one-row chunks read at level 2 over some rows.
    let tiled = file(vec![layer(None, (0, 0), (9, 7), &["R", "G"], true, Compression::ZIP1)], (9, 7));
    let lines = file(vec![layer(None, (0, 0), (8, 16), &["Y"], false, Compression::ZIP1)], (8, 16));
    for (bytes, request) in [
        (&tiled, Request::new(&[(0, 0), (0, 1)], usize::MAX)),
        (&lines, Request { rows: Some(3..14), level: 2, workers: Some(2), ..Request::new(&[(0, 0)], usize::MAX) }),
    ] {
        let index = indexed(bytes);
        let estimate = index.estimate(&request).unwrap();
        let mut seen = Vec::new();
        read_bands(&mut Cursor::new(&bytes[..]), &index, &request, &never, &mut |b| seen.push(b.chunk)).unwrap();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "in order: {seen:?}");
        seen.dedup();
        assert_eq!(seen, (0..estimate.chunks).collect::<Vec<_>>(), "every chunk, numbered as the estimate counts");
    }
}

#[test]
fn a_stop_s_source_is_the_read_error() {
    use std::error::Error as _;
    let read: Stop<String> = Stop::Read(Error::Cancelled);
    assert_eq!(read.source().map(|e| e.to_string()), Some(Error::Cancelled.to_string()));
    assert!(Stop::Sink("full".to_string()).source().is_none());
}
