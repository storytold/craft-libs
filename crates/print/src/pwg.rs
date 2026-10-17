//! PWG raster (`image/pwg-raster`) writer for grey, sRGB and Adobe RGB pages, 8 or 16 bits a colour ([`RasterType`]). Always available (no cargo
//! feature, no dependency, builds for `wasm32`): an app's [`DocumentConverter`](crate::DocumentConverter)
//! that has already rendered pages to pixels calls [`encode`] to get a document a printer accepts.
//! (`craft_print::wsd::pwg` re-exports this module.)
//!
//! The stream is the sync word `RaS2`, then per page a 1796-byte big-endian header followed by the
//! pixel rows. Each row is preceded by a byte saying how many extra times the same row repeats
//! (up to 255) and is packed in runs: a control byte `c` of 0 to 127 repeats the next pixel `c + 1`
//! times, and 129 to 255 introduces `257 - c` literal pixels (byte 128 is reserved and never
//! written). Written from the public format description; it has been checked here only against
//! this crate's own reader, so a first run against a printer or a third-party decoder is still to
//! be done (see the README follow-ups).

use std::fmt;

/// Why a stream could not be written (or, in tests, read).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PwgError {
    /// The pages are not something raster can carry: none, too many, a size outside the limits or a
    /// pixel buffer that does not match its size.
    Invalid(String),
    /// A page or the stream would exceed a size limit (what, in bytes).
    TooLarge(&'static str, usize),
}

impl fmt::Display for PwgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(why) => f.write_str(why),
            Self::TooLarge(what, limit) => write!(f, "{what} is larger than the {limit} byte limit"),
        }
    }
}

impl std::error::Error for PwgError {}

/// The result of writing a stream.
pub type Result<T> = std::result::Result<T, PwgError>;

/// Pages in one stream.
pub const MAX_PAGES: usize = 500;
/// The widest or tallest page in pixels.
pub const MAX_SIDE: u32 = 20_000;
/// The most raw bytes in one page.
pub const MAX_PAGE_BYTES: usize = 384 << 20;
/// The most bytes in the whole stream.
pub const MAX_STREAM_BYTES: usize = 1 << 30;

const SYNC: &[u8; 4] = b"RaS2";
const HEADER: usize = 1796;

/// The pixel formats of a PWG raster page: a colour space and a depth (what a printer lists in
/// `pwg-raster-document-type-supported`: `sgray_8`, `srgb_16`, `adobe-rgb_8`...). The raster header
/// names the colour space (18 sGray, 19 sRGB, 20 Adobe RGB) and the bits per colour; 16-bit samples
/// are written most significant byte first. Types a printer may also list (CMYK, device colour, one
/// bit) are not written here and [`Self::from_keyword`] does not know them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RasterType {
    Sgray8,
    Sgray16,
    Srgb8,
    Srgb16,
    AdobeRgb8,
    AdobeRgb16,
}

impl RasterType {
    /// Every type, in no particular order.
    pub const ALL: [RasterType; 6] = [Self::Sgray8, Self::Sgray16, Self::Srgb8, Self::Srgb16, Self::AdobeRgb8, Self::AdobeRgb16];

    /// What a printer is taken to accept when it does not list its types: the two every PWG raster
    /// printer is required to take.
    pub const BASELINE: [RasterType; 2] = [Self::Srgb8, Self::Sgray8];

    /// The IPP keyword (`pwg-raster-document-type-supported`).
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Sgray8 => "sgray_8",
            Self::Sgray16 => "sgray_16",
            Self::Srgb8 => "srgb_8",
            Self::Srgb16 => "srgb_16",
            Self::AdobeRgb8 => "adobe-rgb_8",
            Self::AdobeRgb16 => "adobe-rgb_16",
        }
    }

    /// The type a printer's keyword names (case does not matter); `None` for one this writer does
    /// not make.
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        let keyword = keyword.trim();
        Self::ALL.into_iter().find(|t| t.keyword().eq_ignore_ascii_case(keyword))
    }

    /// One channel (grey) or three (RGB).
    pub const fn channels(self) -> usize {
        if self.is_gray() { 1 } else { 3 }
    }

    pub const fn is_gray(self) -> bool {
        matches!(self, Self::Sgray8 | Self::Sgray16)
    }

    /// Bits in one colour sample: 8 or 16.
    pub const fn bits(self) -> u32 {
        match self {
            Self::Sgray8 | Self::Srgb8 | Self::AdobeRgb8 => 8,
            _ => 16,
        }
    }

    /// Bytes in one pixel.
    pub const fn bytes_per_pixel(self) -> usize {
        self.channels() * (self.bits() as usize / 8)
    }

    /// The header's colour space code: 18 sGray, 19 sRGB, 20 Adobe RGB.
    pub const fn colour_space(self) -> u32 {
        match self {
            Self::Sgray8 | Self::Sgray16 => 18,
            Self::Srgb8 | Self::Srgb16 => 19,
            Self::AdobeRgb8 | Self::AdobeRgb16 => 20,
        }
    }
}

