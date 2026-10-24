//! Page length and delivered resolution across their whole input space, through the public API: the unit
//! conversions, the resolution each container carries at the resolutions scanners use, the page builder with and
//! without inference, and what a cut page still says about its resolution.

use craft_scan::bw::{LengthPlan, PageRule, page_for};
use craft_scan::density::image_density;
use craft_scan::length::crop_bottom_bytes;
use craft_scan::{InferOptions, LengthRecord, LengthSource, mils_of, rows_for_mils};

/// Resolutions scanners and drivers deliver, including the odd ones (a fake capping 300 to 150, fax modes).
const DPIS: [u32; 14] = [72, 75, 96, 100, 120, 150, 200, 240, 300, 400, 600, 1200, 2400, 4800];

// ---- units ---------------------------------------------------------------------------------------

#[test]
fn rows_and_mils_convert_back_within_half_a_row() {
    for dpi in DPIS {
        for rows in (1..=20 * dpi).step_by(7) {
            let mils = mils_of(rows, dpi).expect("rows and dpi are positive");
            let back = rows_for_mils(mils, dpi);
            // Half a thousandth of an inch, in rows (more than one row above 1000 dpi), plus the rounding of each step.
            assert!(back.abs_diff(rows) <= dpi / 2000 + 1, "{rows} rows at {dpi} dpi -> {mils} mils -> {back} rows");
        }
        // From one row up: a shorter length still takes a whole row (rows_for_mils never answers 0).
        for mils in (1000 / dpi + 1..=14_000).step_by(13) {
            let rows = rows_for_mils(mils, dpi);
            let back = mils_of(rows, dpi).expect("at least one row");
            // Half a row in thousandths of an inch, plus the rounding of each step.
            assert!(back.abs_diff(mils) <= 1000 / dpi / 2 + 1, "{mils} mils at {dpi} dpi -> {rows} rows -> {back} mils");
        }
    }
}

#[test]
fn conversions_never_panic_or_invent_a_length() {
    assert_eq!(mils_of(0, 200), None);
    assert_eq!(mils_of(200, 0), None);
    assert!(mils_of(u32::MAX, 1).is_none() || mils_of(u32::MAX, 1).is_some(), "no overflow panic");
    assert_eq!(rows_for_mils(0, 200), 1, "at least one row");
    let _ = rows_for_mils(u32::MAX, u32::MAX);
    // A legal sheet at 14 in, the longest feeder page measured.
    assert_eq!(mils_of(2800, 200), Some(14_000));
    assert_eq!(rows_for_mils(14_000, 600), 8400);
}

#[test]
fn only_an_inferred_length_is_approximate() {
    for source in LengthSource::ALL.iter().copied() {
        let record = LengthRecord::with_source(2200, 200, source);
        assert_eq!(record.approximate, source == LengthSource::Inferred, "{source:?}");
        assert_eq!((record.source, record.length_mils), (source, Some(11_000)), "{source:?}");
    }
    let unknown = LengthRecord::from_delivered(2200, 0);
    assert_eq!((unknown.source, unknown.length_mils, unknown.approximate), (LengthSource::Driver, None, false));
    let record = LengthRecord::from_delivered(2200, 200);
    assert!((record.length_inches().unwrap_or(0.0) - 11.0).abs() < 1e-9);
    assert!((record.length_mm().unwrap_or(0.0) - 279.4).abs() < 1e-9);
}

// ---- containers ----------------------------------------------------------------------------------

/// A grey image whose pixel at (x, y) is `value(x, y)`.
fn gray(width: u32, height: u32, value: impl Fn(u32, u32) -> u8) -> image::GrayImage {
    image::GrayImage::from_fn(width, height, |x, y| image::Luma([value(x, y)]))
}

/// A 24-bit BMP with the density a Windows driver writes: pels per metre, rounded.
fn bmp_at(width: u32, height: u32, dpi_x: u32, dpi_y: u32, value: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let stride = (width * 3).div_ceil(4) * 4;
    let size = 54 + stride * height;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(stride * height).to_le_bytes());
    let ppm = |dpi: u32| (f64::from(dpi) / 0.0254).round() as i32;
    out.extend_from_slice(&ppm(dpi_x).to_le_bytes());
    out.extend_from_slice(&ppm(dpi_y).to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    for row in (0..height).rev() {
        for x in 0..width {
            let v = value(x, row);
            out.extend_from_slice(&[v, v, v]);
        }
        out.resize(out.len() + (stride - width * 3) as usize, 0);
    }
    out
}

