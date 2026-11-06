use super::*;
use crate::bw;

/// Deterministic noise source.
struct Lcg(u64);

impl Lcg {
    /// The next 31-bit pseudo-random value.
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

/// Paper noise as the measured scanner shows it: about 40 % of paper pixels sit a little below 255.
fn noisy_white(rng: &mut Lcg) -> u8 {
    if rng.next() % 100 < 40 { 255 - 1 - (rng.next() % 12) as u8 } else { 255 }
}

#[derive(Clone, Copy)]
struct Sheet {
    width: usize,
    /// Rows of the image.
    height: usize,
    /// Rows of the sheet (paper); everything below is fill.
    sheet_rows: usize,
    /// Fill frame left, right and above (pixels), as the measured scanner leaves outside its default region.
    frame: usize,
    /// Rows `(first, last)` of printed bars.
    ink: Option<(usize, usize)>,
    noisy: bool,
}

/// The luminance rows of `sheet`: white fill, with the paper area (optionally noisy) and the ink bars drawn in.
fn luma(sheet: Sheet) -> Vec<u8> {
    let mut rng = Lcg(7);
    let mut pixels = vec![255u8; sheet.width * sheet.height];
    for row in sheet.frame..sheet.sheet_rows.min(sheet.height) {
        for col in sheet.frame..sheet.width.saturating_sub(sheet.frame) {
            let mut v = if sheet.noisy { noisy_white(&mut rng) } else { 255 };
            if let Some((a, b)) = sheet.ink
                && (a..=b).contains(&row)
                && col % 4 < 2
                && col > sheet.width / 5
                && col < sheet.width * 4 / 5
            {
                v = 20;
            }
            pixels[row * sheet.width + col] = v;
        }
    }
    pixels
}

/// An 8-bit grey BMP (identity palette) of `pixels`, with `dpi` in both resolution fields, stored top-down or bottom-up.
fn bmp_gray(width: usize, height: usize, pixels: &[u8], dpi: u32, top_down: bool) -> Vec<u8> {
    let stride = (width * 8).div_ceil(32) * 4;
    let data_at = 14 + 40 + 1024;
    let mut out = vec![0u8; data_at + stride * height];
    out[..2].copy_from_slice(b"BM");
    let total = out.len() as u32;
    out[2..6].copy_from_slice(&total.to_le_bytes());
    out[10..14].copy_from_slice(&(data_at as u32).to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    let h = if top_down { -(height as i32) } else { height as i32 };
    out[22..26].copy_from_slice(&h.to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&8u16.to_le_bytes());
    out[34..38].copy_from_slice(&((stride * height) as u32).to_le_bytes());
    let pels = (f64::from(dpi) / 0.0254).round() as u32;
    out[38..42].copy_from_slice(&pels.to_le_bytes());
    out[42..46].copy_from_slice(&pels.to_le_bytes());
    out[46..50].copy_from_slice(&256u32.to_le_bytes());
    for i in 0..256usize {
        out[54 + i * 4..54 + i * 4 + 3].fill(i as u8);
    }
    for row in 0..height {
        let file_row = if top_down { row } else { height - 1 - row };
        let at = data_at + file_row * stride;
        out[at..at + width].copy_from_slice(&pixels[row * width..(row + 1) * width]);
    }
    out
}

/// A `Page` around `bytes` that was delivered at `dpi`, with the driver's length record.
fn page_of(bytes: Vec<u8>, width: usize, height: usize, dpi: u32) -> Page {
    Page {
        bytes,
        format: "bmp",
        width: width as u32,
        height: height as u32,
        dpi,
        dpi_y: dpi,
        requested_dpi: dpi,
        thresholded: false,
        length: LengthRecord::from_delivered(height as u32, dpi),
    }
}

#[test]
fn synthetic_padded_pages_are_cut_at_the_sheet_end_at_several_sizes_and_resolutions() {
    for dpi in [100u32, 200, 300] {
        for sheet_mils in [5500u32, 8500, 10_730, 11_000, 11_693, 13_000] {
            let width = (8.27 * f64::from(dpi)) as usize;
            let sheet_rows = rows_for_mils(sheet_mils, dpi) as usize;
            let height = 14 * dpi as usize;
            let sheet = Sheet { width, height, sheet_rows, frame: 4, ink: Some((height / 8, height / 4)), noisy: true };
            let pixels = luma(sheet);
            let found =
                assess_luma(width, height, &pixels, dpi, &InferOptions::default()).unwrap_or_else(|r| panic!("{dpi} dpi {sheet_mils}: {r:?}"));
            assert_eq!(found.sheet_end_row as usize, sheet_rows, "{dpi} dpi, {sheet_mils} mils");
            assert_eq!(found.rows, sheet_rows as u32 + rows_for_mils(10, dpi));
            // Within the margin plus a pixel of the real length.
            let (want, got) = (i64::from(sheet_mils), i64::from(found.length_mils));
            assert!((got - want).abs() <= 10 + 1000 / i64::from(dpi) + 1, "{got} vs {want} at {dpi} dpi");
            assert!(found.noise_permille >= 300, "{}", found.noise_permille);
        }
    }
}

#[test]
fn the_last_ink_row_is_not_mistaken_for_the_sheet_end_on_a_noisy_scan() {
    // Printing ends at 6 in, the noisy paper continues to 10.5 in, fill below.
    let (dpi, width) = (200u32, 1654usize);
    let sheet = Sheet { width, height: 2800, sheet_rows: 2100, frame: 5, ink: Some((100, 1200)), noisy: true };
    let found = assess_luma(width, 2800, &luma(sheet), dpi, &InferOptions::default()).expect("noisy paper below the ink");
    assert_eq!(found.sheet_end_row, 2100, "cut at the sheet end, not at the last ink row 1200");
}

#[test]
fn a_noiseless_white_margin_is_refused_so_the_end_of_printing_is_not_taken_for_the_sheet_end() {
    // The same page from a scanner whose paper is exactly 255: ink end and sheet end cannot be told apart.
    let width = 1654usize;
    let sheet = Sheet { width, height: 2800, sheet_rows: 2100, frame: 5, ink: Some((100, 1200)), noisy: false };
    assert_eq!(assess_luma(width, 2800, &luma(sheet), 200, &InferOptions::default()), Err(Refusal::NoPaperNoise));
}

#[test]
fn a_page_with_content_to_the_bottom_edge_is_not_cropped() {
    let width = 800usize;
    let sheet = Sheet { width, height: 1200, sheet_rows: 1200, frame: 0, ink: None, noisy: true };
    assert_eq!(assess_luma(width, 1200, &luma(sheet), 200, &InferOptions::default()), Err(Refusal::NoPadding));
}

#[test]
fn a_page_without_padding_is_left_alone_by_infer_and_crop() {
    let (width, height) = (400usize, 1200usize);
    let pixels = luma(Sheet { width, height, sheet_rows: height, frame: 0, ink: None, noisy: true });
    let bytes = bmp_gray(width, height, &pixels, 200, false);
    let mut page = page_of(bytes.clone(), width, height, 200);
    assert_eq!(infer_and_crop(&mut page, &InferOptions::default()), Inference::Kept(Refusal::NoPadding));
    assert_eq!(page.bytes, bytes);
    assert_eq!(page.length.source, LengthSource::Driver);
}

#[test]
fn an_all_white_page_and_garbage_are_refused() {
    let flat = vec![255u8; 100 * 100];
    assert_eq!(assess_luma(100, 100, &flat, 100, &InferOptions::default()), Err(Refusal::AllFill));
    assert_eq!(assess_sheet_end(b"not an image", 200, &InferOptions::default()), Err(Refusal::Unreadable));
    assert_eq!(assess_luma(10, 10, &[0; 5], 200, &InferOptions::default()), Err(Refusal::Unreadable));
    assert_eq!(assess_luma(10, 10, &[0; 100], 0, &InferOptions::default()), Err(Refusal::Unreadable));
}

#[test]
fn a_candidate_shorter_than_the_minimum_is_refused() {
    let (width, height) = (400usize, 1000usize);
    let pixels = luma(Sheet { width, height, sheet_rows: 100, frame: 0, ink: None, noisy: true });
    assert_eq!(assess_luma(width, height, &pixels, 200, &InferOptions::default()), Err(Refusal::TooShort));
}

#[test]
fn exact_fill_means_exactly_255_for_lossless_pages() {
    // A fill of 254 is paper, not fill: the page is "content to the bottom".
    let (width, height) = (300usize, 1000usize);
    let mut pixels = luma(Sheet { width, height, sheet_rows: 600, frame: 0, ink: None, noisy: true });
    for v in &mut pixels[600 * width..] {
        *v = 254;
    }
    assert_eq!(assess_luma(width, height, &pixels, 200, &InferOptions::default()), Err(Refusal::NoPadding));
    // With a tolerance of 1 the 254 rows count as fill again.
    let loose = InferOptions { tolerance: 1, ..InferOptions::default() };
    assert!(assess_luma(width, height, &pixels, 200, &loose).is_ok());
}

#[test]
fn a_stray_dark_pixel_in_the_fill_moves_the_end_and_is_then_refused_for_lack_of_noise() {
    let (width, height) = (300usize, 1000usize);
    let mut pixels = luma(Sheet { width, height, sheet_rows: 600, frame: 0, ink: None, noisy: true });
    pixels[800 * width + 3] = 100;
    assert_eq!(assess_luma(width, height, &pixels, 200, &InferOptions::default()), Err(Refusal::NoPaperNoise));
}

#[test]
fn bmp_inference_and_crop_work_bottom_up_and_top_down_and_keep_the_width() {
    let (dpi, width, height) = (200u32, 600usize, 2000usize);
    let sheet = Sheet { width, height, sheet_rows: 1500, frame: 3, ink: Some((100, 400)), noisy: true };
    let pixels = luma(sheet);
    for top_down in [false, true] {
        let bytes = bmp_gray(width, height, &pixels, dpi, top_down);
        let found = infer_sheet_end(&bytes, dpi, &InferOptions::for_bytes(&bytes)).expect("bmp");
        assert_eq!(found.sheet_end_row, 1500);
        let mut page = page_of(bytes, width, height, dpi);
        let Inference::Cropped(done) = infer_and_crop(&mut page, &InferOptions::default()) else { panic!("cropped") };
        assert_eq!(page.height, done.rows);
        assert_eq!((page.width, page.dpi, page.format), (600, 200, "bmp"));
        assert_eq!(page.length.source, LengthSource::Inferred);
        assert!(page.length.approximate);
        assert_eq!(page.length.length_mils, Some(done.length_mils));
        // The cropped file decodes to exactly the top rows.
        let (w2, h2, again) = bw::decode_luma(&page.bytes).expect("decode");
        assert_eq!((w2, h2 as u32), (width, done.rows));
        assert_eq!(again[..], pixels[..again.len()], "top_down={top_down}");
        assert_eq!(crate::density::image_density(&page.bytes).map(|(x, _)| x.round() as u32), Some(200));
        assert_eq!(crate::bmp::size(&page.bytes), Some((600, done.rows)));
    }
}

#[test]
fn the_width_is_never_touched_even_with_a_fill_frame() {
    let (dpi, width, height) = (200u32, 500usize, 1800usize);
    let pixels = luma(Sheet { width, height, sheet_rows: 1300, frame: 40, ink: None, noisy: true });
    let mut page = page_of(bmp_gray(width, height, &pixels, dpi, true), width, height, dpi);
    assert!(matches!(infer_and_crop(&mut page, &InferOptions::default()), Inference::Cropped(_)));
    assert_eq!(page.width, 500);
}

#[test]
fn a_larger_margin_keeps_more_rows_and_never_more_than_the_image() {
    let (width, height) = (300usize, 1000usize);
    let pixels = luma(Sheet { width, height, sheet_rows: 990, frame: 0, ink: None, noisy: true });
    let big = InferOptions { margin_mils: 500, ..InferOptions::default() };
    assert_eq!(assess_luma(width, height, &pixels, 200, &big).map(|f| f.rows), Ok(1000));
    let none = InferOptions { margin_mils: 0, ..InferOptions::default() };
    assert_eq!(assess_luma(width, height, &pixels, 200, &none).map(|f| f.rows), Ok(991));
}

#[test]
fn png_and_tiff_pages_are_inferred_and_cut_in_their_own_format() {
    let (dpi, width, height) = (200u32, 300usize, 1200usize);
    let pixels = luma(Sheet { width, height, sheet_rows: 900, frame: 2, ink: Some((50, 300)), noisy: true });
    let image = image::GrayImage::from_raw(width as u32, height as u32, pixels).expect("gray");
    for (format, name) in [(image::ImageFormat::Png, "png"), (image::ImageFormat::Tiff, "tiff")] {
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, format).expect("encode");
        let bytes = out.into_inner();
        assert_eq!(crate::image_format(&bytes), name);
        let found = infer_sheet_end(&bytes, dpi, &InferOptions::for_bytes(&bytes)).expect(name);
        assert_eq!(found.sheet_end_row, 900, "{name}");
        let mut page = page_of(bytes, width, height, dpi);
        page.format = name;
        assert!(matches!(infer_and_crop(&mut page, &InferOptions::default()), Inference::Cropped(_)), "{name}");
        assert_eq!(crate::image_format(&page.bytes), name);
        assert_eq!(crate::image_size(&page.bytes), Some((300, page.height)));
    }
}

/// `pixels` (8-bit luminance) encoded as a JPEG at `quality`.
fn jpeg_of(width: usize, height: usize, pixels: &[u8], quality: u8) -> Vec<u8> {
    use image::ImageEncoder;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(pixels, width as u32, height as u32, image::ExtendedColorType::L8)
        .expect("jpeg");
    out.into_inner()
}

#[test]
fn cutting_a_grey_jpeg_keeps_it_grey_and_an_rgb_jpeg_stays_rgb() {
    use image::ImageEncoder;
    let (width, height) = (64usize, 40usize);
    let grey: Vec<u8> = (0..width * height).map(|i| (i * 7 % 200) as u8 + 20).collect();
    let bytes = jpeg_of(width, height, &grey, 90);
    assert_eq!(image::load_from_memory(&bytes).expect("decode").color(), image::ColorType::L8);

    let cut = crop_bottom_bytes(&bytes, 25).expect("cut");
    let decoded = image::load_from_memory(&cut).expect("decode the cut");
    assert_eq!(crate::image_format(&cut), "jpeg");
    assert_eq!((decoded.width(), decoded.height()), (64, 25));
    assert_eq!(decoded.color(), image::ColorType::L8, "a grey scan is still one component after the cut");

    // The same picture as RGB stays RGB, and is larger than the grey one.
    let rgb: Vec<u8> = grey.iter().flat_map(|v| [*v, *v, *v]).collect();
    let mut rgb_bytes = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut rgb_bytes, 90)
        .write_image(&rgb, width as u32, height as u32, image::ExtendedColorType::Rgb8)
        .expect("jpeg");
    let rgb_cut = crop_bottom_bytes(&rgb_bytes.into_inner(), 25).expect("cut");
    assert_eq!(image::load_from_memory(&rgb_cut).expect("decode").color(), image::ColorType::Rgb8);
    assert!(cut.len() < rgb_cut.len(), "{} grey bytes against {} rgb bytes", cut.len(), rgb_cut.len());
}

