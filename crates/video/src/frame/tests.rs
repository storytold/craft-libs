use super::*;

fn u8_plane(w: u32, h: u32, fill: u8) -> Plane {
    Plane::packed(w, h, 8, Samples::U8(vec![fill; (w * h) as usize])).unwrap()
}

#[test]
fn plane_checks_buffer_stride_and_depth() {
    assert_eq!(Plane::new(4, 2, 3, 8, Samples::U8(vec![0; 8])), Err(FrameError::Plane("stride is less than the width")));
    // Two rows at stride 6 need 6 + 4 samples: the last row's padding is not required.
    assert!(Plane::new(4, 2, 6, 8, Samples::U8(vec![0; 10])).is_ok());
    assert_eq!(Plane::new(4, 2, 6, 8, Samples::U8(vec![0; 9])), Err(FrameError::Plane("buffer is shorter than the plane")));
    assert!(Plane::packed(4, 2, 9, Samples::U8(vec![0; 8])).is_err());
    assert!(Plane::packed(4, 2, 0, Samples::U16(vec![0; 8])).is_err());
    assert!(Plane::packed(4, 2, 17, Samples::U16(vec![0; 8])).is_err());
    assert!(Plane::packed(4, 2, 16, Samples::U16(vec![0; 8])).is_ok());
    assert!(Plane::packed(0, 0, 8, Samples::U8(Vec::new())).is_ok());
}

#[test]
fn plane_size_overflow_is_an_error() {
    assert_eq!(Plane::new(1, u32::MAX, usize::MAX, 8, Samples::U8(Vec::new())), Err(FrameError::Plane("plane size overflows")));
}

#[test]
fn rows_honour_the_stride() {
    let p = Plane::new(2, 3, 4, 8, Samples::U8(vec![0, 1, 9, 9, 2, 3, 9, 9, 4, 5])).unwrap();
    assert_eq!(p.row_u8(0), Some(&[0, 1][..]));
    assert_eq!(p.row_u8(1), Some(&[2, 3][..]));
    assert_eq!(p.row_u8(2), Some(&[4, 5][..]));
    assert_eq!(p.row_u8(3), None);
    assert_eq!(p.row_u16(0), None);
    let q = Plane::packed(1, 2, 10, Samples::U16(vec![7, 1023])).unwrap();
    assert_eq!(q.row_u16(1), Some(&[1023][..]));
    assert_eq!(q.row_u8(0), None);
}

#[test]
fn frame_checks_planes_against_its_shape() {
    let planes = || vec![u8_plane(5, 3, 0), u8_plane(3, 2, 0), u8_plane(3, 2, 0)];
    // Odd sizes round the chroma up.
    let f = VideoFrame::new(5, 3, ColorFamily::YCbCr, Subsampling::S420, planes(), None).unwrap();
    assert_eq!((f.width(), f.height(), f.bit_depth()), (5, 3, 8));
    assert_eq!(f.color, ColorTags::UNSPECIFIED);
    assert_eq!(f.pts, None);
    assert!(VideoFrame::new(5, 3, ColorFamily::YCbCr, Subsampling::S422, planes(), None).is_err());
    assert!(VideoFrame::new(5, 3, ColorFamily::Gray, Subsampling::S444, planes(), None).is_err());
    assert!(VideoFrame::new(5, 3, ColorFamily::Rgb, Subsampling::S420, planes(), None).is_err());
    assert_eq!(
        VideoFrame::new(5, 3, ColorFamily::YCbCr, Subsampling { x: 4, y: 0 }, planes(), None),
        Err(FrameError::Shape("subsampling shift above 3"))
    );
    assert!(VideoFrame::new(5, 3, ColorFamily::YCbCr, Subsampling::S420, planes(), Some(u8_plane(3, 2, 0))).is_err());
    let a = VideoFrame::new(5, 3, ColorFamily::YCbCr, Subsampling::S420, planes(), Some(u8_plane(5, 3, 255))).unwrap();
    assert_eq!(a.alpha().map(Plane::width), Some(5));
    let g = VideoFrame::new(5, 3, ColorFamily::Gray, Subsampling::S444, vec![u8_plane(5, 3, 0)], None).unwrap();
    assert_eq!(g.planes().len(), 1);
    assert!(g.plane(1).is_none());
}

