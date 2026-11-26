use std::cell::Cell;
use std::io::{self, Cursor, Seek, SeekFrom, Write};

use crate::decode::{self, Compression, Error, Limits, MAX_HEADER_ATTRIBUTES, SampleType, Samples, Window, f16};
use crate::seek::{Index, Request, read_planes};
use crate::write::{Channel, Metadata, Part, RowSamples, TilePart, TileWriter, write_scanlines, write_scanlines_with_metadata, write_tiles};

const WINDOW: Window = Window { x: -3, y: -5, width: 64, height: 35 };
const HALF: Channel<'_> = Channel { name: "R", sample: SampleType::F16 };
const FLOAT: Channel<'_> = Channel { name: "Z", sample: SampleType::F32 };
const UINT: Channel<'_> = Channel { name: "ID", sample: SampleType::U32 };

fn half_value(row: usize, channel: usize, x: usize) -> f16 {
    f16::from_f32((row * 8 + channel * 32 + x / 8) as f32 / 16.0)
}

fn depth_value(row: usize, x: usize) -> f32 {
    f32::from_bits(0x3f80_0001 + (row * 8 + x / 8) as u32)
}

fn id_value(row: usize, x: usize) -> u32 {
    u32::MAX - (row * 8 + x / 8) as u32
}

fn sample_row(_: usize, row: usize, channel: usize, mut samples: RowSamples<'_>) -> decode::Result<usize> {
    match &mut samples {
        RowSamples::F16(values) => values.iter_mut().enumerate().for_each(|(x, value)| *value = half_value(row, channel, x)),
        RowSamples::F32(values) => values.iter_mut().enumerate().for_each(|(x, value)| *value = depth_value(row, x)),
        RowSamples::U32(values) => values.iter_mut().enumerate().for_each(|(x, value)| *value = id_value(row, x)),
    }
    Ok(samples.len())
}

fn whole_plane<T>(window: Window, sample: impl Fn(usize, usize) -> T) -> Vec<T> {
    (0..window.height as usize).flat_map(|row| (0..window.width as usize).map(move |x| (row, x))).map(|(row, x)| sample(row, x)).collect()
}

fn fill_tile(part: TilePart<'_>, window: Window, tile: &mut TileWriter<'_>) -> decode::Result<()> {
    let x0 = (window.x - part.part.data_window.x) as usize;
    let y0 = (window.y - part.part.data_window.y) as usize;
    let width = window.width as usize;
    // Supply in reverse descriptor order to exercise channel placement independent of callback order.
    for channel in (0..part.part.channels.len()).rev() {
        tile.fill_channel(channel, |mut samples| {
            let at = |i: usize| (y0 + i / width, x0 + i % width);
            match &mut samples {
                RowSamples::F16(values) => values.iter_mut().enumerate().for_each(|(i, value)| {
                    let (y, x) = at(i);
                    *value = half_value(y, channel, x);
                }),
                RowSamples::F32(values) => values.iter_mut().enumerate().for_each(|(i, value)| {
                    let (y, x) = at(i);
                    *value = depth_value(y, x);
                }),
                RowSamples::U32(values) => values.iter_mut().enumerate().for_each(|(i, value)| {
                    let (y, x) = at(i);
                    *value = id_value(y, x);
                }),
            }
            Ok(samples.len())
        })?;
    }
    Ok(())
}

#[test]
fn tiled_lossless_round_trips_all_sample_types_and_clips_edge_tiles() {
    for (compression, name) in [
        (Compression::None, "Uncompressed"),
        (Compression::Rle, "RLE"),
        (Compression::Zip1, "ZIP1"),
        (Compression::Zip16, "ZIP16"),
        (Compression::Piz, "PIZ"),
    ] {
        let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT, HALF, UINT], compression };
        let tiled = TilePart { part, tile_width: 30, tile_height: 18 };
        let mut out = Cursor::new(Vec::new());
        let mut windows = Vec::new();
        write_tiles(&mut out, WINDOW, &[tiled], &Metadata::default(), |part, window, tile| {
            assert_eq!(part, 0);
            windows.push(window);
            fill_tile(tiled, window, tile)
        })
        .unwrap();
        assert_eq!(windows.len(), 6);
        assert_eq!(windows[0], Window { width: 30, height: 18, ..WINDOW });
        assert_eq!(windows[5], Window { x: WINDOW.x + 60, y: WINDOW.y + 18, width: 4, height: 17 });

        let decoded = decode::decode_part(out.get_ref(), 0, &|_| true, &Limits::default()).unwrap();
        assert!(decoded.info.tiled);
        assert_eq!(decoded.info.compression, name);
        assert_eq!(decoded.info.data_window, WINDOW);
        assert_eq!(decoded.channel("Z").unwrap().samples, Samples::F32(whole_plane(WINDOW, depth_value)));
        assert_eq!(decoded.channel("R").unwrap().samples, Samples::F16(whole_plane(WINDOW, |y, x| half_value(y, 1, x))));
        assert_eq!(decoded.channel("ID").unwrap().samples, Samples::U32(whole_plane(WINDOW, id_value)));
    }
}

