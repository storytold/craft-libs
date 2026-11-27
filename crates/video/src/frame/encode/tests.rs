use super::*;

fn picture(family: ColorFamily, sub: Subsampling, depth: u8, wide: bool, alpha: bool) -> VideoFrame {
    let (w, h) = (4, 2);
    let plane = |pw: u32, ph: u32, value: u16| {
        let stride = pw as usize + 2;
        let mut samples = vec![u16::MAX; stride * ph as usize];
        for row in 0..ph as usize {
            samples[row * stride..row * stride + pw as usize].fill(value);
        }
        let samples = if wide { Samples::U16(samples) } else { Samples::U8(samples.into_iter().map(|v| v as u8).collect()) };
        Plane::new(pw, ph, stride, depth, samples).unwrap()
    };
    let mut planes = vec![plane(w, h, 16)];
    if family != ColorFamily::Gray {
        let (cw, ch) = sub.chroma_size(w, h);
        planes.extend([plane(cw, ch, 32), plane(cw, ch, 48)]);
    }
    VideoFrame::new(w, h, family, sub, planes, alpha.then(|| plane(w, h, 64))).unwrap()
}

#[test]
fn packs_rows_and_changes_storage_without_rescaling() {
    for wide in [false, true] {
        let f = picture(ColorFamily::YCbCr, Subsampling::S420, 8, wide, false);
        let p8 = Planar8::try_from(&f).unwrap();
        let p16 = Planar16::try_from(&f).unwrap();
        assert_eq!(&*p8.planes[0], &[16; 8]);
        assert_eq!(&*p8.planes[1], &[32; 2]);
        assert_eq!(&*p16.planes[2], &[48; 2]);
        assert_eq!(p16.frame().bit_depth(), 8);
    }
}

#[test]
fn packed_storage_is_borrowed_and_trailing_samples_are_excluded() {
    let f = VideoFrame::new(2, 1, ColorFamily::Gray, Subsampling::S444, vec![Plane::packed(2, 1, 8, Samples::U8(vec![17, 18, 99])).unwrap()], None)
        .unwrap();
    let p = Planar8::try_from(&f).unwrap();
    assert!(matches!(p.planes[0], Cow::Borrowed(_)));
    assert_eq!(&*p.planes[0], &[17, 18]);
}

#[test]
fn invalid_depth_values_and_fields_are_rejected() {
    let mut f = picture(ColorFamily::YCbCr, Subsampling::S420, 10, true, false);
    assert!(Planar8::try_from(&f).is_err());
    f.planes[1].bit_depth = 9;
    assert!(Planar16::try_from(&f).is_err());
    f.planes[1].bit_depth = 10;
    f.field_order = FieldOrder::TopFieldFirst;
    assert!(Planar16::try_from(&f).is_err());
    f.field_order = FieldOrder::Progressive;
    if let Samples::U16(v) = &mut f.planes[0].samples {
        v[0] = 1024;
    }
    assert!(Planar16::try_from(&f).is_err());
}

#[test]
fn rgba_interleaves_rgb_and_gray_and_preserves_alpha() {
    let f = picture(ColorFamily::Rgb, Subsampling::S444, 8, true, true);
    let rgba = Rgba8::try_from(&f).unwrap();
    assert_eq!(rgba.pixels, [16, 32, 48, 64].repeat(8));
    let f = picture(ColorFamily::Gray, Subsampling::S444, 8, false, false);
    assert_eq!(Rgba8::try_from(&f).unwrap().pixels, [16, 16, 16, 255].repeat(8));
    assert!(Rgba8::try_from(&picture(ColorFamily::YCbCr, Subsampling::S420, 8, false, false)).is_err());
}

#[cfg(feature = "h264-encode")]
#[test]
fn h264_views_reject_alpha_and_wrong_depth() {
    let f = picture(ColorFamily::YCbCr, Subsampling::S420, 8, false, false);
    let p = Planar8::try_from(&f).unwrap();
    assert_eq!(p.h264().unwrap().u, &[32; 2]);
    assert!(Planar16::try_from(&f).unwrap().h264().is_err());
    let f = picture(ColorFamily::YCbCr, Subsampling::S420, 10, true, false);
    assert_eq!(Planar16::try_from(&f).unwrap().h264().unwrap().y, &[16; 8]);
    let f = picture(ColorFamily::YCbCr, Subsampling::S420, 8, false, true);
    assert!(Planar8::try_from(&f).unwrap().h264().is_err());
}