#[test]
fn chroma_planes_may_have_their_own_depth() {
    let y = Plane::packed(2, 2, 10, Samples::U16(vec![0; 4])).unwrap();
    let c = || Plane::packed(1, 1, 8, Samples::U16(vec![0])).unwrap();
    let f = VideoFrame::new(2, 2, ColorFamily::YCbCr, Subsampling::S420, vec![y, c(), c()], None).unwrap();
    assert_eq!(f.bit_depth(), 10);
    assert_eq!(f.planes()[1].bit_depth(), 8);
}

#[cfg(feature = "h264-decode")]
fn h264_picture(y_len: usize) -> crate::h264::decode::Picture {
    crate::h264::decode::Picture {
        width: 4,
        height: 2,
        chroma_width: 2,
        chroma_height: 1,
        y: vec![16; y_len],
        u: vec![128; 2],
        v: vec![128; 2],
        y_stride: 4,
        uv_stride: 2,
        pts: 7,
        poc: 0,
        key: true,
        color: crate::h264::decode::ColorInfo { full_range: true, primaries: 1, transfer: 1, matrix: 1 },
        sar: (0, 0),
        draft: false,
    }
}

#[cfg(feature = "h264-decode")]
#[test]
fn h264_picture_converts() {
    let f = VideoFrame::try_from(h264_picture(8)).unwrap();
    assert_eq!((f.family(), f.subsampling()), (ColorFamily::YCbCr, Subsampling::S420));
    assert_eq!(f.color, ColorTags { primaries: 1, transfer: 1, matrix: 1, full_range: true });
    assert_eq!((f.pts, f.sar), (Some(7), None));
    let mut p = h264_picture(8);
    p.sar = (4, 3);
    assert_eq!(VideoFrame::try_from(p).unwrap().sar, Some((4, 3)));
}

/// The decoder's pictures have public fields; one whose buffer does not fit its size is an error,
/// not an out-of-bounds read later.
#[cfg(feature = "h264-decode")]
#[test]
fn h264_picture_with_a_short_buffer_is_an_error() {
    assert_eq!(VideoFrame::try_from(h264_picture(7)), Err(FrameError::Plane("buffer is shorter than the plane")));
}

#[cfg(feature = "hevc-decode")]
#[test]
fn hevc_picture_converts() {
    use crate::hevc::decode::{ChromaFormat, ColorInfo, Picture, Plane as P};
    let p = Picture {
        width: 4,
        height: 2,
        chroma_width: 2,
        chroma_height: 2,
        chroma: ChromaFormat::C422,
        bit_depth: 10,
        bit_depth_chroma: 10,
        y: P::U16(vec![64; 8]),
        u: P::U16(vec![512; 4]),
        v: P::U16(vec![512; 4]),
        y_stride: 4,
        uv_stride: 2,
        pts: 3,
        poc: 0,
        key: true,
        color: ColorInfo { full_range: false, primaries: 9, transfer: 16, matrix: 9 },
        sar: (1, 1),
        draft: true,
    };
    let f = VideoFrame::try_from(p.clone()).unwrap();
    assert_eq!((f.subsampling(), f.bit_depth()), (Subsampling::S422, 10));
    assert_eq!(f.planes()[2].row_u16(1), Some(&[512, 512][..]));
    assert_eq!(f.color, ColorTags { primaries: 9, transfer: 16, matrix: 9, full_range: false });
    assert_eq!((f.sar, f.draft), (Some((1, 1)), true));

    // 10-bit luma with 8-bit chroma: both stored as `u16`, each plane with its own depth.
    let mixed = VideoFrame::try_from(Picture { bit_depth_chroma: 8, u: P::U16(vec![128; 4]), v: P::U16(vec![128; 4]), ..p }).unwrap();
    assert_eq!(mixed.planes().iter().map(Plane::bit_depth).collect::<Vec<_>>(), [10, 8, 8]);
    assert_eq!(mixed.planes()[1].row_u16(0), Some(&[128, 128][..]));
}

