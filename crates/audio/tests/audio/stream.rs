use craft_audio::{AudioBuffer, AudioError, BitDepth, BwfInfo, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode, encode_with_layout};
use std::io::{Cursor, Seek, SeekFrom, Write};

fn tone(channels: usize, frames: usize) -> AudioBuffer {
    let channels = (0..channels).map(|c| (0..frames).map(|i| ((i * (c + 3)) as f32 * 0.013).sin() * 1.2).collect()).collect();
    AudioBuffer { sample_rate: 44_100, channels }
}

fn streamed(buf: &AudioBuffer, opts: &EncodeOptions, layout: Option<ChannelLayout>) -> Vec<u8> {
    let mut out = Cursor::new(b"pre".to_vec());
    out.set_position(3);
    let mut enc = FileEncoder::new(out, opts, buf.channels.len(), buf.sample_rate, layout).unwrap();
    let mut start = 0;
    for len in [1, 7, 4095, 4097, 333].into_iter().cycle() {
        if start >= buf.frames() {
            break;
        }
        let len = len.min(buf.frames() - start);
        let block: Vec<Vec<f32>> = buf.channels.iter().map(|c| c[start..start + len].to_vec()).collect();
        enc.write(&block, len).unwrap();
        start += len;
    }
    enc.finish().unwrap().into_inner()
}

#[test]
fn streamed_files_match_whole_buffer_encodes() {
    let bwf = BwfInfo { description: "take".into(), time_reference: 123_456, ..Default::default() };
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
        let opts = EncodeOptions { format, bit_depth, dither, bwf };
        for channels in [1, 6] {
            let buf = tone(channels, 20_001);
            // A layout applies to the buffer with as many channels.
            let layout = layout.filter(|l| l.channels() == channels);
            let whole = match layout {
                Some(l) => encode_with_layout(&buf, &opts, l),
                None => encode(&buf, &opts),
            }
            .unwrap();
            assert_eq!(streamed(&buf, &opts, layout), [b"pre".as_slice(), &whole].concat(), "{opts:?}, {channels} channels");
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
        let opts = EncodeOptions { format, bit_depth: BitDepth::Int16, dither: false, bwf: None };
        let sink = FailAfter { inner: Cursor::new(Vec::new()), budget: 256 };
        let mut enc = FileEncoder::new(sink, &opts, 2, buf.sample_rate, None).unwrap();
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
        let opts = EncodeOptions { format, bit_depth: BitDepth::Int16, dither: false, bwf: None };
        let mut enc = FileEncoder::new(Cursor::new(Vec::new()), &opts, 2, buf.sample_rate, None).unwrap();
        enc.write(&buf.channels, 100).unwrap();
        assert!(enc.check_room(limit / 2).is_ok(), "{format:?}");
        assert!(matches!(enc.check_room(limit), Err(AudioError::TooLarge(_))), "{format:?}");
        if let Ok(frames) = usize::try_from(limit) {
            assert!(matches!(enc.write(&[], frames), Err(AudioError::TooLarge(_))), "{format:?}");
        }
        assert_eq!(enc.finish().unwrap().into_inner(), encode(&buf, &opts).unwrap());
    }
}
