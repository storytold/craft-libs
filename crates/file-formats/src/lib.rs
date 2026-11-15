//! Bounded file kind detection and extension resolution shared by Craft applications.
//!
//! This crate identifies a format; it does not parse, validate, or decode the file.
//! Callers remain responsible for choosing which identified formats their product supports.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

/// A supported file family, independent of the application model that consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Eps,
    PostScript,
    Illustrator,
    Dxf,
    Svg,
    /// Gzip-compressed SVG. Kept distinct so writers do not place plain SVG under `.svgz`.
    Svgz,
    Png,
    Jpeg,
    Bmp,
    Gif,
    Tiff,
    WebP,
    Heif,
    Tga,
    Ico,
    Pnm,
    Pcx,
    Xpm,
    Dds,
    Qoi,
    OpenExr,
    Hdr,
    Avif,
    Fits,
    Dpx,
    Jxl,
    Cineon,
    Dicom,
    Iff,
    Ase,
    Psd,
    Affinity,
    Ora,
    Kra,
    Xcf,
    Pdn,
    Raw,
    Pcraft,
    Idml,
    DesignCraft,
    /// Legacy DesignCraft documents serialized as one JSON object.
    DesignCraftJson,
    Indd,
    Docx,
    /// Legacy OLE Compound File Word document; identified by its extension only.
    LegacyDoc,
    Rtf,
    Xlsx,
    TaggedText,
    Pptx,
    PlainText,
    Csv,
    Tsv,
    /// Legacy binary Excel workbooks, identified by extension only.
    LegacyExcel,
    Gcode,
    Studio,
    StencilCraft,
}

/// Availability of one operation in the current client's shared-format build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// The shared reader or writer is included in this build.
    Available,
    /// This format is known, but the corresponding shared operation is not implemented.
    Unsupported,
    /// The operation exists behind a Cargo feature that this client did not enable.
    BuildDisabled,
}

/// Shared reader/writer capability and sequence behavior for a known format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub read: Availability,
    pub write: Availability,
    /// Input may contain multiple frames/pages that the shared reader can expose individually.
    pub multiple_frames: bool,
    /// This format can be opened as an ordered sequence of files by an application adapter.
    pub file_sequence: bool,
}

const fn capability(feature: bool, read: bool, write: bool, multiple_frames: bool, file_sequence: bool) -> Capabilities {
    Capabilities { read: availability(feature, read), write: availability(feature, write), multiple_frames, file_sequence }
}

const fn availability(feature: bool, supported: bool) -> Availability {
    if !supported {
        Availability::Unsupported
    } else if feature {
        Availability::Available
    } else {
        Availability::BuildDisabled
    }
}

impl Format {
    /// Every known format, including formats whose reader or writer is not enabled in this build.
    pub const ALL: [Format; 56] = [
        Self::Pdf,
        Self::Eps,
        Self::PostScript,
        Self::Illustrator,
        Self::Dxf,
        Self::Svg,
        Self::Svgz,
        Self::Png,
        Self::Jpeg,
        Self::Bmp,
        Self::Gif,
        Self::Tiff,
        Self::WebP,
        Self::Heif,
        Self::Tga,
        Self::Ico,
        Self::Pnm,
        Self::Pcx,
        Self::Xpm,
        Self::Dds,
        Self::Qoi,
        Self::OpenExr,
        Self::Hdr,
        Self::Avif,
        Self::Fits,
        Self::Dpx,
        Self::Jxl,
        Self::Cineon,
        Self::Dicom,
        Self::Iff,
        Self::Ase,
        Self::Psd,
        Self::Affinity,
        Self::Ora,
        Self::Kra,
        Self::Xcf,
        Self::Pdn,
        Self::Raw,
        Self::Pcraft,
        Self::Idml,
        Self::DesignCraft,
        Self::DesignCraftJson,
        Self::Indd,
        Self::Docx,
        Self::LegacyDoc,
        Self::Rtf,
        Self::Xlsx,
        Self::TaggedText,
        Self::Pptx,
        Self::PlainText,
        Self::Csv,
        Self::Tsv,
        Self::LegacyExcel,
        Self::Studio,
        Self::StencilCraft,
        Self::Gcode,
    ];