#[test]
fn tiled_multipart_keeps_distinct_tile_sizes_and_metadata() {
    let depth = Part { name: Some("Depth"), data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let beauty = Part { name: Some("Beauty"), data_window: WINDOW, channels: &[HALF], compression: Compression::Piz };
    let parts = [TilePart { part: depth, tile_width: 16, tile_height: 14 }, TilePart { part: beauty, tile_width: 31, tile_height: 20 }];
    let chromaticities = [[0.64, 0.33], [0.3, 0.6], [0.15, 0.06], [0.3127, 0.329]];
    let text = [&[("craftLayerStack", "{\"version\":1}"), ("comments", "Preview")][..], &[("passLabel", "Beauty")][..]];
    let metadata = Metadata { chromaticities: Some(chromaticities), text_attributes: &text };
    let mut out = Cursor::new(Vec::new());
    write_tiles(&mut out, WINDOW, &parts, &metadata, |part, window, tile| fill_tile(parts[part], window, tile)).unwrap();
    let mut source = Cursor::new(out.into_inner());
    let index = Index::build(&mut source, &Limits::default(), &|| false).unwrap();
    let info = index.parts();
    assert_eq!(info[0].name.as_deref(), Some("Depth"));
    assert_eq!(info[1].name.as_deref(), Some("Beauty"));
    assert!(info.iter().all(|part| part.tiled));
    assert_eq!(info[0].text("craftLayerStack"), Some("{\"version\":1}"));
    assert_eq!(info[1].text("craftLayerStack"), None);
    assert_eq!(info[1].text("passLabel"), Some("Beauty"));
    let headers = exr::meta::MetaData::read_from_buffered(Cursor::new(source.get_ref()), true).unwrap().headers;
    assert_eq!(headers[0].own_attributes.comments.as_ref().unwrap().to_string(), "Preview");
    for header in &headers {
        let c = header.shared_attributes.chromaticities.unwrap();
        assert_eq!([[c.red.0, c.red.1], [c.green.0, c.green.1], [c.blue.0, c.blue.1], [c.white.0, c.white.1]], chromaticities);
    }
    let requested_row = 31;
    let request = Request { rows: Some(requested_row..requested_row + 1), ..Request::new(&[(0, 0), (1, 0)], usize::MAX) };
    let read = read_planes(&mut source, &index, &request, &|| false).unwrap();
    assert_eq!(read.stats.chunks, [4, 3]);
    let requested_row = requested_row as usize;
    assert_eq!(read.planes[0], whole_plane(WINDOW, |y, x| if y == requested_row { depth_value(y, x) } else { 0.0 }));
    assert_eq!(read.planes[1], whole_plane(WINDOW, |y, x| if y == requested_row { half_value(y, 0, x).to_f32() } else { 0.0 }));
}

#[test]
fn scanline_metadata_round_trips_a_custom_string() {
    let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let text = [&[("cryptomatte/abc1234/name", "CryptoObject")][..]];
    let metadata = Metadata { text_attributes: &text, ..Metadata::default() };
    let mut out = Cursor::new(Vec::new());
    write_scanlines_with_metadata(&mut out, WINDOW, &[part], &metadata, sample_row).unwrap();
    let parts = decode::parts(out.get_ref(), &Limits::default()).unwrap();
    assert_eq!(parts[0].text("cryptomatte/abc1234/name"), Some("CryptoObject"));
}

#[test]
fn channel_names_set_the_long_name_flag_at_the_openexr_byte_boundary() {
    let window = Window { width: 1, height: 1, ..WINDOW };
    for len in [31, 32, 255] {
        let name = "Z".repeat(len);
        let part = Part { name: None, data_window: window, channels: &[Channel { name: &name, ..FLOAT }], compression: Compression::None };
        let mut out = Cursor::new(Vec::new());
        write_scanlines(&mut out, window, &[part], sample_row).unwrap();
        assert_eq!(u32::from_le_bytes(out.get_ref()[4..8].try_into().unwrap()), 2 | if len > 31 { 0x400 } else { 0 });
        assert_eq!(decode::parts(out.get_ref(), &Limits::default()).unwrap()[0].channels[0].name, name);

        let tiled = TilePart { part, tile_width: 1, tile_height: 1 };
        let mut out = Cursor::new(Vec::new());
        write_tiles(&mut out, window, &[tiled], &Metadata::default(), |_, window, tile| fill_tile(tiled, window, tile)).unwrap();
        assert_eq!(u32::from_le_bytes(out.get_ref()[4..8].try_into().unwrap()), 0x202 | if len > 31 { 0x400 } else { 0 });
        let decoded = decode::decode_part(out.get_ref(), 0, &|_| true, &Limits::default()).unwrap();
        assert_eq!(decoded.channel(&name).unwrap().samples, Samples::F32(vec![depth_value(0, 0)]));
    }
}

#[test]
fn long_part_names_set_the_flag_in_multipart_files() {
    let window = Window { width: 1, height: 1, ..WINDOW };
    let part = Part { name: Some("Beauty"), data_window: window, channels: &[FLOAT], compression: Compression::None };
    for len in [31, 32, 255] {
        let name = "P".repeat(len);
        let mut out = Cursor::new(Vec::new());
        write_scanlines(&mut out, window, &[part, Part { name: Some(&name), ..part }], sample_row).unwrap();
        assert_eq!(u32::from_le_bytes(out.get_ref()[4..8].try_into().unwrap()), 0x1002 | if len > 31 { 0x400 } else { 0 });
        assert_eq!(decode::parts(out.get_ref(), &Limits::default()).unwrap()[1].name.as_deref(), Some(name.as_str()));
    }
}

#[test]
fn invalid_tile_dimensions_and_metadata_fail_before_output() {
    let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let tiled = TilePart { part, tile_width: 16, tile_height: 16 };
    let no_metadata = Metadata::default();
    let duplicate = [&[("pass", "Z"), ("pass", "depth")][..]];
    let reserved = [&[("tiles", "unexpected")][..]];
    let invalid_chromaticities = Metadata { chromaticities: Some([[f32::NAN; 2]; 4]), ..no_metadata };
    for (part, metadata) in [
        (TilePart { tile_width: 0, ..tiled }, no_metadata),
        (TilePart { tile_width: 16_384, tile_height: 16_384, ..tiled }, no_metadata),
        (tiled, invalid_chromaticities),
        (tiled, Metadata { text_attributes: &duplicate, ..no_metadata }),
        (tiled, Metadata { text_attributes: &reserved, ..no_metadata }),
    ] {
        let mut out = Cursor::new(Vec::new());
        let result = write_tiles(&mut out, WINDOW, &[part], &metadata, |_, _, _| panic!("invalid descriptors reached callback"));
        assert!(matches!(result, Err(Error::Encode(_))), "{result:?}");
        assert!(out.get_ref().is_empty());
    }
}

// Chunk count, channels, compression, line order, display and data windows, pixel aspect ratio,
// screen window centre and width, block type and part name
const MULTIPART_SCANLINE_ATTRIBUTES: usize = 11;

fn numbered_attributes(count: usize) -> Vec<(String, String)> {
    (0..count).map(|n| (format!("pass{n}"), format!("value{n}"))).collect()
}

fn borrowed(attributes: &[(String, String)]) -> Vec<(&str, &str)> {
    attributes.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect()
}

#[test]
fn multipart_header_at_the_attribute_limit_round_trips() {
    let window = Window { width: 2, height: 2, ..WINDOW };
    let beauty = Part { name: Some("Beauty"), data_window: window, channels: &[FLOAT], compression: Compression::Zip16 };
    let depth = Part { name: Some("Depth"), ..beauty };
    let user_attributes = numbered_attributes(MAX_HEADER_ATTRIBUTES - MULTIPART_SCANLINE_ATTRIBUTES);
    let borrowed_attributes = borrowed(&user_attributes);
    let text = [&borrowed_attributes[..], &[("passLabel", "Depth")][..]];
    let metadata = Metadata { text_attributes: &text, ..Metadata::default() };
    let mut out = Cursor::new(Vec::new());

    write_scanlines_with_metadata(&mut out, window, &[beauty, depth], &metadata, sample_row).unwrap();

    let index = Index::build(&mut out, &Limits::default(), &|| false).unwrap();
    let last = user_attributes.len() - 1;
    assert_eq!(index.parts()[0].text("pass0"), Some("value0"));
    assert_eq!(index.parts()[0].text(&format!("pass{last}")), Some(format!("value{last}").as_str()));
    assert_eq!(index.parts()[1].text("passLabel"), Some("Depth"));
}

#[test]
fn multipart_header_over_the_attribute_limit_is_refused_before_output() {
    let window = Window { width: 2, height: 2, ..WINDOW };
    let beauty = Part { name: Some("Beauty"), data_window: window, channels: &[FLOAT], compression: Compression::Zip16 };
    let depth = Part { name: Some("Depth"), ..beauty };
    let user_attributes = numbered_attributes(MAX_HEADER_ATTRIBUTES - MULTIPART_SCANLINE_ATTRIBUTES + 1);
    let borrowed_attributes = borrowed(&user_attributes);
    let text = [&[][..], &borrowed_attributes[..]];
    let metadata = Metadata { text_attributes: &text, ..Metadata::default() };
    let mut out = Cursor::new(Vec::new());

    let result = write_scanlines_with_metadata(&mut out, window, &[beauty, depth], &metadata, |_, _, _, _| panic!("reached callback"));

    assert!(matches!(&result, Err(Error::Encode(message)) if message.contains("part 1") && message.contains("4096")), "{result:?}");
    assert!(out.get_ref().is_empty());
}

#[test]
fn tiled_header_with_chromaticities_counts_them_against_the_attribute_limit() {
    let window = Window { width: 2, height: 2, ..WINDOW };
    let part = Part { name: Some("Beauty"), data_window: window, channels: &[FLOAT], compression: Compression::Zip16 };
    let tiled = TilePart { part, tile_width: 2, tile_height: 2 };
    let chromaticities = Some([[0.64, 0.33], [0.3, 0.6], [0.15, 0.06], [0.3127, 0.329]]);
    // The scan-line set plus the tile description and the shared chromaticities
    let user_count = MAX_HEADER_ATTRIBUTES - MULTIPART_SCANLINE_ATTRIBUTES - 2;
    let user_attributes = numbered_attributes(user_count);
    let borrowed_attributes = borrowed(&user_attributes);
    let text = [&borrowed_attributes[..]];
    let metadata = Metadata { chromaticities, text_attributes: &text };
    let mut out = Cursor::new(Vec::new());

    write_tiles(&mut out, window, &[tiled], &metadata, |_, window, tile| fill_tile(tiled, window, tile)).unwrap();

    Index::build(&mut out, &Limits::default(), &|| false).unwrap();
    let over_attributes = numbered_attributes(user_count + 1);
    let borrowed_over = borrowed(&over_attributes);
    let over_text = [&borrowed_over[..]];
    let over = Metadata { chromaticities, text_attributes: &over_text };
    let mut refused = Cursor::new(Vec::new());
    let result = write_tiles(&mut refused, window, &[tiled], &over, |_, _, _| panic!("reached callback"));
    assert!(matches!(result, Err(Error::Encode(_))), "{result:?}");
    assert!(refused.get_ref().is_empty());
}

#[test]
fn incomplete_tiles_are_rejected() {
    let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let tiled = TilePart { part, tile_width: 4, tile_height: 5 };
    let result = write_tiles(&mut Cursor::new(Vec::new()), WINDOW, &[tiled], &Metadata::default(), |_, _, _| Ok(()));
    assert!(matches!(result, Err(Error::Encode(message)) if message.contains("missing channel 0")));

    let short = write_tiles(&mut Cursor::new(Vec::new()), WINDOW, &[tiled], &Metadata::default(), |_, _, tile| {
        tile.fill_channel(0, |plane| Ok(plane.len() - 1))
    });
    assert!(matches!(short, Err(Error::Encode(message)) if message.contains("expected 20 samples, got 19")));

    let duplicate = write_tiles(&mut Cursor::new(Vec::new()), WINDOW, &[tiled], &Metadata::default(), |_, _, tile| {
        tile.fill_channel(0, |plane| Ok(plane.len()))?;
        tile.fill_channel(0, |plane| Ok(plane.len()))
    });
    assert!(matches!(duplicate, Err(Error::Encode(message)) if message.contains("duplicate tile channel 0")));
}

#[test]
fn single_part_lossless_round_trips_all_sample_types() {
    for (compression, name) in [
        (Compression::None, "Uncompressed"),
        (Compression::Rle, "RLE"),
        (Compression::Zip1, "ZIP1"),
        (Compression::Zip16, "ZIP16"),
        (Compression::Piz, "PIZ"),
    ] {
        let mut out = Cursor::new(Vec::new());
        let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT, HALF, UINT], compression };
        write_scanlines(&mut out, WINDOW, &[part], sample_row).unwrap();
        let limits = Limits::default();
        let bytes = out.get_ref();
        assert_eq!(decode::parts(bytes, &limits).unwrap().len(), 1);
        let decoded = decode::decode_part(bytes, 0, &|_| true, &limits).unwrap();
        assert_eq!(decoded.info.name, None);
        assert_eq!(decoded.info.compression, name);
        assert_eq!(decoded.info.data_window, WINDOW);
        assert_eq!(decoded.channel("Z").unwrap().samples, Samples::F32(whole_plane(WINDOW, depth_value)));
        assert_eq!(decoded.channel("R").unwrap().samples, Samples::F16(whole_plane(WINDOW, |row, x| half_value(row, 1, x))));
        assert_eq!(decoded.channel("ID").unwrap().samples, Samples::U32(whole_plane(WINDOW, id_value)));
    }
}

