#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Press-accurate printing for the Craft apps: a PDF page separated into plates by
//! craft-prepress, composited into the printer's RGB profile and printed through
//! `craft_print::rgb` with its color already managed, so overprint, spot inks and the press's
//! colors reach the driver as the plates have them. See the README.

use std::ops::ControlFlow;

use craft_cms::{Builtin, ColorSpace, Intent, Profile};
use craft_prepress::{Band, Conversion, Options, Output, OutputError, plates_in_bands, press_profile};
use craft_print::rgb;

/// How a page becomes the printer's pixels.
#[derive(Clone, Debug)]
pub struct Settings {
    /// How the page separates: overprint, and the printing condition when the page has no
    /// output intent.
    pub separation: Options,
    /// The printer's RGB profile: the pixels go to the driver in this space. sRGB when the
    /// printer's own is not known.
    pub printer: Profile,
    /// The rendering intent from the press to the printer.
    pub intent: Intent,
    /// Black point compensation from the press to the printer.
    pub bpc: bool,
}

impl Settings {
    /// Default separation, relative colorimetric with black point compensation, into `printer`.
    pub fn new(printer: Profile) -> Self {
        Self { separation: Options::default(), printer, intent: Intent::RelativeColorimetric, bpc: true }
    }
}

/// A page as the press prints it, in the printer's RGB.
#[derive(Clone, Debug)]
pub struct PressPage {
    pub width: u32,
    pub height: u32,
    /// The resolution separated at, after craft-prepress's limits.
    pub dpi: f32,
    /// How process color that wasn't CMYK became CMYK.
    pub conversion: Conversion,
    /// Rows top to bottom, `width * 3` bytes each.
    pub rgb: Vec<u8>,
}

impl PressPage {
    /// The page as `craft_print::rgb` takes it.
    pub fn image(&self) -> rgb::RgbImage<'_> {
        rgb::RgbImage { width: self.width, height: self.height, data: &self.rgb }
    }
}

/// Why a page couldn't be rendered or printed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Separate(#[from] craft_prepress::Error),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error("the printer profile is not RGB")]
    PrinterNotRgb,
    #[error("the page image is too large to hold")]
    TooLarge,
    #[error("the plates could not be converted into the printer profile")]
    Composite,
    #[error(transparent)]
    Print(#[from] rgb::PrintError),
}

/// Page `page` (from 0) of `pdf` at `dpi` as the press prints it, in `settings.printer`.
///
/// The page separates in bands (`plates_in_bands`), each band composites into the printer's
/// profile from the printing condition its plates were made for (`press_profile`), and the
/// bands join into one image. Plates made by the device formulas have no profile; they convert
/// from craft-cms's Coated FOGRA39, as craft-prepress's preview does by default. The image is
/// at most `MAX_BANDED_PIXELS` pixels, 3 bytes each. Never panics.
pub fn render(pdf: &[u8], page: usize, dpi: f32, settings: &Settings) -> Result<PressPage, Error> {
    match render_within(pdf, page, dpi, settings, usize::MAX)? {
        Within::Page(page) => Ok(page),
        Within::Over(_) => Err(Error::TooLarge),
    }
}

/// Bytes of the largest image `rgb::print` takes.
const PRINT_BYTES: usize = 512 * 1024 * 1024;

/// A render that fits in a byte limit, or the bytes the page would take at that resolution.
enum Within {
    Page(PressPage),
    Over(usize),
}

/// [`render`], stopping at the first band when the page image would take more than `max` bytes.
fn render_within(pdf: &[u8], page: usize, dpi: f32, settings: &Settings, max: usize) -> Result<Within, Error> {
    if settings.printer.color_space != ColorSpace::Rgb {
        return Err(Error::PrinterNotRgb);
    }
    let press = press_profile(pdf, page, &settings.separation)?.unwrap_or_else(|| Builtin::CoatedFogra39.profile().clone());
    let output = Output::new(&press, &settings.printer, settings.intent, settings.bpc)?;
    let mut joined: Option<PressPage> = None;
    let mut over = None;
    let mut failed = None;
    plates_in_bands(pdf, page, dpi, &settings.separation, |band| {
        let len = image_len(band.plates.width, band.page_height);
        if joined.is_none() && len.is_some_and(|len| len > max) {
            over = len;
            return ControlFlow::Break(());
        }
        match add_band(&mut joined, band, &output) {
            Ok(()) => ControlFlow::Continue(()),
            Err(e) => {
                failed = Some(e);
                ControlFlow::Break(())
            }
        }
    })?;
    if let Some(e) = failed {
        return Err(e);
    }
    if let Some(bytes) = over {
        return Ok(Within::Over(bytes));
    }
    let page = joined.ok_or(Error::Composite)?;
    if Some(page.rgb.len()) != image_len(page.width, page.height) {
        return Err(Error::Composite);
    }
    Ok(Within::Page(page))
}

