//! ffprobe as the external oracle for what the HEVC parameter sets say.

use std::process::Command;

use craft_video::hevc::{Encoder, EncoderConfig, Frame};

/// The VUI carries the configured colour description, and BT.709 when none is set.
#[test]
fn colour_description_reaches_the_vui() {
    let Some(ffprobe) = craft_media_testkit::ffprobe() else { return };
    let (w, h) = (64usize, 64usize);
    let (y, c) = (vec![100u16; w * h], vec![128u16; w * h / 4]);
    for (codes, names) in [(None, ["bt709", "bt709", "bt709"]), (Some([9u8, 16, 9]), ["bt2020", "smpte2084", "bt2020nc"])] {
        let mut cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
        if let Some([p, t, m]) = codes {
            (cfg.colour_primaries, cfg.transfer_characteristics, cfg.matrix_coeffs) = (p, t, m);
        }
        let mut enc = Encoder::new(cfg).unwrap();
        // Annex B: the parameter sets, then the picture's length-prefixed NAL units.
        let mut stream = Vec::new();
        for set in enc.parameter_sets() {
            stream.extend_from_slice(&[0, 0, 0, 1]);
            stream.extend_from_slice(&set);
        }
        let packet = enc.encode(&Frame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }).unwrap();
        let mut rest = packet.data.as_slice();
        while let Some((len, tail)) = rest.split_first_chunk::<4>() {
            let (nal, tail) = tail.split_at(u32::from_be_bytes(*len) as usize);
            stream.extend_from_slice(&[0, 0, 0, 1]);
            stream.extend_from_slice(nal);
            rest = tail;
        }
        let path = craft_media_testkit::fixture_subdir("hevc").join(format!("colour-{}-{}.hevc", std::process::id(), codes.is_some()));
        std::fs::write(&path, stream).unwrap();
        let out = Command::new(&ffprobe).args(["-v", "error", "-show_streams", "-of", "flat"]).arg(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        let s = String::from_utf8_lossy(&out.stdout);
        let [p, t, m] = names;
        for want in [format!("color_primaries=\"{p}\""), format!("color_transfer=\"{t}\""), format!("color_space=\"{m}\"")] {
            assert!(s.contains(&want), "ffprobe output lacks {want}:\n{s}");
        }
    }
}