/// One page of pixels in a [`RasterType`], top row first, rows tightly packed, 16-bit samples most
/// significant byte first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedPage {
    pub width: u32,
    pub height: u32,
    pub kind: RasterType,
    pub data: Vec<u8>,
}

/// One page of 8-bit pixels, top row first, rows tightly packed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterPage {
    pub width: u32,
    pub height: u32,
    /// `true`: one byte per pixel (sGray); `false`: three (sRGB).
    pub gray: bool,
    pub pixels: Vec<u8>,
}

/// A page as the writer reads it, whichever public type it came from.
struct View<'a> {
    width: u32,
    height: u32,
    kind: RasterType,
    data: &'a [u8],
}

impl<'a> From<&'a RasterPage> for View<'a> {
    fn from(p: &'a RasterPage) -> Self {
        View { width: p.width, height: p.height, kind: if p.gray { RasterType::Sgray8 } else { RasterType::Srgb8 }, data: &p.pixels }
    }
}

impl<'a> From<&'a TypedPage> for View<'a> {
    fn from(p: &'a TypedPage) -> Self {
        View { width: p.width, height: p.height, kind: p.kind, data: &p.data }
    }
}

/// Two-sided printing as the raster header states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplex {
    Off,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug)]
pub struct PwgOptions {
    pub dpi: (u32, u32),
    pub duplex: Duplex,
}

fn invalid<T>(why: impl Into<String>) -> Result<T> {
    Err(PwgError::Invalid(why.into()))
}

fn set_u32(header: &mut [u8], at: usize, v: u32) {
    if let Some(slot) = header.get_mut(at..at + 4) {
        slot.copy_from_slice(&v.to_be_bytes());
    }
}

fn set_str(header: &mut [u8], at: usize, s: &str) {
    if let Some(slot) = header.get_mut(at..at + s.len().min(63)) {
        slot.copy_from_slice(s.as_bytes().get(..slot.len()).unwrap_or(&[]));
    }
}

fn page_header(page: &View<'_>, total: u32, opts: &PwgOptions) -> Vec<u8> {
    let mut h = vec![0u8; HEADER];
    let bpp = page.kind.bytes_per_pixel() as u32;
    set_str(&mut h, 0, "PwgRaster");
    let (dx, dy) = (opts.dpi.0.max(1), opts.dpi.1.max(1));
    let points = |px: u32, dpi: u32| ((u64::from(px) * 72 + u64::from(dpi) / 2) / u64::from(dpi)) as u32;
    set_u32(&mut h, 272, u32::from(opts.duplex != Duplex::Off)); // Duplex
    set_u32(&mut h, 276, dx); // HWResolution
    set_u32(&mut h, 280, dy);
    set_u32(&mut h, 340, 1); // NumCopies: copies are a ticket matter
    set_u32(&mut h, 352, points(page.width, dx)); // PageSize in points
    set_u32(&mut h, 356, points(page.height, dy));
    set_u32(&mut h, 368, u32::from(opts.duplex == Duplex::ShortEdge)); // Tumble
    set_u32(&mut h, 372, page.width);
    set_u32(&mut h, 376, page.height);
    set_u32(&mut h, 384, page.kind.bits()); // bits per colour
    set_u32(&mut h, 388, 8 * bpp); // bits per pixel
    set_u32(&mut h, 392, page.width.saturating_mul(bpp)); // bytes per line
    set_u32(&mut h, 396, 0); // chunky
    set_u32(&mut h, 400, page.kind.colour_space()); // sGray / sRGB / Adobe RGB
    set_u32(&mut h, 420, page.kind.channels() as u32); // colours
    set_u32(&mut h, 452, total); // TotalPageCount
    set_u32(&mut h, 456, 1); // CrossFeedTransform
    set_u32(&mut h, 460, 1); // FeedTransform
    set_u32(&mut h, 472, page.width); // ImageBoxRight
    set_u32(&mut h, 476, page.height); // ImageBoxBottom
    h
}

