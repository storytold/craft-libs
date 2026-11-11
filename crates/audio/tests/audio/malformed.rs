//! Hostile inputs must produce `Err` (or a best-effort `Ok`), never a panic.

use craft_audio::{AudioBuffer, AudioError, BitDepth, EncodeFormat, EncodeOptions, decode, encode, probe};
use proptest::prelude::*;

fn small(format: EncodeFormat, depth: BitDepth, channels: usize) -> Vec<u8> {
    let mut b = AudioBuffer::new(44_100, channels as u16, 300);
    for ch in &mut b.channels {
        for (i, s) in ch.iter_mut().enumerate() {
            *s = ((i as f32) * 0.1).sin() * 0.5;
        }
    }
    encode(&b, &EncodeOptions::new(format).with_bit_depth(depth).with_dither(false)).unwrap()
}

#[test]
fn aiff_negative_channel_count_is_malformed() {
    let mut aiff = small(EncodeFormat::Aiff, BitDepth::Int16, 1);
    let comm = aiff.windows(4).position(|w| w == b"COMM").unwrap();
    aiff[comm + 8] = 0x80;
    aiff[comm + 9] = 0;
    assert!(probe(&aiff, None).is_err());
}
fn caf(chunks: &[(&[u8; 4], i64, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"caff\0\x01\0\0".to_vec();
    for (kind, size, body) in chunks {
        out.extend_from_slice(*kind);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(body);
    }
    out
}

fn caf_desc(format: &[u8; 4], bytes_per_packet: u32, frames_per_packet: u32, channels: u32, bits: u32) -> (&'static [u8; 4], i64, Vec<u8>) {
    let mut d = 48_000f64.to_be_bytes().to_vec();
    d.extend_from_slice(format);
    for v in [0, bytes_per_packet, frames_per_packet, channels, bits] {
        d.extend_from_slice(&v.to_be_bytes());
    }
    (b"desc", 32, d)
}

fn packet_table(packets: i64, frames: i64, entries: &[u8]) -> Vec<u8> {
    [&packets.to_be_bytes()[..], &frames.to_be_bytes(), &[0; 8], entries].concat()
}

/// An EBML element: `id`, then `size` as an 8-byte vint (so any size up to 2^56 − 2 fits, and
/// `u64::MAX` writes the reserved "unknown size"), then `body`.
fn ebml(id: &[u8], size: u64, body: &[u8]) -> Vec<u8> {
    let size = if size == u64::MAX { 0x01FF_FFFF_FFFF_FFFF } else { 0x0100_0000_0000_0000 | size };
    [id, &size.to_be_bytes(), body].concat()
}

/// The EBML header of a Matroska file.
fn ebml_header() -> Vec<u8> {
    let doc_type = ebml(&[0x42, 0x82], 8, b"matroska");
    ebml(&[0x1A, 0x45, 0xDF, 0xA3], doc_type.len() as u64, &doc_type)
}

/// #878: symphonia's Matroska reader takes element sizes from the file. With symphonia 0.5 a size
/// far beyond the file was allocated up front and aborted the process (no `catch_unwind` can catch
/// an allocation failure); since 0.6 such reads grow with the data actually read. Keep it that way
/// for the header, for codec data and for block frames.
#[test]
fn matroska_sizes_beyond_the_file_are_errors_not_aborts() {
    let huge = 1u64 << 47;
    // The 30-byte file from the report: EBML header and DocType sizes of 2^48 − 1 and 2^47.
    let reported = [
        0x1A, 0x45, 0xDF, 0xA3, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x42, 0x82, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6D,
        0x61, 0x74, 0x72, 0x6F, 0x73, 0x6B, 0x61,
    ];
    let segment = |body: Vec<u8>| [ebml_header(), ebml(&[0x18, 0x53, 0x80, 0x67], u64::MAX, &body)].concat();
    let info = [ebml(&[0x2A, 0xD7, 0xB1], 3, &[0x0F, 0x42, 0x40]), ebml(&[0x4D, 0x80], 4, b"test"), ebml(&[0x57, 0x41], 4, b"test")].concat();
    let info = ebml(&[0x15, 0x49, 0xA9, 0x66], info.len() as u64, &info);
    // A valid 16-bit PCM track entry's children, followed by `extra`.
    let pcm = |extra: &[u8]| {
        let audio = [ebml(&[0xB5], 8, &48_000f64.to_be_bytes()), ebml(&[0x9F], 1, &[1]), ebml(&[0x62, 0x64], 1, &[16])].concat();
        [
            ebml(&[0xD7], 1, &[1]),
            ebml(&[0x73, 0xC5], 1, &[1]),
            ebml(&[0x83], 1, &[2]),
            ebml(&[0x86], 13, b"A_PCM/INT/LIT"),
            ebml(&[0xE1], audio.len() as u64, &audio),
            extra.to_vec(),
        ]
        .concat()
    };
    // That track with a CodecPrivate claiming 2^46 bytes, inside parents that claim even more: the
    // segment is valid up to that read, which runs out of file.
    let track = ebml(&[0xAE], huge, &pcm(&ebml(&[0x63, 0xA2], huge >> 1, &[0; 16])));
    let codec_data = segment([info.clone(), ebml(&[0x16, 0x54, 0xAE, 0x6B], huge, &track)].concat());
    // The plain track, then a cluster whose SimpleBlock claims 2^47 bytes of frame data.
    let track = pcm(&[]);
    let track = ebml(&[0xAE], track.len() as u64, &track);
    let block = ebml(&[0x1F, 0x43, 0xB6, 0x75], u64::MAX, &[ebml(&[0xE7], 1, &[0]), ebml(&[0xA3], huge, &[0x81, 0, 0, 0x80])].concat());
    let cluster = segment([info, ebml(&[0x16, 0x54, 0xAE, 0x6B], track.len() as u64, &track), block].concat());
    let out_of_file = |r: Result<_, AudioError>| matches!(r, Err(AudioError::Malformed(m)) if m.contains("end of file"));
    for hint in [None, Some("mkv"), Some("wav")] {
        assert!(probe(&reported, hint).is_err(), "{hint:?}");
        assert!(decode(&reported, hint).is_err(), "{hint:?}");
        // Refused at the oversized read itself, not by anything before it.
        assert!(out_of_file(probe(&codec_data, hint).map(|_| ())), "{hint:?}: {:?}", probe(&codec_data, hint).err());
        assert!(out_of_file(decode(&codec_data, hint).map(|_| ())), "{hint:?}");
        // The file and its track open, so decoding reaches the block; the block reader stops at the
        // end of the file, a best-effort read of what is there.
        assert!(probe(&cluster, hint).is_ok(), "{hint:?}: {:?}", probe(&cluster, hint).err());
        assert!(decode(&cluster, hint).map_or(0, |(_, b)| b.frames()) < 1_000, "{hint:?}");
    }
}
fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    [&(8 + body.len() as u32).to_be_bytes()[..], kind, body].concat()
}

/// A mono 16-bit, 48 kHz audio sample entry of `version`, its version fields and codec atoms.
fn sample_entry(kind: &[u8; 4], version: u8, fields: &[u8], codec: &[u8]) -> Vec<u8> {
    let base = [0, 0, 0, 0, 0, 0, 0, 1, 0, version, 0, 0, 0, 0, 0, 0, 0, 1, 0, 16, 0, 0, 0, 0];
    atom(kind, &[&base[..], &(48_000u32 << 16).to_be_bytes(), fields, codec].concat())
}

/// An MP4 of `frames` 2-byte samples described by `entry`; `swap` may replace any version-0 atom.
fn mp4(frames: u32, entry: &[u8], swap: impl Fn(&[u8; 4], Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    let full = |kind: &[u8; 4], fields: &[u32]| {
        let body: Vec<u8> = [0].iter().chain(fields).flat_map(|v: &u32| v.to_be_bytes()).collect();
        swap(kind, atom(kind, &body))
    };
    let moov = |offset: u32| {
        let stsd = atom(b"stsd", &[&[0, 0, 0, 0, 0, 0, 0, 1][..], entry].concat());
        let stbl =
            [stsd, full(b"stts", &[1, frames, 1]), full(b"stsc", &[1, 1, frames, 1]), full(b"stsz", &[2, frames]), full(b"stco", &[1, offset])];
        let minf = [full(b"smhd", &[0]), atom(b"stbl", &stbl.concat())].concat();
        let mdia = [full(b"mdhd", &[0, 0, 48_000, frames, 0]), full(b"hdlr", &[0, u32::from_be_bytes(*b"soun"), 0, 0, 0, 0]), atom(b"minf", &minf)];
        let trak = [full(b"tkhd", &[0; 20]), atom(b"mdia", &mdia.concat())].concat();
        atom(b"moov", &[full(b"mvhd", &[[0, 0, 48_000, frames, 1 << 16].as_slice(), &[0; 18], &[2]].concat()), atom(b"trak", &trak)].concat())
    };
    let ftyp = atom(b"ftyp", b"M4A \0\0\0\0M4A isom");
    let offset = (ftyp.len() + moov(0).len() + 8) as u32;
    let pcm: Vec<u8> = (0..frames).flat_map(|i| (i as i16).wrapping_mul(40).to_le_bytes()).collect();
    [ftyp, moov(offset), atom(b"mdat", &pcm)].concat()
}

/// A false MPEG sync prefix must not let malformed CAF or MP4 size fields bypass the container preflight.
#[test]
fn a_false_sync_does_not_bypass_container_checks() {
    let bad_caf = caf(&[
        caf_desc(b"aac ", 0, 1024, 2, 0),
        (b"pakt", 32, packet_table(1, 1024, &[0x82, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0])),
        (b"data", -1, vec![0; 64]),
    ]);
    assert!(matches!(probe(&bad_caf, None), Err(AudioError::Malformed(m)) if m.contains("larger than the file")));
    let f = [vec![0xFF, 0xE0, 0x00], bad_caf].concat();
    assert!(matches!(probe(&f, None), Err(AudioError::Malformed(m)) if m.contains("larger than the file")));
    assert!(matches!(decode(&f, None), Err(AudioError::Malformed(m)) if m.contains("larger than the file")));

    let pcm = sample_entry(b"sowt", 0, &[], &[]);
    let bad_mp4 = mp4(480, &pcm, |kind, mut a| {
        if kind == b"stsz" {
            a[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        a
    });
    assert!(probe(&bad_mp4, None).is_err());
    let f = [vec![0xFF, 0xE0, 0x00], bad_mp4].concat();
    assert!(matches!(probe(&f, None), Err(AudioError::Malformed(m)) if m.contains("larger than the file")));
    assert!(matches!(decode(&f, None), Err(AudioError::Malformed(m)) if m.contains("larger than the file")));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]


    #[test]
    fn random_bodies_behind_valid_headers_never_panic(body in proptest::collection::vec(any::<u8>(), 0..1024), which in 0usize..4) {
        let mut f = match which {
            0 => b"RIFF\xff\xff\xff\xffWAVE".to_vec(),
            1 => b"RF64\xff\xff\xff\xffWAVE".to_vec(),
            2 => b"FORM\xff\xff\xff\xffAIFC".to_vec(),
            _ => b"FORM\xff\xff\xff\xffAIFF".to_vec(),
        };
        f.extend_from_slice(&body);
        let _ = probe(&f, None);
        let _ = decode(&f, None);
    }

}

/// MP3 in MP4 is decoded by symphonia (PCM, AAC, ALAC, Opus and AC-3 are native), so it reaches
/// the checks in `read/mp4.rs`.
fn mp3_mp4(frames: u32, swap: impl Fn(&[u8; 4], Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    mp4(frames, &sample_entry(b".mp3", 0, &[], &[]), swap)
}

/// An `stts` atom of `n` one-sample runs.
fn one_sample_runs(n: u32) -> Vec<u8> {
    atom(b"stts", &[&[0; 4][..], &n.to_be_bytes(), &[1u32, 1].map(u32::to_be_bytes).concat().repeat(n as usize)[..]].concat())
}

/// soundcraft #1332: symphonia scans an audio track's `stts` table from its start for every
/// packet, so a table of one-sample runs made decoding quadratic. The bound is on that work.
#[test]
fn an_audio_timing_table_with_a_run_per_sample_is_bounded() {
    let frames = 200_000;
    let hostile = mp3_mp4(frames, |k, a| if k == b"stts" { one_sample_runs(frames) } else { a });
    let r = decode(&hostile, None);
    assert!(matches!(&r, Err(AudioError::Malformed(m)) if m.contains("timing table")), "{:?}", r.map(|x| x.0));
    // a table of many runs over few packets is cheap to scan, so it is not refused for its size
    let few = mp3_mp4(100, |k, a| if k == b"stts" { one_sample_runs(100_000) } else { a });
    assert!(!matches!(decode(&few, None), Err(AudioError::Malformed(m)) if m.contains("timing table")));
}

/// The bound follows the sample entry, as symphonia decodes by it, not the handler.
#[test]
fn the_audio_timing_bound_follows_the_sample_entry_not_the_handler() {
    let video_handler = atom(b"hdlr", &[0u32, 0, u32::from_be_bytes(*b"vide"), 0, 0, 0, 0].map(u32::to_be_bytes).concat());
    let frames = 200_000;
    let f = mp3_mp4(frames, |k, a| match k {
        b"stts" => one_sample_runs(frames),
        b"hdlr" => video_handler.clone(),
        _ => a,
    });
    assert!(matches!(decode(&f, None), Err(AudioError::Malformed(m)) if m.contains("timing table")));
}

/// Symphonia keeps the last `stsd` it reads, so the last one decides.
#[test]
fn the_last_sample_entry_decides_whether_the_timing_bound_applies() {
    let video = sample_entry(b"avc1", 0, &[], &[]);
    let audio_stsd = atom(b"stsd", &[&[0, 0, 0, 0, 0, 0, 0, 1][..], &sample_entry(b".mp3", 0, &[], &[])].concat());
    let frames = 200_000;
    let f = mp4(frames, &video, |k, a| if k == b"stts" { [audio_stsd.clone(), one_sample_runs(frames)].concat() } else { a });
    assert!(matches!(decode(&f, None), Err(AudioError::Malformed(m)) if m.contains("timing table")));
}

/// soundcraft #1332: frames per packet times channels sizes the decoder's output buffer, and the
/// frames bound alone left many channels free to multiply it.
#[test]
fn a_caf_packet_of_far_too_many_samples_is_refused() {
    let wide = caf(&[caf_desc(b"ima4", 34, 1 << 20, 18, 16), (b"data", 4, vec![0; 4])]);
    let r = decode(&wide, None);
    assert!(matches!(&r, Err(AudioError::Malformed(m)) if m.contains("declares packets")), "{:?}", r.map(|x| x.0));
    // the same frames in a stereo stream are within the bound
    let stereo = caf(&[caf_desc(b"ima4", 34, 1 << 20, 2, 16), (b"data", 4, vec![0; 4])]);
    assert!(!matches!(decode(&stereo, None), Err(AudioError::Malformed(m)) if m.contains("declares packets")));
}
