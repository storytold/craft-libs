// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Win32 side: winspool (printers, driver settings) and GDI (page geometry, printing).
//! Every `unsafe` block says why it is sound. Buffers the system writes into are `u64`-backed so
//! the structures read from them are aligned.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDCW, CreatePen, DEVMODEW, DIB_RGB_COLORS, DM_IN_BUFFER, DM_IN_PROMPT, DM_OUT_BUFFER, DeleteDC,
    DeleteObject, Ellipse, GetDeviceCaps, GetStockObject, HALFTONE, HDC, HOLLOW_BRUSH, HORZRES, LOGPIXELSX, LOGPIXELSY, LineTo, MoveToEx,
    PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, PS_SOLID, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode,
    StretchDIBits, VERTRES,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, OpenPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE,
    PRINTER_INFO_4W,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::UI::ColorSystem::{ICM_DONE_OUTSIDEDC, ICM_OFF, SetICMMode};

use crate::{ColorPolicy, DevMode, Job, Page, Primitive, PrintError, Printed, Printer, Result, RgbImage, Shape};

const IDOK: i32 = 1;
const IDCANCEL: i32 = 2;
/// Wide strings read from the system stop here at the latest.
const MAX_WCHARS: usize = 32 * 1024;

fn last_error(what: &str) -> PrintError {
    PrintError::System(format!("{what} failed: {}", std::io::Error::last_os_error()))
}

fn wide(s: &str) -> Result<Vec<u16>> {
    if s.contains('\0') {
        return Err(PrintError::Invalid("a name contains a NUL character".into()));
    }
    Ok(s.encode_utf16().chain(Some(0)).collect())
}

/// A zeroed, 8-byte aligned buffer of at least `bytes` bytes.
fn aligned(bytes: usize) -> Vec<u64> {
    vec![0u64; bytes.div_ceil(8).max(1)]
}

fn bytes_of(buf: &[u64], len: usize) -> Vec<u8> {
    buf.iter().flat_map(|w| w.to_le_bytes()).take(len).collect()
}

/// Reads a NUL-terminated wide string.
///
/// # Safety
/// `p` is null or points to a NUL-terminated UTF-16 string that stays alive for the call.
unsafe fn read_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    // SAFETY: the caller guarantees a NUL-terminated string; we stop at the NUL (or the cap).
    while n < MAX_WCHARS && unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: the `n` units before the NUL were just read and are valid.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

pub fn printers() -> Result<Vec<Printer>> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    // SAFETY: a size query: no buffer (null, 0 bytes); the out-params are valid locals.
    unsafe { EnumPrintersW(flags, null(), 4, null_mut(), 0, &mut needed, &mut count) };
    if needed == 0 {
        return Ok(Vec::new());
    }
    let mut buf = aligned(needed as usize);
    // SAFETY: `buf` holds at least `needed` writable bytes, aligned for PRINTER_INFO_4W.
    let ok = unsafe { EnumPrintersW(flags, null(), 4, buf.as_mut_ptr().cast(), needed, &mut needed, &mut count) };
    if ok == 0 {
        return Err(last_error("listing printers"));
    }
    let default = default_printer().ok().flatten();
    let infos = buf.as_ptr().cast::<PRINTER_INFO_4W>();
    let fits = (needed as usize) / std::mem::size_of::<PRINTER_INFO_4W>();
    let mut out = Vec::new();
    for i in 0..(count as usize).min(fits) {
        // SAFETY: the system wrote `count` PRINTER_INFO_4W at the start of `buf` (bounded by
        // the buffer size above); their name pointers point into `buf`, which is alive.
        let name = unsafe { read_wide((*infos.add(i)).pPrinterName) };
        if !name.is_empty() {
            out.push(Printer { default: default.as_deref() == Some(name.as_str()), name });
        }
    }
    Ok(out)
}

