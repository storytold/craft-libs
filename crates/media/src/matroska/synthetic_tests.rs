//! Hand-built files exercising features FFmpeg does not write: unknown-size Segment/Clusters,
//! header stripping, BlockGroups, Colour + mastering metadata, Chapters, Tags, Attachments, CRC-32.

use crate::matroska::codec::Codec;
use crate::matroska::ebml::{self, write_id, write_unknown_size};
use crate::matroska::ids::*;
use crate::matroska::mux::{el, el_float, el_str, el_uint};
use crate::matroska::track::ContentEncoding;
use crate::matroska::{ByteSource, Demuxer, MkvFile, OpenOptions, Packet, open_with};

mod hostile;

fn block(track: u8, rel: i16, flags: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0x80 | track];
    b.extend_from_slice(&rel.to_be_bytes());
    b.push(flags);
    b.extend_from_slice(data);
    b
}

fn synthetic() -> Vec<u8> {
    let mut f = Vec::new();
    let mut h = Vec::new();
    el_str(&mut h, DOC_TYPE, "matroska");
    el_uint(&mut h, DOC_TYPE_VERSION, 4);
    el(&mut f, EBML, &h);
    write_id(&mut f, SEGMENT);
    write_unknown_size(&mut f, 8);
    // Info with a correct CRC-32 and a Void
    let mut info = Vec::new();
    el_uint(&mut info, TIMESTAMP_SCALE, 1_000_000);
    el_str(&mut info, TITLE, "Synthetic");
    el(&mut info, VOID, &[0; 3]);
    let mut crc_info = Vec::new();
    el(&mut crc_info, CRC32, &ebml::crc32(&info).to_le_bytes());
    crc_info.extend_from_slice(&info);
    el(&mut f, INFO, &crc_info);
    // Tracks
    let mut v = Vec::new();
    el_uint(&mut v, TRACK_NUMBER, 1);
    el_uint(&mut v, TRACK_UID, 11);
    el_uint(&mut v, TRACK_TYPE, 1);
    el_str(&mut v, CODEC_ID, "V_MPEG4/ISO/AVC");
    el(&mut v, CODEC_PRIVATE, &[1, 0x64, 0, 0x1F]);
    el_uint(&mut v, DEFAULT_DURATION, 40_000_000);
    let mut mm = Vec::new();
    for (id, val) in
        [(PRIMARY_R_X, 0.708), (PRIMARY_R_Y, 0.292), (PRIMARY_G_X, 0.17), (PRIMARY_G_Y, 0.797), (PRIMARY_B_X, 0.131), (PRIMARY_B_Y, 0.046)]
    {
        el_float(&mut mm, id, val);
    }
    el_float(&mut mm, WHITE_POINT_X, 0.3127);
    el_float(&mut mm, WHITE_POINT_Y, 0.329);
    el_float(&mut mm, LUMINANCE_MAX, 1000.0);
    el_float(&mut mm, LUMINANCE_MIN, 0.0001);
    let mut col = Vec::new();
    el_uint(&mut col, MATRIX_COEFFICIENTS, 9);
    el_uint(&mut col, RANGE, 2);
    el_uint(&mut col, TRANSFER_CHARACTERISTICS, 16);
    el_uint(&mut col, PRIMARIES, 9);
    el_uint(&mut col, MAX_CLL, 1000);
    el_uint(&mut col, MAX_FALL, 400);
    el(&mut col, MASTERING_METADATA, &mm);
    let mut vid = Vec::new();
    el_uint(&mut vid, PIXEL_WIDTH, 1440);
    el_uint(&mut vid, PIXEL_HEIGHT, 1080);
    el_uint(&mut vid, DISPLAY_WIDTH, 1920);
    el_uint(&mut vid, DISPLAY_HEIGHT, 1080);
    el(&mut vid, COLOUR, &col);
    // rectangular, a quarter turn clockwise (portrait phone video)
    let mut proj = Vec::new();
    el_uint(&mut proj, PROJECTION_TYPE, 0);
    el_float(&mut proj, PROJECTION_POSE_ROLL, -90.0);
    el(&mut vid, PROJECTION, &proj);
    el(&mut v, VIDEO, &vid);
    // header stripping: every frame starts with 00 00 00 01
    let mut comp = Vec::new();
    el_uint(&mut comp, CONTENT_COMP_ALGO, 3);
    el(&mut comp, CONTENT_COMP_SETTINGS, &[0, 0, 0, 1]);
    let mut enc = Vec::new();
    el(&mut enc, CONTENT_COMPRESSION, &comp);
    let mut encs = Vec::new();
    el(&mut encs, CONTENT_ENCODING, &enc);
    el(&mut v, CONTENT_ENCODINGS, &encs);
    let mut a = Vec::new();
    el_uint(&mut a, TRACK_NUMBER, 2);
    el_uint(&mut a, TRACK_TYPE, 2);
    el_str(&mut a, CODEC_ID, "A_AAC/MPEG4/LC/SBR");
    el_str(&mut a, LANGUAGE, "ger");
    let mut au = Vec::new();
    el_float(&mut au, SAMPLING_FREQUENCY, 24000.0);
    el_float(&mut au, OUTPUT_SAMPLING_FREQUENCY, 48000.0);
    el_uint(&mut au, CHANNELS, 2);
    el(&mut a, AUDIO, &au);
    let mut tr = Vec::new();
    el(&mut tr, TRACK_ENTRY, &v);
    el(&mut tr, TRACK_ENTRY, &a);
    el(&mut f, TRACKS, &tr);
    // Chapters
    let mut disp = Vec::new();
    el_str(&mut disp, CHAP_STRING, "Intro");
    let mut atom = Vec::new();
    el_uint(&mut atom, CHAPTER_UID, 5);
    el_uint(&mut atom, CHAPTER_TIME_START, 0);
    el_uint(&mut atom, CHAPTER_TIME_END, 40_000_000);
    el(&mut atom, CHAPTER_DISPLAY, &disp);
    let mut ed = Vec::new();
    el(&mut ed, CHAPTER_ATOM, &atom);
    let mut ch = Vec::new();
    el(&mut ch, EDITION_ENTRY, &ed);
    el(&mut f, CHAPTERS, &ch);
    // Attachments
    let mut af = Vec::new();
    el_str(&mut af, FILE_NAME, "font.ttf");
    el_str(&mut af, FILE_MEDIA_TYPE, "font/ttf");
    el_uint(&mut af, FILE_UID, 77);
    el(&mut af, FILE_DATA, &[9; 100]);
    let mut att = Vec::new();
    el(&mut att, ATTACHED_FILE, &af);
    el(&mut f, ATTACHMENTS, &att);
    // Cluster 1 (unknown size): keyframe SimpleBlock, BlockGroup with ReferenceBlock + duration
    write_id(&mut f, CLUSTER);
    write_unknown_size(&mut f, 1);
    el_uint(&mut f, TIMESTAMP, 1000);
    el(&mut f, SIMPLE_BLOCK, &block(1, 0, 0x80, &[0x65, 1, 2]));
    el(&mut f, SIMPLE_BLOCK, &block(2, 5, 0x80, &[0xAA; 10]));
    let mut g = Vec::new();
    el(&mut g, BLOCK, &block(1, 40, 0, &[0x41, 3]));
    el(&mut g, REFERENCE_BLOCK, &[0xD8]);
    el_uint(&mut g, BLOCK_DURATION, 80);
    el(&mut f, BLOCK_GROUP, &g);
    el(&mut f, SIMPLE_BLOCK, &block(9, 0, 0x80, &[1])); // unknown track: ignored
    // Cluster 2 (unknown size), ended by Tags
    write_id(&mut f, CLUSTER);
    write_unknown_size(&mut f, 8);
    el_uint(&mut f, TIMESTAMP, 1120);
    el(&mut f, SIMPLE_BLOCK, &block(1, 0, 0x80, &[0x65, 4]));
    let mut st = Vec::new();
    el_str(&mut st, TAG_NAME, "ARTIST");
    el_str(&mut st, TAG_STRING, "Nobody");
    let mut tg = Vec::new();
    let mut tgt = Vec::new();
    el_uint(&mut tgt, TAG_TRACK_UID, 11);
    el(&mut tg, TARGETS, &tgt);
    el(&mut tg, SIMPLE_TAG, &st);
    let mut tags = Vec::new();
    el(&mut tags, TAG, &tg);
    el(&mut f, TAGS, &tags);
    f
}