    /// Known extensions in lowercase form, in preferred-output order.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Pdf => &["pdf"],
            Self::Eps => &["eps"],
            Self::PostScript => &["ps"],
            Self::Illustrator => &["ai"],
            Self::Dxf => &["dxf"],
            Self::Svg => &["svg"],
            Self::Svgz => &["svgz"],
            Self::Png => &["png", "apng"],
            Self::Jpeg => &["jpg", "jpeg", "jpe", "jfif"],
            Self::Bmp => &["bmp", "dib"],
            Self::Gif => &["gif"],
            Self::Tiff => &["tif", "tiff"],
            Self::WebP => &["webp"],
            Self::Heif => &["heic", "heif", "hif"],
            Self::Tga => &["tga", "icb", "vda", "vst"],
            Self::Ico => &["ico"],
            Self::Pnm => &["pnm", "pbm", "pgm", "ppm", "pam", "pfm"],
            Self::Pcx => &["pcx"],
            Self::Xpm => &["xpm"],
            Self::Dds => &["dds"],
            Self::Qoi => &["qoi"],
            Self::OpenExr => &["exr"],
            Self::Hdr => &["hdr"],
            Self::Avif => &["avif"],
            Self::Fits => &["fits", "fit", "fts"],
            Self::Dpx => &["dpx"],
            Self::Jxl => &["jxl"],
            Self::Cineon => &["cin"],
            Self::Dicom => &["dcm", "dicom", "dc3"],
            Self::Iff => &["iff", "lbm", "ilbm"],
            Self::Ase => &["ase"],
            Self::Psd => &["psd", "psb", "psdt"],
            Self::Affinity => &["afdesign", "afphoto", "afpub", "af"],
            Self::Ora => &["ora"],
            Self::Kra => &["kra"],
            Self::Xcf => &["xcf"],
            Self::Pdn => &["pdn"],
            Self::Raw => &[
                "dng", "3fr", "ari", "arw", "bay", "cap", "cr2", "cr3", "dcs", "dcr", "drf", "eip", "erf", "fff", "gpr", "iiq", "k25", "kdc", "mdc",
                "mef", "mos", "mrw", "nef", "nrw", "obm", "orf", "pef", "ptx", "pxn", "r3d", "raf", "raw", "rwl", "rw2", "rwz", "sr2", "srf", "sti",
                "x3f",
            ],
            Self::Pcraft => &["pcraft"],
            Self::Idml => &["idml"],
            Self::DesignCraft => &["designcraft", "dcdoc"],
            Self::DesignCraftJson => &[],
            Self::Indd => &["indd", "indt"],
            Self::Docx => &["docx"],
            Self::LegacyDoc => &["doc"],
            Self::Rtf => &["rtf"],
            Self::Xlsx => &["xlsx"],
            Self::TaggedText => &[],
            Self::Pptx => &["pptx"],
            Self::PlainText => &["txt", "text", "md"],
            Self::Csv => &["csv"],
            Self::Tsv => &["tsv", "tab"],
            Self::LegacyExcel => &["xls", "xlsm"],
            Self::Studio => &["studio", "studio3"],
            Self::StencilCraft => &["stencilcraft"],
            Self::Gcode => &["gcode", "nc"],
        }
    }

    /// Preferred extension for creating a new file of this type, if one is defined.
    pub fn preferred_extension(self) -> Option<&'static str> {
        self.extensions().first().copied()
    }

    /// Registered MIME type, where the format has a stable MIME registration.
    pub fn mime_type(self) -> Option<&'static str> {
        Some(match self {
            Self::Pdf => "application/pdf",
            Self::Eps => "application/postscript",
            Self::PostScript => "application/postscript",
            Self::Illustrator => "application/illustrator",
            Self::Dxf => "image/vnd.dxf",
            Self::Svg | Self::Svgz => "image/svg+xml",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Bmp => "image/bmp",
            Self::Gif => "image/gif",
            Self::Tiff => "image/tiff",
            Self::WebP => "image/webp",
            Self::Heif => "image/heif",
            Self::Tga => "image/x-tga",
            Self::Ico => "image/vnd.microsoft.icon",
            Self::Pnm => "image/x-portable-anymap",
            Self::Pcx => "image/vnd.zbrush.pcx",
            Self::Xpm => "image/x-xpixmap",
            Self::Dds => "image/vnd-ms.dds",
            Self::Qoi => "image/qoi",
            Self::OpenExr => "image/x-exr",
            Self::Hdr => "image/vnd.radiance",
            Self::Avif => "image/avif",
            Self::Fits => "image/fits",
            Self::Dpx => "image/dpx",
            Self::Jxl => "image/jxl",
            Self::Cineon => "image/x-cineon",
            Self::Dicom => "application/dicom",
            Self::Iff => "image/ilbm",
            Self::Ase => "application/x-adobe-swatch-exchange",
            Self::Psd => "image/vnd.adobe.photoshop",
            Self::Affinity => "application/x-affinity",
            Self::Ora => "image/openraster",
            Self::Kra => "application/x-krita",
            Self::Xcf => "image/x-xcf",
            Self::Pdn => "application/x-paint.net",
            Self::Raw => "image/x-camera-raw",
            Self::Pcraft => "application/vnd.photocraft+zip",
            Self::Idml => "application/vnd.adobe.indesign-idml-package",
            Self::DesignCraft => "application/vnd.designcraft+zip",
            Self::DesignCraftJson => "application/json",
            Self::Indd => "application/x-indesign",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::LegacyDoc => "application/msword",
            Self::Rtf => "application/rtf",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::TaggedText => "text/plain",
            Self::Pptx => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            Self::PlainText => "text/plain",
            Self::Csv => "text/csv",
            Self::Tsv => "text/tab-separated-values",
            Self::LegacyExcel => "application/vnd.ms-excel",
            Self::Studio => "application/x-silhouette-studio",
            Self::StencilCraft => "application/vnd.stencilcraft+json",
            Self::Gcode => "text/x.gcode",
        })
    }

    /// The corresponding flat-raster codec, when this format is handled by `craft-image-codecs`.
    #[cfg(feature = "raster-codecs")]
    pub fn raster_codec(self) -> Option<craft_image_codecs::Format> {
        use craft_image_codecs::Format as Codec;
        Some(match self {
            Self::Png => Codec::Png,
            Self::Jpeg => Codec::Jpeg,
            Self::Bmp => Codec::Bmp,
            Self::Gif => Codec::Gif,
            Self::Tiff => Codec::Tiff,
            Self::WebP => Codec::WebP,
            Self::Heif => Codec::Heif,
            Self::Tga => Codec::Tga,
            Self::Ico => Codec::Ico,
            Self::Pnm => Codec::Pnm,
            Self::Pcx => Codec::Pcx,
            Self::Xpm => Codec::Xpm,
            Self::Dds => Codec::Dds,
            Self::Qoi => Codec::Qoi,
            Self::OpenExr => Codec::OpenExr,
            Self::Hdr => Codec::Hdr,
            Self::Avif => Codec::Avif,
            Self::Fits => Codec::Fits,
            Self::Dpx => Codec::Dpx,
            Self::Jxl => Codec::Jxl,
            Self::Cineon => Codec::Cineon,
            Self::Dicom => Codec::Dicom,
            Self::Iff => Codec::Iff,
            _ => return None,
        })
    }

    /// Operations exposed by this build's shared parser/codec features.
    pub fn capabilities(self) -> Capabilities {
        #[cfg(feature = "raster-codecs")]
        if let Some(format) = self.raster_codec() {
            let codec = format.caps();
            let read = if codec.read {
                Availability::Available
            } else if matches!(self, Self::Heif) && !cfg!(feature = "heif") {
                Availability::BuildDisabled
            } else {
                Availability::Unsupported
            };
            let write = if codec.write {
                Availability::Available
            } else if matches!(self, Self::Avif) && !cfg!(feature = "avif") {
                Availability::BuildDisabled
            } else {
                Availability::Unsupported
            };
            return Capabilities { read, write, multiple_frames: codec.animation, file_sequence: false };
        }
        let raster = cfg!(feature = "raster-codecs");
        match self {
            Self::Png | Self::Jpeg | Self::Bmp | Self::Gif | Self::Tiff | Self::WebP => {
                capability(raster, true, true, matches!(self, Self::Png | Self::Gif | Self::WebP), false)
            }
            Self::Heif => Capabilities {
                read: availability(cfg!(feature = "heif"), true),
                write: Availability::Unsupported,
                multiple_frames: true,
                file_sequence: false,
            },
            Self::Avif => Capabilities {
                read: Availability::Unsupported,
                write: availability(cfg!(feature = "avif"), true),
                multiple_frames: false,
                file_sequence: false,
            },
            Self::Jxl => capability(raster, false, true, false, false),
            Self::Pcx | Self::Xpm | Self::Dicom => capability(raster, true, false, false, false),
            Self::Tga
            | Self::Ico
            | Self::Pnm
            | Self::Dds
            | Self::Qoi
            | Self::OpenExr
            | Self::Hdr
            | Self::Fits
            | Self::Dpx
            | Self::Cineon
            | Self::Iff => capability(raster, true, true, false, false),
            Self::Pdf => capability(cfg!(feature = "pdf"), true, true, false, false),
            Self::Eps | Self::PostScript | Self::Illustrator => capability(cfg!(feature = "pdf"), true, false, false, false),
            Self::Svg | Self::Svgz => capability(cfg!(feature = "svg"), true, false, false, false),
            Self::Dxf | Self::Gcode => capability(cfg!(feature = "plotter"), true, false, false, false),
            Self::Ora => capability(cfg!(feature = "ora"), true, true, false, false),
            Self::Kra => capability(cfg!(feature = "kra"), true, false, false, false),
            Self::Xcf => capability(cfg!(feature = "xcf"), true, false, false, false),
            Self::Pdn => capability(cfg!(feature = "pdn"), true, false, false, false),
            Self::Raw => capability(cfg!(feature = "raw"), true, false, true, true),
            Self::Psd => capability(cfg!(feature = "psd"), true, true, false, false),
            Self::Affinity => capability(cfg!(feature = "affinity"), true, false, false, false),
            Self::Pcraft => capability(cfg!(feature = "pcraft"), true, true, false, false),
            Self::Ase => capability(cfg!(feature = "ase"), true, true, false, false),
            Self::Studio => capability(cfg!(feature = "stencilcraft"), true, false, false, false),
            Self::Idml | Self::Indd => capability(cfg!(feature = "designcraft"), true, false, false, false),
            Self::DesignCraft | Self::DesignCraftJson => capability(cfg!(feature = "designcraft"), true, true, false, false),
            Self::StencilCraft => capability(cfg!(feature = "stencilcraft"), true, true, false, false),
            Self::Docx
            | Self::LegacyDoc
            | Self::Rtf
            | Self::Xlsx
            | Self::Pptx
            | Self::TaggedText
            | Self::PlainText
            | Self::Csv
            | Self::Tsv
            | Self::LegacyExcel => capability(cfg!(feature = "office"), true, true, false, false),
        }
    }
}

