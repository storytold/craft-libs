//! Format identification and capability declarations.

use crate::image::{ChannelLayout, SampleType};

/// Supported (or known) raster file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    Tiff,
    WebP,
    Gif,
    Bmp,
    Tga,
    Ico,
    /// Netpbm family: PBM/PGM/PPM (P1–P6), PAM (P7) and PFM (`PF`/`Pf`).
    Pnm,
    /// ZSoft Paintbrush: 1/2/4-bit planar palettes, 8-bit indexed and 24-bit RGB.
    Pcx,
    /// X PixMap icons (XPM2 plain text and XPM3 C-string arrays).
    Xpm,
    /// DirectDraw Surface: a 2D texture, uncompressed or BC1–BC5. Reading takes the top mip
    /// level; writing can add the full mip chain ([`crate::DdsOptions`]).
    Dds,
    Qoi,
    OpenExr,
    /// Radiance RGBE.
    Hdr,
    /// Encoded behind the `avif` feature and decoded behind `avif-decode` (native targets); see
    /// [`ASYMMETRIC_EXCEPTIONS`] for a build with only one of them.
    Avif,
    /// HEIF/HEIC (HEVC-coded): read-only; see [`ASYMMETRIC_EXCEPTIONS`].
    Heif,
    /// FITS (Flexible Image Transport System), the astronomy format: gray or RGB image data.
    Fits,
    /// DPX (SMPTE 268M Digital Picture Exchange): luma, RGB or RGBA film and video frames.
    Dpx,
    /// JPEG XL: lossless export-only until the separate importer lands.
    Jxl,
    /// Kodak Cineon (`.cin`, the 10-bit film-scan format DPX grew from).
    Cineon,
    /// DICOM single-frame images (uncompressed): read-only.
    Dicom,
    /// Amiga IFF/ILBM (`FORM ... ILBM`): planar bitplanes with a palette or 24-bit RGB.
    Iff,
    /// JPEG 2000 (JP2 container or raw J2K codestream): read-only; see [`ASYMMETRIC_EXCEPTIONS`].
    Jp2,
    /// Sun Raster (`.ras`): 1-, 8-, 24- and 32-bit, uncompressed or RLE.
    SunRaster,
    /// SGI image files (`.sgi`, `.rgb`, `.bw`): 8- and 16-bit gray, gray + alpha, RGB and RGBA.
    Sgi,
    /// JPEG-LS (`.jls`): lossless and near-lossless gray and RGB.
    JpegLs,
}

/// What a format can hold **and** what this crate reads/writes for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatCaps {
    /// Decoding is available in this build.
    pub read: bool,
    /// Encoding is available in this build.
    pub write: bool,
    /// Sample types stored without conversion.
    pub depths: &'static [SampleType],
    /// Layouts stored *and read back* without conversion.
    pub layouts: &'static [ChannelLayout],
    pub alpha: bool,
    pub icc: bool,
    pub exif: bool,
    pub xmp: bool,
    /// Physical resolution (DPI) is preserved.
    pub dpi: bool,
    /// Free-form key/value text is preserved.
    pub text: bool,
    /// The container can hold animation/multiple frames. This crate only
    /// decodes the first frame and encodes a single frame.
    pub animation: bool,
    /// Our encoder is lossy (quantization or lossy compression).
    pub lossy: bool,
}

