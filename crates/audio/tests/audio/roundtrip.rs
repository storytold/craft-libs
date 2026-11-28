use craft_audio::{
    AudioBuffer, BitDepth, BwfInfo, EncodeFormat, EncodeOptions, SampleFormat, decode, detect_container, encode, probe, wav_float_header,
    wav_float_header_with_bwf,
};

fn sine(sr: u32, channels: usize, frames: usize) -> AudioBuffer {
    let mut b = AudioBuffer::new(sr, channels as u16, frames);
    for (c, ch) in b.channels.iter_mut().enumerate() {
        let f = 220.0 * (c as f32 + 1.0);
        for (i, s) in ch.iter_mut().enumerate() {
            *s = 0.8 * (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32 + c as f32).sin();
        }
    }
    b
}

fn tolerance(depth: BitDepth, dither: bool) -> f32 {
    let lsb = match depth {
        BitDepth::Int16 => 1.0 / 32768.0,
        BitDepth::Int24 => 1.0 / 8_388_608.0,
        BitDepth::Int32 => 1e-7,
        BitDepth::Float32 => 0.0,
    };
    lsb * if dither { 1.6 } else { 0.6 } + 1e-7
}

fn max_err(a: &AudioBuffer, b: &AudioBuffer) -> f32 {
    assert_eq!(a.num_channels(), b.num_channels());
    assert_eq!(a.frames(), b.frames());
    a.channels.iter().zip(&b.channels).flat_map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p - q).abs())).fold(0.0, f32::max)
}

const DEPTHS: [BitDepth; 4] = [BitDepth::Int16, BitDepth::Int24, BitDepth::Int32, BitDepth::Float32];

fn expected_sf(d: BitDepth) -> SampleFormat {
    match d {
        BitDepth::Int16 => SampleFormat::Int16,
        BitDepth::Int24 => SampleFormat::Int24,
        BitDepth::Int32 => SampleFormat::Int32,
        BitDepth::Float32 => SampleFormat::Float32,
    }
}

#[test]
fn wav_and_aiff_roundtrip_all_depths_and_layouts() {
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff] {
        for depth in DEPTHS {
            for channels in [1, 2, 6] {
                for dither in [false, true] {
                    let src = sine(48_000, channels, 4801);
                    let opts = EncodeOptions::new(format).with_bit_depth(depth).with_dither(dither);
                    let bytes = encode(&src, &opts).unwrap();
                    assert_eq!(detect_container(&bytes, None), Some(format.container()));
                    let info = probe(&bytes, None).unwrap();
                    assert_eq!(info.container, format.container());
                    assert_eq!(info.sample_rate, 48_000);
                    assert_eq!(info.channels as usize, channels);
                    assert_eq!(info.frames, 4801);
                    assert_eq!(info.sample_format, expected_sf(depth));
                    let (dinfo, out) = decode(&bytes, None).unwrap();
                    assert_eq!(dinfo, info);
                    assert_eq!(out.sample_rate, 48_000);
                    let err = max_err(&src, &out);
                    assert!(err <= tolerance(depth, dither), "{format:?} {depth:?} {channels}ch dither={dither}: err {err}");
                }
            }
        }
    }
}

/// A file that stopped growing before its header was rewritten: the header for 0 frames, the
/// samples of `buf`, then `extra` bytes of a frame that was never finished.
fn unfinished_wav(buf: &AudioBuffer, opts: &EncodeOptions, extra: usize) -> (Vec<u8>, Vec<u8>) {
    let finished = encode(buf, opts).unwrap();
    let mut file = encode(&AudioBuffer::new(buf.sample_rate, buf.num_channels(), 0), opts).unwrap();
    let head = file.len();
    let sample_bytes = match opts.bit_depth {
        BitDepth::Int16 => 2,
        BitDepth::Int24 => 3,
        _ => 4,
    };
    let data_len = buf.frames() * usize::from(buf.num_channels()) * sample_bytes;
    file.extend_from_slice(&finished[head..head + data_len]);
    file.extend(std::iter::repeat_n(0x55, extra));
    (file, finished)
}