/// `image` as a JPEG at quality 95 with the given density in dots per inch.
fn jpeg_at(image: &image::GrayImage, dpi_x: u16, dpi_y: u16) -> Vec<u8> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
    encoder.set_pixel_density(image::codecs::jpeg::PixelDensity { density: (dpi_x, dpi_y), unit: image::codecs::jpeg::PixelDensityUnit::Inches });
    encoder.write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::L8).expect("jpeg");
    out
}

/// CRC-32 (IEEE) of `bytes`, as PNG chunks use.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A PNG with a `pHYs` chunk (pixels per metre), inserted after `IHDR`.
fn png_at(image: &image::GrayImage, dpi_x: u32, dpi_y: u32) -> Vec<u8> {
    let mut plain = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut plain), image::ImageFormat::Png).expect("png");
    let ppm = |dpi: u32| (f64::from(dpi) / 0.0254).round() as u32;
    let mut body = b"pHYs".to_vec();
    body.extend_from_slice(&ppm(dpi_x).to_be_bytes());
    body.extend_from_slice(&ppm(dpi_y).to_be_bytes());
    body.push(1);
    let mut chunk = 9u32.to_be_bytes().to_vec();
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&body).to_be_bytes());
    let after_ihdr = 8 + 4 + 4 + 13 + 4;
    let mut out = plain[..after_ihdr].to_vec();
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&plain[after_ihdr..]);
    out
}

/// A TIFF with `XResolution`/`YResolution` in inches.
fn tiff_at(image: &image::GrayImage, dpi_x: u32, dpi_y: u32) -> Vec<u8> {
    use tiff::encoder::{Rational, TiffEncoder, colortype};
    use tiff::tags::{ResolutionUnit, Tag};
    let mut out = std::io::Cursor::new(Vec::new());
    let mut encoder = TiffEncoder::new(&mut out).expect("tiff");
    let mut page = encoder.new_image::<colortype::Gray8>(image.width(), image.height()).expect("tiff page");
    page.encoder().write_tag(Tag::ResolutionUnit, ResolutionUnit::Inch.to_u16()).expect("unit");
    page.encoder().write_tag(Tag::XResolution, Rational { n: dpi_x, d: 1 }).expect("x");
    page.encoder().write_tag(Tag::YResolution, Rational { n: dpi_y, d: 1 }).expect("y");
    page.write_data(image.as_raw()).expect("tiff data");
    out.into_inner()
}

/// A rule at `dpi` that leaves the page as delivered.
fn plain_rule(dpi: u32) -> PageRule {
    PageRule { dpi, dpi_y: None, bw_threshold: None, length: LengthPlan::default() }
}

#[test]
fn every_container_delivers_the_resolution_it_says_at_every_scanner_resolution() {
    let picture = gray(16, 8, |x, y| ((x * 13 + y * 7) % 200) as u8 + 30);
    for dpi in DPIS {
        let files = [
            ("bmp", bmp_at(16, 8, dpi, dpi, |x, y| ((x * 13 + y * 7) % 200) as u8 + 30)),
            ("png", png_at(&picture, dpi, dpi)),
            ("jpeg", jpeg_at(&picture, dpi as u16, dpi as u16)),
            ("tiff", tiff_at(&picture, dpi, dpi)),
        ];
        for (format, bytes) in files {
            // Asked for something else: the page says what was delivered, rounded to the dpi it was written as.
            let asked = if dpi == 300 { 600 } else { 300 };
            let (page, _) = page_for(&bytes, plain_rule(asked));
            assert_eq!((page.dpi, page.dpi_y, page.requested_dpi), (dpi, dpi, asked), "{format} at {dpi} dpi");
            assert_eq!(page.length.length_mils, mils_of(page.height, dpi), "{format} at {dpi} dpi");
        }
    }
}

