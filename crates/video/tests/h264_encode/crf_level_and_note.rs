//! Constant quality has no bitrate cap, so the level must allow what low factors spend; and every
//! stream says which settings made it.

use crate::common;

use common::synth;
use craft_video::h264::*;

fn crf(w: u32, h: u32, fps: u32, f: f32) -> Encoder {
    let mut c = EncoderConfig::new(w, h, fps, 1);
    c.rate = RateControl::Crf(f);
    c.format = PacketFormat::LengthPrefixed;
    Encoder::new(c).unwrap()
}

#[test]
fn crf_level_allows_the_bitrate_of_the_factor() {
    let level = |w, h, fps, f| crf(w, h, fps, f).avcc()[3];
    // 1080p30: the default and visually lossless factors stay at 4; CRF 0 (measured at 105 Mb/s
    // average, 147 Mb/s peak on camera footage) needs more than 4.2's 62.5 Mb/s
    assert_eq!(level(1920, 1080, 30, 23.0), 40);
    assert_eq!(level(1920, 1080, 30, 18.0), 40);
    assert_eq!(level(1920, 1080, 30, 0.0), 51);
    // never below what the picture size and rate need
    assert_eq!(level(3840, 2160, 30, 51.0), 51);
    // a factor out of range is clamped; one that is not a number is refused
    assert_eq!(level(1920, 1080, 30, -100.0), 51);
    let mut c = EncoderConfig::new(1920, 1080, 30, 1);
    c.rate = RateControl::Crf(f32::NAN);
    assert!(matches!(Encoder::new(c), Err(Error::InvalidConfig(_))));
}

#[test]
fn first_access_unit_carries_the_settings() {
    let (w, h) = (64, 48);
    let mut enc = crf(w as u32, h as u32, 30, 18.0);
    let f = synth(w, h, 0);
    let mut packets = enc.encode(&f.frame(), 0).unwrap();
    packets.extend(enc.flush());
    let text = |p: &Packet| String::from_utf8_lossy(&p.data).into_owned();
    let first = text(&packets[0]);
    for want in ["craft-h264enc", "rc=crf crf=18.0", "profile=high", "level=", "bframes=2", "cabac=1"] {
        assert!(first.contains(want), "{want} missing from {first:?}");
    }
    // once per stream
    assert!(packets.iter().skip(1).all(|p| !text(p).contains("craft-h264enc")));
}

#[test]
fn the_writer_named_in_the_stream_is_the_configured_one() {
    let (w, h) = (64, 48);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
    cfg.writer = "SomeApp 1.2.3".into();
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = enc.encode(&synth(w, h, 0).frame(), 0).unwrap();
    packets.extend(enc.flush());
    let first = String::from_utf8_lossy(&packets[0].data).into_owned();
    assert!(first.contains("SomeApp 1.2.3 - clean-room H.264 encoder - options: "), "{first:?}");
    assert!(!first.contains("craft-h264enc"), "{first:?}");
}