/// Pack `pages` into a PWG raster stream.
pub fn encode(pages: &[RasterPage], opts: &PwgOptions) -> Result<Vec<u8>> {
    encode_views(&pages.iter().map(View::from).collect::<Vec<_>>(), opts)
}

/// [`encode`] for pages of any [`RasterType`] (all pages of a stream may differ).
pub fn encode_typed(pages: &[TypedPage], opts: &PwgOptions) -> Result<Vec<u8>> {
    encode_views(&pages.iter().map(View::from).collect::<Vec<_>>(), opts)
}

fn encode_views(pages: &[View<'_>], opts: &PwgOptions) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return invalid("there are no pages");
    }
    if pages.len() > MAX_PAGES {
        return invalid(format!("more than {MAX_PAGES} pages"));
    }
    let mut out = Vec::new();
    out.extend_from_slice(SYNC);
    for page in pages {
        check_page(page)?;
        out.extend_from_slice(&page_header(page, pages.len() as u32, opts));
        let bpp = page.kind.bytes_per_pixel();
        let stride = page.width as usize * bpp;
        let mut rows = page.data.chunks_exact(stride.max(1)).peekable();
        while let Some(row) = rows.next() {
            let mut repeats = 0u8;
            while repeats < 255 && rows.peek().is_some_and(|next| *next == row) {
                rows.next();
                repeats += 1;
            }
            out.push(repeats);
            pack_row(row, bpp, &mut out);
        }
        if out.len() > MAX_STREAM_BYTES {
            return Err(PwgError::TooLarge("the raster stream", MAX_STREAM_BYTES));
        }
    }
    Ok(out)
}

fn check_page(p: &View<'_>) -> Result<()> {
    if p.width == 0 || p.height == 0 || p.width > MAX_SIDE || p.height > MAX_SIDE {
        return invalid(format!("page size {}x{} is outside 1 to {MAX_SIDE} pixels", p.width, p.height));
    }
    let need = (p.width as usize).checked_mul(p.height as usize).and_then(|n| n.checked_mul(p.kind.bytes_per_pixel()));
    match need {
        Some(n) if n <= MAX_PAGE_BYTES && n == p.data.len() => Ok(()),
        Some(n) if n > MAX_PAGE_BYTES => Err(PwgError::TooLarge("a raster page", MAX_PAGE_BYTES)),
        _ => invalid("the pixel buffer does not match the page size"),
    }
}

fn pack_row(row: &[u8], bpp: usize, out: &mut Vec<u8>) {
    let pixels: Vec<&[u8]> = row.chunks_exact(bpp).collect();
    let n = pixels.len();
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while run < 128 && i + run < n && pixels.get(i + run) == pixels.get(i) {
            run += 1;
        }
        if run >= 2 {
            out.push((run - 1) as u8);
            out.extend_from_slice(pixels.get(i).copied().unwrap_or(&[]));
            i += run;
            continue;
        }
        // A literal stretch: up to 128 pixels, stopping before the next run of two or more.
        let mut m = 1;
        while m < 128 && i + m < n && pixels.get(i + m) != pixels.get(i + m + 1) {
            m += 1;
        }
        if i + m >= n {
            m = n - i;
        }
        if m == 1 {
            out.push(0);
        } else {
            out.push((257 - m) as u8);
        }
        for p in pixels.get(i..i + m).unwrap_or(&[]) {
            out.extend_from_slice(p);
        }
        i += m;
    }
}

