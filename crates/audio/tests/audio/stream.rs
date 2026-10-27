use craft_audio::{AudioBuffer, AudioError, BitDepth, BwfInfo, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode};
use std::io::{Cursor, Seek, SeekFrom, Write};

fn tone(channels: usize, frames: usize) -> AudioBuffer {
    let channels = (0..channels).map(|c| (0..frames).map(|i| ((i * (c + 3)) as f32 * 0.013).sin() * 1.2).collect()).collect();
    AudioBuffer { sample_rate: 44_100, channels }
}

fn streamed(buf: &AudioBuffer, opts: &EncodeOptions) -> Vec<u8> {
    let mut out = Cursor::new(b"pre".to_vec());
    out.set_position(3);
    let mut enc = FileEncoder::new(out, opts, buf.sample_rate, buf.channels.len() as u16).unwrap();
    let mut start = 0;
    for len in [1, 7, 4095, 4097, 333].into_iter().cycle() {
        if start >= buf.frames() {
            break;
        }
        let len = len.min(buf.frames() - start);
        // Borrowed slices, as a caller with its own buffers passes them.
        let block: Vec<&[f32]> = buf.channels.iter().map(|c| &c[start..start + len]).collect();
        enc.write(&block, len).unwrap();
        start += len;
    }
    enc.finish().unwrap().into_inner()
}

#[test]
fn streamed_files_match_whole_buffer_encodes() {
    let bwf = BwfInfo::default().with_description("take").with_time_reference(123_456);
    let cases = [
        (EncodeFormat::Wav, BitDepth::Int24, true, None, None),
        (EncodeFormat::Wav, BitDepth::Float32, false, None, None),
        (EncodeFormat::Wav, BitDepth::Int16, true, Some(bwf), Some(ChannelLayout::SURROUND_5_1_SIDE)),
        (EncodeFormat::Aiff, BitDepth::Int24, true, None, None),
        (EncodeFormat::Aiff, BitDepth::Float32, false, None, None),
        (EncodeFormat::Flac, BitDepth::Int16, true, None, None),
        (EncodeFormat::Flac, BitDepth::Int24, false, None, None),
    ];
    for (format, bit_depth, dither, bwf, layout) in cases {
        let base = EncodeOptions::new(format).with_bit_depth(bit_depth).with_dither(dither);
        let base = match bwf {
            Some(b) => base.with_bwf(b),
            None => base,
        };
        for channels in [1, 6] {
            let buf = tone(channels, 20_001);
            // A layout applies to the buffer with as many channels.
            let opts = match layout.filter(|l: &ChannelLayout| usize::from(l.channels()) == channels) {
                Some(l) => base.clone().with_layout(l),
                None => base.clone(),
            };
            let whole = encode(&buf, &opts).unwrap();
            assert_eq!(streamed(&buf, &opts), [b"pre".as_slice(), &whole].concat(), "{opts:?}, {channels} channels");
        }
    }
}

/// A `Write + Seek` sink that accepts `budget` bytes in total and then fails every write, so
/// a `write_all` on it appends a prefix of the block and only then reports the error.
struct FailAfter {
    inner: Cursor<Vec<u8>>,
    budget: usize,
}

impl Write for FailAfter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.budget == 0 {
            return Err(std::io::Error::other("sink is full"));
        }
        let n = buf.len().min(self.budget);
        self.budget -= n;
        self.inner.write(&buf[..n])
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for FailAfter {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn a_failed_write_poisons_the_encoder() {
    let buf = tone(2, 20_001);
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff, EncodeFormat::Flac] {
        let opts = EncodeOptions::new(format).with_bit_depth(BitDepth::Int16).with_dither(false);
        let sink = FailAfter { inner: Cursor::new(Vec::new()), budget: 256 };
        let mut enc = FileEncoder::new(sink, &opts, buf.sample_rate, 2).unwrap();
        // The sink accepts a prefix of the first block, then fails.
        assert!(enc.write(&buf.channels, buf.frames()).is_err(), "{format:?}");
        // Later calls must not claim success: the bytes on the sink no longer match the
        // encoder's counters (and for FLAC the failed frame's samples are already gone).
        assert!(matches!(enc.write(&buf.channels, 16), Err(AudioError::Encode(_))), "{format:?}");
        assert!(matches!(enc.finish(), Err(AudioError::Encode(_))), "{format:?}");
    }
}

