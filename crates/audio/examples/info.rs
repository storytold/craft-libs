//! Print what a file holds, then decode it whole.
//!
//! `cargo run -p craft-audio --example info --features decode -- song.flac`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: info <file>")?;
    let bytes = std::fs::read(&path)?;
    let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str());

    // Container, codec, rate, speakers and length, from the headers where they say.
    let info = craft_audio::probe(&bytes, ext)?;
    println!("{} in {}, {} Hz, {}, {:.2} s", info.codec.name(), info.container.name(), info.sample_rate, info.layout, info.duration_secs());

    // Every audio track, for files that have several.
    for t in craft_audio::tracks(&bytes, ext)? {
        println!("  track {}: {} {} Hz {} ch", t.index, t.codec.name(), t.sample_rate, t.channels);
    }

    // The whole first track: planar f32, WAV channel order.
    let (_, buf) = craft_audio::decode(&bytes, ext)?;
    println!("decoded {} frames, peak {:.3}", buf.frames(), buf.peak());
    Ok(())
}
