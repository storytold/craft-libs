//! ffprobe as the external oracle for what the AV1 sequence header says.

use std::process::Command;

use craft_video::av1::{Encoder, EncoderConfig, Frame};

/// IVF with one temporal unit (temporal delimiter and the packet).
fn ivf(w: usize, h: usize, packet: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"DKIF");
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(b"AV01");
    v.extend_from_slice(&(w as u16).to_le_bytes());
    v.extend_from_slice(&(h as u16).to_le_bytes());
    v.extend_from_slice(&30u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&((packet.len() + 2) as u32).to_le_bytes());
    v.extend_from_slice(&0u64.to_le_bytes());
    v.extend_from_slice(&[0x12, 0x00]); // OBU_TEMPORAL_DELIMITER
    v.extend_from_slice(packet);
    v
}

/// `color_config` carries the configured colour description, and BT.709 when none is set.
#[test]
fn colour_description_reaches_the_sequence_header() {
    let Some(ffprobe) = craft_media_testkit::ffprobe() else { return };
    let (w, h) = (64usize, 64usize);
    let (y, c) = (vec![100u16; w * h], vec![128u16; w * h / 4]);
    for (codes, names) in [(None, ["bt709", "bt709", "bt709"]), (Some([9u8, 16, 9]), ["bt2020", "smpte2084", "bt2020nc"])] {
        let mut cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
        if let Some([p, t, m]) = codes {
            (cfg.color_primaries, cfg.transfer_characteristics, cfg.matrix_coefficients) = (p, t, m);
        }
        let mut enc = Encoder::new(cfg).unwrap();
        let packet = enc.encode(&Frame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }).unwrap();
        let path = craft_media_testkit::fixture_subdir("av1").join(format!("colour-{}-{}.ivf", std::process::id(), codes.is_some()));
        std::fs::write(&path, ivf(w, h, &packet.data)).unwrap();
        let out = Command::new(&ffprobe).args(["-v", "error", "-show_streams", "-of", "flat"]).arg(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        let s = String::from_utf8_lossy(&out.stdout);
        let [p, t, m] = names;
        for want in [format!("color_primaries=\"{p}\""), format!("color_transfer=\"{t}\""), format!("color_space=\"{m}\"")] {
            assert!(s.contains(&want), "ffprobe output lacks {want}:\n{s}");
        }
    }
}
