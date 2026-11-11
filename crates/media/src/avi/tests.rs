use super::*;

mod hostile;

#[test]
fn stream_number_supports_decimal_and_hex_ids() {
    assert_eq!(Stream::number(b"10dc"), Some(10));
    assert_eq!(Stream::number(b"0adc"), Some(10));
    assert_eq!(Stream::number(b"00dc"), Some(0));
    assert_eq!(Stream::number(b"09wb"), Some(9));
    assert_eq!(Stream::number(b"0Fdc"), Some(15));
    // `from_str_radix` would take a sign; a chunk id never has one
    assert_eq!(Stream::number(b"+1dc"), None);
    assert_eq!(Stream::number(b"ixdc"), None);
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(ltype: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut d = ltype.to_vec();
    d.extend_from_slice(body);
    chunk(b"LIST", &d)
}

fn strl(video: bool) -> Vec<u8> {
    let mut strh = vec![0u8; 56];
    strh[..4].copy_from_slice(if video { b"vids" } else { b"auds" });
    strh[4..8].copy_from_slice(if video { b"MJPG" } else { &[0; 4] });
    let (scale, rate, ss) = if video { (1u32, 25u32, 0u32) } else { (4, 192_000, 4) };
    strh[20..24].copy_from_slice(&scale.to_le_bytes());
    strh[24..28].copy_from_slice(&rate.to_le_bytes());
    // declared length: 3 frames, 6 audio blocks
    strh[32..36].copy_from_slice(&(if video { 3u32 } else { 6 }).to_le_bytes());
    strh[44..48].copy_from_slice(&ss.to_le_bytes());
    let strf = if video {
        let mut b = vec![0u8; 40];
        b[..4].copy_from_slice(&40u32.to_le_bytes());
        b[4..8].copy_from_slice(&64i32.to_le_bytes());
        b[8..12].copy_from_slice(&48i32.to_le_bytes());
        b[14..16].copy_from_slice(&24u16.to_le_bytes());
        b[16..20].copy_from_slice(b"MJPG");
        b
    } else {
        let mut b = vec![0u8; 18];
        b[..2].copy_from_slice(&1u16.to_le_bytes());
        b[2..4].copy_from_slice(&2u16.to_le_bytes());
        b[4..8].copy_from_slice(&48_000u32.to_le_bytes());
        b[8..12].copy_from_slice(&192_000u32.to_le_bytes());
        b[12..14].copy_from_slice(&4u16.to_le_bytes());
        b[14..16].copy_from_slice(&16u16.to_le_bytes());
        b
    };
    let mut body = chunk(b"strh", &strh);
    body.extend(chunk(b"strf", &strf));
    body.extend(chunk(b"strn", if video { b"Camera\0" } else { b"Mic\0" }));
    list(b"strl", &body)
}

/// A small AVI: 3 video frames (the second a delta, the third odd-sized) and 2 audio chunks,
/// optionally inside a `rec ` list, with an `idx1` whose offsets are relative to `movi` or absolute.
fn avi(idx1: bool, absolute: bool, rec: bool) -> Vec<u8> {
    let mut avih = vec![0u8; 56];
    avih[0..4].copy_from_slice(&40_000u32.to_le_bytes());
    avih[12..16].copy_from_slice(&0x10u32.to_le_bytes());
    avih[16..20].copy_from_slice(&3u32.to_le_bytes());
    avih[24..28].copy_from_slice(&2u32.to_le_bytes());
    avih[32..36].copy_from_slice(&64u32.to_le_bytes());
    avih[36..40].copy_from_slice(&48u32.to_le_bytes());
    let mut hdrl = chunk(b"avih", &avih);
    hdrl.extend(strl(true));
    hdrl.extend(strl(false));
    let hdrl = list(b"hdrl", &hdrl);
    let samples: [(&[u8; 4], Vec<u8>, bool); 5] = [
        (b"00dc", vec![1; 10], true),
        (b"01wb", vec![2; 16], true),
        (b"00dc", vec![3; 6], false),
        (b"01wb", vec![4; 8], true),
        (b"00dc", vec![5; 7], true),
    ];
    let mut body = Vec::new();
    let mut entries = Vec::new();
    for (id, data, key) in &samples {
        entries.push((**id, body.len() as u32 + 4, data.len() as u32, *key));
        body.extend(chunk(id, data));
    }
    let movi_body = if rec {
        let r = list(b"rec ", &body);
        // the samples moved 12 bytes in (the rec list's header and type)
        for e in &mut entries {
            e.1 += 12;
        }
        r
    } else {
        body
    };
    let movi = list(b"movi", &movi_body);
    let mut riff = b"AVI ".to_vec();
    riff.extend(hdrl);
    // position of the `movi` type id: after `RIFF` <size>, what `riff` holds so far, `LIST` <size>
    let movi_at = (8 + riff.len() + 8) as u32;
    riff.extend(movi);
    if idx1 {
        let mut ix = Vec::new();
        for (id, off, len, key) in entries {
            ix.extend_from_slice(&id);
            ix.extend_from_slice(&(if key { 0x10u32 } else { 0 }).to_le_bytes());
            ix.extend_from_slice(&(if absolute { off + movi_at } else { off }).to_le_bytes());
            ix.extend_from_slice(&len.to_le_bytes());
        }
        riff.extend(chunk(b"idx1", &ix));
    }
    chunk(b"RIFF", &riff)
}

fn table(f: &AviFile, s: usize) -> Vec<(u32, bool)> {
    f.streams[s].chunks.iter().map(|c| (c.size, c.key)).collect()
}

#[test]
fn headers_and_every_index_path_agree() {
    for (idx1, absolute, rec, want) in [
        (true, false, false, IndexSource::Idx1),
        (true, true, false, IndexSource::Idx1),
        (true, false, true, IndexSource::Idx1),
        (false, false, false, IndexSource::Scan),
        (false, false, true, IndexSource::Scan),
    ] {
        let file = avi(idx1, absolute, rec);
        let f = open(&file).unwrap();
        let what = format!("idx1 {idx1} absolute {absolute} rec {rec}");
        assert_eq!(f.index, want, "{what}");
        assert_eq!((f.header.width, f.header.height, f.header.total_frames, f.header.micro_sec_per_frame), (64, 48, 3, 40_000));
        let (v, a) = (&f.streams[0], &f.streams[1]);
        assert_eq!((v.kind, v.handler, v.scale, v.rate, v.name.as_deref()), (StreamKind::Video, *b"MJPG", 1, 25, Some("Camera")));
        assert_eq!(v.video.as_ref().map(|b| (b.width, b.height, b.bit_count, b.compression)), Some((64, 48, 24, *b"MJPG")));
        let w = a.audio.as_ref().unwrap();
        assert_eq!((w.format_tag, w.channels, w.sample_rate, w.block_align, w.bits_per_sample, a.sample_size), (1, 2, 48_000, 4, 16, 4));
        // the index knows the delta frame; a scan cannot
        let keys = if idx1 { [true, false, true] } else { [true; 3] };
        assert_eq!(table(&f, 0), vec![(10, keys[0]), (6, keys[1]), (7, keys[2])], "{what}");
        assert_eq!(f.streams[0].keyframes_known, idx1);
        assert_eq!(table(&f, 1), vec![(16, true), (8, true)], "{what}");
        assert_eq!(f.read_chunk(&file, 0, 1).unwrap(), vec![3; 6], "{what}");
        assert_eq!(f.read_chunk(&file, 0, 2).unwrap(), vec![5; 7], "{what}");
        assert_eq!(f.read_chunk(&file, 1, 1).unwrap(), vec![4; 8], "{what}");
        assert!(f.read_chunk(&file, 0, 3).is_err() && f.read_chunk(&file, 5, 0).is_err());
    }
    // the idx1 path can be skipped
    let f = open_with(&avi(true, false, false), OpenOptions { ignore_idx1: true, ..Default::default() }).unwrap();
    assert_eq!(f.index, IndexSource::Scan);
}

#[test]
fn not_avi_and_truncated_recordings() {
    assert!(matches!(open(&b"RIFF\0\0\0\0WAVEfmt ".to_vec()), Err(Error::NotAvi)));
    assert!(matches!(open(&Vec::new()), Err(Error::NotAvi)));
    assert!(sniff(&avi(true, false, false)) && !sniff(b"RIFF"));
    // a recording cut off in its last frame: the headers and the whole frames are there
    let full = avi(false, false, false);
    let cut = full[..full.len() - 4].to_vec();
    let f = open(&cut).unwrap();
    assert_eq!(f.streams[0].chunks.len(), 3);
    assert!(f.read_chunk(&cut, 0, 0).is_ok());
}

/// The idx1 entries of one stream damaged (offsets past the file) while the other's are fine: the
/// video still comes from a scan of `movi` instead of vanishing, its key frames unknown, and the
/// audio keeps its index. A stream with no data at all stays empty.
#[test]
fn a_stream_whose_index_entries_are_damaged_is_scanned() {
    let mut file = avi(true, false, false);
    let ix = file.windows(4).rposition(|w| w == b"idx1").unwrap() + 8;
    for e in file[ix..].as_chunks_mut::<16>().0.iter_mut().filter(|e| &e[..4] == b"00dc") {
        e[8..12].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
    }
    let f = open(&file).unwrap();
    assert_eq!(f.index, IndexSource::Idx1);
    assert_eq!(table(&f, 0), vec![(10, true), (6, true), (7, true)]);
    assert!(!f.streams[0].keyframes_known);
    assert_eq!(f.read_chunk(&file, 0, 1).unwrap(), vec![3; 6]);
    assert_eq!(table(&f, 1), vec![(16, true), (8, true)]);
    assert!(f.streams[1].keyframes_known);
    // with an AVIX extension after it: the first RIFF's frames come first, then the extension's
    let mut extended = file.clone();
    extended.extend(chunk(b"RIFF", &[b"AVIX".to_vec(), list(b"movi", &chunk(b"00dc", &[6; 9]))].concat()));
    let f = open(&extended).unwrap();
    assert_eq!(table(&f, 0), vec![(10, true), (6, true), (7, true), (9, true)]);
    assert_eq!(table(&f, 1), vec![(16, true), (8, true)]);
    // an audio-only recording of a file that declares video: nothing to find
    let mut silent = avi(true, false, false);
    while let Some(i) = silent.windows(4).position(|w| w == b"00dc") {
        silent[i..i + 4].copy_from_slice(b"JUNK");
    }
    let f = open(&silent).unwrap();
    assert!(f.streams[0].chunks.is_empty());
    assert_eq!(table(&f, 1), vec![(16, true), (8, true)]);
}

/// A healthy file whose index leaves a stream empty because it declares no data (an audio track
/// that never recorded a sample) opens from the index alone: the stream is not looked for in
/// `movi`, which a large file would pay for with a read per chunk.
#[test]
fn an_empty_stream_that_declares_no_data_is_not_scanned() {
    struct Counted<'a>(&'a [u8], std::cell::Cell<usize>);
    impl ByteSource for Counted<'_> {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
            self.1.set(self.1.get() + 1);
            self.0.read_at(offset, buf)
        }
    }
    let open_counted = |file: &[u8]| {
        let src = Counted(file, Default::default());
        (open(&src).unwrap(), src.1.get())
    };
    let intact = avi(true, false, false);
    // the audio's index entries gone and its length zero; its chunks stay where a scan would find them
    let mut file = intact.clone();
    let ix = file.windows(4).rposition(|w| w == b"idx1").unwrap() + 8;
    for e in file[ix..].as_chunks_mut::<16>().0.iter_mut().filter(|e| &e[..4] == b"01wb") {
        e[..4].copy_from_slice(b"JUNK");
    }
    let strh = file.windows(4).enumerate().filter(|(_, w)| w == b"strh").nth(1).unwrap().0 + 8;
    file[strh + 32..strh + 36].fill(0);
    let (f, reads) = open_counted(&file);
    assert_eq!(f.index, IndexSource::Idx1);
    assert_eq!(table(&f, 0), vec![(10, true), (6, false), (7, true)]);
    assert!(f.streams[1].chunks.is_empty() && f.streams[1].keyframes_known);
    assert_eq!(reads, open_counted(&intact).1);
}