#[cfg(feature = "prores")]
#[test]
fn prores_preserves_colour_aspect_and_alpha() {
    let mut f = picture(ColorFamily::YCbCr, Subsampling::S444, 10, true, true);
    f.sar = Some((2, 3)); // 4x2 at SAR 2:3 has display aspect 4:3.
    let native = crate::prores::Frame::try_from(&f).unwrap();
    assert_eq!(native.aspect_ratio, 2);
    assert_eq!(native.color.primaries, 0);
    assert_eq!(native.alpha, Some(vec![64; 8]));
    assert_eq!(VideoFrame::try_from(native).unwrap().sar, f.sar);
    f.sar = Some((7, 11));
    assert!(crate::prores::Frame::try_from(&f).is_err());
}

#[cfg(feature = "dnx")]
#[test]
fn dnx_reorders_rgb_to_gbr_and_reduces_aspect() {
    let mut f = picture(ColorFamily::Rgb, Subsampling::S444, 10, true, false);
    f.sar = Some((100000, 100000));
    let native = crate::dnx::Frame::try_from(&f).unwrap();
    assert!(native.rgb);
    assert_eq!(native.par, (1, 1));
    assert_eq!(native.y, vec![32; 8]);
    assert_eq!(native.cb, vec![48; 8]);
    assert_eq!(native.cr, vec![16; 8]);
}

#[cfg(feature = "apv-encode")]
#[test]
fn apv_maps_gray_and_alpha_chroma() {
    let f = picture(ColorFamily::Gray, Subsampling::S444, 10, true, false);
    let native = crate::apv::Frame::try_from(&f).unwrap();
    assert_eq!(native.chroma, crate::apv::ChromaFormat::Monochrome);
    assert!(native.cb.is_empty());
    let f = picture(ColorFamily::YCbCr, Subsampling::S444, 10, true, true);
    assert_eq!(crate::apv::Frame::try_from(&f).unwrap().chroma, crate::apv::ChromaFormat::Yuv4444);
}

#[cfg(feature = "ffv1")]
#[test]
fn ffv1_input_round_trips_samples_through_encoder() {
    let f = picture(ColorFamily::Rgb, Subsampling::S444, 10, true, true);
    let native = crate::ffv1::Frame::try_from(&f).unwrap();
    let enc = crate::ffv1::Encoder::new(&native, Default::default()).unwrap();
    let dec = crate::ffv1::Decoder::new(&enc.config_record(), f.width, f.height).unwrap();
    assert_eq!(dec.decode(&enc.encode(&native).unwrap()).unwrap(), native);
}

#[cfg(feature = "mjpeg")]
#[test]
fn mjpeg_interleaves_and_refuses_alpha() {
    let f = picture(ColorFamily::Rgb, Subsampling::S444, 8, false, false);
    let native = crate::mjpeg::Frame::try_from(&f).unwrap();
    assert_eq!(native.samples, crate::mjpeg::Samples::U8([16, 32, 48].repeat(8)));
    assert!(crate::mjpeg::Frame::try_from(&picture(ColorFamily::Rgb, Subsampling::S444, 8, false, true)).is_err());
}

