use craft_audio::{AudioBuffer, BitDepth, BwfInfo, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode};
use std::io::Cursor;

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
    let out = enc.finish().unwrap();
    // the sink is left at the end of the file
    assert_eq!(out.position(), out.get_ref().len() as u64);
    out.into_inner()
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
        (EncodeFormat::Flac, BitDepth::Int16, false, None, Some(ChannelLayout::SURROUND_5_1_SIDE)),
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