/// Every single-byte change and every truncation of a valid file opens or fails, never panics, and
/// whatever opens reads its chunks without panicking.
#[test]
fn mutations_never_panic() {
    for base in [avi(true, false, true), avi(false, false, false), avi(true, true, false)] {
        for cut in 0..base.len() {
            let b = base[..cut].to_vec();
            if let Ok(f) = open(&b) {
                for (s, st) in f.streams.iter().enumerate() {
                    for i in 0..st.chunks.len() {
                        let _ = f.read_chunk(&b, s, i);
                    }
                }
            }
        }
        for at in 0..base.len() {
            for v in [0x00u8, 0xFF, 0x7F, 0x80] {
                let mut b = base.clone();
                b[at] = v;
                if let Ok(f) = open(&b) {
                    for (s, st) in f.streams.iter().enumerate() {
                        for i in 0..st.chunks.len() {
                            let _ = f.read_chunk(&b, s, i);
                        }
                    }
                }
            }
        }
    }
}

/// An OpenDML file: each stream's `indx` super index points at an `ix##` standard index (base
/// offset + offsets to the data, bit 31 of the size marking delta frames). Built twice: the second
/// pass knows where the `ix##` chunks landed.
fn avi_odml(ix_at: [u64; 2]) -> (Vec<u8>, [u64; 2]) {
    let indx = |stream: u8, at: u64| {
        let mut b = Vec::new();
        b.extend_from_slice(&4u16.to_le_bytes()); // wLongsPerEntry
        b.extend_from_slice(&[0, 0]); // subtype, AVI_INDEX_OF_INDEXES
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[b'0', b'0' + stream, if stream == 0 { b'd' } else { b'w' }, if stream == 0 { b'c' } else { b'b' }]);
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&at.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        chunk(b"indx", &b)
    };
    let mut hdrl = chunk(b"avih", &[0; 56]);
    for (s, at) in ix_at.iter().enumerate() {
        let l = strl(s == 0);
        // strl = LIST <size> strl …: append indx inside it
        let mut body = l[12..].to_vec();
        body.extend(indx(s as u8, *at));
        hdrl.extend(list(b"strl", &body));
    }
    let mut riff = b"AVI ".to_vec();
    riff.extend(list(b"hdrl", &hdrl));
    // movi: video 10 (key) 6 (delta) 7 (key), audio 16 8; then ix00, ix01
    let movi_data_at = 8 + riff.len() as u64 + 12; // RIFF hdr, so far, LIST hdr + `movi`
    let mut body = Vec::new();
    let mut at: [Vec<(u64, u32, bool)>; 2] = [Vec::new(), Vec::new()];
    for (s, data, key) in
        [(0usize, vec![1u8; 10], true), (1, vec![2; 16], true), (0, vec![3; 6], false), (1, vec![4; 8], true), (0, vec![5; 7], true)]
    {
        let id = if s == 0 { b"00dc" } else { b"01wb" };
        at[s].push((movi_data_at + body.len() as u64 + 8, data.len() as u32, key));
        body.extend(chunk(id, &data));
    }
    let mut where_ix = [0u64; 2];
    for (s, entries) in at.iter().enumerate() {
        let base = 100u64; // offsets are relative to qwBaseOffset
        let mut b = Vec::new();
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&[0, 1]); // subtype 0, AVI_INDEX_OF_CHUNKS
        b.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        b.extend_from_slice(if s == 0 { b"00dc" } else { b"01wb" });
        b.extend_from_slice(&base.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for &(off, size, key) in entries {
            b.extend_from_slice(&((off - base) as u32).to_le_bytes());
            b.extend_from_slice(&(size | if key { 0 } else { 0x8000_0000 }).to_le_bytes());
        }
        where_ix[s] = movi_data_at + body.len() as u64;
        body.extend(chunk(if s == 0 { b"ix00" } else { b"ix01" }, &b));
    }
    riff.extend(list(b"movi", &body));
    (chunk(b"RIFF", &riff), where_ix)
}