/// Formats that are enabled but intentionally not symmetric, with the reason.
/// The test-suite asserts that every other enabled format is read+write.
pub const ASYMMETRIC_EXCEPTIONS: &[(Format, &str)] = &[
    (
        Format::Xpm,
        "XPM is a legacy Unix icon interchange format. Reading existing XPM icons is supported; writing modern image assets should use PNG instead.",
    ),
    (
        Format::Dicom,
        "DICOM single-frame images are imported for medical review; this crate has no DICOM writer, so save an imported frame as TIFF or PNG instead.",
    ),
    (
        Format::Pcx,
        "PCX is supported for opening legacy paint and clipart assets. New files should use PNG; writing obsolete indexed and planar variants is intentionally not provided.",
    ),
    (
        Format::Avif,
        "AVIF encode (ravif and rav1e, pure Rust) is behind the non-default `avif` feature and AVIF \
         decode (craft-video's clean-room pure-Rust AV1 decoder) behind the non-default \
         `avif-decode` feature, native targets only (on wasm32 a decoder panic would abort). A build \
         with only one of them reads or writes AVIF, not both.",
    ),
    (
        Format::Jxl,
        "Lossless JPEG XL export uses the pure-Rust zune-jpegxl encoder in every build; JPEG XL decode \
         (craft-jxl on jxl-oxide, pure Rust) is behind the non-default `jxl` feature, which official \
         builds enable. Without the feature JPEG XL is written but not read.",
    ),
    (
        Format::Heif,
        "HEIC decode uses heic-rs (pure Rust, in the optional craft-heif crate behind the \
         non-default `heif` feature, which official builds enable), so iPhone and Mac photos open; \
         writing needs an HEVC encoder, and the mature ones (x265, libheif) are C, so write stays \
         unsupported. Without the feature HEIF is detected but neither read nor written.",
    ),
    (Format::Jp2, "JPEG 2000 decode uses hayro-jpeg2000 (pure Rust); writing JPEG 2000 is not provided."),
];

use ChannelLayout as L;
use SampleType as S;

const RGB_GRAY: &[ChannelLayout] = &[L::Gray, L::GrayA, L::Rgb, L::Rgba];
const ALL_LAYOUTS: &[ChannelLayout] = &[L::Gray, L::GrayA, L::Rgb, L::Rgba, L::Cmyk, L::CmykA];
/// TIFF also stores CIE L*a*b* (CIELab).
const TIFF_LAYOUTS: &[ChannelLayout] = &[L::Gray, L::GrayA, L::Rgb, L::Rgba, L::Cmyk, L::CmykA, L::Lab, L::LabA];

impl Format {
    /// Every format known to the crate (enabled or not).
    pub const ALL: [Format; 27] = [
        Format::Png,
        Format::Jpeg,
        Format::Jp2,
        Format::Jxl,
        Format::Tiff,
        Format::WebP,
        Format::Gif,
        Format::Bmp,
        Format::Tga,
        Format::Ico,
        Format::Pnm,
        Format::Pcx,
        Format::Xpm,
        Format::Dds,
        Format::Qoi,
        Format::OpenExr,
        Format::Hdr,
        Format::Avif,
        Format::Heif,
        Format::Fits,
        Format::Dpx,
        Format::Cineon,
        Format::Dicom,
        Format::Iff,
        Format::SunRaster,
        Format::Sgi,
        Format::JpegLs,
    ];

    pub fn caps(self) -> FormatCaps {
        caps(self)
    }