pub fn default_printer() -> Result<Option<String>> {
    let mut len = 0u32;
    // SAFETY: a size query with a null buffer; `len` is a valid local.
    unsafe { GetDefaultPrinterW(null_mut(), &mut len) };
    if len == 0 || len as usize > MAX_WCHARS {
        return Ok(None);
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds `len` writable wide chars.
    if unsafe { GetDefaultPrinterW(buf.as_mut_ptr(), &mut len) } == 0 {
        return Ok(None);
    }
    // SAFETY: on success the buffer holds a NUL-terminated name.
    Ok(Some(unsafe { read_wide(buf.as_ptr()) }))
}

/// An open printer handle, closed on drop.
struct OpenPrinter(PRINTER_HANDLE);

impl OpenPrinter {
    fn open(name: &[u16]) -> Result<Self> {
        let mut h = PRINTER_HANDLE { Value: null_mut() };
        // SAFETY: `name` is NUL-terminated; `h` is a valid out-param; no defaults.
        if unsafe { OpenPrinterW(name.as_ptr(), &mut h, null()) } == 0 || h.Value.is_null() {
            return Err(last_error("opening the printer"));
        }
        Ok(OpenPrinter(h))
    }
}

impl Drop for OpenPrinter {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenPrinterW and is closed once.
        unsafe { ClosePrinter(self.0) };
    }
}

/// `settings` copied into an aligned buffer the driver can read.
///
/// There is deliberately no check of the printer name stored in the settings: Windows keeps only
/// 31 characters of it, and drivers fill it differently, so a long queue name ("Sublimation -
/// EPSON ET-18100 Series") failed such a check and the settings were silently dropped. The driver
/// itself validates them instead (see [`Dc::create`]).
fn aligned_copy(settings: &DevMode) -> Vec<u64> {
    let b = settings.as_bytes();
    let mut buf = aligned(b.len());
    for (slot, chunk) in buf.iter_mut().zip(b.chunks(8)) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        *slot = u64::from_le_bytes(w);
    }
    buf
}

pub fn properties(printer: &str, current: Option<&DevMode>, parent: Option<isize>, prompt: bool) -> Result<Option<DevMode>> {
    let name = wide(printer)?;
    let p = OpenPrinter::open(&name)?;
    let hwnd: HWND = parent.map_or(null_mut(), |h| h as HWND);
    // SAFETY: with fmode 0 DocumentPropertiesW only returns the size the settings need.
    let needed = unsafe { DocumentPropertiesW(hwnd, p.0, name.as_ptr(), null_mut(), null(), 0) };
    if needed <= 0 || needed as usize > (1 << 20) {
        return Err(last_error("reading the printer's settings"));
    }
    let mut out = aligned(needed as usize);
    let input = current.map(aligned_copy);
    let mut mode = DM_OUT_BUFFER;
    if input.is_some() {
        mode |= DM_IN_BUFFER;
    }
    if prompt {
        mode |= DM_IN_PROMPT;
    }
    let inp: *const DEVMODEW = input.as_ref().map_or(null(), |b| b.as_ptr().cast());
    // SAFETY: `out` holds `needed` aligned writable bytes as the size query asked for; `inp` is
    // null or an aligned copy of settings written by this printer's driver.
    let r = unsafe { DocumentPropertiesW(hwnd, p.0, name.as_ptr(), out.as_mut_ptr().cast(), inp, mode) };
    match r {
        IDOK => {
            let all = bytes_of(&out, needed as usize);
            let word = |at: usize| all.get(at..at + 2).map_or(0, |w| usize::from(u16::from_le_bytes([w[0], w[1]])));
            let len = (word(68) + word(70)).min(all.len());
            DevMode::from_bytes(all.into_iter().take(len).collect()).map(Some)
        }
        IDCANCEL => Ok(None),
        _ => Err(last_error("the printer's settings dialog")),
    }
}

/// A printer device context, deleted on drop.
struct Dc(HDC);