#[test]
fn opendml_standard_indexes() {
    let (_, at) = avi_odml([0, 0]);
    let (file, again) = avi_odml(at);
    assert_eq!(at, again, "the layout does not depend on the offsets");
    let f = open(&file).unwrap();
    assert_eq!(f.index, IndexSource::OpenDml);
    assert_eq!(table(&f, 0), vec![(10, true), (6, false), (7, true)]);
    assert_eq!(table(&f, 1), vec![(16, true), (8, true)]);
    assert_eq!(f.read_chunk(&file, 0, 2).unwrap(), vec![5; 7]);
    // ignoring the OpenDML index, the scan finds the same chunks (and skips the ix## chunks)
    let s = open_with(&file, OpenOptions { ignore_odml: true, ..Default::default() }).unwrap();
    assert_eq!(s.index, IndexSource::Scan);
    assert_eq!(s.streams[0].chunks.iter().map(|c| c.offset).collect::<Vec<_>>(), f.streams[0].chunks.iter().map(|c| c.offset).collect::<Vec<_>>());
    // a broken super index (pointing nowhere) falls back to a scan
    let (bad, _) = avi_odml([3, 5]);
    assert_eq!(open(&bad).unwrap().index, IndexSource::Scan);
    // and mutations of the OpenDML file never panic
    for i in 0..file.len() {
        let mut b = file.clone();
        b[i] ^= 0xFF;
        if let Ok(f) = open(&b) {
            for (s, st) in f.streams.iter().enumerate() {
                for k in 0..st.chunks.len() {
                    let _ = f.read_chunk(&b, s, k);
                }
            }
        }
    }
}