#[test]
fn repair_wav_rewrites_the_sizes_of_an_unfinished_file() {
    let bwf = BwfInfo::default().with_description("take 3").with_time_reference(48_000 * 3600);
    let cases = [
        (sine(48_000, 2, 1_000), EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Float32).with_bwf(bwf.clone()), 5),
        // 3 bytes a frame and an odd frame count: the data needs a pad byte.
        (sine(44_100, 1, 999), EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24).with_bwf(bwf.clone()), 2),
        (sine(44_100, 1, 999), EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24).with_bwf(bwf), 0),
        (sine(48_000, 6, 300), EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int16).with_force_rf64(true), 11),
        (sine(48_000, 2, 0), EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int16), 3),
    ];
    for (buf, opts, extra) in cases {
        let (file, finished) = unfinished_wav(&buf, &opts, extra);
        assert_ne!(file, finished);
        let mut file = std::io::Cursor::new(file);
        let repaired = craft_audio::repair_wav(&mut file).unwrap();
        assert_eq!(repaired.frames, buf.frames() as u64);
        let mut file = file.into_inner();
        assert!(file.len() as u64 >= repaired.len);
        file.truncate(repaired.len as usize);
        assert_eq!(file, finished, "{opts:?}");
        // A finished file stays as it is.
        let mut again = std::io::Cursor::new(file.clone());
        assert_eq!(craft_audio::repair_wav(&mut again).unwrap(), repaired);
        assert_eq!(again.into_inner(), file);
    }
}

/// A file whose data is all zeros past `head`, `len` bytes long, without holding them in memory.
struct SparseWav {
    head: Vec<u8>,
    len: u64,
    pos: u64,
}

impl std::io::Read for SparseWav {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (self.len.saturating_sub(self.pos)).min(buf.len() as u64) as usize;
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = self.head.get(self.pos as usize + i).copied().unwrap_or(0);
        }
        self.pos += n as u64;
        Ok(n)
    }
}

impl std::io::Write for SparseWav {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let end = self.pos as usize + buf.len();
        if end <= self.head.len() {
            self.head[self.pos as usize..end].copy_from_slice(buf);
        } else {
            assert!(buf.iter().all(|&b| b == 0), "only padding is written past the header");
        }
        self.pos += buf.len() as u64;
        self.len = self.len.max(self.pos);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::io::Seek for SparseWav {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            std::io::SeekFrom::Start(p) => p,
            std::io::SeekFrom::End(d) => self.len.checked_add_signed(d).unwrap(),
            std::io::SeekFrom::Current(d) => self.pos.checked_add_signed(d).unwrap(),
        };
        Ok(self.pos)
    }
}