    /// Readable or writable in this build.
    pub fn is_enabled(self) -> bool {
        let c = caps(self);
        c.read || c.write
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Tiff => "TIFF",
            Format::WebP => "WebP",
            Format::Gif => "GIF",
            Format::Bmp => "BMP",
            Format::Tga => "TGA",
            Format::Ico => "ICO",
            Format::Pnm => "Netpbm",
            Format::Pcx => "PCX",
            Format::Xpm => "XPM",
            Format::Dds => "DDS",
            Format::Qoi => "QOI",
            Format::OpenExr => "OpenEXR",
            Format::Hdr => "Radiance HDR",
            Format::Avif => "AVIF",
            Format::Heif => "HEIF",
            Format::Fits => "FITS",
            Format::Dpx => "DPX",
            Format::Jxl => "JPEG XL",
            Format::Cineon => "Cineon",
            Format::Dicom => "DICOM",
            Format::Iff => "Amiga IFF",
            Format::Jp2 => "JPEG 2000",
            Format::SunRaster => "Sun Raster",
            Format::Sgi => "SGI",
            Format::JpegLs => "JPEG-LS",
        }
    }

    /// Recognized file extensions (lowercase, without dot); the first one is
    /// the preferred extension for writing.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Format::Png => &["png", "apng"],
            Format::Jpeg => &["jpg", "jpeg", "jpe", "jfif"],
            Format::Tiff => &["tif", "tiff"],
            Format::WebP => &["webp"],
            Format::Gif => &["gif"],
            Format::Bmp => &["bmp", "dib"],
            Format::Tga => &["tga", "icb", "vda", "vst"],
            Format::Ico => &["ico"],
            Format::Pnm => &["pnm", "pbm", "pgm", "ppm", "pam", "pfm"],
            Format::Pcx => &["pcx"],
            Format::Xpm => &["xpm"],
            Format::Dds => &["dds"],
            Format::Qoi => &["qoi"],
            Format::OpenExr => &["exr"],
            Format::Hdr => &["hdr"],
            Format::Avif => &["avif"],
            Format::Heif => &["heic", "heif", "hif"],
            Format::Fits => &["fits", "fit", "fts"],
            Format::Dpx => &["dpx"],
            Format::Jxl => &["jxl"],
            Format::Cineon => &["cin"],
            Format::Dicom => &["dcm", "dicom", "dc3"],
            Format::Iff => &["iff", "lbm", "ilbm"],
            Format::Jp2 => &["jp2", "j2k", "j2c", "jpf", "jpx"],
            Format::SunRaster => &["ras", "sun"],
            Format::Sgi => &["sgi", "rgb", "rgba", "bw"],
            Format::JpegLs => &["jls"],
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::Tiff => "image/tiff",
            Format::WebP => "image/webp",
            Format::Gif => "image/gif",
            Format::Bmp => "image/bmp",
            Format::Tga => "image/x-tga",
            Format::Ico => "image/x-icon",
            Format::Pnm => "image/x-portable-anymap",
            Format::Pcx => "image/x-pcx",
            Format::Xpm => "image/x-xpixmap",
            Format::Dds => "image/vnd-ms.dds",
            Format::Qoi => "image/qoi",
            Format::OpenExr => "image/x-exr",
            Format::Hdr => "image/vnd.radiance",
            Format::Avif => "image/avif",
            Format::Heif => "image/heif",
            Format::Fits => "image/fits",
            Format::Dpx => "image/x-dpx",
            Format::Jxl => "image/jxl",
            Format::Cineon => "image/x-cin",
            Format::Dicom => "application/dicom",
            Format::Iff => "image/x-iff",
            Format::Jp2 => "image/jp2",
            Format::SunRaster => "image/x-sun-raster",
            Format::Sgi => "image/x-sgi",
            Format::JpegLs => "image/jls",
        }
    }

    /// Maximum encodable dimensions, if the format has a hard limit.
    pub fn max_dimensions(self) -> Option<(u32, u32)> {
        match self {
            Format::Jpeg | Format::Gif | Format::Iff | Format::Sgi | Format::JpegLs => Some((65535, 65535)),
            Format::WebP => Some((16384, 16384)),
            Format::Ico => Some((256, 256)),
            _ => None,
        }
    }
}