#[test]
fn opendml_repeated_index_offsets_are_read_once() {
    let (_, at) = avi_odml([0, 0]);
    let (file, _) = avi_odml(at);
    let mut streams = open(&file).unwrap().streams;
    // A malicious super index repeats each valid standard-index address. It must not multiply
    // its chunks, allocations, or traversal work.
    for (stream, &index) in streams.iter_mut().zip(&at) {
        stream.super_index = vec![index; 100];
    }
    assert!(read_odml(&file, &mut streams));
    assert_eq!(streams[0].chunks.len(), 3);
    assert_eq!(streams[1].chunks.len(), 2);
}

/// Tiny `AVIX` forms whose `movi` lists claim the rest of the file: each list is held to its
/// form, so the chunks after them are neither recorded once per list nor recorded at all.
#[test]
fn lists_do_not_reach_past_their_container() {
    let mut file = avi(false, false, false);
    let forms = 64;
    for _ in 0..forms {
        let mut riff = b"RIFF".to_vec();
        riff.extend_from_slice(&16u32.to_le_bytes());
        riff.extend_from_slice(b"AVIX");
        riff.extend_from_slice(b"LIST");
        riff.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        riff.extend_from_slice(b"movi");
        file.extend(riff);
    }
    for _ in 0..32 {
        file.extend(chunk(b"00dc", &[9; 8]));
    }
    let f = open(&file).unwrap();
    assert_eq!(f.index, IndexSource::Scan);
    assert_eq!(table(&f, 0), vec![(10, true), (6, true), (7, true)]);
    assert_eq!(table(&f, 1), vec![(16, true), (8, true)]);

    // `rec ` lists inside one `movi` that each claim the rest of it
    let mut body = Vec::new();
    for _ in 0..64 {
        // LIST(16) rec  { LIST(huge) rec  … }
        body.extend_from_slice(b"LIST");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(b"rec LIST");
        body.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        body.extend_from_slice(b"rec ");
    }
    for _ in 0..32 {
        body.extend(chunk(b"00dc", &[9; 8]));
    }
    let mut streams = open(&avi(false, false, false)).unwrap().streams;
    clear(&mut streams);
    let movi = list(b"movi", &body);
    scan(&movi, &mut streams, &|_| true, 12, movi.len() as u64, 0);
    assert_eq!(streams[0].chunks.len(), 32);
}

