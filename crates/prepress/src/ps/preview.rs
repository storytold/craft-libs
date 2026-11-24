//! An EPS's TIFF preview: the source page rendered over white, as an uncompressed baseline RGB
//! TIFF (TIFF 6.0, Part 1).

use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::raster::{self, RenderCache};
use craft_pdf::syntax::Pdf;

use super::PsError;

/// Page `page` of `doc` at `dpi` as a TIFF.
pub(super) fn tiff(doc: &Pdf, page: usize, dpi: f32) -> Result<Vec<u8>, PsError> {
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(PsError::Options("the preview's resolution must be positive"));
    }
    let p = doc.pages().get(page).ok_or(PsError::Preview(format!("no page {page}")))?;
    let scale = dpi / 72.0;
    let options = raster::Options { scale: [scale, scale], background: [255, 255, 255, 255], ..Default::default() };
    let cache = RenderCache::new();
    let rendered = raster::render(p, &cache, &InterpreterSettings::default(), &options).map_err(|e| PsError::Preview(e.to_string()))?;
    // Over an opaque background every pixel is opaque, so premultiplied RGB is RGB.
    let rgb: Vec<u8> = rendered.bytes().as_chunks::<4>().0.iter().flat_map(|[r, g, b, _]| [*r, *g, *b]).collect();
    encode(rendered.width, rendered.height, dpi, &rgb).ok_or(PsError::Preview("the preview is too large for a TIFF".into()))
}

/// `rgb` (three bytes per pixel, rows top to bottom) as a little-endian TIFF of one strip.
fn encode(width: u32, height: u32, dpi: f32, rgb: &[u8]) -> Option<Vec<u8>> {
    const ENTRIES: u32 = 12;
    let data_len = u32::try_from(rgb.len()).ok()?;
    let ifd = 8;
    let extra = ifd + 2 + ENTRIES * 12 + 4;
    let (bits, xres, yres) = (extra, extra + 6, extra + 14);
    let data = extra + 22;
    data.checked_add(data_len)?;
    let resolution = (dpi * 100.0).round().clamp(1.0, u32::MAX as f32) as u32;

    let mut out = Vec::with_capacity(rgb.len() + data as usize);
    out.extend(b"II");
    out.extend(42u16.to_le_bytes());
    out.extend(ifd.to_le_bytes());
    out.extend((ENTRIES as u16).to_le_bytes());
    let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(count.to_le_bytes());
        out.extend(value.to_le_bytes());
    };
    let (short, long, rational) = (3, 4, 5);
    entry(256, long, 1, width);
    entry(257, long, 1, height);
    entry(258, short, 3, bits);
    entry(259, short, 1, 1);
    entry(262, short, 1, 2);
    entry(273, long, 1, data);
    entry(277, short, 1, 3);
    entry(278, long, 1, height);
    entry(279, long, 1, data_len);
    entry(282, rational, 1, xres);
    entry(283, rational, 1, yres);
    entry(296, short, 1, 2);
    out.extend(0u32.to_le_bytes());
    for _ in 0..3 {
        out.extend(8u16.to_le_bytes());
    }
    for _ in 0..2 {
        out.extend(resolution.to_le_bytes());
        out.extend(100u32.to_le_bytes());
    }
    out.extend_from_slice(rgb);
    Some(out)
}
