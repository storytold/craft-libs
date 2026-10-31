//! Write a tone as WAV and FLAC, whole and streamed.
//!
//! `cargo run -p craft-audio --example encode --features lossless-encode -- out`

use craft_audio::{AudioBuffer, BitDepth, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir)?;
    let rate = 48_000;
    let tone: Vec<f32> = (0..rate).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect();
    let stereo = AudioBuffer { sample_rate: rate as u32, channels: vec![tone.clone(), tone.clone()] };

    // A whole buffer at once.
    let wav = encode(&stereo, &EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24).with_dither(true))?;
    std::fs::write(dir.join("tone.wav"), wav)?;

    // A 5.1 file whose layout a channel count alone cannot name (side surrounds).
    let surround = AudioBuffer { sample_rate: rate as u32, channels: vec![tone.clone(); 6] };
    let wav = encode(&surround, &EncodeOptions::default().with_layout(ChannelLayout::SURROUND_5_1_SIDE))?;
    std::fs::write(dir.join("tone-5.1.wav"), wav)?;

    // Streamed: the same bytes, block by block into a file.
    let opts = EncodeOptions::new(EncodeFormat::Flac);
    let mut enc = FileEncoder::new(std::fs::File::create(dir.join("tone.flac"))?, &opts, rate as u32, 2)?;
    for chunk in tone.chunks(4096) {
        enc.write(&[chunk, chunk], chunk.len())?;
    }
    enc.finish()?;
    Ok(())
}