#[cfg(any(
    feature = "h264-encode",
    feature = "hevc-encode",
    feature = "av1-encode",
    feature = "mpeg2-encode",
    feature = "mpeg4-encode",
    feature = "vp9-encode"
))]
#[test]
fn planar_views_are_accepted_by_the_encoders() {
    let planes = vec![
        Plane::packed(32, 32, 8, Samples::U8(vec![64; 1024])).unwrap(),
        Plane::packed(16, 16, 8, Samples::U8(vec![128; 256])).unwrap(),
        Plane::packed(16, 16, 8, Samples::U8(vec![128; 256])).unwrap(),
    ];
    let f = VideoFrame::new(32, 32, ColorFamily::YCbCr, Subsampling::S420, planes, None).unwrap();
    #[cfg(any(feature = "h264-encode", feature = "mpeg2-encode", feature = "mpeg4-encode", feature = "vp9-encode"))]
    let p = Planar8::try_from(&f).unwrap();
    #[cfg(any(feature = "hevc-encode", feature = "av1-encode"))]
    let deep = Planar16::try_from(&f).unwrap();
    #[cfg(feature = "h264-encode")]
    {
        let mut enc = crate::h264::Encoder::new(crate::h264::EncoderConfig::new(32, 32, 30, 1)).unwrap();
        let mut packets = enc.encode(&p.h264().unwrap(), 7).unwrap();
        packets.extend(enc.flush());
        assert!(!packets.is_empty());
    }
    #[cfg(feature = "hevc-encode")]
    {
        let mut enc = crate::hevc::Encoder::new(crate::hevc::EncoderConfig::new(32, 32, 30, 1)).unwrap();
        assert!(!enc.encode(&deep.hevc().unwrap()).unwrap().data.is_empty());
    }
    #[cfg(feature = "av1-encode")]
    {
        let mut enc = crate::av1::Encoder::new(crate::av1::EncoderConfig::new(32, 32, 30, 1)).unwrap();
        assert!(!enc.encode(&deep.av1().unwrap()).unwrap().data.is_empty());
    }
    #[cfg(feature = "mpeg2-encode")]
    {
        let mut enc = crate::mpeg2::Encoder::new(crate::mpeg2::EncoderConfig::new(32, 32, 30, 1)).unwrap();
        let mut packets = enc.encode(&p.mpeg2().unwrap(), 7).unwrap();
        packets.extend(enc.flush());
        assert!(!packets.is_empty());
    }
    #[cfg(feature = "mpeg4-encode")]
    {
        let mut enc = crate::mpeg4::Encoder::new(crate::mpeg4::EncoderConfig::new(32, 32, 30, 1)).unwrap();
        assert!(!enc.encode(&p.mpeg4().unwrap()).unwrap().data.is_empty());
    }
    #[cfg(feature = "vp9-encode")]
    {
        let mut enc = crate::vp9::Vp9Encoder::new(crate::vp9::EncoderConfig { width: 32, height: 32, ..Default::default() });
        let (y, u, v) = p.vp9().unwrap();
        assert!(!enc.encode_frame(y, u, v, true).is_empty());
    }
}