#[test]
fn lossy_encodings_preserve_integer_ids() {
    for compression in [Compression::Pxr24, Compression::B44, Compression::B44a, Compression::Dwaa, Compression::Dwab] {
        let mut out = Cursor::new(Vec::new());
        let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT, HALF, UINT], compression };
        write_scanlines(&mut out, WINDOW, &[part], sample_row).unwrap();
        let decoded = decode::decode_part(out.get_ref(), 0, &|_| true, &Limits::default()).unwrap();
        assert_eq!(decoded.channel("ID").unwrap().samples, Samples::U32(whole_plane(WINDOW, id_value)), "{compression:?}");
    }
}

#[test]
fn named_parts_round_trip_and_support_selective_reads() {
    let beauty = [HALF, Channel { name: "G", ..HALF }, Channel { name: "B", ..HALF }, Channel { name: "A", ..HALF }];
    let depth_window = Window { x: WINDOW.x + 1, y: WINDOW.y + 2, width: WINDOW.width - 1, height: WINDOW.height - 2 };
    let parts = [
        Part { name: Some("Depth"), data_window: depth_window, channels: &[FLOAT], compression: Compression::Zip1 },
        Part { name: Some("Beauty"), data_window: WINDOW, channels: &beauty, compression: Compression::Zip16 },
    ];
    let mut out = Cursor::new(Vec::new());
    write_scanlines(&mut out, WINDOW, &parts, sample_row).unwrap();
    let limits = Limits::default();
    let mut source = Cursor::new(out.into_inner());
    let index = Index::build(&mut source, &limits, &|| false).unwrap();
    let headers = index.parts();
    assert_eq!(headers.iter().map(|part| part.name.as_deref().unwrap()).collect::<Vec<_>>(), ["Depth", "Beauty"]);
    assert_eq!(decode::pick_part(headers), Some(1));
    assert_eq!((&*headers[0].compression, &*headers[1].compression), ("ZIP1", "ZIP16"));
    assert_eq!(headers[0].data_window, depth_window);
    assert!(headers.iter().all(|part| part.display_window == WINDOW));
    let beauty = &headers[1].channels;
    assert_eq!(beauty.iter().map(|channel| channel.name.as_str()).collect::<Vec<_>>(), ["A", "B", "G", "R"]);
    assert!(beauty.iter().all(|channel| channel.sample == SampleType::F16));
    assert_eq!(headers[0].channels[0].sample, SampleType::F32);
    let depth = decode::decode_part(source.get_ref(), 0, &|_| true, &limits).unwrap();
    assert_eq!(depth.channel("Z").unwrap().samples, Samples::F32(whole_plane(depth_window, depth_value)));

    let row = 17;
    let request = Request { rows: Some(row..row + 1), fill: &[-1.0, -2.0], ..Request::new(&[(1, 3), (0, 0)], usize::MAX) };
    let read = read_planes(&mut source, &index, &request, &|| false).unwrap();
    assert_eq!(read.stats.chunks, [1, 1]);
    let row = row as usize;
    for (plane, expected) in [
        (&read.planes[0], whole_plane(WINDOW, |y, x| if y == row { half_value(y, 0, x).to_f32() } else { -1.0 })),
        (&read.planes[1], whole_plane(WINDOW, |y, x| if y == row && x > 0 { depth_value(y - 2, x - 1) } else { -2.0 })),
    ] {
        assert_eq!(plane, &expected);
    }
}