impl Dc {
    fn create(printer: &str, settings: Option<&DevMode>, manual_copies: bool) -> Result<Self> {
        let name = wide(printer)?;
        let normalized = if manual_copies {
            let defaults = match settings {
                Some(s) => s.clone(),
                None => {
                    properties(printer, None, None, false)?.ok_or_else(|| PrintError::Invalid("the printer refused its default settings".into()))?
                }
            };
            Some(defaults.for_manual_copies()?)
        } else {
            None
        };
        let settings = normalized.as_ref().or(settings);
        // The driver validates and completes the settings first (DM_IN_BUFFER | DM_OUT_BUFFER, no
        // dialog). If it refuses them, that is an error: never print on the defaults instead.
        let merged = match settings {
            Some(s) => Some(
                properties(printer, Some(s), None, false)?
                    .ok_or_else(|| PrintError::Invalid(format!("printer “{printer}” refused these settings; choose Print Settings… again")))?,
            ),
            None => None,
        };
        #[cfg(test)]
        if manual_copies && let Some(dm) = &merged {
            eprintln!(
                "validated driver copy fields: flags={:?} copies={:?} collate={:?}",
                dm.as_bytes().get(72..76),
                dm.as_bytes().get(86..88),
                dm.as_bytes().get(100..102)
            );
        }
        if manual_copies && !merged.as_ref().is_some_and(DevMode::has_manual_copies) {
            return Err(PrintError::Invalid("the driver refused single-copy settings; no print job was sent".into()));
        }
        let dm = merged.as_ref().map(aligned_copy);
        let dmp: *const DEVMODEW = dm.as_ref().map_or(null(), |b| b.as_ptr().cast());
        // SAFETY: `name` is NUL-terminated; `dmp` is null or aligned settings from this
        // printer's driver, alive for the call.
        let hdc = unsafe { CreateDCW(null(), name.as_ptr(), null(), dmp) };
        if hdc.is_null() {
            return Err(last_error(&format!("opening printer “{printer}”")));
        }
        Ok(Dc(hdc))
    }

    fn cap(&self, index: u32) -> i32 {
        // SAFETY: a valid printer DC and a documented capability index.
        unsafe { GetDeviceCaps(self.0, index as i32) }
    }

    fn page(&self) -> Result<Page> {
        let (dx, dy) = (self.cap(LOGPIXELSX), self.cap(LOGPIXELSY));
        if dx <= 0 || dy <= 0 {
            return Err(PrintError::System("the printer reported no resolution".into()));
        }
        let (fx, fy) = (72.0 / f64::from(dx), 72.0 / f64::from(dy));
        let (pw, ph) = (self.cap(PHYSICALWIDTH), self.cap(PHYSICALHEIGHT));
        let (ox, oy) = (self.cap(PHYSICALOFFSETX), self.cap(PHYSICALOFFSETY));
        let (hw, vh) = (self.cap(HORZRES), self.cap(VERTRES));
        let (pw, ph) = if pw > 0 && ph > 0 { (pw, ph) } else { (hw, vh) };
        Ok(Page {
            paper: (f64::from(pw) * fx, f64::from(ph) * fy),
            printable: (f64::from(ox) * fx, f64::from(oy) * fy, f64::from(hw) * fx, f64::from(vh) * fy),
            dpi: (dx as u32, dy as u32),
        })
    }
}

impl Drop for Dc {
    fn drop(&mut self) {
        // SAFETY: the DC came from CreateDCW and is deleted once.
        unsafe { DeleteDC(self.0) };
    }
}

pub fn page(printer: &str, settings: Option<&DevMode>) -> Result<Page> {
    Dc::create(printer, settings, false)?.page()
}

/// RGB rows top-down → a bottom-up 24-bit DIB (BGR, rows padded to 4 bytes).
fn dib(image: &RgbImage<'_>) -> Result<(BITMAPINFO, Vec<u8>)> {
    let (w, h) = (image.width as usize, image.height as usize);
    let stride = (w.checked_mul(3).ok_or_else(|| PrintError::Invalid("image too large".into()))? + 3) & !3;
    let total = stride
        .checked_mul(h)
        .filter(|t| *t <= i32::MAX as usize)
        .ok_or_else(|| PrintError::Invalid("image too large to print in one piece".into()))?;
    let mut bits = vec![0u8; total];
    for (y, src) in image.data.chunks_exact(w * 3).enumerate() {
        let start = (h - 1 - y) * stride;
        let Some(dst) = bits.get_mut(start..start + w * 3) else {
            continue;
        };
        for (d, s) in dst.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<3>().0) {
            *d = [s[2], s[1], s[0]];
        }
    }
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: image.width as i32,
        biHeight: image.height as i32,
        biPlanes: 1,
        biBitCount: 24,
        biCompression: BI_RGB,
        biSizeImage: total as u32,
        ..Default::default()
    };
    Ok((BITMAPINFO { bmiHeader: header, bmiColors: [Default::default()] }, bits))
}