#[cfg(feature = "av1-decode")]
#[test]
fn av1_monochrome_is_gray() {
    let p = crate::av1::decode::Picture {
        width: 3,
        height: 2,
        bit_depth: 8,
        subsampling_x: 1,
        subsampling_y: 1,
        mono_chrome: true,
        planes: [vec![1; 6], Vec::new(), Vec::new()],
        color_primaries: 1,
        transfer_characteristics: 13,
        matrix_coefficients: 1,
        full_range: true,
        pts: 5,
        draft: false,
    };
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!((f.family(), f.subsampling(), f.planes().len()), (ColorFamily::Gray, Subsampling::S444, 1));
    assert_eq!(f.color, ColorTags { primaries: 1, transfer: 13, matrix: 1, full_range: true });
}

#[cfg(feature = "av1-decode")]
#[test]
fn av1_420_rounds_chroma_up() {
    let p = crate::av1::decode::Picture {
        width: 3,
        height: 3,
        bit_depth: 10,
        subsampling_x: 1,
        subsampling_y: 1,
        mono_chrome: false,
        planes: [vec![0; 9], vec![0; 4], vec![0; 4]],
        color_primaries: 2,
        transfer_characteristics: 2,
        matrix_coefficients: 2,
        full_range: false,
        pts: 0,
        draft: false,
    };
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!((f.planes()[1].width(), f.planes()[1].height()), (2, 2));
}

/// The identity matrix codes G, B, R in the Y, U, V planes.
#[cfg(feature = "av1-decode")]
#[test]
fn av1_identity_matrix_is_rgb_reordered_to_r_g_b() {
    let p = crate::av1::decode::Picture {
        width: 2,
        height: 2,
        bit_depth: 8,
        subsampling_x: 0,
        subsampling_y: 0,
        mono_chrome: false,
        planes: [vec![1; 4], vec![2; 4], vec![3; 4]],
        color_primaries: 1,
        transfer_characteristics: 13,
        matrix_coefficients: 0,
        full_range: true,
        pts: 0,
        draft: false,
    };
    let f = VideoFrame::try_from(p.clone()).unwrap();
    assert_eq!(f.family(), ColorFamily::Rgb);
    let first = |f: &VideoFrame, i: usize| f.planes()[i].row_u16(0).and_then(|r| r.first().copied());
    assert_eq!([first(&f, 0), first(&f, 1), first(&f, 2)], [Some(3), Some(1), Some(2)]);
    assert_eq!(f.color, ColorTags { primaries: 1, transfer: 13, matrix: 0, full_range: true });

    // Any other matrix keeps the coded order.
    let y = VideoFrame::try_from(crate::av1::decode::Picture { matrix_coefficients: 1, ..p }).unwrap();
    assert_eq!(y.family(), ColorFamily::YCbCr);
    assert_eq!([first(&y, 0), first(&y, 1), first(&y, 2)], [Some(1), Some(2), Some(3)]);
}

#[cfg(feature = "vp9-decode")]
#[test]
fn vp9_color_space_maps_to_a_matrix() {
    assert_eq!([0, 1, 2, 3, 4, 5, 6, 7].map(convert::vp9_matrix), [2, 5, 1, 6, 7, 9, 2, 0]);
}

