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

/// Resolve a name and file bytes to a format. Strong signatures take precedence over
/// extension hints. Only format-specific bounded probes and extension tables live here.
pub fn detect(name: &str, bytes: &[u8]) -> Option<Format> {
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
    if bytes.starts_with(b"gimp xcf ") || gzipped_xcf(name, bytes) {
        return Some(Format::Xcf);
    }
    if bytes.starts_with(b"\0\xffKA") {
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
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "pdf" => Some(Format::Pdf),
        "eps" => Some(Format::Eps),
        "ps" => Some(Format::PostScript),
        "ai" => Some(Format::Illustrator),
        "dxf" => Some(Format::Dxf),
        "svg" => Some(Format::Svg),
        "svgz" => Some(Format::Svgz),
        "png" => Some(Format::Png),
        "jpg" | "jpeg" => Some(Format::Jpeg),
        "bmp" => Some(Format::Bmp),
        "gif" => Some(Format::Gif),
        "tif" | "tiff" => Some(Format::Tiff),
        "webp" => Some(Format::WebP),
        "heic" | "heif" => Some(Format::Heif),
        "ase" => Some(Format::Ase),
        "psd" | "psb" => Some(Format::Psd),
        "afdesign" | "afphoto" | "afpub" => Some(Format::Affinity),
        "ora" => Some(Format::Ora),
        "kra" => Some(Format::Kra),
        "xcf" => Some(Format::Xcf),
        "pdn" => Some(Format::Pdn),
        "dng" | "3fr" | "ari" | "arw" | "bay" | "cap" | "cr2" | "cr3" | "dcs" | "dcr" | "drf" | "eip" | "erf" | "fff" | "gpr" | "iiq" | "k25"
        | "kdc" | "mdc" | "mef" | "mos" | "mrw" | "nef" | "nrw" | "obm" | "orf" | "pef" | "ptx" | "pxn" | "r3d" | "raf" | "raw" | "rwl" | "rw2"
        | "rwz" | "sr2" | "srf" | "sti" | "x3f" => Some(Format::Raw),
        "pcraft" => Some(Format::Pcraft),
        "idml" => Some(Format::Idml),
        "designcraft" | "dcdoc" => Some(Format::DesignCraft),
        "indd" | "indt" => Some(Format::Indd),
        "docx" => Some(Format::Docx),
        "doc" => Some(Format::LegacyDoc),
        "rtf" => Some(Format::Rtf),
        "xlsx" => Some(Format::Xlsx),
        "pptx" => Some(Format::Pptx),
        "txt" | "text" | "md" => Some(Format::PlainText),
        "csv" => Some(Format::Csv),
        "tsv" | "tab" => Some(Format::Tsv),
        "xls" | "xlsm" => Some(Format::LegacyExcel),
        "studio" | "studio3" => Some(Format::Studio),
        "stencilcraft" => Some(Format::StencilCraft),
        "gcode" | "nc" => Some(Format::Gcode),
        _ => None,
    }
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