#[test]
fn a_fax_resolution_keeps_its_two_axes_through_every_container() {
    let picture = gray(16, 8, |_, _| 128);
    for (x, y) in [(200u32, 100u32), (204, 196), (300, 150)] {
        for (format, bytes) in
            [("bmp", bmp_at(16, 8, x, y, |_, _| 128)), ("png", png_at(&picture, x, y)), ("jpeg", jpeg_at(&picture, x as u16, y as u16))]
        {
            let (page, _) = page_for(&bytes, plain_rule(300));
            assert_eq!((page.dpi, page.dpi_y), (x, y), "{format} {x}x{y}");
            // The length runs down the page, so it uses the vertical resolution.
            assert_eq!(page.length.length_mils, mils_of(page.height, y), "{format} {x}x{y}");
        }
    }
}

/// A request with two resolutions, answered by a file that records none, still has two resolutions on the page.
#[test]
fn a_non_square_request_keeps_its_vertical_resolution_when_the_file_carries_no_density() {
    use image::ImageEncoder;
    let picture = gray(16, 8, |x, y| ((x * 13 + y * 7) % 200) as u8 + 30);
    let mut no_density_jpeg = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut no_density_jpeg, 95);
    encoder.set_pixel_density(image::codecs::jpeg::PixelDensity { density: (1, 1), unit: image::codecs::jpeg::PixelDensityUnit::PixelAspectRatio });
    encoder.write_image(picture.as_raw(), 16, 8, image::ExtendedColorType::L8).expect("jpeg");
    let mut no_phys_png = Vec::new();
    picture.write_to(&mut std::io::Cursor::new(&mut no_phys_png), image::ImageFormat::Png).expect("png");
    assert_eq!(image_density(&no_density_jpeg), None);
    assert_eq!(image_density(&no_phys_png), None);

    let settings = craft_scan::ScanSettings { dpi: 200, dpi_y: Some(100), ..craft_scan::ScanSettings::default() };
    for (format, bytes) in [("jpeg", &no_density_jpeg), ("png", &no_phys_png)] {
        let (page, _) = page_for(bytes, PageRule::for_settings(&settings));
        assert_eq!((page.dpi, page.dpi_y, page.requested_dpi), (200, 100, 200), "{format}");
        // The length runs down the page, so it uses the vertical resolution.
        assert_eq!(page.length.length_mils, mils_of(page.height, 100), "{format}");

        // A thresholded page writes the same two resolutions into its BMP.
        let bw = craft_scan::ScanSettings { color: craft_scan::Color::BlackAndWhite, ..settings.clone() };
        let (page, _) = page_for(bytes, PageRule::for_settings(&bw));
        assert!(page.thresholded, "{format}");
        assert_eq!(image_density(&page.bytes).map(|(x, y)| (x.round(), y.round())), Some((200.0, 100.0)), "{format} bmp");
    }

    // A square request is unchanged.
    let square = craft_scan::ScanSettings { dpi: 200, ..craft_scan::ScanSettings::default() };
    let (page, _) = page_for(&no_density_jpeg, PageRule::for_settings(&square));
    assert_eq!((page.dpi, page.dpi_y), (200, 200));
}

// ---- inference through the page builder ----------------------------------------------------------

/// A sheet on a 14 in feeder scan: noisy paper to `sheet_rows`, pure white fill below, a thin fill frame on the
/// sides, some ink in the upper part. The shape the measured feeder delivers.
fn feeder_page(width: u32, height: u32, sheet_rows: u32) -> impl Fn(u32, u32) -> u8 {
    move |x, y| {
        if y >= sheet_rows || x < 3 || x + 3 >= width {
            return 255;
        }
        if (height / 10..height / 5).contains(&y) && x % 9 < 4 {
            return 40;
        }
        // Paper noise in the high 200s, deterministic.
        let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) % 40;
        (215 + n) as u8
    }
}

/// A rule at `dpi` that infers the sheet end with the default options.
fn infer_rule(dpi: u32) -> PageRule {
    PageRule { dpi, dpi_y: None, bw_threshold: None, length: LengthPlan { source: LengthSource::Driver, infer: Some(InferOptions::default()) } }
}