/// Draw caller-composed strokes on a valid page DC. Geometry and marks remain with callers.
fn draw_primitives(dc: HDC, primitives: &[Primitive], page: &Page) -> Result<()> {
    for primitive in primitives {
        let [r, g, b] = primitive.color;
        let color = u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16);
        let width = (primitive.width * f64::from(page.dpi.0) / 72.0).round().clamp(1.0, 1e8) as i32;
        // SAFETY: documented pen/stock brush inputs; owned pen is deleted after restoring DC state.
        let pen = unsafe { CreatePen(PS_SOLID, width, color) };
        if pen.is_null() {
            return Err(last_error("creating a print stroke"));
        }
        // SAFETY: the stock brush is system-owned and is not deleted.
        let hollow = unsafe { GetStockObject(HOLLOW_BRUSH) };
        if hollow.is_null() {
            let error = last_error("getting a print brush");
            // SAFETY: this function owns pen.
            unsafe { DeleteObject(pen) };
            return Err(error);
        }
        // SAFETY: DC is valid for this page and these are valid GDI objects.
        let old_pen = unsafe { SelectObject(dc, pen) };
        if old_pen.is_null() || old_pen as isize == -1 {
            let error = last_error("selecting a print stroke");
            // SAFETY: the selection failed; this function still owns pen.
            unsafe { DeleteObject(pen) };
            return Err(error);
        }
        // SAFETY: valid page DC and a system-owned stock brush.
        let old_brush = unsafe { SelectObject(dc, hollow) };
        if old_brush.is_null() || old_brush as isize == -1 {
            let error = last_error("selecting a print brush");
            // SAFETY: restore the successfully selected pen before deleting it.
            unsafe {
                SelectObject(dc, old_pen);
                DeleteObject(pen);
            }
            return Err(error);
        }
        let point = |(x, y)| {
            let (x, y, _, _) = crate::device_rect(page, (x, y, 0.0, 0.0));
            (x, y)
        };
        let ok = match primitive.shape {
            Shape::Line { from, to } => {
                let (x, y) = point(from);
                let (u, v) = point(to);
                // SAFETY: valid DC and bounded device coordinates.
                unsafe { MoveToEx(dc, x, y, null_mut()) != 0 && LineTo(dc, u, v) != 0 }
            }
            Shape::Ellipse { bounds } => {
                let (x, y, w, h) = crate::device_rect(page, bounds);
                // SAFETY: valid DC and bounded device coordinates.
                unsafe { Ellipse(dc, x, y, x.saturating_add(w), y.saturating_add(h)) != 0 }
            }
        };
        let error = (!ok).then(|| last_error("drawing print strokes"));
        // SAFETY: restore original selections before deleting this function's pen.
        unsafe {
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
            DeleteObject(pen);
        }
        if let Some(error) = error {
            return Err(error);
        }
    }
    Ok(())
}