fn assert_synthetic_metadata(f: &MkvFile, bytes: &[u8]) {
    assert!(f.live);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    assert_eq!((f.crc_checked, f.crc_errors.len()), (1, 0));
    assert_eq!(f.info.title.as_deref(), Some("Synthetic"));
    let video = &f.tracks[0];
    assert!(matches!(&video.codec, Codec::Avc { avcc } if avcc[1] == 0x64));
    let info = video.video.as_ref().unwrap();
    assert_eq!(info.pixel_aspect(), (4, 3));
    assert_eq!(info.projection.as_ref().map(|projection| projection.roll), Some(-90.0));
    assert_eq!(info.display_rotation(), Some(1));
    let color = info.color.as_ref().unwrap();
    assert!(color.full_range());
    assert_eq!(
        (color.matrix_coefficients, color.transfer_characteristics, color.primaries, color.max_cll, color.max_fall),
        (Some(9), Some(16), Some(9), Some(1000), Some(400))
    );
    let mastering = color.mastering.as_ref().unwrap();
    assert_eq!(mastering.primaries[1], (0.17, 0.797));
    assert_eq!(mastering.white_point, (0.3127, 0.329));
    assert_eq!((mastering.luminance_max, mastering.luminance_min), (1000.0, 0.0001));
    assert_eq!(video.encodings, vec![ContentEncoding::HeaderStripping { bytes: vec![0, 0, 0, 1], scope: 1 }]);
    // Legacy AAC ID synthesizes explicit SBR signaling (LC, 24 kHz, stereo to 48 kHz).
    assert_eq!(f.tracks[1].codec, Codec::Aac { asc: vec![0x13, 0x10, 0x56, 0xE5, 0x98] });
    assert_eq!(f.tracks[1].language, "ger");
    assert_eq!(f.chapters[0].chapters[0].title(), Some("Intro"));
    assert_eq!(f.chapters[0].chapters[0].end_ns, Some(40_000_000));
    assert_eq!(f.attachments.len(), 1);
    let attachment = &f.attachments[0];
    assert_eq!((attachment.name.as_str(), attachment.media_type.as_str(), attachment.uid), ("font.ttf", "font/ttf", 77));
    assert_eq!(&bytes[attachment.data_offset as usize..][..100], &[9; 100]);
    assert_eq!(f.tags[0].targets.track_uids, vec![11]);
    assert_eq!(f.tags[0].simple[0].string.as_deref(), Some("Nobody"));
    assert_eq!(f.clusters.len(), 2);
    assert_eq!(video.samples.len(), 3);
    assert_eq!(video.keyframes(), &[0, 2]);
}