/// Capability table. `read`/`write` reflect the current build's features.
pub fn caps(format: Format) -> FormatCaps {
    let base = FormatCaps {
        read: true,
        write: true,
        depths: &[S::U8],
        layouts: &[L::Rgb, L::Rgba],
        alpha: true,
        icc: false,
        exif: false,
        xmp: false,
        dpi: false,
        text: false,
        animation: false,
        lossy: false,
    };
    match format {
        Format::Png => FormatCaps {
            depths: &[S::U8, S::U16],
            layouts: RGB_GRAY,
            icc: true,
            exif: true,
            xmp: true,
            dpi: true,
            text: true,
            animation: true,
            ..base
        },
        Format::Jpeg => {
            FormatCaps { layouts: &[L::Gray, L::Rgb, L::Cmyk], alpha: false, icc: true, exif: true, xmp: true, dpi: true, lossy: true, ..base }
        }
        Format::Tiff => {
            FormatCaps { depths: &[S::U8, S::U16, S::F32], layouts: TIFF_LAYOUTS, icc: true, exif: true, xmp: true, dpi: true, text: true, ..base }
        }
        // Resolution travels in EXIF (WebP has no field of its own).
        Format::WebP => FormatCaps { icc: true, exif: true, xmp: true, dpi: true, animation: true, ..base },
        Format::Gif => FormatCaps { layouts: &[L::Rgba], animation: true, lossy: true, ..base },
        // Resolution in the header's pixels per metre (BITMAPINFOHEADER and later).
        Format::Bmp => FormatCaps { dpi: true, ..base },
        Format::Tga => FormatCaps { layouts: RGB_GRAY, ..base },
        Format::Ico => FormatCaps { layouts: &[L::Rgba], ..base },
        Format::Pnm => FormatCaps { depths: &[S::U8, S::U16, S::F32], layouts: ALL_LAYOUTS, ..base },
        Format::Pcx => FormatCaps { read: true, write: false, layouts: &[L::Rgb], alpha: false, ..base },
        Format::Xpm => FormatCaps { read: true, write: false, layouts: &[L::Rgba], ..base },
        // Uncompressed by default; a block compression is lossy (see `fidelity_warnings_with`).
        Format::Dds => FormatCaps { layouts: RGB_GRAY, ..base },
        Format::Qoi => base,
        Format::OpenExr => FormatCaps { depths: &[S::F16, S::F32], layouts: RGB_GRAY, ..base },
        Format::Hdr => FormatCaps { depths: &[S::F32], layouts: &[L::Rgb], alpha: false, lossy: true, ..base },
        Format::Avif => {
            FormatCaps { read: cfg!(all(feature = "avif-decode", not(target_arch = "wasm32"))), write: cfg!(feature = "avif"), lossy: true, ..base }
        }
        Format::Heif => {
            FormatCaps { read: cfg!(feature = "heif"), write: false, depths: &[S::U8, S::U16], icc: true, exif: true, xmp: true, lossy: true, ..base }
        }
        Format::Fits => FormatCaps { depths: &[S::U8, S::U16, S::F32], layouts: &[L::Gray, L::Rgb], alpha: false, text: true, ..base },
        // `text`: the DPX header fields only (see `fidelity_warnings_with`).
        Format::Dpx => FormatCaps { depths: &[S::U8, S::U16], layouts: &[L::Gray, L::Rgb, L::Rgba], text: true, ..base },
        // GrayA is widened to RGBA at the same depth; the 16-bit alpha header is corrected.
        Format::Jxl => FormatCaps { read: cfg!(feature = "jxl"), depths: &[S::U8, S::U16], layouts: &[L::Gray, L::Rgb, L::Rgba], ..base },
        Format::Cineon => FormatCaps { depths: &[S::U16], layouts: &[L::Gray, L::Rgb], alpha: false, ..base },
        Format::Dicom => FormatCaps { read: true, write: false, depths: &[S::U8, S::U16], layouts: &[L::Gray, L::Rgb], alpha: false, ..base },
        Format::Iff => FormatCaps { depths: &[S::U8], layouts: &[L::Rgb, L::Rgba], alpha: true, ..base },
        Format::SunRaster => FormatCaps {
            read: cfg!(feature = "sunraster"),
            write: cfg!(feature = "sunraster"),
            layouts: &[L::Gray, L::Rgb, L::Rgba],
            alpha: true,
            ..base
        },
        Format::Sgi => {
            FormatCaps { read: cfg!(feature = "sgi"), write: cfg!(feature = "sgi"), depths: &[S::U8, S::U16], layouts: RGB_GRAY, alpha: true, ..base }
        }
        Format::JpegLs => FormatCaps { depths: &[S::U8, S::U16], layouts: &[L::Gray, L::Rgb], alpha: false, ..base },
        Format::Jp2 => FormatCaps { read: true, write: false, layouts: &[L::Gray, L::GrayA, L::Rgb, L::Rgba, L::Cmyk, L::CmykA], icc: true, ..base },
    }
}

/// What [`detect_prefix`] makes of the first bytes of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    /// The bytes identify this format, as [`detect`] reports it.
    Format(Format),
    /// The bytes start with a JPEG SOI and end before the frame header that tells JPEG from
    /// JPEG-LS. Read the file up to at least this many bytes and call [`detect_prefix`] or
    /// [`PrefixDetector::detect`] again.
    NeedBytes(usize),
    /// The bytes match no format.
    Unknown,
}

