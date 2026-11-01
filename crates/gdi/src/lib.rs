//! Safe Windows driver settings, printable geometry and RGB/GDI transport.
//! Callers own ICC conversion and page composition; this crate draws their pixels and strokes.
// SPDX-License-Identifier: MIT OR Apache-2.0
// Derived from Moiztermind's PhotoCraft printing proposal (#3013), with qualification fixes.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

use std::fmt;

/// Why a printing call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    /// Not on Windows.
    Unsupported,
    /// Bad input (an unknown printer, a malformed DEVMODE, an empty image).
    Invalid(String),
    /// A Win32 call failed.
    System(String),
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrintError::Unsupported => f.write_str("printing to a printer needs Windows here"),
            PrintError::Invalid(m) | PrintError::System(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for PrintError {}

pub type Result<T> = std::result::Result<T, PrintError>;

/// One installed printer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub default: bool,
}

/// A printer driver's settings (a Win32 DEVMODEW with the driver's private part), as bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevMode(Vec<u8>);

/// Offsets inside DEVMODEW: `dmSize` and `dmDriverExtra` follow the 32-wchar device name and
/// two version words.
const DM_SIZE_AT: usize = 68;
const DM_EXTRA_AT: usize = 70;
/// The smallest DEVMODEW a driver writes (Windows 95's, up to `dmICMIntent`).
const DM_MIN: usize = 156;
/// Larger blobs are not driver settings.
const DM_MAX: usize = 1 << 20;

impl DevMode {
    /// Validates driver settings bytes: the header's own sizes must add up to the length.
    pub fn from_bytes(b: Vec<u8>) -> Result<Self> {
        let word = |at: usize| b.get(at..at + 2).map(|w| usize::from(u16::from_le_bytes([w[0], w[1]])));
        let (Some(size), Some(extra)) = (word(DM_SIZE_AT), word(DM_EXTRA_AT)) else {
            return Err(PrintError::Invalid("printer settings are too short".into()));
        };
        if b.len() > DM_MAX || size < DM_MIN || size.checked_add(extra) != Some(b.len()) {
            return Err(PrintError::Invalid("printer settings are damaged; choose Print Settings again".into()));
        }
        Ok(DevMode(b))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The application emits every copy as a page. Disable driver copies/collation too,
    /// preserving the driver's private settings and all other public fields.
    #[cfg(any(windows, test))]
    pub(crate) fn for_manual_copies(&self) -> Result<Self> {
        let mut bytes = self.0.clone();
        let invalid = || PrintError::Invalid("printer settings are too short".into());
        let fields: [u8; 4] = bytes.get(72..76).and_then(|b| b.try_into().ok()).ok_or_else(invalid)?;
        let fields = u32::from_le_bytes(fields) | 0x100 | 0x8000; // DM_COPIES | DM_COLLATE
        bytes.get_mut(72..76).ok_or_else(invalid)?.copy_from_slice(&fields.to_le_bytes());
        bytes.get_mut(86..88).ok_or_else(invalid)?.copy_from_slice(&1i16.to_le_bytes());
        bytes.get_mut(100..102).ok_or_else(invalid)?.copy_from_slice(&0i16.to_le_bytes());
        Ok(Self(bytes))
    }

    #[cfg(any(windows, test))]
    pub(crate) fn has_manual_copies(&self) -> bool {
        let fields = self.0.get(72..76).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes);
        // dmFields identifies valid fields. A driver without hardware copies/collation may
        // clear their flags and return unspecified values; those cannot multiply our pages.
        fields.is_some_and(|f| {
            (f & 0x100 == 0 || self.0.get(86..88) == Some(1i16.to_le_bytes().as_slice()))
                && (f & 0x8000 == 0 || self.0.get(100..102) == Some(0i16.to_le_bytes().as_slice()))
        })
    }

    /// Hex, for JSON parameters and remembered print settings.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        if !s.len().is_multiple_of(2) || s.len() > DM_MAX * 2 {
            return Err(PrintError::Invalid("printer settings are damaged; choose Print Settings again".into()));
        }
        let bytes: Option<Vec<u8>> = (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
        Self::from_bytes(bytes.ok_or_else(|| PrintError::Invalid("printer settings are damaged; choose Print Settings again".into()))?)
    }
}

/// What a printer will print on with given settings. Points (1/72 in), origin top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    /// The whole sheet.
    pub paper: (f64, f64),
    /// The area the printer can reach: (x, y, width, height).
    pub printable: (f64, f64, f64, f64),
    /// Device resolution (dots per inch).
    pub dpi: (u32, u32),
}

/// An 8-bit RGB image, rows top to bottom, `width * 3` bytes each.
pub struct RgbImage<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

/// A vector shape in points from the sheet's top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A straight line.
    Line { from: (f64, f64), to: (f64, f64) },
    /// An unfilled ellipse, (left, top, width, height).
    Ellipse { bounds: (f64, f64, f64, f64) },
}