#[test]
fn an_inferred_page_says_its_length_and_keeps_its_resolution_in_every_lossless_container() {
    for dpi in [100u32, 200, 300] {
        let (width, height) = (dpi * 2, dpi * 14);
        for sheet_mils in [8_500u32, 10_750, 11_000, 13_000] {
            let sheet_rows = rows_for_mils(sheet_mils, dpi);
            let value = feeder_page(width, height, sheet_rows);
            let picture = gray(width, height, &value);
            for (format, bytes) in
                [("bmp", bmp_at(width, height, dpi, dpi, &value)), ("png", png_at(&picture, dpi, dpi)), ("tiff", tiff_at(&picture, dpi, dpi))]
            {
                let case = format!("{format} at {dpi} dpi, {sheet_mils} mils");
                let (page, _) = page_for(&bytes, infer_rule(dpi));
                assert_eq!(page.length.source, LengthSource::Inferred, "{case}");
                assert!(page.length.approximate, "{case}");
                assert_eq!(page.length.length_mils, mils_of(page.height, dpi), "{case}: the record is the page it describes");
                let got = page.length.length_mils.unwrap_or(0);
                assert!(got.abs_diff(sheet_mils) <= 10 + 1000 / dpi + 1, "{case}: {got}");
                assert_eq!(page.width, width, "{case}: the width is never cut");
                assert_eq!((page.dpi, page.dpi_y), (dpi, dpi), "{case}");
                // The cut file itself still says how big it is, so a PDF made from the folder later is the right size.
                assert_eq!(
                    image_density(&page.bytes).map(|(x, y)| (x.round() as u32, y.round() as u32)),
                    Some((dpi, dpi)),
                    "{case}: the cut file lost its resolution"
                );
                let decoded = image::load_from_memory(&page.bytes).expect("the cut page decodes");
                assert_eq!((decoded.width(), decoded.height()), (page.width, page.height), "{case}");
            }
        }
    }
}

#[test]
fn a_cut_jpeg_keeps_its_resolution() {
    let dpi = 200u32;
    let (width, height) = (dpi * 2, dpi * 14);
    let picture = gray(width, height, feeder_page(width, height, rows_for_mils(11_000, dpi)));
    let bytes = jpeg_at(&picture, dpi as u16, dpi as u16);
    let cut = crop_bottom_bytes(&bytes, 2210).expect("a JPEG can be cut");
    assert_eq!(image_density(&cut).map(|(x, y)| (x.round() as u32, y.round() as u32)), Some((dpi, dpi)));
}

#[test]
fn without_inference_the_page_is_never_cut_and_keeps_the_plans_source() {
    let dpi = 200u32;
    let (width, height) = (dpi * 2, dpi * 14);
    let value = feeder_page(width, height, rows_for_mils(11_000, dpi));
    let bytes = bmp_at(width, height, dpi, dpi, &value);
    for source in [LengthSource::Driver, LengthSource::Size, LengthSource::Device] {
        let rule = PageRule { dpi, dpi_y: None, bw_threshold: None, length: LengthPlan { source, infer: None } };
        let (page, _) = page_for(&bytes, rule);
        assert_eq!((page.height, page.length.source, page.length.approximate), (height, source, false), "{source:?}");
        assert_eq!(page.length.length_mils, Some(14_000), "{source:?}");
        assert_eq!(page.bytes, bytes, "{source:?}: the delivered bytes are kept");
    }
}

#[test]
fn a_refused_inference_keeps_the_page_and_the_plans_source() {
    // A noiseless white margin: the end of the printing cannot be told from the end of the sheet.
    let dpi = 200u32;
    let (width, height) = (dpi * 2, dpi * 14);
    let bytes = bmp_at(width, height, dpi, dpi, |x, y| if y < height / 3 && x % 5 == 0 { 30 } else { 255 });
    let (page, _) = page_for(&bytes, infer_rule(dpi));
    assert_eq!((page.height, page.length.source, page.length.approximate), (height, LengthSource::Driver, false));
    assert_eq!(page.bytes, bytes);
}
