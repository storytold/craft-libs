//! The device-independent preview (EPSI): a bitmap in hexadecimal comment lines between
//! `%%BeginPreview: width height depth lines` and `%%EndPreview` (EPSF 3.0, section 6).

/// Largest EPSI bitmap side read (pixels).
pub const MAX_EPSI: usize = 8192;

/// A grey bitmap, 8 bits a pixel, row by row from the top: 0 is black, 255 white.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub gray: Vec<u8>,
}

/// The EPSI preview of `ps`, if it has one of 1, 2, 4 or 8 bits a pixel and all its samples.
/// In the file a sample of 0 is white (EPSF 3.0: "0 is white"); the bitmap is turned around.
pub fn epsi(ps: &[u8]) -> Option<Bitmap> {
    let text = String::from_utf8_lossy(ps);
    let mut lines = text.split(['\n', '\r']).filter(|l| !l.is_empty());
    let head = lines.find_map(|l| l.strip_prefix("%%BeginPreview:"))?;
    let mut n = head.split_whitespace().map_while(|w| w.parse::<usize>().ok());
    let (w, h, depth) = (n.next()?, n.next()?, n.next()?);
    if !(1..=MAX_EPSI).contains(&w) || !(1..=MAX_EPSI).contains(&h) || ![1, 2, 4, 8].contains(&depth) {
        return None;
    }
    let row = (w * depth).div_ceil(8);
    let need = row * h;
    let mut bytes = Vec::with_capacity(need.min(ps.len() / 2));
    let mut high = None;
    'lines: for l in lines {
        if l.starts_with("%%EndPreview") {
            break;
        }
        for digit in l.strip_prefix('%')?.chars().filter_map(|c| c.to_digit(16)) {
            match high.take() {
                None => high = Some(digit),
                Some(hi) => bytes.push((hi << 4 | digit) as u8),
            }
            if bytes.len() == need {
                break 'lines;
            }
        }
    }
    // A preview with its samples missing is none, not a blank image of the size it claims.
    if bytes.len() < need {
        return None;
    }
    let max = (1u32 << depth) - 1;
    let mut gray = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let bit = y * row * 8 + x * depth;
            let byte = u32::from(bytes.get(bit / 8).copied().unwrap_or(0));
            let v = (byte >> (8 - depth - bit % 8)) & max;
            gray.push((255 - v * 255 / max) as u8);
        }
    }
    Some(Bitmap { width: w as u32, height: h as u32, gray })
}