#[test]
fn read_chunk_range_reads_only_the_range_and_refuses_one_outside_the_chunk() {
    let data: Vec<u8> = (0..100).collect();
    let stream = Stream {
        kind: StreamKind::Audio,
        handler: [0; 4],
        scale: 1,
        rate: 1,
        start: 0,
        length: 0,
        sample_size: 0,
        video: None,
        audio: None,
        name: None,
        chunks: vec![Chunk { offset: 10, size: 50, key: true }],
        keyframes_known: false,
        super_index: Vec::new(),
    };
    let file = AviFile { header: MainHeader::default(), streams: vec![stream], index: IndexSource::Scan };
    assert_eq!(file.read_chunk_range(&data, 0, 0, 5, 3).unwrap(), [15, 16, 17]);
    assert_eq!(file.read_chunk_range(&data, 0, 0, 0, 50).unwrap().len(), 50);
    assert!(file.read_chunk_range(&data, 0, 0, 48, 3).is_err(), "past the chunk's size");
    assert!(file.read_chunk_range(&data, 0, 0, usize::MAX, 2).is_err());
    assert!(file.read_chunk_range(&data, 0, 1, 0, 1).is_err());
}

/// A file with the given `hdrl` body and one video chunk in `movi`.
fn avi_with_hdrl(hdrl: &[u8]) -> Vec<u8> {
    let mut riff = b"AVI ".to_vec();
    riff.extend(list(b"hdrl", hdrl));
    riff.extend(list(b"movi", &chunk(b"00dc", &[1; 4])));
    chunk(b"RIFF", &riff)
}