#[test]
fn blocks_past_a_format_limit_are_refused_before_anything_is_written() {
    let buf = tone(2, 100);
    for (format, limit) in [(EncodeFormat::Aiff, 1u64 << 30), (EncodeFormat::Flac, 1u64 << 36)] {
        let opts = EncodeOptions::new(format).with_bit_depth(BitDepth::Int16).with_dither(false);
        let mut enc = FileEncoder::new(Cursor::new(Vec::new()), &opts, buf.sample_rate, 2).unwrap();
        enc.write(&buf.channels, 100).unwrap();
        assert!(enc.check_room(limit / 2).is_ok(), "{format:?}");
        assert!(matches!(enc.check_room(limit), Err(AudioError::TooLarge(_))), "{format:?}");
        if let Ok(frames) = usize::try_from(limit) {
            assert!(matches!(enc.write::<Vec<f32>>(&[], frames), Err(AudioError::TooLarge(_))), "{format:?}");
        }
        assert_eq!(enc.finish().unwrap().into_inner(), encode(&buf, &opts).unwrap());
    }
}

// A source that stops serving reads past `fail_from`, as a network-backed file might.
struct FlakySource {
    bytes: Vec<u8>,
    fail_from: u64,
}

use craft_media::ByteSource;

impl ByteSource for FlakySource {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        if offset + buf.len() as u64 > self.fail_from {
            return Err(std::io::Error::other("connection reset"));
        }
        self.bytes.as_slice().read_at(offset, buf)
    }
}

#[test]
fn decoder_reads_any_read_seek_source_and_reports_source_failures() {
    use craft_audio::{DecodeOptions, Decoder};
    use craft_media::ReadSeekSource;
    let wav = encode(&tone(2, 48_000), &EncodeOptions::new(EncodeFormat::Wav)).unwrap();

    // `ReadSeekSource` (any `Read + Seek`) is accepted, so a `BufReader<File>` works the same way.
    let source = ReadSeekSource::new(Cursor::new(wav.clone())).unwrap();
    let info = Decoder::open(source, Some("wav"), &DecodeOptions::default()).unwrap().probe().unwrap();
    assert_eq!((info.frames, info.damaged), (48_000, 0));

    // A source failing half way through gives an I/O error, not concealed silence.
    let half = wav.len() as u64 / 2;
    let mut dec = Decoder::open(FlakySource { bytes: wav, fail_from: half }, Some("wav"), &DecodeOptions::default()).unwrap();
    let err = loop {
        match dec.next_block() {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the source failure was not reported"),
            Err(e) => break e,
        }
    };
    assert!(matches!(err, AudioError::Io(_)), "{err}");
    assert_eq!(dec.info().damaged, 0);
}

#[test]
fn seeking_to_or_past_the_end_ends_the_stream() {
    use craft_audio::{DecodeOptions, Decoder};
    let buf = tone(2, 4800);
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff, EncodeFormat::Flac] {
        let file = encode(&buf, &EncodeOptions::new(format).with_bit_depth(BitDepth::Int16)).unwrap();
        for target in [4800, 4801, 1_000_000] {
            let mut dec = Decoder::from_bytes(&file, None, &DecodeOptions::default()).unwrap();
            assert_eq!(dec.seek(target).unwrap(), 4800, "{format:?} seek({target})");
            assert!(dec.next_block().unwrap().is_none(), "{format:?} seek({target})");
            // Seeking back still works.
            assert_eq!(dec.seek(4000).unwrap(), 4000, "{format:?}");
            let left: usize = std::iter::from_fn(|| dec.next_block().unwrap()).map(|b| b.frames()).sum();
            assert_eq!(left, 800, "{format:?}");
        }
    }
}

#[test]
fn single_track_files_refuse_other_track_indexes() {
    use craft_audio::{DecodeOptions, decode_with};
    let buf = tone(1, 1000);
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff] {
        let file = encode(&buf, &EncodeOptions::new(format)).unwrap();
        assert!(decode_with(&file, None, &DecodeOptions::default().with_track(0)).is_ok(), "{format:?}");
        let e = decode_with(&file, None, &DecodeOptions::default().with_track(1)).unwrap_err();
        assert!(matches!(e, AudioError::Unsupported(_)), "{format:?}: {e}");
    }
}
