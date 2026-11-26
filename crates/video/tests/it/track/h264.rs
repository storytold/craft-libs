//! H.264 in MP4 (an `avcC` record, length-prefixed packets), MPEG-TS (Annex B, no configuration)
//! and AVI with the parameter sets in the stream header only (an Annex B configuration).

use super::common;
use craft_media::CodecId;
use craft_video::{TrackDecoder, VideoError};

const X264: &[&str] = &["-c:v", "libx264", "-preset", "fast", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p"];

/// The NAL unit types of an Annex B packet.
fn nal_types(data: &[u8]) -> Vec<u8> {
    craft_media::bitstream::annexb_nals(data).iter().filter_map(|n| n.first()).map(|b| b & 0x1f).collect()
}

/// The parameter sets are in the `avcC` record only, so decoding again after a reset needs them
/// applied again.
#[test]
fn mp4_avcc() {
    let Some(f) = common::fixture("h264_avcc", "mp4", X264, "yuv420p") else { return };
    let track = common::mp4(&f.file);
    assert_eq!((track.params.codec, track.params.extradata.first()), (CodecId::H264, Some(&1)), "an avcC record");
    common::check_track(&track, &f);
}

#[test]
fn ts_annex_b() {
    let Some(f) = common::fixture("h264", "ts", X264, "yuv420p") else { return };
    let track = common::ts(&f.file);
    assert_eq!((track.params.codec, track.params.extradata.len()), (CodecId::H264, 0));
    common::check_track(&track, &f);
}

fn global_header_avi() -> Option<(common::Track, common::Fixture)> {
    let args = ["-c:v", "libx264", "-preset", "fast", "-bf", "0", "-g", "12", "-pix_fmt", "yuv420p", "-flags", "+global_header"];
    let f = common::fixture("h264_global_header", "avi", &args, "yuv420p")?;
    let track = common::avi(&f.file);
    assert_eq!(track.params.codec, CodecId::H264);
    assert_eq!(nal_types(&track.params.extradata).iter().filter(|&&t| t == 7 || t == 8).count(), 2, "an SPS and a PPS in the stream header");
    for (i, p) in track.packets.iter().enumerate() {
        assert!(!nal_types(&p.data).contains(&7), "chunk {i} repeats the SPS");
    }
    Some((track, f))
}

/// AVI written with global headers: the chunks need the parameter sets from the stream header.
#[test]
fn avi_annex_b_configuration() {
    let Some((track, f)) = global_header_avi() else { return };
    common::check_track(&track, &f);
}

/// Annex B configuration with the first picture's slice after the parameter sets is refused,
/// rather than decoding that picture into the reference store.
#[test]
fn annex_b_configuration_with_a_slice_is_refused() {
    let Some((mut track, _)) = global_header_avi() else { return };
    track.params.extradata.extend_from_slice(&track.packets[0].data);
    assert!(nal_types(&track.params.extradata).contains(&5), "an IDR slice");
    match TrackDecoder::new(&track.params) {
        Err(VideoError::Malformed(m)) => assert!(m.contains("NAL unit type 5"), "{m}"),
        other => panic!("{other:?}"),
    }
}