#[test]
fn a_jpeg_with_visible_paper_noise_is_inferred_with_a_tolerance_and_cropped() {
    let (dpi, width, height) = (200u32, 320usize, 1200usize);
    // Stronger noise than the lossless fixtures: JPEG smooths small amplitudes away.
    let mut rng = Lcg(99);
    let mut pixels = vec![255u8; width * height];
    for row in 8..900 {
        for col in 8..width - 8 {
            pixels[row * width + col] = 255 - 6 - (rng.next() % 40) as u8;
        }
    }
    let bytes = jpeg_of(width, height, &pixels, 90);
    assert_eq!(InferOptions::for_bytes(&bytes).tolerance, InferOptions::JPEG_TOLERANCE);
    let found = infer_sheet_end(&bytes, dpi, &InferOptions::for_bytes(&bytes)).expect("jpeg noise survives");
    // JPEG blocks and ringing blur the end by up to a block either way.
    assert!((i64::from(found.sheet_end_row) - 900).abs() <= 16, "{found:?}");
    let options = InferOptions::for_bytes(&bytes);
    let mut page = page_of(bytes, width, height, dpi);
    page.format = "jpeg";
    assert!(matches!(infer_and_crop(&mut page, &options), Inference::Cropped(_)));
    assert_eq!(crate::image_format(&page.bytes), "jpeg");
    assert_eq!(crate::image_size(&page.bytes), Some((320, page.height)));
}