fn assert_synthetic_packets(f: &MkvFile, bytes: &[u8]) {
    let packets: Vec<Packet> = Demuxer::from_slice(bytes).unwrap().map(|packet| packet.unwrap()).collect();
    assert_eq!(packets.len(), 4);
    assert_eq!((packets[0].track, packets[0].pts, packets[0].keyframe, packets[0].duration), (0, 1000, true, 40));
    assert_eq!(packets[0].data, [0, 0, 0, 1, 0x65, 1, 2]);
    assert_eq!((packets[1].track, packets[1].pts, packets[1].data.len()), (1, 1005, 10));
    assert_eq!((packets[2].pts, packets[2].keyframe, packets[2].duration), (1040, false, 80));
    assert_eq!(packets[2].data, [0, 0, 0, 1, 0x41, 3]);
    assert_eq!((packets[3].pts, packets[3].pts_ns), (1120, 1_120_000_000));
    assert_eq!(f.read_sample(bytes, 0, 1).unwrap(), packets[2].data);
}

fn assert_crc_mismatch_is_reported(bytes: &[u8]) {
    let mut corrupted = bytes.to_vec();
    let title = corrupted.windows(9).position(|window| window == b"Synthetic").unwrap();
    corrupted[title] = b's';
    let file = open_with(&corrupted[..], &OpenOptions { index: true, verify_crc: true, index_limit: None }).unwrap();
    assert_eq!(file.crc_errors.len(), 1);
}

