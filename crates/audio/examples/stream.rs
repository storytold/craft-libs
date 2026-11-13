//! Decode a long file block by block from disk, starting at an offset, without loading it.
//!
//! `cargo run -p craft-audio --example stream --features decode -- recording.m4a 60`

use craft_audio::{DecodeOptions, Decoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: stream <file> [start seconds]")?;
    let start: f64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(0.0);

    let file = std::fs::File::open(&path)?;
    let mut dec = Decoder::open(file, None, &DecodeOptions::default())?;
    let rate = dec.info().sample_rate;
    println!("{} {} Hz {}", dec.info().codec.name(), rate, dec.info().layout);

    // Seeking is sample-exact: the next block starts at exactly this frame.
    dec.seek((start * f64::from(rate)) as u64)?;

    let (mut frames, mut peak) = (0usize, 0f32);
    while let Some(block) = dec.next_block()? {
        frames += block.frames();
        peak = peak.max(block.peak());
    }
    println!("{frames} frames from {start} s, peak {peak:.3}");
    Ok(())
}