#[test]
fn a_jpeg_whose_noise_was_smoothed_away_is_refused_not_guessed() {
    let (width, height) = (320usize, 1200usize);
    let mut pixels = vec![255u8; width * height];
    for row in 8..900 {
        for col in 8..width - 8 {
            // Plus or minus one level: gone after quantisation, so the paper reads as fill.
            pixels[row * width + col] = 254 + (((row + col) % 2) as u8);
        }
    }
    let bytes = jpeg_of(width, height, &pixels, 75);
    let result = assess_sheet_end(&bytes, 200, &InferOptions::for_bytes(&bytes));
    assert!(matches!(result, Err(Refusal::AllFill | Refusal::NoPaperNoise)), "{result:?}");
}

#[test]
fn a_cut_is_refused_when_the_image_is_not_shorter_or_cannot_be_cut() {
    let pixels = vec![200u8; 100 * 50];
    let bytes = bmp_gray(100, 50, &pixels, 200, false);
    assert!(crop_bottom_bytes(&bytes, 50).is_none());
    assert!(crop_bottom_bytes(&bytes, 0).is_none());
    assert!(crop_bottom_bytes(b"junk", 5).is_none());
    assert_eq!(crop_bottom_bytes(&bytes, 10).map(|b| crate::bmp::size(&b)), Some(Some((100, 10))));
}

