//! MPEG-2 video in MPEG-TS (no configuration), and with its sequence header moved from the
//! packets into `extradata`.

use crate::common;
use craft_media::CodecId;

/// Closed GOPs every 12 frames with B-frames (ffmpeg's encoder needs scene change detection off
/// for closed GOPs).
const MPEG2: &[&str] =
    &["-c:v", "mpeg2video", "-bf", "2", "-g", "12", "-flags", "+cgop", "-sc_threshold", "1000000000", "-q:v", "4", "-pix_fmt", "yuv420p"];

/// MPEG-2's IDCT is not bit-exact by definition (any IEEE 1180-accurate IDCT conforms), so the
/// frames are compared with ffmpeg's within the MPEG-2 oracle tests' tolerance.
fn fixture() -> Option<(common::Track, common::Fixture)> {
    let mut f = common::fixture("mpeg2", "ts", MPEG2, "yuv420p")?;
    f.max_diff = 4;
    let track = common::ts(&f.file);
    assert_eq!((track.params.codec, track.params.extradata.len()), (CodecId::Mpeg2Video, 0));
    Some((track, f))
}

/// The byte range of the sequence header and its extensions in `data`: from the sequence header
/// start code to the next start code that is not an extension.
fn sequence_header(data: &[u8]) -> Option<std::ops::Range<usize>> {
    let codes: Vec<(usize, u8)> = (0..data.len().saturating_sub(3)).filter(|&i| data[i..i + 3] == [0, 0, 1]).map(|i| (i, data[i + 3])).collect();
    let k = codes.iter().position(|c| c.1 == 0xb3)?;
    let end = codes[k + 1..].iter().find(|c| c.1 != 0xb5).map_or(data.len(), |c| c.0);
    Some(codes[k].0..end)
}

#[test]
fn ts() {
    let Some((track, f)) = fixture() else { return };
    common::check_track(&track, &f);
}

/// With every sequence header taken out of the packets, the one in `extradata` is what decodes
/// them, before and after a reset.
#[test]
fn sequence_header_from_extradata() {
    let Some((mut track, f)) = fixture() else { return };
    let first = sequence_header(&track.packets[0].data).unwrap();
    track.params.extradata = track.packets[0].data[first].to_vec();
    for p in &mut track.packets {
        while let Some(r) = sequence_header(&p.data) {
            p.data.drain(r);
        }
    }
    let mut bare = track.clone();
    bare.params.extradata.clear();
    let mut dec = craft_video::TrackDecoder::new(&bare.params).unwrap();
    let decoded = bare.packets.iter().filter_map(|p| dec.decode(&p.data, p.pts).ok()).flatten().count();
    assert_eq!(decoded, 0, "the packets decode without a sequence header");
    common::check_track(&track, &f);
}