pub fn print(job: &Job<'_>, image: &RgbImage<'_>, rect: (f64, f64, f64, f64)) -> Result<Printed> {
    if image.width > i32::MAX as u32 || image.height > i32::MAX as u32 {
        return Err(PrintError::Invalid("image too large".into()));
    }
    let dc = Dc::create(job.printer, job.settings, true)?;
    let page = dc.page()?;
    let (x, y, w, h) = crate::device_rect(&page, rect);
    let (bmi, bits) = dib(image)?;
    let title = wide(job.title)?;
    let output = job.output_file.map(wide).transpose()?;
    let info = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: title.as_ptr(),
        lpszOutput: output.as_ref().map_or(null(), |o| o.as_ptr()),
        lpszDatatype: null(),
        fwType: 0,
    };
    let color_mode = match job.color_policy {
        ColorPolicy::DriverManaged => ICM_OFF,
        // ICM_OFF enables old-style halftone correction. Pixels converted by the caller
        // need both DC conversion and that correction off (Microsoft SetICMMode contract).
        ColorPolicy::ApplicationManaged | ColorPolicy::Unmanaged => ICM_DONE_OUTSIDEDC,
    };
    let set_color_mode = || {
        // SAFETY: a valid printer DC and a documented SetICMMode value.
        if unsafe { SetICMMode(dc.0, color_mode) } == 0 { Err(last_error("setting GDI color policy")) } else { Ok(()) }
    };
    set_color_mode()?;
    // SAFETY: a valid DC; HALFTONE resampling needs the brush origin reset afterwards.
    unsafe {
        SetStretchBltMode(dc.0, HALFTONE);
        SetBrushOrgEx(dc.0, 0, 0, null_mut());
    }
    // SAFETY: `info` and the strings it points to live until EndDoc/AbortDoc below.
    if unsafe { StartDocW(dc.0, &info) } <= 0 {
        return Err(last_error("starting the print job"));
    }
    let fail = |what: &str| {
        let e = last_error(what);
        // SAFETY: the job was started on this DC.
        unsafe { AbortDoc(dc.0) };
        e
    };
    for _ in 0..job.copies.clamp(1, 999) {
        // SAFETY: inside a started document on a valid DC.
        if unsafe { StartPage(dc.0) } <= 0 {
            return Err(fail("starting a page"));
        }
        // StartPage saves/restores DC state. Reapply the policy for each page, before pixels.
        if let Err(error) = set_color_mode() {
            // SAFETY: this DC owns the document started above.
            unsafe { AbortDoc(dc.0) };
            return Err(error);
        }
        // SAFETY: `bits` is a bottom-up DIB exactly as `bmi` describes (24-bit, padded rows,
        // `biSizeImage` bytes); the source rectangle is the whole image.
        let lines = unsafe {
            StretchDIBits(dc.0, x, y, w, h, 0, 0, image.width as i32, image.height as i32, bits.as_ptr().cast(), &bmi, DIB_RGB_COLORS, SRCCOPY)
        };
        if lines <= 0 {
            return Err(fail("sending the image to the printer"));
        }
        if let Err(error) = draw_primitives(dc.0, job.primitives, &page) {
            // SAFETY: this DC owns the started document.
            unsafe { AbortDoc(dc.0) };
            return Err(error);
        }
        // SAFETY: closes the page started above.
        if unsafe { EndPage(dc.0) } <= 0 {
            return Err(fail("finishing a page"));
        }
    }
    // SAFETY: closes the document started above.
    if unsafe { EndDoc(dc.0) } <= 0 {
        return Err(last_error("finishing the print job"));
    }
    Ok(Printed { device_rect: (x, y, w, h), dpi: page.dpi })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn dib_is_bottom_up_bgr_with_padded_rows() {
        // 2×2: red, green / blue, white.
        let data = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let (bmi, bits) = dib(&RgbImage { width: 2, height: 2, data: &data }).unwrap();
        assert_eq!(bmi.bmiHeader.biHeight, 2);
        assert_eq!(bits.len(), 16, "6 bytes per row padded to 8");
        // First stored row is the bottom one: blue, white.
        assert_eq!(&bits[0..6], &[255, 0, 0, 255, 255, 255]);
        assert_eq!(&bits[8..14], &[0, 0, 255, 0, 255, 0]);
    }

    #[test]
    fn settings_are_copied_aligned_and_intact() {
        let mut b: Vec<u8> = (0..230u32).map(|i| (i * 7) as u8).collect();
        b[68..70].copy_from_slice(&220u16.to_le_bytes());
        b[70..72].copy_from_slice(&10u16.to_le_bytes());
        let d = DevMode::from_bytes(b.clone()).unwrap();
        let buf = aligned_copy(&d);
        assert_eq!(buf.as_ptr().align_offset(std::mem::align_of::<DEVMODEW>()), 0);
        assert_eq!(bytes_of(&buf, b.len()), b, "no name check, no truncation: the bytes go through as chosen");
    }

    /// Settings chosen for a printer are what the driver prints with: paper and resolution survive
    /// the driver's own validation (needs "Microsoft Print to PDF"; run by hand).
    #[test]
    #[ignore]
    fn chosen_settings_reach_the_device_context() {
        let printer = "Microsoft Print to PDF";
        let d = crate::default_settings(printer).unwrap();
        let mut b = d.as_bytes().to_vec();
        // dmPaperSize = DMPAPER_A3 (8); dmFields |= DM_PAPERSIZE (2).
        b[78..80].copy_from_slice(&8i16.to_le_bytes());
        let fields = u32::from_le_bytes([b[72], b[73], b[74], b[75]]) | 2;
        b[72..76].copy_from_slice(&fields.to_le_bytes());
        let a3 = DevMode::from_bytes(b).unwrap();
        let pg = crate::page(printer, Some(&a3)).unwrap();
        assert!((pg.paper.0 - 841.9).abs() < 2.0 && (pg.paper.1 - 1190.6).abs() < 2.0, "A3, not the default: {:?}", pg.paper);
        // Landscape (dmOrientation = 2, DM_ORIENTATION = 1): the page comes back wide.
        let mut b = a3.as_bytes().to_vec();
        b[76..78].copy_from_slice(&2i16.to_le_bytes());
        let fields = u32::from_le_bytes([b[72], b[73], b[74], b[75]]) | 1;
        b[72..76].copy_from_slice(&fields.to_le_bytes());
        let pg = crate::page(printer, Some(&DevMode::from_bytes(b).unwrap())).unwrap();
        assert!((pg.paper.0 - 1190.6).abs() < 2.0 && (pg.paper.1 - 841.9).abs() < 2.0, "landscape A3: {:?}", pg.paper);
    }

    /// A real job through Windows' "Microsoft Print to PDF" driver, written to a file (needs that
    /// printer installed; run by hand).
    #[test]
    #[ignore]
    fn prints_through_a_real_driver_to_a_file() {
        let out = std::env::temp_dir().join(format!("pc-winprint-{}.pdf", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let settings = crate::default_settings("Microsoft Print to PDF").unwrap();
        let mut bytes = settings.as_bytes().to_vec();
        bytes[86..88].copy_from_slice(&5i16.to_le_bytes());
        let fields = u32::from_le_bytes(bytes[72..76].try_into().unwrap()) | 0x100;
        bytes[72..76].copy_from_slice(&fields.to_le_bytes());
        let settings = DevMode::from_bytes(bytes).unwrap();
        let (w, h) = (300u32, 200u32);
        let data: Vec<u8> = (0..w * h).flat_map(|i| if (i % w) < w / 2 { [220, 30, 30] } else { [30, 30, 220] }).collect();
        let out_s = out.to_string_lossy().into_owned();
        let primitives = [
            Primitive { shape: Shape::Line { from: (60.0, 72.0), to: (66.0, 72.0) }, width: 0.3, color: [0; 3] },
            Primitive { shape: Shape::Ellipse { bounds: (175.0, 50.0, 10.0, 10.0) }, width: 0.3, color: [0; 3] },
        ];
        let job = Job {
            printer: "Microsoft Print to PDF",
            settings: Some(&settings),
            title: "PhotoCraft test",
            output_file: Some(&out_s),
            copies: 2,
            color_policy: ColorPolicy::ApplicationManaged,
            primitives: &primitives,
        };
        let done = crate::print(&job, &RgbImage { width: w, height: h, data: &data }, (72.0, 72.0, 216.0, 144.0)).unwrap();
        assert_eq!(done.device_rect.2, done.dpi.0 as i32 * 3, "3 inches wide");
        let pdf = std::fs::read(&out).unwrap();
        assert!(pdf.starts_with(b"%PDF"), "the driver wrote a PDF");
        assert!(pdf.len() > 1000, "the page has content");
        // The driver vectorises our GDI line/ellipse calls as real PDF content: a job with every
        // mark on is meaningfully bigger than the same image with marks off.
        let no_marks_job = Job { primitives: &[], ..job };
        let done2 = crate::print(&no_marks_job, &RgbImage { width: w, height: h, data: &data }, (72.0, 72.0, 216.0, 144.0)).unwrap();
        let _ = done2;
        let pdf_no_marks = std::fs::read(&out).unwrap();
        assert!(pdf.len() > pdf_no_marks.len() + 100, "marks add vector content: {} vs {}", pdf.len(), pdf_no_marks.len());
        if let Ok(evidence) = std::env::var("PHOTOCRAFT_WINPRINT_TEST_EVIDENCE") {
            let evidence = std::path::Path::new(&evidence);
            std::fs::create_dir_all(evidence).unwrap();
            std::fs::write(evidence.join("copies2-driver5-marks.pdf"), &pdf).unwrap();
            std::fs::write(evidence.join("copies2-driver5-no-marks.pdf"), &pdf_no_marks).unwrap();
        }
        let _ = std::fs::remove_file(&out);
    }

    /// Lists this machine's printers and their pages (needs real printers; run by hand).
    #[test]
    #[ignore]
    fn lists_real_printers() {
        for p in printers().unwrap() {
            println!("{}{} {:?}", p.name, if p.default { " (default)" } else { "" }, page(&p.name, None));
        }
    }
}