/// A reader for tests: the pages of a stream as (width, height, gray, pixels).
#[cfg(test)]
pub(crate) fn decode_typed_for_tests(data: &[u8]) -> Result<Vec<TypedPage>> {
    let be = |d: &[u8], at: usize| -> Result<u32> {
        d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| PwgError::Invalid("short header".into()))
    };
    if data.get(..4) != Some(&SYNC[..]) {
        return invalid("bad sync word");
    }
    let mut at = 4;
    let mut pages = Vec::new();
    while at < data.len() {
        let h = data.get(at..at + HEADER).ok_or_else(|| PwgError::Invalid("short header".into()))?;
        at += HEADER;
        let (w, ht, bpp) = (be(h, 372)? as usize, be(h, 376)? as usize, (be(h, 388)? / 8) as usize);
        if bpp == 0 || w == 0 || ht == 0 {
            return invalid("empty page");
        }
        let mut pixels = Vec::new();
        let mut rows_done = 0;
        while rows_done < ht {
            let repeat = *data.get(at).ok_or_else(|| PwgError::Invalid("short rows".into()))? as usize + 1;
            at += 1;
            let mut row = Vec::new();
            while row.len() < w * bpp {
                let c = *data.get(at).ok_or_else(|| PwgError::Invalid("short row".into()))?;
                at += 1;
                if c <= 127 {
                    let px = data.get(at..at + bpp).ok_or_else(|| PwgError::Invalid("short run".into()))?;
                    at += bpp;
                    for _ in 0..=c {
                        row.extend_from_slice(px);
                    }
                } else if c == 128 {
                    return invalid("reserved control byte");
                } else {
                    let n = 257 - c as usize;
                    row.extend_from_slice(data.get(at..at + n * bpp).ok_or_else(|| PwgError::Invalid("short literal".into()))?);
                    at += n * bpp;
                }
            }
            if row.len() != w * bpp {
                return invalid("row overran");
            }
            for _ in 0..repeat.min(ht - rows_done) {
                pixels.extend_from_slice(&row);
            }
            rows_done += repeat;
        }
        let kind = match (be(h, 400)?, be(h, 384)?) {
            (18, 8) => RasterType::Sgray8,
            (18, 16) => RasterType::Sgray16,
            (19, 8) => RasterType::Srgb8,
            (19, 16) => RasterType::Srgb16,
            (20, 8) => RasterType::AdobeRgb8,
            (20, 16) => RasterType::AdobeRgb16,
            other => return invalid(format!("unknown colour space and depth {other:?}")),
        };
        pages.push(TypedPage { width: w as u32, height: ht as u32, kind, data: pixels });
    }
    Ok(pages)
}