#[cfg(feature = "mjpeg")]
#[test]
fn mjpeg_frames_split_into_planes() {
    use crate::mjpeg::{Frame as M, Samples as S};
    // 2x2 RGB: pixels (1,2,3) (4,5,6) (7,8,9) (10,11,12).
    let rgb = M { width: 2, height: 2, channels: 3, samples: S::U8((1..=12).collect()), damaged: false };
    let f = VideoFrame::try_from(rgb).unwrap();
    assert_eq!((f.family(), f.subsampling(), f.bit_depth()), (ColorFamily::Rgb, Subsampling::S444, 8));
    assert_eq!(f.color, ColorTags { matrix: 0, full_range: true, ..ColorTags::UNSPECIFIED });
    let plane = |i: usize| f.plane(i).unwrap().samples().clone();
    assert_eq!(plane(0), Samples::U8(vec![1, 4, 7, 10]));
    assert_eq!(plane(1), Samples::U8(vec![2, 5, 8, 11]));
    assert_eq!(plane(2), Samples::U8(vec![3, 6, 9, 12]));
    assert!(f.alpha().is_none() && f.pts.is_none());
    // A 12-bit gray frame is stored at 16 bits.
    let gray = M { width: 3, height: 1, channels: 1, samples: S::U16(vec![0, 32768, 65535]), damaged: false };
    let f = VideoFrame::try_from(gray).unwrap();
    assert_eq!((f.family(), f.bit_depth(), f.planes().len()), (ColorFamily::Gray, 16, 1));
    assert_eq!(f.plane(0).unwrap().samples(), &Samples::U16(vec![0, 32768, 65535]));
}

#[cfg(feature = "mjpeg")]
#[test]
fn mjpeg_frames_that_do_not_fit_are_errors() {
    use crate::mjpeg::{Frame as M, Samples as S};
    let bad = |channels: u8, n: usize| M { width: 2, height: 2, channels, samples: S::U8(vec![0; n]), damaged: false };
    assert!(VideoFrame::try_from(bad(2, 8)).is_err(), "two channels is not a Motion JPEG frame");
    assert!(VideoFrame::try_from(bad(3, 11)).is_err(), "a short buffer");
    assert!(VideoFrame::try_from(bad(1, 5)).is_err(), "a long buffer");
    assert!(VideoFrame::try_from(M { width: u32::MAX, height: u32::MAX, channels: 3, samples: S::U8(Vec::new()), damaged: false }).is_err());
}

#[cfg(feature = "mjpeg")]
#[test]
fn a_decoded_mjpeg_picture_converts() {
    use crate::mjpeg::{Decoder, Encoder, EncoderConfig};
    let (w, h) = (24usize, 16usize);
    let px: Vec<u8> = (0..w * h * 3).map(|i| (i * 7 % 251) as u8).collect();
    let packet = Encoder::new(EncoderConfig::default()).unwrap().encode(w as u32, h as u32, 3, &px).unwrap();
    let decoded = Decoder::new().decode(&packet).unwrap();
    let want_r: Vec<u8> = match &decoded.samples {
        crate::mjpeg::Samples::U8(v) => v.iter().step_by(3).copied().collect(),
        crate::mjpeg::Samples::U16(_) => panic!("an 8-bit picture"),
    };
    let f = VideoFrame::try_from(decoded).unwrap();
    assert_eq!((f.width(), f.height(), f.family()), (24, 16, ColorFamily::Rgb));
    assert_eq!(f.plane(0).unwrap().samples(), &Samples::U8(want_r));
}

#[cfg(feature = "mpeg2-decode")]
#[test]
fn mpeg2_picture_converts() {
    use crate::mpeg2::{ChromaFormat, Picture, PictureType, SequenceInfo};
    let info = SequenceInfo {
        mpeg2: true,
        width: 5,
        height: 4,
        chroma: ChromaFormat::Yuv420,
        profile_and_level: None,
        progressive_sequence: false,
        low_delay: false,
        frame_rate: None,
        sar: (16, 15),
        dar: None,
        colour: Some((5, 5, 5)),
        video_format: None,
        bit_rate: 0,
    };
    let p = Picture {
        width: 5,
        height: 4,
        chroma: ChromaFormat::Yuv420,
        y: vec![16; 20],
        cb: vec![128; 6],
        cr: vec![128; 6],
        pts: 9,
        picture_type: PictureType::I,
        temporal_reference: 0,
        progressive_frame: false,
        top_field_first: false,
        repeat_first_field: false,
        field_pictures: false,
        info: std::sync::Arc::new(info),
    };
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!(f.field_order, FieldOrder::BottomFieldFirst);
    assert_eq!(f.color, ColorTags { primaries: 5, transfer: 5, matrix: 5, full_range: false });
    assert_eq!((f.sar, f.pts), (Some((16, 15)), Some(9)));
}

