//! Write a tone as WAV and FLAC, whole and streamed, and as AAC packets.
//!
//! `cargo run -p craft-audio --example encode --features lossless-encode,aac-encode -- out`

use craft_audio::{AudioBuffer, BitDepth, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode, encode_with_layout};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let rate = 48_000;
    let tone: Vec<f32> = (0..rate).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect();
    let stereo = AudioBuffer { sample_rate: rate as u32, channels: vec![tone.clone(), tone.clone()] };

    // A whole buffer at once.
    let wav = encode(&stereo, &EncodeOptions { format: EncodeFormat::Wav, bit_depth: BitDepth::Int24, dither: true, bwf: None })?;
    std::fs::write(dir.join("tone.wav"), wav)?;

    // A 5.1 file whose layout a channel count alone cannot name (side surrounds).
    let surround = AudioBuffer { sample_rate: rate as u32, channels: vec![tone.clone(); 6] };
    let wav = encode_with_layout(&surround, &EncodeOptions::default(), ChannelLayout::SURROUND_5_1_SIDE)?;
    std::fs::write(dir.join("tone-5.1.wav"), wav)?;

    // Streamed: the same bytes, block by block into a file.
    let opts = EncodeOptions { format: EncodeFormat::Flac, ..Default::default() };
    let mut enc = FileEncoder::new(std::fs::File::create(dir.join("tone.flac"))?, &opts, 2, rate as u32, None)?;
    for chunk in tone.chunks(4096) {
        enc.write(&[chunk, chunk], chunk.len())?;
    }
    enc.finish()?;

    // AAC packets for an MP4 or Matroska muxer, with the configuration its sample entry needs.
    let mut aac = craft_audio::aac::Encoder::new(craft_audio::aac::EncoderConfig::cbr(rate as u32, 2, 128_000))?;
    let mut units = aac.encode(&[&tone, &tone]);
    units.extend(aac.flush());
    println!("{} AAC access units, AudioSpecificConfig {:02x?}, priming {}", units.len(), aac.audio_specific_config(), aac.priming_samples());
    Ok(())
}