/// The 8-bit pages of a stream, for the older tests.
#[cfg(test)]
pub(crate) fn decode_for_tests(data: &[u8]) -> Result<Vec<RasterPage>> {
    decode_typed_for_tests(data)?
        .into_iter()
        .map(|p| match p.kind {
            RasterType::Sgray8 | RasterType::Srgb8 => Ok(RasterPage { width: p.width, height: p.height, gray: p.kind.is_gray(), pixels: p.data }),
            other => invalid(format!("a {} page is not 8-bit gray or sRGB", other.keyword())),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> PwgOptions {
        PwgOptions { dpi: (300, 300), duplex: Duplex::Off }
    }

    fn page(w: u32, h: u32, gray: bool, f: impl Fn(u32, u32) -> u8) -> RasterPage {
        let bpp = if gray { 1 } else { 3 };
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                for c in 0..bpp {
                    pixels.push(f(x, y).wrapping_add(c as u8 * 7));
                }
            }
        }
        RasterPage { width: w, height: h, gray, pixels }
    }

    #[test]
    fn pages_round_trip_through_the_reader() {
        let pages = vec![
            page(5, 4, true, |x, y| (x * 13 + y * 7) as u8),
            page(300, 3, false, |x, _| if x % 50 < 25 { 0 } else { 200 }),
            page(1, 1, true, |_, _| 5),
            page(130, 300, true, |_, _| 255),
            page(257, 2, false, |x, y| (x * y) as u8),
        ];
        let bytes = encode(&pages, &opts()).unwrap();
        assert_eq!(decode_for_tests(&bytes).unwrap(), pages);
    }

    #[test]
    fn the_header_has_the_documented_size_and_fields() {
        let p = page(300, 600, false, |_, _| 9);
        let bytes = encode(std::slice::from_ref(&p), &PwgOptions { dpi: (300, 300), duplex: Duplex::ShortEdge }).unwrap();
        assert_eq!(&bytes[..4], b"RaS2");
        let h = &bytes[4..4 + HEADER];
        assert_eq!(&h[..9], b"PwgRaster");
        let be = |at: usize| u32::from_be_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]]);
        assert_eq!((be(372), be(376), be(384), be(388), be(392), be(400), be(420)), (300, 600, 8, 24, 900, 19, 3));
        assert_eq!((be(352), be(356)), (72, 144)); // points
        assert_eq!((be(272), be(368), be(276), be(280), be(452)), (1, 1, 300, 300, 1));
    }

    #[test]
    fn identical_rows_and_pixels_compress() {
        let flat = page(1000, 1000, true, |_, _| 255);
        let bytes = encode(&[flat], &opts()).unwrap();
        // 1000 rows collapse to 4 row groups of at most 256, each a few run bytes.
        assert!(bytes.len() < 4 + HEADER + 100, "{}", bytes.len());
    }

    #[test]
    fn worst_case_literals_stay_within_one_control_byte_per_128() {
        let noisy = page(1000, 1, true, |x, _| (x * 37 % 251) as u8);
        let bytes = encode(&[noisy], &opts()).unwrap();
        assert!(bytes.len() <= 4 + HEADER + 1 + 1000 + 8 + 1);
    }

    #[test]
    fn bad_pages_are_errors() {
        assert!(encode(&[], &opts()).is_err());
        let ok = page(2, 2, true, |_, _| 1);
        assert!(encode(&[RasterPage { pixels: vec![0; 3], ..ok.clone() }], &opts()).is_err());
        assert!(encode(&[RasterPage { width: 0, ..ok.clone() }], &opts()).is_err());
        assert!(encode(&[RasterPage { width: MAX_SIDE + 1, ..ok.clone() }], &opts()).is_err());
        assert!(matches!(
            encode(&[RasterPage { width: MAX_SIDE, height: MAX_SIDE, gray: false, pixels: vec![] }], &opts()),
            Err(PwgError::TooLarge(..))
        ));
        assert!(encode(&vec![ok; MAX_PAGES + 1], &opts()).is_err());
        // A zero dpi cannot divide by zero.
        assert!(encode(&[page(2, 2, true, |_, _| 1)], &PwgOptions { dpi: (0, 0), duplex: Duplex::Off }).is_ok());
    }

    #[test]
    fn the_reader_rejects_garbage() {
        assert!(decode_for_tests(b"nope").is_err());
        assert!(decode_for_tests(b"RaS2\0\0").is_err());
        let mut bytes = encode(&[page(4, 4, true, |_, _| 3)], &opts()).unwrap();
        bytes.truncate(bytes.len() - 1);
        assert!(decode_for_tests(&bytes).is_err());
    }

    fn typed(w: u32, h: u32, kind: RasterType, f: impl Fn(u32, u32, usize) -> u8) -> TypedPage {
        let per_sample = kind.bits() as usize / 8;
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                for c in 0..kind.channels() {
                    for k in 0..per_sample {
                        data.push(f(x, y, c * per_sample + k));
                    }
                }
            }
        }
        TypedPage { width: w, height: h, kind, data }
    }

    #[test]
    fn every_type_round_trips_and_names_itself_in_the_header() {
        let pages: Vec<TypedPage> = RasterType::ALL.iter().map(|&k| typed(7, 5, k, |x, y, c| (x * 31 + y * 17 + c as u32 * 5) as u8)).collect();
        let bytes = encode_typed(&pages, &opts()).unwrap();
        assert_eq!(decode_typed_for_tests(&bytes).unwrap(), pages);
        // the header of each kind: colour space, bits per colour, bits per pixel, colours
        for (i, kind) in RasterType::ALL.into_iter().enumerate() {
            let one = encode_typed(std::slice::from_ref(&pages[i]), &opts()).unwrap();
            let h = &one[4..4 + HEADER];
            let be = |at: usize| u32::from_be_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]]);
            let want_cs = if kind.is_gray() {
                18
            } else if matches!(kind, RasterType::Srgb8 | RasterType::Srgb16) {
                19
            } else {
                20
            };
            let channels = kind.channels() as u32;
            assert_eq!((be(400), be(384), be(388), be(420)), (want_cs, kind.bits(), kind.bits() * channels, channels), "{kind:?}");
            assert_eq!(be(392) as usize, 7 * kind.bytes_per_pixel(), "bytes per line, {kind:?}");
        }
    }

    #[test]
    fn a_16_bit_page_keeps_both_bytes_of_a_sample() {
        let mut p = typed(2, 1, RasterType::Srgb16, |_, _, _| 0);
        p.data = vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA];
        let bytes = encode_typed(std::slice::from_ref(&p), &opts()).unwrap();
        assert_eq!(decode_typed_for_tests(&bytes).unwrap(), [p]);
    }

    #[test]
    fn the_keywords_round_trip_and_unknown_ones_are_ignored() {
        for kind in RasterType::ALL {
            assert_eq!(RasterType::from_keyword(kind.keyword()), Some(kind));
            assert_eq!(RasterType::from_keyword(&kind.keyword().to_ascii_uppercase()), Some(kind));
        }
        for other in ["cmyk_8", "rgb_8", "black_1", "device1_8", "adobe-rgb", ""] {
            assert_eq!(RasterType::from_keyword(other), None, "{other}");
        }
        assert!(RasterType::BASELINE.contains(&RasterType::Srgb8) && RasterType::BASELINE.contains(&RasterType::Sgray8));
    }

    #[test]
    fn a_typed_page_with_the_wrong_buffer_is_an_error() {
        let mut p = typed(3, 3, RasterType::AdobeRgb16, |_, _, _| 1);
        p.data.pop();
        assert!(encode_typed(&[p], &opts()).is_err());
    }
}