#[cfg(feature = "vp8-decode")]
#[test]
fn vp8_picture_converts() {
    let p = crate::vp8::Picture {
        width: 3,
        height: 3,
        chroma_width: 2,
        chroma_height: 2,
        y: vec![0; 9],
        u: vec![0; 4],
        v: vec![0; 4],
        pts: 1,
        key: true,
        color_space: 0,
    };
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!(f.color, ColorTags { matrix: 6, ..ColorTags::UNSPECIFIED });
    assert_eq!(f.pts, Some(1));
}

#[cfg(feature = "prores")]
#[test]
fn prores_frame_converts() {
    use crate::prores::{ChromaFormat, ColorInfo, Frame, Interlace};
    let mut p = Frame::new(5, 2, ChromaFormat::Yuv422, 10, true).unwrap();
    p.color = ColorInfo { primaries: 0, transfer: 1, matrix: 0 };
    p.interlace = Interlace::TopFieldFirst;
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!((f.planes()[1].width(), f.planes()[1].height()), (3, 2));
    assert_eq!(f.alpha().map(Plane::bit_depth), Some(10));
    assert_eq!(f.color, ColorTags { primaries: 2, transfer: 1, matrix: 2, full_range: false });
    assert_eq!((f.field_order, f.pts), (FieldOrder::TopFieldFirst, None));
    assert_eq!(f.sar, None);
}

#[cfg(feature = "prores")]
#[test]
fn prores_aspect_ratio_gives_the_sample_aspect_ratio() {
    use crate::prores::{ChromaFormat, Frame};
    let sar = |code: u8, w: u32, h: u32| {
        let mut p = Frame::new(w, h, ChromaFormat::Yuv422, 10, false).unwrap();
        p.aspect_ratio = code;
        VideoFrame::try_from(p).unwrap().sar
    };
    // 0 is unknown and 4 to 15 are reserved.
    assert_eq!([sar(0, 720, 486), sar(4, 720, 486)], [None, None]);
    assert_eq!(sar(1, 720, 486), Some((1, 1)));
    // 720 x 486 shown at 4:3 and at 16:9.
    assert_eq!(sar(2, 720, 486), Some((9, 10)));
    assert_eq!(sar(3, 720, 486), Some((6, 5)));
    // 1920 x 1080 at 16:9 has square pixels.
    assert_eq!(sar(3, 1920, 1080), Some((1, 1)));
    assert_eq!(convert::prores_sar(2, u32::MAX, u32::MAX), Some((4, 3)));
    assert_eq!(convert::prores_sar(2, 0, 4), None);
}

#[cfg(feature = "dnx")]
#[test]
fn dnx_rgb_is_reordered_to_r_g_b() {
    use crate::dnx::{ChromaFormat, Frame};
    let mut p = Frame::new(2, 2, ChromaFormat::Yuv444, 10, false);
    p.rgb = true;
    // Stored G, B, R.
    p.y = vec![1; 4];
    p.cb = vec![2; 4];
    p.cr = vec![3; 4];
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!(f.family(), ColorFamily::Rgb);
    let first = |i: usize| f.planes()[i].row_u16(0).and_then(|r| r.first().copied());
    assert_eq!([first(0), first(1), first(2)], [Some(3), Some(1), Some(2)]);
    assert_eq!(f.color, ColorTags { matrix: 0, full_range: true, ..ColorTags::UNSPECIFIED });
}

#[cfg(feature = "dnx")]
#[test]
fn dnx_ycbcr_keeps_its_order() {
    use crate::dnx::{ChromaFormat, ColorVolume, Frame};
    let mut p = Frame::new(4, 2, ChromaFormat::Yuv420, 8, true);
    p.color_volume = ColorVolume::Bt2020Ncl;
    p.par = (0, 0);
    let f = VideoFrame::try_from(p).unwrap();
    assert_eq!((f.family(), f.subsampling()), (ColorFamily::YCbCr, Subsampling::S420));
    assert_eq!(f.color, ColorTags { matrix: 9, ..ColorTags::UNSPECIFIED });
    assert_eq!(f.sar, None);
    assert!(f.alpha().is_some());
}