fn assert_seek_builds_an_index(bytes: &[u8]) {
    let mut demuxer = Demuxer::with_options(bytes, OpenOptions { index: false, verify_crc: false, index_limit: None }).unwrap();
    assert_eq!(demuxer.file().tracks.len(), 2);
    assert_eq!(demuxer.seek(0, 1_100_000_000).unwrap().pts, 1000);
    assert_eq!(demuxer.seek(0, 1_130_000_000).unwrap().pts, 1120);
    assert_eq!(demuxer.next_packet().unwrap().unwrap().pts, 1120);
    assert!(demuxer.file().indexed);
}

#[test]
fn synthetic_file() {
    let bytes = synthetic();
    let file = open_with(&bytes[..], &OpenOptions { index: true, verify_crc: true, index_limit: None }).unwrap();
    assert_synthetic_metadata(&file, &bytes);
    assert_synthetic_packets(&file, &bytes);
    assert_crc_mismatch_is_reported(&bytes);
    assert_seek_builds_an_index(&bytes);
}

/// One video track and one cluster at timestamp `ts` holding `content`.
fn one_cluster(ts: u64, content: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    let mut h = Vec::new();
    el_str(&mut h, DOC_TYPE, "matroska");
    el(&mut f, EBML, &h);
    write_id(&mut f, SEGMENT);
    write_unknown_size(&mut f, 8);
    let mut info = Vec::new();
    el_uint(&mut info, TIMESTAMP_SCALE, 1_000_000);
    el(&mut f, INFO, &info);
    let mut v = Vec::new();
    el_uint(&mut v, TRACK_NUMBER, 1);
    el_uint(&mut v, TRACK_UID, 1);
    el_uint(&mut v, TRACK_TYPE, 1);
    el_str(&mut v, CODEC_ID, "V_VP9");
    let mut tr = Vec::new();
    el(&mut tr, TRACK_ENTRY, &v);
    el(&mut f, TRACKS, &tr);
    write_id(&mut f, CLUSTER);
    write_unknown_size(&mut f, 8);
    el_uint(&mut f, TIMESTAMP, ts);
    f.extend_from_slice(content);
    f
}

/// Counts the bytes read from `data`.
struct Counting<'a>(&'a [u8], std::cell::Cell<u64>);

impl ByteSource for Counting<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.1.set(self.1.get() + buf.len() as u64);
        self.0.read_at(offset, buf)
    }
}

/// Fake Cluster candidates every 16 bytes: a Cluster of 512 KiB whose first child is a 64 KiB Void, so
/// `plausible` has to look 64 KiB away from each candidate and then finds a Cluster ID, not a Timestamp.
fn dense_fake_clusters(total: usize) -> Vec<u8> {
    let unit = fake_cluster_unit();
    assert_eq!(unit.len(), 16);
    let mut f = Vec::new();
    let mut h = Vec::new();
    el_str(&mut h, DOC_TYPE, "matroska");
    el(&mut f, EBML, &h);
    write_id(&mut f, SEGMENT);
    write_unknown_size(&mut f, 8);
    // an undecodable first Segment child forces a resync into the fake candidates
    f.push(0x00);
    while f.len() < total {
        f.extend_from_slice(&unit);
    }
    f
}

