//! Demux an MP4 with craft-media and decode its audio packets with `TrackDecoder`: what an app
//! with its own container code (or a network stream) does instead of `decode`.
//!
//! `cargo run -p craft-audio --example packets --features decode -- clip.m4a`

use craft_audio::TrackDecoder;
use craft_media::isobmff;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: packets <file.mp4>")?;
    let bytes = std::fs::read(&path)?;

    let file = isobmff::open(bytes.as_slice())?;
    let track = file.track_of_kind(isobmff::TrackKind::Audio).ok_or("no audio track")?;
    let t = &file.tracks[track];
    // The container-neutral codec description every craft-media demuxer produces.
    let params = t.codec_params().ok_or("no sample entry")?;

    let packets: Vec<Vec<u8>> = (0..t.samples.len()).map(|i| file.read_sample(bytes.as_slice(), track, i)).collect::<Result<_, _>>()?;
    // The first packets reveal HE-AAC that the configuration does not announce.
    let lookahead: Vec<&[u8]> = packets.iter().take(craft_audio::LOOKAHEAD_PACKETS).map(Vec::as_slice).collect();
    let mut dec = TrackDecoder::new(&params, &lookahead)?;
    println!("{} {} Hz {}", dec.codec().name(), dec.sample_rate(), dec.layout());

    let mut frames = 0;
    for p in &packets {
        // A damaged packet is replaced by concealment of its length, so the timeline holds.
        let block = dec.decode(p).unwrap_or_else(|_| dec.conceal(Some(p)));
        frames += block.frames();
    }
    // Container timing (the edit list's priming) is the caller's to apply; `decode` does it.
    println!("{} packets, {frames} decoded frames", packets.len());
    Ok(())
}