/// Identify a format from the first bytes of a file, which may be all of it or only its start.
/// A JPEG and a JPEG-LS file both start with an SOI, and only the frame header after their marker
/// segments tells them apart. When `bytes` ends before that header, the answer is
/// [`Detection::NeedBytes`]; every other answer is the one [`detect`] gives.
///
/// Each call walks the marker segments from the SOI. To read more of a file after each
/// [`Detection::NeedBytes`], use a [`PrefixDetector`], which starts each walk where the previous
/// one stopped.
pub fn detect_prefix(bytes: &[u8]) -> Detection {
    PrefixDetector::new().detect(bytes)
}

/// [`detect_prefix`] for a file read in steps. The detector keeps the offset of the JPEG marker
/// where its last walk stopped, so it reads each marker segment once, however many steps the file
/// is read in.
#[derive(Debug, Clone, Default)]
pub struct PrefixDetector {
    jpeg_pos: usize,
}

impl PrefixDetector {
    /// A detector that has seen no bytes.
    pub fn new() -> Self {
        Self::default()
    }

    /// The answer [`detect_prefix`] gives for `bytes`. The `bytes` of each call must start with
    /// the `bytes` of the previous call on this detector.
    pub fn detect(&mut self, bytes: &[u8]) -> Detection {
        use crate::codecs::jpegls::{JpegFrame, jpeg_frame_from};
        match jpeg_frame_from(bytes, &mut self.jpeg_pos) {
            Some(JpegFrame::Short(len)) => Detection::NeedBytes(len),
            Some(JpegFrame::Ls) => Detection::Format(Format::JpegLs),
            // The walk found no SOF55 frame header, so `detect` answers JPEG for these bytes.
            Some(JpegFrame::Other) if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) => Detection::Format(Format::Jpeg),
            Some(JpegFrame::Other) | None => detect(bytes).map_or(Detection::Unknown, Detection::Format),
        }
    }
}

/// Identify a format from its leading bytes (magic numbers). TGA has no
/// magic and is recognized last, by header plausibility.
///
/// JPEG-LS is told from JPEG by the frame header that follows the marker segments, so `bytes`
/// must reach that header; a buffer that ends before it is reported as [`Format::Jpeg`] when it
/// starts with `FF D8 FF`, and a buffer of only `FF D8` matches no format. Use [`detect_prefix`] on
/// a buffer that may hold only the start of a file: for `FF D8` it asks for a 4-byte prefix.
pub fn detect(bytes: &[u8]) -> Option<Format> {
    let b = bytes;
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if crate::codecs::jpegls::looks_like_jpegls(b) {
        return Some(Format::JpegLs);
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Format::Jpeg);
    }
    // JPEG XL bare codestream and ISO/IEC 18181-2 container signature box.
    if b.starts_with(&[0xFF, 0x0A]) || b.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n") {
        return Some(Format::Jxl);
    }
    // JPEG 2000 container signature box or raw J2K codestream (SOC + SIZ).
    if b.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") || b.starts_with(b"\xff\x4f\xff\x51") {
        return Some(Format::Jp2);
    }
    if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") || b.starts_with(b"II+\0") || b.starts_with(b"MM\0+") {
        return Some(Format::Tiff);
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(Format::WebP);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if b.starts_with(b"qoif") {
        return Some(Format::Qoi);
    }
    if b.starts_with(b"DDS ") {
        return Some(Format::Dds);
    }
    if b.starts_with(&[0x76, 0x2F, 0x31, 0x01]) {
        return Some(Format::OpenExr);
    }
    if b.starts_with(b"#?RADIANCE") || b.starts_with(b"#?RGBE") {
        return Some(Format::Hdr);
    }
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        let prefix = &b[..b.len().min(64)];
        if is_avif_ftyp(prefix) {
            return Some(Format::Avif);
        }
        if is_heif_ftyp(prefix) {
            return Some(Format::Heif);
        }
    }
    if b.len() >= 14 && b.starts_with(b"BM") {
        return Some(Format::Bmp);
    }
    if b.len() >= 6 && b[0..4] == [0, 0, 1, 0] && (b[4] != 0 || b[5] != 0) {
        return Some(Format::Ico);
    }
    if b.len() >= 3 && b[0] == b'P' && matches!(b[1], b'1'..=b'7' | b'F' | b'f') && b[2].is_ascii_whitespace() {
        return Some(Format::Pnm);
    }
    if b.starts_with(b"SDPX") || b.starts_with(b"XPDS") {
        return Some(Format::Dpx);
    }
    if crate::codecs::dpx::is_cineon(b) {
        return Some(Format::Cineon);
    }
    if crate::codecs::dicom::is_dicom(b) {
        return Some(Format::Dicom);
    }
    if crate::codecs::iff::looks_like_iff(b) {
        return Some(Format::Iff);
    }
    if b.starts_with(b"SIMPLE  =") {
        return Some(Format::Fits);
    }
    if b.starts_with(b"/* XPM */") || b.starts_with(b"! XPM2\n") || b.starts_with(b"! XPM2\r\n") {
        return Some(Format::Xpm);
    }
    #[cfg(feature = "sunraster")]
    if crate::codecs::sunraster::looks_like_sunraster(b) {
        return Some(Format::SunRaster);
    }
    #[cfg(feature = "sgi")]
    if crate::codecs::sgi::looks_like_sgi(b) {
        return Some(Format::Sgi);
    }
    if crate::codecs::pcx::looks_like_pcx(b) {
        return Some(Format::Pcx);
    }
    if looks_like_tga(b) {
        return Some(Format::Tga);
    }
    None
}

