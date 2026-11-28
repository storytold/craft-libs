//! TIFF with JPEG-compressed strips (compression 7, TIFF Technical Note #2): each strip a whole
//! JPEG, RGB stored as YCbCr, 8-bit gray and RGB only.

use craft_image_codecs::*;
use std::io::Cursor;

fn jpeg(subsampled: bool) -> EncodeOptions {
    EncodeOptions { tiff_compression: TiffCompression::Jpeg, jpeg_quality: 90, jpeg_chroma_subsampling: subsampled, ..Default::default() }
}

/// A smooth 8-bit image: gradients, which JPEG keeps close at quality 90.
fn gradient(w: u32, h: u32, layout: ChannelLayout) -> Image {
    let ch = layout.channels();
    let mut data = Vec::with_capacity(w as usize * h as usize * ch);
    for y in 0..h {
        for x in 0..w {
            let px = [(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, ((x + y) * 255 / (w + h)) as u8];
            data.extend_from_slice(&px[..ch]);
        }
    }
    Image::from_raw(w, h, layout, SampleType::U8, data).unwrap()
}

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.len() as f64
}

/// Each strip's bytes, read with the `tiff` crate's tag reader rather than ours.
fn strips(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut d = tiff::decoder::Decoder::new(Cursor::new(bytes)).unwrap();
    let offsets = d.get_tag_u64_vec(tiff::tags::Tag::StripOffsets).unwrap();
    let counts = d.get_tag_u64_vec(tiff::tags::Tag::StripByteCounts).unwrap();
    offsets.iter().zip(&counts).map(|(&at, &n)| bytes[at as usize..(at + n) as usize].to_vec()).collect()
}

/// Odd width, a height that is no multiple of 16 and several strips: the file opens to the
/// same size and layout, close to the source, and smaller than without compression.
#[test]
fn gray_and_rgb_round_trip_close_to_the_source() {
    for layout in [ChannelLayout::Rgb, ChannelLayout::Gray] {
        for subsampled in [true, false] {
            let img = gradient(701, 2003, layout);
            let bytes = encode(&img, Format::Tiff, &jpeg(subsampled)).unwrap();
            let label = format!("{layout:?} subsampled={subsampled}");
            let page = &tiff_info(&bytes).unwrap().pages[0];
            assert_eq!(page.compression, 7, "{label}");
            assert!(strips(&bytes).len() > 1, "{label}: one strip");
            let back = decode(&bytes).unwrap();
            assert_eq!((back.dimensions(), back.layout(), back.sample_type()), (img.dimensions(), layout, SampleType::U8), "{label}");
            let err = mean_abs_diff(back.data(), img.data());
            assert!(err < 2.0, "{label}: mean error {err}");
            let plain = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
            assert!(bytes.len() < plain.len() / 4, "{label}: {} vs {}", bytes.len(), plain.len());
        }
    }
}

/// Each strip is a complete JPEG on its own (no shared tables), as a JPEG reader that knows
/// nothing of TIFF opens it, and the strips stack up to the image.
#[test]
fn every_strip_is_a_standalone_jpeg() {
    let img = gradient(333, 1000, ChannelLayout::Rgb);
    let bytes = encode(&img, Format::Tiff, &jpeg(true)).unwrap();
    let mut rows = 0;
    for strip in strips(&bytes) {
        let j = image::load_from_memory_with_format(&strip, image::ImageFormat::Jpeg).unwrap();
        assert_eq!(j.width(), 333);
        rows += j.height();
    }
    assert_eq!(rows, 1000);
}

/// The colour is right, not only the shape: a saturated colour opens as that colour (RGB goes
/// through YCbCr and back, so swapped or unconverted channels would show here).
#[test]
fn colour_survives_the_ycbcr_round_trip() {
    for subsampled in [true, false] {
        let px = [200u8, 40, 90];
        let img = Image::from_raw(48, 40, ChannelLayout::Rgb, SampleType::U8, px.repeat(48 * 40)).unwrap();
        let back = decode(&encode(&img, Format::Tiff, &jpeg(subsampled)).unwrap()).unwrap();
        for (got, want) in back.data()[..3].iter().zip(px) {
            assert!(got.abs_diff(want) <= 3, "subsampled={subsampled}: {:?}", &back.data()[..3]);
        }
    }
}

/// Metadata goes in the TIFF tags as with the other compressions, and the pyramid levels are
/// JPEG too; BigTIFF holds the same strips.
#[test]
fn metadata_pyramid_and_bigtiff_follow() {
    let mut img = gradient(300, 200, ChannelLayout::Rgb);
    img.meta.dpi = Some((300.0, 300.0));
    let opts = EncodeOptions { tiff_pyramid: true, ..jpeg(true) };
    let bytes = encode(&img, Format::Tiff, &opts).unwrap();
    let info = tiff_info(&bytes).unwrap();
    assert!(info.pages.len() > 1 && info.pages.iter().all(|p| p.compression == 7), "{:?}", info.pages);
    assert_eq!(decode(&bytes).unwrap().meta.dpi, Some((300.0, 300.0)));
    let big = encode(&img, Format::Tiff, &EncodeOptions { tiff_bigtiff: true, ..jpeg(true) }).unwrap();
    assert!(tiff_info(&big).unwrap().big_tiff);
    assert!(mean_abs_diff(decode(&big).unwrap().data(), img.data()) < 2.0);
}

/// Images JPEG strips can't hold are refused, never converted or written another way.
#[test]
fn refuses_what_jpeg_cannot_store() {
    let rgba = Image::from_raw(4, 4, ChannelLayout::Rgba, SampleType::U8, vec![9; 64]).unwrap();
    let rgb16 = Image::from_raw(4, 4, ChannelLayout::Rgb, SampleType::U16, vec![9; 96]).unwrap();
    let cmyk = Image::from_raw(4, 4, ChannelLayout::Cmyk, SampleType::U8, vec![9; 64]).unwrap();
    let lab = Image::from_raw(4, 4, ChannelLayout::Lab, SampleType::U8, vec![9; 48]).unwrap();
    let gray_f32 = Image::from_raw(4, 4, ChannelLayout::Gray, SampleType::F32, vec![0; 64]).unwrap();
    let mut spot = gradient(4, 4, ChannelLayout::Gray);
    spot.meta.extra_samples.push(ExtraSample { kind: 0, data: vec![1; 16] });
    let wide = Image::from_raw(70_000, 1, ChannelLayout::Gray, SampleType::U8, vec![0; 70_000]).unwrap();
    let images = [("rgba", rgba), ("rgb16", rgb16), ("cmyk", cmyk), ("lab", lab), ("gray f32", gray_f32), ("spot", spot), ("wide", wide)];
    for (label, img) in images {
        let err = encode(&img, Format::Tiff, &jpeg(true)).unwrap_err();
        assert!(err.to_string().contains("JPEG"), "{label}: {err}");
    }
}

/// A save with JPEG strips is reported as lossy before it happens, as lossy WebP is.
#[test]
fn fidelity_reports_jpeg_strips_as_lossy() {
    let img = gradient(8, 8, ChannelLayout::Rgb);
    let lossy = |opts: &EncodeOptions| fidelity_warnings_with(&img, Format::Tiff, opts).contains(&FidelityWarning::LossyCompression);
    assert!(lossy(&jpeg(true)));
    assert!(!lossy(&EncodeOptions { tiff_compression: TiffCompression::Lzw, ..Default::default() }));
}