#[test]
fn repair_wav_turns_a_file_past_4_gib_into_rf64() {
    let frames = 600_000_000u64; // 4.8 GB of stereo float
    let head = wav_float_header(48_000, 2, 0).unwrap();
    let len = head.len() as u64 + frames * 8 + 3;
    let mut file = SparseWav { head: head.clone(), len, pos: 0 };
    let repaired = craft_audio::repair_wav(&mut file).unwrap();
    assert_eq!((repaired.frames, repaired.len), (frames, len - 3));
    assert_eq!(file.head, wav_float_header(48_000, 2, frames).unwrap());

    // A JUNK chunk holding other bytes becomes a ds64 chunk with an empty table.
    let mut dirty = head.clone();
    assert_eq!(&dirty[12..16], b"JUNK");
    dirty[20..48].fill(0xAA);
    let mut file = SparseWav { head: dirty, len, pos: 0 };
    craft_audio::repair_wav(&mut file).unwrap();
    assert_eq!(file.head, wav_float_header(48_000, 2, frames).unwrap());

    // A repair that stopped after naming the ds64 chunk but before writing RF64 finishes next time.
    let mut half = head.clone();
    half[12..16].copy_from_slice(b"ds64");
    let mut file = SparseWav { head: half, len, pos: 0 };
    assert_eq!(craft_audio::repair_wav(&mut file).unwrap(), repaired);
    assert_eq!(file.head, wav_float_header(48_000, 2, frames).unwrap());

    // In a RIFF file only the chunk right after WAVE becomes ds64; a later ds64 chunk is left as it is.
    let at = head.windows(4).position(|w| w == b"data").unwrap();
    let later: Vec<u8> = [&b"ds64"[..], &28u32.to_le_bytes(), &[0xBB; 28]].concat();
    let mut two = head.clone();
    two.splice(at..at, later.iter().copied());
    let mut file = SparseWav { head: two, len: len + 36, pos: 0 };
    assert_eq!(craft_audio::repair_wav(&mut file).unwrap().frames, frames);
    assert_eq!((&file.head[..4], &file.head[12..16]), (&b"RF64"[..], &b"ds64"[..]));
    assert_eq!(file.head[at..at + 36], later[..]);

    // In an RF64 file the ds64 chunk right after WAVE takes the sizes; a later ds64 chunk is left as it is.
    let finished = wav_float_header(48_000, 2, frames).unwrap();
    assert_eq!((&finished[..4], &finished[12..16]), (&b"RF64"[..], &b"ds64"[..]));
    let mut stale = finished.clone();
    stale[20..44].fill(0);
    stale.splice(at..at, later.iter().copied());
    let mut file = SparseWav { head: stale, len: len + 36, pos: 0 };
    let r = craft_audio::repair_wav(&mut file).unwrap();
    assert_eq!((r.frames, r.len), (frames, len + 33));
    let field = |at: usize| u64::from_le_bytes(file.head[at..at + 8].try_into().unwrap());
    assert_eq!((field(20), field(28), field(36)), (len + 25, frames * 8, frames));
    assert_eq!(file.head[at..at + 36], later[..]);

    // A RIFF file with no JUNK chunk to turn into ds64 cannot grow past 4 GiB.
    let bare = craft_audio::pcm_wav_header(0, 2, 48_000, 16);
    assert_eq!(&bare[12..16], b"fmt ");
    let len = bare.len() as u64 + (1u64 << 32);
    let mut file = SparseWav { head: bare, len, pos: 0 };
    assert!(matches!(craft_audio::repair_wav(&mut file), Err(craft_audio::AudioError::TooLarge(_))));
}

#[test]
fn repair_wav_writes_the_frame_count_into_the_first_fact_chunk() {
    let mut file = wav_float_header(48_000, 2, 0).unwrap();
    let fact = file.windows(4).position(|w| w == b"fact").unwrap() + 8;
    let at = file.windows(4).position(|w| w == b"data").unwrap();
    let later: Vec<u8> = [&b"fact"[..], &4u32.to_le_bytes(), &[0xCC; 4]].concat();
    file.splice(at..at, later.iter().copied());
    file.extend(std::iter::repeat_n(0, 100 * 8));
    let mut file = std::io::Cursor::new(file);
    assert_eq!(craft_audio::repair_wav(&mut file).unwrap().frames, 100);
    let file = file.into_inner();
    assert_eq!(file[fact..fact + 4], 100u32.to_le_bytes());
    assert_eq!(file[at..at + 12], later[..]);
}

#[test]
fn rf64_data_size_comes_from_the_ds64_chunk_after_wave() {
    let opts = EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int16).with_force_rf64(true);
    let wav = encode(&sine(48_000, 2, 300), &opts).unwrap();
    assert_eq!(&wav[12..16], b"ds64");
    // A later ds64 chunk that gives the data 4 bytes, one frame.
    let mut body = [0u8; 28];
    body[8..16].copy_from_slice(&4u64.to_le_bytes());
    let later = [&b"ds64"[..], &28u32.to_le_bytes(), &body].concat();
    let at = wav.windows(4).position(|w| w == b"data").unwrap();
    let mut two = wav.clone();
    two.splice(at..at, later);
    let (info, buf) = decode(&two, Some("wav")).unwrap();
    assert_eq!((info.frames, buf.frames()), (300, 300));

    // A 16-byte first ds64 chunk gives the reader the data size, so repair does not use the later one.
    let mut short = two;
    short[16..20].copy_from_slice(&16u32.to_le_bytes());
    short[28..36].copy_from_slice(&4u64.to_le_bytes());
    short[36..44].copy_from_slice(&[&b"JUNK"[..], &4u32.to_le_bytes()].concat());
    assert_eq!(decode(&short, Some("wav")).unwrap().1.frames(), 1);
    let repaired = craft_audio::repair_wav(&mut std::io::Cursor::new(short));
    assert!(matches!(repaired, Err(craft_audio::AudioError::Malformed(_))), "{repaired:?}");
}

