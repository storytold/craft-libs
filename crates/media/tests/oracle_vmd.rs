//! VMD files written by the test kit (ffmpeg cannot write VMD), read by ffprobe (skipped without
//! it): the picture size, the sound format and the picture rate agree with what the demuxer
//! reports.

use std::process::Command;

use craft_media::vmd::open;
use craft_media_testkit::vmd::{Audio, Frame, Spec, build};

/// `key=value` lines of ffprobe's stream description.
fn probe(bytes: &[u8], name: &str) -> Option<String> {
    let ffprobe = craft_media_testkit::ffprobe()?;
    let dir = craft_media_testkit::private_dir("vmd-probe");
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-show_entries", "stream=codec_name,width,height,sample_rate,channels,r_frame_rate", "-of", "default=nw=1"])
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn ffprobe_agrees_on_the_streams() {
    // (frame length field, audio flags): 8-bit mono, 16-bit mono, new and old stereo
    let sounds = [None, Some((20i16, 0u16)), Some((-20, 0)), Some((-20, 0x200)), Some((-20, 0x8000)), Some((20, 0x8000))];
    for (i, sound) in sounds.into_iter().enumerate() {
        let mut spec = Spec::new(32, 24);
        spec.audio = sound.map(|(frame_length, flags)| Audio { sample_rate: 22050, frame_length, buffers: 4, flags });
        let mut picture = vec![2u8];
        picture.extend(std::iter::repeat_n(7, 32 * 24));
        let blocks: Vec<Vec<Frame>> = (0..3)
            .map(|_| {
                let mut block = vec![Frame::video([0, 0, 31, 23], false, picture.clone())];
                if sound.is_some() {
                    // one block of silence
                    block.insert(0, Frame::audio(3, vec![]));
                }
                block
            })
            .collect();
        let bytes = build(&spec, &blocks);
        let f = open(&bytes).unwrap();
        let Some(report) = probe(&bytes, &format!("streams{i}.vmd")) else { return };
        let has = |line: &str| assert!(report.lines().any(|l| l == line), "case {i}: no `{line}` in\n{report}");

        has("codec_name=vmdvideo");
        has(&format!("width={}", f.header.width));
        has(&format!("height={}", f.header.height));
        let (num, den) = f.frame_rate();
        let g = gcd(num, den);
        has(&format!("r_frame_rate={}/{}", num / g, den / g));
        match f.header.audio {
            Some(a) => {
                has("codec_name=vmdaudio");
                has(&format!("sample_rate={}", a.sample_rate));
                has(&format!("channels={}", a.channels));
            }
            None => assert!(!report.contains("vmdaudio"), "case {i}"),
        }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}