/// A caller-composed vector stroke. No document layout or print-mark policy lives here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primitive {
    pub shape: Shape,
    /// Stroke width in points.
    pub width: f64,
    /// RGB color (GDI COLORREF).
    pub color: [u8; 3],
}

impl Primitive {
    fn valid(&self) -> bool {
        if !self.width.is_finite() || self.width <= 0.0 || self.width > 72.0 {
            return false;
        }
        match self.shape {
            Shape::Line { from: (x, y), to: (u, v) } => [x, y, u, v].iter().all(|v| v.is_finite()),
            Shape::Ellipse { bounds: (x, y, w, h) } => [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0,
        }
    }
}

/// One print job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPolicy {
    /// The driver applies its color settings to the RGB image.
    DriverManaged,
    /// The caller has converted the pixels into the printer's RGB space.
    ApplicationManaged,
    /// The caller deliberately requests no DC color correction.
    Unmanaged,
}

/// One print job.
pub struct Job<'a> {
    pub printer: &'a str,
    pub settings: Option<&'a DevMode>,
    /// The name in the print queue.
    pub title: &'a str,
    /// Print to this file instead of the device (drivers that write files, and tests).
    pub output_file: Option<&'a str>,
    pub copies: u32,
    pub color_policy: ColorPolicy,
    pub primitives: &'a [Primitive],
}

/// Where a job went: the image box in device pixels and the device resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Printed {
    pub device_rect: (i32, i32, i32, i32),
    pub dpi: (u32, u32),
}

/// Maps an image box in points (origin top-left of the sheet) to device pixels (origin at the
/// printable area's corner, as GDI draws).
pub fn device_rect(page: &Page, rect: (f64, f64, f64, f64)) -> (i32, i32, i32, i32) {
    let (dx, dy) = (f64::from(page.dpi.0) / 72.0, f64::from(page.dpi.1) / 72.0);
    let px = |v: f64| {
        if v.is_finite() { v.round().clamp(-1e8, 1e8) as i32 } else { 0 }
    };
    let (x, y, w, h) = rect;
    (px((x - page.printable.0) * dx), px((y - page.printable.1) * dy), px(w * dx), px(h * dy))
}