#[test]
fn streams_past_the_limit_are_ignored() {
    let hdrl: Vec<u8> = (0..MAX_STREAMS + 6).flat_map(|_| strl(true)).collect();
    let f = open(&avi_with_hdrl(&hdrl)).unwrap();
    assert_eq!(f.streams.len(), MAX_STREAMS);
}

#[test]
fn a_header_list_past_the_limit_is_refused() {
    let mut hdrl = strl(true);
    hdrl.extend(chunk(b"JUNK", &vec![0; MAX_HEADER as usize - hdrl.len() - 8]));
    assert!(open(&avi_with_hdrl(&hdrl)).is_ok(), "exactly the limit");
    hdrl.extend(chunk(b"JUNK", &[0; 2]));
    assert!(matches!(open(&avi_with_hdrl(&hdrl)), Err(Error::Invalid(_))));
}

#[test]
fn rec_lists_nested_past_the_limit_are_not_scanned() {
    let nested = |depth: u32| {
        let mut body = chunk(b"00dc", &[7; 4]);
        for _ in 0..depth {
            body = list(b"rec ", &body);
        }
        let mut riff = b"AVI ".to_vec();
        riff.extend(list(b"hdrl", &strl(true)));
        riff.extend(list(b"movi", &body));
        open(&chunk(b"RIFF", &riff)).unwrap().streams[0].chunks.len()
    };
    // `movi` itself is depth 0
    assert_eq!(nested(MAX_DEPTH - 1), 1);
    assert_eq!(nested(MAX_DEPTH), 0);
    assert_eq!(nested(500), 0);
}

/// A source that is `.1` long and zero after its real bytes, and counts the bytes read.
struct Sparse(Vec<u8>, u64, std::cell::Cell<u64>);

impl ByteSource for Sparse {
    fn len(&self) -> u64 {
        self.1
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.2.set(self.2.get() + buf.len() as u64);
        buf.fill(0);
        let real = self.0.get(offset.min(self.0.len() as u64) as usize..).unwrap_or_default();
        let n = real.len().min(buf.len());
        buf[..n].copy_from_slice(&real[..n]);
        Ok(())
    }
}

/// A standard index that declares more entries than the index budget is not read (nor
/// allocated): the file opens from a scan.
#[test]
fn opendml_indexes_past_the_byte_budget_are_not_read() {
    let (_, at) = avi_odml([0, 0]);
    let (mut file, _) = avi_odml(at);
    // the video's ix00: claim the rest of a 1 GiB file, full of entries
    let ix = at[0] as usize;
    file[ix + 4..ix + 8].copy_from_slice(&0x3000_0000u32.to_le_bytes());
    file[ix + 12..ix + 16].copy_from_slice(&((MAX_ODML_INDEX_BYTES / 8) as u32 + 1).to_le_bytes());
    let mut streams = open(&file).unwrap().streams;
    let src = Sparse(file, 1 << 30, Default::default());
    assert!(!read_odml(&src, &mut streams));
    assert!(src.2.get() < 1024, "only headers were read, not {} bytes", src.2.get());
    assert!(streams[0].chunks.is_empty());
    // one entry fewer fits the budget in bytes (and is then read)
    let mut file = src.0;
    file[ix + 12..ix + 16].copy_from_slice(&(((MAX_ODML_INDEX_BYTES - 24) / 8) as u32).to_le_bytes());
    let src = Sparse(file, 1 << 30, Default::default());
    read_odml(&src, &mut streams);
    assert!(src.2.get() >= MAX_ODML_INDEX_BYTES);
}