/// [`render`] at `dpi`, or at the resolution that keeps the page image within `max` bytes when
/// `dpi` would exceed it. The page then separates twice: the first time stops at its first band.
fn fitted(pdf: &[u8], page: usize, dpi: f32, settings: &Settings, max: usize) -> Result<PressPage, Error> {
    let bytes = match render_within(pdf, page, dpi, settings, max)? {
        Within::Page(page) => return Ok(page),
        Within::Over(bytes) => bytes,
    };
    // Pixels scale with the square of the resolution; the margin covers the floored sides.
    let dpi = dpi * ((max as f64 / bytes as f64).sqrt() * 0.99) as f32;
    match render_within(pdf, page, dpi, settings, max)? {
        Within::Page(page) => Ok(page),
        Within::Over(_) => Err(Error::TooLarge),
    }
}

/// Prints page `page` of `pdf` as the press prints it, on `job.printer` with `job.settings`,
/// into `rect` (points from the sheet's top-left, as `rgb::print` takes it), `job.copies`
/// times.
///
/// The page separates at the printer's resolution, the larger of its two, whatever the size of
/// `rect`, within craft-prepress's limits and the 512 MiB image `rgb::print` takes: past that,
/// at the resolution that fits. The job is application-managed whatever `job.color_policy` says:
/// the pixels are in the printer's profile already, and a second conversion by the driver
/// would undo the press colors. One call is one job; `rgb::print` takes one image per job.
pub fn print(job: &rgb::Job<'_>, pdf: &[u8], page: usize, settings: &Settings, rect: (f64, f64, f64, f64)) -> Result<rgb::Printed, Error> {
    let device = rgb::page(job.printer, job.settings)?;
    let dpi = device.dpi.0.max(device.dpi.1) as f32;
    let press = fitted(pdf, page, dpi, settings, PRINT_BYTES)?;
    let job = rgb::Job { color_policy: rgb::ColorPolicy::ApplicationManaged, ..*job };
    Ok(rgb::print(&job, &press.image(), rect)?)
}

/// Bytes of a `width` × `height` RGB image, `None` on overflow.
fn image_len(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)?.checked_mul(3)
}

/// Composites `band` into `output` and appends it to the page, allocated on the first band.
fn add_band(joined: &mut Option<PressPage>, band: Band, output: &Output) -> Result<(), Error> {
    let plates = band.plates;
    if joined.is_none() {
        let len = image_len(plates.width, band.page_height).ok_or(Error::TooLarge)?;
        let mut rgb = Vec::new();
        rgb.try_reserve_exact(len).map_err(|_| Error::TooLarge)?;
        *joined = Some(PressPage { width: plates.width, height: band.page_height, dpi: plates.dpi, conversion: plates.conversion.clone(), rgb });
    }
    let page = joined.as_mut().ok_or(Error::Composite)?;
    let start = image_len(plates.width, band.top).ok_or(Error::TooLarge)?;
    let pixels = plates.composite_to(&[], output);
    let fits = plates.width == page.width
        && page.rgb.len() == start
        && image_len(plates.width, plates.height) == Some(pixels.len())
        && start.checked_add(pixels.len()).is_some_and(|end| Some(end) <= image_len(page.width, page.height));
    if !fits {
        return Err(Error::Composite);
    }
    page.rgb.extend_from_slice(&pixels);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_too_large_to_print_separates_at_a_resolution_that_fits() {
        let content = "0 0 1 0 k 0 0 40 40 re f";
        let pdf = format!(
            "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
trailer << /Root 1 0 R >>
%%EOF",
            content.len()
        );
        let settings = Settings::new(craft_cms::builtin::srgb().clone());
        // 40 pt at 1800 dpi is 1000 × 1000 pixels, 3 MB.
        let max = 1_000_000;
        assert!(matches!(render_within(pdf.as_bytes(), 0, 1800.0, &settings, max).unwrap(), Within::Over(3_000_000)));
        let page = fitted(pdf.as_bytes(), 0, 1800.0, &settings, max).unwrap();
        assert!(page.rgb.len() <= max && page.rgb.len() > max * 9 / 10, "{} bytes", page.rgb.len());
        assert!(page.dpi < 1800.0 * 0.6, "at {} dpi", page.dpi);
        assert_eq!(fitted(pdf.as_bytes(), 0, 72.0, &settings, max).unwrap().dpi, 72.0, "a page that fits keeps its resolution");
    }
}