/// A file whose writes fail once `writes` of them have gone through.
struct FailingWrites {
    file: std::io::Cursor<Vec<u8>>,
    writes: usize,
}

impl std::io::Read for FailingWrites {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.file, buf)
    }
}

impl std::io::Write for FailingWrites {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.writes == 0 {
            return Err(std::io::Error::other("disk full"));
        }
        self.writes -= 1;
        std::io::Write::write(&mut self.file, buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::io::Seek for FailingWrites {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.file, to)
    }
}

#[test]
fn repair_wav_runs_again_after_a_write_error() {
    let mut u8_mono = craft_audio::pcm_wav_header(0, 1, 8_000, 8);
    u8_mono.extend([0x80; 3]);
    let opts = EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24);
    let (int24, _) = unfinished_wav(&sine(44_100, 1, 999), &opts, 2);
    for (file, frames) in [(u8_mono, 3), (int24, 999)] {
        let mut whole = std::io::Cursor::new(file.clone());
        let repaired = craft_audio::repair_wav(&mut whole).unwrap();
        assert_eq!(repaired.frames, frames);
        for writes in 0.. {
            let mut failing = FailingWrites { file: std::io::Cursor::new(file.clone()), writes };
            if craft_audio::repair_wav(&mut failing).is_ok() {
                break;
            }
            let mut again = failing.file;
            assert_eq!(craft_audio::repair_wav(&mut again).unwrap(), repaired, "stopped after {writes} writes");
            assert_eq!(again.into_inner(), *whole.get_ref(), "stopped after {writes} writes");
        }
    }
}