#[test]
fn codec_params_map_fourccs_and_format_tags() {
    use crate::{CodecId, PcmFormat};
    let f = open(&avi(true, false, false)).unwrap();
    let v = f.streams[0].codec_params();
    assert_eq!((v.codec, v.tag, v.width, v.height), (CodecId::Mjpeg, Some(*b"MJPG"), Some(64), Some(48)));
    let a = f.streams[1].codec_params();
    assert_eq!((a.codec, a.sample_rate, a.channels, a.pcm), (CodecId::Pcm, Some(48_000), Some(2), Some(PcmFormat::int(16, true, false))));
    let mut s = f.streams[0].clone();
    let video = |s: &mut Stream, fourcc: &[u8; 4], height: i32| {
        let b = s.video.as_mut().unwrap();
        (b.compression, b.height, b.extra) = (*fourcc, height, vec![1, 2]);
        s.codec_params()
    };
    for (fourcc, codec) in [
        (b"h264", CodecId::H264),
        (b"X264", CodecId::H264),
        (b"hvc1", CodecId::Hevc),
        (b"XVID", CodecId::Mpeg4Video),
        (b"apch", CodecId::ProRes),
        (b"AVdn", CodecId::Dnxhd),
        (b"YUY2", CodecId::Unknown),
        (&[0; 4], CodecId::Unknown),
    ] {
        let p = video(&mut s, fourcc, -48);
        assert_eq!((p.codec, p.tag, p.height, p.extradata.as_slice()), (codec, Some(*fourcc), Some(48), &[1u8, 2][..]), "{fourcc:?}");
    }
    assert_eq!(video(&mut s, b"MJPG", i32::MIN).height, Some(1 << 31));
    let mut s = f.streams[1].clone();
    for (tag, bits, codec, pcm) in [
        (1, 8, CodecId::Pcm, Some(PcmFormat::int(8, false, false))),
        (3, 32, CodecId::Pcm, Some(PcmFormat::float(32, false))),
        (0x55, 0, CodecId::Mp3, None),
        (0x50, 0, CodecId::Mp2, None),
        (0x2000, 0, CodecId::Ac3, None),
        (0xFF, 0, CodecId::Aac, None),
        (0x1234, 0, CodecId::Unknown, None),
    ] {
        let w = s.audio.as_mut().unwrap();
        (w.format_tag, w.bits_per_sample) = (tag, bits);
        let p = s.codec_params();
        assert_eq!((p.codec, p.pcm), (codec, pcm), "tag {tag:#x}");
    }
    // a stream that is neither
    s.audio = None;
    assert_eq!(s.codec_params().codec, CodecId::Unknown);
}

