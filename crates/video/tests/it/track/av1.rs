//! AV1 in MP4 (an `av1C` record whose config OBUs hold the sequence header).

use super::common;
use craft_media::CodecId;

/// libaom when ffmpeg has it, else SVT-AV1: a fixture name and the encoder arguments.
fn encoder() -> Option<(&'static str, &'static [&'static str])> {
    if common::has_encoder("libaom-av1") {
        Some(("av1_aom", &["-c:v", "libaom-av1", "-cpu-used", "8", "-b:v", "300k", "-g", "12", "-pix_fmt", "yuv420p"]))
    } else if common::has_encoder("libsvtav1") {
        Some(("av1_svt", &["-c:v", "libsvtav1", "-preset", "12", "-g", "12", "-pix_fmt", "yuv420p"]))
    } else {
        craft_media_testkit::skip("ffmpeg has neither libaom-av1 nor libsvtav1")
    }
}

fn fixture() -> Option<(common::Track, common::Fixture)> {
    let (name, args) = encoder()?;
    let f = common::fixture(name, "mp4", args, "yuv420p")?;
    let track = common::mp4(&f.file);
    assert_eq!(track.params.codec, CodecId::Av1);
    Some((track, f))
}

/// The OBUs of a packet as (type, bytes); every OBU has a size field, as MP4 stores them.
fn obus(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let header = data[pos];
        assert!(header & 2 != 0, "an OBU without a size field");
        let mut p = pos + 1 + usize::from(header & 4 != 0);
        let mut size = 0usize;
        for i in 0.. {
            let b = data[p];
            p += 1;
            size |= usize::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                break;
            }
        }
        out.push(((header >> 3) & 0xf, &data[pos..p + size]));
        pos = p + size;
    }
    out
}

#[test]
fn mp4_av1c() {
    let Some((track, f)) = fixture() else { return };
    common::check_track(&track, &f);
}

/// With every sequence header taken out of the packets, the one in `av1C` is what decodes them.
///
/// The record's config OBUs are the stream's own sequence header: SVT-AV1 writes `av1C` before it
/// encodes, with a sequence header that differs from the one it then puts in the stream.
#[test]
fn sequence_header_from_av1c() {
    let Some((mut track, f)) = fixture() else { return };
    let header = obus(&track.packets[0].data).into_iter().find(|o| o.0 == 1).unwrap().1.to_vec();
    track.params.extradata = [&track.params.extradata[..4], &header].concat();
    for p in &mut track.packets {
        p.data = obus(&p.data).into_iter().filter(|o| o.0 != 1).flat_map(|o| o.1.to_vec()).collect();
    }
    let mut bare = track.clone();
    bare.params.extradata.clear();
    let mut dec = craft_video::TrackDecoder::new(&bare.params).unwrap();
    let decoded = bare.packets.iter().filter_map(|p| dec.decode(&p.data, p.pts).ok()).flatten().count();
    assert_eq!(decoded, 0, "the packets decode without a sequence header");
    common::check_track(&track, &f);
}