/// Resolve a name and file bytes to a format. Strong signatures take precedence over
/// extension hints. Only format-specific bounded probes and extension tables live here.
pub fn detect(name: &str, bytes: &[u8]) -> Option<Format> {
    // Container identifiers are stronger than generic ZIP signatures and filenames. The shared
    // readers own their bounded, format-specific directory/mimetype probes.
    #[cfg(feature = "pcraft")]
    if craft_pcraft_bundle::is_pcraft_zip(bytes) {
        return Some(Format::Pcraft);
    }
    #[cfg(feature = "ora")]
    if craft_ora::is_ora(bytes) {
        return Some(Format::Ora);
    }
    #[cfg(feature = "kra")]
    if craft_kra::is_kra(bytes) {
        return Some(Format::Kra);
    }
    if craft_idml::is_idml(bytes) {
        return Some(Format::Idml);
    }
    if craft_zip::stored_mimetype(bytes) == Some(b"application/vnd.designcraft+zip") || designcraft_zip_has_document(bytes) {
        return Some(Format::DesignCraft);
    }
    if is_designcraft_json(bytes) {
        return Some(Format::DesignCraftJson);
    }
    if bytes.starts_with(&INDD_SIGNATURE) {
        return Some(Format::Indd);
    }
    if let Some(found) = craft_eps::sniff(bytes) {
        return Some(match (found.kind, found.illustrator) {
            (_, true) => Format::Illustrator,
            (craft_eps::Kind::Pdf, _) => Format::Pdf,
            (craft_eps::Kind::Eps, _) => Format::Eps,
            (craft_eps::Kind::PostScript, _) => Format::PostScript,
        });
    }
    if bytes.starts_with(b"8BPS") {
        return Some(Format::Psd);
    }
    if bytes.starts_with(b"ASEF") {
        return Some(Format::Ase);
    }
    if bytes.starts_with(b"PDN3") {
        return Some(Format::Pdn);
    }
    // OpenEXR's four-byte magic is its canonical signature (v2, section 2.1.1). Keep it in
    // the shared registry so file-handle importers can route without relying on a filename.
    if bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        return Some(Format::OpenExr);
    }
    if bytes.starts_with(b"gimp xcf ") || gzipped_xcf(name, bytes) {
        return Some(Format::Xcf);
    }
    // Several camera formats use TIFF containers. Ask the shared RAW parser before the generic
    // raster signature so TIFF-based RAW files reach the RAW adapter, while ordinary TIFF files
    // retain the raster classification.
    #[cfg(feature = "raw")]
    if craft_raw::probe(bytes).is_some() {
        return Some(Format::Raw);
    }
    if bytes.starts_with(b"\0\xffKA") {
        return Some(Format::Affinity);
    }
    // The short `.af` alias is accepted only as an explicit Affinity filename. Treating it as
    // a generic image based on unrelated pixel bytes makes misspelled native documents appear
    // to open successfully as flat previews.
    if extension(name).is_some_and(|ext| ext.eq_ignore_ascii_case("af")) {
        return Some(Format::Affinity);
    }
    if let Some(raster) = raster_signature(bytes) {
        return Some(raster);
    }
    if bytes.starts_with(b"silhouette04;") || bytes.starts_with(b"silhouette05;") {
        return Some(Format::Studio);
    }
    let tagged = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes).trim_ascii_start();
    if tagged.starts_with(b"<ASCII-") || tagged.starts_with(b"<UNICODE-") || tagged.starts_with(b"<ANSI-") {
        return Some(Format::TaggedText);
    }
    if bytes.trim_ascii_start().starts_with(b"{\\rtf") {
        return Some(Format::Rtf);
    }
    if craft_svg::sniff::is_svgz(bytes)
        && let Ok(text) = craft_svg::svgz::text(bytes, 64 * 1024 * 1024)
        && craft_svg::sniff::is_svg(text.as_bytes())
    {
        return Some(Format::Svgz);
    }
    if craft_svg::sniff::is_svg(bytes) {
        return Some(Format::Svg);
    }
    if let Some(ext) = extension(name)
        && let Some(format) = from_extension(ext)
    {
        return Some(format);
    }
    let head = bytes.get(..4096).unwrap_or(bytes);
    if dxf_ascii_prefix(head) || head.starts_with(b"AutoCAD Binary DXF\r\n\x1a\0") {
        return Some(Format::Dxf);
    }
    None
}

