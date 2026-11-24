//! [`HostParser`] (the headers and tiles for a stateless hardware decoder) reads a stream the way
//! [`Decoder`] does: it shows the same pictures, at the same sizes, and hands over each frame's
//! tiles exactly once. Skips without ffmpeg / libsvtav1.

use crate::common::{Spec, ffmpeg, ivf_frames, make};
use craft_video::av1::decode::{Decoder, HostEvent, HostParser};

/// What the fixture exercised: frames with several tiles, and pictures shown again from a slot.
#[derive(Default)]
struct Seen {
    multi_tile: usize,
    show_existing: usize,
}

/// The size of every picture the host would output, in order.
fn host_shown_sizes(frames: &[Vec<u8>]) -> (Vec<(u32, u32)>, Seen) {
    let mut parser = HostParser::new();
    let mut slots = [(0u32, 0u32); 8];
    let (mut shown, mut seen) = (Vec::new(), Seen::default());
    for (i, data) in frames.iter().enumerate() {
        for event in parser.parse(data).unwrap_or_else(|e| panic!("temporal unit {i}: {e}")) {
            match event {
                HostEvent::Frame(f) => {
                    let h = &f.header;
                    let count = h.tile_info.cols * h.tile_info.rows;
                    let numbers: Vec<usize> = f.tiles.iter().map(|t| t.number).collect();
                    assert_eq!(numbers, (0..count).collect::<Vec<_>>(), "temporal unit {i}: each tile once, in order");
                    assert!(f.tiles.iter().all(|t| !t.data.is_empty()), "temporal unit {i}: a tile without data");
                    seen.multi_tile += usize::from(count > 1);
                    let size = (h.upscaled_width, h.frame_height);
                    if h.show_frame {
                        shown.push(size);
                    }
                    for (slot, s) in slots.iter_mut().enumerate() {
                        if (h.refresh_frame_flags >> slot) & 1 == 1 {
                            *s = size;
                        }
                    }
                }
                HostEvent::ShowExisting { slot, key_frame, .. } => {
                    let size = slots[slot];
                    shown.push(size);
                    seen.show_existing += 1;
                    if key_frame {
                        slots = [size; 8];
                    }
                }
            }
        }
    }
    (shown, seen)
}

/// Whether the decoder and the host show the same pictures; what the fixture exercised, or `None`
/// after a skip.
fn agrees(spec: &Spec) -> Option<Seen> {
    let ff = ffmpeg()?;
    let frames = ivf_frames(&make(&ff, spec));
    let mut dec = Decoder::new();
    let mut decoded = Vec::new();
    for (i, data) in frames.iter().enumerate() {
        decoded.extend(dec.decode_pts(data, i as i64).unwrap());
    }
    decoded.extend(dec.flush_result().unwrap());
    let decoded: Vec<(u32, u32)> = decoded.iter().map(|p| (p.width, p.height)).collect();
    let (shown, seen) = host_shown_sizes(&frames);
    assert!(!decoded.is_empty(), "{}: nothing decoded", spec.name);
    assert_eq!(shown, decoded, "{}: the host shows other pictures than the decoder", spec.name);
    Some(seen)
}

/// Hidden reference frames shown later (show_existing_frame) and frames split into four tiles.
#[test]
fn host_parser_shows_what_the_decoder_shows() {
    let spec = Spec::new(
        "host_tiles_hidden",
        "testsrc2=s=512x256:r=25",
        20,
        "yuv420p",
        &["-preset", "8", "-g", "12", "-svtav1-params", "tile-columns=1:tile-rows=1"],
    );
    if let Some(seen) = agrees(&spec) {
        assert!(seen.multi_tile > 0, "{}: the fixture has no frame with several tiles", spec.name);
        assert!(seen.show_existing > 0, "{}: the fixture shows no hidden frame", spec.name);
    }
}

/// Superres: the host's output size is the upscaled width, as the decoder's is. (SVT-AV1 does
/// not split superres frames into tiles.)
#[test]
fn host_parser_sizes_superres_frames_like_the_decoder() {
    agrees(&Spec::new(
        "host_superres",
        "testsrc2=s=512x256:r=25",
        8,
        "yuv420p",
        &["-preset", "8", "-svtav1-params", "superres-mode=1:superres-denom=12:superres-kf-denom=13"],
    ));
}
