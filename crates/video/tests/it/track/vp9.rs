//! VP9 in MP4 (a `vpcC` record, which the decoder does not need).

use super::common;
use craft_media::CodecId;

#[test]
fn mp4() {
    let args = ["-c:v", "libvpx-vp9", "-deadline", "good", "-cpu-used", "5", "-b:v", "400k", "-g", "12", "-pix_fmt", "yuv420p"];
    let Some(f) = common::fixture("vp9", "mp4", &args, "yuv420p") else { return };
    let track = common::mp4(&f.file);
    assert_eq!(track.params.codec, CodecId::Vp9);
    assert!(!track.params.extradata.is_empty(), "a vpcC record");
    common::check_track(&track, &f);
}