#[test]
fn resync_over_dense_fake_clusters_is_bounded_by_the_recovery_budget() {
    let f = dense_fake_clusters(1 << 20);
    let src = Counting(&f, std::cell::Cell::new(0));
    let _ = open_with(&src, &OpenOptions::default());
    let read = src.1.get();
    // 64 MiB of recovery work plus the one candidate that overdraws it; unbounded this is gigabytes
    assert!(read <= (64 << 20) + (1 << 20), "read {read} bytes of a {}-byte file", f.len());
}

/// `prefix` followed by zeros up to `len` bytes, without holding them.
struct Sparse {
    prefix: Vec<u8>,
    len: u64,
}

impl ByteSource for Sparse {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|e| e > self.len) {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        if buf.len() > 64 << 20 {
            return Err(std::io::Error::other("a read this large is the bug under test"));
        }
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.prefix.get((offset as usize).saturating_add(i)).copied().unwrap_or(0);
        }
        Ok(())
    }
}

#[test]
fn a_block_larger_than_u32_is_skipped_without_reading_its_body() {
    // flags 0x00 / 0x02 / 0x06 / 0x04: no lacing, Xiph, EBML and fixed-size lacing
    for flags in [0x80u8, 0x82, 0x86, 0x84] {
        let frame = (1u64 << 32) + 100;
        let mut hdr = Vec::new();
        write_id(&mut hdr, SIMPLE_BLOCK);
        ebml::write_size(&mut hdr, 4 + frame, 8);
        hdr.extend_from_slice(&[0x81, 0, 0, flags]);
        let prefix = one_cluster(0, &hdr);
        let len = prefix.len() as u64 + frame;
        let f = open_with(Sparse { prefix, len }, &OpenOptions::default());
        let f = f.unwrap_or_else(|e| panic!("flags {flags:#x}: {e:?}"));
        assert!(f.tracks[0].samples.is_empty(), "flags {flags:#x}: {:?}", f.tracks[0].samples);
    }
}

/// One fake Cluster candidate: it costs the resync two window refills, about 128 KiB.
fn fake_cluster_unit() -> Vec<u8> {
    let mut unit = Vec::new();
    write_id(&mut unit, CLUSTER);
    ebml::write_size(&mut unit, 512 << 10, 8);
    write_id(&mut unit, VOID);
    ebml::write_size(&mut unit, 64 << 10, 3);
    unit
}

#[test]
fn packet_reading_keeps_its_recovery_budget_after_the_index_is_built() {
    // a Cluster, about 40 MiB of recovery reads of damage, then a second Cluster
    let mut f = one_cluster(0, &{
        let mut b = Vec::new();
        el(&mut b, SIMPLE_BLOCK, &block(1, 0, 0x80, &[1]));
        b
    });
    f.push(0x00);
    for _ in 0..320 {
        f.extend_from_slice(&fake_cluster_unit());
    }
    f.resize(f.len() + (700 << 10), 0);
    write_id(&mut f, CLUSTER);
    write_unknown_size(&mut f, 8);
    el_uint(&mut f, TIMESTAMP, 10);
    el(&mut f, SIMPLE_BLOCK, &block(1, 0, 0x80, &[2]));
    let mut d = Demuxer::with_options(&f[..], OpenOptions { index: false, verify_crc: false, index_limit: None }).unwrap();
    d.build_index().unwrap();
    assert_eq!(d.file().tracks[0].samples.len(), 2, "the index found both clusters: {:?}", d.file().warnings);
    let mut n = 0;
    while d.next_packet().unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, 2, "reading ended at the damage: {:?}", d.file().warnings);
}