#[cfg(test)]
mod tests;

/// DesignCraft's older ZIPs predate their leading `mimetype`; they are identified by the
/// `document.json` member. The directory is bounded before it is indexed and no entry is inflated.
fn designcraft_zip_has_document(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK") {
        return false;
    }
    craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 100_000, max_total_bytes: 0 })
        .is_ok_and(|archive| archive.contains("document.json"))
}

/// Read only a bounded prefix, stopping as soon as the model's required top-level keys have
/// appeared. This distinguishes legacy DesignCraft JSON from arbitrary JSON without decoding the
/// document or walking its arrays and assets.
fn is_designcraft_json(bytes: &[u8]) -> bool {
    const LIMIT: usize = 1 << 20;
    const REQUIRED: u16 = (1 << 10) - 1;
    const KEYS: [&str; 10] = ["title", "settings", "spreads", "parents", "layers", "stories", "styles", "swatches", "sections", "next_id"];

    struct Probe;
    impl<'de> serde::de::Visitor<'de> for Probe {
        type Value = bool;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a DesignCraft document object")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut found = 0u16;
            while let Some(key) = map.next_key::<String>()? {
                if let Some(index) = KEYS.iter().position(|candidate| *candidate == key) {
                    found |= 1 << index;
                }
                map.next_value::<serde::de::IgnoredAny>()?;
                if found == REQUIRED {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }

    use serde::de::DeserializeSeed;
    struct Seed;
    impl<'de> DeserializeSeed<'de> for Seed {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_map(Probe)
        }
    }

    let prefix = bytes.get(..bytes.len().min(LIMIT)).unwrap_or(bytes);
    let mut deserializer = serde_json::Deserializer::from_slice(prefix);
    Seed.deserialize(&mut deserializer).unwrap_or(false)
}

/// Resolve a file extension to a family, case-insensitively.
pub fn from_extension(ext: &str) -> Option<Format> {
    let ext = ext.trim_start_matches('.');
    Format::ALL.into_iter().find(|format| format.extensions().iter().any(|candidate| candidate.eq_ignore_ascii_case(ext)))
}

const INDD_SIGNATURE: [u8; 16] = [0x06, 0x06, 0xED, 0xF5, 0xD8, 0x1D, 0x46, 0xE5, 0xBD, 0x31, 0xEF, 0xE7, 0xFE, 0x74, 0xB7, 0x1D];

fn extension(name: &str) -> Option<&str> {
    Some(name.rsplit(['/', '\\']).next()?.rsplit_once('.')?.1)
}

fn gzipped_xcf(name: &str, bytes: &[u8]) -> bool {
    let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    bytes.starts_with(&[0x1f, 0x8b]) && (lower.ends_with(".xcf.gz") || lower.ends_with(".xcfgz"))
}

fn dxf_ascii_prefix(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().map(str::trim);
    matches!((lines.next(), lines.next(), lines.next(), lines.next()), (Some("0"), Some("SECTION"), Some("2"), Some(section)) if matches!(section, "HEADER" | "CLASSES" | "TABLES" | "BLOCKS" | "ENTITIES" | "OBJECTS"))
}

fn raster_signature(bytes: &[u8]) -> Option<Format> {
    let head = bytes.get(..16).unwrap_or(bytes);
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if head.starts_with(b"\xff\xd8\xff") {
        return Some(Format::Jpeg);
    }
    if head.starts_with(b"BM") {
        return Some(Format::Bmp);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") || head.starts_with(b"II+\0") || head.starts_with(b"MM\0+") {
        return Some(Format::Tiff);
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some(Format::WebP);
    }
    None
}