#[test]
fn bad_descriptors_fail_before_writing_or_requesting_rows() {
    let part = Part { name: Some("Depth"), data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let duplicate = [FLOAT, FLOAT];
    let bad_channel = [Channel { name: "Z\0hidden", ..FLOAT }];
    let long_name = "Z".repeat(256);
    let long_channel = [Channel { name: &long_name, ..FLOAT }];
    let empty_name = [Channel { name: "", ..FLOAT }];
    for parts in [
        vec![],
        vec![part, part],
        vec![part, Part { name: None, ..part }],
        vec![Part { name: Some("🎞"), ..part }],
        vec![Part { channels: &duplicate, ..part }],
        vec![Part { channels: &bad_channel, ..part }],
        vec![Part { channels: &long_channel, ..part }],
        vec![Part { channels: &empty_name, ..part }],
        vec![Part { channels: &[], ..part }],
        vec![Part { data_window: Window { width: 0, ..WINDOW }, ..part }],
        vec![Part { data_window: Window { x: i32::MAX, ..WINDOW }, ..part }],
        vec![Part { data_window: Window { width: 1 << 27, ..WINDOW }, ..part }],
        vec![Part { data_window: Window { height: 1 << 21, ..WINDOW }, compression: Compression::Zip1, ..part }],
    ] {
        let mut out = Cursor::new(Vec::new());
        let result = write_scanlines(&mut out, WINDOW, &parts, |_, _, _, _| panic!("invalid descriptors reached callback"));
        assert!(matches!(result, Err(Error::Encode(_))), "{parts:?}: {result:?}");
        assert!(out.get_ref().is_empty());
    }
}

#[test]
fn short_rows_and_callback_errors_stop_writing() {
    let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let mut calls = 0;
    let short = write_scanlines(&mut Cursor::new(Vec::new()), WINDOW, &[part], |_, _, _, row| {
        calls += 1;
        Ok(row.len() - 1)
    });
    assert!(matches!(short, Err(Error::Encode(message)) if message.contains("expected 64 samples, got 63")));
    assert_eq!(calls, 1);

    let stopped = Error::Encode("source became unavailable".into());
    let result = write_scanlines(&mut Cursor::new(Vec::new()), WINDOW, &[part], |_, _, _, _| Err(stopped.clone()));
    assert_eq!(result, Err(stopped));
}

struct LimitedOutput<'a> {
    cursor: Cursor<Vec<u8>>,
    limit: &'a Cell<u64>,
}

impl Write for LimitedOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cursor.position() + bytes.len() as u64 > self.limit.get() {
            return Err(io::Error::other("synthetic full disk"));
        }
        self.cursor.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for LimitedOutput<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}

#[test]
fn chunks_are_written_before_requesting_more_rows() {
    let limit = Cell::new(u64::MAX);
    let mut out = LimitedOutput { cursor: Cursor::new(Vec::new()), limit: &limit };
    let part = Part { name: None, data_window: WINDOW, channels: &[FLOAT], compression: Compression::Zip16 };
    let mut rows = 0;
    let result = write_scanlines(&mut out, WINDOW, &[part], |_, _, _, row| {
        rows += 1;
        // Let the header and offset placeholders through, then fail the first block write.
        limit.set(0);
        Ok(row.len())
    });
    assert!(matches!(result, Err(Error::Encode(message)) if message.contains("synthetic full disk")));
    assert!(rows > 0 && rows < WINDOW.height, "received {rows} rows before first block write");
}