#[test]
fn page_length_and_length_source_have_stable_wire_strings() {
    assert_eq!(PageLength::default(), PageLength::Driver);
    let names: Vec<_> = PageLength::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(names, ["driver", "size", "auto", "infer"]);
    assert_eq!(PageLength::parse("match"), Some(PageLength::Size));
    for value in PageLength::ALL {
        assert_eq!(PageLength::parse(value.as_str()), Some(*value));
    }
    assert_eq!(PageLength::parse("auto-infer"), Some(PageLength::AutoInfer));
    assert_eq!(PageLength::parse("nope"), None);
    let sources: Vec<_> = LengthSource::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(sources, ["driver", "device", "size", "inferred"]);
    assert!(PageLength::AutoInfer.may_infer() && !PageLength::Auto.may_infer());
    assert!(!PageLength::Driver.is_requested());
}

#[test]
fn records_convert_rows_to_inches_and_millimetres() {
    let record = LengthRecord::with_source(2146, 200, LengthSource::Inferred);
    assert_eq!(record.length_mils, Some(10_730));
    assert!(record.approximate);
    assert!((record.length_inches().unwrap() - 10.73).abs() < 1e-9);
    assert!((record.length_mm().unwrap() - 272.542).abs() < 0.01);
    let driver = LengthRecord::from_delivered(2800, 200);
    assert_eq!((driver.source, driver.approximate, driver.length_mils), (LengthSource::Driver, false, Some(14_000)));
    assert_eq!(LengthRecord::from_delivered(0, 200).length_mils, None);
    assert_eq!(LengthRecord::from_delivered(10, 0).length_mils, None);
}