#[cfg(any(feature = "apv-decode", feature = "apv-encode"))]
#[test]
fn apv_frames_convert() {
    use crate::apv::{ChromaFormat, Frame};
    let mono = VideoFrame::try_from(Frame::new(3, 2, ChromaFormat::Monochrome, 10, false)).unwrap();
    assert_eq!((mono.family(), mono.planes().len()), (ColorFamily::Gray, 1));
    let rgba = VideoFrame::try_from(Frame::new(3, 2, ChromaFormat::Yuv4444, 12, false)).unwrap();
    assert_eq!((rgba.family(), rgba.subsampling()), (ColorFamily::YCbCr, Subsampling::S444));
    assert!(rgba.alpha().is_some());
    assert_eq!(rgba.color, ColorTags { primaries: 1, transfer: 1, matrix: 1, full_range: false });
    let c422 = VideoFrame::try_from(Frame::new(3, 2, ChromaFormat::Yuv422, 10, false)).unwrap();
    assert_eq!(c422.planes()[1].width(), 2);
}

#[cfg(feature = "ffv1")]
fn ffv1_frame(colorspace: crate::ffv1::Colorspace, shift: (u8, u8), sizes: &[usize]) -> crate::ffv1::Frame {
    crate::ffv1::Frame { width: 3, height: 3, colorspace, bit_depth: 8, chroma_shift: shift, planes: sizes.iter().map(|&n| vec![0; n]).collect() }
}

#[cfg(feature = "ffv1")]
#[test]
fn ffv1_frames_convert() {
    use crate::ffv1::Colorspace;
    let g = VideoFrame::try_from(ffv1_frame(Colorspace::YCbCr, (0, 0), &[9, 9])).unwrap();
    assert_eq!((g.family(), g.alpha().is_some()), (ColorFamily::Gray, true));
    let y = VideoFrame::try_from(ffv1_frame(Colorspace::YCbCr, (1, 1), &[9, 4, 4])).unwrap();
    assert_eq!((y.family(), y.subsampling(), y.alpha().is_some()), (ColorFamily::YCbCr, Subsampling::S420, false));
    assert_eq!(y.color, ColorTags::UNSPECIFIED);
    let r = VideoFrame::try_from(ffv1_frame(Colorspace::Rgb, (0, 0), &[9, 9, 9, 9])).unwrap();
    assert_eq!((r.family(), r.alpha().is_some()), (ColorFamily::Rgb, true));
    assert_eq!(r.color, ColorTags { matrix: 0, full_range: true, ..ColorTags::UNSPECIFIED });
    assert!(VideoFrame::try_from(ffv1_frame(Colorspace::Rgb, (0, 0), &[9])).is_err());
    assert!(VideoFrame::try_from(ffv1_frame(Colorspace::YCbCr, (0, 0), &[])).is_err());
    // FFV1 codes shifts up to 3: an 8 x 1 frame at (3, 0) has one chroma sample per plane.
    let wide = crate::ffv1::Frame {
        width: 8,
        height: 1,
        colorspace: Colorspace::YCbCr,
        bit_depth: 8,
        chroma_shift: (3, 0),
        planes: vec![vec![0; 8], vec![0; 1], vec![0; 1]],
    };
    let wide = VideoFrame::try_from(wide).unwrap();
    assert_eq!(wide.subsampling(), Subsampling { x: 3, y: 0 });
    assert_eq!((wide.planes()[1].width(), wide.planes()[1].height(), wide.planes()[2].samples().len()), (1, 1, 1));
    // A 2 x 2 chroma plane needs 4 samples.
    assert!(VideoFrame::try_from(ffv1_frame(Colorspace::YCbCr, (1, 1), &[9, 3, 4])).is_err());
}