/// `WAVE_FORMAT_EXTENSIBLE` resolves to the tag its sub-format GUID starts with.
#[test]
fn wave_format_extensible_resolves_to_its_sub_format() {
    let file = avi(true, false, false);
    let strf = file.windows(4).enumerate().filter(|(_, w)| w == b"strf").nth(1).unwrap().0;
    // an 18-byte WAVEFORMATEX becomes 40 bytes: cbSize 22, then valid bits, mask, GUID
    let mut ext = file[strf + 8..strf + 26].to_vec();
    ext[..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    ext[16..18].copy_from_slice(&22u16.to_le_bytes());
    ext.extend_from_slice(&[16, 0, 3, 0, 0, 0]);
    ext.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
    // rebuild the stream list around the longer strf
    let strl_at = file.windows(4).enumerate().filter(|(_, w)| w == b"strl").nth(1).unwrap().0 - 8;
    let old = le32(&file, strl_at + 4) as usize;
    let mut body = file[strl_at + 12..strf].to_vec();
    body.extend(chunk(b"strf", &ext));
    body.extend_from_slice(&file[strf + 26..strl_at + 8 + old]);
    let mut hdrl = chunk(b"avih", &[0; 56]);
    hdrl.extend(strl(true));
    hdrl.extend(list(b"strl", &body));
    let f = open(&avi_with_hdrl(&hdrl)).unwrap();
    let w = f.streams[1].audio.as_ref().unwrap();
    assert_eq!((w.format_tag, w.extra.len()), (3, 22));
    assert_eq!(f.streams[1].codec_params().codec, crate::CodecId::Pcm);
}

/// Random multi-byte damage to every kind of file (including OpenDML): opening, reading every
/// chunk and mapping the codecs never panics.
#[test]
fn random_damage_never_panics() {
    let (_, at) = avi_odml([0, 0]);
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for base in [avi(true, false, true), avi(false, false, false), avi(true, true, false), avi_odml(at).0] {
        for _ in 0..4000 {
            let mut b = base.clone();
            for _ in 0..1 + next() % 6 {
                let at = (next() % b.len() as u64) as usize;
                // whole 32-bit fields as often as single bytes: sizes, counts and offsets
                let v = [0u32, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, next() as u32][(next() % 6) as usize].to_le_bytes();
                let n = if next() % 2 == 0 { 1 } else { 4 };
                for (d, s) in b[at..].iter_mut().zip(&v[..n]) {
                    *d = *s;
                }
            }
            b.truncate(b.len() - (next() % 3) as usize * (next() % 40) as usize);
            let Ok(f) = open(&b) else { continue };
            for (s, st) in f.streams.iter().enumerate() {
                let _ = st.codec_params();
                for i in 0..st.chunks.len() {
                    let _ = f.read_chunk(&b, s, i);
                    let _ = f.read_chunk_range(&b, s, i, 1, usize::MAX);
                }
            }
        }
    }
}

/// The bytes after an H.264 / HEVC format header say how the chunks are framed: nothing (sets in
/// band), Annex B parameter sets, or an `avcC` / `hvcC` record with its length-prefix size.
#[test]
fn nal_framing_reads_the_stream_header_setup() {
    let f = open(&avi(true, false, false)).unwrap();
    let mut s = f.streams[0].clone();
    assert_eq!(s.nal_framing(), None, "Motion JPEG has no NAL units");
    assert_eq!(f.streams[1].nal_framing(), None, "nor has audio");
    let mut framing = |fourcc: &[u8; 4], extra: &[u8]| {
        let b = s.video.as_mut().unwrap();
        (b.compression, b.extra) = (*fourcc, extra.to_vec());
        (s.nal_framing(), s.codec_params().extradata == extra)
    };
    assert_eq!(framing(b"H264", &[]), (Some(NalFraming::AnnexB { header_sets: false }), true));
    assert_eq!(framing(b"H264", &[0, 0, 0, 1, 0x67, 0x64]), (Some(NalFraming::AnnexB { header_sets: true }), true));
    assert_eq!(framing(b"hevc", &[0, 0, 1, 0x40, 1]), (Some(NalFraming::AnnexB { header_sets: true }), true));
    // avcC: version 1, profile, compatibility, level, then 6 reserved bits and lengthSizeMinusOne
    assert_eq!(framing(b"avc1", &[1, 0x64, 0, 0x1F, 0xFF, 0xE1]), (Some(NalFraming::LengthPrefixed(4)), true));
    assert_eq!(framing(b"avc1", &[1, 0x64, 0, 0x1F, 0xFD]), (Some(NalFraming::LengthPrefixed(2)), true));
    // hvcC keeps it in byte 21
    let mut hvcc = vec![1u8; 23];
    hvcc[21] = 0xF2;
    assert_eq!(framing(b"hvc1", &hvcc), (Some(NalFraming::LengthPrefixed(3)), true));
    // a record cut short still frames the chunks, with the usual four bytes
    assert_eq!(framing(b"hvc1", &[1, 2, 3]), (Some(NalFraming::LengthPrefixed(4)), true));
    // setup that is neither: left as Annex B with the sets in band
    assert_eq!(framing(b"H264", &[9, 9, 9]), (Some(NalFraming::AnnexB { header_sets: false }), true));
}