#[test]
fn named_sizes_have_extents_except_auto_and_custom() {
    assert_eq!(PageSize::A4.extent_mils(), Some((8267, 11692)));
    assert_eq!(PageSize::Letter.extent_mils(), Some((8500, 11000)));
    assert_eq!(PageSize::Legal.extent_mils(), Some((8500, 14000)));
    assert_eq!(PageSize::Auto.extent_mils(), None);
    assert_eq!(PageSize::Custom.extent_mils(), None);
}

#[test]
fn a_page_built_by_the_page_builder_starts_as_a_driver_length() {
    let pixels = vec![255u8; 40 * 20];
    let bytes = bmp_gray(40, 20, &pixels, 100, false);
    let (page, _) = bw::page_for(&bytes, bw::PageRule { dpi: 100, dpi_y: None, bw_threshold: None, length: Default::default() });
    assert_eq!(page.length, LengthRecord { source: LengthSource::Driver, length_mils: Some(200), approximate: false });
}

#[test]
fn the_optional_local_scan_boundary_is_found_where_the_measured_scanner_put_it() {
    // Local and optional: set CRAFT_SCAN_PADDED_SAMPLE to a 200 dpi BMP of a 14 in extent from a scanner that pads
    // with pure white (the sheet ends at about 10.73 inches). Read only, never stored in the repository, skipped
    // when unset or unreadable.
    let Some(path) = std::env::var_os("CRAFT_SCAN_PADDED_SAMPLE") else { return };
    let Ok(bytes) = std::fs::read(path) else { return };
    let found = assess_sheet_end(&bytes, 200, &InferOptions::default()).expect("measured scan");
    assert_eq!(found.sheet_end_row, 2146);
    assert!((10_720..=10_780).contains(&found.length_mils), "{found:?}");
    assert!(found.noise_permille > 200);
}

