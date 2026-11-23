//! Conversion of decoded VP9 pictures into `craft_video::frame::VideoFrame`.

use super::common;
use craft_video::frame::{ColorFamily, Samples, VideoFrame};

/// An RGB stream converts to R, G, B. The reference decode is ffmpeg's `gbrp` (G, B, R), so the
/// frame's planes 0, 1, 2 must equal reference planes 2, 0, 1.
#[test]
fn rgb_planes_come_out_as_r_g_b() {
    let f = common::fixture("profile1_rgb");
    let Some((ivf, yuv)) = common::ensure(f) else { return };
    let reference = std::fs::read(yuv).unwrap();
    let pics = common::decode_file_threads(&ivf, 1).map_err(|e| e.1).unwrap();
    assert_eq!(pics.len(), f.frames as usize);
    let n = (f.width * f.height) as usize;
    for (i, pic) in pics.into_iter().enumerate() {
        assert_eq!(pic.color.color_space, 7);
        let frame = VideoFrame::try_from(pic).unwrap();
        assert_eq!(frame.family(), ColorFamily::Rgb);
        assert_eq!(frame.color.matrix, 0);
        assert!(frame.color.full_range);
        let refp = &reference[3 * n * i..3 * n * (i + 1)];
        let (g, b, r) = (&refp[..n], &refp[n..2 * n], &refp[2 * n..]);
        for (plane, want, name) in [(0, r, "R"), (1, g, "G"), (2, b, "B")] {
            let Samples::U8(got) = frame.planes()[plane].samples() else { panic!("8-bit storage") };
            assert_eq!(got.as_slice(), want, "frame {i} plane {name}");
        }
        // The test source is not gray, so a swapped order would not pass by accident.
        assert_ne!(r, g);
    }
}

/// A 4:2:2 12-bit stream keeps its subsampling and 16-bit storage.
#[test]
fn ycbcr_422_12bit() {
    let f = common::fixture("profile3_422_12bit");
    let Some((ivf, _)) = common::ensure(f) else { return };
    let pics = common::decode_file_threads(&ivf, 1).map_err(|e| e.1).unwrap();
    let frame = VideoFrame::try_from(pics.into_iter().next().unwrap()).unwrap();
    assert_eq!(frame.family(), ColorFamily::YCbCr);
    assert_eq!(frame.subsampling(), craft_video::frame::Subsampling::S422);
    assert_eq!(frame.bit_depth(), 12);
    assert_eq!((frame.planes()[1].width(), frame.planes()[1].height()), (88, 144));
    assert!(matches!(frame.planes()[0].samples(), Samples::U16(_)));
    assert_eq!(frame.pts, Some(0));
}