/// The printers installed for this user, the default first.
pub fn printers() -> Result<Vec<Printer>> {
    #[cfg(windows)]
    {
        let mut v = windows::printers()?;
        v.sort_by(|a, b| b.default.cmp(&a.default).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(v)
    }
    #[cfg(not(windows))]
    Err(PrintError::Unsupported)
}

/// The default printer's name.
pub fn default_printer() -> Result<Option<String>> {
    #[cfg(windows)]
    {
        windows::default_printer()
    }
    #[cfg(not(windows))]
    Err(PrintError::Unsupported)
}

/// A printer's settings as its driver starts them.
pub fn default_settings(printer: &str) -> Result<DevMode> {
    #[cfg(windows)]
    {
        windows::properties(printer, None, None, false)?.ok_or_else(|| PrintError::System("the printer driver gave no settings".into()))
    }
    #[cfg(not(windows))]
    {
        let _ = printer;
        Err(PrintError::Unsupported)
    }
}

/// Shows the driver's Print Settings dialog (modal; `parent` is a window handle). `None` when
/// the user cancels.
pub fn settings_dialog(printer: &str, current: Option<&DevMode>, parent: Option<isize>) -> Result<Option<DevMode>> {
    #[cfg(windows)]
    {
        windows::properties(printer, current, parent, true)
    }
    #[cfg(not(windows))]
    {
        let _ = (printer, current, parent);
        Err(PrintError::Unsupported)
    }
}

/// The sheet and printable area for `printer` with `settings`.
pub fn page(printer: &str, settings: Option<&DevMode>) -> Result<Page> {
    #[cfg(windows)]
    {
        windows::page(printer, settings)
    }
    #[cfg(not(windows))]
    {
        let _ = (printer, settings);
        Err(PrintError::Unsupported)
    }
}

/// Prints `image` into `rect` (points, origin top-left of the sheet), `job.copies` pages.
pub fn print(job: &Job<'_>, image: &RgbImage<'_>, rect: (f64, f64, f64, f64)) -> Result<Printed> {
    let row = (image.width as usize).checked_mul(3).ok_or_else(|| PrintError::Invalid("image too large".into()))?;
    if image.width == 0 || image.height == 0 || row.checked_mul(image.height as usize) != Some(image.data.len()) {
        return Err(PrintError::Invalid("nothing to print".into()));
    }
    if ![rect.0, rect.1, rect.2, rect.3].iter().all(|v| v.is_finite()) || !(rect.2 > 0.0 && rect.3 > 0.0) {
        return Err(PrintError::Invalid("the print size is empty".into()));
    }
    if image.data.len() > 512 * 1024 * 1024 || job.primitives.len() > 10_000 || job.primitives.iter().any(|p| !p.valid()) {
        return Err(PrintError::Invalid("print image or vector strokes exceed supported limits".into()));
    }
    #[cfg(windows)]
    {
        windows::print(job, image, rect)
    }
    #[cfg(not(windows))]
    {
        let _ = job;
        Err(PrintError::Unsupported)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn devmode(size: u16, extra: u16) -> Vec<u8> {
        let mut b = vec![0u8; usize::from(size) + usize::from(extra)];
        b[DM_SIZE_AT..DM_SIZE_AT + 2].copy_from_slice(&size.to_le_bytes());
        b[DM_EXTRA_AT..DM_EXTRA_AT + 2].copy_from_slice(&extra.to_le_bytes());
        b
    }

    #[test]
    fn devmode_checks_its_own_sizes_and_round_trips_hex() {
        let d = DevMode::from_bytes(devmode(220, 1200)).unwrap();
        assert_eq!(DevMode::from_hex(&d.to_hex()).unwrap(), d);
        for bad in [vec![], vec![0; 40], devmode(100, 0), {
            let mut b = devmode(220, 10);
            b.pop();
            b
        }] {
            assert!(DevMode::from_bytes(bad).is_err());
        }
        for bad in ["", "abc", "zz", &"00".repeat(300)] {
            assert!(DevMode::from_hex(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn manual_copies_disable_driver_multiplication_without_losing_private_settings() {
        let mut bytes = devmode(220, 32);
        bytes[72..76].copy_from_slice(&0x8100u32.to_le_bytes());
        bytes[86..88].copy_from_slice(&7i16.to_le_bytes());
        bytes[100..102].copy_from_slice(&1i16.to_le_bytes());
        bytes[220..].fill(0x5a);
        let original = DevMode::from_bytes(bytes).unwrap();
        let normalized = original.for_manual_copies().unwrap();
        assert!(normalized.has_manual_copies());
        assert!(!original.has_manual_copies());
        assert_eq!(&normalized.as_bytes()[220..], &original.as_bytes()[220..]);
        assert_eq!(normalized.for_manual_copies().unwrap(), normalized);
        let mut ignored = normalized.as_bytes().to_vec();
        ignored[72..76].copy_from_slice(&0u32.to_le_bytes());
        ignored[86..88].copy_from_slice(&0i16.to_le_bytes());
        ignored[100..102].copy_from_slice(&1i16.to_le_bytes());
        assert!(DevMode::from_bytes(ignored.clone()).unwrap().has_manual_copies(), "drivers may omit unsupported copy/collation fields");
        ignored[72..76].copy_from_slice(&0x8100u32.to_le_bytes());
        assert!(!DevMode::from_bytes(ignored).unwrap().has_manual_copies(), "active driver fields must retain our copy policy");
    }

    #[test]
    fn device_rect_is_relative_to_the_printable_corner() {
        // A3 at 360 dpi with a 3 mm (8.5 pt) unprintable margin.
        let page = Page { paper: (841.89, 1190.55), printable: (8.5, 8.5, 824.89, 1173.55), dpi: (360, 360) };
        assert_eq!(device_rect(&page, (8.5, 8.5, 72.0, 36.0)), (0, 0, 360, 180));
        assert_eq!(device_rect(&page, (72.0, 144.0, 144.0, 72.0)), (318, 678, 720, 360));
        assert_eq!(device_rect(&page, (f64::NAN, f64::INFINITY, 1.0, 1.0)).0, 0);
    }

    #[test]
    fn print_rejects_empty_or_short_images_before_any_system_call() {
        let job =
            Job { printer: "x", settings: None, title: "t", output_file: None, copies: 1, color_policy: ColorPolicy::Unmanaged, primitives: &[] };
        let short = RgbImage { width: 2, height: 2, data: &[0; 11] };
        assert!(matches!(print(&job, &short, (0.0, 0.0, 10.0, 10.0)), Err(PrintError::Invalid(_))));
        let ok = RgbImage { width: 2, height: 2, data: &[0; 12] };
        assert!(matches!(print(&job, &ok, (0.0, 0.0, 0.0, 10.0)), Err(PrintError::Invalid(_))));
        for rect in [(f64::NAN, 0.0, 10.0, 10.0), (0.0, 0.0, f64::INFINITY, 10.0)] {
            assert!(matches!(print(&job, &ok, rect), Err(PrintError::Invalid(_))));
        }
    }

    #[test]
    fn hostile_vector_geometry_is_rejected_before_opening_a_printer() {
        let image = RgbImage { width: 1, height: 1, data: &[0; 3] };
        for shape in [Shape::Line { from: (f64::NAN, 0.0), to: (1.0, 1.0) }, Shape::Ellipse { bounds: (0.0, 0.0, -1.0, 1.0) }] {
            let primitives = [Primitive { shape, width: 0.3, color: [0; 3] }];
            let job = Job {
                printer: "not a printer",
                settings: None,
                title: "test",
                output_file: None,
                copies: 1,
                color_policy: ColorPolicy::Unmanaged,
                primitives: &primitives,
            };
            assert!(matches!(print(&job, &image, (0.0, 0.0, 10.0, 10.0)), Err(PrintError::Invalid(_))));
        }
    }
}