#[test]
fn repair_wav_refuses_what_it_cannot_repair() {
    use craft_audio::AudioError;
    let repair = |bytes: Vec<u8>| craft_audio::repair_wav(&mut std::io::Cursor::new(bytes));
    let wav = encode(&sine(48_000, 1, 100), &EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int16)).unwrap();
    assert!(matches!(repair(b"not a wave file at all".to_vec()), Err(AudioError::Malformed(_))));
    // Every cut inside the header is an error, not a panic.
    let data = wav.windows(4).position(|w| w == b"data").unwrap();
    for cut in 0..data + 8 {
        assert!(repair(wav[..cut].to_vec()).is_err(), "cut at {cut}");
    }
    // A compressed format (format tag 2, Microsoft ADPCM) has no fixed frame size to count by.
    let fmt = wav.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
    let mut adpcm = wav.clone();
    adpcm[fmt..fmt + 2].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(repair(adpcm), Err(AudioError::Unsupported(_))));
    // The chunk walk stops, as the reader's does, after 100,000 chunks.
    let mut many = wav[..12].to_vec();
    many.extend(b"JUNK\0\0\0\0".repeat(100_001));
    many.extend_from_slice(&wav[12..]);
    assert!(matches!(repair(many), Err(AudioError::Malformed(_))));
    // 8-bit mono: an even data length could end in a sample or in the pad after an odd count.
    let u8_mono = craft_audio::pcm_wav_header(0, 1, 8_000, 8);
    for (samples, ok) in [(3, true), (4, false)] {
        let mut file = u8_mono.clone();
        file.extend(std::iter::repeat_n(0x80, samples));
        assert_eq!(repair(file.clone()).map(|r| r.frames).ok(), ok.then_some(3), "{samples} bytes");
        if ok {
            // Once repaired, the size settles it.
            let mut file = std::io::Cursor::new(file);
            let r = craft_audio::repair_wav(&mut file).unwrap();
            assert_eq!(repair(file.into_inner()).unwrap(), r);
        }
    }
    // A block size that is not channels × sample bytes would count the wrong frames.
    let mut odd_align = wav.clone();
    odd_align[fmt + 12..fmt + 14].copy_from_slice(&4u16.to_le_bytes());
    assert!(matches!(repair(odd_align), Err(AudioError::Malformed(_))));
    // Widths the reader does not decode: 16-bit float and 40-bit PCM, each with a block size that fits.
    let mut float16 = wav.clone();
    float16[fmt..fmt + 2].copy_from_slice(&3u16.to_le_bytes());
    let mut pcm40 = wav.clone();
    pcm40[fmt + 12..fmt + 16].copy_from_slice(&[5, 0, 40, 0]);
    for bad in [float16, pcm40] {
        assert!(craft_audio::decode(&bad, Some("wav")).is_err());
        assert!(matches!(repair(bad), Err(AudioError::Unsupported(_))));
    }
    // An extensible file whose subformat is PCM's tag under another GUID is not PCM.
    let ext = encode(&sine(48_000, 2, 100), &EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24)).unwrap();
    let fmt = ext.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
    assert!(repair(ext.clone()).is_ok());
    let mut other_guid = ext.clone();
    other_guid[fmt + 39] ^= 0xFF;
    assert!(matches!(repair(other_guid), Err(AudioError::Unsupported(_))));
    // Extensible fields the reader refuses: a cbSize below 22 and more valid bits than the sample holds.
    let mut short_cb = ext.clone();
    short_cb[fmt + 16..fmt + 18].copy_from_slice(&21u16.to_le_bytes());
    let mut wide_valid = ext.clone();
    wide_valid[fmt + 18..fmt + 20].copy_from_slice(&25u16.to_le_bytes());
    for bad in [short_cb, wide_valid] {
        assert!(craft_audio::decode(&bad, Some("wav")).is_err());
        assert!(matches!(repair(bad), Err(AudioError::Malformed(_))));
    }
    // The Ambisonic PCM subformat {00000001-0721-11D3-8644-C8C1CA000000}, which the reader reads as PCM.
    let mut ambisonic = ext;
    ambisonic[fmt + 26..fmt + 40].copy_from_slice(&[0x00, 0x00, 0x21, 0x07, 0xD3, 0x11, 0x86, 0x44, 0xC8, 0xC1, 0xCA, 0x00, 0x00, 0x00]);
    assert_eq!(repair(ambisonic).unwrap().frames, 100);
}

#[test]
fn float_wav_header_with_bwf_keeps_its_bext_as_the_file_grows() {
    let take = sine(48_000, 2, 1_000);
    let bwf = BwfInfo::default()
        .with_description("take 3")
        .with_originator("recorder")
        .with_origination("2026-11-20", "18:30:00")
        .with_time_reference(48_000 * 66_600);
    let mut file = wav_float_header_with_bwf(48_000, 2, 0, &bwf).unwrap();
    let len = file.len();
    assert!(len > wav_float_header(48_000, 2, 0).unwrap().len());
    file.extend(take.interleaved().iter().flat_map(|s| s.to_le_bytes()));
    file[..len].copy_from_slice(&wav_float_header_with_bwf(48_000, 2, 600, &bwf).unwrap());
    let (info, decoded) = decode(&file, None).unwrap();
    assert_eq!((info.bwf.as_ref(), decoded.frames()), (Some(&bwf), 600));
    file[..len].copy_from_slice(&wav_float_header_with_bwf(48_000, 2, 1_000, &bwf).unwrap());
    let opts = EncodeOptions::default().with_bit_depth(BitDepth::Float32).with_bwf(bwf.clone());
    assert_eq!(file, encode(&take, &opts).unwrap());
    let big = wav_float_header_with_bwf(48_000, 2, 600_000_000, &bwf).unwrap();
    assert_eq!((&big[..4], big.len()), (&b"RF64"[..], len));
    assert!(wav_float_header_with_bwf(48_000, 0, 0, &bwf).is_err());
}
