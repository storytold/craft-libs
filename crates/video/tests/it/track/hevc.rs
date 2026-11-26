//! HEVC in MP4 (an `hvcC` record, length-prefixed packets) and MPEG-TS (Annex B, no
//! configuration), 8-bit and 10-bit.

use super::common;
use craft_media::CodecId;

/// Closed GOPs every 12 frames with B-frames, so decoding from a key packet gives every frame
/// from there on.
const X265: &[&str] = &["-c:v", "libx265", "-preset", "fast", "-x265-params", "log-level=error:keyint=12:min-keyint=12:open-gop=0:bframes=2"];

fn args(pix_fmt: &str, mp4: bool) -> Vec<&str> {
    let mut a = X265.to_vec();
    a.extend(["-pix_fmt", pix_fmt]);
    if mp4 {
        a.extend(["-tag:v", "hvc1"]);
    }
    a
}

/// The parameter sets are in the `hvcC` record only, so decoding again after a reset needs them
/// applied again.
#[test]
fn mp4_hvcc() {
    let Some(f) = common::fixture("hevc_hvcc", "mp4", &args("yuv420p", true), "yuv420p") else { return };
    let track = common::mp4(&f.file);
    assert_eq!((track.params.codec, track.params.extradata.first()), (CodecId::Hevc, Some(&1)), "an hvcC record");
    common::check_track(&track, &f);
}

#[test]
fn mp4_hvcc_10bit() {
    let Some(f) = common::fixture("hevc_hvcc_10bit", "mp4", &args("yuv420p10le", true), "yuv420p10le") else { return };
    let track = common::mp4(&f.file);
    let frames = common::decode(&track);
    assert!(frames.iter().all(|f| f.bit_depth() == 10));
    common::check_track(&track, &f);
}

#[test]
fn ts_annex_b() {
    let Some(f) = common::fixture("hevc", "ts", &args("yuv420p", false), "yuv420p") else { return };
    let track = common::ts(&f.file);
    assert_eq!((track.params.codec, track.params.extradata.len()), (CodecId::Hevc, 0));
    common::check_track(&track, &f);
}

/// The NAL unit types of an Annex B packet.
fn nal_types(data: &[u8]) -> Vec<u8> {
    craft_media::bitstream::annexb_nals(data).iter().filter_map(|n| n.first()).map(|b| (b >> 1) & 0x3f).collect()
}

/// AVI written with global headers: the chunks need the VPS, SPS and PPS from the stream header.
/// (ffmpeg writes an empty FourCC for HEVC in AVI unless it is given one.)
#[test]
fn avi_annex_b_configuration() {
    // AVI has no timestamps (a chunk's index stands in), so no B-frames.
    let params = "log-level=error:keyint=12:min-keyint=12:open-gop=0:bframes=0";
    let a = ["-c:v", "libx265", "-preset", "fast", "-x265-params", params, "-pix_fmt", "yuv420p", "-flags", "+global_header", "-tag:v", "HEVC"];
    let Some(f) = common::fixture("hevc_global_header", "avi", &a, "yuv420p") else { return };
    let track = common::avi(&f.file);
    assert_eq!(track.params.codec, CodecId::Hevc);
    let sets = nal_types(&track.params.extradata);
    assert!([32, 33, 34].iter().all(|t| sets.contains(t)), "a VPS, an SPS and a PPS in the stream header: {sets:?}");
    for (i, p) in track.packets.iter().enumerate() {
        assert!(!nal_types(&p.data).contains(&33), "chunk {i} repeats the SPS");
    }
    common::check_track(&track, &f);
}