/// The major brand and complete compatible brands in a leading ISO-BMFF `ftyp` box.
///
/// Pass at most the bytes the caller wants to inspect. A missing box or incomplete major
/// brand yields an empty iterator. For sniffing, a declared size below 12 still permits the
/// major brand. Compatible brands stop at the declared box end or the available prefix,
/// whichever comes first. This does not validate the full box.
pub fn ftyp_brands(prefix: &[u8]) -> impl Iterator<Item = &[u8; 4]> + Clone {
    let brands = match prefix.first_chunk::<12>() {
        Some(header) if &header[4..8] == b"ftyp" => {
            let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            &prefix[8..size.clamp(12, prefix.len())]
        }
        _ => &[],
    };
    brands.as_chunks::<4>().0.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, brand)| brand)
}

fn is_avif_ftyp(b: &[u8]) -> bool {
    ftyp_brands(b).any(|c| c == b"avif" || c == b"avis")
}

/// HEVC-coded HEIF (`heic`, `heix`, ...) or a generic HEIF file (`mif1`, `msf1`, `miaf`) that is
/// not AVIF (checked first). Sequence-only files (`hevc`, `msf1`) are recognised so that opening
/// one says what is unsupported instead of "unrecognized format".
fn is_heif_ftyp(b: &[u8]) -> bool {
    ftyp_brands(b).any(|c| matches!(c, b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"miaf"))
}

fn looks_like_tga(b: &[u8]) -> bool {
    if b.len() < 18 {
        return false;
    }
    if b.len() >= 26 && &b[b.len() - 18..] == b"TRUEVISION-XFILE.\0" {
        return true;
    }
    let cmap_type = b[1];
    let img_type = b[2];
    let cmap_len = u16::from_le_bytes([b[5], b[6]]);
    let cmap_depth = b[7];
    let w = u16::from_le_bytes([b[12], b[13]]);
    let h = u16::from_le_bytes([b[14], b[15]]);
    let depth = b[16];
    let desc = b[17];
    let type_ok = matches!(img_type, 1 | 2 | 3 | 9 | 10 | 11);
    let cmap_ok = match cmap_type {
        0 => cmap_len == 0 && cmap_depth == 0 && !matches!(img_type, 1 | 9),
        1 => matches!(img_type, 1 | 9) && cmap_len > 0 && matches!(cmap_depth, 15 | 16 | 24 | 32),
        _ => false,
    };
    let depth_ok = match img_type {
        1 | 9 => depth == 8,
        3 | 11 => matches!(depth, 8 | 16),
        _ => matches!(depth, 15 | 16 | 24 | 32),
    };
    type_ok && cmap_ok && depth_ok && w > 0 && h > 0 && desc & 0xC0 == 0
}

/// Look up a format by file extension (case-insensitive, with or without a
/// leading dot, or a full file name/path).
pub fn from_extension(ext: &str) -> Option<Format> {
    let ext = ext.rsplit(['.', '/', '\\']).next().unwrap_or(ext).to_ascii_lowercase();
    Format::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
}