#[test]
fn the_page_builder_infers_before_thresholding_so_black_and_white_pages_keep_the_paper_noise_evidence() {
    let (dpi, width, height) = (200u32, 400usize, 1800usize);
    let pixels = luma(Sheet { width, height, sheet_rows: 1400, frame: 3, ink: Some((100, 500)), noisy: true });
    let bytes = bmp_gray(width, height, &pixels, dpi, true);
    let plan = bw::LengthPlan { source: LengthSource::Driver, infer: Some(InferOptions::default()) };
    // Gray: cropped and recorded.
    let (gray, _) = bw::page_for(&bytes, bw::PageRule { dpi, dpi_y: None, bw_threshold: None, length: plan });
    assert_eq!((gray.height, gray.length.source, gray.length.approximate), (1402, LengthSource::Inferred, true));
    assert_eq!(gray.length.length_mils, Some(7010));
    // Black and white: the threshold turns the noise white, so inference must have run first.
    let (bw1, from) = bw::page_for(&bytes, bw::PageRule { dpi, dpi_y: None, bw_threshold: Some(128), length: plan });
    assert_eq!((bw1.height, bw1.thresholded, from), (1402, true, Some(8)));
    assert_eq!(bw1.length.source, LengthSource::Inferred);
    // No inference requested: untouched, and the plan's source is what is recorded.
    let sized = bw::LengthPlan { source: LengthSource::Size, infer: None };
    let (kept, _) = bw::page_for(&bytes, bw::PageRule { dpi, dpi_y: None, bw_threshold: None, length: sized });
    assert_eq!((kept.height, kept.length.source, kept.length.approximate), (1800, LengthSource::Size, false));
    // A refusal keeps the page and the plan's source.
    let blank = bmp_gray(width, height, &vec![255u8; width * height], dpi, true);
    let (refused, _) = bw::page_for(&blank, bw::PageRule { dpi, dpi_y: None, bw_threshold: None, length: plan });
    assert_eq!((refused.height, refused.length.source), (1800, LengthSource::Driver));
}

#[test]
fn presence_has_stable_wire_strings() {
    let names: Vec<_> = Presence::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(names, ["unknown", "absent", "present"]);
    assert_eq!(Presence::default(), Presence::Unknown);
}

#[test]
fn the_page_builder_gives_a_jpeg_the_formats_tolerance_even_with_default_options() {
    let (dpi, width, height) = (200u32, 320usize, 1200usize);
    let mut rng = Lcg(5);
    let mut pixels = vec![255u8; width * height];
    for row in 8..900 {
        for col in 8..width - 8 {
            pixels[row * width + col] = 255 - 6 - (rng.next() % 40) as u8;
        }
    }
    let bytes = jpeg_of(width, height, &pixels, 90);
    let plan = bw::LengthPlan { source: LengthSource::Driver, infer: Some(InferOptions::default()) };
    let (page, _) = bw::page_for(&bytes, bw::PageRule { dpi, dpi_y: None, bw_threshold: None, length: plan });
    assert_eq!(page.length.source, LengthSource::Inferred, "{:?}", page.length);
    assert!(page.length.approximate && page.height < 940 && page.height > 880, "{}", page.height);
    assert_eq!((page.format, page.width), ("jpeg", 320));
}