#[cfg(any(feature = "vp8-encode", feature = "hap", feature = "mjpeg"))]
#[test]
fn interleaved_inputs_are_accepted_by_the_encoders() {
    let f = picture(ColorFamily::Rgb, Subsampling::S444, 8, false, false);
    #[cfg(any(feature = "vp8-encode", feature = "hap"))]
    let rgba = Rgba8::try_from(&f).unwrap();
    #[cfg(feature = "vp8-encode")]
    assert!(!crate::vp8::encode_rgba(&rgba.pixels, rgba.width, rgba.height, &Default::default()).unwrap().data.is_empty());
    #[cfg(feature = "hap")]
    assert!(!crate::hap::Encoder::new(rgba.width, rgba.height, crate::hap::Variant::Hap).unwrap().encode_rgba(&rgba.pixels).unwrap().is_empty());
    #[cfg(feature = "mjpeg")]
    assert!(!crate::mjpeg::Encoder::default().encode_frame(&crate::mjpeg::Frame::try_from(&f).unwrap()).unwrap().is_empty());
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
#[test]
fn owned_frames_are_accepted_by_the_encoders() {
    let f = VideoFrame::new(
        64,
        64,
        ColorFamily::YCbCr,
        Subsampling::S422,
        vec![
            Plane::packed(64, 64, 10, Samples::U16(vec![256; 4096])).unwrap(),
            Plane::packed(32, 64, 10, Samples::U16(vec![512; 2048])).unwrap(),
            Plane::packed(32, 64, 10, Samples::U16(vec![512; 2048])).unwrap(),
        ],
        None,
    )
    .unwrap();
    #[cfg(feature = "prores")]
    assert!(
        !crate::prores::Encoder::new(crate::prores::Profile::Proxy, 64, 64).encode(&crate::prores::Frame::try_from(&f).unwrap()).unwrap().is_empty()
    );
    #[cfg(feature = "dnx")]
    assert!(
        !crate::dnx::Encoder::new(crate::dnx::Profile::Hqx, 64, 64).unwrap().encode(&crate::dnx::Frame::try_from(&f).unwrap()).unwrap().is_empty()
    );
    #[cfg(feature = "apv-encode")]
    assert!(
        !crate::apv::Encoder::new(crate::apv::Profile::P422_10, 64, 64)
            .unwrap()
            .encode(&crate::apv::Frame::try_from(&f).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn zero_sizes_and_oversized_allocations_are_errors() {
    let f = VideoFrame::new(0, 0, ColorFamily::Gray, Subsampling::S444, vec![Plane::packed(0, 0, 8, Samples::U8(vec![])).unwrap()], None).unwrap();
    assert!(Planar8::try_from(&f).is_err());
    assert!(reserve::<u16>(usize::MAX).is_err());
    assert!(reserve::<u8>(crate::budget::DECODE_MEMORY_BUDGET as usize + 1).is_err());
}

#[cfg(any(
    feature = "h264-encode",
    feature = "hevc-encode",
    feature = "av1-encode",
    feature = "mpeg2-encode",
    feature = "mpeg4-encode",
    feature = "vp9-encode"
))]
#[test]
fn views_reject_incompatible_chroma_and_depth() {
    let f = picture(ColorFamily::YCbCr, Subsampling::S422, 8, false, false);
    #[cfg(any(feature = "h264-encode", feature = "mpeg2-encode", feature = "mpeg4-encode", feature = "vp9-encode"))]
    let p = Planar8::try_from(&f).unwrap();
    #[cfg(feature = "h264-encode")]
    assert!(p.h264().is_err());
    #[cfg(feature = "mpeg4-encode")]
    assert!(p.mpeg4().is_err());
    #[cfg(feature = "vp9-encode")]
    assert!(p.vp9().is_err());
    #[cfg(feature = "mpeg2-encode")]
    assert_eq!(p.mpeg2().unwrap().cb.len(), 4);
    #[cfg(any(feature = "hevc-encode", feature = "av1-encode"))]
    {
        let p = Planar16::try_from(&f).unwrap();
        #[cfg(feature = "hevc-encode")]
        assert!(p.hevc().is_err());
        #[cfg(feature = "av1-encode")]
        assert!(p.av1().is_err());
        let f = picture(ColorFamily::YCbCr, Subsampling::S420, 12, true, false);
        let p = Planar16::try_from(&f).unwrap();
        #[cfg(feature = "hevc-encode")]
        assert!(p.hevc().is_err());
        #[cfg(feature = "av1-encode")]
        assert!(p.av1().is_err());
    }
}

#[test]
fn unequal_chroma_strides_and_high_depth_samples_are_preserved() {
    let mut f = picture(ColorFamily::YCbCr, Subsampling::S422, 10, true, false);
    f.planes[2] = Plane::new(2, 2, 7, 10, Samples::U16(vec![1023, 900, 65535, 65535, 65535, 65535, 65535, 800, 700])).unwrap();
    let p = Planar16::try_from(&f).unwrap();
    assert_eq!(&*p.planes[2], &[1023, 900, 800, 700]);
    assert_eq!(&*p.planes[1], &[32; 4]);
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
#[test]
fn owned_inputs_reject_shapes_that_would_drop_information() {
    #[cfg(feature = "prores")]
    {
        let mut f = picture(ColorFamily::YCbCr, Subsampling::S422, 10, true, false);
        f.color.full_range = true;
        assert!(crate::prores::Frame::try_from(&f).is_err());
        let f = picture(ColorFamily::YCbCr, Subsampling::S422, 10, true, true);
        assert!(crate::prores::Frame::try_from(&f).is_err());
    }
    #[cfg(feature = "dnx")]
    {
        let f = picture(ColorFamily::Rgb, Subsampling::S444, 10, true, true);
        assert!(crate::dnx::Frame::try_from(&f).is_err());
    }
    #[cfg(feature = "apv-encode")]
    {
        let f = picture(ColorFamily::Rgb, Subsampling::S444, 10, true, false);
        assert!(crate::apv::Frame::try_from(&f).is_err());
        let f = picture(ColorFamily::Gray, Subsampling::S444, 10, true, true);
        assert!(crate::apv::Frame::try_from(&f).is_err());
    }
}
